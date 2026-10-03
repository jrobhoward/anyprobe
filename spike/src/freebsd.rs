//! FreeBSD: DTrace USDT registered at runtime.
//!
//! FreeBSD's toolchain normally runs `dtrace -G` over object files to turn
//! probe calls into `nop`s and embed DOF, which does not fit a Cargo build.
//! Instead each site is emitted as its final instruction plus a record in
//! the `set_anyprobe_probes` section. [`register`] reads the section through
//! the linker's `__start_`/`__stop_` symbols, builds DOF with the `dof` crate
//! and hands it to the kernel with the `DTRACEHIOC_ADDDOF` ioctl.
//!
//! - A probe site is a `nop` with its arguments in the System V argument
//!   registers, which is where `fasttrap` reads them.
//! - An is-enabled site is `xor eax, eax`; `fasttrap` emulates it as setting
//!   `rax` to 1 while the probe is enabled.
//! - Probe sites are `readonly` rather than `nomem`, because the tracer reads
//!   memory through pointer arguments.

use std::fs::OpenOptions;
use std::io;
use std::os::fd::AsRawFd;
use std::sync::OnceLock;
use std::sync::atomic::AtomicU64;

use crate::dof_records::{self, RECORD_VERSION};

pub(crate) const NAME: &str = "freebsd-dtrace";

#[cfg(not(target_arch = "x86_64"))]
compile_error!("the FreeBSD DTrace backend supports x86_64 only");

// Keeps `__start_`/`__stop_set_anyprobe_probes` defined when no record made it
// into the link, and keeps the section writable: the records' own section
// flags are "aw", and a read-only static here would split it in two.
#[unsafe(link_section = "set_anyprobe_probes")]
#[used]
static SECTION_ANCHOR: [AtomicU64; 0] = [];

/// Emits a site: `$insn` at a numeric local label, then a record pointing at
/// it. The `R` flag (`SHF_GNU_RETAIN`) stops `--gc-sections` dropping the
/// records, which nothing references.
macro_rules! site {
    ($insn:literal, $kind:literal, $probe:literal, [$($ty:literal),*], $($operands:tt)*) => {
        core::arch::asm!(
            concat!("990: ", $insn),
            ".pushsection set_anyprobe_probes, \"awR\", \"progbits\"",
            ".balign 8",
            "991: .4byte 992f-991b",
            concat!(".byte ", RECORD_VERSION_LITERAL!()),
            concat!(".byte ", $kind),
            concat!(".2byte ", count!($($ty)*)),
            ".8byte 990b",
            ".asciz \"spike\"",
            concat!(".asciz \"", $probe, "\""),
            ".asciz \"work\"",
            $(concat!(".asciz \"", $ty, "\""),)*
            ".balign 8",
            "992: .popsection",
            $($operands)*
        )
    };
}

macro_rules! count {
    () => {
        "0"
    };
    ($a:literal) => {
        "1"
    };
    ($a:literal $b:literal) => {
        "2"
    };
    ($a:literal $b:literal $c:literal) => {
        "3"
    };
}

// `concat!` needs a literal; this must equal `RECORD_VERSION`, which the
// assertion below enforces.
macro_rules! RECORD_VERSION_LITERAL {
    () => {
        "1"
    };
}
const _: () = assert!(RECORD_VERSION == 1);

#[inline(always)]
fn is_enabled_entry() -> bool {
    let rax: u64;
    // SAFETY: executes `xor eax, eax`, writing only `rax` (and flags, which
    // are not preserved). The record only emits data.
    unsafe {
        site!("xor eax, eax", "1", "work-entry", [], out("rax") rax, options(nomem, nostack));
    }
    rax != 0
}

#[inline(always)]
fn is_enabled_return() -> bool {
    let rax: u64;
    // SAFETY: as for `is_enabled_entry`.
    unsafe {
        site!("xor eax, eax", "1", "work-return", [], out("rax") rax, options(nomem, nostack));
    }
    rax != 0
}

pub(crate) fn register() -> io::Result<()> {
    static RESULT: OnceLock<Result<(), (io::ErrorKind, String)>> = OnceLock::new();
    RESULT
        .get_or_init(|| register_once().map_err(|e| (e.kind(), e.to_string())))
        .clone()
        .map_err(|(kind, msg)| io::Error::new(kind, msg))
}

fn register_once() -> io::Result<()> {
    unsafe extern "C" {
        #[link_name = "__start_set_anyprobe_probes"]
        static START: u8;
        #[link_name = "__stop_set_anyprobe_probes"]
        static STOP: u8;
    }
    let start = &raw const START;
    let stop = &raw const STOP;
    // SAFETY: the linker defines both symbols at the bounds of the
    // `set_anyprobe_probes` output section, which `SECTION_ANCHOR` keeps
    // present. Nothing writes to the records after startup.
    let section = unsafe {
        std::slice::from_raw_parts(start, (stop as usize).saturating_sub(start as usize))
    };
    let sites = dof_records::parse(section)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{e:?}")))?;
    let dof = dof::serialize_section(&dof_records::to_dof(&sites));
    add_dof(&dof)
}

fn add_dof(dof: &[u8]) -> io::Result<()> {
    let mut modname = [0 as libc::c_char; 64];
    let exe = std::env::current_exe().ok();
    let name = exe
        .as_deref()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("anyprobe");
    for (dst, src) in modname.iter_mut().zip(name.bytes().take(63)) {
        *dst = src as libc::c_char;
    }
    let helper = dof::dof_bindings::dof_helper {
        dofhp_mod: modname,
        dofhp_addr: dof.as_ptr() as u64,
        dofhp_dof: dof.as_ptr() as u64,
        dofhp_pid: std::process::id() as i32,
        dofhp_gen: 0,
    };
    // _IOWR('z', 3, dof_helper_t), from <sys/ioccom.h>'s encoding.
    const IOC_INOUT: u64 = 0xc000_0000;
    let size = std::mem::size_of::<dof::dof_bindings::dof_helper>() as u64;
    let cmd = IOC_INOUT | ((size & 0x1fff) << 16) | ((b'z' as u64) << 8) | 3;

    let helper_dev = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/dtrace/helper")?;
    // SAFETY: `helper` is a fully initialized `dof_helper_t` whose `dofhp_dof`
    // points at `dof`, which outlives the call; the kernel copies the DOF in
    // before returning.
    let rc = unsafe { libc::ioctl(helper_dev.as_raw_fd(), cmd as libc::c_ulong, &helper) };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }
    // The ioctl returns a generation number that would let a later
    // DTRACEHIOC_REMOVE unregister the DOF. Probes stay for the life of the
    // process, so it is not kept.
    Ok(())
}

#[inline(always)]
pub(crate) fn entry_enabled() -> bool {
    is_enabled_entry()
}

#[inline(always)]
pub(crate) fn return_enabled() -> bool {
    is_enabled_return()
}

#[inline(always)]
pub(crate) fn fire_entry(id: u64, label: &str) {
    // SAFETY: executes one `nop`. The arguments sit in the System V argument
    // registers for `fasttrap` to read; the pointer and length describe
    // `label`, which outlives the site.
    unsafe {
        site!(
            "nop", "0", "work-entry", ["uint64_t", "char *", "uint64_t"],
            in("rdi") id, in("rsi") label.as_ptr(), in("rdx") label.len() as u64,
            options(readonly, nostack, preserves_flags)
        );
    }
}

#[inline(always)]
pub(crate) fn fire_return(id: u64, result: u64) {
    // SAFETY: as for `fire_entry`; both arguments are plain integers.
    unsafe {
        site!(
            "nop", "0", "work-return", ["uint64_t", "uint64_t"],
            in("rdi") id, in("rsi") result,
            options(readonly, nostack, preserves_flags)
        );
    }
}
