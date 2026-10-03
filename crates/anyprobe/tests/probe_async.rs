//! `#[probe]` on `async fn`, and the `unwind` and `symbol` options, from a
//! user's point of view with no tracer attached: every supported shape
//! compiles, and behaves as it did without the attribute.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(non_snake_case)]

use std::fmt::Debug;
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::{Pin, pin};
use std::task::{Context, Poll, Waker};

/// Polls to completion. The futures here make progress on every poll, so
/// the waker does nothing.
fn block_on<F: Future>(future: F) -> F::Output {
    let mut cx = Context::from_waker(Waker::noop());
    let mut future = pin!(future);
    loop {
        if let Poll::Ready(out) = future.as_mut().poll(&mut cx) {
            return out;
        }
    }
}

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

fn yield_once() -> YieldOnce {
    YieldOnce(false)
}

fn assert_send<F: Future + Send>(_: &F) {}

#[anyprobe::probe(provider = "t", ret = native)]
async fn natives(id: u64, path: &str) -> u64 {
    yield_once().await;
    id * 10 + path.len() as u64
}

#[anyprobe::probe(provider = "t")]
async fn fallible(x: &str) -> Result<u32, Box<dyn std::error::Error + Send + Sync>> {
    if x.is_empty() {
        return Ok(0);
    }
    yield_once().await;
    let n: u32 = x.parse()?;
    Ok(n)
}

#[anyprobe::probe(provider = "t", ret = debug)]
async fn coerced(flag: bool) -> Box<dyn Debug + Send> {
    if flag {
        return Box::new(1u8);
    }
    Box::new("s")
}

#[anyprobe::probe(provider = "t")]
async fn borrowed<'a>(x: &'a str) -> &'a str {
    &x[1..]
}

#[anyprobe::probe(provider = "t")]
async fn elided(x: &str) -> &str {
    yield_once().await;
    &x[1..]
}

#[anyprobe::probe(provider = "t")]
async fn opaque(n: u8) -> impl Debug {
    n
}

#[anyprobe::probe(provider = "t", debug(u))]
async fn generic<U: Clone + Debug + Send + Sync>(u: &U, n: usize) -> Vec<U> {
    yield_once().await;
    vec![u.clone(); n]
}

#[anyprobe::probe(provider = "t", debug(s))]
async fn by_value(s: String) -> String {
    yield_once().await;
    s
}

#[anyprobe::probe(provider = "t")]
async fn unit_early(x: u8) {
    if x > 1 {
        return;
    }
    yield_once().await;
}

#[anyprobe::probe(provider = "t", debug(a, b, c, d))]
async fn collapsed(a: &str, b: &str, c: &str, d: Option<u8>) -> usize {
    a.len() + b.len() + c.len() + usize::from(d.unwrap_or(0))
}

#[derive(Debug)]
struct Store(Vec<u8>);

impl Store {
    #[anyprobe::probe(provider = "t", debug(self))]
    async fn push(&mut self, v: u8) -> &mut Vec<u8> {
        yield_once().await;
        self.0.push(v);
        &mut self.0
    }

    #[anyprobe::probe(provider = "t")]
    async fn into_inner(self) -> Vec<u8> {
        self.0
    }
}

trait Source {
    async fn read(&self) -> u8;
}

impl Source for u8 {
    #[anyprobe::probe(provider = "t")]
    async fn read(&self) -> u8 {
        *self
    }
}

#[anyprobe::probe(provider = "t", unwind)]
async fn slow(id: u64) -> u64 {
    yield_once().await;
    id
}

#[anyprobe::probe(provider = "t", unwind)]
async fn panics_async(id: u64) -> u64 {
    yield_once().await;
    assert!(id != 1, "odd {id}");
    id
}

#[anyprobe::probe(provider = "t", unwind)]
fn may_panic(id: u64) -> u64 {
    assert!(id != 1, "odd {id}");
    id
}

#[anyprobe::probe(provider = "t", unwind, ret = native)]
fn unwind_and_ret(id: u64) -> u64 {
    id + 1
}

#[anyprobe::probe(provider = "anyprobe_test", symbol)]
fn exported(id: u64) -> u64 {
    id + 1
}

#[anyprobe::probe(provider = "anyprobe_test", symbol = "anyprobe_test_custom_symbol")]
fn exported_custom(id: u64) -> u64 {
    id + 2
}

#[test]
fn async_fn____native_arguments____returns_its_value() {
    assert_eq!(block_on(natives(3, "/ab")), 33);
}

#[test]
fn async_fn____question_mark____converts_errors() {
    assert_eq!(block_on(fallible("")).unwrap(), 0);
    assert_eq!(block_on(fallible("12")).unwrap(), 12);
    assert!(block_on(fallible("x")).is_err());
}

#[test]
fn async_fn____returns_of_different_types____coerce_to_the_declared_type() {
    assert_eq!(format!("{:?}", block_on(coerced(true))), "1");
    assert_eq!(format!("{:?}", block_on(coerced(false))), "\"s\"");
}

#[test]
fn async_fn____borrows____return_borrows_of_arguments() {
    assert_eq!(block_on(borrowed("abc")), "bc");
    assert_eq!(block_on(elided("abc")), "bc");
}

#[test]
fn async_fn____impl_trait_return____is_inferred() {
    assert_eq!(format!("{:?}", block_on(opaque(4))), "4");
}

#[test]
fn async_fn____generic_and_by_value____work() {
    assert_eq!(block_on(generic(&7u8, 3)), vec![7, 7, 7]);
    assert_eq!(block_on(by_value("v".to_owned())), "v");
}

#[test]
fn async_fn____early_return_of_unit____completes() {
    block_on(unit_early(2));
    block_on(unit_early(0));
}

#[test]
fn async_fn____too_many_values____collapse_and_still_run() {
    assert_eq!(block_on(collapsed("a", "bb", "ccc", Some(4))), 10);
}

#[test]
fn async_fn____methods_and_trait_impls____work() {
    let mut store = Store(Vec::new());
    assert_eq!(block_on(store.push(5)), &mut vec![5]);
    assert_eq!(block_on(store.into_inner()), vec![5]);
    assert_eq!(block_on(9u8.read()), 9);
}

#[test]
fn async_fn____send_arguments____give_a_send_future() {
    let path = String::from("/p");
    let f = natives(1, &path);
    assert_send(&f);
    let g = generic(&1u8, 2);
    assert_send(&g);
    let h = slow(3);
    assert_send(&h);
    drop((f, g, h));
}

#[test]
fn unwind____async_fn_dropped_mid_await____does_not_disturb_the_caller() {
    let mut cx = Context::from_waker(Waker::noop());
    let mut future = Box::pin(slow(5));
    assert!(future.as_mut().poll(&mut cx).is_pending());
    // Cancelled: the guard inside the future drops here.
    drop(future);
    assert_eq!(block_on(slow(6)), 6);
}

#[test]
fn unwind____async_fn_panicking____still_propagates_the_panic() {
    let caught = catch_unwind(AssertUnwindSafe(|| block_on(panics_async(1))));
    assert!(caught.is_err());
    assert_eq!(block_on(panics_async(2)), 2);
}

#[test]
fn unwind____sync_fn_panicking____still_propagates_the_panic() {
    let caught = catch_unwind(|| may_panic(1));
    let message = caught.unwrap_err();
    assert_eq!(message.downcast_ref::<String>().unwrap(), "odd 1");
    assert_eq!(may_panic(2), 2);
    assert_eq!(unwind_and_ret(2), 3);
}

#[test]
fn symbol____default_and_custom_names____are_exported() {
    // Calling through the exported names shows the symbols exist and link.
    unsafe extern "Rust" {
        #[link_name = "anyprobe_test__exported"]
        fn by_default_name(id: u64) -> u64;
        #[link_name = "anyprobe_test_custom_symbol"]
        fn by_custom_name(id: u64) -> u64;
    }
    // SAFETY: both names are exported by `#[probe(symbol)]` on Rust
    // functions with exactly these signatures, defined above.
    let (a, b) = unsafe { (by_default_name(1), by_custom_name(1)) };
    assert_eq!((a, b), (2, 3));
    assert_eq!((exported(1), exported_custom(1)), (2, 3));
}
