//! Targets with no supported tracer: every probe compiles to nothing.

pub(crate) const NAME: &str = "noop";

pub(crate) fn register() -> std::io::Result<()> {
    Ok(())
}

#[inline(always)]
pub(crate) fn entry_enabled() -> bool {
    false
}

#[inline(always)]
pub(crate) fn return_enabled() -> bool {
    false
}

#[inline(always)]
pub(crate) fn fire_entry(_id: u64, _label: &str) {}

#[inline(always)]
pub(crate) fn fire_return(_id: u64, _result: u64) {}
