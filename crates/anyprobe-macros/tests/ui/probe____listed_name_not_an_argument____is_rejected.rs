#[anyprobe::probe(provider = "t", skip(conn))]
fn hit(id: u64) {}

fn main() {}
