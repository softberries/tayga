//! Body → tokens: mask variable parts, split on whitespace, bound the length.

use tayga_analysis::fingerprint::mask;

pub const WILDCARD: &str = "<*>";
pub const TRUNCATED: &str = "<…>";
pub const EMPTY: &str = "<empty>";
pub const MAX_TOKENS: usize = 64;

/// Masking version stored by the logminer: 1 masks HTTP status codes; 3 keeps them as
/// exact-match tokens (2 kept them but let Drain generalise them away, so it is retired).
pub fn masking_version(keep_http_status: bool) -> u32 {
    if keep_http_status { 3 } else { 1 }
}

/// True when `t` ends in `HTTP/<d>` or `HTTP/<d>.<d>`, optionally followed by `"`.
fn is_http_version(t: &str) -> bool {
    let t = t.strip_suffix('"').unwrap_or(t);
    let Some(pos) = t.rfind("HTTP/") else {
        return false;
    };
    match &t.as_bytes()[pos + 5..] {
        [a] => a.is_ascii_digit(),
        [a, b'.', c] => a.is_ascii_digit() && c.is_ascii_digit(),
        _ => false,
    }
}

/// A three-digit HTTP status code, 100..=599.
pub fn is_status(t: &str) -> bool {
    t.len() == 3
        && t.bytes().all(|b| b.is_ascii_digit())
        && t.parse::<u16>().is_ok_and(|n| (100..=599).contains(&n))
}

/// True when `tok` (a token from [`tokens`] or from a stored template string) is a kept status
/// code. `mask` turns every digit run into `<*>`, so with `keep_http_status` the only tokens that
/// can be bare digits are the kept ones: the shape alone identifies them, with no marker and no
/// side table, and the rule re-applies unchanged to a template string read back from storage
/// (the `HTTP/x` token before it is itself masked to `<*>` there, so it cannot be re-checked).
pub fn is_protected(tok: &str, keep_http_status: bool) -> bool {
    keep_http_status && is_status(tok)
}

/// Masked tokens; any token containing a wildcard becomes exactly `<*>`.
/// With `keep_http_status`, a status code right after an `HTTP/x` token is kept literally.
pub fn tokens(body: &str, keep_http_status: bool) -> Vec<String> {
    let mut out = Vec::new();
    let mut prev: Option<&str> = None;
    for (i, raw) in body.split_ascii_whitespace().enumerate() {
        if i == MAX_TOKENS {
            out.push(TRUNCATED.to_string());
            break;
        }
        let keep = keep_http_status && is_status(raw) && prev.is_some_and(is_http_version);
        prev = Some(raw);
        if keep {
            out.push(raw.to_string());
            continue;
        }
        let masked = mask(raw);
        out.push(if masked.contains(WILDCARD) {
            WILDCARD.to_string()
        } else {
            masked
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
            tokens("Found 10 products from database", true),
            ["Found", "<*>", "products", "from", "database"]
        );
        assert_eq!(
            tokens("[2026-10-04T08:52:08.181Z] \"GET /x HTTP/1.1\" 200", true)[0],
            "<*>"
        );
        assert_eq!(tokens("user=abc-12 ok", true), ["<*>", "ok"]);
    }

    #[test]
    fn long_and_empty_bodies_are_bounded() {
        assert_eq!(tokens("", true), [EMPTY]);
        assert_eq!(tokens(" \t\n ", true), [EMPTY]);
        let long = "word ".repeat(10_000);
        let t = tokens(&long, true);
        assert_eq!(t.len(), MAX_TOKENS + 1);
        assert_eq!(t.last().map(String::as_str), Some(TRUNCATED));
        let huge = "x".repeat(1 << 20);
        assert_eq!(tokens(&huge, true).len(), 1);
    }

    fn legacy_tokens(body: &str) -> Vec<String> {
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

    #[test]
    fn per_token_masking_equals_whole_string_masking() {
        for body in [
            "card 4111111111111111 declined for order 9f8e7d6c-5b4a-3210-fedc-ba9876543210 after 1.5s",
            "GET /api/products/66VCHSJNUP took 12ms",
            "  leading and  double  spaces 42 ",
        ] {
            assert_eq!(tokens(body, false), legacy_tokens(body), "{body}");
        }
    }

    #[test]
    fn keeps_status_after_http_version() {
        let t = tokens(
            r#"[2026-10-05T10:00:00.000Z] "GET /api/cart HTTP/1.1" 503 UF 0 91 2 - "-" "python""#,
            true,
        );
        assert!(t.contains(&"503".to_string()), "{t:?}");
        let t2 = tokens(r#""POST /x HTTP/2" 200 - 5"#, true);
        assert!(t2.contains(&"200".to_string()));
    }

    #[test]
    fn other_numbers_stay_masked() {
        assert!(!tokens(r#""GET / HTTP/1.1" 600 x"#, true).contains(&"600".to_string()));
        assert!(!tokens("retry 503 times", true).contains(&"503".to_string()));
        assert!(!tokens(r#""GET / HTTP/1.1" 503 x"#, false).contains(&"503".to_string()));
    }

    #[test]
    fn masking_version_reflects_flag() {
        assert_eq!(masking_version(true), 3);
        assert_eq!(masking_version(false), 1);
    }

    #[test]
    fn only_kept_status_tokens_are_protected() {
        let line = r#"[2026-10-05T10:00:00.000Z] "GET /api/cart HTTP/1.1" 503 UF 0 91 2 - "-" "python/3.12""#;
        let t = tokens(line, true);
        let protected: Vec<&str> = t
            .iter()
            .filter(|x| is_protected(x, true))
            .map(String::as_str)
            .collect();
        assert_eq!(protected, ["503"], "{t:?}");
        assert!(t.iter().all(|x| !is_protected(x, false)));
        // No masked token can look like a status code: every digit run becomes `<*>`.
        for body in ["retry 503 times", "took 200ms", "code=404", "v1.2.3 ok 599"] {
            assert!(
                tokens(body, true).iter().all(|x| !is_protected(x, true)),
                "{body}"
            );
        }
        // The rule re-applies to a stored template string.
        let template = t.join(" ");
        assert_eq!(
            template
                .split(' ')
                .filter(|x| is_protected(x, true))
                .count(),
            1
        );
    }
}
