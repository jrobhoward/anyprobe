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

use core::sync::atomic::AtomicU16;

pub(crate) const NAME: &str = "linux-sdt";

// One semaphore per probe name. The note stores its link-time address and the
// tracer writes to it, so it lives in `.probes` like the semaphores `sys/sdt.h`
// produces.
#[unsafe(link_section = ".probes")]
#[used]
static SEMA_WORK_ENTRY: AtomicU16 = AtomicU16::new(0);

#[unsafe(link_section = ".probes")]
#[used]
static SEMA_WORK_RETURN: AtomicU16 = AtomicU16::new(0);

/// Emits one SDT probe site: the `nop` the tracer patches, the note that
/// describes it, and (once per object) the `.stapsdt.base` section tools use
/// to detect prelink adjustments.
///
/// Labels are numeric local labels so that every copy of the block, from
/// inlining or monomorphization, assembles to another site rather than a
/// duplicate symbol. `_.stapsdt.base` is the one named symbol, guarded by
/// `.ifndef`.
///
/// `.stapsdt.base` is executable (`"axGR"`, where `sys/sdt.h` uses `"aG"`) so
/// that it lands in the same segment as the probe sites. perf turns a site's
/// address into a file offset using the base section's offset, which is only
/// right if the two share a segment's address-to-offset delta. GNU ld gives
/// every segment the same delta, so `"aG"` works there; lld does not, and with
/// rust-lld (the default linker on x86_64 Linux) perf put every probe 0x1000
/// bytes from its `nop`.
///
/// Sites are `readonly`, not `nomem`: an attached tracer reads memory through
/// pointer arguments at the `nop`. Under `nomem` the compiler may sink or drop
/// a store to a buffer whose only reader is the probe — such as an encoded
/// argument written just before the site.
macro_rules! sdt_site {
    ($provider:literal, $probe:literal, $sema:path, $args:literal, $($operands:tt)*) => {
        // SAFETY: the block executes a single `nop` and otherwise only emits
        // data into non-executed sections. The operands are read-only register
        // inputs that the `nop` leaves untouched.
        #[allow(named_asm_labels)]
        unsafe {
            core::arch::asm!(
                "990: nop",
                ".pushsection .note.stapsdt, \"\", \"note\"",
                ".balign 4",
                ".4byte 992f-991f, 994f-993f, 3",
                "991: .asciz \"stapsdt\"",
                "992: .balign 4",
                "993: .8byte 990b",
                ".8byte _.stapsdt.base",
                ".8byte {sema}",
                concat!(".asciz \"", $provider, "\""),
                concat!(".asciz \"", $probe, "\""),
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
                sema = sym $sema,
                $($operands)*
            )
        }
    };
}

// x86-64 needs AT&T operand syntax: SDT argument strings name registers as
// `%rdi`, which is what `{id}` renders to under `att_syntax`.
#[cfg(target_arch = "x86_64")]
macro_rules! probe_work_entry {
    ($id:expr, $ptr:expr, $len:expr) => {
        sdt_site!(
            "spike", "work__entry", SEMA_WORK_ENTRY, "8@{id} 8@{ptr} 8@{len}",
            id = in(reg) $id, ptr = in(reg) $ptr, len = in(reg) $len,
            options(att_syntax, readonly, nostack, preserves_flags)
        )
    };
}

#[cfg(target_arch = "x86_64")]
macro_rules! probe_work_return {
    ($id:expr, $result:expr) => {
        sdt_site!(
            "spike", "work__return", SEMA_WORK_RETURN, "8@{id} 8@{result}",
            id = in(reg) $id, result = in(reg) $result,
            options(att_syntax, readonly, nostack, preserves_flags)
        )
    };
}

// AArch64 registers render as `x0`, which is the SDT spelling already.
#[cfg(target_arch = "aarch64")]
macro_rules! probe_work_entry {
    ($id:expr, $ptr:expr, $len:expr) => {
        sdt_site!(
            "spike", "work__entry", SEMA_WORK_ENTRY, "8@{id} 8@{ptr} 8@{len}",
            id = in(reg) $id, ptr = in(reg) $ptr, len = in(reg) $len,
            options(readonly, nostack, preserves_flags)
        )
    };
}

#[cfg(target_arch = "aarch64")]
macro_rules! probe_work_return {
    ($id:expr, $result:expr) => {
        sdt_site!(
            "spike", "work__return", SEMA_WORK_RETURN, "8@{id} 8@{result}",
            id = in(reg) $id, result = in(reg) $result,
            options(readonly, nostack, preserves_flags)
        )
    };
}

/// Reads a semaphore with a direct PC-relative load.
///
/// A Rust load of a static from an rlib goes through the GOT, since rustc
/// cannot rule out the rlib ending up in a dylib where the symbol could be
/// preempted. That is a second dependent load on every check. `sys/sdt.h`
/// avoids it with hidden-visibility semaphores; Rust cannot declare
/// visibility, so the load is written in `asm!` with a `sym` operand, which
/// the linker resolves directly.
///
/// A Rust `dylib` cannot link a direct reference to a symbol it exports, so
/// `--cfg anyprobe_dylib` switches back to the plain Rust load.
#[cfg(all(target_arch = "x86_64", not(anyprobe_dylib)))]
macro_rules! sema_load {
    ($sema:path) => {{
        let v: u32;
        // SAFETY: reads the two bytes of a `static` semaphore, which is valid
        // for the whole program. Nothing is written.
        unsafe {
            core::arch::asm!(
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
macro_rules! sema_load {
    ($sema:path) => {{
        let v: u32;
        // SAFETY: reads the two bytes of a `static` semaphore, which is valid
        // for the whole program. Nothing is written.
        unsafe {
            core::arch::asm!(
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
macro_rules! sema_load {
    ($sema:path) => {
        $sema.load(core::sync::atomic::Ordering::Relaxed)
    };
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
compile_error!("the Linux SDT backend supports x86_64 and aarch64 only");

pub(crate) fn register() -> std::io::Result<()> {
    Ok(())
}

#[inline(always)]
pub(crate) fn entry_enabled() -> bool {
    sema_load!(SEMA_WORK_ENTRY) != 0
}

#[inline(always)]
pub(crate) fn return_enabled() -> bool {
    sema_load!(SEMA_WORK_RETURN) != 0
}

#[inline(always)]
pub(crate) fn fire_entry(id: u64, label: &str) {
    probe_work_entry!(id, label.as_ptr(), label.len() as u64);
}

#[inline(always)]
pub(crate) fn fire_return(id: u64, result: u64) {
    probe_work_return!(id, result);
}
