#[anyprobe::probe(provider = "t", unwind, unwind)]
fn hit(id: u64) -> u64 {
    id
}

fn main() {}
