#[anyprobe::probe(provider = "t")]
const fn double(x: u32) -> u32 {
    x * 2
}

fn main() {}
