#[anyprobe::probe(provider = "t", symbol = "my-symbol")]
fn hit(id: u64) -> u64 {
    id
}

fn main() {}
