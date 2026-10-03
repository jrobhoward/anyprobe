#![allow(non_snake_case)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]

use super::*;

#[test]
fn raw_range____within_the_file____returns_the_bytes() {
    assert_eq!(raw_range(&[1, 2, 3, 4], 1, 2), Some(&[2, 3][..]));
}

#[test]
fn raw_range____empty_at_the_end____returns_nothing_to_read() {
    assert_eq!(raw_range(&[1, 2], 2, 0), Some(&[][..]));
}

#[test]
fn raw_range____past_the_end____is_none() {
    assert_eq!(raw_range(&[1, 2, 3], 2, 2), None);
    assert_eq!(raw_range(&[1, 2, 3], 4, 0), None);
}

#[test]
fn raw_range____start_plus_length_overflows____is_none() {
    assert_eq!(raw_range(&[1, 2, 3], u32::MAX, u32::MAX), None);
}
