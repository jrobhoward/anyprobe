#[anyprobe::probe(provider = "t", symbol)]
#[inline]
fn hit(id: u64) -> u64 {
    id
}

fn main() {}
