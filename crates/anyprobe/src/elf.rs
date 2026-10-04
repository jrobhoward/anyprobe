//! What the ELF backends, Linux and FreeBSD, share: the registry section.
//! Its name is a C identifier, so the linker defines `__start_` and `__stop_`
//! symbols at its bounds.

/// Emits a probe's registry record. Called by `probes!`.
#[doc(hidden)]
#[macro_export]
macro_rules! __anyprobe_register {
    ($body:expr) => {
        $crate::__anyprobe_record!("anyprobe_probes", $body);
    };
}

/// The registry section of the executable or library this is linked into.
/// Its name is a C identifier, so the linker defines `__start_` and `__stop_`
/// symbols at its bounds.
pub(crate) fn registry_section() -> &'static [u8] {
    // Puts the section in every binary that calls this, so its bounds are
    // defined even with no probes. The parser skips zero bytes.
    #[used]
    #[unsafe(link_section = "anyprobe_probes")]
    static PAD: [u8; 1] = [0];

    unsafe extern "C" {
        static __start_anyprobe_probes: u8;
        static __stop_anyprobe_probes: u8;
    }
    let start = &raw const __start_anyprobe_probes;
    let stop = &raw const __stop_anyprobe_probes;
    // SAFETY: the linker sets `__start_anyprobe_probes` and
    // `__stop_anyprobe_probes` to the start and end of the section holding
    // `PAD` and every record, so the range is one allocation of initialized
    // bytes. Every byte belongs to an immutable `static`, never written, and
    // lives for the whole program.
    unsafe { core::slice::from_raw_parts(start, (stop as usize).saturating_sub(start as usize)) }
}
