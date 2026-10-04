#[anyprobe::probe(provider = "t", symbol)]
#[unsafe(export_name = "mine")]
fn hit(id: u64) -> u64 {
    id
}

fn main() {}
