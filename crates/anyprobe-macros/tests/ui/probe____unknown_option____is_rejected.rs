#[anyprobe::probe(provider = "t", json(x))]
fn hit(x: u64) {
    let _ = x;
}

fn main() {}
