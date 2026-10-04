//! Classification of probe argument types, and what each backend needs for
//! each class.
//!
//! Types are recognized by syntax: the macro has to know an argument's shape
//! (how many operands, signed or not, which C type and which ETW field) to
//! build the probe's metadata, and type information is not available to a
//! procedural macro.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Ident, Type};

/// The shapes an argument can take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// `u8`..`u64`, `usize`; the name is the Rust type.
    Unsigned(&'static str),
    /// `i8`..`i64`, `isize`; the name is the Rust type.
    Signed(&'static str),
    Bool,
    Char,
    /// `*const T` or `*mut T`.
    Pointer,
    /// `&str`.
    Str,
    /// `&[u8]`.
    Bytes,
    /// `Option<&str>`: like `&str`, with a null pointer and length 0 for
    /// `None`.
    OptStr,
    /// `Option<&[u8]>`: like `&[u8]`, with a null pointer and length 0 for
    /// `None`.
    OptBytes,
    /// `&CStr`: one pointer to NUL-terminated bytes.
    CStr,
}

const UNSIGNED: [&str; 5] = ["u8", "u16", "u32", "u64", "usize"];
const SIGNED: [&str; 5] = ["i8", "i16", "i32", "i64", "isize"];

/// The types the macro accepts, for error messages.
pub(crate) const ACCEPTED: &str = "u8, u16, u32, u64, usize, i8, i16, i32, i64, isize, bool, \
     char, *const T, *mut T, &str, &[u8], Option<&str>, Option<&[u8]> or &CStr";

fn single_ident(ty: &Type) -> Option<String> {
    match ty {
        Type::Path(p) if p.qself.is_none() && p.path.segments.len() == 1 => {
            let seg = &p.path.segments[0];
            seg.arguments.is_none().then(|| seg.ident.to_string())
        }
        Type::Group(g) => single_ident(&g.elem),
        Type::Paren(p) => single_ident(&p.elem),
        _ => None,
    }
}

/// Classifies `ty`, or returns `None` if it is not an accepted type.
pub(crate) fn classify(ty: &Type) -> Option<Kind> {
    if let Some(name) = single_ident(ty) {
        if let Some(n) = UNSIGNED.iter().find(|n| **n == name) {
            return Some(Kind::Unsigned(n));
        }
        if let Some(n) = SIGNED.iter().find(|n| **n == name) {
            return Some(Kind::Signed(n));
        }
        return match name.as_str() {
            "bool" => Some(Kind::Bool),
            "char" => Some(Kind::Char),
            _ => None,
        };
    }
    match ty {
        Type::Ptr(_) => Some(Kind::Pointer),
        Type::Reference(r) if r.mutability.is_none() => match &*r.elem {
            Type::Slice(s) if single_ident(&s.elem).as_deref() == Some("u8") => Some(Kind::Bytes),
            elem if single_ident(elem).as_deref() == Some("str") => Some(Kind::Str),
            elem if is_cstr(elem) => Some(Kind::CStr),
            _ => None,
        },
        Type::Path(_) => match option_of(ty).and_then(classify) {
            Some(Kind::Str) => Some(Kind::OptStr),
            Some(Kind::Bytes) => Some(Kind::OptBytes),
            _ => None,
        },
        Type::Group(g) => classify(&g.elem),
        Type::Paren(p) => classify(&p.elem),
        _ => None,
    }
}

/// `ty` without the invisible groups and parentheses around it.
fn unwrap_group(ty: &Type) -> &Type {
    match ty {
        Type::Group(g) => unwrap_group(&g.elem),
        Type::Paren(p) => unwrap_group(&p.elem),
        _ => ty,
    }
}

/// The `T` of `Option<T>`, spelled `Option` or with its full path through
/// `core` or `std`.
fn option_of(ty: &Type) -> Option<&Type> {
    let Type::Path(p) = ty else { return None };
    if p.qself.is_some() || !path_is(&p.path, "option", "Option") {
        return None;
    }
    let syn::PathArguments::AngleBracketed(a) = &p.path.segments.last()?.arguments else {
        return None;
    };
    match a.args.iter().collect::<Vec<_>>().as_slice() {
        [syn::GenericArgument::Type(t)] => Some(t),
        _ => None,
    }
}

/// Whether `ty` is `CStr`, spelled `CStr` or with its full path through
/// `core::ffi` or `std::ffi`.
fn is_cstr(ty: &Type) -> bool {
    match ty {
        Type::Path(p) => {
            p.qself.is_none()
                && p.path.segments.iter().all(|s| s.arguments.is_none())
                && path_is(&p.path, "ffi", "CStr")
        }
        Type::Group(g) => is_cstr(&g.elem),
        Type::Paren(p) => is_cstr(&p.elem),
        _ => false,
    }
}

/// Whether `path` is `name` alone, or `core::module::name` or
/// `std::module::name`, with or without a leading `::`. Generic arguments
/// on the last segment are not compared.
fn path_is(path: &syn::Path, module: &str, name: &str) -> bool {
    let idents: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
    match idents.as_slice() {
        [last] => path.leading_colon.is_none() && last == name,
        [root, m, last] => (root == "core" || root == "std") && m == module && last == name,
        _ => false,
    }
}

/// Whether `ty` names the same type wherever it is written: it mentions only
/// primitive types and paths that start with `::` or `crate`. A probe module
/// writes its parameter types inside itself, so a type that is not
/// self-contained (`*const Request`) needs the caller's names imported.
pub(crate) fn is_self_contained(ty: &Type) -> bool {
    const PRIMITIVES: [&str; 7] = ["bool", "char", "str", "f32", "f64", "u128", "i128"];
    match ty {
        Type::Path(p) if p.qself.is_none() => {
            let segments = &p.path.segments;
            let rooted = p.path.leading_colon.is_some()
                || segments
                    .first()
                    .is_some_and(|s| s.ident == "crate" || s.ident == "$crate");
            let primitive = segments.len() == 1
                && segments.first().is_some_and(|s| {
                    let name = s.ident.to_string();
                    UNSIGNED.contains(&name.as_str())
                        || SIGNED.contains(&name.as_str())
                        || PRIMITIVES.contains(&name.as_str())
                });
            (rooted || primitive)
                && segments.iter().all(|s| match &s.arguments {
                    syn::PathArguments::None => true,
                    syn::PathArguments::AngleBracketed(a) => a.args.iter().all(|arg| match arg {
                        syn::GenericArgument::Type(t) => is_self_contained(t),
                        syn::GenericArgument::Lifetime(_) => true,
                        _ => false,
                    }),
                    syn::PathArguments::Parenthesized(_) => false,
                })
        }
        Type::Ptr(p) => is_self_contained(&p.elem),
        Type::Reference(r) => is_self_contained(&r.elem),
        Type::Slice(s) => is_self_contained(&s.elem),
        Type::Array(a) => matches!(&a.len, syn::Expr::Lit(_)) && is_self_contained(&a.elem),
        Type::Tuple(t) => t.elems.iter().all(is_self_contained),
        Type::Never(_) => true,
        Type::Group(g) => is_self_contained(&g.elem),
        Type::Paren(p) => is_self_contained(&p.elem),
        _ => false,
    }
}

impl Kind {
    /// The parameter type `fire` declares. References lose any lifetime, so
    /// the generated signature needs no generic parameters; `Option`s are
    /// written with their full path; other types keep the caller's spelling.
    pub(crate) fn param_type(self, declared: &Type) -> TokenStream {
        match self {
            Kind::Str => quote!(&str),
            Kind::Bytes => quote!(&[u8]),
            Kind::OptStr => quote!(::core::option::Option<&str>),
            Kind::OptBytes => quote!(::core::option::Option<&[u8]>),
            // The caller's spelling, so an import of `CStr` is used.
            Kind::CStr => match unwrap_group(declared) {
                Type::Reference(r) => {
                    let elem = &r.elem;
                    quote!(&#elem)
                }
                _ => quote!(#declared),
            },
            _ => quote!(#declared),
        }
    }

    /// One expression per operand passed to the probe site on Linux and
    /// macOS. Integers are widened to 64 bits, so every operand fills one
    /// register and the SDT format and C types are the same on every
    /// architecture.
    pub(crate) fn operands(self, arg: &Ident) -> Vec<TokenStream> {
        match self {
            Kind::Unsigned(_) | Kind::Bool | Kind::Char => vec![quote!(#arg as u64)],
            Kind::Signed(_) => vec![quote!(#arg as i64)],
            Kind::Pointer => vec![quote!(#arg as usize as u64)],
            Kind::Str | Kind::Bytes => vec![quote!(#arg.as_ptr()), quote!(#arg.len() as u64)],
            Kind::OptStr | Kind::OptBytes => vec![
                quote!(match #arg {
                    ::core::option::Option::Some(v) => v.as_ptr(),
                    ::core::option::Option::None => ::core::ptr::null(),
                }),
                quote!(match #arg {
                    ::core::option::Option::Some(v) => v.len() as u64,
                    ::core::option::Option::None => 0,
                }),
            ],
            Kind::CStr => vec![quote!(#arg.as_ptr())],
        }
    }

    /// The SDT argument size for each operand: `8` unsigned, `-8` signed.
    pub(crate) fn sdt_sizes(self) -> &'static [&'static str] {
        match self {
            Kind::Signed(_) => &["-8"],
            Kind::Str | Kind::Bytes | Kind::OptStr | Kind::OptBytes => &["8", "8"],
            _ => &["8"],
        }
    }

    /// The C type DTrace records for each operand.
    pub(crate) fn c_types(self) -> &'static [&'static str] {
        match self {
            Kind::Unsigned(_) | Kind::Bool | Kind::Char => &["uint64_t"],
            Kind::Signed(_) => &["int64_t"],
            Kind::Pointer => &["uintptr_t"],
            Kind::Str | Kind::OptStr => &["char *", "uint64_t"],
            Kind::Bytes | Kind::OptBytes => &["uint8_t *", "uint64_t"],
            Kind::CStr => &["char *"],
        }
    }

    /// The argument's type in the probe registry: how its value reaches the
    /// tracer. Integers keep their Rust width, which ETW uses.
    pub(crate) fn registry_type(self) -> &'static str {
        match self {
            Kind::Unsigned(n) | Kind::Signed(n) => n,
            Kind::Bool => "bool",
            Kind::Char => "char",
            Kind::Pointer => "ptr",
            Kind::Str => "str",
            Kind::Bytes => "bytes",
            Kind::OptStr => "opt_str",
            Kind::OptBytes => "opt_bytes",
            Kind::CStr => "cstr",
        }
    }

    /// The `tracelogging_dynamic::EventBuilder` method, value expression and
    /// `OutType` variant for the argument's one ETW field.
    pub(crate) fn etw_field(self, arg: &Ident) -> (&'static str, TokenStream, &'static str) {
        match self {
            Kind::Unsigned("usize") => ("add_u64", quote!(#arg as u64), "Default"),
            Kind::Unsigned(n) => (etw_method(n), quote!(#arg), "Default"),
            Kind::Signed("isize") => ("add_i64", quote!(#arg as i64), "Default"),
            Kind::Signed(n) => (etw_method(n), quote!(#arg), "Default"),
            Kind::Bool => ("add_bool32", quote!(#arg as i32), "Default"),
            Kind::Char => ("add_u32", quote!(#arg as u32), "Default"),
            Kind::Pointer => ("add_u64", quote!(#arg as usize as u64), "Hex"),
            Kind::Str => ("add_str8", quote!(#arg), "Utf8"),
            Kind::Bytes => ("add_binary", quote!(#arg), "Default"),
            // ETW has no absent string: `None` is recorded as an empty one.
            Kind::OptStr => ("add_str8", quote!(#arg.unwrap_or("")), "Utf8"),
            Kind::OptBytes => ("add_binary", quote!(#arg.unwrap_or(&[])), "Default"),
            Kind::CStr => ("add_str8", quote!(#arg.to_bytes()), "Utf8"),
        }
    }
}

fn etw_method(rust_type: &str) -> &'static str {
    match rust_type {
        "u8" => "add_u8",
        "u16" => "add_u16",
        "u32" => "add_u32",
        "i8" => "add_i8",
        "i16" => "add_i16",
        "i32" => "add_i32",
        "i64" => "add_i64",
        _ => "add_u64",
    }
}

/// Hex encoding of a C type name, as ld64 expects in a probe symbol.
pub(crate) fn hex(c_type: &str) -> String {
    c_type.bytes().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
#[path = "args_tests.rs"]
mod args_tests;
