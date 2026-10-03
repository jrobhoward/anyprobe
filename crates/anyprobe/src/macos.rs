//! macOS: DTrace USDT through ld64.
//!
//! ld64 treats calls to `__dtrace_probe$provider$probe$v1$<types>` and
//! `__dtrace_isenabled$provider$probe$v1` as probe sites. It replaces each
//! probe call with `nop`s and each is-enabled call with an instruction that
//! zeroes the return register, and builds the DOF for the provider into a
//! `__TEXT,__dof_<provider>` section. dyld registers that DOF at load time, so
//! nothing runs at startup.
//!
//! `probes!` computes the symbol names (what `dtrace -h` would generate,
//! argument C types hex-encoded) and the argument registers for both
//! architectures.
//!
//! Every site is an `asm!` call. ld64 rewrites call instructions in place, so
//! a site must be a real call followed by more code in the same function. A
//! Rust-level call that ends a function compiles to a tail call (a branch with
//! no return after it); once rewritten to a `nop`, execution would fall off
//! the end of the function into whatever follows. A call inside `asm!` is
//! never a tail call.

pub(crate) const NAME: &str = "macos-dtrace";

/// Defines one probe's symbols, `enabled` and `fire`. Called by `probes!`.
#[doc(hidden)]
#[macro_export]
macro_rules! __anyprobe_define_probe {
    (
        provider: $provider:literal,
        name: $name:literal,
        etw_provider: $etw:path,
        params: [$($param:ident: $ty:ty),*],
        sdt: $sdt:literal, [$($sdt_op:tt)*],
        dtrace: {
            probe: $probe:literal,
            is_enabled: $is_enabled:literal,
            stability: $stability:literal,
            typedefs: $typedefs:literal,
            aarch64: [$($areg:tt = ($aval:expr)),*],
            x86_64: [$($xreg:tt = ($xval:expr)),*],
        },
        etw: [$($etw_field:tt)*],
    ) => {
        /// The provider this probe belongs to.
        pub const PROVIDER: &str = $provider;
        /// The probe's name.
        pub const NAME: &str = $name;

        // Only referenced through `sym`; the signatures do not matter.
        unsafe extern "C" {
            #[link_name = $probe]
            fn __anyprobe_probe();
            #[link_name = $is_enabled]
            fn __anyprobe_is_enabled();
            #[link_name = $stability]
            fn __anyprobe_stability();
            #[link_name = $typedefs]
            fn __anyprobe_typedefs();
        }

        /// Whether a tracer is attached to this probe.
        #[inline(always)]
        #[must_use]
        pub fn enabled() -> bool {
            $crate::__anyprobe_is_enabled!(__anyprobe_is_enabled)
        }

        /// Fires the probe. Every inlined copy is a separate probe site.
        #[inline(always)]
        pub fn fire($($param: $ty),*) {
            #[cfg(target_arch = "aarch64")]
            $crate::__anyprobe_probe_call!(
                __anyprobe_probe, __anyprobe_typedefs, __anyprobe_stability,
                $($areg = $aval),*
            );
            #[cfg(target_arch = "x86_64")]
            $crate::__anyprobe_probe_call!(
                __anyprobe_probe, __anyprobe_typedefs, __anyprobe_stability,
                $($xreg = $xval),*
            );
        }
    };
}

/// Nothing to register on macOS: dyld registers the DOF ld64 built.
#[doc(hidden)]
#[macro_export]
macro_rules! __anyprobe_define_provider {
    ($ident:ident, $name:literal) => {};
}

/// An is-enabled check as a bare call instruction rather than a Rust call.
///
/// ld64 rewrites the call into a single instruction that zeroes the return
/// register, so the only register the site ever writes is the result (plus
/// the link register on AArch64, which `bl` would write). A Rust-level call
/// would make the compiler spill every caller-saved register around a site
/// that, once linked, is not a call.
#[cfg(target_arch = "aarch64")]
#[doc(hidden)]
#[macro_export]
macro_rules! __anyprobe_is_enabled {
    ($sym:path) => {{
        let enabled: u32;
        // SAFETY: ld64 replaces this `bl` with an instruction that writes
        // only `w0`; an unresolved symbol is a link error, so no build reaches
        // a real call. `x30` is declared clobbered for the `bl` encoding.
        unsafe {
            ::core::arch::asm!(
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
#[doc(hidden)]
#[macro_export]
macro_rules! __anyprobe_is_enabled {
    ($sym:path) => {{
        let enabled: u32;
        // SAFETY: ld64 replaces this `call` with `xor %eax, %eax` and padding;
        // an unresolved symbol is a link error, so no build reaches a real
        // call.
        unsafe {
            ::core::arch::asm!(
                "call {f}",
                f = sym $sym,
                lateout("eax") enabled,
                options(nomem, preserves_flags),
            );
        }
        enabled != 0
    }};
}

/// A probe call as an `asm!` call instruction; see the module docs for why it
/// must not be a Rust call. The `.reference` directives after it are what
/// `dtrace -h` headers emit.
#[cfg(target_arch = "aarch64")]
#[doc(hidden)]
#[macro_export]
macro_rules! __anyprobe_probe_call {
    ($sym:path, $typedefs:path, $stability:path, $($reg:tt = $val:expr),*) => {
        // SAFETY: ld64 replaces this `bl` with a `nop`; an unresolved symbol is
        // a link error, so no build reaches a real call. When DTrace enables
        // the probe it reads the argument registers and memory they point to,
        // hence `readonly`. The C clobbers cover the `bl` encoding.
        unsafe {
            ::core::arch::asm!(
                "bl {f}",
                ".reference {typedefs}",
                ".reference {stability}",
                f = sym $sym,
                typedefs = sym $typedefs,
                stability = sym $stability,
                $(in($reg) $val,)*
                clobber_abi("C"),
                options(readonly, nostack),
            );
        }
    };
}

#[cfg(target_arch = "x86_64")]
#[doc(hidden)]
#[macro_export]
macro_rules! __anyprobe_probe_call {
    ($sym:path, $typedefs:path, $stability:path, $($reg:tt = $val:expr),*) => {
        // SAFETY: as for the AArch64 variant; `call` is replaced with `nop`s.
        unsafe {
            ::core::arch::asm!(
                "call {f}",
                ".reference {typedefs}",
                ".reference {stability}",
                f = sym $sym,
                typedefs = sym $typedefs,
                stability = sym $stability,
                $(in($reg) $val,)*
                clobber_abi("C"),
                options(readonly),
            );
        }
    };
}
