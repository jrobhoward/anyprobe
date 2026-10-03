//! Windows: ETW TraceLogging.
//!
//! Each probe provider is one ETW provider of the same name; its GUID is the
//! standard TraceLogging hash of that name, so it is the same in every build.
//! Each probe is an event of the same name at level Verbose.
//!
//! Providers are shared by name across the process: every probe naming the
//! provider `myapp`, from any `probes!` block or `#[probe]` function in any
//! crate, uses one ETW registration. ETW limits registrations per process, so
//! one per probe site would not scale.
//!
//! The enabled check is one load of the probe's own flag. The first check
//! attaches the probe to its provider, registering the provider if no probe
//! has yet; from then on the provider's enable callback keeps the flag
//! current. Nothing runs at startup. See [`etw::Probe`].

pub(crate) const NAME: &str = "windows-etw";

/// Defines one probe's `enabled` and `fire`. Called by `probes!`.
#[doc(hidden)]
#[macro_export]
macro_rules! __anyprobe_define_probe {
    (
        provider: $provider:literal,
        name: $name:literal,
        params: [$($param:ident: $ty:ty),*],
        sdt: $sdt:literal, [$($sdt_op:tt)*],
        dtrace: { $($dtrace:tt)* },
        etw: [$($method:ident($field:literal, ($value:expr), $out:ident)),*],
    ) => {
        /// The provider this probe belongs to.
        pub const PROVIDER: &str = $provider;
        /// The probe's name.
        pub const NAME: &str = $name;

        static PROBE: $crate::__private::etw::Probe = $crate::__private::etw::Probe::new($provider);

        /// Whether an ETW session is listening to this probe.
        #[inline(always)]
        #[must_use]
        pub fn enabled() -> bool {
            PROBE.enabled()
        }

        /// Writes the probe's event.
        #[inline(always)]
        pub fn fire($($param: $ty),*) {
            PROBE.write($name, |_event| {
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

pub mod etw {
    //! The ETW provider runtime behind generated code.

    use core::pin::Pin;
    use core::ptr;
    use core::sync::atomic::{AtomicPtr, AtomicU8, Ordering};
    use std::cell::RefCell;
    use std::sync::{Mutex, MutexGuard, Once, OnceLock, PoisonError};

    pub use tracelogging_dynamic::OutType;
    use tracelogging_dynamic::{EventBuilder, Guid, Level};

    const OFF: u8 = 0;
    const ON: u8 = 1;
    const DETACHED: u8 = 2;

    unsafe extern "C" {
        fn atexit(callback: extern "C" fn()) -> core::ffi::c_int;
    }

    /// One probe: an event in a provider, and whether a session listens.
    ///
    /// `state` answers the enabled check in one load: off, on, or not yet
    /// attached to its provider. The first check that sees "not yet attached"
    /// attaches it. The provider's enable callback keeps `state` current from
    /// then on. ETW reports sessions already listening as a provider
    /// registers, so a session started before the first check still enables
    /// the probe.
    pub struct Probe {
        provider_name: &'static str,
        state: AtomicU8,
        once: Once,
        provider: AtomicPtr<Provider>,
    }

    impl core::fmt::Debug for Probe {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.debug_struct("Probe")
                .field("provider", &self.provider_name)
                .field("state", &self.state.load(Ordering::Relaxed))
                .finish_non_exhaustive()
        }
    }

    impl Probe {
        /// A probe in the provider named `provider`.
        #[must_use]
        pub const fn new(provider: &'static str) -> Self {
            Probe {
                provider_name: provider,
                state: AtomicU8::new(DETACHED),
                once: Once::new(),
                provider: AtomicPtr::new(ptr::null_mut()),
            }
        }

        /// Whether any session is listening to the provider at level Verbose.
        #[inline(always)]
        #[must_use]
        pub fn enabled(&'static self) -> bool {
            match self.state.load(Ordering::Relaxed) {
                OFF => false,
                ON => true,
                _ => self.attach(),
            }
        }

        #[cold]
        #[inline(never)]
        fn attach(&'static self) -> bool {
            self.once
                .call_once(|| match Provider::get(self.provider_name) {
                    Some(provider) => {
                        self.provider
                            .store(ptr::from_ref(provider).cast_mut(), Ordering::Release);
                        provider.add(&self.state);
                    }
                    None => self.state.store(OFF, Ordering::Relaxed),
                });
            self.state.load(Ordering::Relaxed) == ON
        }

        /// Writes one event named `name`; `fields` adds its fields.
        #[inline(never)]
        pub fn write(&'static self, name: &str, fields: impl FnOnce(&mut EventBuilder)) {
            let provider = self.provider.load(Ordering::Acquire);
            if provider.is_null() {
                return;
            }
            // SAFETY: a non-null pointer was stored from a `&'static Provider`
            // in `attach`; providers are leaked and never freed.
            let provider = unsafe { &*provider };
            let Some(inner) = provider.inner.get() else {
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
                    event.write(inner, None, None);
                }
            });
        }
    }

    /// One registered ETW provider, shared by every probe naming it.
    struct Provider {
        name: &'static str,
        inner: OnceLock<tracelogging_dynamic::Provider>,
        /// The `state` of every probe attached to this provider.
        probes: Mutex<Vec<&'static AtomicU8>>,
    }

    /// Every provider registered in this process. Providers are leaked: ETW
    /// holds their address until they unregister, at exit.
    static PROVIDERS: Mutex<Vec<&'static Provider>> = Mutex::new(Vec::new());

    fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
        m.lock().unwrap_or_else(PoisonError::into_inner)
    }

    impl Provider {
        /// The provider named `name`, registered on first use. `None` if ETW
        /// refused the registration.
        fn get(name: &'static str) -> Option<&'static Provider> {
            let mut providers = lock(&PROVIDERS);
            if let Some(p) = providers.iter().find(|p| p.name == name) {
                return Some(*p);
            }
            let provider: &'static Provider = Box::leak(Box::new(Provider {
                name,
                inner: OnceLock::new(),
                probes: Mutex::new(Vec::new()),
            }));
            let mut options = tracelogging_dynamic::Provider::options();
            options.callback(callback, ptr::from_ref(provider) as usize);
            let inner = tracelogging_dynamic::Provider::new(name, &options);
            let inner = provider.inner.get_or_init(|| inner);
            // SAFETY: `inner` is inside a leaked allocation, so it never moves
            // and is never dropped. `register` requires a provider in a DLL to
            // be unregistered before the DLL unloads: `unregister_all`,
            // registered with `atexit` below, does that, and in a DLL the CRT
            // runs `atexit` handlers at unload. The enable callback may run
            // during `register`; it locks only this provider's `probes`, not
            // `PROVIDERS`, which is held here.
            let rc = unsafe { Pin::static_ref(inner).register() };
            if rc != 0 {
                // Left out of `PROVIDERS`, so the next probe naming it
                // tries again.
                return None;
            }
            providers.push(provider);
            static AT_EXIT: Once = Once::new();
            AT_EXIT.call_once(|| {
                // SAFETY: `unregister_all` is a plain `extern "C" fn()`.
                unsafe { atexit(unregister_all) };
            });
            Some(provider)
        }

        fn listening(&self) -> bool {
            self.inner
                .get()
                .is_some_and(|inner| inner.enabled(Level::Verbose, 0))
        }

        /// Attaches a probe's `state`, setting it to the provider's current
        /// state. Under the same lock as the callback, so no change is lost.
        fn add(&self, state: &'static AtomicU8) {
            let mut probes = lock(&self.probes);
            state.store(if self.listening() { ON } else { OFF }, Ordering::Relaxed);
            probes.push(state);
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
        // SAFETY: `context` is the address of a leaked `Provider`, set in
        // `Provider::get`.
        let this = unsafe { &*(context as *const Provider) };
        // `tracelogging_dynamic` updates its own level and keywords before
        // calling this, so `listening` already reflects the change.
        let probes = lock(&this.probes);
        let now = if this.listening() { ON } else { OFF };
        for state in probes.iter() {
            state.store(now, Ordering::Relaxed);
        }
    }

    /// Unregisters every provider at exit; their probes stay off afterwards.
    /// Skips the work if another thread holds the lock, since at exit that
    /// thread may never release it.
    extern "C" fn unregister_all() {
        let Ok(providers) = PROVIDERS.try_lock() else {
            return;
        };
        for provider in providers.iter() {
            if let Ok(probes) = provider.probes.try_lock() {
                for state in probes.iter() {
                    state.store(OFF, Ordering::Relaxed);
                }
            }
            if let Some(inner) = provider.inner.get() {
                inner.unregister();
            }
        }
    }

    /// The provider GUID ETW tools need for `name`, as `{xxxxxxxx-...}`.
    #[must_use]
    pub fn guid_string(name: &str) -> String {
        let guid = Guid::from_name(name);
        format!("{{{}}}", String::from_utf8_lossy(&guid.to_utf8_bytes()))
    }
}
