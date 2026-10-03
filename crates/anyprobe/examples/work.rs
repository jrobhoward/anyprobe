//! The spike's driver program, on the real crate.
//!
//! Same provider (`spike`), probes, labels and command line as
//! `anyprobe-spike`, so `spike/scripts/attach-*` check this example with
//! `ATTACH_CRATE=anyprobe`. Usage: `work [ITERATIONS] [INTERVAL_MS]`;
//! `ITERATIONS` of 0 (the default) runs until killed. Each iteration calls
//! `work` from two inlined call sites and `work_generic` for two types.
//! Changes in whether `work__entry` is enabled are printed as
//! `entry-enabled=true|false`.

use std::io::Write;
use std::time::Duration;

anyprobe::probes! {
    provider = "spike";

    /// Entry to `work`: the iteration and a label naming the call site.
    fn work__entry(id: u64, label: &str);
    /// Return from `work`: the iteration and the result.
    fn work__return(id: u64, result: u64);
}

#[inline(always)]
fn work(id: u64, label: &str) -> u64 {
    if work__entry::enabled() {
        fire_entry(id, label);
    }
    let result = compute(id, label);
    if work__return::enabled() {
        fire_return(id, result);
    }
    result
}

#[inline(never)]
fn work_generic<T: Into<u64> + Copy>(id: T) -> u64 {
    if work__entry::enabled() {
        fire_entry_generic(id);
    }
    let result = compute(id.into(), "generic");
    if work__return::enabled() {
        fire_return(id.into(), result);
    }
    result
}

#[inline(always)]
fn compute(id: u64, label: &str) -> u64 {
    id.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ label.len() as u64
}

#[cold]
#[inline(never)]
fn fire_entry(id: u64, label: &str) {
    work__entry::fire(id, label);
}

#[cold]
#[inline(never)]
fn fire_entry_generic<T: Into<u64> + Copy>(id: T) {
    work__entry::fire(id.into(), std::any::type_name::<T>());
}

#[cold]
#[inline(never)]
fn fire_return(id: u64, result: u64) {
    work__return::fire(id, result);
}

fn main() {
    let mut args = std::env::args().skip(1);
    let iterations: u64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(0);
    let interval = Duration::from_millis(args.next().and_then(|a| a.parse().ok()).unwrap_or(100));

    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "pid={}", std::process::id());
    let _ = writeln!(out, "backend={}", anyprobe::BACKEND);
    if let Err(e) = anyprobe::registration() {
        let _ = writeln!(out, "register-error={e}");
    }
    #[cfg(windows)]
    {
        let guid = anyprobe::__private::etw::guid_string(work__entry::PROVIDER);
        let _ = writeln!(
            out,
            "etw-provider={} etw-guid={guid}",
            work__entry::PROVIDER
        );
    }
    let _ = out.flush();

    let mut enabled = false;
    let mut checksum = 0u64;
    let mut i = 0u64;
    while iterations == 0 || i < iterations {
        checksum ^= work(i, "first-site");
        checksum ^= work(i, "second-site");
        checksum ^= work_generic(i as u32);
        checksum ^= work_generic(i);

        let now = work__entry::enabled();
        if now != enabled {
            enabled = now;
            let _ = writeln!(out, "entry-enabled={enabled} iteration={i}");
            let _ = out.flush();
        }
        i += 1;
        std::thread::sleep(interval);
    }
    let _ = writeln!(out, "done checksum={checksum:#x}");
}
