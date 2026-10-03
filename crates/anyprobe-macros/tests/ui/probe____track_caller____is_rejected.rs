#[anyprobe::probe(provider = "t")]
#[track_caller]
fn check(ok: bool) {
    assert!(ok);
}

fn main() {}
