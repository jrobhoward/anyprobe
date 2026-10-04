//! FreeBSD: DTrace USDT, registered at runtime.
//!
//! FreeBSD's C toolchain runs `dtrace -G` over object files to turn probe
//! calls into `nop`s and link in DOF plus `drti.o`, whose constructor hands
//! the DOF to the kernel. That does not fit a Cargo build. Instead each site
//! is emitted as its final instruction plus a record in the `anyprobe_sites`
//! section (see `sites.rs`). A constructor in `.init_array` reads the
//! section, builds DOF with the `dof` crate and passes it to the kernel with
//! the `DTRACEHIOC_ADDDOF` ioctl on `/dev/dtrace/helper`, as `drti.o` does.
//!
//! - A probe site is a `nop` with its arguments in the System V argument
//!   registers, where `fasttrap` reads them. Sites are `readonly` rather than
//!   `nomem`, because the tracer reads memory through pointer arguments.
//! - An is-enabled site is `xor eax, eax`; `fasttrap` emulates it as setting
//!   `eax` to 1 while the probe is enabled.
//! - Each executable or shared library registers its own sites: the
//!   `__start_`/`__stop_` symbols, the constructor and the state below are
//!   per object.

use std::ffi::CStr;
use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::sync::OnceLock;
use std::sync::atomic::AtomicU64;

use crate::error::RegistrationError;
use crate::sites;

pub(crate) const NAME: &str = "freebsd-dtrace";

const HELPER: &str = "/dev/dtrace/helper";

pub(crate) use crate::elf::registry_section;

/// Defines one probe's `enabled` and `fire`. Called by `probes!`.
#[doc(hidden)]
#[macro_export]
macro_rules! __anyprobe_define_probe {
    (
        provider: $provider:literal,
        name: $name:literal,
        params: [$($param:ident: $ty:ty),*],
        sdt: $sdt:literal, [$($sdt_op:tt)*],
        dtrace: {
            probe: $probe:literal,
            is_enabled: $is_enabled:literal,
            stability: $stability:literal,
            typedefs: $typedefs:literal,
            dtrace_name: $dtrace_name:literal,
            function: $function:literal,
            c_types: [$($c_type:literal),*],
            aarch64: [$($areg:tt = ($aval:expr)),*],
            x86_64: [$($xreg:tt = ($xval:expr)),*],
        },
        etw: [$($etw_field:tt)*],
    ) => {
        /// The provider this probe belongs to.
        pub const PROVIDER: &str = $provider;
        /// The probe's name.
        pub const NAME: &str = $name;

        /// Whether a tracer is attached to this probe.
        #[inline(always)]
        #[must_use]
        pub fn enabled() -> bool {
            let enabled: u32;
            // SAFETY: executes `xor eax, eax`, which writes only `eax` and
            // the flags; while the probe is enabled `fasttrap` emulates it
            // as writing 1 to `eax` instead. The record is data in another
            // section.
            unsafe {
                $crate::__anyprobe_site!(
                    "xor eax, eax", "1", $provider, $dtrace_name, $function, [$($c_type),*],
                    lateout("eax") enabled,
                    options(nomem, nostack),
                );
            }
            enabled != 0
        }

        /// Fires the probe. Every inlined copy is a separate probe site.
        #[inline(always)]
        pub fn fire($($param: $ty),*) {
            // SAFETY: executes one `nop`. The arguments sit in the System V
            // argument registers for `fasttrap` to read when the probe is
            // enabled; pointer arguments point at memory that outlives the
            // site, hence `readonly`.
            unsafe {
                $crate::__anyprobe_site!(
                    "nop", "0", $provider, $dtrace_name, $function, [$($c_type),*],
                    $(in($xreg) $xval,)*
                    options(readonly, nostack, preserves_flags),
                );
            }
        }
    };
}

/// Emits one site: `$insn` at a numeric local label, then a record pointing
/// at it in `anyprobe_sites` (layout in `sites.rs`; the version byte is
/// `sites::RECORD_VERSION`). The label is numeric so that every copy of the
/// block, from inlining or monomorphization, is another site rather than a
/// duplicate symbol. The `R` flag (`SHF_GNU_RETAIN`) stops `--gc-sections`
/// from dropping the records, which nothing references; `w` because the
/// dynamic linker relocates each site address.
#[doc(hidden)]
#[macro_export]
macro_rules! __anyprobe_site {
    (
        $insn:literal, $kind:literal, $provider:literal, $name:literal, $function:literal,
        [$($c_type:literal),*], $($operands:tt)*
    ) => {
        ::core::arch::asm!(
            ::core::concat!("990: ", $insn),
            ".pushsection anyprobe_sites, \"awR\", \"progbits\"",
            ".balign 8",
            "991: .4byte 992f-991b",
            ".byte 1",
            ::core::concat!(".byte ", $kind),
            ".2byte 0",
            ".8byte 990b",
            ::core::concat!(".asciz \"", $provider, "\""),
            ::core::concat!(".asciz \"", $name, "\""),
            ::core::concat!(".asciz \"", $function, "\""),
            $(::core::concat!(".asciz \"", $c_type, "\""),)*
            ".balign 8",
            "992: .popsection",
            $($operands)*
        )
    };
}

const _: () = assert!(sites::RECORD_VERSION == 1);

// Keeps `__start_`/`__stop_anyprobe_sites` defined when no site made it into
// the link, and keeps the section writable: the records' flags are "awR",
// and a read-only static here would conflict with them.
#[used]
#[unsafe(link_section = "anyprobe_sites")]
static SITES_ANCHOR: [AtomicU64; 0] = [];

/// The site table of the executable or library this is linked into.
fn site_table() -> &'static [u8] {
    unsafe extern "C" {
        static __start_anyprobe_sites: u8;
        static __stop_anyprobe_sites: u8;
    }
    let start = &raw const __start_anyprobe_sites;
    let stop = &raw const __stop_anyprobe_sites;
    // SAFETY: the linker sets both symbols to the bounds of the
    // `anyprobe_sites` output section, which `SITES_ANCHOR` keeps present,
    // so the range is one allocation of initialized bytes. The dynamic
    // linker has applied every relocation in it before any constructor
    // runs, and nothing writes to it afterwards.
    unsafe { core::slice::from_raw_parts(start, (stop as usize).saturating_sub(start as usize)) }
}

/// A registration the kernel accepted.
struct Registered {
    /// The DOF passed to the kernel. Kept for the life of the object: the
    /// kernel identifies a registration by this buffer's address and refuses
    /// a second one at an address it already holds (`EALREADY`), which a
    /// freed and reused buffer would hit for the next library to register.
    _dof: Vec<u8>,
    /// The generation `DTRACEHIOC_REMOVE` takes, or `None` if there was
    /// nothing to register.
    generation: Option<i32>,
}

static STATE: OnceLock<Result<Registered, RegistrationError>> = OnceLock::new();

fn state() -> &'static Result<Registered, RegistrationError> {
    STATE.get_or_init(register)
}

pub(crate) fn registration() -> Result<(), RegistrationError> {
    state().as_ref().map(|_| ()).map_err(Clone::clone)
}

// Registers at load time, before `main` or before `dlopen` returns. Runs in
// every executable or library that links anyprobe, probes or not.
#[used]
#[unsafe(link_section = ".init_array")]
static INIT: extern "C" fn() = init;

extern "C" fn init() {
    let _ = state();
}

// Unregisters when the object is unloaded, so a `dlclose`d library leaves no
// tracepoints in text that is no longer mapped. At process exit the kernel
// would drop them anyway.
#[used]
#[unsafe(link_section = ".fini_array")]
static FINI: extern "C" fn() = fini;

extern "C" fn fini() {
    if let Some(Ok(Registered {
        generation: Some(generation),
        ..
    })) = STATE.get()
        && let Ok(helper) = open_helper()
    {
        let mut generation = *generation;
        // SAFETY: `DTRACEHIOC_REMOVE` reads one `int`, the generation, from
        // the pointer, which points at a live local.
        unsafe {
            libc::ioctl(helper.as_raw_fd(), DTRACEHIOC_REMOVE, &raw mut generation);
        }
    }
}

fn register() -> Result<Registered, RegistrationError> {
    let sites = sites::parse(site_table())?;
    if sites.is_empty() {
        return Ok(Registered {
            _dof: Vec::new(),
            generation: None,
        });
    }
    let dof = dof::serialize_section(&sites::to_dof(&sites)?);
    let helper = open_helper().map_err(|code| RegistrationError::Open { path: HELPER, code })?;

    // `drti.o` passes the object's load base in `dofhp_addr`, which the
    // kernel adds to the DOF's relocations. This DOF holds run-time site
    // addresses and no relocations, so the base is unused; once registered,
    // the kernel overwrites the field with the DOF's address anyway.
    let mut info = dof::dof_bindings::dof_helper {
        dofhp_mod: module_name(),
        dofhp_addr: dof.as_ptr() as u64,
        dofhp_dof: dof.as_ptr() as u64,
        dofhp_pid: std::process::id() as i32,
        dofhp_gen: 0,
    };
    // SAFETY: `info` is a fully initialized `dof_helper_t` whose `dofhp_dof`
    // points at `dof`, which is kept alive with the registration. The kernel
    // copies the DOF in before returning and writes only `dofhp_gen`.
    let rc = unsafe { libc::ioctl(helper.as_raw_fd(), DTRACEHIOC_ADDDOF, &raw mut info) };
    if rc < 0 {
        return Err(RegistrationError::Refused { code: errno() });
    }
    Ok(Registered {
        _dof: dof,
        generation: Some(info.dofhp_gen),
    })
}

fn open_helper() -> Result<File, i32> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(HELPER)
        .map_err(|e| e.raw_os_error().unwrap_or(libc::EIO))
}

fn errno() -> i32 {
    std::io::Error::last_os_error()
        .raw_os_error()
        .unwrap_or(libc::EIO)
}

/// The file name of the executable or library this is linked into, which
/// DTrace shows as the probe's module.
fn module_name() -> [libc::c_char; 64] {
    let mut out = [0; 64];
    let mut info = core::mem::MaybeUninit::<libc::Dl_info>::zeroed();
    // SAFETY: `SITES_ANCHOR` is a static in this object, so `dladdr` finds
    // the object containing it and fills `info`, whose `dli_fname` then
    // points at a NUL-terminated path owned by the dynamic linker.
    let path = unsafe {
        if libc::dladdr((&raw const SITES_ANCHOR).cast(), info.as_mut_ptr()) == 0 {
            None
        } else {
            let fname = info.assume_init().dli_fname;
            (!fname.is_null()).then(|| CStr::from_ptr(fname).to_bytes())
        }
    };
    let name = path
        .map(|p| p.rsplit(|&b| b == b'/').next().unwrap_or(p))
        .unwrap_or(b"anyprobe");
    for (dst, &src) in out.iter_mut().zip(name.iter().take(63)) {
        *dst = src as libc::c_char;
    }
    out
}

// `_IOWR('z', 3, dof_helper_t)` and `_IOW('z', 2, int)`, from
// `<sys/dtrace.h>` and `<sys/ioccom.h>`'s encoding.
const IOC_IN: libc::c_ulong = 0x8000_0000;
const IOC_INOUT: libc::c_ulong = 0xc000_0000;
const fn ioc(dir: libc::c_ulong, nr: libc::c_ulong, size: usize) -> libc::c_ulong {
    dir | (((size as libc::c_ulong) & 0x1fff) << 16) | ((b'z' as libc::c_ulong) << 8) | nr
}
const DTRACEHIOC_ADDDOF: libc::c_ulong = ioc(
    IOC_INOUT,
    3,
    core::mem::size_of::<dof::dof_bindings::dof_helper>(),
);
const DTRACEHIOC_REMOVE: libc::c_ulong = ioc(IOC_IN, 2, core::mem::size_of::<libc::c_int>());
