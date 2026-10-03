#[anyprobe::probe(provider = "t", symbol)]
fn first<T: Copy>(items: &[T]) -> T {
    items[0]
}

fn main() {}
