//! Compile-fail tests for `#[probe]` whose expected output is mostly rustc's
//! own wording: a trait bound the generated code needs and the argument's type
//! lacks. Each directory runs only under the feature set it describes.
//!
//! The wording changes between rustc releases, so these are `#[ignore]`d and
//! run on the toolchain pinned in `.github/workflows/ci.yml` (job `ui-rustc`):
//! `cargo +1.99.0 test -p anyprobe --test ui_rustc -- --ignored`, once per
//! feature set (default, `--features autoref`). After moving the pin,
//! regenerate with `TRYBUILD=overwrite` on the same commands and review the
//! diff. Inputs rejected with this crate's own messages are in `ui.rs`.

#![allow(non_snake_case)]

#[test]
#[ignore = "rustc-worded diagnostics; run with --ignored on the pinned toolchain"]
fn probe____trait_bounds_the_argument_lacks____fail_with_rustc_messages() {
    let t = trybuild::TestCases::new();
    if cfg!(feature = "autoref") {
        t.compile_fail("tests/ui_rustc/autoref/*.rs");
    }
    if cfg!(feature = "serde") {
        t.compile_fail("tests/ui_rustc/serde/*.rs");
    }
}
