//! Serves the web app (`ui/dist`) with a client-routing fallback.
//!
//! Design:
//! - Assets come from an [`Assets`] source. Production uses [`Embedded`]
//!   (rust-embed over `../../ui/dist`, feature `embed-ui`); tests inject an
//!   in-memory map, so serving behaviour is tested without a UI build.
//! - The embed derive uses `allow_missing`: a missing `ui/dist` compiles to an
//!   empty set instead of failing, and `build.rs` emits a warning. With no
//!   `index.html` the server serves [`PLACEHOLDER`]. `cargo test` therefore
//!   works without Node.
//! - The router is meant to be merged last: it only has a fallback handler
//!   plus redirect routes, so every explicit route (`/api`, `/metrics`,
//!   `/healthz`) wins over it.
//! - The old server-rendered UI's URLs 308 to their new client routes, query
//!   string kept (the filter names did not change).

use axum::Router;
use axum::extract::{RawPathParams, State};
use axum::http::{HeaderValue, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use std::borrow::Cow;
use std::sync::Arc;

pub const PLACEHOLDER: &str = "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>Tayga</title></head><body style=\"font-family:system-ui,sans-serif;max-width:40rem;margin:4rem auto;padding:0 1rem\"><h1>UI not built</h1><p>Run <code>npm --prefix ui run build</code> and rebuild tayga-api.</p></body></html>";

const IMMUTABLE: &str = "public, max-age=31536000, immutable";
const NO_CACHE: &str = "no-cache";
/// Root-level static files (favicon, fonts) are not content-hashed.
const ROOT_FILE_CACHE: &str = "public, max-age=3600";

/// One embedded file.
pub struct Asset {
    pub data: Cow<'static, [u8]>,
    pub mime: String,
}

/// A source of static files, keyed by path relative to the dist root.
pub trait Assets: Send + Sync + 'static {
    fn get(&self, path: &str) -> Option<Asset>;
}

/// The app embedded at compile time from `ui/dist`.
#[cfg(feature = "embed-ui")]
#[derive(rust_embed::RustEmbed)]
// Keep in sync with the `ui/dist` path in build.rs.
#[folder = "../../ui/dist"]
#[allow_missing = true]
struct Dist;

#[cfg(feature = "embed-ui")]
pub struct Embedded;

#[cfg(feature = "embed-ui")]
impl Assets for Embedded {
    fn get(&self, path: &str) -> Option<Asset> {
        let f = Dist::get(path)?;
        Some(Asset {
            mime: f.metadata.mimetype().to_string(),
            data: f.data,
        })
    }
}

/// Serves nothing, so only the placeholder is shown (`embed-ui` off).
pub struct NoAssets;

impl Assets for NoAssets {
    fn get(&self, _path: &str) -> Option<Asset> {
        None
    }
}

/// An old server-rendered UI URL and where it moves to in the new app.
pub struct Redirect308 {
    pub from: &'static str,
    /// `{name}` placeholders are filled from the `from` path params.
    pub to: &'static str,
}

/// `/`, `/stories/{id}` kept their paths and are client routes (index
/// fallback). The group detail became the home page's selected group; its id
/// is JSON-quoted because the router parses search values as JSON and a u64
/// fingerprint can exceed 2^53.
pub const OLD_URL_REDIRECTS: &[Redirect308] = &[
    Redirect308 {
        from: "/groups/{fp}",
        to: "/?group=%22{fp}%22",
    },
    Redirect308 {
        from: "/alerts",
        to: "/logs/alerts",
    },
    Redirect308 {
        from: "/templates",
        to: "/logs/templates",
    },
    Redirect308 {
        from: "/templates/{id}",
        to: "/logs/templates/{id}",
    },
    Redirect308 {
        from: "/service-map",
        to: "/map",
    },
];

/// The production router: embedded assets (or placeholder) and the redirects.
pub fn router() -> Router {
    #[cfg(feature = "embed-ui")]
    let assets: Arc<dyn Assets> = Arc::new(Embedded);
    #[cfg(not(feature = "embed-ui"))]
    let assets: Arc<dyn Assets> = Arc::new(NoAssets);
    router_with(assets, OLD_URL_REDIRECTS)
}

pub fn router_with(assets: Arc<dyn Assets>, redirects: &'static [Redirect308]) -> Router {
    let mut app = Router::new();
    for r in redirects {
        let to = r.to;
        app = app.route(
            r.from,
            get(move |params: RawPathParams, uri: Uri| async move {
                redirect(to, &params, uri.query())
            }),
        );
    }
    app.fallback(serve).with_state(assets)
}

fn redirect(template: &str, params: &RawPathParams, query: Option<&str>) -> Redirect {
    let mut out = template.to_string();
    for (key, value) in params {
        out = out.replace(&format!("{{{key}}}"), &encode(value));
    }
    if let Some(q) = query.filter(|q| !q.is_empty()) {
        out.push(if out.contains('?') { '&' } else { '?' });
        out.push_str(q);
    }
    Redirect::permanent(&out)
}

/// Percent-encode everything except RFC 3986 unreserved characters.
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn is_reserved(path: &str) -> bool {
    ["/api", "/metrics", "/healthz"]
        .iter()
        .any(|p| path == *p || path.strip_prefix(p).is_some_and(|r| r.starts_with('/')))
}

fn file(asset: Asset, cache: &'static str) -> Response {
    let mut res = asset.data.into_owned().into_response();
    let h = res.headers_mut();
    if let Ok(v) = HeaderValue::from_str(&asset.mime) {
        h.insert(header::CONTENT_TYPE, v);
    }
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    res
}

fn index(assets: &dyn Assets) -> Response {
    match assets.get("index.html") {
        Some(a) => file(a, NO_CACHE),
        None => {
            let mut res = axum::response::Html(PLACEHOLDER).into_response();
            res.headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static(NO_CACHE));
            res
        }
    }
}

fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        axum::Json(serde_json::json!({ "error": "not found" })),
    )
        .into_response()
}

/// Percent-decode a URL path; `None` when the result is not valid UTF-8.
fn decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// A relative asset path that could escape the dist root (rust-embed reads
/// from disk in debug builds) or is otherwise not a plain file path.
fn is_unsafe(rel: &str) -> bool {
    rel.starts_with('/')
        || rel.contains('\\')
        || rel.contains('\0')
        || rel.split('/').any(|seg| seg == "..")
}

async fn serve(State(assets): State<Arc<dyn Assets>>, method: Method, uri: Uri) -> Response {
    let raw = uri.path();
    if is_reserved(raw) {
        return not_found();
    }
    if method != Method::GET && method != Method::HEAD {
        let mut res = StatusCode::METHOD_NOT_ALLOWED.into_response();
        res.headers_mut()
            .insert(header::ALLOW, HeaderValue::from_static("GET, HEAD"));
        return res;
    }
    let Some(path) = decode(raw) else {
        return not_found();
    };
    if is_reserved(&path) {
        return not_found();
    }
    let rel = path.strip_prefix('/').unwrap_or(&path);
    if is_unsafe(rel) {
        return not_found();
    }
    if rel.starts_with("assets/") {
        return match assets.get(rel) {
            Some(a) => file(a, IMMUTABLE),
            None => not_found(),
        };
    }
    if !rel.is_empty() && rel != "index.html" {
        if let Some(a) = assets.get(rel) {
            return file(a, ROOT_FILE_CACHE);
        }
        // A missing file (last segment has an extension) is a 404, not a page.
        if rel.rsplit('/').next().is_some_and(|s| s.contains('.')) {
            return not_found();
        }
    }
    index(assets.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use std::collections::HashMap;
    use tower::ServiceExt;

    struct Mem(HashMap<&'static str, (&'static str, &'static [u8])>);

    impl Assets for Mem {
        fn get(&self, path: &str) -> Option<Asset> {
            self.0.get(path).map(|(mime, data)| Asset {
                data: Cow::Borrowed(*data),
                mime: (*mime).to_string(),
            })
        }
    }

    fn fixture() -> Arc<dyn Assets> {
        Arc::new(Mem(HashMap::from([
            ("index.html", ("text/html", &b"<html>app</html>"[..])),
            ("assets/x.js", ("text/javascript", &b"console.log(1)"[..])),
            ("favicon.svg", ("image/svg+xml", &b"<svg/>"[..])),
        ])))
    }

    async fn call(app: Router, method: Method, uri: &str) -> Response {
        app.oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
    }

    async fn body(res: Response) -> String {
        let b = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        String::from_utf8(b.to_vec()).unwrap()
    }

    fn app() -> Router {
        router_with(fixture(), OLD_URL_REDIRECTS)
    }

    fn header_of(res: &Response, name: header::HeaderName) -> &str {
        res.headers().get(name).unwrap().to_str().unwrap()
    }

    #[tokio::test]
    async fn fallback_serves_index_for_client_routes() {
        for uri in ["/", "/stories/abc", "/traces", "/logs/templates/7"] {
            let res = call(app(), Method::GET, uri).await;
            assert_eq!(res.status(), StatusCode::OK, "{uri}");
            assert_eq!(header_of(&res, header::CACHE_CONTROL), "no-cache");
            assert_eq!(body(res).await, "<html>app</html>", "{uri}");
        }
    }

    #[tokio::test]
    async fn reserved_prefixes_are_json_404_not_index() {
        for uri in ["/api/v1/unknown", "/api", "/metrics/x", "/healthz/x"] {
            let res = call(app(), Method::GET, uri).await;
            assert_eq!(res.status(), StatusCode::NOT_FOUND, "{uri}");
            assert_eq!(header_of(&res, header::CONTENT_TYPE), "application/json");
            assert_eq!(body(res).await, r#"{"error":"not found"}"#);
        }
        // A lookalike prefix is a client route.
        let res = call(app(), Method::GET, "/apiary").await;
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn assets_are_immutable_with_content_type() {
        let res = call(app(), Method::GET, "/assets/x.js").await;
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            header_of(&res, header::CACHE_CONTROL),
            "public, max-age=31536000, immutable"
        );
        assert_eq!(header_of(&res, header::CONTENT_TYPE), "text/javascript");
        assert_eq!(body(res).await, "console.log(1)");
    }

    #[tokio::test]
    async fn missing_asset_and_missing_file_are_404() {
        for uri in ["/assets/nope.js", "/missing.png"] {
            let res = call(app(), Method::GET, uri).await;
            assert_eq!(res.status(), StatusCode::NOT_FOUND, "{uri}");
        }
    }

    #[tokio::test]
    async fn root_static_file_is_served() {
        let res = call(app(), Method::GET, "/favicon.svg").await;
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(header_of(&res, header::CONTENT_TYPE), "image/svg+xml");
        assert_eq!(
            header_of(&res, header::CACHE_CONTROL),
            "public, max-age=3600"
        );
    }

    #[tokio::test]
    async fn non_get_is_405_with_allow_but_reserved_wins() {
        let res = call(app(), Method::POST, "/traces").await;
        assert_eq!(res.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(header_of(&res, header::ALLOW), "GET, HEAD");
        let res = call(app(), Method::POST, "/api/v1/unknown").await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        assert_eq!(body(res).await, r#"{"error":"not found"}"#);
    }

    #[tokio::test]
    async fn head_on_asset_matches_get_headers() {
        let get = call(app(), Method::GET, "/assets/x.js").await;
        let head = call(app(), Method::HEAD, "/assets/x.js").await;
        assert_eq!(head.status(), StatusCode::OK);
        for h in [header::CONTENT_TYPE, header::CACHE_CONTROL] {
            assert_eq!(header_of(&head, h.clone()), header_of(&get, h));
        }
    }

    #[tokio::test]
    async fn traversal_paths_are_404_even_if_the_source_has_the_key() {
        let mem: Arc<dyn Assets> = Arc::new(Mem(HashMap::from([
            ("assets/../secret", ("text/plain", &b"secret"[..])),
            ("assets/x.js", ("text/javascript", &b"ok"[..])),
        ])));
        let app = router_with(mem, &[]);
        for uri in [
            "/assets/../secret",
            "/assets/%2e%2e/secret",
            "/assets/%2E%2E/secret",
            "/assets/..%5Csecret",
            "/assets/a%00b",
            "//etc/passwd",
        ] {
            let res = call(app.clone(), Method::GET, uri).await;
            assert_eq!(res.status(), StatusCode::NOT_FOUND, "{uri}");
        }
        let res = call(app, Method::GET, "/assets/x.js").await;
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn placeholder_when_dist_missing() {
        let app = router_with(Arc::new(NoAssets), &[]);
        let res = call(app, Method::GET, "/traces").await;
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(header_of(&res, header::CACHE_CONTROL), "no-cache");
        let b = body(res).await;
        assert!(b.contains("UI not built"));
        assert!(b.contains("npm --prefix ui run build"));
    }

    #[tokio::test]
    async fn every_redirect_is_308_to_the_new_path() {
        let cases = [
            ("/groups/123", "/?group=%22123%22"),
            ("/groups/a%2Fb", "/?group=%22a%2Fb%22"),
            (
                "/groups/18446744073709551615?since=24h",
                "/?group=%2218446744073709551615%22&since=24h",
            ),
            ("/alerts", "/logs/alerts"),
            (
                "/alerts?since=24h&kind=spike",
                "/logs/alerts?since=24h&kind=spike",
            ),
            ("/templates", "/logs/templates"),
            (
                "/templates?service=cart&q=x",
                "/logs/templates?service=cart&q=x",
            ),
            ("/templates/42", "/logs/templates/42"),
            ("/service-map", "/map"),
            ("/service-map?since=1h", "/map?since=1h"),
            ("/service-map?", "/map"),
        ];
        assert_eq!(OLD_URL_REDIRECTS.len(), 5);
        for (from, to) in cases {
            let res = call(app(), Method::GET, from).await;
            assert_eq!(res.status(), StatusCode::PERMANENT_REDIRECT, "{from}");
            assert_eq!(header_of(&res, header::LOCATION), to, "{from}");
        }
    }

    #[tokio::test]
    async fn kept_old_paths_are_client_routes() {
        for uri in ["/", "/stories/0123456789abcdef0123456789abcdef"] {
            let res = call(app(), Method::GET, uri).await;
            assert_eq!(res.status(), StatusCode::OK, "{uri}");
            assert_eq!(body(res).await, "<html>app</html>", "{uri}");
        }
    }

    #[tokio::test]
    async fn explicit_routes_win_over_the_fallback() {
        let live = Router::new().route("/healthz", get(|| async { "ok" }));
        let app = live.merge(app());
        let res = call(app.clone(), Method::GET, "/healthz").await;
        assert_eq!(body(res).await, "ok");
        let res = call(app, Method::GET, "/traces").await;
        assert_eq!(body(res).await, "<html>app</html>");
    }
}
