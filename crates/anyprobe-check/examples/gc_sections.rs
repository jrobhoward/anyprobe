//! Links `anyprobe-check` and calls only [`anyprobe_check::fire_all`], so
//! `--gc-sections` drops the library's other probed functions. Each dropped
//! function's SDT note must still name an address in an executable segment;
//! `spike/scripts/attach-linux-attr.sh` checks that on the built binary.

fn main() {
    let text = std::env::args().next().unwrap_or_default();
    anyprobe_check::fire_all(&text, text.as_bytes());
}
