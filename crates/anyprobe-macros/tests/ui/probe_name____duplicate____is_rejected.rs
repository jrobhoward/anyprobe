anyprobe::probes! {
    provider = "app";
    fn hit();
    fn hit(x: u8);
}

fn main() {}
