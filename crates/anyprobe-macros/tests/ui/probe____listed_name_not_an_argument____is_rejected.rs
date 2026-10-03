#[anyprobe::probe(provider = "t", skip(conn))]
fn hit(id: u64) {
    let _ = id;
}

fn main() {}
