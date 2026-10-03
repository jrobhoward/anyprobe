//! `#[probe]`: parsing, validation and expansion.
//!
//! The attribute wraps a function in an entry probe and a return probe, and
//! with `unwind` an unwind probe. Each is an ordinary `define_probe!` module,
//! defined inside the function body so that it needs no names from outside
//! it, next to a `#[cold]` helper that encodes the arguments and fires. The
//! function itself gains the enabled checks and runs its original body as a
//! closure (`call_once`), or for an `async fn` as an awaited `async` block,
//! to capture the return value.
//!
//! An `async fn` passes an invocation id as the first argument of each probe,
//! so a tracer can pair entries and returns of calls that interleave.

use std::collections::HashMap;

use proc_macro2::{Span, TokenStream, TokenTree};
use quote::{format_ident, quote, quote_spanned};
use syn::ext::IdentExt;
use syn::spanned::Spanned;
use syn::{AttrStyle, FnArg, GenericParam, Ident, ItemFn, LitStr, Pat, ReturnType, Token, Type};

use crate::args::{self, Kind};
use crate::names;
use crate::probes::{self, Arg, MAX_OPERANDS, Probe};

/// How an argument, or the return value, reaches the probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Native,
    Serde,
    Debug,
    Skip,
}

impl Mode {
    fn option_name(self) -> &'static str {
        match self {
            Mode::Native => "native",
            Mode::Serde => "serde",
            Mode::Debug => "debug",
            Mode::Skip => "skip",
        }
    }
}

#[derive(Default)]
struct Options {
    name: Option<LitStr>,
    provider: Option<LitStr>,
    /// Argument name to its listed mode and where it was listed.
    modes: HashMap<String, (Mode, Span)>,
    ret: Option<(Mode, Span)>,
    /// Where `unwind` was given.
    unwind: Option<Span>,
    /// `symbol`, with its name if one was given, and where it was given.
    symbol: Option<(Option<LitStr>, Span)>,
}

const OPTIONS: &str = "expected `name`, `provider`, `serde(..)`, `debug(..)`, `skip(..)`, \
     `native(..)`, `ret`, `unwind` or `symbol`";

fn parse_options(attr: TokenStream) -> syn::Result<Options> {
    let mut options = Options::default();
    let parser = syn::meta::parser(|meta| {
        let path = &meta.path;
        let mode = [Mode::Native, Mode::Serde, Mode::Debug, Mode::Skip]
            .into_iter()
            .find(|m| path.is_ident(m.option_name()));
        if let Some(mode) = mode {
            return meta.parse_nested_meta(|arg| {
                let Some(ident) = arg.path.get_ident() else {
                    return Err(arg.error("expected an argument name"));
                };
                let name = ident.unraw().to_string();
                let span = ident.span();
                if let Some((previous, _)) = options.modes.insert(name.clone(), (mode, span)) {
                    return Err(syn::Error::new(
                        span,
                        format!(
                            "`{name}` is already listed in `{}(..)`",
                            previous.option_name()
                        ),
                    ));
                }
                Ok(())
            });
        }
        if path.is_ident("name") || path.is_ident("provider") {
            let slot = if path.is_ident("name") {
                &mut options.name
            } else {
                &mut options.provider
            };
            if slot.is_some() {
                return Err(meta.error("given twice"));
            }
            *slot = Some(meta.value()?.parse()?);
            return Ok(());
        }
        if path.is_ident("ret") {
            if options.ret.is_some() {
                return Err(meta.error("given twice"));
            }
            let value: Ident = meta.value()?.parse()?;
            let mode = match value.to_string().as_str() {
                "native" => Mode::Native,
                "serde" => Mode::Serde,
                "debug" => Mode::Debug,
                _ => {
                    return Err(syn::Error::new(
                        value.span(),
                        "expected `ret = native`, `ret = serde` or `ret = debug`",
                    ));
                }
            };
            options.ret = Some((mode, value.span()));
            return Ok(());
        }
        if path.is_ident("unwind") {
            if options.unwind.is_some() {
                return Err(meta.error("given twice"));
            }
            options.unwind = Some(path.span());
            return Ok(());
        }
        if path.is_ident("symbol") {
            if options.symbol.is_some() {
                return Err(meta.error("given twice"));
            }
            let name = if meta.input.peek(Token![=]) {
                Some(meta.value()?.parse()?)
            } else {
                None
            };
            options.symbol = Some((name, path.span()));
            return Ok(());
        }
        Err(meta.error(format!("unknown option; {OPTIONS}")))
    });
    syn::parse::Parser::parse2(parser, attr)?;
    Ok(options)
}

/// Expands `#[probe(attr)] item`. On error the item is emitted unchanged
/// after the error, so that one mistake does not also report every caller
/// of the function as broken.
pub(crate) fn expand(attr: TokenStream, item: TokenStream) -> TokenStream {
    let func: ItemFn = match syn::parse2(item.clone()) {
        Ok(f) => f,
        Err(e) => {
            let e = syn::Error::new(e.span(), "`#[probe]` applies to a function with a body");
            return quote!(#item)
                .into_iter()
                .chain(e.to_compile_error())
                .collect();
        }
    };
    match expand_fn(attr, func) {
        Ok(tokens) => tokens,
        Err(e) => {
            let e = e.to_compile_error();
            quote!(#e #item)
        }
    }
}

/// An argument the entry probe passes.
struct Passed {
    /// The name as written, for errors and the ETW field.
    field: String,
    /// The helper's parameter name.
    param: Ident,
    shape: Shape,
}

enum Shape {
    /// Passed as itself. `expr` evaluates, at the call site, to the value of
    /// the probe parameter's type.
    Native { kind: Kind, expr: TokenStream },
    /// Passed as an encoded `Value`. `expr` builds it at the call site;
    /// `encoding` is its registry type (`json`, `debug` or `auto`).
    Encoded {
        expr: TokenStream,
        encoding: &'static str,
    },
}

fn expand_fn(attr: TokenStream, mut func: ItemFn) -> syn::Result<TokenStream> {
    let mut options = parse_options(attr)?;
    check_signature(&func, &options)?;
    let is_async = func.sig.asyncness.is_some();

    let crate_name = std::env::var("CARGO_CRATE_NAME").ok();
    let provider = probes::provider_name(options.provider.as_ref(), crate_name.as_deref())?;
    let (base, base_span) = match &options.name {
        Some(lit) => (lit.value(), lit.span()),
        None => (func.sig.ident.unraw().to_string(), func.sig.ident.span()),
    };
    let entry_name = format!("{base}__entry");
    let return_name = format!("{base}__return");
    let unwind_name = format!("{base}__unwind");
    let mut checked = vec![&entry_name, &return_name];
    if options.unwind.is_some() {
        checked.push(&unwind_name);
    }
    for name in checked {
        names::check_probe(name).map_err(|msg| {
            let hint = if options.name.is_none() {
                "; set a shorter one with `name = \"...\"`"
            } else {
                ""
            };
            syn::Error::new(base_span, format!("{msg}{hint}"))
        })?;
    }
    let symbol = symbol_name(&options, &provider, &base)?;

    let passed = collect_args(&func, &mut options)?;
    let ret = return_shape(&func, options.ret)?;

    // The invocation id of an `async fn` takes one operand of its own.
    let operands: usize = usize::from(is_async)
        + passed
            .iter()
            .map(|p| match &p.shape {
                Shape::Native { kind, .. } => kind.sdt_sizes().len(),
                Shape::Encoded { .. } => 2,
            })
            .sum::<usize>();
    let collapse = operands > MAX_OPERANDS;

    let entry_ident = format_ident!("__anyprobe_entry");
    let return_ident = format_ident!("__anyprobe_return");
    let unwind_ident = format_ident!("__anyprobe_unwind");
    let site = Site {
        function: func.sig.ident.unraw().to_string(),
        span: func.sig.ident.span(),
    };
    let (entry_probe, entry_helper, entry_call) = entry_parts(
        &provider,
        &entry_name,
        &entry_ident,
        &passed,
        collapse,
        is_async,
        &site,
    );
    let (return_probe, return_helper, return_call) =
        return_parts(&provider, &return_name, &return_ident, ret, is_async, &site);
    let (unwind_probe, unwind_helper) = if options.unwind.is_some() {
        unwind_parts(&provider, &unwind_name, &unwind_ident, is_async, &site)
    } else {
        (TokenStream::new(), TokenStream::new())
    };

    // Inner attributes of the body (`#![allow(..)]`) move onto the function,
    // where they apply to the same code.
    for attr in &mut func.attrs {
        attr.style = AttrStyle::Outer;
    }
    let ItemFn {
        attrs,
        vis,
        sig,
        block,
    } = &func;

    // An `unsafe fn` body may call unsafe code directly (before edition
    // 2024); inside a closure or an `async` block that needs an `unsafe`
    // block.
    let body = if sig.unsafety.is_some() {
        quote!({ unsafe #block })
    } else {
        quote!(#block)
    };
    let private = quote!(::anyprobe::__private);

    // With `unwind`, a guard that fires the unwind probe if it is dropped
    // before the body returns: a panic, or for an `async fn` the future
    // dropped before completion. On return it is forgotten, so the normal
    // path runs no drop code.
    let (guard, disarm) = match (options.unwind.is_some(), is_async) {
        (false, _) => (TokenStream::new(), TokenStream::new()),
        (true, false) => (quote!(let __anyprobe_guard = __anyprobe::Guard;), forget()),
        (true, true) => (
            quote!(let __anyprobe_guard = __anyprobe::Guard(__anyprobe_invocation);),
            forget(),
        ),
    };

    let run = if is_async {
        // The entry probe fires at the first poll, which is when an `async
        // fn` body starts. The invocation id is drawn only if it is on.
        let start = quote! {
            let __anyprobe_invocation: u64 = if __anyprobe::#entry_ident::enabled() {
                let __anyprobe_invocation = #private::next_invocation();
                #entry_call
                __anyprobe_invocation
            } else {
                0
            };
        };
        // The body runs in an `async move` block, awaited here, so that its
        // `return`s end the block rather than skip the return probe. A block
        // takes its output type from its first `return`; the unreachable one
        // first pins it to the declared type, so later `return`s coerce to it
        // (`Box<u8>` to `Box<dyn Debug>`) as they would in the `async fn`.
        // `impl Trait` cannot be written there; the type is then inferred.
        let pin = match &sig.output {
            ReturnType::Type(_, ty) if !contains_impl(ty) => quote! {
                // The `if false` and its `loop {}` are dead on purpose: they
                // only name the type, so the caller's lints are told so.
                #[allow(unreachable_code, clippy::empty_loop, clippy::diverging_sub_expression)]
                if false {
                    let __anyprobe_never: #ty = loop {};
                    return __anyprobe_never;
                }
            },
            _ => TokenStream::new(),
        };
        quote! {
            #start
            #guard
            let __anyprobe_ret = async move {
                #pin
                #body
            }
            .await;
            #disarm
        }
    } else {
        // The closure is annotated with the return type so that `?` in the
        // body converts errors as it does in the function. `impl Trait`
        // cannot be written there; the type is then inferred.
        let ret_annotation = match &sig.output {
            ReturnType::Type(arrow, ty) if !contains_impl(ty) => quote!(#arrow #ty),
            _ => TokenStream::new(),
        };
        quote! {
            if __anyprobe::#entry_ident::enabled() {
                #entry_call
            }
            #guard
            // The body moves into a closure that is called once; in the
            // caller's crate that can trip these two on code the caller wrote.
            #[allow(unused_unsafe, clippy::redundant_closure_call)]
            let __anyprobe_ret = #private::call_once(move || #ret_annotation #body);
            #disarm
        }
    };

    // `symbol`: a stable, unmangled symbol for raw uprobes and DTrace's `pid`
    // provider, and `inline(never)` so that the symbol is where the code
    // runs.
    let symbol_attrs = symbol.map(|name| {
        quote! {
            #[inline(never)]
            #[unsafe(export_name = #name)]
        }
    });

    Ok(quote! {
        #(#attrs)*
        #symbol_attrs
        #vis #sig {
            // Generated, so the caller's documentation, naming and style
            // lints do not apply to it (probe modules are lower-case names
            // with `__` in them).
            #[doc(hidden)]
            #[allow(
                dead_code,
                missing_docs,
                non_snake_case,
                unreachable_pub,
                clippy::all,
                clippy::pedantic,
                clippy::nursery
            )]
            mod __anyprobe {
                #entry_probe
                #return_probe
                #unwind_probe
                #entry_helper
                #return_helper
                #unwind_helper
            }
            #run
            if __anyprobe::#return_ident::enabled() {
                #return_call
            }
            __anyprobe_ret
        }
    })
}

/// Forgets the unwind guard once the body has returned.
fn forget() -> TokenStream {
    quote! {
        // Forgetting is the point: the guard's `Drop` is the unwind probe,
        // and it must not run on a normal return.
        #[allow(clippy::mem_forget)]
        ::core::mem::forget(__anyprobe_guard);
    }
}

/// The `symbol` name, if `symbol` was given: the one given, or
/// `{provider}__{base}`.
fn symbol_name(options: &Options, provider: &str, base: &str) -> syn::Result<Option<LitStr>> {
    let Some((given, span)) = &options.symbol else {
        return Ok(None);
    };
    let (name, span) = match given {
        Some(lit) => (lit.value(), lit.span()),
        None => (format!("{provider}__{base}"), *span),
    };
    let mut chars = name.chars();
    let valid = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '$'));
    if !valid {
        return Err(syn::Error::new(
            span,
            format!(
                "symbol `{name}` must be ASCII letters, digits, `_`, `.` and `$`, \
                 starting with a letter or `_`"
            ),
        ));
    }
    Ok(Some(LitStr::new(&name, span)))
}

fn check_signature(func: &ItemFn, options: &Options) -> syn::Result<()> {
    let sig = &func.sig;
    let unsupported = |span: Span, what: &str| -> syn::Result<()> {
        Err(syn::Error::new(
            span,
            format!("`#[probe]` does not support {what}"),
        ))
    };
    if let Some(t) = &sig.constness {
        return unsupported(t.span(), "`const fn`: a probe cannot run at compile time");
    }
    if let Some(v) = &sig.variadic {
        return unsupported(v.span(), "variadic functions");
    }
    if let ReturnType::Type(_, ty) = &sig.output
        && matches!(**ty, Type::Never(_))
    {
        return unsupported(ty.span(), "functions returning `!`: they never return");
    }
    if let Some(attr) = func
        .attrs
        .iter()
        .find(|a| a.path().is_ident("track_caller"))
    {
        return unsupported(
            attr.span(),
            "`#[track_caller]`: the body runs in a closure, which would report its own \
             location rather than the caller's",
        );
    }
    if let Some((_, span)) = &options.symbol {
        let span = *span;
        if sig.asyncness.is_some() {
            return Err(syn::Error::new(
                span,
                "`symbol` does not apply to an `async fn`: its symbol only creates the \
                 future, and the body runs later, in `poll`",
            ));
        }
        let generic = sig
            .generics
            .params
            .iter()
            .any(|p| !matches!(p, GenericParam::Lifetime(_)))
            || sig.inputs.iter().any(|input| match input {
                FnArg::Typed(t) => contains_impl(&t.ty),
                FnArg::Receiver(_) => false,
            });
        if generic {
            return Err(syn::Error::new(
                span,
                "`symbol` needs a function that is not generic over types or consts \
                 (`impl Trait` arguments included): each instantiation would need its \
                 own symbol",
            ));
        }
        if let Some(attr) = func.attrs.iter().find(|a| {
            ["inline", "no_mangle", "export_name"]
                .iter()
                .any(|n| a.path().is_ident(n))
                || a.path().is_ident("unsafe")
        }) {
            return Err(syn::Error::new(
                attr.path().span(),
                "`symbol` sets `#[inline(never)]` and the exported name itself; remove this \
                 attribute",
            ));
        }
    }
    Ok(())
}

/// Each argument the entry probe passes, in order, with its mode applied.
/// Checks that every listed name is an argument; `options.modes` is drained.
fn collect_args(func: &ItemFn, options: &mut Options) -> syn::Result<Vec<Passed>> {
    let mut passed = Vec::new();
    for input in &func.sig.inputs {
        let (name, value, ty, is_self) = match input {
            FnArg::Receiver(r) => (
                "self".to_owned(),
                quote_spanned!(r.self_token.span=> self),
                (*r.ty).clone(),
                true,
            ),
            FnArg::Typed(t) => match &*t.pat {
                Pat::Ident(p) if p.by_ref.is_none() && p.subpat.is_none() => {
                    let ident = &p.ident;
                    (
                        ident.unraw().to_string(),
                        quote!(#ident),
                        (*t.ty).clone(),
                        false,
                    )
                }
                Pat::Wild(_) => continue,
                pat => {
                    return Err(syn::Error::new(
                        pat.span(),
                        "`#[probe]` needs each argument bound to a plain name (or `_`); \
                         bind it to a name and destructure it in the body",
                    ));
                }
            },
        };
        let listed = options.modes.remove(&name);
        // Errors about the encoding (no `serde` feature, no `Serialize`
        // impl) point at where the argument was listed.
        let listed_span = listed.map_or_else(|| ty.span(), |(_, s)| s);
        let mode = match listed {
            Some((mode, _)) => Some(mode),
            None if is_self => Some(Mode::Skip),
            None => None,
        };
        let param = if is_self {
            format_ident!("__anyprobe_self")
        } else {
            format_ident!("{}", name)
        };
        let lit = LitStr::new(&name, Span::call_site());
        let shape = match mode {
            Some(Mode::Skip) => continue,
            Some(Mode::Serde) => Shape::Encoded {
                expr: quote_spanned!(listed_span=> ::anyprobe::__private::serde_value!(#lit, &#value)),
                encoding: "json",
            },
            Some(Mode::Debug) => Shape::Encoded {
                expr: quote_spanned!(listed_span=> ::anyprobe::__private::Value::debug(&#value)),
                encoding: "debug",
            },
            Some(Mode::Native) => {
                if is_self {
                    return Err(syn::Error::new(
                        listed_span,
                        "`self` cannot be `native`; use `debug(self)` or `serde(self)`",
                    ));
                }
                native_shape(&ty, &value).unwrap_or_else(|| Shape::Native {
                    kind: Kind::Unsigned("u64"),
                    expr: quote_spanned!(ty.span()=> ::anyprobe::Native::to_u64(&#value)),
                })
            }
            None => native_shape(&ty, &value).unwrap_or_else(|| Shape::Encoded {
                expr: quote_spanned!(ty.span()=> ::anyprobe::__private::unlisted!(#lit, &#value)),
                encoding: "auto",
            }),
        };
        passed.push(Passed {
            field: name,
            param,
            shape,
        });
    }
    if let Some((name, (_, span))) = options.modes.drain().min_by_key(|(n, _)| n.clone()) {
        return Err(syn::Error::new(span, format!("no argument named `{name}`")));
    }
    Ok(passed)
}

/// The native shape of a value of type `ty`, if its type is recognized:
/// one of the `probes!` types, or a reference to a scalar one (passed by
/// value), or `&mut str` / `&mut [u8]`.
fn native_shape(ty: &Type, value: &TokenStream) -> Option<Shape> {
    if let Some(kind) = args::classify(ty) {
        let expr = match kind {
            Kind::Pointer => quote!(#value as *const ()),
            _ => quote!(#value),
        };
        return Some(Shape::Native { kind, expr });
    }
    let r = match ty {
        Type::Reference(r) => r,
        Type::Group(g) => return native_shape(&g.elem, value),
        Type::Paren(p) => return native_shape(&p.elem, value),
        _ => return None,
    };
    if let Some(kind) = args::classify(&r.elem) {
        let expr = match kind {
            Kind::Pointer => quote!(*#value as *const ()),
            Kind::Str | Kind::Bytes => return None,
            _ => quote!(*#value),
        };
        return Some(Shape::Native { kind, expr });
    }
    // `&mut str` and `&mut [u8]`: classify the shared form.
    if r.mutability.is_some() {
        let elem = &r.elem;
        let shared: Type = syn::parse_quote!(&#elem);
        if let Some(kind @ (Kind::Str | Kind::Bytes)) = args::classify(&shared) {
            return Some(Shape::Native {
                kind,
                expr: quote!(&*#value),
            });
        }
    }
    None
}

/// The probe parameter type for a native kind. Pointers lose their pointee
/// type, which may name generics the probe module cannot see.
fn param_type(kind: Kind) -> TokenStream {
    match kind {
        Kind::Unsigned(n) | Kind::Signed(n) => {
            let n = format_ident!("{n}");
            quote!(#n)
        }
        Kind::Bool => quote!(bool),
        Kind::Char => quote!(char),
        Kind::Pointer => quote!(*const ()),
        Kind::Str => quote!(&str),
        Kind::Bytes => quote!(&[u8]),
    }
}

/// A native value, already of its probe parameter type, as a `Value`.
fn to_value(kind: Kind, v: &TokenStream) -> TokenStream {
    let value = quote!(::anyprobe::__private::Value);
    match kind {
        Kind::Unsigned(_) => quote!(#value::U64(#v as u64)),
        Kind::Signed(_) => quote!(#value::I64(#v as i64)),
        Kind::Bool => quote!(#value::Bool(#v)),
        Kind::Char => quote!(#value::Char(#v)),
        Kind::Pointer => quote!(#value::Ptr(#v as usize)),
        Kind::Str => quote!(#value::Str(#v)),
        Kind::Bytes => quote!(#value::Bytes(#v)),
    }
}

/// What every probe of one function records in the registry besides its
/// arguments: the function's name and where it is written.
#[derive(Clone)]
struct Site {
    function: String,
    span: Span,
}

fn probe(args: Vec<Arg>, module: &Ident, origin: &'static str, site: &Site) -> Probe {
    Probe {
        attrs: Vec::new(),
        vis: syn::parse_quote!(pub),
        name: module.clone(),
        args,
        origin,
        function: site.function.clone(),
        span: site.span,
    }
}

/// The invocation id argument an `async fn`'s probes pass first.
fn invocation_arg() -> Arg {
    Arg {
        name: format_ident!("__anyprobe_invocation"),
        field: "invocation".to_owned(),
        ty: quote!(u64),
        kind: Kind::Unsigned("u64"),
        registry_type: "u64",
    }
}

/// A string argument carrying a value encoded as `encoding`.
fn str_arg(name: Ident, field: String, encoding: &'static str) -> Arg {
    Arg {
        name,
        field,
        ty: quote!(&str),
        kind: Kind::Str,
        registry_type: encoding,
    }
}

/// The entry probe's module, its cold helper, and the call to the helper.
/// With `invocation`, every probe argument list starts with the invocation
/// id, and the call passes the local `__anyprobe_invocation`.
fn entry_parts(
    provider: &str,
    name: &str,
    module: &Ident,
    passed: &[Passed],
    collapse: bool,
    invocation: bool,
    site: &Site,
) -> (TokenStream, TokenStream, TokenStream) {
    let (args, helper, call) = if collapse {
        entry_collapsed(module, passed, invocation)
    } else {
        entry_separate(module, passed, invocation)
    };
    let probe = probe(args, module, "entry", site);
    (define(provider, name, probe), helper, call)
}

/// The invocation id's probe argument, helper parameter and call argument,
/// or nothing.
fn invocation_parts(invocation: bool) -> (Vec<Arg>, TokenStream, TokenStream) {
    if invocation {
        (
            vec![invocation_arg()],
            quote!(__anyprobe_invocation: u64,),
            quote!(__anyprobe_invocation,),
        )
    } else {
        (Vec::new(), TokenStream::new(), TokenStream::new())
    }
}

/// Every argument in one JSON object, passed as the one `args` string.
fn entry_collapsed(
    module: &Ident,
    passed: &[Passed],
    invocation: bool,
) -> (Vec<Arg>, TokenStream, TokenStream) {
    let private = quote!(::anyprobe::__private);
    let (mut args, inv_param, inv_arg) = invocation_parts(invocation);
    let params = passed.iter().map(|p| &p.param);
    let params_again = params.clone();
    let fields = passed.iter().map(|p| p.field.as_str());
    let object = format_ident!("args");
    let helper = quote! {
        #[cold]
        #[inline(never)]
        pub fn fire_entry(#inv_param #(#params: #private::Value<'_>),*) {
            #private::encode::object([#(#fields),*], [#(#params_again),*], |#object| {
                #module::fire(#inv_arg #object)
            });
        }
    };
    let values = passed.iter().map(|p| match &p.shape {
        Shape::Native { kind, expr } => to_value(*kind, expr),
        Shape::Encoded { expr, .. } => expr.clone(),
    });
    let call = quote!(__anyprobe::fire_entry(#inv_arg #(#values),*););
    args.push(str_arg(object, "args".to_owned(), "object"));
    (args, helper, call)
}

/// Each argument as its own probe argument; encoded ones as strings.
fn entry_separate(
    module: &Ident,
    passed: &[Passed],
    invocation: bool,
) -> (Vec<Arg>, TokenStream, TokenStream) {
    let private = quote!(::anyprobe::__private);
    let (mut args, inv_param, inv_arg) = invocation_parts(invocation);
    let mut helper_params = Vec::new();
    let mut encoded = Vec::new();
    for p in passed {
        let param = &p.param;
        match &p.shape {
            Shape::Native { kind, .. } => {
                let ty = param_type(*kind);
                helper_params.push(quote!(#param: #ty));
                args.push(Arg {
                    name: param.clone(),
                    field: p.field.clone(),
                    ty,
                    kind: *kind,
                    registry_type: kind.registry_type(),
                });
            }
            Shape::Encoded { encoding, .. } => {
                helper_params.push(quote!(#param: #private::Value<'_>));
                args.push(str_arg(param.clone(), p.field.clone(), encoding));
                encoded.push(param);
            }
        }
    }
    let params = passed.iter().map(|p| &p.param);
    let fire = quote!(#module::fire(#inv_arg #(#params),*));
    let body = if encoded.is_empty() {
        quote!(#fire;)
    } else {
        quote!(#private::encode::text([#(#encoded),*], |[#(#encoded),*]| #fire);)
    };
    let helper = quote! {
        #[cold]
        #[inline(never)]
        pub fn fire_entry(#inv_param #(#helper_params),*) {
            #body
        }
    };
    let exprs = passed.iter().map(|p| match &p.shape {
        Shape::Native { expr, .. } | Shape::Encoded { expr, .. } => expr,
    });
    let call = quote!(__anyprobe::fire_entry(#inv_arg #(#exprs),*););
    (args, helper, call)
}

/// How the return probe passes the return value.
enum Ret {
    None,
    Native(Kind, TokenStream),
    /// The value's expression and its registry type.
    Encoded(TokenStream, &'static str),
}

fn return_shape(func: &ItemFn, ret: Option<(Mode, Span)>) -> syn::Result<Ret> {
    let Some((mode, span)) = ret else {
        return Ok(Ret::None);
    };
    let ty = match &func.sig.output {
        ReturnType::Type(_, ty) if !matches!(&**ty, Type::Tuple(t) if t.elems.is_empty()) => ty,
        _ => {
            return Err(syn::Error::new(
                span,
                "`ret` needs a return value; this function returns `()`",
            ));
        }
    };
    let value = quote!(__anyprobe_ret);
    Ok(match mode {
        Mode::Native => match native_shape(ty, &value) {
            Some(Shape::Native { kind, expr }) => Ret::Native(kind, expr),
            _ => Ret::Native(
                Kind::Unsigned("u64"),
                quote_spanned!(ty.span()=> ::anyprobe::Native::to_u64(&#value)),
            ),
        },
        Mode::Serde => Ret::Encoded(
            quote_spanned!(span=> ::anyprobe::__private::serde_value!("ret", &#value)),
            "json",
        ),
        Mode::Debug => Ret::Encoded(
            quote_spanned!(span=> ::anyprobe::__private::Value::debug(&#value)),
            "debug",
        ),
        Mode::Skip => Ret::None,
    })
}

/// The return probe's module, its cold helper, and the call to the helper.
/// With `invocation`, as for [`entry_parts`].
fn return_parts(
    provider: &str,
    name: &str,
    module: &Ident,
    ret: Ret,
    invocation: bool,
    site: &Site,
) -> (TokenStream, TokenStream, TokenStream) {
    let private = quote!(::anyprobe::__private);
    let (mut args, inv_param, inv_arg) = invocation_parts(invocation);
    let r = format_ident!("ret");
    let (ret_args, param, body, arg) = match ret {
        Ret::None => (
            Vec::new(),
            quote!(),
            quote!(#module::fire(#inv_arg);),
            quote!(),
        ),
        Ret::Native(kind, expr) => {
            let ty = param_type(kind);
            (
                vec![Arg {
                    name: r.clone(),
                    field: "ret".to_owned(),
                    ty: ty.clone(),
                    kind,
                    registry_type: kind.registry_type(),
                }],
                quote!(#r: #ty),
                quote!(#module::fire(#inv_arg #r);),
                expr,
            )
        }
        Ret::Encoded(expr, encoding) => (
            vec![str_arg(r.clone(), "ret".to_owned(), encoding)],
            quote!(#r: #private::Value<'_>),
            quote!(#private::encode::text([#r], |[#r]| #module::fire(#inv_arg #r));),
            expr,
        ),
    };
    args.extend(ret_args);
    let helper = quote! {
        #[cold]
        #[inline(never)]
        pub fn fire_return(#inv_param #param) {
            #body
        }
    };
    let call = quote!(__anyprobe::fire_return(#inv_arg #arg););
    let probe = probe(args, module, "return", site);
    (define(provider, name, probe), helper, call)
}

/// The unwind probe's module, and the guard whose drop fires it.
///
/// For a sync fn the guard is only dropped while a panic unwinds, so the
/// probe takes no arguments. For an `async fn` it is also dropped when the
/// future is dropped before completion; the probe passes the invocation id
/// and whether the thread was panicking, which tells the two apart.
fn unwind_parts(
    provider: &str,
    name: &str,
    module: &Ident,
    invocation: bool,
    site: &Site,
) -> (TokenStream, TokenStream) {
    let private = quote!(::anyprobe::__private);
    let (args, guard) = if invocation {
        let panicking = format_ident!("panicking");
        (
            vec![
                invocation_arg(),
                Arg {
                    name: panicking.clone(),
                    field: "panicking".to_owned(),
                    ty: quote!(bool),
                    kind: Kind::Bool,
                    registry_type: "bool",
                },
            ],
            quote! {
                pub struct Guard(pub u64);

                impl ::core::ops::Drop for Guard {
                    #[inline]
                    fn drop(&mut self) {
                        if #module::enabled() {
                            fire_unwind(self.0, #private::panicking());
                        }
                    }
                }

                #[cold]
                #[inline(never)]
                pub fn fire_unwind(__anyprobe_invocation: u64, #panicking: bool) {
                    #module::fire(__anyprobe_invocation, #panicking);
                }
            },
        )
    } else {
        (
            Vec::new(),
            quote! {
                pub struct Guard;

                impl ::core::ops::Drop for Guard {
                    #[inline]
                    fn drop(&mut self) {
                        if #module::enabled() {
                            fire_unwind();
                        }
                    }
                }

                #[cold]
                #[inline(never)]
                pub fn fire_unwind() {
                    #module::fire();
                }
            },
        )
    };
    let probe = probe(args, module, "unwind", site);
    (define(provider, name, probe), guard)
}

/// The probe module for `probe`, named `module`, with probe name `name`.
fn define(provider: &str, name: &str, probe: Probe) -> TokenStream {
    probes::define_named_probe(provider, name, &probe)
}

/// Whether `ty` mentions `impl Trait` anywhere.
fn contains_impl(ty: &Type) -> bool {
    fn scan(tokens: TokenStream) -> bool {
        tokens.into_iter().any(|t| match t {
            TokenTree::Ident(i) => i == "impl",
            TokenTree::Group(g) => scan(g.stream()),
            _ => false,
        })
    }
    scan(quote!(#ty))
}

#[cfg(test)]
#[path = "attr_tests.rs"]
mod attr_tests;
