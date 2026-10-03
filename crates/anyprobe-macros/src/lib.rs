//! Procedural macros for [`anyprobe`](https://docs.rs/anyprobe).
//!
//! Use them through the `anyprobe` crate, which re-exports them. The code they
//! generate calls into `anyprobe::__private`, so this crate is not useful on its
//! own.

use proc_macro::TokenStream;

mod args;
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
