//! Tests for the shape of `#[probe]` expansions. What the expansions do is
//! tested through the facade in `anyprobe/tests`; these pin the parts that
//! no test without a tracer attached can observe.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(non_snake_case)]

use super::*;

/// Expands `#[probe(attr)] item` and returns the tokens as one string,
/// without spaces, so the checks do not depend on token spacing.
fn expand_str(attr: &str, item: &str) -> String {
    let out = expand(attr.parse().unwrap(), item.parse().unwrap()).to_string();
    out.split_whitespace().collect()
}

#[test]
fn expand____sync_fn____has_no_invocation_id() {
    let out = expand_str("provider = \"t\"", "fn f(id: u64) -> u64 { id }");
    assert!(!out.contains("next_invocation"), "{out}");
    assert!(out.contains("call_once(move||->u64"), "{out}");
}

#[test]
fn expand____async_fn____draws_an_invocation_id_only_when_entry_is_on() {
    let out = expand_str("provider = \"t\"", "async fn f(id: u64) -> u64 { id }");
    assert!(
        out.contains(
            "if__anyprobe::__anyprobe_entry::enabled(){let__anyprobe_invocation=\
             ::anyprobe::__private::next_invocation();"
        ),
        "{out}"
    );
    assert!(out.contains("}else{0};"), "{out}");
}

#[test]
fn expand____async_fn____passes_the_invocation_id_first() {
    let out = expand_str(
        "provider = \"t\", ret = native",
        "async fn f(id: u64) -> u64 { id }",
    );
    assert!(
        out.contains("fire_entry(__anyprobe_invocation,id)"),
        "{out}"
    );
    assert!(
        out.contains("fire_return(__anyprobe_invocation,__anyprobe_ret)"),
        "{out}"
    );
    assert!(
        out.contains("params:[__anyprobe_invocation:u64,id:u64]"),
        "{out}"
    );
}

#[test]
fn expand____async_fn____awaits_the_body_with_its_return_type_pinned() {
    let out = expand_str(
        "provider = \"t\"",
        "async fn f() -> Box<dyn Debug> { Box::new(1u8) }",
    );
    assert!(
        out.contains("let__anyprobe_never:Box<dynDebug>=loop{};return__anyprobe_never;"),
        "{out}"
    );
    assert!(out.contains(".await;"), "{out}");
}

#[test]
fn expand____async_fn_returning_impl_trait____does_not_pin_the_type() {
    let out = expand_str("provider = \"t\"", "async fn f() -> impl Debug { 1u8 }");
    assert!(!out.contains("__anyprobe_never"), "{out}");
}

#[test]
fn expand____async_fn_at_six_values____collapses_for_the_invocation_id() {
    // Three `&str` take six operands; the invocation id makes seven.
    let out = expand_str(
        "provider = \"t\"",
        "async fn f(a: &str, b: &str, c: &str) -> usize { a.len() + b.len() + c.len() }",
    );
    assert!(out.contains("encode::object"), "{out}");
    let sync = expand_str(
        "provider = \"t\"",
        "fn f(a: &str, b: &str, c: &str) -> usize { a.len() + b.len() + c.len() }",
    );
    assert!(!sync.contains("encode::object"), "{sync}");
}

#[test]
fn expand____unwind_on_sync_fn____arms_and_forgets_a_guard() {
    let out = expand_str("provider = \"t\", unwind", "fn f(id: u64) -> u64 { id }");
    assert!(
        out.contains("let__anyprobe_guard=__anyprobe::Guard;"),
        "{out}"
    );
    assert!(
        out.contains("::core::mem::forget(__anyprobe_guard);"),
        "{out}"
    );
    assert!(out.contains("name:\"f__unwind\""), "{out}");
    assert!(out.contains("pubfnfire_unwind()"), "{out}");
}

#[test]
fn expand____unwind_on_async_fn____passes_invocation_and_panicking() {
    let out = expand_str(
        "provider = \"t\", unwind",
        "async fn f(id: u64) -> u64 { id }",
    );
    assert!(
        out.contains("let__anyprobe_guard=__anyprobe::Guard(__anyprobe_invocation);"),
        "{out}"
    );
    assert!(
        out.contains("fire_unwind(self.0,::anyprobe::__private::panicking())"),
        "{out}"
    );
}

#[test]
fn expand____no_unwind____has_no_guard() {
    let out = expand_str("provider = \"t\"", "fn f(id: u64) -> u64 { id }");
    assert!(!out.contains("Guard"), "{out}");
    assert!(!out.contains("__unwind"), "{out}");
}

#[test]
fn expand____symbol____exports_provider_and_name_without_inlining() {
    let out = expand_str("provider = \"t\", symbol", "fn hit(id: u64) -> u64 { id }");
    assert!(
        out.contains("#[inline(never)]#[unsafe(export_name=\"t__hit\")]"),
        "{out}"
    );
    let named = expand_str(
        "provider = \"t\", name = \"other\", symbol = \"custom.sym$1\"",
        "fn hit(id: u64) -> u64 { id }",
    );
    assert!(named.contains("export_name=\"custom.sym$1\""), "{named}");
}

#[test]
fn expand____symbol_with_lifetimes_only____is_accepted() {
    let out = expand_str(
        "provider = \"t\", symbol",
        "fn first<'a>(x: &'a str) -> &'a str { x }",
    );
    assert!(out.contains("export_name"), "{out}");
    assert!(!out.contains("compile_error"), "{out}");
}
