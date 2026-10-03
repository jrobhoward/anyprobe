#[anyprobe::probe(provider = "t")]
fn fail(code: i32) -> ! {
    std::process::exit(code)
}

fn main() {}
