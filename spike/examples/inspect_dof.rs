//! Prints and checks the DOF ld64 built into a macOS binary.
//!
//! Usage: `cargo run --example inspect_dof -- PATH`.
//!
//! For every DOF probe entry it prints the containing function, the number of
//! probe and is-enabled sites, and each site's address. ld64 writes one entry
//! per function that contains sites, so a probe name can appear several times;
//! `dof::Section` keys probes by name and would merge them, so this reads the
//! raw DOF instead.
//!
//! It also checks every site and exits non-zero if one is wrong:
//!
//! - a probe site must be a `nop` and an is-enabled site must zero the return
//!   register, i.e. ld64 rewrote the call;
//! - the instruction after a site must still be inside the same function. A
//!   site that ends its function was a tail call: once ld64 turns it into a
//!   `nop`, execution falls into the next function.
//!
//! Site addresses are the DOF section's address plus the site offset as a
//! signed 32-bit value, which is how ld64 lays them out (`dofpr_addr` is 0).
//! On x86-64 a site is the call's operand, one byte into the instruction.

use dof::dof_bindings::{DOF_SECT_PROVIDER, dof_hdr, dof_probe, dof_provider, dof_sec};
use goblin::mach::{Mach, MachO, cputype};
use std::mem::size_of;

fn read<T: Copy>(buf: &[u8], offset: usize) -> Result<T, String> {
    let bytes = buf
        .get(offset..offset + size_of::<T>())
        .ok_or_else(|| format!("{} bytes at {offset:#x} out of range", size_of::<T>()))?;
    // SAFETY: `bytes` holds exactly `size_of::<T>()` bytes, and every `T` read
    // here is a `repr(C)` DOF struct of integers, valid for any bit pattern.
    Ok(unsafe { bytes.as_ptr().cast::<T>().read_unaligned() })
}

fn cstr(strtab: &[u8], offset: u32) -> String {
    let s = strtab.get(offset as usize..).unwrap_or_default();
    let end = s.iter().position(|&b| b == 0).unwrap_or(s.len());
    String::from_utf8_lossy(&s[..end]).into_owned()
}

fn u32s(buf: &[u8]) -> Vec<u32> {
    buf.chunks_exact(4)
        .map(|c| u32::from_ne_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

#[derive(Clone, Copy, PartialEq)]
enum Arch {
    Arm64,
    X86_64,
}

impl Arch {
    /// Where the rewritten instruction starts. On x86-64 ld64 records the
    /// address of the call's 32-bit operand, one byte past the opcode.
    fn insn_start(self, site: u64) -> u64 {
        match self {
            Arch::Arm64 => site,
            Arch::X86_64 => site - 1,
        }
    }

    /// Length of the instruction ld64 left at a site.
    fn site_len(self) -> u64 {
        match self {
            Arch::Arm64 => 4,
            Arch::X86_64 => 5,
        }
    }

    fn is_nop(self, code: &[u8]) -> bool {
        match self {
            Arch::Arm64 => code.starts_with(&0xd503_201f_u32.to_le_bytes()),
            // ld64 replaces the 5-byte call with `nop; nopl (%rax)`.
            Arch::X86_64 => code.starts_with(&[0x90, 0x0f, 0x1f, 0x40, 0x00]),
        }
    }

    fn zeroes_result(self, code: &[u8]) -> bool {
        match self {
            // movz x0, #0
            Arch::Arm64 => code.starts_with(&0xd280_0000_u32.to_le_bytes()),
            // xor eax, eax (either encoding), then `nop` padding
            Arch::X86_64 => {
                (code.starts_with(&[0x33, 0xc0]) || code.starts_with(&[0x31, 0xc0]))
                    && code.get(2..5) == Some(&[0x90; 3])
            }
        }
    }
}

/// Text symbols sorted by address, and a way to read code at an address.
struct Image<'a> {
    arch: Arch,
    data: &'a [u8],
    /// Address, size and file offset of `__TEXT,__text`.
    text: (u64, u64, u64),
    symbols: Vec<(u64, String)>,
}

impl Image<'_> {
    fn code(&self, addr: u64) -> &[u8] {
        let (start, size, offset) = self.text;
        if addr < start || addr >= start + size {
            return &[];
        }
        &self.data[(offset + addr - start) as usize..]
    }

    /// The function containing `addr`, and where the next one starts.
    fn function(&self, addr: u64) -> (&str, u64) {
        let i = self.symbols.partition_point(|(a, _)| *a <= addr);
        let name = i.checked_sub(1).map_or("?", |i| self.symbols[i].1.as_str());
        let end = self
            .symbols
            .get(i)
            .map_or(self.text.0 + self.text.1, |(a, _)| *a);
        (name, end)
    }

    /// Checks one site; returns a description of what is wrong with it.
    fn check(&self, site: u64, is_enabled: bool) -> Option<String> {
        let addr = self.arch.insn_start(site);
        let code = self.code(addr);
        let rewritten = if is_enabled {
            self.arch.zeroes_result(code)
        } else {
            self.arch.is_nop(code)
        };
        if !rewritten {
            return Some(format!("{addr:#x}: not rewritten by ld64"));
        }
        let (name, end) = self.function(addr);
        if addr + self.arch.site_len() >= end {
            return Some(format!(
                "{addr:#x}: last instruction of {name}; falls through into the next function"
            ));
        }
        None
    }
}

fn dump(
    image: &Image,
    dof_addr: u64,
    buf: &[u8],
    problems: &mut Vec<String>,
) -> Result<(), String> {
    let header: dof_hdr = read(buf, 0)?;
    let sections = (0..header.dofh_secnum as usize)
        .map(|i| {
            read::<dof_sec>(
                buf,
                header.dofh_secoff as usize + i * header.dofh_secsize as usize,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let body = |index: u32| -> &[u8] {
        let s = &sections[index as usize];
        let start = s.dofs_offset as usize;
        &buf[start..start + s.dofs_size as usize]
    };
    let site = |off: u32| dof_addr.wrapping_add_signed(i64::from(off as i32));
    for (i, sec) in sections.iter().enumerate() {
        if sec.dofs_type != DOF_SECT_PROVIDER {
            continue;
        }
        let provider: dof_provider = read(body(i as u32), 0)?;
        let strtab = body(provider.dofpv_strtab);
        let offsets = u32s(body(provider.dofpv_proffs));
        let enabled = u32s(body(provider.dofpv_prenoffs));
        let probes = body(provider.dofpv_probes);
        for p in 0..probes.len() / size_of::<dof_probe>() {
            let probe: dof_probe = read(probes, p * size_of::<dof_probe>())?;
            let offs = &offsets[probe.dofpr_offidx as usize..][..probe.dofpr_noffs as usize];
            let en = &enabled[probe.dofpr_enoffidx as usize..][..probe.dofpr_nenoffs as usize];
            let probe_sites: Vec<u64> = offs.iter().map(|&o| site(o)).collect();
            let enabled_sites: Vec<u64> = en.iter().map(|&o| site(o)).collect();
            println!(
                "{}:{}:{} sites={} is_enabled_sites={} at={probe_sites:x?} enabled_at={enabled_sites:x?}",
                cstr(strtab, provider.dofpv_name),
                cstr(strtab, probe.dofpr_func),
                cstr(strtab, probe.dofpr_name),
                offs.len(),
                en.len(),
            );
            problems.extend(probe_sites.iter().filter_map(|&a| image.check(a, false)));
            problems.extend(enabled_sites.iter().filter_map(|&a| image.check(a, true)));
        }
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("usage: inspect_dof PATH")?;
    let data = std::fs::read(&path)?;
    let macho: MachO = match Mach::parse(&data)? {
        Mach::Binary(m) => m,
        Mach::Fat(_) => return Err("universal binaries are not supported".into()),
    };
    let arch = match macho.header.cputype {
        cputype::CPU_TYPE_ARM64 => Arch::Arm64,
        cputype::CPU_TYPE_X86_64 => Arch::X86_64,
        other => return Err(format!("unsupported CPU type {other:#x}").into()),
    };

    let mut text = None;
    let mut dofs = Vec::new();
    for segment in &macho.segments {
        for (section, bytes) in segment.sections()? {
            match section.name()? {
                "__text" => text = Some((section.addr, section.size, u64::from(section.offset))),
                name if name.starts_with("__dof_") => dofs.push((section.addr, bytes)),
                _ => {}
            }
        }
    }
    let mut symbols: Vec<(u64, String)> = macho
        .symbols()
        .filter_map(Result::ok)
        .filter(|(_, nlist)| !nlist.is_stab() && nlist.n_sect != 0 && nlist.n_value != 0)
        .map(|(name, nlist)| (nlist.n_value, name.to_owned()))
        .collect();
    symbols.sort();
    symbols.dedup_by_key(|(a, _)| *a);
    let image = Image {
        arch,
        data: &data,
        text: text.ok_or("no __TEXT,__text section")?,
        symbols,
    };

    let mut problems = Vec::new();
    for (addr, bytes) in dofs {
        dump(&image, addr, bytes, &mut problems)?;
    }
    for p in &problems {
        eprintln!("error: {p}");
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(format!("{} bad probe site(s)", problems.len()).into())
    }
}
