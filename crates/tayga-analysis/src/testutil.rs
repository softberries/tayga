//! Builders for hand-written traces in unit tests.

use crate::model::{EventRec, LogRec, SpanKind, SpanRec, StatusCode, TraceBundle};

pub fn span(id: &str, parent: &str, service: &str, name: &str, start: u64, end: u64) -> SpanRec {
    SpanRec {
        span_id: id.into(),
        parent_span_id: parent.into(),
        service: service.into(),
        name: name.into(),
        kind: SpanKind::Internal,
        start_ns: start,
        end_ns: end,
        status: StatusCode::Unset,
        status_message: String::new(),
        attrs: Vec::new(),
        events: Vec::new(),
    }
}

pub fn err(mut s: SpanRec, message: &str) -> SpanRec {
    s.status = StatusCode::Error;
    s.status_message = message.into();
    s
}

pub fn kind(mut s: SpanRec, k: SpanKind) -> SpanRec {
    s.kind = k;
    s
}

pub fn attr(mut s: SpanRec, key: &str, value: &str) -> SpanRec {
    s.attrs.push((key.into(), value.into()));
    s
}

pub fn exception(mut s: SpanRec, kind: &str, message: &str) -> SpanRec {
    s.events.push(EventRec {
        ts_ns: s.start_ns,
        name: "exception".into(),
        attrs: vec![
            ("exception.type".into(), kind.into()),
            ("exception.message".into(), message.into()),
        ],
    });
    s
}

pub fn log(span_id: &str, severity_number: i32, body: &str, ts_ns: u64) -> LogRec {
    LogRec {
        ts_ns,
        span_id: span_id.into(),
        severity_number,
        severity_text: String::new(),
        body: body.into(),
        service: "svc".into(),
    }
}

pub fn bundle(spans: Vec<SpanRec>) -> TraceBundle {
    let mut b = TraceBundle::new("t1");
    b.spans = spans;
    b
}

#[test]
fn builders_compose() {
    let s = exception(
        attr(
            kind(
                err(span("a", "", "svc", "op", 1, 2), "boom"),
                SpanKind::Client,
            ),
            "k",
            "v",
        ),
        "E",
        "msg",
    );
    let mut b = bundle(vec![s]);
    b.logs.push(log("a", 17, "body", 1));
    assert_eq!(b.spans[0].status, StatusCode::Error);
    assert_eq!(b.spans[0].kind, SpanKind::Client);
    assert_eq!(b.spans[0].attr("k"), Some("v"));
    assert_eq!(b.spans[0].events[0].name, "exception");
    assert_eq!(b.logs.len(), 1);
}
