//! `probes!`: parsing, validation and expansion.
//!
//! The expansion is target-independent. For each probe it computes everything
//! any backend needs and passes it to `::anyprobe::__private::define_probe!`,
//! whose definition `anyprobe` selects by `cfg` for the target being built.

use std::collections::HashSet;

use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
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

/// Identifier of the per-block provider static the probe modules share.
const PROVIDER_STATIC: &str = "__ANYPROBE_PROVIDER";

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
struct Arg {
    name: Ident,
    ty: TokenStream,
    kind: Kind,
}

/// One validated probe.
pub(crate) struct Probe {
    attrs: Vec<Attribute>,
    vis: syn::Visibility,
    name: Ident,
    args: Vec<Arg>,
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

    // The provider and the probe modules live in one wrapper module, so the
    // probes reach the provider with `super::` wherever the block is: in a
    // module, or in a function body, where `super::` from a nested module
    // skips the function's scope. The probes are re-exported with their
    // declared visibility. The first probe's name keeps the wrapper unique: a
    // second block reusing it would already clash on the re-export.
    let provider_static = format_ident!("{PROVIDER_STATIC}");
    let wrapper = format_ident!("__anyprobe_{}", probes[0].name);
    let definitions = probes
        .iter()
        .map(|p| define_probe(&provider, &provider_static, p));
    let reexports = probes.iter().map(|p| {
        let (vis, name) = (&p.vis, &p.name);
        quote! {
            #[allow(unused_imports)]
            #vis use #wrapper::#name;
        }
    });
    Ok(quote! {
        #[doc(hidden)]
        #[allow(non_snake_case)]
        mod #wrapper {
            #[allow(unused_imports)]
            use super::*;

            ::anyprobe::__private::define_provider!(#provider_static, #provider);
            #(#definitions)*
        }
        #(#reexports)*
    })
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
            name: arg_name,
            kind,
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
        name,
        args,
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

fn define_probe(provider: &str, provider_static: &Ident, probe: &Probe) -> TokenStream {
    let Probe {
        attrs, name, args, ..
    } = probe;
    let name_str = name.to_string();

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
        dtrace_symbols(provider, &name_str, &c_types);
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
        let field = a.name.to_string();
        quote!(#method(#field, (#value), #out))
    });

    quote! {
        #(#attrs)*
        #[allow(non_snake_case)]
        pub mod #name {
            #[allow(unused_imports)]
            use super::*;

            ::anyprobe::__private::define_probe! {
                provider: #provider,
                name: #name_str,
                etw_provider: super::#provider_static,
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
        }
    }
}

#[cfg(test)]
#[path = "probes_tests.rs"]
mod probes_tests;
