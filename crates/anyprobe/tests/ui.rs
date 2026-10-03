//! Compile-fail tests for `#[probe]` whose result depends on this crate's
//! features: each directory runs only under the feature set it describes.
//! Inputs rejected whatever the features are tested in `anyprobe-macros`.
//!
//! Regenerate the expected output after an intended change with
//! `TRYBUILD=overwrite cargo test -p anyprobe --test ui`, once per feature
//! set (`--no-default-features`, `--features autoref`), then review the diff.

#![allow(non_snake_case)]

#[test]
fn probe____feature_dependent_inputs____fail_with_their_messages() {
    let t = trybuild::TestCases::new();
    if cfg!(feature = "autoref") {
        t.compile_fail("tests/ui/autoref/*.rs");
    } else {
        t.compile_fail("tests/ui/no-autoref/*.rs");
    }
    if cfg!(feature = "serde") {
        t.compile_fail("tests/ui/serde/*.rs");
    } else {
        t.compile_fail("tests/ui/no-serde/*.rs");
    }
}
