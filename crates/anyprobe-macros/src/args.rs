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
}

const UNSIGNED: [&str; 5] = ["u8", "u16", "u32", "u64", "usize"];
const SIGNED: [&str; 5] = ["i8", "i16", "i32", "i64", "isize"];

/// The types the macro accepts, for error messages.
pub(crate) const ACCEPTED: &str = "u8, u16, u32, u64, usize, i8, i16, i32, i64, isize, bool, \
     char, *const T, *mut T, &str or &[u8]";

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
            _ => None,
        },
        Type::Group(g) => classify(&g.elem),
        Type::Paren(p) => classify(&p.elem),
        _ => None,
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
    /// the generated signature needs no generic parameters; other types keep
    /// the caller's spelling.
    pub(crate) fn param_type(self, declared: &Type) -> TokenStream {
        match self {
            Kind::Str => quote!(&str),
            Kind::Bytes => quote!(&[u8]),
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
        }
    }

    /// The SDT argument size for each operand: `8` unsigned, `-8` signed.
    pub(crate) fn sdt_sizes(self) -> &'static [&'static str] {
        match self {
            Kind::Signed(_) => &["-8"],
            Kind::Str | Kind::Bytes => &["8", "8"],
            _ => &["8"],
        }
    }

    /// The C type DTrace records for each operand.
    pub(crate) fn c_types(self) -> &'static [&'static str] {
        match self {
            Kind::Unsigned(_) | Kind::Bool | Kind::Char => &["uint64_t"],
            Kind::Signed(_) => &["int64_t"],
            Kind::Pointer => &["uintptr_t"],
            Kind::Str => &["char *", "uint64_t"],
            Kind::Bytes => &["uint8_t *", "uint64_t"],
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
