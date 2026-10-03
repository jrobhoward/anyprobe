#[derive(Debug)]
struct Opts;

#[anyprobe::probe(provider = "t", serde(opts))]
fn hit(opts: Opts) {}

fn main() {}
