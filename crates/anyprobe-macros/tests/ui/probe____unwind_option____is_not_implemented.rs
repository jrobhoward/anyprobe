#[anyprobe::probe(provider = "t", unwind)]
fn hit(x: u64) {}

fn main() {}
