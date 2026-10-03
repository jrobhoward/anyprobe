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
/// if request__start::enabled() {
///     request__start::fire(7, "/index.html");
/// }
/// ```
///
/// Each declaration becomes a module with the declaration's name, visibility
/// and doc comments, holding:
///
/// - `enabled() -> bool`: whether a tracer is attached to the probe. Keep
///   any work done only for the probe inside this check.
/// - `fire(...)`: fires the probe with the declared arguments. Every inlined
///   copy of `fire` is a separate probe site under the same name.
/// - `PROVIDER` and `NAME`: the provider and probe names.
///
/// # Provider
///
/// `provider = "...";` is optional and defaults to the crate name. It must be
/// ASCII letters, digits and `_`, not start or end with a digit (DTrace
/// appends the process id to it), and be at most 58 bytes. On Windows it is
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
///
/// At most six values per probe. Types are recognized as written, so a type
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
/// - `provider = "..."`: the default is the crate name. The rules are those
///   of [`probes!`](macro@probes).
/// - `serde(a, b)`: encode these arguments as JSON (needs the `serde`
///   feature, on by default).
/// - `debug(a, b)`: encode these arguments with `{:?}`.
/// - `skip(a, b)`: leave these arguments out.
/// - `native(a, b)`: pass these as one 64-bit value each, through
///   `anyprobe::Native`; for type aliases and newtypes.
/// - `ret = native | serde | debug`: pass the return value to the return
///   probe. Without `ret` the return probe has no arguments.
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
/// followed by a NUL byte, and are cut at 4096 bytes. When the arguments
/// would take more than six values (`&str`, `&[u8]` and encoded arguments
/// take two), they are passed instead as one JSON object of all of them,
/// `{"id":1,"path":"/x",...}`.
///
/// # Not supported
///
/// `async fn`, `const fn`, functions returning `!`, `#[track_caller]`, and
/// arguments bound by a pattern other than a name or `_`.
#[proc_macro_attribute]
pub fn probe(attr: TokenStream, item: TokenStream) -> TokenStream {
    attr::expand(attr.into(), item.into()).into()
}
