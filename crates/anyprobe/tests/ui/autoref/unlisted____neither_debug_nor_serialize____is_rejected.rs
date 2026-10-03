struct Opts;

#[anyprobe::probe(provider = "t")]
fn hit(id: u64, opts: Opts) {}

fn main() {}
