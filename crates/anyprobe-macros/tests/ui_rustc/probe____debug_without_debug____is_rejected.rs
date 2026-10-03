struct Opts;

#[anyprobe::probe(provider = "t", debug(opts))]
fn hit(opts: Opts) {}

fn main() {}
