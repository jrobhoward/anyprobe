//! Compile-fail tests whose expected output is mostly rustc's own wording: a
//! trait bound the generated code needs and the argument's type lacks.
//!
//! The wording changes between rustc releases, so these are `#[ignore]`d and
//! run on the toolchain pinned in `.github/workflows/ci.yml` (job `ui-rustc`):
//! `cargo +1.99.0 test -p anyprobe-macros --test ui_rustc -- --ignored`.
//! After moving the pin, regenerate with `TRYBUILD=overwrite` on the same
//! command and review the diff. Inputs the macros reject with their own
//! messages are in `ui.rs`.

#![allow(non_snake_case)]

#[test]
#[ignore = "rustc-worded diagnostics; run with --ignored on the pinned toolchain"]
fn probe____trait_bounds_the_argument_lacks____fail_with_rustc_messages() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui_rustc/*.rs");
}
