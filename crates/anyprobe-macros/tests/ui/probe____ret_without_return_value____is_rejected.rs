#[anyprobe::probe(provider = "t", ret = debug)]
fn hit(id: u64) {}

fn main() {}
