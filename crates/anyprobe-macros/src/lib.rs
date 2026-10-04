//! Procedural macros for [`anyprobe`](https://docs.rs/anyprobe).
//!
//! Use them through the `anyprobe` crate, which re-exports them. The code they
//! generate calls into `anyprobe::__private`, so this crate is not useful on its
//! own.

use proc_macro::TokenStream;

mod args;
mod attr;
mod names;
mod probes;

/// Defines probes.
///
/// ```
/// anyprobe::probes! {
///     provider = "myapp";
///
///     /// A request started.
///     pub fn request__start(id: u64, path: &str);
///     fn request__done(id: u64, status: u16, ok: bool);
/// }
///
/// anyprobe::fire!(request__start(7, "/index.html"));
///
/// if request__done::enabled() {
///     request__done::fire(7, 200, true);
/// }
/// ```
///
/// Each declaration becomes a module with the declaration's name, visibility
/// and doc comments, holding:
///
/// - `enabled() -> bool`: whether a tracer is attached to the probe. Keep
///   any work done only for the probe inside this check.
/// - `fire(...)`: fires the probe with the declared arguments. Every inlined
///   copy of `fire` is a separate probe site under the same name. Its
///   arguments are evaluated on every call; `anyprobe::fire!(probe(...))`
///   evaluates them only when `enabled()` is true.
/// - `PROVIDER` and `NAME`: the provider and probe names.
///
/// # Provider
///
/// `provider = "...";` is optional and defaults to the crate name. It must be
/// ASCII letters, digits and `_`, not start or end with a digit (DTrace
/// appends the process id to it), and be at most 58 bytes. A crate name that
/// ends in a digit gets a `_` after it, so the default for a crate named
/// `http2` is `http2_`; a name given here is used as written. On Windows it is
/// also the ETW provider name. The probes in one block share one provider.
///
/// # Probes
///
/// A probe name is ASCII letters, digits and `_`, at most 63 bytes. DTrace
/// shows `__` as `-` (`request__start` is `request-start`); the other tracers
/// use the name as written.
///
/// Arguments are plain names with one of these types:
///
/// | Type | Values passed |
/// |---|---|
/// | `u8`, `u16`, `u32`, `u64`, `usize` | 1, widened to 64 bits |
/// | `i8`, `i16`, `i32`, `i64`, `isize` | 1, sign-extended to 64 bits |
/// | `bool` | 1 (0 or 1) |
/// | `char` | 1, the code point |
/// | `*const T`, `*mut T` | 1, the address |
/// | `&str`, `&[u8]` | 2: pointer and length |
/// | `Option<&str>`, `Option<&[u8]>` | 2: pointer and length; a null pointer and 0 for `None` |
/// | `&CStr` | 1, a pointer to NUL-terminated bytes |
///
/// At most five values per probe. Types are recognized as written, so a type
/// alias is rejected even if it names one of these. Probes return nothing and
/// cannot be generic, `async`, `const`, `unsafe` or `extern`; only doc, `cfg`
/// and lint attributes are allowed on them.
#[proc_macro]
pub fn probes(input: TokenStream) -> TokenStream {
    probes::expand(input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Probes a function's entry and return.
///
/// ```
/// #[anyprobe::probe(provider = "myapp")]
/// fn parse_request(id: u64, path: &str) -> u32 {
///     // ...
/// #   path.len() as u32
/// }
/// # parse_request(1, "/");
/// ```
///
/// defines the probes `myapp:parse_request__entry(id, path)` and
/// `myapp:parse_request__return()`. The function checks whether each probe
/// is enabled and encodes its arguments only if it is; with no tracer
/// attached the cost is the two checks. `return` and `?` in the body work as
/// before.
///
/// # Options
///
/// - `name = "..."`: the probes' base name; the default is the function's
///   name. Methods have no access to their type's name, so two methods named
///   `new` share probe names unless one sets `name`.
/// - `provider = "..."`: the default is the crate name, with a `_` after a
///   trailing digit. The rules are those of [`probes!`](macro@probes).
/// - `serde(a, b)`: encode these arguments as JSON (needs the `serde`
///   feature, on by default).
/// - `debug(a, b)`: encode these arguments with `{:?}`.
/// - `skip(a, b)`: leave these arguments out.
/// - `native(a, b)`: pass these as one 64-bit value each, through
///   `anyprobe::Native`; for type aliases and newtypes.
/// - `ret = native | serde | debug`: pass the return value to the return
///   probe. Without `ret` the return probe has no arguments.
/// - `unwind`: also define `{name}__unwind`, which fires when the function
///   does not return: a panic unwinds through it, or, for an `async fn`, its
///   future is dropped before it completes. On the normal path the guard
///   that does this runs no code; its check runs only while unwinding or
///   dropping. It adds unwinding code to the function and, for an `async
///   fn`, eight bytes to the future. A panic under `panic = "abort"` never
///   unwinds, so it does not fire.
/// - `symbol` or `symbol = "..."`: also export the function under a stable,
///   unmangled name, `{provider}__{name}` by default, and never inline it,
///   so that raw uprobes, DTrace's `pid` provider and other tools that
///   attach by symbol find it. The function must not be generic over types
///   or consts (rustc also refuses methods of generic `impl` blocks), nor an
///   `async fn`, and it must not carry `#[inline]`, `#[no_mangle]` or
///   `#[export_name]`. Two functions with the same symbol fail to link.
///
/// # `async fn`
///
/// ```
/// #[anyprobe::probe(provider = "myapp", ret = native)]
/// async fn fetch(id: u64, path: &str) -> u64 {
///     // ...
/// #   id
/// }
/// # let _ = fetch(1, "/");
/// ```
///
/// defines `myapp:fetch__entry(invocation, id, path)` and
/// `myapp:fetch__return(invocation, ret)`. The entry probe fires when the
/// body starts, at the future's first poll, and the return probe when it
/// completes. `invocation` is a number unique to the call, so a tracer can
/// pair an entry with its return when calls interleave; it is 0 on the
/// return (and unwind) probe of a call that started while the entry probe
/// was off. It takes one of the five values. With `unwind`, the unwind probe
/// is `fetch__unwind(invocation, panicking)`: `panicking` is 1 for a panic,
/// 0 for a future dropped before completion.
///
/// # Arguments
///
/// An argument not listed in an option is passed natively if its type is
/// written as one of the [`probes!`](macro@probes) types, `char`, a reference
/// to one of the scalar ones (passed by value), `&mut str` or `&mut [u8]`.
/// Any other unlisted argument is a compile error naming the fixes, or, with
/// the `autoref` feature, is encoded as JSON if it implements `Serialize`,
/// otherwise with `{:?}`. `self` is left out unless listed in `debug(self)`
/// or `serde(self)`, and so is an argument named `_`.
///
/// Encoded values reach the tracer as a string (pointer and length),
/// followed by a NUL byte, and are cut at 4096 bytes; a cut value ends with
/// `...`. When the arguments
/// would take more than five values (`&str`, `&[u8]`, their `Option`s and
/// encoded arguments take two), they are passed instead as one JSON object
/// of all of them, `{"id":1,"path":"/x",...}`, where `None` is `null`.
///
/// # Not supported
///
/// `const fn`, functions returning `!`, `#[track_caller]`, arguments bound
/// by a pattern other than a name or `_`, and functions that return a future
/// without being `async fn` (`fn f() -> impl Future`, `#[async_trait]`).
#[proc_macro_attribute]
pub fn probe(attr: TokenStream, item: TokenStream) -> TokenStream {
    attr::expand(attr.into(), item.into()).into()
}
