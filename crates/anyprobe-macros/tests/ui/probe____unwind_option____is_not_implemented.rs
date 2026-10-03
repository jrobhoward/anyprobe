#[anyprobe::probe(provider = "t", unwind)]
fn hit(x: u64) {
    let _ = x;
}

fn main() {}
