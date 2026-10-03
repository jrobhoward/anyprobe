#[anyprobe::probe(provider = "t", symbol)]
async fn handle(id: u64) -> u64 {
    id
}

fn main() {}
