struct Conn;

impl Conn {
    #[anyprobe::probe(provider = "t", native(self))]
    fn hit(&self) {}
}

fn main() {}
