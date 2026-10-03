//! Hand-written probes for each anyprobe backend.
//!
//! Each backend exposes the same four functions for the two probes
//! `spike:work__entry(u64 id, char *label, u64 len)` and
//! `spike:work__return(u64 id, u64 result)`. Enabled checks are inlined into
//! every caller, so inlining and monomorphization produce several check sites
//! per probe. Firing happens in a `#[cold]` helper, which is where encoding
//! will live once the macro exists.

#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod backend;

#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod backend;

#[cfg(target_os = "freebsd")]
#[path = "freebsd.rs"]
mod backend;

#[cfg(windows)]
#[path = "windows.rs"]
mod backend;

#[cfg(not(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "freebsd",
    windows
)))]
#[path = "noop.rs"]
mod backend;

#[cfg(any(target_os = "freebsd", test))]
mod dof_records;

/// Name of the backend compiled for this target.
pub const BACKEND: &str = backend::NAME;

/// Registers the probes with the OS, where the backend needs it.
///
/// Linux and macOS need nothing at runtime. FreeBSD hands DOF to the kernel;
/// Windows registers the ETW provider. Calling it more than once does nothing
/// after the first call.
///
/// # Errors
///
/// Returns the OS error if registration fails. The probes then stay disabled;
/// the program is otherwise unaffected.
pub fn register() -> std::io::Result<()> {
    backend::register()
}

/// The ETW provider name and GUID, for starting a session against it.
#[cfg(windows)]
#[must_use]
pub fn etw_provider() -> (&'static str, String) {
    (backend::PROVIDER_NAME, backend::provider_guid())
}

/// Whether a tracer currently has `work__entry` enabled.
#[must_use]
#[inline]
pub fn entry_enabled() -> bool {
    backend::entry_enabled()
}

/// The probed function, inlined into every caller so each call site carries
/// its own enabled checks.
#[must_use]
#[inline(always)]
pub fn work(id: u64, label: &str) -> u64 {
    if backend::entry_enabled() {
        fire_entry(id, label);
    }
    let result = compute(id, label);
    if backend::return_enabled() {
        fire_return(id, result);
    }
    result
}

/// A generic probed function. Each instantiation is a separate copy of the
/// checks and of the cold helper that fires the probe.
#[must_use]
#[inline(never)]
pub fn work_generic<T: Into<u64> + Copy>(id: T) -> u64 {
    if backend::entry_enabled() {
        fire_entry_generic(id);
    }
    let result = compute(id.into(), "generic");
    if backend::return_enabled() {
        fire_return(id.into(), result);
    }
    result
}

/// `work` behind a call, for benchmarking against [`baseline`].
#[must_use]
#[inline(never)]
pub fn work_outlined(id: u64, label: &str) -> u64 {
    work(id, label)
}

/// The same computation as [`work_outlined`] with no probes.
#[must_use]
#[inline(never)]
pub fn baseline(id: u64, label: &str) -> u64 {
    compute(id, label)
}

#[inline(always)]
fn compute(id: u64, label: &str) -> u64 {
    id.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ label.len() as u64
}

#[cold]
#[inline(never)]
fn fire_entry(id: u64, label: &str) {
    backend::fire_entry(id, label);
}

#[cold]
#[inline(never)]
fn fire_entry_generic<T: Into<u64> + Copy>(id: T) {
    backend::fire_entry(id.into(), std::any::type_name::<T>());
}

#[cold]
#[inline(never)]
fn fire_return(id: u64, result: u64) {
    backend::fire_return(id, result);
}
