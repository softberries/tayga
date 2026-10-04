//! Body → tokens: mask variable parts, split on whitespace, bound the length.

use tayga_analysis::fingerprint::mask;

pub const WILDCARD: &str = "<*>";
pub const TRUNCATED: &str = "<…>";
pub const EMPTY: &str = "<empty>";
pub const MAX_TOKENS: usize = 64;

/// Masked tokens; any token containing a wildcard becomes exactly `<*>`.
pub fn tokens(body: &str) -> Vec<String> {
    let masked = mask(body);
    let mut out = Vec::new();
    for (i, t) in masked.split_ascii_whitespace().enumerate() {
        if i == MAX_TOKENS {
            out.push(TRUNCATED.to_string());
            break;
        }
        out.push(if t.contains(WILDCARD) {
            WILDCARD.to_string()
        } else {
            t.to_string()
        });
    }
    if out.is_empty() {
        out.push(EMPTY.to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_and_normalizes_tokens() {
        assert_eq!(
            tokens("Found 10 products from database"),
            ["Found", "<*>", "products", "from", "database"]
        );
        assert_eq!(
            tokens("[2026-10-04T08:52:08.181Z] \"GET /x HTTP/1.1\" 200")[0],
            "<*>"
        );
        assert_eq!(tokens("user=abc-12 ok"), ["<*>", "ok"]);
    }

    #[test]
    fn long_and_empty_bodies_are_bounded() {
        assert_eq!(tokens(""), [EMPTY]);
        assert_eq!(tokens(" \t\n "), [EMPTY]);
        let long = "word ".repeat(10_000);
        let t = tokens(&long);
        assert_eq!(t.len(), MAX_TOKENS + 1);
        assert_eq!(t.last().map(String::as_str), Some(TRUNCATED));
        let huge = "x".repeat(1 << 20);
        assert_eq!(tokens(&huge).len(), 1);
    }
}
