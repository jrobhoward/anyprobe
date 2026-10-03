//! `#[probe]` on `async fn`, with `unwind`, and with `symbol`, for
//! `spike/scripts/attach-macos-attr.sh`.
//!
//! Usage: `attr_async [ITERATIONS] [INTERVAL_MS]` (defaults 1 and 20). Each
//! iteration, with `i` the iteration number:
//!
//! - polls two `fetch` calls, `fetch(2i, "/a")` and `fetch(2i + 1, "/bb")`,
//!   so that both have started before either finishes, and the second
//!   finishes first. Their return probes can only be paired with their entry
//!   probes by the invocation id. `fetch` returns `id * 10 + path.len()`.
//! - polls `slow(i)` once and drops it: cancelled, so its unwind probe fires
//!   with `panicking` 0 and its return probe does not.
//! - calls `may_panic(i)`, which panics for odd `i` (caught here): its
//!   unwind probe fires for odd `i`, its return probe for even.
//! - calls `exported(i)`, which has `symbol`: exported as
//!   `attr_async__exported`, so DTrace's `pid` provider reaches it by name.
//!
//! Probes, all in the provider `attr_async`: `fetch__entry(invocation, id,
//! path)`, `fetch__return(invocation, ret)`, `slow__entry(invocation, id)`,
//! `slow__return(invocation)`, `slow__unwind(invocation, panicking)`,
//! `may_panic__entry(id)`, `may_panic__return()`, `may_panic__unwind()`,
//! `exported__entry(id)`, `exported__return()`.

use std::future::Future;
use std::io::Write;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::{Pin, pin};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

/// Pending on its first poll, ready on the second.
struct YieldOnce(bool);

impl Future for YieldOnce {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
        if self.0 {
            Poll::Ready(())
        } else {
            self.0 = true;
            Poll::Pending
        }
    }
}

#[anyprobe::probe(provider = "attr_async", ret = native)]
async fn fetch(id: u64, path: &str) -> u64 {
    YieldOnce(false).await;
    id * 10 + path.len() as u64
}

#[anyprobe::probe(provider = "attr_async", unwind)]
async fn slow(id: u64) -> u64 {
    YieldOnce(false).await;
    id
}

#[anyprobe::probe(provider = "attr_async", unwind)]
fn may_panic(id: u64) -> u64 {
    assert!(id.is_multiple_of(2), "odd {id}");
    id
}

#[anyprobe::probe(provider = "attr_async", symbol)]
fn exported(id: u64) -> u64 {
    id + 1
}

fn main() {
    let mut args = std::env::args().skip(1);
    let iterations: u64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(1);
    let interval = Duration::from_millis(args.next().and_then(|a| a.parse().ok()).unwrap_or(20));
    // `may_panic` panics on purpose; its message is noise here.
    std::panic::set_hook(Box::new(|_| {}));

    let mut cx = Context::from_waker(Waker::noop());
    let mut sum = 0u64;
    for i in 0..iterations {
        let mut a = pin!(fetch(2 * i, "/a"));
        let mut b = pin!(fetch(2 * i + 1, "/bb"));
        // Both start (entry probes), then `b` finishes before `a`.
        assert!(a.as_mut().poll(&mut cx).is_pending());
        assert!(b.as_mut().poll(&mut cx).is_pending());
        if let Poll::Ready(v) = b.as_mut().poll(&mut cx) {
            sum += v;
        }
        if let Poll::Ready(v) = a.as_mut().poll(&mut cx) {
            sum += v;
        }

        let mut s = Box::pin(slow(i));
        assert!(s.as_mut().poll(&mut cx).is_pending());
        drop(s);

        if let Ok(v) = catch_unwind(AssertUnwindSafe(|| may_panic(i))) {
            sum += v;
        }
        sum += exported(i);
        std::thread::sleep(interval);
    }
    let _ = writeln!(std::io::stdout(), "done sum={sum}");
}
