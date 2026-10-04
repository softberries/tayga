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
//!   plus redirect routes, so every explicit route (askama pages included)
//!   wins over it.

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

/// An old askama URL and where it moves to in the new app.
pub struct Redirect308 {
    pub from: &'static str,
    /// `{name}` placeholders are filled from the `from` path params.
    pub to: &'static str,
}

pub const OLD_URL_REDIRECTS: &[Redirect308] = &[
    Redirect308 {
        from: "/groups/{fp}",
        to: "/stories?group={fp}",
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

/// Paths the askama UI (`ui.rs`) still serves. A redirect whose `from` is
/// listed here is not mounted, because the askama page must keep winning.
/// Task 13 deletes askama and empties this list, which activates every
/// redirect above.
pub const ASKAMA_PATHS: &[&str] = &[
    "/",
    "/stories/{id}",
    "/groups/{fp}",
    "/service-map",
    "/alerts",
    "/templates",
    "/templates/{id}",
];

/// Redirects safe to mount: those not shadowing a live askama route.
pub fn active_redirects(askama_paths: &[&str]) -> Vec<&'static Redirect308> {
    OLD_URL_REDIRECTS
        .iter()
        .filter(|r| !askama_paths.contains(&r.from))
        .collect()
}

/// The production router: embedded assets (or placeholder) and live redirects.
pub fn router() -> Router {
    #[cfg(feature = "embed-ui")]
    let assets: Arc<dyn Assets> = Arc::new(Embedded);
    #[cfg(not(feature = "embed-ui"))]
    let assets: Arc<dyn Assets> = Arc::new(NoAssets);
    router_with(assets, &active_redirects(ASKAMA_PATHS))
}

pub fn router_with(assets: Arc<dyn Assets>, redirects: &[&'static Redirect308]) -> Router {
    let mut app = Router::new();
    for r in redirects {
        let to = r.to;
        app = app.route(
            r.from,
            get(move |params: RawPathParams| async move { redirect(to, &params) }),
        );
    }
    app.fallback(serve).with_state(assets)
}

fn redirect(template: &str, params: &RawPathParams) -> Redirect {
    let mut out = template.to_string();
    for (key, value) in params {
        out = out.replace(&format!("{{{key}}}"), &encode(value));
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

async fn serve(State(assets): State<Arc<dyn Assets>>, method: Method, uri: Uri) -> Response {
    if method != Method::GET && method != Method::HEAD {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    let path = uri.path();
    if is_reserved(path) {
        return not_found();
    }
    let rel = path.trim_start_matches('/');
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
        router_with(fixture(), &active_redirects(&[]))
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
    async fn non_get_is_405() {
        let res = call(app(), Method::POST, "/traces").await;
        assert_eq!(res.status(), StatusCode::METHOD_NOT_ALLOWED);
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
            ("/groups/abc123", "/stories?group=abc123"),
            ("/groups/a%2Fb", "/stories?group=a%2Fb"),
            ("/alerts", "/logs/alerts"),
            ("/templates", "/logs/templates"),
            ("/templates/42", "/logs/templates/42"),
            ("/service-map", "/map"),
        ];
        for (from, to) in cases {
            let res = call(app(), Method::GET, from).await;
            assert_eq!(res.status(), StatusCode::PERMANENT_REDIRECT, "{from}");
            assert_eq!(header_of(&res, header::LOCATION), to, "{from}");
        }
    }

    #[test]
    fn redirect_table_is_complete() {
        assert_eq!(OLD_URL_REDIRECTS.len(), 5);
        assert_eq!(active_redirects(&[]).len(), 5);
    }

    #[test]
    fn redirects_shadowing_askama_routes_are_not_mounted() {
        // Until Task 13 every redirect source is an askama route.
        assert!(active_redirects(ASKAMA_PATHS).is_empty());
        let only_alerts = active_redirects(&["/groups/{fp}", "/templates"]);
        let froms: Vec<_> = only_alerts.iter().map(|r| r.from).collect();
        assert_eq!(froms, ["/alerts", "/templates/{id}", "/service-map"]);
    }

    #[tokio::test]
    async fn askama_style_routes_win_over_the_fallback() {
        let live = Router::new().route("/alerts", get(|| async { "askama" }));
        let app = live.merge(router_with(fixture(), &active_redirects(ASKAMA_PATHS)));
        let res = call(app.clone(), Method::GET, "/alerts").await;
        assert_eq!(body(res).await, "askama");
        let res = call(app, Method::GET, "/traces").await;
        assert_eq!(body(res).await, "<html>app</html>");
    }
}
