//! `probes!`: parsing, validation and expansion.
//!
//! The expansion is target-independent. For each probe it computes everything
//! any backend needs and passes it to `::anyprobe::__private::define_probe!`,
//! whose definition `anyprobe` selects by `cfg` for the target being built.

use std::collections::HashSet;

use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote, quote_spanned};
use syn::ext::IdentExt;
use syn::parse::{Parse, ParseStream};
use syn::spanned::Spanned;
use syn::{Attribute, FnArg, ForeignItemFn, Ident, LitStr, Pat, ReturnType, Token};

use crate::args::{self, Kind};
use crate::names;

syn::custom_keyword!(provider);

/// Most operands one probe may pass. macOS on x86-64 passes probe arguments
/// in the six System V integer argument registers.
pub(crate) const MAX_OPERANDS: usize = 6;

const AARCH64_REGS: [&str; MAX_OPERANDS] = ["x0", "x1", "x2", "x3", "x4", "x5"];
const X86_64_REGS: [&str; MAX_OPERANDS] = ["rdi", "rsi", "rdx", "rcx", "r8", "r9"];

/// DTrace's default stability attributes, as `dtrace -h` writes them for a
/// provider with no `#pragma D attributes`.
const STABILITY: &str = "1_1_0_1_1_0_1_1_0_1_1_0_1_1_0";

struct Input {
    provider: Option<LitStr>,
    probes: Vec<ForeignItemFn>,
}

impl Parse for Input {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let provider = if input.peek(provider) {
            input.parse::<provider>()?;
            input.parse::<Token![=]>()?;
            let name: LitStr = input.parse()?;
            input.parse::<Token![;]>()?;
            Some(name)
        } else {
            None
        };
        let mut probes = Vec::new();
        while !input.is_empty() {
            probes.push(input.parse()?);
        }
        Ok(Input { provider, probes })
    }
}

/// One validated argument.
pub(crate) struct Arg {
    /// The parameter name `fire` declares.
    pub(crate) name: Ident,
    /// The ETW field name.
    pub(crate) field: String,
    /// The parameter type `fire` declares.
    pub(crate) ty: TokenStream,
    pub(crate) kind: Kind,
    /// The argument's type in the probe registry: `kind`'s, or for an
    /// encoded value passed as a string, its encoding.
    pub(crate) registry_type: &'static str,
}

/// One validated probe.
pub(crate) struct Probe {
    pub(crate) attrs: Vec<Attribute>,
    pub(crate) vis: syn::Visibility,
    pub(crate) name: Ident,
    pub(crate) args: Vec<Arg>,
    /// What defined the probe, for the registry: `probes`, or for
    /// `#[probe]` `entry`, `return` or `unwind`.
    pub(crate) origin: &'static str,
    /// The function `#[probe]` annotates; empty for `probes!`.
    pub(crate) function: String,
    /// Where the probe was written, for the registry's file and line.
    pub(crate) span: Span,
}

/// Expands a `probes!` invocation.
pub(crate) fn expand(input: TokenStream) -> syn::Result<TokenStream> {
    let input: Input = syn::parse2(input)?;
    let crate_name = std::env::var("CARGO_CRATE_NAME").ok();
    let provider = provider_name(input.provider.as_ref(), crate_name.as_deref())?;

    let mut seen = HashSet::new();
    let mut errors: Option<syn::Error> = None;
    let mut probes = Vec::new();
    for item in input.probes {
        match validate(item) {
            Ok(probe) => {
                if !seen.insert(probe.name.to_string()) {
                    combine(
                        &mut errors,
                        syn::Error::new(probe.name.span(), "duplicate probe name in this block"),
                    );
                }
                probes.push(probe);
            }
            Err(e) => combine(&mut errors, e),
        }
    }
    if let Some(e) = errors {
        return Err(e);
    }
    if probes.is_empty() {
        return Ok(TokenStream::new());
    }

    let definitions = probes.iter().map(|p| define_probe(&provider, p));
    Ok(quote!(#(#definitions)*))
}

fn combine(errors: &mut Option<syn::Error>, e: syn::Error) {
    match errors {
        Some(existing) => existing.combine(e),
        None => *errors = Some(e),
    }
}

/// The provider name: the literal given, or else `crate_name`, the crate being
/// compiled (`CARGO_CRATE_NAME`, set by Cargo while it compiles).
pub(crate) fn provider_name(
    given: Option<&LitStr>,
    crate_name: Option<&str>,
) -> syn::Result<String> {
    let (name, span) = match given {
        Some(lit) => (lit.value(), lit.span()),
        None => match crate_name {
            Some(name) => (name.to_owned(), Span::call_site()),
            None => {
                return Err(syn::Error::new(
                    Span::call_site(),
                    "CARGO_CRATE_NAME is not set; name the provider with `provider = \"...\";`",
                ));
            }
        },
    };
    names::check_provider(&name).map_err(|msg| {
        let hint = if given.is_none() {
            "; the default is the crate name, so set one with `provider = \"...\";`"
        } else {
            ""
        };
        syn::Error::new(span, format!("{msg}{hint}"))
    })?;
    Ok(name)
}

fn is_allowed_attr(attr: &Attribute) -> bool {
    ["doc", "cfg", "cfg_attr", "allow", "expect", "warn", "deny"]
        .iter()
        .any(|name| attr.path().is_ident(name))
}

fn validate(item: ForeignItemFn) -> syn::Result<Probe> {
    let sig = &item.sig;
    let unsupported = |span: Span, what: &str| -> syn::Result<Probe> {
        Err(syn::Error::new(span, format!("probes cannot be {what}")))
    };
    if let Some(t) = &sig.constness {
        return unsupported(t.span(), "`const`");
    }
    if let Some(t) = &sig.asyncness {
        return unsupported(t.span(), "`async`");
    }
    if let Some(t) = &sig.unsafety {
        return unsupported(t.span(), "`unsafe`");
    }
    if let Some(abi) = &sig.abi {
        return unsupported(abi.span(), "`extern`");
    }
    if !sig.generics.params.is_empty() || sig.generics.where_clause.is_some() {
        return unsupported(sig.generics.params.span(), "generic");
    }
    if let Some(v) = &sig.variadic {
        return unsupported(v.span(), "variadic");
    }
    if let ReturnType::Type(_, ty) = &sig.output {
        return Err(syn::Error::new(
            ty.span(),
            "probes return nothing; remove the return type",
        ));
    }
    if let Some(attr) = item.attrs.iter().find(|a| !is_allowed_attr(a)) {
        return Err(syn::Error::new(
            attr.path().span(),
            "only doc, cfg and lint attributes are allowed on a probe",
        ));
    }

    let name = sig.ident.clone();
    names::check_probe(&name.to_string()).map_err(|msg| syn::Error::new(name.span(), msg))?;

    let mut args = Vec::new();
    for input in &sig.inputs {
        let typed = match input {
            FnArg::Typed(t) => t,
            FnArg::Receiver(r) => {
                return Err(syn::Error::new(r.span(), "probes take no `self` argument"));
            }
        };
        let arg_name = match &*typed.pat {
            Pat::Ident(p) if p.by_ref.is_none() && p.mutability.is_none() && p.subpat.is_none() => {
                p.ident.clone()
            }
            pat => {
                return Err(syn::Error::new(
                    pat.span(),
                    "probe arguments must be plain names",
                ));
            }
        };
        let kind = args::classify(&typed.ty).ok_or_else(|| {
            syn::Error::new(
                typed.ty.span(),
                format!(
                    "unsupported probe argument type; expected {}",
                    args::ACCEPTED
                ),
            )
        })?;
        args.push(Arg {
            ty: kind.param_type(&typed.ty),
            field: arg_name.unraw().to_string(),
            name: arg_name,
            kind,
            registry_type: kind.registry_type(),
        });
    }

    let operands: usize = args.iter().map(|a| a.kind.operands(&a.name).len()).sum();
    if operands > MAX_OPERANDS {
        return Err(syn::Error::new(
            sig.paren_token.span.join(),
            format!(
                "this probe passes {operands} values (`&str` and `&[u8]` count as two); \
                 the limit is {MAX_OPERANDS}, because macOS x86-64 passes probe arguments \
                 in six registers"
            ),
        ));
    }

    Ok(Probe {
        attrs: item.attrs,
        vis: item.vis,
        span: name.span(),
        name,
        args,
        origin: "probes",
        function: String::new(),
    })
}

/// The DTrace symbols ld64 recognizes for one probe: probe, is-enabled,
/// stability and typedefs. The same names `dtrace -h` generates for a provider
/// definition with these argument types.
pub(crate) fn dtrace_symbols(provider: &str, probe: &str, c_types: &[&str]) -> [String; 4] {
    let mut probe_sym = format!("__dtrace_probe${provider}${probe}$v1");
    for ty in c_types {
        probe_sym.push('$');
        probe_sym.push_str(&args::hex(ty));
    }
    [
        probe_sym,
        format!("__dtrace_isenabled${provider}${probe}$v1"),
        format!("__dtrace_stability${provider}$v1${STABILITY}"),
        format!("__dtrace_typedefs${provider}$v2"),
    ]
}

/// The SDT argument format: `size@{operand}` per operand, space-separated.
/// `{aN}` is replaced with the operand's register by `asm!`.
pub(crate) fn sdt_format(sizes: &[&str]) -> String {
    sizes
        .iter()
        .enumerate()
        .map(|(i, size)| format!("{size}@{{a{i}}}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// One probe's module, named after the probe.
fn define_probe(provider: &str, probe: &Probe) -> TokenStream {
    define_named_probe(provider, &probe.name.to_string(), probe)
}

/// One probe's module, `probe.name`, for the probe `name_str`:
/// `define_probe!` with every string each backend needs.
pub(crate) fn define_named_probe(provider: &str, name_str: &str, probe: &Probe) -> TokenStream {
    let Probe {
        attrs,
        vis,
        name,
        args,
        ..
    } = probe;

    let params = args.iter().map(|a| {
        let (n, t) = (&a.name, &a.ty);
        quote!(#n: #t)
    });

    let operands: Vec<TokenStream> = args.iter().flat_map(|a| a.kind.operands(&a.name)).collect();
    let sizes: Vec<&str> = args
        .iter()
        .flat_map(|a| a.kind.sdt_sizes().iter().copied())
        .collect();
    let c_types: Vec<&str> = args
        .iter()
        .flat_map(|a| a.kind.c_types().iter().copied())
        .collect();

    let sdt = sdt_format(&sizes);
    let sdt_ops = operands.iter().enumerate().map(|(i, op)| {
        let id = format_ident!("a{i}");
        quote!(#id = (#op))
    });

    let [probe_sym, enabled_sym, stability_sym, typedefs_sym] =
        dtrace_symbols(provider, name_str, &c_types);
    let aarch64 = operands
        .iter()
        .zip(AARCH64_REGS)
        .map(|(op, reg)| quote!(#reg = (#op)));
    let x86_64 = operands
        .iter()
        .zip(X86_64_REGS)
        .map(|(op, reg)| quote!(#reg = (#op)));

    let etw = args.iter().map(|a| {
        let (method, value, out) = a.kind.etw_field(&a.name);
        let method = format_ident!("{method}");
        let out = format_ident!("{out}");
        let field = &a.field;
        quote!(#method(#field, (#value), #out))
    });

    let registry = registry_record(provider, name_str, probe);

    quote! {
        #(#attrs)*
        #[allow(non_snake_case)]
        #vis mod #name {
            #[allow(unused_imports)]
            use super::*;

            ::anyprobe::__private::define_probe! {
                provider: #provider,
                name: #name_str,
                params: [#(#params),*],
                sdt: #sdt, [#(#sdt_ops),*],
                dtrace: {
                    probe: #probe_sym,
                    is_enabled: #enabled_sym,
                    stability: #stability_sym,
                    typedefs: #typedefs_sym,
                    aarch64: [#(#aarch64),*],
                    x86_64: [#(#x86_64),*],
                },
                etw: [#(#etw),*],
            }

            #registry
        }
    }
}

/// The probe's registry record: `register!` with the record's body, the
/// fields `anyprobe::registry` parses, each followed by a NUL. `file!()`,
/// `line!()` and `module_path!()` expand in the caller's crate, at the
/// probe's span.
fn registry_record(provider: &str, name_str: &str, probe: &Probe) -> TokenStream {
    let mut head = String::new();
    for field in [
        REGISTRY_VERSION,
        provider,
        name_str,
        probe.origin,
        &probe.function,
    ] {
        head.push_str(field);
        head.push('\0');
    }
    let mut tail = format!("{}\0", probe.args.len());
    for arg in &probe.args {
        tail.push_str(&arg.field);
        tail.push('\0');
        tail.push_str(arg.registry_type);
        tail.push('\0');
    }
    let span = probe.span;
    quote_spanned! {span=>
        ::anyprobe::__private::register!(::core::concat!(
            #head,
            ::core::module_path!(), "\0",
            ::core::file!(), "\0",
            ::core::line!(), "\0",
            #tail
        ));
    }
}

/// The registry record format version, the first field of every record.
/// `anyprobe::registry` rejects records of any other version.
const REGISTRY_VERSION: &str = "1";

#[cfg(test)]
#[path = "probes_tests.rs"]
mod probes_tests;
