//! Targets with no supported tracer, and every target under
//! `--cfg anyprobe_noop`: probes compile to nothing, `enabled()` is `false`
//! and `fire` does nothing.

pub(crate) const NAME: &str = "noop";

/// Nothing registers at runtime on this target.
#[allow(clippy::unnecessary_wraps)]
pub(crate) fn registration() -> Result<(), crate::RegistrationError> {
    Ok(())
}

/// Registry records: none on this target. Called by `probes!`.
#[doc(hidden)]
#[macro_export]
macro_rules! __anyprobe_register {
    ($body:expr) => {};
}

/// The registry section: empty on this target.
pub(crate) fn registry_section() -> &'static [u8] {
    &[]
}

/// Defines one probe as a no-op. Called by `probes!`.
#[doc(hidden)]
#[macro_export]
macro_rules! __anyprobe_define_probe {
    (
        provider: $provider:literal,
        name: $name:literal,
        params: [$($param:ident: $ty:ty),*],
        sdt: $sdt:literal, [$($sdt_op:tt)*],
        dtrace: { $($dtrace:tt)* },
        etw: [$($etw_field:tt)*],
    ) => {
        /// The provider this probe belongs to.
        pub const PROVIDER: &str = $provider;
        /// The probe's name.
        pub const NAME: &str = $name;

        /// Whether a tracer is attached to this probe: never, on this target.
        #[inline(always)]
        #[must_use]
        pub fn enabled() -> bool {
            false
        }

        /// Fires the probe: does nothing on this target.
        #[inline(always)]
        #[allow(unused_variables)]
        pub fn fire($($param: $ty),*) {}
    };
}
