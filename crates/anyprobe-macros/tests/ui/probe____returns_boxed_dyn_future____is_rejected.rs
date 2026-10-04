use std::future::Future;
use std::pin::Pin;

// The shape `#[async_trait]` gives a trait method before `#[probe]` sees it.
#[anyprobe::probe(provider = "t")]
fn fetch(id: u64) -> Pin<Box<dyn Future<Output = u64> + Send + 'static>> {
    Box::pin(async move { id })
}

fn main() {}
