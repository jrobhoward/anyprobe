#[anyprobe::probe(provider = "t")]
fn sum((a, b): (u32, u32)) -> u32 {
    a + b
}

fn main() {}
