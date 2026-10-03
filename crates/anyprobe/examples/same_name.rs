//! Three `#[probe]` functions whose probes share one name with different
//! argument types, for `spike/scripts/attach-macos-attr.sh`.
//!
//! `Foo::new(x: u32)`, `Bar::new()` and `other::new(label: &str)` all default
//! to the probe names `new__entry` and `new__return`. ld64 writes one DOF
//! entry per containing function, each with its own argument types; the
//! script checks that DTrace attaches to all three and reads each one's
//! arguments as that function declares them.
//!
//! Usage: `same_name [ITERATIONS] [INTERVAL_MS]` (defaults 1 and 20). Each
//! iteration calls each function once, with the iteration number as `x`.

use std::io::Write;
use std::time::Duration;

struct Foo(u32);
struct Bar;

impl Foo {
    #[anyprobe::probe(provider = "same_name")]
    fn new(x: u32) -> Foo {
        Foo(x)
    }
}

impl Bar {
    #[anyprobe::probe(provider = "same_name")]
    fn new() -> Bar {
        Bar
    }
}

mod other {
    #[anyprobe::probe(provider = "same_name")]
    pub(crate) fn new(label: &str) -> usize {
        label.len()
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let iterations: u64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(1);
    let interval = Duration::from_millis(args.next().and_then(|a| a.parse().ok()).unwrap_or(20));

    let mut sum = 0u64;
    for i in 0..iterations {
        sum += u64::from(Foo::new(i as u32).0);
        let Bar = Bar::new();
        sum += other::new("label") as u64;
        std::thread::sleep(interval);
    }
    let _ = writeln!(std::io::stdout(), "done sum={sum}");
}
