#[derive(Debug)]
struct Opts;

#[anyprobe::probe(provider = "t", debug(opts), skip(opts))]
fn hit(opts: Opts) {
    let _ = opts;
}

fn main() {}
