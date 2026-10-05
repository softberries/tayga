//! Optional login (spec §2): config validation, Argon2id password check, a stateless
//! HMAC-signed session cookie, a per-IP failure limiter and the middleware that guards `/api/*`.
//!
//! - The session token is `b64url(username|expiry_unix) . b64url(HMAC-SHA256(key, payload))`.
//!   It is stateless: logout only clears the cookie, so a stolen cookie stays valid until expiry.
//! - The limiter keys on the TCP peer address (`ConnectInfo`), never on `X-Forwarded-For`.
//! - Passwords, hashes, keys and cookies are never logged.

use argon2::{ARGON2ID_IDENT, Argon2, PasswordHash, PasswordVerifier};
use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use hmac::{Hmac, KeyInit, Mac};
use serde::Deserialize;
use sha2::Sha256;
use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

pub const COOKIE: &str = "tayga_session";
/// `/api/*` paths reachable without a session.
const OPEN_API: [&str; 3] = ["/api/v1/auth/login", "/api/v1/auth/me", "/api/v1/config"];
const MIN_KEY_BYTES: usize = 32;
const WINDOW: Duration = Duration::from_secs(5 * 60);
const MAX_FAILURES: usize = 5;
const MAX_IPS: usize = 10_000;

/// `[auth]` settings; `TAYGA__AUTH__*` in the environment.
#[derive(Deserialize, Clone)]
#[serde(default)]
pub struct AuthSettings {
    pub enabled: bool,
    pub username: String,
    /// Argon2id PHC string (`tayga-devtools hash-password`).
    pub password_hash: String,
    /// `<n>s`, `<n>m`, `<n>h` or `<n>d`.
    pub session_ttl: String,
    /// Base64 of 32 or more bytes; empty means a random key per process.
    pub session_key: String,
    pub secure_cookie: bool,
}

impl Default for AuthSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            username: String::new(),
            password_hash: String::new(),
            session_ttl: "12h".into(),
            session_key: String::new(),
            secure_cookie: false,
        }
    }
}

pub struct Auth {
    username: String,
    hash: PasswordHash,
    key: Vec<u8>,
    ttl: Duration,
    secure: bool,
    limiter: Limiter,
}

/// Redacted: never prints the hash or the key.
impl fmt::Debug for Auth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Auth")
            .field("username", &self.username)
            .field("ttl", &self.ttl)
            .field("secure", &self.secure)
            .finish_non_exhaustive()
    }
}

/// Result of a credential check that went through the limiter.
enum Verdict {
    Ok,
    Denied,
    Limited(u64),
}

impl Auth {
    /// `None` when auth is disabled. Errors name the missing or invalid key.
    pub fn from_settings(s: &AuthSettings) -> anyhow::Result<Option<Arc<Auth>>> {
        if !s.enabled {
            return Ok(None);
        }
        let username = s.username.trim();
        anyhow::ensure!(
            !username.is_empty(),
            "auth.username is required when auth.enabled is true"
        );
        anyhow::ensure!(
            !username.contains(['|', ':']),
            "auth.username must not contain '|' or ':'"
        );
        anyhow::ensure!(
            !s.password_hash.trim().is_empty(),
            "auth.password_hash is required when auth.enabled is true"
        );
        let hash = PasswordHash::new(s.password_hash.trim())
            .map_err(|e| anyhow::anyhow!("auth.password_hash is not a valid PHC string: {e}"))?;
        anyhow::ensure!(
            hash.algorithm == ARGON2ID_IDENT,
            "auth.password_hash must be an Argon2id PHC string"
        );
        let ttl = parse_ttl(&s.session_ttl).ok_or_else(|| {
            anyhow::anyhow!("auth.session_ttl must be <n>s, <n>m, <n>h or <n>d with n > 0")
        })?;
        let key = if s.session_key.trim().is_empty() {
            tracing::warn!(
                "auth.session_key is unset: a random key is used, so a restart signs everyone out"
            );
            let mut key = vec![0u8; MIN_KEY_BYTES];
            rand::fill(&mut key[..]);
            key
        } else {
            let key = STANDARD
                .decode(s.session_key.trim())
                .map_err(|_| anyhow::anyhow!("auth.session_key is not valid base64"))?;
            anyhow::ensure!(
                key.len() >= MIN_KEY_BYTES,
                "auth.session_key must decode to at least {MIN_KEY_BYTES} bytes"
            );
            key
        };
        Ok(Some(Arc::new(Auth {
            username: username.to_string(),
            hash,
            key,
            ttl,
            secure: s.secure_cookie,
            limiter: Limiter::default(),
        })))
    }

    fn mac(&self) -> HmacSha256 {
        HmacSha256::new_from_slice(&self.key).expect("HMAC takes a key of any length")
    }

    fn sign(&self, username: &str, expiry_unix: u64) -> String {
        let payload = format!("{username}|{expiry_unix}");
        let tag = self.mac().chain_update(payload.as_bytes()).finalize();
        format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(payload),
            URL_SAFE_NO_PAD.encode(tag.into_bytes())
        )
    }

    /// The username of a token whose MAC matches (constant time) and that expires after `now`.
    fn verify_token(&self, token: &str, now_unix: u64) -> Option<String> {
        let (payload, tag) = token.split_once('.')?;
        let payload = URL_SAFE_NO_PAD.decode(payload).ok()?;
        let tag = URL_SAFE_NO_PAD.decode(tag).ok()?;
        self.mac().chain_update(&payload).verify_slice(&tag).ok()?;
        let payload = String::from_utf8(payload).ok()?;
        let (username, expiry) = payload.rsplit_once('|')?;
        let expiry: u64 = expiry.parse().ok()?;
        (expiry > now_unix && !username.contains('|')).then(|| username.to_string())
    }

    /// The signed-in user from the session cookie, if it is valid for the configured user.
    fn session_user(&self, headers: &HeaderMap) -> Option<String> {
        let token = session_cookie(headers)?;
        self.verify_token(token, now_unix())
            .filter(|u| *u == self.username)
    }

    /// Argon2 runs even when the username is wrong, so both failures take the same time.
    fn credentials_match(&self, username: &str, password: &str) -> bool {
        let user_ok: bool = username.as_bytes().ct_eq(self.username.as_bytes()).into();
        let pass_ok = Argon2::default()
            .verify_password(password.as_bytes(), &self.hash)
            .is_ok();
        user_ok & pass_ok
    }

    /// Check credentials for `ip` through the limiter; Argon2 runs on a blocking thread.
    async fn check(self: &Arc<Self>, ip: IpAddr, username: String, password: String) -> Verdict {
        if let Err(retry) = self.limiter.attempt(ip, Instant::now()) {
            return Verdict::Limited(retry);
        }
        let auth = self.clone();
        let ok = tokio::task::spawn_blocking(move || auth.credentials_match(&username, &password))
            .await
            .unwrap_or(false);
        if ok {
            self.limiter.clear(ip);
            Verdict::Ok
        } else {
            Verdict::Denied
        }
    }

    fn cookie(&self, value: &str, max_age: u64) -> HeaderValue {
        let secure = if self.secure { "; Secure" } else { "" };
        HeaderValue::from_str(&format!(
            "{COOKIE}={value}; HttpOnly; SameSite=Strict; Path=/; Max-Age={max_age}{secure}"
        ))
        .expect("cookie is ASCII")
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn parse_ttl(s: &str) -> Option<Duration> {
    let s = s.trim();
    let unit = match s.chars().last()? {
        's' => 1,
        'm' => 60,
        'h' => 3_600,
        'd' => 86_400,
        _ => return None,
    };
    let n: u64 = s[..s.len() - 1].parse().ok()?;
    let secs = n.checked_mul(unit).filter(|&v| v > 0)?;
    Some(Duration::from_secs(secs))
}

fn session_cookie(headers: &HeaderMap) -> Option<&str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .find_map(|pair| {
            let (name, value) = pair.trim().split_once('=')?;
            (name == COOKIE).then_some(value)
        })
}

/// `(user, password)` from an `Authorization: Basic` value, split at the first colon.
fn basic_credentials(value: &[u8]) -> Option<(String, String)> {
    let value = std::str::from_utf8(value).ok()?;
    let (scheme, encoded) = value.trim().split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("basic") {
        return None;
    }
    let decoded = STANDARD.decode(encoded.trim()).ok()?;
    let decoded = String::from_utf8(decoded).ok()?;
    let (user, pass) = decoded.split_once(':')?;
    Some((user.to_string(), pass.to_string()))
}

/// Failed (and in-flight) credential checks per peer IP within the window. An attempt is
/// recorded before the check runs, so concurrent guesses cannot overshoot the limit; a success
/// clears the IP.
#[derive(Default)]
struct Limiter {
    ips: Mutex<HashMap<IpAddr, VecDeque<Instant>>>,
}

impl Limiter {
    /// Record an attempt, or `Err(retry_after_secs)` when the IP is at the limit.
    fn attempt(&self, ip: IpAddr, now: Instant) -> Result<(), u64> {
        let mut ips = self.ips.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(times) = ips.get_mut(&ip) {
            prune(times, now);
            if times.len() >= MAX_FAILURES {
                let oldest = times.front().copied().unwrap_or(now);
                let wait = WINDOW.saturating_sub(now.saturating_duration_since(oldest));
                return Err(wait.as_secs_f64().ceil().max(1.0) as u64);
            }
        } else if ips.len() >= MAX_IPS {
            ips.retain(|_, times| {
                prune(times, now);
                !times.is_empty()
            });
            if ips.len() >= MAX_IPS
                && let Some(stale) = ips
                    .iter()
                    .min_by_key(|(_, times)| times.back().copied())
                    .map(|(ip, _)| *ip)
            {
                ips.remove(&stale);
            }
        }
        ips.entry(ip).or_default().push_back(now);
        Ok(())
    }

    fn clear(&self, ip: IpAddr) {
        self.ips
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&ip);
    }
}

fn prune(times: &mut VecDeque<Instant>, now: Instant) {
    while times
        .front()
        .is_some_and(|t| now.saturating_duration_since(*t) >= WINDOW)
    {
        times.pop_front();
    }
}

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(serde_json::json!({ "error": message }))).into_response()
}

/// 401 without `WWW-Authenticate`, so browsers do not show their own prompt.
fn unauthorized() -> Response {
    error(StatusCode::UNAUTHORIZED, "unauthorized")
}

fn too_many(retry_after: u64) -> Response {
    let mut res = error(StatusCode::TOO_MANY_REQUESTS, "too many attempts");
    res.headers_mut()
        .insert(header::RETRY_AFTER, HeaderValue::from(retry_after));
    res
}

fn is_json(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("application/json"))
}

#[derive(Deserialize)]
struct LoginBody {
    username: String,
    password: String,
}

/// The `auth/*` routes; empty when auth is disabled, so they answer 404.
pub fn routes(auth: Option<Arc<Auth>>) -> Router {
    let Some(auth) = auth else {
        return Router::new();
    };
    Router::new()
        .route("/api/v1/auth/login", post(login))
        .route("/api/v1/auth/logout", post(logout))
        .route("/api/v1/auth/me", get(me))
        .with_state(auth)
}

async fn login(
    State(auth): State<Arc<Auth>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !is_json(&headers) {
        return error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "content type must be application/json",
        );
    }
    let Ok(body) = serde_json::from_slice::<LoginBody>(&body) else {
        return error(
            StatusCode::BAD_REQUEST,
            "body must be {\"username\", \"password\"}",
        );
    };
    match auth.check(peer.ip(), body.username, body.password).await {
        Verdict::Ok => {
            tracing::info!(peer = %peer.ip(), "login succeeded");
            let ttl = auth.ttl.as_secs();
            let token = auth.sign(&auth.username, now_unix() + ttl);
            let mut res = StatusCode::NO_CONTENT.into_response();
            res.headers_mut()
                .insert(header::SET_COOKIE, auth.cookie(&token, ttl));
            res
        }
        Verdict::Denied => {
            tracing::warn!(peer = %peer.ip(), "login failed");
            unauthorized()
        }
        Verdict::Limited(retry) => too_many(retry),
    }
}

async fn logout(State(auth): State<Arc<Auth>>) -> Response {
    let mut res = StatusCode::NO_CONTENT.into_response();
    res.headers_mut()
        .insert(header::SET_COOKIE, auth.cookie("", 0));
    res
}

async fn me(State(auth): State<Arc<Auth>>, headers: HeaderMap) -> Response {
    match auth.session_user(&headers) {
        Some(username) => Json(serde_json::json!({ "username": username })).into_response(),
        None => unauthorized(),
    }
}

/// Guards `/api/*` except the open paths: a valid session cookie or Basic credentials pass.
pub async fn require(
    State(auth): State<Arc<Auth>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    req: Request,
    next: Next,
) -> Response {
    let path = req.uri().path();
    if !path.starts_with("/api/") || OPEN_API.contains(&path) {
        return next.run(req).await;
    }
    if auth.session_user(req.headers()).is_some() {
        return next.run(req).await;
    }
    let Some((username, password)) = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| basic_credentials(v.as_bytes()))
    else {
        return unauthorized();
    };
    match auth.check(peer.ip(), username, password).await {
        Verdict::Ok => next.run(req).await,
        Verdict::Denied => {
            tracing::warn!(peer = %peer.ip(), "basic auth failed");
            unauthorized()
        }
        Verdict::Limited(retry) => too_many(retry),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::{ApiMetrics, api_router};
    use crate::routes_v2::{self, ClientConfig, LAG_TTL, LagCache, LagFetch};
    use crate::spa::{self, NoAssets};
    use crate::testrepo::FakeRepo;
    use argon2::{Algorithm, Params, PasswordHasher, Version};
    use axum::body::Body;
    use axum::extract::connect_info::MockConnectInfo;
    use axum::http::Method;
    use axum::middleware::from_fn_with_state;
    use prometheus_client::registry::Registry;
    use tower::ServiceExt;

    const PEER: [u8; 4] = [10, 0, 0, 1];

    /// Low-cost Argon2id so tests stay fast.
    fn test_hash(password: &str) -> String {
        let params = Params::new(Params::MIN_M_COST, 1, 1, None).unwrap();
        Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
            .hash_password(password.as_bytes())
            .unwrap()
            .to_string()
    }

    fn settings() -> AuthSettings {
        AuthSettings {
            enabled: true,
            username: "admin".into(),
            password_hash: test_hash("secret"),
            session_key: STANDARD.encode([7u8; 32]),
            ..AuthSettings::default()
        }
    }

    fn auth() -> Arc<Auth> {
        Auth::from_settings(&settings()).unwrap().unwrap()
    }

    fn config_err(s: AuthSettings) -> String {
        format!("{:#}", Auth::from_settings(&s).expect_err("config error"))
    }

    // token

    #[test]
    fn token_round_trips_until_expiry() {
        let a = auth();
        let now = 1_900_000_000;
        let t = a.sign("admin", now + 60);
        assert_eq!(a.verify_token(&t, now).as_deref(), Some("admin"));
        assert_eq!(a.verify_token(&t, now + 61), None);
    }

    #[test]
    fn tampered_token_fails() {
        let a = auth();
        let now = 1_900_000_000;
        let t = a.sign("admin", now + 60);
        let dot = t.find('.').unwrap();
        for i in [0, dot + 1] {
            let mut bytes = t.clone().into_bytes();
            bytes[i] = if bytes[i] == b'A' { b'B' } else { b'A' };
            let tampered = String::from_utf8(bytes).unwrap();
            assert_ne!(tampered, t);
            assert_eq!(a.verify_token(&tampered, now), None, "byte {i}");
        }
        // A forged payload with the old MAC fails too.
        let forged = format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(format!("admin|{}", now + 999_999)),
            t.split_once('.').unwrap().1
        );
        assert_eq!(a.verify_token(&forged, now), None);
        for garbage in ["", ".", "abc", "a.b.c", "!!!.???"] {
            assert_eq!(a.verify_token(garbage, now), None, "{garbage}");
        }
    }

    #[test]
    fn token_signed_with_other_key_fails() {
        let a = auth();
        let other = Auth::from_settings(&AuthSettings {
            session_key: STANDARD.encode([9u8; 32]),
            ..settings()
        })
        .unwrap()
        .unwrap();
        let now = 1_900_000_000;
        assert_eq!(a.verify_token(&other.sign("admin", now + 60), now), None);
    }

    // config

    #[test]
    fn enabled_requires_username_and_hash() {
        let e = config_err(AuthSettings {
            username: " ".into(),
            ..settings()
        });
        assert!(e.contains("auth.username"), "{e}");
        let e = config_err(AuthSettings {
            password_hash: String::new(),
            ..settings()
        });
        assert!(e.contains("auth.password_hash"), "{e}");
        let e = config_err(AuthSettings {
            password_hash: "plain-password".into(),
            ..settings()
        });
        assert!(e.contains("auth.password_hash"), "{e}");
        assert!(
            !e.contains("plain-password"),
            "error must not echo the value: {e}"
        );
        let e = config_err(AuthSettings {
            username: "ad:min".into(),
            ..settings()
        });
        assert!(e.contains("auth.username"), "{e}");
        let e = config_err(AuthSettings {
            session_ttl: "0h".into(),
            ..settings()
        });
        assert!(e.contains("auth.session_ttl"), "{e}");
    }

    #[test]
    fn short_session_key_is_rejected() {
        let e = config_err(AuthSettings {
            session_key: STANDARD.encode([1u8; 31]),
            ..settings()
        });
        assert!(e.contains("auth.session_key"), "{e}");
        let e = config_err(AuthSettings {
            session_key: "not base64!".into(),
            ..settings()
        });
        assert!(e.contains("auth.session_key"), "{e}");
    }

    #[test]
    fn unset_session_key_gets_random_key() {
        let s = AuthSettings {
            session_key: String::new(),
            ..settings()
        };
        let a = Auth::from_settings(&s).unwrap().unwrap();
        let b = Auth::from_settings(&s).unwrap().unwrap();
        assert_eq!(a.key.len(), MIN_KEY_BYTES);
        assert_ne!(a.key, b.key);
    }

    #[test]
    fn disabled_returns_none() {
        assert!(
            Auth::from_settings(&AuthSettings::default())
                .unwrap()
                .is_none()
        );
        assert_eq!(AuthSettings::default().session_ttl, "12h");
        assert_eq!(parse_ttl("12h"), Some(Duration::from_secs(43_200)));
        assert_eq!(parse_ttl("90m"), Some(Duration::from_secs(5_400)));
        assert_eq!(parse_ttl("12"), None);
        assert_eq!(parse_ttl("h"), None);
    }

    // basic header

    fn basic(user_pass: &str) -> String {
        format!("Basic {}", STANDARD.encode(user_pass))
    }

    #[test]
    fn basic_splits_at_first_colon() {
        assert_eq!(
            basic_credentials(basic("u:pa:ss").as_bytes()),
            Some(("u".into(), "pa:ss".into()))
        );
        assert_eq!(
            basic_credentials(format!("basic {}", STANDARD.encode("u:")).as_bytes()),
            Some(("u".into(), String::new()))
        );
    }

    #[test]
    fn basic_rejects_garbage() {
        let not_utf8 = format!("Basic {}", STANDARD.encode([0xff, b':', 0xfe]));
        let inputs: [&[u8]; 7] = [
            b"Basic !!!",
            b"Basic",
            b"",
            b"Bearer abc",
            b"Basic \xff\xfe",
            not_utf8.as_bytes(),
            b"Basic dXNlcg==", // "user", no colon
        ];
        for input in inputs {
            assert_eq!(basic_credentials(input), None, "{input:?}");
        }
    }

    // limiter

    fn ip(n: u32) -> IpAddr {
        IpAddr::from(n.to_be_bytes())
    }

    #[test]
    fn sixth_failure_in_five_minutes_is_limited() {
        let l = Limiter::default();
        let t0 = Instant::now();
        for i in 0..5 {
            assert_eq!(l.attempt(ip(1), t0 + Duration::from_secs(i)), Ok(()));
        }
        let retry = l.attempt(ip(1), t0 + Duration::from_secs(10)).unwrap_err();
        assert!(retry > 0 && retry <= 300, "{retry}");
        assert_eq!(retry, 290);
        // Another IP is not affected.
        assert_eq!(l.attempt(ip(2), t0), Ok(()));
    }

    #[test]
    fn failures_expire_after_window() {
        let l = Limiter::default();
        let t0 = Instant::now();
        for _ in 0..5 {
            l.attempt(ip(1), t0).unwrap();
        }
        assert!(l.attempt(ip(1), t0 + Duration::from_secs(299)).is_err());
        assert_eq!(l.attempt(ip(1), t0 + WINDOW), Ok(()));
        // A success clears the IP.
        let t1 = t0 + WINDOW;
        for _ in 0..4 {
            l.attempt(ip(1), t1).unwrap();
        }
        l.clear(ip(1));
        for _ in 0..5 {
            assert_eq!(l.attempt(ip(1), t1), Ok(()));
        }
    }

    #[test]
    fn limiter_is_bounded() {
        let l = Limiter::default();
        let t0 = Instant::now();
        for n in 0..=MAX_IPS as u32 {
            l.attempt(ip(n), t0 + Duration::from_millis(n.into()))
                .unwrap();
        }
        let ips = l.ips.lock().unwrap();
        assert!(ips.len() <= MAX_IPS, "{}", ips.len());
        // The entry with the oldest last failure went first.
        assert!(!ips.contains_key(&ip(0)));
        assert!(ips.contains_key(&ip(MAX_IPS as u32)));
    }

    // router

    fn app_with(auth: Option<Arc<Auth>>, peer: [u8; 4]) -> Router {
        let repo = Arc::new(FakeRepo::default());
        let metrics = ApiMetrics::default();
        let fetch: LagFetch = Arc::new(|| Box::pin(async { Ok(Vec::new()) }));
        let v2 = routes_v2::router(
            repo.clone(),
            metrics.clone(),
            LagCache::new(fetch, LAG_TTL),
            ClientConfig::new("", "", auth.is_some()),
        );
        // Composed as in main.
        let mut app = api_router(repo, metrics)
            .merge(v2)
            .merge(tayga_common::metrics::router(Arc::new(Registry::default())))
            .merge(spa::router_with(Arc::new(NoAssets)))
            .merge(routes(auth.clone()));
        if let Some(auth) = auth {
            app = app.layer(from_fn_with_state(auth, require));
        }
        app.layer(MockConnectInfo(SocketAddr::from((peer, 4000))))
    }

    fn app(auth: Arc<Auth>) -> Router {
        app_with(Some(auth), PEER)
    }

    async fn send(app: &Router, req: Request<Body>) -> (StatusCode, HeaderMap, String) {
        let res = app.clone().oneshot(req).await.unwrap();
        let (status, headers) = (res.status(), res.headers().clone());
        let body = axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .unwrap();
        (status, headers, String::from_utf8_lossy(&body).into_owned())
    }

    fn get_req(uri: &str) -> axum::http::request::Builder {
        Request::builder().method(Method::GET).uri(uri)
    }

    fn login_req(username: &str, password: &str) -> Request<Body> {
        Request::post("/api/v1/auth/login")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({ "username": username, "password": password }).to_string(),
            ))
            .unwrap()
    }

    /// `name=value` of the session cookie in `Set-Cookie`.
    fn cookie_pair(headers: &HeaderMap) -> String {
        let set = headers.get(header::SET_COOKIE).unwrap().to_str().unwrap();
        set.split(';').next().unwrap().to_string()
    }

    #[tokio::test]
    async fn login_sets_cookie_and_me_works() {
        let app = app(auth());
        let (status, headers, _) = send(&app, login_req("admin", "secret")).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let set = headers.get(header::SET_COOKIE).unwrap().to_str().unwrap();
        assert!(set.starts_with("tayga_session="), "{set}");
        assert!(
            set.ends_with("; HttpOnly; SameSite=Strict; Path=/; Max-Age=43200"),
            "{set}"
        );
        assert!(!set.contains("Secure"), "{set}");
        let (status, _, body) = send(
            &app,
            get_req("/api/v1/auth/me")
                .header(
                    header::COOKIE,
                    format!("other=1; {}", cookie_pair(&headers)),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, r#"{"username":"admin"}"#);

        let secure = Auth::from_settings(&AuthSettings {
            secure_cookie: true,
            ..settings()
        })
        .unwrap()
        .unwrap();
        let (_, headers, _) =
            send(&app_with(Some(secure), PEER), login_req("admin", "secret")).await;
        let set = headers.get(header::SET_COOKIE).unwrap().to_str().unwrap();
        assert!(set.ends_with("; Secure"), "{set}");
    }

    #[tokio::test]
    async fn wrong_password_and_unknown_user_give_same_401() {
        let app = app(auth());
        let wrong = send(&app, login_req("admin", "nope")).await;
        let unknown = send(&app, login_req("root", "secret")).await;
        assert_eq!(wrong.0, StatusCode::UNAUTHORIZED);
        assert_eq!((wrong.0, &wrong.2), (unknown.0, &unknown.2));
        assert_eq!(wrong.2, r#"{"error":"unauthorized"}"#);
        for h in [&wrong.1, &unknown.1] {
            assert!(h.get(header::SET_COOKIE).is_none());
            assert!(h.get(header::WWW_AUTHENTICATE).is_none());
        }
    }

    #[tokio::test]
    async fn login_requires_json_content_type() {
        let app = app(auth());
        let body = r#"{"username":"admin","password":"secret"}"#;
        for ct in [
            None,
            Some("text/plain"),
            Some("application/x-www-form-urlencoded"),
        ] {
            let mut req = Request::post("/api/v1/auth/login");
            if let Some(ct) = ct {
                req = req.header(header::CONTENT_TYPE, ct);
            }
            let (status, headers, _) = send(&app, req.body(Body::from(body)).unwrap()).await;
            assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE, "{ct:?}");
            assert!(headers.get(header::SET_COOKIE).is_none());
        }
        let (status, _, _) = send(
            &app,
            Request::post("/api/v1/auth/login")
                .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn sixth_bad_login_is_429_with_retry_after() {
        let app = app(auth());
        for _ in 0..5 {
            let (status, _, _) = send(&app, login_req("admin", "nope")).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
        }
        // Even the right password is refused while limited.
        let (status, headers, body) = send(&app, login_req("admin", "secret")).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        let retry: u64 = headers[header::RETRY_AFTER]
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        assert!(retry > 0 && retry <= 300, "{retry}");
        assert_eq!(body, r#"{"error":"too many attempts"}"#);
    }

    #[tokio::test]
    async fn forwarded_for_does_not_dodge_the_limiter() {
        let auth = auth();
        let app = app(auth.clone());
        for i in 0..6 {
            let mut req = login_req("admin", "nope");
            req.headers_mut().insert(
                "x-forwarded-for",
                HeaderValue::from_str(&format!("192.0.2.{i}")).unwrap(),
            );
            let (status, _, _) = send(&app, req).await;
            let want = if i < 5 {
                StatusCode::UNAUTHORIZED
            } else {
                StatusCode::TOO_MANY_REQUESTS
            };
            assert_eq!(status, want, "attempt {i}");
        }
        // The key is the peer address: another peer is not limited.
        let other = app_with(Some(auth), [10, 0, 0, 2]);
        let (status, _, _) = send(&other, login_req("admin", "secret")).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn protected_api_needs_session_or_basic() {
        let app = app(auth());
        let uri = "/api/v1/story-groups";
        let (status, headers, body) = send(&app, get_req(uri).body(Body::empty()).unwrap()).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body, r#"{"error":"unauthorized"}"#);
        assert!(headers.get(header::WWW_AUTHENTICATE).is_none());

        let (_, login_headers, _) = send(&app, login_req("admin", "secret")).await;
        let (status, _, _) = send(
            &app,
            get_req(uri)
                .header(header::COOKIE, cookie_pair(&login_headers))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "cookie");

        let (status, _, _) = send(
            &app,
            get_req(uri)
                .header(header::AUTHORIZATION, basic("admin:secret"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "basic");

        for bad in [
            basic("admin:nope"),
            basic("root:secret"),
            "Basic !!!".to_string(),
            String::new(),
        ] {
            let (status, headers, body) = send(
                &app,
                get_req(uri)
                    .header(header::AUTHORIZATION, &bad)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{bad}");
            assert_eq!(body, r#"{"error":"unauthorized"}"#);
            assert!(headers.get(header::WWW_AUTHENTICATE).is_none());
        }

        let (status, _, _) = send(
            &app,
            get_req(uri)
                .header(header::COOKIE, "tayga_session=forged.token")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "forged cookie");
    }

    #[tokio::test]
    async fn basic_failures_count_toward_the_limit() {
        let app = app(auth());
        let req = |cred: &str| {
            get_req("/api/v1/story-groups")
                .header(header::AUTHORIZATION, basic(cred))
                .body(Body::empty())
                .unwrap()
        };
        for _ in 0..5 {
            assert_eq!(
                send(&app, req("admin:nope")).await.0,
                StatusCode::UNAUTHORIZED
            );
        }
        let (status, headers, _) = send(&app, req("admin:secret")).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert!(headers.get(header::RETRY_AFTER).is_some());
    }

    #[tokio::test]
    async fn open_routes_stay_open() {
        let app = app(auth());
        for uri in ["/healthz", "/metrics", "/api/v1/config", "/", "/stories/x"] {
            let (status, _, _) = send(&app, get_req(uri).body(Body::empty()).unwrap()).await;
            assert_eq!(status, StatusCode::OK, "{uri}");
        }
        // Missing asset: the SPA's own 404, not the guard's 401.
        let (status, _, _) = send(&app, get_req("/assets/x").body(Body::empty()).unwrap()).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (_, _, body) = send(&app, get_req("/api/v1/config").body(Body::empty()).unwrap()).await;
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["auth_enabled"], true);
        // `me` is reachable without a session; its own handler answers 401.
        let (status, _, body) = send(
            &app,
            get_req("/api/v1/auth/me").body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body, r#"{"error":"unauthorized"}"#);
        // Unmatched API paths are guarded too.
        let (status, _, _) = send(&app, get_req("/api/v1/nope").body(Body::empty()).unwrap()).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn disabled_auth_changes_nothing() {
        let app = app_with(None, PEER);
        let (status, _, _) = send(
            &app,
            get_req("/api/v1/story-groups").body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        for (method, uri) in [
            (Method::GET, "/api/v1/auth/me"),
            (Method::POST, "/api/v1/auth/login"),
            (Method::POST, "/api/v1/auth/logout"),
        ] {
            let (status, _, _) = send(
                &app,
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
        }
        let (_, _, body) = send(&app, get_req("/api/v1/config").body(Body::empty()).unwrap()).await;
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["auth_enabled"], false);
    }

    #[tokio::test]
    async fn logout_clears_cookie() {
        let app = app(auth());
        let (_, login_headers, _) = send(&app, login_req("admin", "secret")).await;
        let (status, headers, _) = send(
            &app,
            Request::post("/api/v1/auth/logout")
                .header(header::COOKIE, cookie_pair(&login_headers))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let set = headers.get(header::SET_COOKIE).unwrap().to_str().unwrap();
        assert_eq!(
            set,
            "tayga_session=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0"
        );
    }
}
