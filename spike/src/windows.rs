//! Windows: ETW TraceLogging.
//!
//! One provider, `anyprobe.spike`, with one event per probe. The provider GUID
//! is derived from the name by the standard TraceLogging name hash, so it is
//! the same in every build. `write_event!` checks the provider's level and
//! keyword mask before evaluating any field, which is the enabled check.

use tracelogging as tlg;

pub(crate) const NAME: &str = "windows-etw";

tlg::define_provider!(PROVIDER, "anyprobe.spike");

/// The provider name, for starting a session against it.
pub(crate) const PROVIDER_NAME: &str = "anyprobe.spike";

pub(crate) fn register() -> std::io::Result<()> {
    // SAFETY: `register` requires the provider to be unregistered before the
    // module that holds it unloads. The provider lives in the executable, which
    // only unloads at process exit.
    let rc = unsafe { PROVIDER.register() };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::from_raw_os_error(rc as i32))
    }
}

/// The provider's GUID, formatted for `logman` and `tracelog`.
pub(crate) fn provider_guid() -> String {
    format!("{{{}}}", PROVIDER.id().to_utf8_bytes().escape_ascii())
}

#[inline(always)]
pub(crate) fn entry_enabled() -> bool {
    PROVIDER.enabled(tlg::Level::Verbose, 0)
}

#[inline(always)]
pub(crate) fn return_enabled() -> bool {
    PROVIDER.enabled(tlg::Level::Verbose, 0)
}

#[inline(always)]
pub(crate) fn fire_entry(id: u64, label: &str) {
    tlg::write_event!(
        PROVIDER,
        "work__entry",
        level(Verbose),
        u64("id", &id),
        str8("label", label),
    );
}

#[inline(always)]
pub(crate) fn fire_return(id: u64, result: u64) {
    tlg::write_event!(
        PROVIDER,
        "work__return",
        level(Verbose),
        u64("id", &id),
        u64("result", &result),
    );
}
