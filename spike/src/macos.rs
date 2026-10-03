//! macOS: DTrace USDT through ld64.
//!
//! ld64 treats calls to `__dtrace_probe$provider$probe$v1$<types>` and
//! `__dtrace_isenabled$provider$probe$v1` as probe sites. It replaces each
//! probe call with `nop`s and each is-enabled call with an instruction that
//! zeroes the return register, and builds the DOF for the provider into a
//! `__TEXT,__dof_<provider>` section. dyld registers that DOF at load time, so
//! nothing runs at startup.
//!
//! The symbol names are what `dtrace -h` generates for the provider
//! definition; argument C types are hex-encoded and joined with `$`. The
//! stability and typedefs symbols only need to be referenced.
//!
//! Every site is an `asm!` call. ld64 rewrites call instructions in place, so
//! a site must be a real call followed by more code in the same function,
//! never a tail call; see `probe_call!`.

use core::ffi::c_char;

pub(crate) const NAME: &str = "macos-dtrace";

unsafe extern "C" {
    #[link_name = "__dtrace_stability$spike$v1$1_1_0_1_1_0_1_1_0_1_1_0_1_1_0"]
    fn stability();
    #[link_name = "__dtrace_typedefs$spike$v2"]
    fn typedefs();

    #[link_name = "__dtrace_isenabled$spike$work__entry$v1"]
    fn work_entry_isenabled() -> i32;
    // uint64_t, char *, uint64_t
    #[link_name = "__dtrace_probe$spike$work__entry$v1$75696e7436345f74$63686172202a$75696e7436345f74"]
    fn work_entry_probe(id: u64, label: *const c_char, len: u64);

    #[link_name = "__dtrace_isenabled$spike$work__return$v1"]
    fn work_return_isenabled() -> i32;
    // uint64_t, uint64_t
    #[link_name = "__dtrace_probe$spike$work__return$v1$75696e7436345f74$75696e7436345f74"]
    fn work_return_probe(id: u64, result: u64);
}

/// An is-enabled check as a bare call instruction rather than a Rust call.
///
/// ld64 rewrites the call into a single instruction that zeroes the return
/// register, so the only register the site ever writes is the result (plus
/// the link register on AArch64, which `bl` would write). A Rust-level call
/// would make the compiler spill every caller-saved register around a site
/// that, once linked, is not a call.
#[cfg(target_arch = "aarch64")]
macro_rules! is_enabled {
    ($sym:path) => {{
        let enabled: u32;
        // SAFETY: ld64 replaces this `bl` with an instruction that writes
        // only `w0`; an unresolved symbol is a link error, so no build reaches
        // a real call. `x30` is declared clobbered for the `bl` encoding.
        unsafe {
            core::arch::asm!(
                "bl {f}",
                f = sym $sym,
                lateout("w0") enabled,
                out("x30") _,
                options(nomem, nostack, preserves_flags),
            );
        }
        enabled != 0
    }};
}

#[cfg(target_arch = "x86_64")]
macro_rules! is_enabled {
    ($sym:path) => {{
        let enabled: u32;
        // SAFETY: ld64 replaces this `call` with `xor %eax, %eax` and padding;
        // an unresolved symbol is a link error, so no build reaches a real
        // call.
        unsafe {
            core::arch::asm!(
                "call {f}",
                f = sym $sym,
                lateout("eax") enabled,
                options(nomem, preserves_flags),
            );
        }
        enabled != 0
    }};
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
compile_error!("the macOS DTrace backend supports x86_64 and aarch64 only");

/// A probe call as an `asm!` call instruction rather than a Rust call.
///
/// ld64 replaces the call with a `nop`. A Rust-level call that ends a
/// function compiles to a tail call (`b`, no `ret`), and once ld64 turns that
/// into a `nop`, execution falls off the end of the function into whatever
/// follows it. A call inside `asm!` is never turned into a tail call. The
/// `.reference` directives after it are what `dtrace -h` headers emit, and
/// serve the same purpose there.
#[cfg(target_arch = "aarch64")]
macro_rules! probe_call {
    ($sym:path, $($reg:tt = $val:expr),*) => {
        // SAFETY: ld64 replaces this `bl` with a `nop`; an unresolved symbol is
        // a link error, so no build reaches a real call. When DTrace enables
        // the probe it reads the argument registers and memory they point to,
        // hence `readonly`. The C clobbers cover the `bl` encoding.
        unsafe {
            core::arch::asm!(
                "bl {f}",
                ".reference {typedefs}",
                ".reference {stability}",
                f = sym $sym,
                typedefs = sym typedefs,
                stability = sym stability,
                $(in($reg) $val,)*
                clobber_abi("C"),
                options(readonly, nostack),
            );
        }
    };
}

#[cfg(target_arch = "x86_64")]
macro_rules! probe_call {
    ($sym:path, $($reg:tt = $val:expr),*) => {
        // SAFETY: as for the AArch64 variant; `call` is replaced with `nop`s.
        unsafe {
            core::arch::asm!(
                "call {f}",
                ".reference {typedefs}",
                ".reference {stability}",
                f = sym $sym,
                typedefs = sym typedefs,
                stability = sym stability,
                $(in($reg) $val,)*
                clobber_abi("C"),
                options(readonly),
            );
        }
    };
}

pub(crate) fn register() -> std::io::Result<()> {
    Ok(())
}

#[inline(always)]
pub(crate) fn entry_enabled() -> bool {
    is_enabled!(work_entry_isenabled)
}

#[inline(always)]
pub(crate) fn return_enabled() -> bool {
    is_enabled!(work_return_isenabled)
}

#[inline(always)]
pub(crate) fn fire_entry(id: u64, label: &str) {
    let ptr: *const c_char = label.as_ptr().cast();
    let len = label.len() as u64;
    #[cfg(target_arch = "aarch64")]
    probe_call!(work_entry_probe, "x0" = id, "x1" = ptr, "x2" = len);
    #[cfg(target_arch = "x86_64")]
    probe_call!(work_entry_probe, "rdi" = id, "rsi" = ptr, "rdx" = len);
}

#[inline(always)]
pub(crate) fn fire_return(id: u64, result: u64) {
    #[cfg(target_arch = "aarch64")]
    probe_call!(work_return_probe, "x0" = id, "x1" = result);
    #[cfg(target_arch = "x86_64")]
    probe_call!(work_return_probe, "rdi" = id, "rsi" = result);
}
