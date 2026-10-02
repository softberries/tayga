//! Stable grouping key for stories (spec §9.5).

use regex::Regex;
use std::sync::LazyLock;
use xxhash_rust::xxh3::Xxh3;

static UUID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b")
        .expect("valid regex")
});
static HEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(?:0x)?[0-9a-f]{8,}\b").expect("valid regex"));
static NUM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d+(?:\.\d+)?").expect("valid regex"));

/// Replaces variable tokens (UUIDs, hex runs of 8+, numbers) with `<*>`.
pub fn mask(s: &str) -> String {
    let s = UUID.replace_all(s, "<*>");
    let s = HEX.replace_all(&s, "<*>");
    NUM.replace_all(&s, "<*>").into_owned()
}

/// xxh3 of the parts, each terminated by `\0` so `["ab","c"]` ≠ `["a","bc"]`.
pub fn fingerprint(parts: &[&str]) -> u64 {
    let mut h = Xxh3::new();
    for p in parts {
        h.update(p.as_bytes());
        h.update(&[0]);
    }
    h.digest()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_variable_tokens() {
        assert_eq!(
            mask(
                "card 4111111111111111 declined for order 9f8e7d6c-5b4a-3210-fedc-ba9876543210 after 1.5s, ref 0xdeadbeef"
            ),
            "card <*> declined for order <*> after <*>s, ref <*>"
        );
        assert_eq!(
            mask("Invalid token. app.loyalty.level=gold"),
            "Invalid token. app.loyalty.level=gold"
        );
    }

    #[test]
    fn fingerprint_is_stable_and_separator_aware() {
        assert_eq!(fingerprint(&["a", "b"]), fingerprint(&["a", "b"]));
        assert_ne!(fingerprint(&["ab", "c"]), fingerprint(&["a", "bc"]));
    }
}
