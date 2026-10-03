#[anyprobe::probe(provider = "t", name = "a_name_that_is_long_enough_to_push_the_return_probe_past_63")]
fn hit() {}

fn main() {}
