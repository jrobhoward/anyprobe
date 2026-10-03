#[anyprobe::probe(provider = "t", ret = debug)]
fn hit(id: u64) {
    let _ = id;
}

fn main() {}
