//! Batch fingerprints of the masked token sequence (sub-project 4 spec §3.1). The value is
//! defined by [`reference_fingerprint`]: four 32-bit lanes over the tokens of
//! [`tokens`], each token followed by a 0 byte. Every [`BatchFingerprinter`] returns exactly
//! that value for an ASCII body and `None` for any other body (Unicode digits and word
//! boundaries need the regex path).

use crate::preprocess::{EMPTY, MAX_TOKENS, TRUNCATED, is_http_version, is_status_bytes, tokens};

/// Bodies of one batch in the Arrow string-column layout: one byte buffer and `len + 1`
/// offsets (`u32`, so the GPU reads them as they are).
#[derive(Debug, Clone)]
pub struct BodyBatch {
    bytes: Vec<u8>,
    offsets: Vec<u32>,
}

impl Default for BodyBatch {
    fn default() -> Self {
        Self::new()
    }
}

impl BodyBatch {
    pub fn new() -> Self {
        Self {
            bytes: Vec::new(),
            offsets: vec![0],
        }
    }

    /// Appends `body`. `false`, and nothing appended, when the batch would pass `u32::MAX`
    /// bytes.
    pub fn push(&mut self, body: &str) -> bool {
        let Some(end) = self
            .bytes
            .len()
            .checked_add(body.len())
            .and_then(|e| u32::try_from(e).ok())
        else {
            return false;
        };
        self.bytes.extend_from_slice(body.as_bytes());
        self.offsets.push(end);
        true
    }

    /// Empties the batch, keeping its allocations.
    pub fn clear(&mut self) {
        self.bytes.clear();
        self.offsets.truncate(1);
    }

    pub fn len(&self) -> usize {
        self.offsets.len() - 1
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn body(&self, i: usize) -> &[u8] {
        &self.bytes[self.offsets[i] as usize..self.offsets[i + 1] as usize]
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn offsets(&self) -> &[u32] {
        &self.offsets
    }
}

/// Cache key of a masked token sequence, and an independent second hash that verifies a hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fingerprint {
    pub key: u64,
    pub check: u64,
}

const SEEDS: [u32; 4] = [0x811C_9DC5, 0x9E37_79B9, 0x85EB_CA6B, 0xC2B2_AE35];
const PRIMES: [u32; 4] = [0x0100_0193, 0x9E37_79B1, 0x85EB_CA77, 0xC2B2_AE3D];

/// The murmur3 32-bit finaliser.
fn fmix32(mut h: u32) -> u32 {
    h ^= h >> 16;
    h = h.wrapping_mul(0x85EB_CA6B);
    h ^= h >> 13;
    h = h.wrapping_mul(0xC2B2_AE35);
    h ^ (h >> 16)
}

/// The fingerprint of four finished lanes (also the GPU's output layout).
pub(crate) fn from_lanes(f: [u32; 4]) -> Fingerprint {
    Fingerprint {
        key: (u64::from(f[0]) << 32) | u64::from(f[1]),
        check: (u64::from(f[2]) << 32) | u64::from(f[3]),
    }
}

struct Lanes {
    h: [u32; 4],
    len: u32,
}

impl Lanes {
    fn new() -> Self {
        Self { h: SEEDS, len: 0 }
    }

    fn byte(&mut self, b: u8) {
        for (h, p) in self.h.iter_mut().zip(PRIMES) {
            *h = (*h ^ u32::from(b)).wrapping_mul(p);
        }
        self.len = self.len.wrapping_add(1);
    }

    /// One output token and its terminating 0 byte.
    fn token(&mut self, t: &[u8]) {
        for &b in t {
            self.byte(b);
        }
        self.byte(0);
    }

    fn finish(self) -> Fingerprint {
        from_lanes(self.h.map(|h| fmix32(h ^ self.len)))
    }
}

/// The definition: the lanes over `tokens(body, keep_http_status)`. Allocates; the oracle of
/// the tests and benchmarks.
pub fn reference_fingerprint(body: &str, keep_http_status: bool) -> Fingerprint {
    let mut l = Lanes::new();
    for t in tokens(body, keep_http_status) {
        l.token(t.as_bytes());
    }
    l.finish()
}

/// ASCII whitespace as `str::split_ascii_whitespace` splits on it (not `\x0B`).
fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\x0C' | b'\r')
}

fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// True when `mask` changes the ASCII token `t`, or `t` already holds `<*>`: then `tokens`
/// emits `<*>`. `mask` replaces UUIDs, `\b(?:0x)?[0-9a-f]{8,}\b` (case-insensitive) and digit
/// runs. In ASCII every UUID and every `0x` form contains a digit, so the rule is: a digit, a
/// literal `<*>`, or a whole word (a maximal `[A-Za-z0-9_]` run) of 8 or more hex letters.
fn masks(t: &[u8]) -> bool {
    if t.iter().any(u8::is_ascii_digit) || t.windows(3).any(|w| w == b"<*>") {
        return true;
    }
    let mut run = 0usize;
    let mut all_hex = true;
    for &b in t.iter().chain(std::iter::once(&b' ')) {
        if is_word(b) {
            run += 1;
            all_hex &= b.is_ascii_hexdigit();
        } else {
            if run >= 8 && all_hex {
                return true;
            }
            run = 0;
            all_hex = true;
        }
    }
    false
}

/// [`reference_fingerprint`] in one pass without allocating; `None` for a body with a
/// non-ASCII byte.
pub fn fingerprint_body(body: &[u8], keep_http_status: bool) -> Option<Fingerprint> {
    if !body.is_ascii() {
        return None;
    }
    let mut l = Lanes::new();
    let mut n = 0usize;
    let mut prev: &[u8] = &[];
    let mut i = 0;
    loop {
        while i < body.len() && is_ws(body[i]) {
            i += 1;
        }
        if i == body.len() {
            break;
        }
        let start = i;
        while i < body.len() && !is_ws(body[i]) {
            i += 1;
        }
        let t = &body[start..i];
        if n == MAX_TOKENS {
            l.token(TRUNCATED.as_bytes());
            n += 1;
            break;
        }
        if keep_http_status && n > 0 && is_status_bytes(t) && is_http_version(prev) {
            l.token(t);
        } else if masks(t) {
            l.token(b"<*>");
        } else {
            l.token(t);
        }
        prev = t;
        n += 1;
    }
    if n == 0 {
        l.token(EMPTY.as_bytes());
    }
    Some(l.finish())
}

/// Fingerprints a batch of bodies. Backends differ in speed only: each returns
/// [`fingerprint_body`] for every body.
pub trait BatchFingerprinter: Send + Sync {
    /// `scalar`, `parallel` or `gpu`: the metric label and the log field.
    fn name(&self) -> &'static str;
    /// Replaces `out` with one entry per body of `batch`, in order.
    fn fingerprint(
        &self,
        batch: &BodyBatch,
        keep_http_status: bool,
        out: &mut Vec<Option<Fingerprint>>,
    );
}

/// One pass per body on the calling thread: the reference backend and the default.
pub struct ScalarFingerprinter;

impl BatchFingerprinter for ScalarFingerprinter {
    fn name(&self) -> &'static str {
        "scalar"
    }

    fn fingerprint(
        &self,
        batch: &BodyBatch,
        keep_http_status: bool,
        out: &mut Vec<Option<Fingerprint>>,
    ) {
        out.clear();
        out.extend((0..batch.len()).map(|i| fingerprint_body(batch.body(i), keep_http_status)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EDGES: &[&str] = &[
        "",
        " \t\n ",
        "Found 10 products from database",
        "user=abc-12 ok",
        "deadbeef",
        "deadbee",
        "DEADBEEFCAFE x",
        "_deadbeefcafe",
        "x-deadbeefcafe-y",
        "a.deadbeefcafe",
        "deadbeefcafez",
        "a<*>b",
        "<*",
        "<empty>",
        "a\x0Bb c",
        "\x0Cform\rfeed",
        r#"[2026-10-05T10:00:00.000Z] "GET /api/cart HTTP/1.1" 503 UF 0 91 2 - "-" "python""#,
        r#""POST /x HTTP/2" 200 - 5"#,
        "upstream replied HTTP/1.1 503",
        "HTTP/1.1 600",
        "HTTP/1.10 503",
        "HTTP/1.1\" 404",
        "HTTP/x 503",
        "retry 503 times",
        "GET /api/products/66VCHSJNUP took 12ms",
    ];

    #[test]
    fn ascii_bodies_match_the_reference() {
        let long = "word ".repeat(100);
        let sixty_five = (0..65)
            .map(|i| format!("w{}", i % 3))
            .collect::<Vec<_>>()
            .join(" ");
        let sixty_four = "a ".repeat(64);
        for body in
            EDGES
                .iter()
                .copied()
                .chain([long.as_str(), sixty_five.as_str(), sixty_four.as_str()])
        {
            for keep in [true, false] {
                assert_eq!(
                    fingerprint_body(body.as_bytes(), keep),
                    Some(reference_fingerprint(body, keep)),
                    "{body:?} keep={keep}"
                );
            }
        }
    }

    #[test]
    fn non_ascii_bodies_are_not_fingerprinted() {
        for body in [
            "zażółć 1",
            "count \u{0663}",
            "caf\u{e9}deadbeef",
            "\u{2026}",
        ] {
            assert_eq!(fingerprint_body(body.as_bytes(), true), None, "{body:?}");
        }
    }

    #[test]
    fn same_masked_sequence_same_fingerprint() {
        let a = fingerprint_body(b"user 123 logged in from 10.0.0.4", true);
        let b = fingerprint_body(b"user  9  logged in\tfrom 192.168.1.1", true);
        assert_eq!(a, b);
        assert_ne!(
            a,
            fingerprint_body(b"user 123 logged out from 10.0.0.4", true)
        );
        // A kept status code separates, a masked one does not.
        let ok = fingerprint_body(br#""GET / HTTP/1.1" 200 x"#, true);
        let err = fingerprint_body(br#""GET / HTTP/1.1" 503 x"#, true);
        assert_ne!(ok, err);
        assert_eq!(
            fingerprint_body(br#""GET / HTTP/1.1" 200 x"#, false),
            fingerprint_body(br#""GET / HTTP/1.1" 503 x"#, false)
        );
    }

    /// Pins the algorithm (spec §3.1): the GPU kernel and the CPU code must not drift together.
    #[test]
    fn known_values() {
        let pin = |body: &str, keep: bool, key: u64, check: u64| {
            assert_eq!(
                reference_fingerprint(body, keep),
                Fingerprint { key, check },
                "{body:?}"
            );
        };
        pin(
            "Found 10 products",
            true,
            0x86B7_8B79_87CF_AA4D,
            0xCC83_F545_1A8B_6FBF,
        );
        pin("", true, 0x2894_3DBD_03E1_AA0A, 0x47FC_590F_FD76_A512);
        pin(
            "upstream replied HTTP/1.1 503",
            true,
            0xFCF6_0AB0_F942_23AB,
            0x49A5_00E5_5824_1E84,
        );
        pin(
            "upstream replied HTTP/1.1 503",
            false,
            0x0FAD_5444_19C2_9EBF,
            0x9323_5360_762D_8367,
        );
    }

    #[test]
    fn batch_layout_is_arrow_like() {
        let mut b = BodyBatch::new();
        assert!(b.is_empty());
        assert!(b.push("ab") && b.push("") && b.push("cde"));
        assert_eq!(b.len(), 3);
        assert_eq!(b.offsets(), &[0, 2, 2, 5]);
        assert_eq!(b.bytes(), b"abcde");
        assert_eq!(
            (b.body(0), b.body(1), b.body(2)),
            (&b"ab"[..], &b""[..], &b"cde"[..])
        );
        b.clear();
        assert!(b.is_empty());
        assert_eq!(b.offsets(), &[0]);
    }

    #[test]
    fn scalar_fills_one_entry_per_body() {
        let mut b = BodyBatch::new();
        for body in ["a 1", "\u{e9}", ""] {
            assert!(b.push(body));
        }
        let mut out = vec![None; 7];
        ScalarFingerprinter.fingerprint(&b, true, &mut out);
        assert_eq!(
            out,
            vec![
                Some(reference_fingerprint("a 1", true)),
                None,
                Some(reference_fingerprint("", true))
            ]
        );
    }
}
