// Sub-project 4 spec §3.3: one invocation per body; the same value as `fingerprint_body`.

struct Params {
    count: u32,
    keep: u32,
    pad0: u32,
    pad1: u32,
}

@group(0) @binding(0) var<storage, read> bytes: array<u32>;
@group(0) @binding(1) var<storage, read> offsets: array<u32>;
@group(0) @binding(2) var<uniform> params: Params;
@group(0) @binding(3) var<storage, read_write> lanes: array<vec4<u32>>;
@group(0) @binding(4) var<storage, read_write> valid: array<u32>;

const SEEDS = vec4<u32>(0x811C9DC5u, 0x9E3779B9u, 0x85EBCA6Bu, 0xC2B2AE35u);
const PRIMES = vec4<u32>(0x01000193u, 0x9E3779B1u, 0x85EBCA77u, 0xC2B2AE3Du);
const MAX_TOKENS: u32 = 64u;

var<private> h: vec4<u32>;
var<private> hlen: u32;

fn byte_at(p: u32) -> u32 {
    return (bytes[p >> 2u] >> ((p & 3u) * 8u)) & 0xFFu;
}

fn is_ws(c: u32) -> bool {
    return c == 32u || c == 9u || c == 10u || c == 12u || c == 13u;
}

fn is_digit(c: u32) -> bool {
    return c >= 48u && c <= 57u;
}

fn is_word(c: u32) -> bool {
    return is_digit(c) || (c >= 65u && c <= 90u) || (c >= 97u && c <= 122u) || c == 95u;
}

fn is_hex_letter(c: u32) -> bool {
    return (c >= 65u && c <= 70u) || (c >= 97u && c <= 102u);
}

fn mix(c: u32) {
    h = (h ^ vec4<u32>(c)) * PRIMES;
    hlen = hlen + 1u;
}

fn emit_range(s: u32, e: u32) {
    for (var p = s; p < e; p++) {
        mix(byte_at(p));
    }
    mix(0u);
}

// `<*>`
fn emit_wildcard() {
    mix(60u); mix(42u); mix(62u); mix(0u);
}

// `<…>` (U+2026 is E2 80 A6 in UTF-8)
fn emit_truncated() {
    mix(60u); mix(0xE2u); mix(0x80u); mix(0xA6u); mix(62u); mix(0u);
}

// `<empty>`
fn emit_empty() {
    mix(60u); mix(101u); mix(109u); mix(112u); mix(116u); mix(121u); mix(62u); mix(0u);
}

fn masks(s: u32, e: u32) -> bool {
    var run = 0u;
    var all_hex = true;
    for (var p = s; p <= e; p++) {
        var c = 32u;
        if p < e {
            c = byte_at(p);
        }
        if is_digit(c) {
            return true;
        }
        if c == 60u && p + 2u < e && byte_at(p + 1u) == 42u && byte_at(p + 2u) == 62u {
            return true;
        }
        if is_word(c) {
            run = run + 1u;
            all_hex = all_hex && is_hex_letter(c);
        } else {
            if run >= 8u && all_hex {
                return true;
            }
            run = 0u;
            all_hex = true;
        }
    }
    return false;
}

fn is_status(s: u32, e: u32) -> bool {
    if e - s != 3u {
        return false;
    }
    let a = byte_at(s);
    return a >= 49u && a <= 53u && is_digit(byte_at(s + 1u)) && is_digit(byte_at(s + 2u));
}

fn is_http_version(s: u32, e0: u32) -> bool {
    var e = e0;
    if byte_at(e - 1u) == 34u {
        e = e - 1u;
    }
    if e - s < 5u {
        return false;
    }
    // The last `HTTP/`, as `rfind`.
    for (var k = 0u; k <= e - s - 5u; k++) {
        let q = e - 5u - k;
        if byte_at(q) == 72u && byte_at(q + 1u) == 84u && byte_at(q + 2u) == 84u
            && byte_at(q + 3u) == 80u && byte_at(q + 4u) == 47u {
            let rest = e - (q + 5u);
            if rest == 1u {
                return is_digit(byte_at(q + 5u));
            }
            if rest == 3u {
                return is_digit(byte_at(q + 5u)) && byte_at(q + 6u) == 46u && is_digit(byte_at(q + 7u));
            }
            return false;
        }
    }
    return false;
}

fn fmix(x: u32) -> u32 {
    var v = x;
    v = v ^ (v >> 16u);
    v = v * 0x85EBCA6Bu;
    v = v ^ (v >> 13u);
    v = v * 0xC2B2AE35u;
    return v ^ (v >> 16u);
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if i >= params.count {
        return;
    }
    let s = offsets[i];
    let e = offsets[i + 1u];
    // The rule of `fingerprint_body`: no fingerprint for a NUL (it would collide with the
    // token terminator) or a non-ASCII byte (Unicode digits and word boundaries).
    for (var p = s; p < e; p++) {
        let c = byte_at(p);
        if c == 0u || c >= 128u {
            valid[i] = 0u;
            return;
        }
    }
    h = SEEDS;
    hlen = 0u;
    var n = 0u;
    var prev_s = 0u;
    var prev_e = 0u;
    var p = s;
    loop {
        while p < e && is_ws(byte_at(p)) {
            p++;
        }
        if p >= e {
            break;
        }
        let ts = p;
        while p < e && !is_ws(byte_at(p)) {
            p++;
        }
        let te = p;
        if n == MAX_TOKENS {
            emit_truncated();
            n = n + 1u;
            break;
        }
        if params.keep == 1u && n > 0u && is_status(ts, te) && is_http_version(prev_s, prev_e) {
            emit_range(ts, te);
        } else if masks(ts, te) {
            emit_wildcard();
        } else {
            emit_range(ts, te);
        }
        prev_s = ts;
        prev_e = te;
        n = n + 1u;
    }
    if n == 0u {
        emit_empty();
    }
    lanes[i] = vec4<u32>(fmix(h.x ^ hlen), fmix(h.y ^ hlen), fmix(h.z ^ hlen), fmix(h.w ^ hlen));
    valid[i] = 1u;
}
