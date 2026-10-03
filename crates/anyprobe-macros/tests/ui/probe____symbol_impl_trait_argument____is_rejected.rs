#[anyprobe::probe(provider = "t", symbol)]
fn to_byte(x: impl Into<u8>) -> u8 {
    x.into()
}

fn main() {}
