anyprobe::probes! {
    provider = "app";
    fn hit((a, b): (u8, u8));
}

fn main() {}
