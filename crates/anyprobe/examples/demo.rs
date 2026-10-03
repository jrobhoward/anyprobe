//! A program to attach a tracer to by hand. It runs until stopped and
//! handles one made-up order per interval, printing each one. The
//! walkthroughs in `docs/usage/` attach to it.
//!
//! Usage: `demo [INTERVAL_MS]`, default 1000.
//!
//! Probes, all in the provider `demo`:
//!
//! - `tick(n)`: from `probes!`, at the start of each iteration.
//! - `checkout__entry(id, customer, order)`, `checkout__return(ret)`: from
//!   `#[probe]` on `checkout`. `order` is its `{:?}` text and `ret` the
//!   total in cents.

use std::time::Duration;

anyprobe::probes! {
    provider = "demo";

    /// An iteration of the main loop started.
    fn tick(n: u64);
}

#[derive(Debug)]
struct Order {
    items: u32,
    express: bool,
}

#[anyprobe::probe(provider = "demo", debug(order), ret = native)]
fn checkout(id: u64, customer: &str, order: &Order) -> u64 {
    let shipping = if order.express { 1500 } else { 500 };
    // Every fifth order ships free.
    let shipping = if id % 5 == 4 { 0 } else { shipping };
    u64::from(order.items) * 1250 + shipping
}

fn main() {
    let interval = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1000);
    println!("pid={} backend={}", std::process::id(), anyprobe::BACKEND);
    // The GUID an ETW session enables; `cargo anyprobe wprp` writes it too.
    #[cfg(windows)]
    println!("etw-guid={}", anyprobe::__private::etw::guid_string("demo"));
    let customers = ["ana", "bo", "chen"];
    let mut n = 0u64;
    loop {
        if tick::enabled() {
            tick::fire(n);
        }
        let customer = customers[(n % 3) as usize];
        let order = Order {
            items: (n % 3) as u32 + 1,
            express: n % 2 == 1,
        };
        let total = checkout(n, customer, &order);
        println!("order {n}: {customer}, {order:?}, {total} cents");
        std::thread::sleep(Duration::from_millis(interval));
        n += 1;
    }
}
