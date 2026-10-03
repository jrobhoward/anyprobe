//! Windows: ETW TraceLogging.
//!
//! Each `probes!` block defines one provider, named after the probe provider;
//! its GUID is the standard TraceLogging hash of that name, so it is the same
//! in every build. Each probe is an event of the same name at level Verbose.
//!
//! The enabled check is one load of an atomic flag that the provider's enable
//! callback keeps current. The provider registers itself on the first check of
//! any of its probes, so nothing runs at startup; see [`etw::Provider`].

pub(crate) const NAME: &str = "windows-etw";

/// Defines one probe's `enabled` and `fire`. Called by `probes!`.
#[doc(hidden)]
#[macro_export]
macro_rules! __anyprobe_define_probe {
    (
        provider: $provider:literal,
        name: $name:literal,
        etw_provider: $etw:path,
        params: [$($param:ident: $ty:ty),*],
        sdt: $sdt:literal, [$($sdt_op:tt)*],
        dtrace: { $($dtrace:tt)* },
        etw: [$($method:ident($field:literal, ($value:expr), $out:ident)),*],
    ) => {
        /// The provider this probe belongs to.
        pub const PROVIDER: &str = $provider;
        /// The probe's name.
        pub const NAME: &str = $name;

        /// Whether an ETW session is listening to this probe.
        #[inline(always)]
        #[must_use]
        pub fn enabled() -> bool {
            $etw.enabled()
        }

        /// Writes the probe's event.
        #[inline(always)]
        pub fn fire($($param: $ty),*) {
            $etw.write($name, |_event| {
                $(
                    _event.$method(
                        $field,
                        $value,
                        $crate::__private::etw::OutType::$out,
                        0,
                    );
                )*
            });
        }
    };
}

/// Defines the provider static a `probes!` block's probes share.
#[doc(hidden)]
#[macro_export]
macro_rules! __anyprobe_define_provider {
    ($ident:ident, $name:literal) => {
        #[doc(hidden)]
        static $ident: $crate::__private::etw::Provider = {
            extern "C" fn unregister() {
                $ident.unregister();
            }
            $crate::__private::etw::Provider::new($name, unregister)
        };
    };
}

pub mod etw {
    //! The ETW provider runtime behind generated code.

    use core::pin::Pin;
    use core::sync::atomic::{AtomicU8, Ordering};
    use std::cell::RefCell;
    use std::sync::{Once, OnceLock};

    pub use tracelogging_dynamic::OutType;
    use tracelogging_dynamic::{EventBuilder, Guid, Level};

    const OFF: u8 = 0;
    const ON: u8 = 1;
    const UNREGISTERED: u8 = 2;

    unsafe extern "C" {
        fn atexit(callback: extern "C" fn()) -> core::ffi::c_int;
    }

    /// One ETW provider, registered lazily.
    ///
    /// `state` answers the enabled check in one load: off, on, or not yet
    /// registered. The first check that sees "not yet registered" registers
    /// the provider. ETW reports sessions already listening as the provider
    /// registers, so a session started earlier still takes effect; the enable
    /// callback keeps `state` current from then on. The provider is invisible
    /// to ETW until that first check.
    pub struct Provider {
        name: &'static str,
        state: AtomicU8,
        once: Once,
        provider: OnceLock<tracelogging_dynamic::Provider>,
        on_exit: extern "C" fn(),
    }

    impl core::fmt::Debug for Provider {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.debug_struct("Provider")
                .field("name", &self.name)
                .field("state", &self.state.load(Ordering::Relaxed))
                .finish_non_exhaustive()
        }
    }

    impl Provider {
        /// A provider named `name`. `on_exit` must call [`Provider::unregister`]
        /// on this provider; it is registered with `atexit` when the provider
        /// registers, as `tracelogging_dynamic` requires for providers in DLLs.
        #[must_use]
        pub const fn new(name: &'static str, on_exit: extern "C" fn()) -> Self {
            Provider {
                name,
                state: AtomicU8::new(UNREGISTERED),
                once: Once::new(),
                provider: OnceLock::new(),
                on_exit,
            }
        }

        /// Whether any session is listening at level Verbose.
        #[inline(always)]
        #[must_use]
        pub fn enabled(&'static self) -> bool {
            match self.state.load(Ordering::Relaxed) {
                OFF => false,
                ON => true,
                _ => self.register(),
            }
        }

        #[cold]
        #[inline(never)]
        fn register(&'static self) -> bool {
            self.once.call_once(|| {
                let mut options = tracelogging_dynamic::Provider::options();
                options.callback(callback, self as *const Self as usize);
                let provider = self
                    .provider
                    .get_or_init(|| tracelogging_dynamic::Provider::new(self.name, &options));
                // SAFETY: the provider is in a `static`, so it never moves and
                // is never dropped. `register` requires a provider in a DLL to
                // be unregistered before the DLL unloads: `on_exit`, registered
                // with `atexit` below, does that, and in a DLL the CRT runs
                // `atexit` handlers at unload.
                let rc = unsafe { Pin::static_ref(provider).register() };
                if rc == 0 {
                    // SAFETY: `on_exit` is a plain `extern "C" fn()` that
                    // unregisters a static provider.
                    unsafe { atexit(self.on_exit) };
                    let now = if provider.enabled(Level::Verbose, 0) {
                        ON
                    } else {
                        OFF
                    };
                    // The enable callback may already have stored a newer
                    // state; only replace "not yet registered".
                    let _ = self.state.compare_exchange(
                        UNREGISTERED,
                        now,
                        Ordering::Relaxed,
                        Ordering::Relaxed,
                    );
                } else {
                    self.state.store(OFF, Ordering::Relaxed);
                }
            });
            self.state.load(Ordering::Relaxed) == ON
        }

        /// Unregisters the provider; its probes stay disabled afterwards.
        pub fn unregister(&self) {
            self.state.store(OFF, Ordering::Relaxed);
            if let Some(provider) = self.provider.get() {
                provider.unregister();
            }
        }

        /// Writes one event named `name`; `fields` adds its fields.
        #[inline(never)]
        pub fn write(&'static self, name: &str, fields: impl FnOnce(&mut EventBuilder)) {
            let Some(provider) = self.provider.get() else {
                return;
            };
            thread_local! {
                static EVENT: RefCell<EventBuilder> = RefCell::new(EventBuilder::new());
            }
            // A probe fired while another is being written on the same thread
            // (from a field's encoding, say) is dropped rather than panicking.
            let _ = EVENT.try_with(|event| {
                if let Ok(mut event) = event.try_borrow_mut() {
                    event.reset(name, Level::Verbose, 0, 0);
                    fields(&mut event);
                    event.write(provider, None, None);
                }
            });
        }
    }

    fn callback(
        _source_id: &Guid,
        _event_control_code: u32,
        _level: Level,
        _match_any_keyword: u64,
        _match_all_keyword: u64,
        _filter_data: usize,
        context: usize,
    ) {
        // SAFETY: `context` is the address of a `static Provider`, set in
        // `Provider::register`.
        let this = unsafe { &*(context as *const Provider) };
        // `tracelogging_dynamic` updates its own level and keywords before
        // calling this, so `enabled` already reflects the change.
        if let Some(provider) = this.provider.get() {
            let now = if provider.enabled(Level::Verbose, 0) {
                ON
            } else {
                OFF
            };
            this.state.store(now, Ordering::Relaxed);
        }
    }

    /// The provider GUID ETW tools need for `name`, as `{xxxxxxxx-...}`.
    #[must_use]
    pub fn guid_string(name: &str) -> String {
        let guid = Guid::from_name(name);
        format!("{{{}}}", String::from_utf8_lossy(&guid.to_utf8_bytes()))
    }
}
