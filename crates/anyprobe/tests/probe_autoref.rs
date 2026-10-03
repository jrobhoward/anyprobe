//! `#[probe]` with the `autoref` feature: unlisted arguments that are not
//! native pick `Serialize`, then `Debug`.

#![cfg(feature = "autoref")]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(non_snake_case)]

use std::fmt::Debug;

use anyprobe::__private::Value;

#[derive(Debug, serde::Serialize)]
struct Both {
    n: u32,
}

#[derive(Debug)]
struct DebugOnly;

#[derive(serde::Serialize)]
struct SerializeOnly;

fn select<T: Debug + serde::Serialize>(value: &T) -> Value<'_> {
    anyprobe::__private::unlisted!("value", value)
}

fn select_debug<T: Debug>(value: &T) -> Value<'_> {
    anyprobe::__private::unlisted!("value", value)
}

#[test]
fn unlisted____serialize_and_debug____is_serde() {
    assert!(matches!(select(&Both { n: 1 }), Value::Serde(_)));
}

#[test]
fn unlisted____debug_only____is_debug() {
    let v = anyprobe::__private::unlisted!("v", &DebugOnly);
    assert!(matches!(v, Value::Debug(_)));
}

#[test]
fn unlisted____serialize_only____is_serde() {
    let v = anyprobe::__private::unlisted!("v", &SerializeOnly);
    assert!(matches!(v, Value::Serde(_)));
}

#[test]
fn unlisted____generic_with_debug_bound____uses_the_declared_bound() {
    // `Both` is `Serialize`, but the function only declares `Debug`.
    assert!(matches!(select_debug(&Both { n: 1 }), Value::Debug(_)));
}

#[anyprobe::probe(ret = debug)]
fn handle(id: u64, both: Both, debug_only: DebugOnly, ser: SerializeOnly) -> u32 {
    let _ = (debug_only, ser);
    both.n + id as u32
}

#[anyprobe::probe]
fn generic<T: Debug>(t: T) -> T {
    t
}

#[test]
fn probe____unlisted_arguments____compile_and_behave() {
    assert_eq!(handle(1, Both { n: 2 }, DebugOnly, SerializeOnly), 3);
    assert_eq!(generic(5), 5);
}
