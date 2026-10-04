//! Generated code under the strictest lint groups a caller might enable.
//!
//! Probe expansions compile in the caller's crate, under the caller's lints.
//! The workspace runs `cargo clippy --all-targets -- -Dwarnings`, so any lint
//! below that an expansion trips fails that gate. The `usdt` crate's
//! expansions trip `clippy::cast_lossless` in callers (its issues #240 and
//! #270); this keeps anyprobe's from doing the same.
//!
//! `unused` is denied, so `cargo test` fails on it too: a body nested as a
//! block inside generated code trips `unused_braces`, but only when the
//! function is written on one line, which rustfmt never produces. The
//! one-line functions below keep that shape under `#[rustfmt::skip]`.

#![deny(unused)]
#![warn(clippy::pedantic, clippy::nursery, clippy::cast_lossless)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(non_snake_case)]

use std::ffi::CStr;

anyprobe::probes! {
    provider = "anyprobe_lints";

    /// Every unsigned width.
    pub fn unsigned_ints(a: u8, b: u16, c: u32, d: u64, e: usize);
    /// Every signed width.
    pub fn signed_ints(a: i8, b: i16, c: i32, d: i64, e: isize);
    /// The other one-value kinds.
    pub fn scalars(on: bool, c: char, p: *const u8, label: &CStr);
    /// Two-value kinds.
    pub fn slices(text: &str, bytes: &[u8], last: u8);
    /// A `'static` borrow.
    pub fn fixed(more: &'static str);
    /// Optional two-value kinds.
    pub fn optional(name: Option<&str>, key: Option<&[u8]>);
    /// No arguments.
    pub fn empty();
}

#[derive(Debug)]
struct Opts {
    verbose: bool,
}

#[anyprobe::probe(provider = "anyprobe_lints", debug(opts), ret = native)]
fn attributed(id: u32, small: u8, text: &str, opts: &Opts, name: Option<&str>) -> u16 {
    u16::from(small)
        + u16::from(opts.verbose)
        + u16::try_from(id).unwrap_or(0)
        + u16::try_from(text.len() + name.map_or(0, str::len)).unwrap_or(0)
}

#[anyprobe::probe(provider = "anyprobe_lints", unwind)]
async fn attributed_async(id: u64) -> u64 {
    id
}

#[rustfmt::skip]
#[anyprobe::probe(provider = "anyprobe_lints", ret = native)]
async fn one_line_async(id: u64) -> u64 { id }

#[rustfmt::skip]
#[anyprobe::probe(provider = "anyprobe_lints")]
async fn one_line_async_opaque(id: u64) -> impl std::fmt::Debug { id }

#[rustfmt::skip]
#[anyprobe::probe(provider = "anyprobe_lints")]
async fn one_line_async_unit() {}

/// # Safety
///
/// None needed; `unsafe` only changes how the body is wrapped.
#[rustfmt::skip]
#[anyprobe::probe(provider = "anyprobe_lints", ret = native)]
async unsafe fn one_line_async_unsafe(id: u64) -> u64 { id }

#[rustfmt::skip]
#[anyprobe::probe(provider = "anyprobe_lints", ret = native)]
fn one_line(id: u64) -> u64 { id }

#[rustfmt::skip]
#[anyprobe::probe(provider = "anyprobe_lints")]
fn one_line_opaque(id: u64) -> impl std::fmt::Debug { id }

#[rustfmt::skip]
#[anyprobe::probe(provider = "anyprobe_lints")]
fn one_line_unit() {}

#[test]
fn probe____one_line_bodies____compile_cleanly() {
    drop(one_line_async(1));
    drop(one_line_async_opaque(1));
    drop(one_line_async_unit());
    // SAFETY: the function has no safety requirements.
    drop(unsafe { one_line_async_unsafe(1) });
    assert_eq!(one_line(1), 1);
    drop(one_line_opaque(1));
    one_line_unit();
}

#[test]
fn probes____every_kind_under_strict_lints____compiles_cleanly() {
    let value = 5u8;
    anyprobe::fire!(unsigned_ints(1, 2, 3, 4, 5));
    anyprobe::fire!(signed_ints(-1, -2, -3, -4, -5));
    anyprobe::fire!(scalars(true, 'x', &raw const value, c"label"));
    anyprobe::fire!(slices("text", b"bytes", 5));
    anyprobe::fire!(fixed("more"));
    anyprobe::fire!(optional(Some("name"), None));
    anyprobe::fire!(empty());
    if unsigned_ints::enabled() {
        unsigned_ints::fire(1, 2, 3, 4, 5);
    }
    let opts = Opts { verbose: true };
    assert_eq!(attributed(1, 2, "ab", &opts, None), 6);
    drop(attributed_async(1));
}
