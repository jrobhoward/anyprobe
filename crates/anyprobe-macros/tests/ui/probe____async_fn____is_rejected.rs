#[anyprobe::probe(provider = "t")]
async fn handle(id: u64) -> u64 {
    id
}

fn main() {}
