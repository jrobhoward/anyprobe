//! Reading a built binary: its registry section, and the probe names its
//! tracer metadata has sites for.

use std::collections::HashSet;
use std::path::Path;

use goblin::Object;
use goblin::elf::Elf;
use goblin::mach::{Mach, MachO, SingleArch, cputype};
use goblin::pe::PE;

use crate::error::Error;

/// What was read from one binary.
#[derive(Debug)]
pub struct Binary {
    /// The object format: `ELF`, `Mach-O` or `PE`.
    pub format: &'static str,
    /// The registry section's bytes; empty if it has none.
    pub registry: Vec<u8>,
    /// `(provider, probe)` for every probe with at least one site in the
    /// tracer metadata, names normalized by [`site_key`]. `None` where the
    /// metadata is not in the file (Windows builds it at runtime).
    pub sites: Option<HashSet<(String, String)>>,
}

/// The key `sites` uses for a probe: DTrace writes `__` in a probe name as
/// `-`, SDT keeps it, so both are compared with `-`.
pub fn site_key(provider: &str, name: &str) -> (String, String) {
    (provider.to_owned(), name.replace("__", "-"))
}

/// Reads `path`. For a universal (fat) Mach-O file, `arch` picks the slice
/// (`aarch64` or `x86_64`, the first part of a target triple); without it the
/// first slice is read.
pub fn read(path: &Path, arch: Option<&str>) -> Result<Binary, Error> {
    let bytes = std::fs::read(path).map_err(|source| Error::Read {
        path: path.to_owned(),
        source,
    })?;
    let format_error = |what: String| Error::Format {
        path: path.to_owned(),
        what,
    };
    let object = Object::parse(&bytes).map_err(|e| format_error(e.to_string()))?;
    let (format, registry, sites) = match object {
        Object::Elf(elf) => {
            let (registry, sites) = read_elf(&elf, &bytes).map_err(format_error)?;
            ("ELF", registry, Some(sites))
        }
        Object::Mach(Mach::Binary(macho)) => {
            let (registry, sites) = read_macho(&macho).map_err(format_error)?;
            ("Mach-O", registry, Some(sites))
        }
        Object::Mach(Mach::Fat(fat)) => {
            let wanted = arch.map(|a| match a {
                "aarch64" | "arm64" | "arm64e" => cputype::CPU_TYPE_ARM64,
                "x86_64" => cputype::CPU_TYPE_X86_64,
                _ => 0,
            });
            let mut chosen = None;
            for index in 0..fat.narches {
                if let Ok(SingleArch::MachO(macho)) = fat.get(index)
                    && wanted.is_none_or(|w| macho.header.cputype == w)
                {
                    chosen = Some(macho);
                    break;
                }
            }
            let macho = chosen.ok_or_else(|| {
                format_error(format!(
                    "universal binary has no {} slice",
                    arch.unwrap_or("Mach-O")
                ))
            })?;
            let (registry, sites) = read_macho(&macho).map_err(format_error)?;
            ("Mach-O", registry, Some(sites))
        }
        Object::PE(pe) => ("PE", read_pe(&pe, &bytes).map_err(format_error)?, None),
        _ => {
            return Err(format_error(
                "not an executable or shared library (ELF, Mach-O or PE)".to_owned(),
            ));
        }
    };
    Ok(Binary {
        format,
        registry,
        sites,
    })
}

type Read = (Vec<u8>, HashSet<(String, String)>);

/// The `anyprobe_probes` section, and the probes in `.note.stapsdt` (Linux)
/// and `anyprobe_sites` (FreeBSD).
fn read_elf(elf: &Elf<'_>, bytes: &[u8]) -> Result<Read, String> {
    let mut registry = Vec::new();
    let mut sites = HashSet::new();
    for header in &elf.section_headers {
        let name = elf.shdr_strtab.get_at(header.sh_name);
        if name == Some("anyprobe_probes")
            && let Some(range) = header.file_range()
        {
            registry = bytes
                .get(range)
                .ok_or("anyprobe_probes section is past the end of the file")?
                .to_vec();
        } else if name == Some("anyprobe_sites")
            && let Some(range) = header.file_range()
        {
            let table = bytes
                .get(range)
                .ok_or("anyprobe_sites section is past the end of the file")?;
            read_site_table(table, elf.little_endian, &mut sites)?;
        }
    }
    // A note's description is three addresses (the site, `.stapsdt.base`,
    // the semaphore), then the provider, name and arguments, each ending
    // with a NUL.
    let addresses = if elf.is_64 { 24 } else { 12 };
    if let Some(notes) = elf.iter_note_sections(bytes, Some(".note.stapsdt")) {
        for note in notes {
            let note = note.map_err(|e| format!("bad SDT note: {e}"))?;
            if note.name != "stapsdt" || note.n_type != 3 {
                continue;
            }
            let mut strings = note
                .desc
                .get(addresses..)
                .unwrap_or_default()
                .split(|&b| b == 0)
                .map(String::from_utf8_lossy);
            if let (Some(provider), Some(name)) = (strings.next(), strings.next()) {
                sites.insert(site_key(&provider, &name));
            }
        }
    }
    Ok((registry, sites))
}

/// Adds the probes in a FreeBSD site table to `sites`. Each record is its
/// length (4 bytes, the target's byte order), version, kind, 2 zero bytes,
/// the site address (8 bytes), then the NUL-terminated provider and probe
/// name; zero bytes between records are skipped 8 at a time. Only the names
/// are read, so the site addresses, which the dynamic linker fills in for a
/// position-independent executable, do not matter.
fn read_site_table(
    table: &[u8],
    little_endian: bool,
    sites: &mut HashSet<(String, String)>,
) -> Result<(), String> {
    const HEADER_LEN: usize = 16;
    let mut offset = 0;
    while offset < table.len() {
        let rest = &table[offset..];
        if rest.iter().take(8).all(|&b| b == 0) {
            offset += rest.len().min(8);
            continue;
        }
        let bad = || format!("bad site record at byte {offset} of anyprobe_sites");
        let len = rest.get(..4).ok_or_else(bad)?;
        let len = [len[0], len[1], len[2], len[3]];
        let len = if little_endian {
            u32::from_le_bytes(len)
        } else {
            u32::from_be_bytes(len)
        } as usize;
        let record = rest
            .get(..len)
            .filter(|_| len >= HEADER_LEN)
            .ok_or_else(bad)?;
        let mut strings = record[HEADER_LEN..]
            .split(|&b| b == 0)
            .map(String::from_utf8_lossy);
        if let (Some(provider), Some(name)) = (strings.next(), strings.next()) {
            sites.insert(site_key(&provider, &name));
        }
        offset += len;
    }
    Ok(())
}

/// The `__DATA,__anyprobe` section, and the probes in the `__TEXT,__dof_*`
/// sections ld64 built.
fn read_macho(macho: &MachO<'_>) -> Result<Read, String> {
    let mut registry = Vec::new();
    let mut sites = HashSet::new();
    for segment in &macho.segments {
        let sections = segment
            .sections()
            .map_err(|e| format!("bad Mach-O section table: {e}"))?;
        for (section, data) in sections {
            let (Ok(segname), Ok(sectname)) = (section.segname(), section.name()) else {
                continue;
            };
            if segname == "__DATA" && sectname == "__anyprobe" {
                registry = data.to_vec();
            } else if segname == "__TEXT" && sectname.starts_with("__dof_") {
                read_dof(data, &mut sites).map_err(|e| format!("bad DOF in {sectname}: {e}"))?;
            }
        }
    }
    Ok((registry, sites))
}

/// Adds the probes in one DOF section to `sites`. ld64 aligns a `__dof_*`
/// section to one byte, but `dof` reads the DOF headers in place, which
/// needs their 8-byte alignment. The bytes are copied to an address that has
/// it, so the result does not depend on where the section sits in the file.
fn read_dof(data: &[u8], sites: &mut HashSet<(String, String)>) -> Result<(), dof::Error> {
    let mut buf = vec![0; data.len() + 7];
    let start = buf.as_ptr().addr().wrapping_neg() % 8;
    let aligned = &mut buf[start..start + data.len()];
    aligned.copy_from_slice(data);
    let dof = dof::des::deserialize_section(aligned)?;
    for provider in dof.providers.values() {
        for probe in provider.probes.values() {
            sites.insert(site_key(&provider.name, &probe.name));
        }
    }
    Ok(())
}

/// The `.aprobe` section. The file holds it padded to the file alignment
/// with zeros, which the registry parser skips.
fn read_pe(pe: &PE<'_>, bytes: &[u8]) -> Result<Vec<u8>, String> {
    for section in &pe.sections {
        if section.name().ok() == Some(".aprobe") {
            let len = section.size_of_raw_data.min(section.virtual_size);
            return raw_range(bytes, section.pointer_to_raw_data, len)
                .map(<[u8]>::to_vec)
                .ok_or_else(|| ".aprobe section is past the end of the file".to_owned());
        }
    }
    Ok(Vec::new())
}

/// `len` bytes of `bytes` from `start`, or `None` if they are not all there.
fn raw_range(bytes: &[u8], start: u32, len: u32) -> Option<&[u8]> {
    let start = usize::try_from(start).ok()?;
    let end = start.checked_add(usize::try_from(len).ok()?)?;
    bytes.get(start..end)
}

#[cfg(test)]
#[path = "binary_tests.rs"]
mod binary_tests;
