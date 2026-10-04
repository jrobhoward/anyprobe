//! Validation of provider and probe names.
//!
//! Names must be valid on every backend at once: SDT notes, DTrace (provider
//! names gain the pid as a suffix) and ETW.

/// Longest provider name. DTrace appends the pid to the provider name and
/// limits the result to 64 bytes; six bytes are left for the pid.
pub(crate) const MAX_PROVIDER_LEN: usize = 58;

/// Longest probe name (DTrace's limit, less the terminating NUL).
pub(crate) const MAX_PROBE_LEN: usize = 63;

/// Names that macOS cannot link as a provider or probe name: D's reserved
/// words and the fixed-width integer types D defines. ld64 writes each
/// provider into a D declaration (`provider NAME { probe NAME(...); }`) and
/// compiles it, and a reserved name fails there with "Could not compile
/// reconstructed dtrace script". Rejected on every target so a probe that
/// builds on one platform builds on all of them. Typedefs from the kernel's
/// type data (`size_t`, `pid_t`) fail too, but that set varies by macOS
/// release and is not listed.
const D_RESERVED: &[&str] = &[
    "auto",
    "break",
    "case",
    "char",
    "const",
    "continue",
    "counter",
    "default",
    "do",
    "double",
    "else",
    "enum",
    "extern",
    "float",
    "for",
    "goto",
    "if",
    "import",
    "inline",
    "int",
    "int16_t",
    "int32_t",
    "int64_t",
    "int8_t",
    "intptr_t",
    "long",
    "offsetof",
    "probe",
    "provider",
    "register",
    "restrict",
    "return",
    "self",
    "short",
    "signed",
    "sizeof",
    "static",
    "string",
    "stringof",
    "struct",
    "switch",
    "this",
    "translator",
    "typedef",
    "uint16_t",
    "uint32_t",
    "uint64_t",
    "uint8_t",
    "uintptr_t",
    "union",
    "unsigned",
    "userland",
    "void",
    "volatile",
    "while",
    "xlate",
];

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Checks a provider name; returns why it is rejected.
pub(crate) fn check_provider(name: &str) -> Result<(), String> {
    if !is_identifier(name) {
        return Err(format!(
            "provider name `{name}` must be ASCII letters, digits and `_`, not starting with a digit"
        ));
    }
    if name.len() > MAX_PROVIDER_LEN {
        return Err(format!(
            "provider name `{name}` is {} bytes; the limit is {MAX_PROVIDER_LEN}, since DTrace appends the pid",
            name.len()
        ));
    }
    if name.ends_with(|c: char| c.is_ascii_digit()) {
        return Err(format!(
            "provider name `{name}` must not end with a digit: DTrace appends the pid to it"
        ));
    }
    if D_RESERVED.contains(&name) {
        return Err(format!(
            "provider name `{name}` is reserved in DTrace's D language, and macOS fails to link it"
        ));
    }
    Ok(())
}

/// Checks a probe name; returns why it is rejected.
pub(crate) fn check_probe(name: &str) -> Result<(), String> {
    if !is_identifier(name) {
        return Err(format!(
            "probe name `{name}` must be ASCII letters, digits and `_`, not starting with a digit"
        ));
    }
    if name.len() > MAX_PROBE_LEN {
        return Err(format!(
            "probe name `{name}` is {} bytes; the limit is {MAX_PROBE_LEN}",
            name.len()
        ));
    }
    if D_RESERVED.contains(&name) {
        return Err(format!(
            "probe name `{name}` is reserved in DTrace's D language, and macOS fails to link it"
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "names_tests.rs"]
mod names_tests;
