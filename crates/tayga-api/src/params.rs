//! Query and path parameter parsing shared by JSON and HTML routes.

const MAX_SINCE_SECS: u32 = 7 * 24 * 3600;

#[derive(Debug, Clone, PartialEq)]
pub struct GroupFilter {
    pub since_secs: u32,
    pub kind: Option<String>,
    pub service: Option<String>,
}

/// `<n>[smhd]`, between 1 second and 7 days.
pub fn parse_since(raw: &str) -> Result<u32, String> {
    let raw = raw.trim();
    let last_char_pos = raw
        .char_indices()
        .next_back()
        .map(|(i, _)| i)
        .ok_or_else(|| format!("invalid since {raw:?}: expected e.g. 15m, 1h, 7d"))?;
    let (digits, unit) = raw.split_at(last_char_pos);
    let n: u64 = digits
        .parse()
        .map_err(|_| format!("invalid since {raw:?}: expected e.g. 15m, 1h, 7d"))?;
    let secs = match unit {
        "s" => n,
        "m" => n.saturating_mul(60),
        "h" => n.saturating_mul(3600),
        "d" => n.saturating_mul(86_400),
        _ => return Err(format!("invalid since {raw:?}: unit must be s, m, h or d")),
    };
    if secs == 0 || secs > u64::from(MAX_SINCE_SECS) {
        return Err(format!("since must be between 1s and 7d (got {raw})"));
    }
    Ok(secs as u32)
}

/// Fingerprints are u64 rendered as decimal strings.
pub fn parse_fingerprint(raw: &str) -> Result<String, String> {
    raw.parse::<u64>()
        .map(|v| v.to_string())
        .map_err(|_| format!("invalid fingerprint {raw:?}"))
}

/// Story/trace ids: 32 lowercase hex characters.
pub fn parse_hex_id(raw: &str) -> Result<String, String> {
    if raw.len() == 32 && raw.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(raw.to_ascii_lowercase())
    } else {
        Err(format!("invalid id {raw:?}: expected 32 hex characters"))
    }
}

/// Builds a filter from optional query values; empty strings mean "no filter".
pub fn group_filter(
    since: Option<&str>,
    kind: Option<&str>,
    service: Option<&str>,
) -> Result<GroupFilter, String> {
    let since_secs = parse_since(since.unwrap_or("1h"))?;
    let kind = kind.filter(|k| !k.is_empty()).map(str::to_string);
    if let Some(k) = &kind
        && k != "error"
        && k != "slow"
    {
        return Err(format!("invalid kind {k:?}: expected error or slow"));
    }
    Ok(GroupFilter {
        since_secs,
        kind,
        service: service.filter(|s| !s.is_empty()).map(str::to_string),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn since_units() {
        assert_eq!(parse_since("30s"), Ok(30));
        assert_eq!(parse_since("15m"), Ok(900));
        assert_eq!(parse_since("1h"), Ok(3600));
        assert_eq!(parse_since("7d"), Ok(604_800));
    }

    #[test]
    fn since_rejects_out_of_range() {
        assert!(parse_since("0m").is_err());
        assert!(parse_since("8d").is_err());
        assert!(parse_since("99999999999999d").is_err());
        assert!(parse_since("1w").is_err());
        assert!(parse_since("").is_err());
        assert!(parse_since("h").is_err());
        // Regression: Unicode inputs must not panic
        assert!(parse_since("5é").is_err());
        assert!(parse_since("é").is_err());
        assert!(parse_since("５m").is_err()); // full-width digit
        assert!(parse_since("1h\u{0301}").is_err()); // combining accent
    }

    #[test]
    fn fingerprint_and_ids() {
        assert_eq!(
            parse_fingerprint("17393964261140422938"),
            Ok("17393964261140422938".into())
        );
        assert!(parse_fingerprint("abc").is_err());
        assert!(parse_fingerprint("-1").is_err());
        assert!(parse_hex_id(&"A".repeat(32)).is_ok());
        assert!(parse_hex_id(&"z".repeat(32)).is_err());
        assert!(parse_hex_id(&"a".repeat(10_000)).is_err());
    }

    #[test]
    fn filter_defaults_and_validation() {
        assert_eq!(
            group_filter(None, Some(""), None).unwrap(),
            GroupFilter {
                since_secs: 3600,
                kind: None,
                service: None
            }
        );
        assert!(group_filter(Some("1h"), Some("bogus"), None).is_err());
        assert_eq!(
            group_filter(Some("5m"), Some("slow"), Some("payment"))
                .unwrap()
                .service
                .as_deref(),
            Some("payment")
        );
    }
}
