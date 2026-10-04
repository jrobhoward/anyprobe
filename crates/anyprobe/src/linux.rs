//! Linux: SystemTap SDT v3 notes, gated by semaphores.
//!
//! A probe site is a `nop` plus a non-allocated `.note.stapsdt` ELF note
//! recording its address, the provider and probe names, the argument
//! locations, and the address of a 16-bit semaphore. A tracer that attaches
//! increments the semaphore (kernel uprobes `ref_ctr_offset`, Linux 4.20+) and
//! replaces the `nop` with a breakpoint. Until then the only cost is the
//! semaphore load.
//!
//! Format reference: <https://sourceware.org/systemtap/wiki/UserSpaceProbeImplementation>.

pub(crate) const NAME: &str = "linux-sdt";

/// Nothing registers at runtime on this target.
#[allow(clippy::unnecessary_wraps)]
pub(crate) fn registration() -> Result<(), crate::RegistrationError> {
    Ok(())
}

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
    unsafe { core::slice::from_raw_parts(start, stop as usize - start as usize) }
}

/// Defines one probe's semaphore, `enabled` and `fire`. Called by `probes!`.
#[doc(hidden)]
#[macro_export]
macro_rules! __anyprobe_define_probe {
    (
        provider: $provider:literal,
        name: $name:literal,
        params: [$($param:ident: $ty:ty),*],
        sdt: $sdt:literal, [$($op:ident = ($opv:expr)),*],
        dtrace: { $($dtrace:tt)* },
        etw: [$($etw_field:tt)*],
    ) => {
        /// The provider this probe belongs to.
        pub const PROVIDER: &str = $provider;
        /// The probe's name.
        pub const NAME: &str = $name;

        // The note stores the semaphore's link-time address and the tracer
        // writes to it, so it lives in `.probes` like the semaphores
        // `sys/sdt.h` produces.
        #[unsafe(link_section = ".probes")]
        #[used]
        static SEMAPHORE: ::core::sync::atomic::AtomicU16 = ::core::sync::atomic::AtomicU16::new(0);

        /// Whether a tracer is attached to this probe.
        #[inline(always)]
        #[must_use]
        pub fn enabled() -> bool {
            $crate::__anyprobe_sema_load!(SEMAPHORE) != 0
        }

        /// Fires the probe. Every inlined copy is a separate probe site.
        #[inline(always)]
        pub fn fire($($param: $ty),*) {
            $crate::__anyprobe_sdt_site!($provider, $name, SEMAPHORE, $sdt, $($op = in(reg) $opv,)*);
        }
    };
}

/// Emits one SDT probe site: the `nop` the tracer patches, the note that
/// describes it, and (once per object) the `.stapsdt.base` section tools use
/// to detect prelink adjustments and the byte that page-aligns `.probes`.
///
/// Labels are numeric local labels so that every copy of the block, from
/// inlining or monomorphization, assembles to another site rather than a
/// duplicate symbol. `_.stapsdt.base` and `_.anyprobe.probes_page` are the
/// named symbols, each guarded by `.ifndef` and deduplicated across objects
/// as a comdat group.
///
/// `.probes` starts on a page of its own (4 KiB on x86-64; 64 KiB on AArch64,
/// whose kernels may use 64 KiB pages). The kernel raises a semaphore for
/// `bpftrace -c` and perf by its file offset, in the first writable mapping of
/// that file page. rust-lld packs the RELRO segment and the data segment
/// into the file back to back, so the page holding `.probes` can also be the
/// last page of the RELRO segment, mapped at another address: the kernel then
/// raised a copy the program never reads, and every probe stayed off. One
/// retained, page-aligned byte in `.probes` raises the whole output section's
/// alignment, so no other segment maps its first page. It costs up to a page
/// of padding.
///
/// `.stapsdt.base` is executable (`"axGR"`, where `sys/sdt.h` uses `"aG"`) so
/// that it lands in the same segment as the probe sites. perf turns a site's
/// address into a file offset using the base section's offset, which is only
/// right if the two share a segment's address-to-offset delta; with rust-lld
/// and `"aG"`, perf put every probe 0x1000 bytes from its `nop`.
///
/// `.note.stapsdt` is retained (`"R"`, where `sys/sdt.h` uses `""`). The note
/// is not allocated, so `--gc-sections` never collects it, but it can collect
/// the function the note's site is in: a probed `pub fn` in a library that
/// the program never calls. rust-lld then resolves the site's address against
/// the discarded section, and the note names an address in the ELF header.
/// A retained section is a garbage-collection root, so the function its sites
/// refer to stays in the binary, as GNU ld already keeps it. `SHF_LINK_ORDER`
/// (`"o"`) would drop the note with the function instead, but it needs a
/// named symbol in the function's section, and sites use numeric labels only.
///
/// The note section also carries `unique, 1`. The `usdt` and `probe` crates
/// and `sys/sdt.h` emit `.note.stapsdt` with no flags, and an inlined site of
/// theirs can share an object with an anyprobe site; the assembler rejects
/// one section with two sets of flags. A unique id makes anyprobe's notes a
/// section of their own in the object, and the linker merges both into one
/// `.note.stapsdt` in the output, where tracers find them by name.
///
/// Each site also carries a `NONE` relocation against
/// `__start_anyprobe_probes`, which emits no code. rustc 1.88, the MSRV, does
/// not mark a `#[used]` static as retained (1.99 does), and GNU ld under
/// `--gc-sections` keeps a C-identifier section only while live code refers
/// to its `__start_` or `__stop_` symbol. Without the relocation, a binary
/// that never calls [`list`](crate::list) has no registry records, and
/// `cargo anyprobe list` finds none. The reference is weak, so it never
/// fails a link.
///
/// Sites are `readonly`, not `nomem`: an attached tracer reads memory through
/// pointer arguments at the `nop`. Under `nomem` the compiler may sink or drop
/// a store to a buffer whose only reader is the probe.
#[cfg(target_arch = "x86_64")]
#[doc(hidden)]
#[macro_export]
macro_rules! __anyprobe_sdt_site {
    ($provider:literal, $name:literal, $sema:path, $args:literal, $($operands:tt)*) => {
        // SAFETY: the block executes a single `nop` and otherwise only emits
        // data into non-executed sections. The operands are read-only register
        // inputs that the `nop` leaves untouched.
        #[allow(named_asm_labels)]
        unsafe {
            ::core::arch::asm!(
                "990: nop",
                ".weak __start_anyprobe_probes",
                ".reloc 990b, BFD_RELOC_NONE, __start_anyprobe_probes",
                ".pushsection .note.stapsdt, \"R\", \"note\", unique, 1",
                ".balign 4",
                ".4byte 992f-991f, 994f-993f, 3",
                "991: .asciz \"stapsdt\"",
                "992: .balign 4",
                "993: .8byte 990b",
                ".8byte _.stapsdt.base",
                ".8byte {sema}",
                concat!(".asciz \"", $provider, "\""),
                concat!(".asciz \"", $name, "\""),
                concat!(".asciz \"", $args, "\""),
                "994: .balign 4",
                ".popsection",
                ".ifndef _.stapsdt.base",
                ".pushsection .stapsdt.base, \"axGR\", \"progbits\", .stapsdt.base, comdat",
                ".weak _.stapsdt.base",
                ".hidden _.stapsdt.base",
                "_.stapsdt.base: .space 1",
                ".size _.stapsdt.base, 1",
                ".popsection",
                ".endif",
                ".ifndef _.anyprobe.probes_page",
                ".pushsection .probes, \"awGR\", \"progbits\", _.anyprobe.probes_page, comdat",
                ".balign 4096",
                ".weak _.anyprobe.probes_page",
                ".hidden _.anyprobe.probes_page",
                "_.anyprobe.probes_page: .space 1",
                ".size _.anyprobe.probes_page, 1",
                ".popsection",
                ".endif",
                sema = sym $sema,
                $($operands)*
                // AT&T operand syntax: SDT argument strings name registers
                // as `%rdi`, which is how `{aN}` renders under `att_syntax`.
                options(att_syntax, readonly, nostack, preserves_flags),
            )
        }
    };
}

/// AArch64 variant of the x86-64 definition above.
#[cfg(target_arch = "aarch64")]
#[doc(hidden)]
#[macro_export]
macro_rules! __anyprobe_sdt_site {
    ($provider:literal, $name:literal, $sema:path, $args:literal, $($operands:tt)*) => {
        // SAFETY: the block executes a single `nop` and otherwise only emits
        // data into non-executed sections. The operands are read-only register
        // inputs that the `nop` leaves untouched.
        #[allow(named_asm_labels)]
        unsafe {
            ::core::arch::asm!(
                "990: nop",
                ".weak __start_anyprobe_probes",
                ".reloc 990b, BFD_RELOC_NONE, __start_anyprobe_probes",
                ".pushsection .note.stapsdt, \"R\", \"note\", unique, 1",
                ".balign 4",
                ".4byte 992f-991f, 994f-993f, 3",
                "991: .asciz \"stapsdt\"",
                "992: .balign 4",
                "993: .8byte 990b",
                ".8byte _.stapsdt.base",
                ".8byte {sema}",
                concat!(".asciz \"", $provider, "\""),
                concat!(".asciz \"", $name, "\""),
                concat!(".asciz \"", $args, "\""),
                "994: .balign 4",
                ".popsection",
                ".ifndef _.stapsdt.base",
                ".pushsection .stapsdt.base, \"axGR\", \"progbits\", .stapsdt.base, comdat",
                ".weak _.stapsdt.base",
                ".hidden _.stapsdt.base",
                "_.stapsdt.base: .space 1",
                ".size _.stapsdt.base, 1",
                ".popsection",
                ".endif",
                ".ifndef _.anyprobe.probes_page",
                ".pushsection .probes, \"awGR\", \"progbits\", _.anyprobe.probes_page, comdat",
                ".balign 65536",
                ".weak _.anyprobe.probes_page",
                ".hidden _.anyprobe.probes_page",
                "_.anyprobe.probes_page: .space 1",
                ".size _.anyprobe.probes_page, 1",
                ".popsection",
                ".endif",
                sema = sym $sema,
                $($operands)*
                // AArch64 registers render as `x0`, the SDT spelling.
                options(readonly, nostack, preserves_flags),
            )
        }
    };
}

/// Reads a semaphore with a direct PC-relative load.
///
/// A Rust load of a static from an rlib goes through the GOT, since rustc
/// cannot rule out the rlib ending up in a dylib where the symbol could be
/// preempted: a second dependent load on every check. `sys/sdt.h` avoids it
/// with hidden-visibility semaphores; Rust cannot declare visibility, so the
/// load is written in `asm!` with a `sym` operand, which the linker resolves
/// directly.
///
/// A Rust `dylib` cannot link a direct reference to a symbol it exports, so
/// `--cfg anyprobe_dylib` switches back to the plain Rust load.
#[cfg(all(target_arch = "x86_64", not(anyprobe_dylib)))]
#[doc(hidden)]
#[macro_export]
macro_rules! __anyprobe_sema_load {
    ($sema:path) => {{
        let v: u32;
        // SAFETY: reads the two bytes of a `static` semaphore, which is valid
        // for the whole program. Nothing is written.
        unsafe {
            ::core::arch::asm!(
                "movzwl {sema}(%rip), {v:e}",
                sema = sym $sema,
                v = out(reg) v,
                options(att_syntax, readonly, nostack, preserves_flags)
            );
        }
        v
    }};
}

#[cfg(all(target_arch = "aarch64", not(anyprobe_dylib)))]
#[doc(hidden)]
#[macro_export]
macro_rules! __anyprobe_sema_load {
    ($sema:path) => {{
        let v: u32;
        // SAFETY: reads the two bytes of a `static` semaphore, which is valid
        // for the whole program. Nothing is written.
        unsafe {
            ::core::arch::asm!(
                "adrp {v:x}, {sema}",
                "ldrh {v:w}, [{v:x}, :lo12:{sema}]",
                sema = sym $sema,
                v = out(reg) v,
                options(readonly, nostack, preserves_flags)
            );
        }
        v
    }};
}

#[cfg(anyprobe_dylib)]
#[doc(hidden)]
#[macro_export]
macro_rules! __anyprobe_sema_load {
    ($sema:path) => {
        $sema.load(::core::sync::atomic::Ordering::Relaxed)
    };
}
