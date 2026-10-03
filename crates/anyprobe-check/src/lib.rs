//! One probe per argument kind, fired from non-generic, non-inlined
//! functions.
//!
//! `cargo check` and `cargo clippy` stop before codegen, so they never
//! assemble an `asm!` template. A library build does reach codegen and needs
//! no linker, so `cargo build -p anyprobe-check --lib --target ...` checks
//! every backend's generated assembly from any host.

anyprobe::probes! {
    provider = "anyprobe_check";

    /// No arguments.
    pub fn empty();
    /// Every unsigned width.
    pub fn unsigned(a: u8, b: u16, c: u32, d: u64, e: usize);
    /// Every signed width.
    pub fn signed(a: i8, b: i16, c: i32, d: i64, e: isize);
    /// Booleans and pointers.
    pub fn flags(on: bool, at: *const u8, to: *mut u64);
    /// Two-operand arguments, at the six-operand limit.
    pub fn slices(text: &str, bytes: &[u8], more: &'static str);
}

/// Fires every probe that is enabled.
#[inline(never)]
pub fn fire_all(text: &str, bytes: &[u8]) {
    if empty::enabled() {
        empty::fire();
    }
    if unsigned::enabled() {
        unsigned::fire(1, 2, 3, 4, 5);
    }
    if signed::enabled() {
        signed::fire(-1, -2, -3, -4, -5);
    }
    if flags::enabled() {
        flags::fire(true, text.as_ptr(), core::ptr::null_mut());
    }
    if slices::enabled() {
        slices::fire(text, bytes, "static");
    }
}

/// Whether any probe here is enabled.
#[inline(never)]
#[must_use]
pub fn any_enabled() -> bool {
    empty::enabled()
        || unsigned::enabled()
        || signed::enabled()
        || flags::enabled()
        || slices::enabled()
}

/// `#[probe]` with native arguments and a native return value.
#[anyprobe::probe(ret = native)]
#[must_use]
pub fn attr_native(id: u64, delta: i32, ok: bool, c: char, text: &str, at: *const u8) -> u64 {
    let _ = (ok, c, at);
    id.wrapping_add(delta as u64) ^ text.len() as u64
}

/// `#[probe]` with `debug` arguments and return value.
#[anyprobe::probe(debug(values), ret = debug)]
#[must_use]
pub fn attr_debug(id: u64, values: &[u16]) -> Vec<u16> {
    let _ = id;
    values.iter().rev().copied().collect()
}

/// `#[probe]` with a `serde` argument.
#[cfg(feature = "serde")]
#[anyprobe::probe(serde(values))]
#[must_use]
pub fn attr_serde(values: &[u32]) -> usize {
    values.len()
}

/// `#[probe]` with more arguments than fit, collapsed into one object.
#[anyprobe::probe(debug(d))]
#[must_use]
pub fn attr_collapsed(a: &str, b: &str, c: &str, d: Option<u8>) -> usize {
    a.len() + b.len() + c.len() + usize::from(d.unwrap_or(0))
}
