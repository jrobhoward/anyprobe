use std::future::Future;

#[anyprobe::probe(provider = "t")]
fn fetch(id: u64) -> impl Future<Output = u64> {
    async move { id }
}

fn main() {}
