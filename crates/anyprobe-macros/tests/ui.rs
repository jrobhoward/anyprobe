//! Compile-fail tests: every input `probes!` rejects, and its message.
//!
//! Regenerate the expected output after an intended change with
//! `TRYBUILD=overwrite cargo test -p anyprobe-macros --test ui`, then review
//! the diff.

#![allow(non_snake_case)]

#[test]
fn probes____rejected_inputs____fail_with_their_messages() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/*.rs");
}
