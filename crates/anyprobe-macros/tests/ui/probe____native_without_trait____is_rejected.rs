struct RowId(u64);

#[anyprobe::probe(provider = "t", native(row))]
fn hit(row: RowId) {}

fn main() {}
