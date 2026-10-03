//! Drives the spike's probes so a tracer has something to attach to.
//!
//! Usage: `anyprobe-spike [ITERATIONS] [INTERVAL_MS]`. `ITERATIONS` of 0 (the
//! default) runs until killed. Each iteration calls `work` from two inlined
//! call sites and `work_generic` for two types, so every probe has several
//! sites. Changes in whether `work__entry` is enabled are printed as
//! `entry-enabled=true|false`, which is how CI confirms that attaching a
//! tracer reached the process.

use std::io::Write;
use std::time::Duration;

fn main() {
    let mut args = std::env::args().skip(1);
    let iterations: u64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(0);
    let interval = Duration::from_millis(args.next().and_then(|a| a.parse().ok()).unwrap_or(100));

    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "pid={}", std::process::id());
    let _ = writeln!(out, "backend={}", anyprobe_spike::BACKEND);
    #[cfg(windows)]
    {
        let (name, guid) = anyprobe_spike::etw_provider();
        let _ = writeln!(out, "etw-provider={name} etw-guid={guid}");
    }
    if let Err(e) = anyprobe_spike::register() {
        let _ = writeln!(out, "register-error={e}");
    }
    let _ = out.flush();

    let mut enabled = false;
    let mut checksum = 0u64;
    let mut i = 0u64;
    while iterations == 0 || i < iterations {
        checksum ^= anyprobe_spike::work(i, "first-site");
        checksum ^= anyprobe_spike::work(i, "second-site");
        checksum ^= anyprobe_spike::work_generic(i as u32);
        checksum ^= anyprobe_spike::work_generic(i);

        let now = anyprobe_spike::entry_enabled();
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
