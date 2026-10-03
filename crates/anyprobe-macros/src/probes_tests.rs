//! Tests for `probes!` expansion and the per-backend strings it computes.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(non_snake_case)]

use super::*;

fn expand_str(input: &str) -> Result<String, String> {
    let tokens: TokenStream = input.parse().unwrap();
    expand(tokens)
        .map(|t| t.to_string())
        .map_err(|e| e.to_string())
}

#[test]
fn dtrace_symbols____spike_probe____match_dtrace_h_output() {
    // `dtrace -h` on macOS 27 for
    // `provider spike { probe work__entry(uint64_t, char *, uint64_t); };`
    let [probe, enabled, stability, typedefs] =
        dtrace_symbols("spike", "work__entry", &["uint64_t", "char *", "uint64_t"]);
    assert_eq!(
        probe,
        "__dtrace_probe$spike$work__entry$v1$75696e7436345f74$63686172202a$75696e7436345f74"
    );
    assert_eq!(enabled, "__dtrace_isenabled$spike$work__entry$v1");
    assert_eq!(
        stability,
        "__dtrace_stability$spike$v1$1_1_0_1_1_0_1_1_0_1_1_0_1_1_0"
    );
    assert_eq!(typedefs, "__dtrace_typedefs$spike$v2");
}

#[test]
fn dtrace_symbols____no_arguments____have_no_type_suffix() {
    let [probe, ..] = dtrace_symbols("httpx", "a__b", &[]);
    assert_eq!(probe, "__dtrace_probe$httpx$a__b$v1");
}

#[test]
fn sdt_format____mixed_sizes____numbers_operands_in_order() {
    assert_eq!(sdt_format(&["8", "-8", "8"]), "8@{a0} -8@{a1} 8@{a2}");
    assert_eq!(sdt_format(&[]), "");
}

#[test]
fn expand____empty_block____emits_nothing() {
    assert_eq!(expand_str("provider = \"app\";").unwrap(), "");
}

#[test]
fn expand____probe____defines_provider_and_module() {
    let out = expand_str("provider = \"app\"; pub fn hit(id: u64, path: &str);").unwrap();
    assert!(out.contains("mod __anyprobe_hit"), "{out}");
    assert!(
        out.contains("define_provider ! (__ANYPROBE_PROVIDER , \"app\")"),
        "{out}"
    );
    assert!(out.contains("pub mod hit"), "{out}");
    assert!(out.contains("pub use __anyprobe_hit :: hit ;"), "{out}");
    assert!(out.contains("params : [id : u64 , path : & str]"), "{out}");
    assert!(out.contains("sdt : \"8@{a0} 8@{a1} 8@{a2}\""), "{out}");
    assert!(out.contains("\"x0\" = (id as u64)"), "{out}");
    assert!(out.contains("\"rsi\" = (path . as_ptr ())"), "{out}");
    assert!(out.contains("add_str8 (\"path\" , (path) , Utf8)"), "{out}");
}

#[test]
fn expand____str_with_lifetime____declares_plain_str() {
    let out = expand_str("provider = \"app\"; fn hit(path: &'static str);").unwrap();
    assert!(out.contains("params : [path : & str]"), "{out}");
}

#[test]
fn expand____docs____move_onto_the_module() {
    let out = expand_str("provider = \"app\"; /// Fired on a hit.\nfn hit();").unwrap();
    assert!(out.contains("# [doc = \" Fired on a hit.\"]"), "{out}");
}

#[test]
fn provider_name____not_given____is_the_crate_name() {
    assert_eq!(provider_name(None, Some("my_crate")).unwrap(), "my_crate");
}

#[test]
fn provider_name____given____wins_over_the_crate_name() {
    let lit: LitStr = syn::parse_quote!("app");
    assert_eq!(provider_name(Some(&lit), Some("my_crate")).unwrap(), "app");
}

#[test]
fn provider_name____no_crate_name____asks_for_one() {
    let err = provider_name(None, None).unwrap_err().to_string();
    assert!(err.contains("CARGO_CRATE_NAME is not set"), "{err}");
}

#[test]
fn provider_name____crate_name_ends_in_digit____suggests_setting_one() {
    let err = provider_name(None, Some("http2")).unwrap_err().to_string();
    assert!(err.contains("must not end with a digit"), "{err}");
    assert!(err.contains("the default is the crate name"), "{err}");
}

#[test]
fn expand____unsupported_type____names_the_accepted_types() {
    let err = expand_str("provider = \"app\"; fn hit(x: u128);").unwrap_err();
    assert!(err.contains("unsupported probe argument type"), "{err}");
    assert!(err.contains("&[u8]"), "{err}");
}

#[test]
fn expand____seven_operands____is_rejected() {
    let err =
        expand_str("provider = \"app\"; fn hit(a: &str, b: &str, c: &str, d: u8);").unwrap_err();
    assert!(err.contains("passes 7 values"), "{err}");
}

#[test]
fn expand____six_operands____is_accepted() {
    assert!(expand_str("provider = \"app\"; fn hit(a: &str, b: &str, c: &[u8]);").is_ok());
}

#[test]
fn expand____return_type____is_rejected() {
    let err = expand_str("provider = \"app\"; fn hit() -> u8;").unwrap_err();
    assert!(err.contains("return nothing"), "{err}");
}

#[test]
fn expand____duplicate_names____are_rejected() {
    let err = expand_str("provider = \"app\"; fn hit(); fn hit(x: u8);").unwrap_err();
    assert!(err.contains("duplicate probe name"), "{err}");
}

#[test]
fn expand____bad_provider____is_rejected() {
    let err = expand_str("provider = \"http2\"; fn hit();").unwrap_err();
    assert!(err.contains("must not end with a digit"), "{err}");
}

#[test]
fn expand____pattern_argument____is_rejected() {
    let err = expand_str("provider = \"app\"; fn hit(mut x: u8);").unwrap_err();
    assert!(err.contains("plain names"), "{err}");
}

#[test]
fn expand____other_attribute____is_rejected() {
    let err = expand_str("provider = \"app\"; #[inline] fn hit();").unwrap_err();
    assert!(err.contains("only doc, cfg and lint attributes"), "{err}");
}
