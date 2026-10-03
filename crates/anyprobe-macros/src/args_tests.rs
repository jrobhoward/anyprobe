//! Tests for argument classification and per-backend metadata.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(non_snake_case)]

use super::*;
use syn::parse_quote;

fn kind(ty: Type) -> Option<Kind> {
    classify(&ty)
}

#[test]
fn classify____integers____keep_their_rust_name() {
    assert_eq!(kind(parse_quote!(u16)), Some(Kind::Unsigned("u16")));
    assert_eq!(kind(parse_quote!(usize)), Some(Kind::Unsigned("usize")));
    assert_eq!(kind(parse_quote!(i8)), Some(Kind::Signed("i8")));
    assert_eq!(kind(parse_quote!(isize)), Some(Kind::Signed("isize")));
}

#[test]
fn classify____bool_and_pointers____are_accepted() {
    assert_eq!(kind(parse_quote!(bool)), Some(Kind::Bool));
    assert_eq!(kind(parse_quote!(*const Foo)), Some(Kind::Pointer));
    assert_eq!(kind(parse_quote!(*mut u8)), Some(Kind::Pointer));
}

#[test]
fn classify____str_with_or_without_lifetime____is_str() {
    assert_eq!(kind(parse_quote!(&str)), Some(Kind::Str));
    assert_eq!(kind(parse_quote!(&'static str)), Some(Kind::Str));
    assert_eq!(kind(parse_quote!(&'a str)), Some(Kind::Str));
}

#[test]
fn classify____byte_slice____is_bytes() {
    assert_eq!(kind(parse_quote!(&[u8])), Some(Kind::Bytes));
}

#[test]
fn classify____unsupported_types____are_rejected() {
    assert_eq!(kind(parse_quote!(u128)), None);
    assert_eq!(kind(parse_quote!(char)), None);
    assert_eq!(kind(parse_quote!(f64)), None);
    assert_eq!(kind(parse_quote!(String)), None);
    assert_eq!(kind(parse_quote!(&mut str)), None);
    assert_eq!(kind(parse_quote!(&[u16])), None);
    assert_eq!(kind(parse_quote!(std::primitive::u64)), None);
    assert_eq!(kind(parse_quote!(Option<u64>)), None);
}

#[test]
fn operands____str____are_pointer_then_length() {
    let arg: Ident = parse_quote!(path);
    let ops: Vec<String> = Kind::Str
        .operands(&arg)
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(ops, ["path . as_ptr ()", "path . len () as u64"]);
    assert_eq!(Kind::Str.sdt_sizes(), ["8", "8"]);
    assert_eq!(Kind::Str.c_types(), ["char *", "uint64_t"]);
}

#[test]
fn operands____signed____widen_to_i64() {
    let arg: Ident = parse_quote!(x);
    let ops: Vec<String> = Kind::Signed("i16")
        .operands(&arg)
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(ops, ["x as i64"]);
    assert_eq!(Kind::Signed("i16").sdt_sizes(), ["-8"]);
    assert_eq!(Kind::Signed("i16").c_types(), ["int64_t"]);
}

#[test]
fn etw_field____each_kind____uses_the_matching_builder_method() {
    let arg: Ident = parse_quote!(v);
    let method = |k: Kind| k.etw_field(&arg).0;
    assert_eq!(method(Kind::Unsigned("u8")), "add_u8");
    assert_eq!(method(Kind::Unsigned("u32")), "add_u32");
    assert_eq!(method(Kind::Unsigned("usize")), "add_u64");
    assert_eq!(method(Kind::Signed("i64")), "add_i64");
    assert_eq!(method(Kind::Signed("isize")), "add_i64");
    assert_eq!(method(Kind::Bool), "add_bool32");
    assert_eq!(Kind::Pointer.etw_field(&arg).2, "Hex");
    let (m, value, out) = Kind::Str.etw_field(&arg);
    assert_eq!(
        (m, value.to_string().as_str(), out),
        ("add_str8", "v", "Utf8")
    );
    assert_eq!(method(Kind::Bytes), "add_binary");
}

#[test]
fn hex____c_types____match_dtrace_h_output() {
    // From `dtrace -h` on macOS 27.
    assert_eq!(hex("uint64_t"), "75696e7436345f74");
    assert_eq!(hex("char *"), "63686172202a");
    assert_eq!(hex("uint8_t *"), "75696e74385f74202a");
    assert_eq!(hex("int64_t"), "696e7436345f74");
    assert_eq!(hex("uintptr_t"), "75696e747074725f74");
}
