//! USDT-style probes for Rust.
//!
//! A probe is a named point in a program that a tracer can attach to while the
//! program runs. With no tracer attached a probe costs one enabled check; its
//! arguments are not computed. `anyprobe` emits each platform's native probe
//! format, so the platform's own tools attach:
//!
//! | Target | Probe format | Tracers |
//! |---|---|---|
//! | Linux (x86-64, AArch64) | SystemTap SDT notes with semaphores | bpftrace, perf |
//! | macOS (x86-64, AArch64) | DTrace USDT, built by the linker | dtrace |
//! | FreeBSD (x86-64) | DTrace USDT, registered at startup | dtrace |
//! | Windows | ETW TraceLogging | WPR, PerfView, logman |
//! | anything else | nothing; checks are `false` | none |
//!
//! `--cfg anyprobe_noop` compiles every probe to nothing on every target;
//! see [Turning probes off](#turning-probes-off).
//!
//! [`probe`] probes a function's entry and return:
//!
//! ```
//! # #[derive(Debug)] struct Opts;
//! #[anyprobe::probe(provider = "myapp", debug(opts), ret = native)]
//! fn handle(id: u64, path: &str, opts: &Opts) -> u32 {
//!     // ...
//! #   0
//! }
//! # handle(1, "/", &Opts);
//! ```
//!
//! This defines `myapp:handle__entry(id, path, opts)` and
//! `myapp:handle__return(ret)`. Integers, `bool`, `char`, raw pointers, `&str`,
//! `&[u8]`, their `Option`s and `&CStr` are passed as they are; other arguments are listed in
//! `debug(..)`, `serde(..)` (JSON) or `skip(..)`. The attribute's
//! documentation lists every option.
//!
//! [`probes!`] defines probes to fire from anywhere in a function. Each one
//! becomes a module with `enabled()` and `fire(...)`, and [`fire!`] calls
//! `fire` only when `enabled()` is true, so the arguments are computed only
//! while a tracer is attached:
//!
//! ```
//! anyprobe::probes! {
//!     provider = "myapp";
//!
//!     /// A request started.
//!     pub fn request__start(id: u64, path: &str);
//! }
//!
//! fn handle(id: u64, path: &str) {
//!     anyprobe::fire!(request__start(id, path));
//!     // ...
//! }
//! # handle(1, "/");
//! ```
//!
//! [`list`] returns every probe compiled into the binary, with its arguments
//! and where it is defined (see [`registry`]). The `cargo-anyprobe` tool
//! reads the same description from a built binary and writes bpftrace, D
//! and WPR scripts for it.
//!
//! Attaching on each platform, for the example above: bpftrace on Linux,
//! dtrace on macOS and FreeBSD.
//!
//! ```text
//! sudo bpftrace -p PID -e 'usdt:*:myapp:request__start { printf("%r\n", buf(arg1, arg2)); }'
//! sudo dtrace -p PID -n 'myapp$target:::request-start { printf("%s\n", copyinstr(arg1, arg2)); }'
//! ```
//!
//! On FreeBSD the probes are registered with the kernel at startup;
//! [`registration`] reports whether that worked.
//!
//! On Windows, start an ETW session for the provider `myapp` (PerfView
//! `*myapp`, or `logman` with the provider's name-derived GUID).
//!
//! # Features
//!
//! - `serde` (default): `serde(..)` in [`probe`], encoding as JSON.
//! - `autoref`: arguments [`probe`] does not list and cannot pass natively
//!   are encoded as JSON if they implement `serde::Serialize`, otherwise with
//!   `{:?}`, instead of being a compile error. Implies `serde`. In generic
//!   code the choice follows the declared bounds: an argument of type `T`
//!   where `T: Debug` is encoded with `{:?}`, even if the concrete type is
//!   also `Serialize`.
//!
//! # Turning probes off
//!
//! A program that depends, directly or not, on a library with probes can
//! compile every probe out by building with `--cfg anyprobe_noop`, for
//! example in the program's `.cargo/config.toml`:
//!
//! ```toml
//! [build]
//! rustflags = ["--cfg", "anyprobe_noop"]
//! ```
//!
//! The probes in every crate of the build then behave as on a target with no
//! tracer: `enabled()` is `false`, `fire` does nothing, no tracer metadata
//! is emitted, [`list`] is empty, nothing registers at startup on FreeBSD or
//! with ETW on Windows, and [`BACKEND`] is `noop`. The API is unchanged, so
//! the libraries compile as they are. A `symbol` given to [`probe`] still
//! sets the function's symbol name.
//!
//! It is a cfg rather than a Cargo feature because Cargo unifies features: a
//! library enabling such a feature would turn off the probes of every other
//! crate in the build. The cfg is set by whoever builds the final binary.

#![cfg_attr(docsrs, feature(doc_cfg))]

pub use anyprobe_macros::{probe, probes};
pub use error::{RegistrationError, RegistryError};
pub use native::Native;
pub use registry::list;

// Public only so `__private` can re-export it.
#[doc(hidden)]
pub mod encode;
mod error;
mod native;
pub mod registry;

// `--cfg anyprobe_noop` selects the no-op backend on every target, for a
// build that wants no probes in any crate (see the crate docs). A cfg, not a
// feature: whoever builds the final binary decides, and no library can.
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64"),
    not(anyprobe_noop)
))]
#[path = "linux.rs"]
mod backend;

#[cfg(all(
    target_os = "macos",
    any(target_arch = "x86_64", target_arch = "aarch64"),
    not(anyprobe_noop)
))]
#[path = "macos.rs"]
mod backend;

#[cfg(all(target_os = "freebsd", target_arch = "x86_64", not(anyprobe_noop)))]
#[path = "freebsd.rs"]
mod backend;

#[cfg(all(windows, not(anyprobe_noop)))]
#[path = "windows.rs"]
mod backend;

#[cfg(any(
    anyprobe_noop,
    not(any(
        all(
            any(target_os = "linux", target_os = "macos"),
            any(target_arch = "x86_64", target_arch = "aarch64")
        ),
        all(target_os = "freebsd", target_arch = "x86_64"),
        windows
    ))
))]
#[path = "noop.rs"]
mod backend;

// The registry section and its `register!`, shared by the ELF backends.
#[cfg(all(
    any(
        all(
            target_os = "linux",
            any(target_arch = "x86_64", target_arch = "aarch64")
        ),
        all(target_os = "freebsd", target_arch = "x86_64")
    ),
    not(anyprobe_noop)
))]
mod elf;

// The FreeBSD site table. Plain data processing, so its tests run on every
// host.
#[cfg(any(
    all(target_os = "freebsd", target_arch = "x86_64", not(anyprobe_noop)),
    test
))]
mod sites;

/// Name of the probe backend compiled for this target: `linux-sdt`,
/// `macos-dtrace`, `freebsd-dtrace`, `windows-etw` or `noop`. `noop` on
/// every target under `--cfg anyprobe_noop`.
pub const BACKEND: &str = backend::NAME;

/// Whether the probes of this executable or library are registered with the
/// tracer.
///
/// Only FreeBSD registers probes at runtime: a constructor hands them to the
/// kernel through `/dev/dtrace/helper` when the executable or library is
/// loaded. If that fails, the probes stay off for the life of the process,
/// and this returns why. The usual causes are DTrace not being loaded
/// (`kldload dtraceall`) and a user who may not open the device, which is
/// `root:wheel`, mode `0660`, by default. On every other target there is
/// nothing to register at startup and this returns `Ok`.
///
/// On Windows that includes ETW: a provider registers the first time one of
/// its probes is checked, not at startup, so there is nothing to report yet
/// when this is called. If ETW refuses a registration (the per-process limit
/// on registrations, or no memory), the probe that asked stays off for the
/// life of the process, and the next probe naming that provider that has not
/// been checked yet asks again. Nothing reports the refusal.
///
/// It reports on the executable or library it is compiled into, not on
/// shared libraries loaded alongside it, which register on their own.
///
/// ```
/// if let Err(e) = anyprobe::registration() {
///     eprintln!("probes unavailable: {e}");
/// }
/// ```
///
/// # Errors
///
/// [`RegistrationError`] when registration failed.
pub fn registration() -> Result<(), RegistrationError> {
    backend::registration()
}

/// A new id for correlating probes: unique within the process, across
/// threads, and never 0.
///
/// Pass the same id to several probes, such as the start and end of a
/// request, so a tracer can pair them when calls interleave. Taking an id
/// only when the first probe is enabled keeps the disabled path to the
/// enabled check; the later probes then pass 0, which a tracer reads as "the
/// first probe was off when this started". `#[probe]` passes ids from the
/// same counter to the probes of an `async fn`.
///
/// ```
/// anyprobe::probes! {
///     provider = "myapp";
///
///     /// A request started: its id and path.
///     pub fn request__start(id: u64, path: &str);
///     /// A request finished: its id and status.
///     pub fn request__done(id: u64, status: u32);
/// }
///
/// fn handle(path: &str) -> u32 {
///     let id = if request__start::enabled() {
///         let id = anyprobe::next_id();
///         request__start::fire(id, path);
///         id
///     } else {
///         0
///     };
///     let status = 200;
///     if request__done::enabled() {
///         request__done::fire(id, status);
///     }
///     status
/// }
/// # handle("/");
/// ```
#[cold]
#[inline(never)]
#[must_use]
pub fn next_id() -> u64 {
    use core::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Fires a [`probes!`] probe if it is enabled, evaluating the arguments only
/// then.
///
/// `fire!(request__start(id, path))` expands to
///
/// ```text
/// if request__start::enabled() {
///     request__start::fire(id, path)
/// }
/// ```
///
/// so an argument that costs time to compute costs nothing while no tracer
/// is attached, and the disabled path is the enabled check alone. The probe
/// is named by its module path, as in `fire!(net::request__start(..))`.
/// Calling `enabled()` directly remains useful when one check guards several
/// probes or other work.
///
/// ```
/// anyprobe::probes! {
///     provider = "myapp";
///
///     /// A request started: its id and path.
///     pub fn request__start(id: u64, path: &str);
/// }
///
/// fn handle(id: u64, segments: &[&str]) {
///     // `join` runs only while a tracer is attached.
///     anyprobe::fire!(request__start(id, &segments.join("/")));
///     // ...
/// }
/// # handle(1, &["a", "b"]);
/// ```
#[macro_export]
macro_rules! fire {
    ($($probe:ident)::+ ( $($arg:expr),* $(,)? )) => {
        if $($probe)::+::enabled() {
            $($probe)::+::fire($($arg),*)
        }
    };
}

// Compiles README.md's examples as doctests, so they cannot drift from the API.
#[cfg(doctest)]
#[doc = include_str!("../../../README.md")]
struct ReadmeDoctests;

/// Support for code generated by [`probes!`] and [`probe`]. Not part of the
/// public API: it changes without notice, and only the macros may use it.
#[doc(hidden)]
pub mod __private {
    pub use crate::__anyprobe_define_probe as define_probe;
    pub use crate::__anyprobe_register as register;
    pub use crate::__anyprobe_serde_value as serde_value;
    pub use crate::__anyprobe_unlisted as unlisted;
    pub use crate::encode::{self, Value};

    #[cfg(all(windows, not(anyprobe_noop)))]
    pub use crate::backend::etw;

    /// Whether the `autoref` and `serde` features are on, for tests that
    /// depend on them.
    pub const AUTOREF: bool = cfg!(feature = "autoref");
    /// See [`AUTOREF`].
    pub const SERDE: bool = cfg!(feature = "serde");

    /// Calls `f`. `#[probe]` runs a function's body through this to capture
    /// its return value. Taking `FnOnce` lets the body move out of, and
    /// return borrows of, the arguments it captured, as the function itself
    /// could.
    #[inline(always)]
    pub fn call_once<R>(f: impl FnOnce() -> R) -> R {
        f()
    }

    /// A new invocation id for an `async fn` under `#[probe]`, passed as the
    /// first argument of its probes so a tracer can pair each entry with its
    /// return when calls interleave. An id of 0 on a return or unwind probe
    /// means the entry probe was off when the call started. The same counter
    /// as [`next_id`](crate::next_id), so the two never hand out one id twice.
    pub use crate::next_id as next_invocation;

    /// Whether this thread is unwinding from a panic, for `#[probe(unwind)]`.
    /// Called from here so the caller's crate needs no `::std` path.
    #[inline]
    #[must_use]
    pub fn panicking() -> bool {
        std::thread::panicking()
    }

    /// Encoding selection for arguments `#[probe]` does not list, with the
    /// `autoref` feature: `Serialize`, then `Debug`, then a compile error.
    ///
    /// Method resolution tries `&&&Select` first and removes one reference
    /// per step, so the first impl whose bounds hold wins. In generic code
    /// the bounds are the declared ones, not the concrete type's.
    #[cfg(feature = "autoref")]
    pub mod autoref {
        use super::Value;

        /// The value being encoded.
        #[derive(Debug)]
        pub struct Select<'a, T>(pub &'a T);

        /// First choice: JSON.
        pub trait ViaSerde<'a> {
            /// The encoded value.
            fn __anyprobe_value(&self) -> Value<'a>;
        }

        impl<'a, T: serde::Serialize> ViaSerde<'a> for &&Select<'a, T> {
            #[inline(always)]
            fn __anyprobe_value(&self) -> Value<'a> {
                Value::serde(self.0)
            }
        }

        /// Second choice: `{:?}`.
        pub trait ViaDebug<'a> {
            /// The encoded value.
            fn __anyprobe_value(&self) -> Value<'a>;
        }

        impl<'a, T: core::fmt::Debug> ViaDebug<'a> for &Select<'a, T> {
            #[inline(always)]
            fn __anyprobe_value(&self) -> Value<'a> {
                Value::debug(self.0)
            }
        }

        /// Neither applies. The impl always matches, so method resolution
        /// ends here, and the bound on the method, never met, is the
        /// compile error (with `NoEncoding`'s message).
        pub trait ViaNone<'a> {
            /// The value's type.
            type Inner;
            /// Never callable.
            fn __anyprobe_value(&self) -> Value<'a>
            where
                Self::Inner: NoEncoding;
        }

        impl<'a, T> ViaNone<'a> for Select<'a, T> {
            type Inner = T;
            fn __anyprobe_value(&self) -> Value<'a>
            where
                T: NoEncoding,
            {
                Value::U64(0)
            }
        }

        /// Implemented by nothing.
        #[diagnostic::on_unimplemented(
            message = "`{Self}` has no probe encoding",
            label = "implements neither `serde::Serialize` nor `Debug`",
            note = "add the argument to `skip(...)` in `#[probe]`, or implement `Debug` or `serde::Serialize` for its type"
        )]
        pub trait NoEncoding {}
    }
}

/// `serde(arg)` in `#[probe]`: the argument as a JSON [`Value`](encode::Value).
#[cfg(feature = "serde")]
#[doc(hidden)]
#[macro_export]
macro_rules! __anyprobe_serde_value {
    ($name:literal, $value:expr) => {
        $crate::__private::Value::serde($value)
    };
}

/// Without the `serde` feature, `serde(arg)` is a compile error. It lives
/// outside the backends so that it fires on every target.
#[cfg(not(feature = "serde"))]
#[doc(hidden)]
#[macro_export]
macro_rules! __anyprobe_serde_value {
    ($name:literal, $value:expr) => {
        ::core::compile_error!(::core::concat!(
            "`serde(",
            $name,
            ")` needs the \"serde\" feature of anyprobe, which is off"
        ))
    };
}

/// An argument `#[probe]` does not list and cannot encode natively, with
/// the `autoref` feature: picks an encoding from the traits it implements.
#[cfg(feature = "autoref")]
#[doc(hidden)]
#[macro_export]
macro_rules! __anyprobe_unlisted {
    ($name:literal, $value:expr) => {{
        #[allow(unused_imports)]
        use $crate::__private::autoref::{ViaDebug as _, ViaNone as _, ViaSerde as _};
        (&&&$crate::__private::autoref::Select($value)).__anyprobe_value()
    }};
}

/// Without the `autoref` feature an unlisted argument that is not natively
/// encodable is a compile error naming the fixes.
#[cfg(not(feature = "autoref"))]
#[doc(hidden)]
#[macro_export]
macro_rules! __anyprobe_unlisted {
    ($name:literal, $value:expr) => {
        ::core::compile_error!(::core::concat!(
            "argument `",
            $name,
            "` has no native probe encoding; add `serde(",
            $name,
            ")`, `debug(",
            $name,
            ")` or `skip(",
            $name,
            ")` to `#[probe]`, or enable the \"autoref\" feature of anyprobe"
        ))
    };
}
