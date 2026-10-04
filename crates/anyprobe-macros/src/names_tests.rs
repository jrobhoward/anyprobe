//! Tests for provider and probe name validation.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(non_snake_case)]

use super::*;

#[test]
fn check_provider____identifier____is_accepted() {
    assert_eq!(check_provider("my_app"), Ok(()));
    assert_eq!(check_provider("_x"), Ok(()));
    assert_eq!(check_provider("App2x"), Ok(()));
}

#[test]
fn check_provider____trailing_digit____is_rejected() {
    let err = check_provider("http2").unwrap_err();
    assert!(err.contains("must not end with a digit"), "{err}");
}

#[test]
fn check_provider____leading_digit____is_rejected() {
    assert!(check_provider("2fast").is_err());
}

#[test]
fn check_provider____hyphen____is_rejected() {
    assert!(check_provider("my-app").is_err());
}

#[test]
fn check_provider____empty____is_rejected() {
    assert!(check_provider("").is_err());
}

#[test]
fn check_provider____at_limit____is_accepted() {
    assert_eq!(check_provider(&"p".repeat(MAX_PROVIDER_LEN)), Ok(()));
}

#[test]
fn check_provider____over_limit____is_rejected() {
    let err = check_provider(&"p".repeat(MAX_PROVIDER_LEN + 1)).unwrap_err();
    assert!(err.contains("appends the pid"), "{err}");
}

#[test]
fn check_probe____double_underscore____is_accepted() {
    assert_eq!(check_probe("work__entry"), Ok(()));
}

#[test]
fn check_probe____trailing_digit____is_accepted() {
    assert_eq!(check_probe("stage2"), Ok(()));
}

#[test]
fn check_probe____non_ascii____is_rejected() {
    assert!(check_probe("wörk").is_err());
}

#[test]
fn check_probe____over_limit____is_rejected() {
    assert!(check_probe(&"p".repeat(MAX_PROBE_LEN + 1)).is_err());
    assert_eq!(check_probe(&"p".repeat(MAX_PROBE_LEN)), Ok(()));
}

#[test]
fn check_provider____d_reserved_word____is_rejected() {
    let err = check_provider("string").unwrap_err();
    assert!(err.contains("reserved in DTrace's D language"), "{err}");
    assert!(check_provider("for").is_err());
}

#[test]
fn check_probe____d_reserved_word____is_rejected() {
    for name in ["int", "signed", "unsigned", "probe", "uint64_t", "xlate"] {
        let err = check_probe(name).unwrap_err();
        assert!(err.contains("reserved in DTrace's D language"), "{err}");
    }
}

#[test]
fn check_probe____d_reserved_word_as_part____is_accepted() {
    assert_eq!(check_probe("int__entry"), Ok(()));
    assert_eq!(check_probe("signed_ints"), Ok(()));
    assert_eq!(check_probe("Int"), Ok(()));
}

#[test]
fn d_reserved____list____is_sorted_without_duplicates() {
    assert!(D_RESERVED.windows(2).all(|w| w[0] < w[1]));
}
