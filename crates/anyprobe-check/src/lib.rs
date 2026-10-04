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
    pub fn unsigned_ints(a: u8, b: u16, c: u32, d: u64, e: usize);
    /// Every signed width.
    pub fn signed_ints(a: i8, b: i16, c: i32, d: i64, e: isize);
    /// Booleans and pointers.
    pub fn flags(on: bool, at: *const u8, to: *mut u64);
    /// Two-operand arguments, at the five-operand limit.
    pub fn slices(text: &str, bytes: &[u8], last: u8);
    /// A `'static` borrow.
    pub fn fixed(more: &'static str);
    /// Optional strings and bytes, and a C string.
    pub fn optional(name: Option<&str>, key: Option<&[u8]>, label: &core::ffi::CStr);
}

/// Fires every probe that is enabled.
#[inline(never)]
pub fn fire_all(text: &str, bytes: &[u8]) {
    if empty::enabled() {
        empty::fire();
    }
    if unsigned_ints::enabled() {
        unsigned_ints::fire(1, 2, 3, 4, 5);
    }
    if signed_ints::enabled() {
        signed_ints::fire(-1, -2, -3, -4, -5);
    }
    if flags::enabled() {
        flags::fire(true, text.as_ptr(), core::ptr::null_mut());
    }
    if slices::enabled() {
        slices::fire(text, bytes, 5);
    }
    if fixed::enabled() {
        fixed::fire("static");
    }
    if optional::enabled() {
        optional::fire(Some(text), None, c"label");
    }
    #[cfg(all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ))]
    foreign_sdt_site();
}

/// An SDT site written as `sys/sdt.h`, the `usdt` crate and the `probe` crate
/// write theirs: `.note.stapsdt` with no flags, `.stapsdt.base` as `"aG"`, no
/// semaphore and no arguments. Inlined into [`fire_all`], so it shares an
/// object with anyprobe's sites, which carry other flags on the same section
/// names; the assembler rejects that unless the sections stay apart.
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
#[inline(always)]
fn foreign_sdt_site() {
    // SAFETY: the block executes a single `nop` and otherwise only emits data
    // into non-executed sections. It has no operands.
    unsafe {
        core::arch::asm!(
            "990: nop",
            ".pushsection .note.stapsdt, \"\", \"note\"",
            ".balign 4",
            ".4byte 992f-991f, 994f-993f, 3",
            "991: .asciz \"stapsdt\"",
            "992: .balign 4",
            "993: .8byte 990b",
            ".8byte _.stapsdt.base",
            ".8byte 0",
            ".asciz \"foreign\"",
            ".asciz \"site\"",
            ".asciz \"\"",
            "994: .balign 4",
            ".popsection",
            ".ifndef _.stapsdt.base",
            ".pushsection .stapsdt.base, \"aG\", \"progbits\", .stapsdt.base, comdat",
            ".weak _.stapsdt.base",
            ".hidden _.stapsdt.base",
            "_.stapsdt.base: .space 1",
            ".size _.stapsdt.base, 1",
            ".popsection",
            ".endif",
            options(nomem, nostack, preserves_flags),
        );
    }
}

/// Whether any probe here is enabled.
#[inline(never)]
#[must_use]
pub fn any_enabled() -> bool {
    empty::enabled()
        || unsigned_ints::enabled()
        || signed_ints::enabled()
        || flags::enabled()
        || slices::enabled()
        || fixed::enabled()
        || optional::enabled()
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

/// `#[probe]` with optional strings and bytes and a C string, passed
/// natively.
#[anyprobe::probe(ret = native)]
#[must_use]
pub fn attr_optional(name: Option<&str>, key: Option<&[u8]>, label: &core::ffi::CStr) -> u64 {
    (name.map_or(0, str::len) + key.map_or(0, <[u8]>::len) + label.to_bytes().len()) as u64
}
