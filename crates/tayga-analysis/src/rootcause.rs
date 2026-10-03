//! Deepest failing span and a one-line explanation (spec §9.2).

use crate::model::{SEVERITY_ERROR, SpanKind, SpanRec, StatusCode};
use crate::tree::SpanTree;

#[derive(Debug, Clone, PartialEq)]
pub struct RootCause {
    pub span: usize,
    /// Other error leaves, earliest end first.
    pub also_failed: Vec<usize>,
    /// Top-level ancestor → root-cause span.
    pub path: Vec<usize>,
    pub message: String,
    pub exception_type: String,
    pub summary: String,
}

pub fn error_flags(tree: &SpanTree) -> Vec<bool> {
    (0..tree.bundle.spans.len())
        .map(|i| {
            let s = tree.span(i);
            s.status == StatusCode::Error
                || s.events.iter().any(|e| e.name == "exception")
                || tree.span_logs[i]
                    .iter()
                    .any(|&l| tree.bundle.logs[l].severity_number >= SEVERITY_ERROR)
        })
        .collect()
}

/// The error leaf (error span without error descendants) that ended first.
pub fn find_root_cause(tree: &SpanTree, is_err: &[bool]) -> Option<RootCause> {
    let mut err_below = vec![false; is_err.len()];
    for &i in tree.order.iter().rev() {
        if let Some(p) = tree.parent[i]
            && (is_err[i] || err_below[i])
        {
            err_below[p] = true;
        }
    }
    let mut leaves: Vec<usize> = (0..is_err.len())
        .filter(|&i| is_err[i] && !err_below[i])
        .collect();
    leaves.sort_by_key(|&i| (tree.span(i).end_ns, tree.span(i).start_ns, i));
    let (&cause, rest) = leaves.split_first()?;
    let message = error_message(tree, cause);
    Some(RootCause {
        span: cause,
        also_failed: rest.to_vec(),
        path: tree.path_to(cause),
        exception_type: exception_attr(tree.span(cause), "exception.type").unwrap_or_default(),
        summary: explain(tree, cause, &message),
        message,
    })
}

/// Who a client span tried to reach (spec §9.2 order).
pub fn peer_of(span: &SpanRec) -> String {
    span.attr("peer.service")
        .or_else(|| span.attr("rpc.service"))
        .map(str::to_string)
        .or_else(|| {
            span.attr("rpc.method")
                .and_then(|m| m.rsplit_once('/'))
                .map(|(svc, _)| svc.to_string())
        })
        .or_else(|| span.attr("server.address").map(str::to_string))
        .or_else(|| {
            span.attr("url.full")
                .or_else(|| span.attr("http.url"))
                .and_then(url_host)
                .map(str::to_string)
        })
        .unwrap_or_else(|| "an unknown peer".to_string())
}

/// Host of an absolute URL: scheme, userinfo, port and path removed.
fn url_host(url: &str) -> Option<&str> {
    let (_, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = if host_port.starts_with('[') {
        host_port.find(']').map_or(host_port, |i| &host_port[..=i])
    } else {
        host_port.split(':').next().unwrap_or("")
    };
    (!host.is_empty()).then_some(host)
}

fn operation_of(span: &SpanRec) -> &str {
    span.attr("rpc.method")
        .or_else(|| span.attr("http.route"))
        .unwrap_or(&span.name)
}

fn exception_attr(span: &SpanRec, key: &str) -> Option<String> {
    span.events
        .iter()
        .filter(|e| e.name == "exception")
        .find_map(|e| {
            e.attrs
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
        })
}

fn error_message(tree: &SpanTree, i: usize) -> String {
    let s = tree.span(i);
    if !s.status_message.is_empty() {
        return s.status_message.clone();
    }
    if let Some(m) = exception_attr(s, "exception.message") {
        return m;
    }
    tree.span_logs[i]
        .iter()
        .map(|&l| &tree.bundle.logs[l])
        .find(|l| l.severity_number >= SEVERITY_ERROR)
        .map(|l| l.body.clone())
        .unwrap_or_else(|| "error".to_string())
}

fn explain(tree: &SpanTree, i: usize, message: &str) -> String {
    let s = tree.span(i);
    if s.kind == SpanKind::Client && tree.children[i].is_empty() {
        format!(
            "{} could not reach {} ({}): {}",
            s.service,
            peer_of(s),
            operation_of(s),
            message
        )
    } else {
        format!("{} {} failed: {}", s.service, s.name, message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{attr, bundle, err, exception, kind, log, span};
    use proptest::prelude::*;

    #[test]
    fn deepest_server_error_is_the_cause() {
        let b = bundle(vec![
            err(
                kind(
                    span("a", "", "frontend", "POST /api/checkout", 0, 100),
                    SpanKind::Server,
                ),
                "500",
            ),
            err(
                kind(
                    span(
                        "b",
                        "a",
                        "checkout",
                        "oteldemo.PaymentService/Charge",
                        10,
                        90,
                    ),
                    SpanKind::Client,
                ),
                "rpc error",
            ),
            err(
                kind(
                    span(
                        "c",
                        "b",
                        "payment",
                        "oteldemo.PaymentService/Charge",
                        20,
                        80,
                    ),
                    SpanKind::Server,
                ),
                "Invalid token",
            ),
        ]);
        let t = SpanTree::build(&b).unwrap();
        let rc = find_root_cause(&t, &error_flags(&t)).unwrap();
        assert_eq!(rc.span, 2);
        assert_eq!(rc.path, vec![0, 1, 2]);
        assert!(rc.also_failed.is_empty());
        assert_eq!(
            rc.summary,
            "payment oteldemo.PaymentService/Charge failed: Invalid token"
        );
    }

    #[test]
    fn client_error_without_server_child_names_the_peer() {
        let client = attr(
            attr(
                err(
                    kind(
                        span(
                            "b",
                            "a",
                            "checkout",
                            "oteldemo.PaymentService/Charge",
                            10,
                            90,
                        ),
                        SpanKind::Client,
                    ),
                    "connection refused",
                ),
                "rpc.method",
                "oteldemo.PaymentService/Charge",
            ),
            "server.address",
            "172.26.0.20",
        );
        let b = bundle(vec![span("a", "", "frontend", "POST", 0, 100), client]);
        let t = SpanTree::build(&b).unwrap();
        let rc = find_root_cause(&t, &error_flags(&t)).unwrap();
        assert_eq!(rc.span, 1);
        assert_eq!(
            rc.summary,
            "checkout could not reach oteldemo.PaymentService (oteldemo.PaymentService/Charge): connection refused"
        );
    }

    #[test]
    fn peer_falls_back_in_spec_order() {
        let s = span("x", "", "svc", "call", 0, 1);
        assert_eq!(
            peer_of(&attr(s.clone(), "server.address", "10.0.0.1")),
            "10.0.0.1"
        );
        assert_eq!(
            peer_of(&attr(
                attr(s.clone(), "server.address", "10.0.0.1"),
                "rpc.method",
                "pkg.Cart/Get"
            )),
            "pkg.Cart"
        );
        assert_eq!(
            peer_of(&attr(
                attr(s.clone(), "rpc.method", "pkg.Cart/Get"),
                "rpc.service",
                "pkg.CartService"
            )),
            "pkg.CartService"
        );
        assert_eq!(
            peer_of(&attr(
                attr(s.clone(), "rpc.service", "x"),
                "peer.service",
                "cart"
            )),
            "cart"
        );
        assert_eq!(
            peer_of(&attr(s.clone(), "url.full", "http://agent:8000/x")),
            "agent"
        );
        assert_eq!(
            peer_of(&attr(
                s.clone(),
                "http.url",
                "https://u:p@api.example.com/v1?q=1"
            )),
            "api.example.com"
        );
        assert_eq!(
            peer_of(&attr(
                attr(s.clone(), "url.full", "http://agent:8000/x"),
                "server.address",
                "10.0.0.1"
            )),
            "10.0.0.1"
        );
        assert_eq!(
            peer_of(&attr(s.clone(), "url.full", "/relative")),
            "an unknown peer"
        );
        assert_eq!(peer_of(&s), "an unknown peer");
    }

    #[test]
    fn exception_event_and_error_log_mark_errors_and_supply_messages() {
        let mut b = bundle(vec![
            span("a", "", "frontend", "GET", 0, 100),
            exception(
                span("b", "a", "ad", "GetAds", 10, 20),
                "java.lang.IllegalStateException",
                "no ads",
            ),
            span("c", "a", "cart", "GetCart", 30, 40),
        ]);
        b.logs = vec![log("c", 17, "redis timeout", 35)];
        let t = SpanTree::build(&b).unwrap();
        let flags = error_flags(&t);
        assert_eq!(flags, vec![false, true, true]);
        let rc = find_root_cause(&t, &flags).unwrap();
        assert_eq!(rc.span, 1, "ad ended first");
        assert_eq!(rc.message, "no ads");
        assert_eq!(rc.exception_type, "java.lang.IllegalStateException");
        assert_eq!(rc.also_failed, vec![2]);
    }

    #[test]
    fn no_errors_no_root_cause() {
        let b = bundle(vec![span("a", "", "frontend", "GET", 0, 100)]);
        let t = SpanTree::build(&b).unwrap();
        assert!(find_root_cause(&t, &error_flags(&t)).is_none());
    }

    proptest! {
        #[test]
        fn root_cause_is_an_error_leaf(raw in prop::collection::vec((any::<u16>(), 0u64..1_000, 0u64..1_000, any::<bool>()), 1..60)) {
            let spans = raw.iter().enumerate().map(|(i, &(p, start, len, failed))| {
                let parent = if i == 0 { String::new() } else { format!("s{}", p as usize % i) };
                let s = span(&format!("s{i}"), &parent, "svc", "op", start, start + len);
                if failed { err(s, "x") } else { s }
            }).collect();
            let b = bundle(spans);
            let t = SpanTree::build(&b).unwrap();
            let flags = error_flags(&t);
            match find_root_cause(&t, &flags) {
                None => prop_assert!(flags.iter().all(|f| !f)),
                Some(rc) => {
                    prop_assert!(flags[rc.span]);
                    for (j, &f) in flags.iter().enumerate() {
                        if f && j != rc.span {
                            prop_assert!(!t.path_to(j).contains(&rc.span), "root cause has an error descendant");
                        }
                    }
                }
            }
        }
    }
}
