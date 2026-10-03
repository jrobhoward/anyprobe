#[anyprobe::probe(provider = "t", ret = json)]
fn hit(id: u64) -> u64 {
    id
}

fn main() {}
