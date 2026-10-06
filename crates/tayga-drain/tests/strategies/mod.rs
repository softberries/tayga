//! Bodies that reach every rule of `preprocess::tokens`: kept and masked status codes, hex
//! words, `<*>` literals, `\x0B` (not a separator), non-ASCII digits and letters, 65+ tokens.

use proptest::prelude::*;

fn token() -> impl Strategy<Value = String> {
    prop_oneof![
        prop::sample::select(vec![
            "GET",
            "\"GET",
            "/api/cart",
            "HTTP/1.1",
            "HTTP/2\"",
            "HTTP/1.10",
            "200",
            "503",
            "600",
            "099",
            "<*>",
            "a<*>b",
            "<*",
            "deadbeef",
            "DEADBEEFCAFE",
            "_deadbeef",
            "deadbee",
            "x.deadbeefcafe",
            "user=abc-12",
            "0x1f",
            "a\x0Bb",
            "ok",
            "<empty>",
            "\u{e9}t\u{e9}",
            "\u{0663}",
        ])
        .prop_map(str::to_string),
        "[a-fA-F_./=-]{1,12}",
        "[ -~]{1,10}",
    ]
}

/// One log body of 0 to 79 tokens with mixed ASCII whitespace between them.
pub fn body() -> impl Strategy<Value = String> {
    (
        prop::collection::vec(token(), 0..80),
        prop::collection::vec(
            prop::sample::select(vec![" ", "\t", "  ", "\n", "\r\n", "\x0C"]),
            80,
        ),
    )
        .prop_map(|(tokens, seps)| {
            let mut s = String::new();
            for (i, t) in tokens.iter().enumerate() {
                if i > 0 {
                    s.push_str(seps[i]);
                }
                s.push_str(t);
            }
            s
        })
}
