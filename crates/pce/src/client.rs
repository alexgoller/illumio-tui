//! The only place HTTP happens. Every request passes the rate limiter, is
//! counted, and has its status mapped to `PceError`. GETs back off on
//! 429/503; POSTs never retry (a repeated POST could apply a change twice).
use crate::auth::Credentials;
use crate::error::{PceError, Result};
use crate::limiter::RateLimiter;
use reqwest::StatusCode;
use reqwest::header::{ACCEPT, HeaderMap, HeaderValue};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::time::Instant;
use url::Url;

#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub scheme: String,
    pub host: String,
    pub port: u16,
    pub org_id: u64,
    pub tls_verify: bool,
    pub credentials: Credentials,
    pub requests_per_second: f64,
    pub burst: u32,
    pub timeout: Duration,
    pub backoff_base: Duration,
    pub max_retries: u32,
}

impl ClientConfig {
    pub fn new(host: impl Into<String>, org_id: u64, credentials: Credentials) -> Self {
        Self {
            scheme: "https".into(),
            host: host.into(),
            port: 443,
            org_id,
            tls_verify: true,
            credentials,
            requests_per_second: 2.0,
            burst: 5,
            timeout: Duration::from_secs(30),
            backoff_base: Duration::from_secs(2),
            max_retries: 4,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ApiResponse<T> {
    pub body: T,
    pub status: u16,
    pub total_count: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct StatsSnapshot {
    pub calls: u64,
    pub bytes: u64,
}

/// HTTP method of a mutation. Serialises as the method name so it can be
/// printed in dry runs and audit entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum WriteMethod {
    Post,
    Put,
    Delete,
}

impl WriteMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            WriteMethod::Post => "POST",
            WriteMethod::Put => "PUT",
            WriteMethod::Delete => "DELETE",
        }
    }
}

#[derive(Default)]
struct Stats {
    calls: AtomicU64,
    bytes: AtomicU64,
}

pub struct PceClient {
    http: reqwest::Client,
    base: Url,
    org_id: u64,
    credentials: Credentials,
    limiter: RateLimiter,
    stats: Stats,
    timeout: Duration,
    backoff_base: Duration,
    max_retries: u32,
    paused_until: Mutex<Option<Instant>>,
}

impl PceClient {
    pub fn new(cfg: ClientConfig) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(cfg.timeout)
            .danger_accept_invalid_certs(!cfg.tls_verify)
            .user_agent(concat!("illumio-tui/", env!("CARGO_PKG_VERSION")))
            // The PCE API never redirects legitimately, and a followed 3xx
            // turns a POST/PUT/DELETE into a GET whose 200 would read as a
            // success. A 3xx maps to `PceError::Server`.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| PceError::Network(e.to_string()))?;
        let base = Url::parse(&format!(
            "{}://{}:{}/api/v2",
            cfg.scheme, cfg.host, cfg.port
        ))
        .map_err(|e| PceError::Invalid(format!("bad PCE host: {e}")))?;
        Ok(Self {
            http,
            base,
            org_id: cfg.org_id,
            credentials: cfg.credentials,
            limiter: RateLimiter::new(cfg.requests_per_second, cfg.burst),
            stats: Stats::default(),
            timeout: cfg.timeout,
            backoff_base: cfg.backoff_base,
            max_retries: cfg.max_retries,
            paused_until: Mutex::new(None),
        })
    }

    pub fn org_id(&self) -> u64 {
        self.org_id
    }

    /// `/orgs/{org}/{path}` — the api path for an org-scoped resource.
    pub fn org_path(&self, path: &str) -> String {
        format!("/orgs/{}/{}", self.org_id, path.trim_start_matches('/'))
    }

    /// Full URL for an api path such as `/orgs/5/labels` or a PCE href.
    pub fn url_for(&self, api_path: &str) -> Url {
        let mut u = self.base.clone();
        let joined = format!(
            "{}/{}",
            u.path().trim_end_matches('/'),
            api_path.trim_start_matches('/')
        );
        u.set_path(&joined);
        u
    }

    pub fn stats(&self) -> StatsSnapshot {
        StatsSnapshot {
            calls: self.stats.calls.load(Ordering::Relaxed),
            bytes: self.stats.bytes.load(Ordering::Relaxed),
        }
    }

    pub async fn get<T: DeserializeOwned>(
        &self,
        api_path: &str,
        query: &[(&str, String)],
    ) -> Result<ApiResponse<T>> {
        self.get_with_timeout(api_path, query, self.timeout).await
    }

    /// `get` with its own timeout for the whole request, body included
    /// (a large traffic download).
    pub async fn get_with_timeout<T: DeserializeOwned>(
        &self,
        api_path: &str,
        query: &[(&str, String)],
        timeout: Duration,
    ) -> Result<ApiResponse<T>> {
        let url = self.url_for(api_path);
        let build = || self.http.get(url.clone()).query(query).timeout(timeout);
        let (status, headers, bytes) = self.send(build, true, timeout).await?;
        Self::decode(api_path, status, &headers, &bytes)
    }

    pub async fn post<B: Serialize, T: DeserializeOwned>(
        &self,
        api_path: &str,
        body: &B,
        headers: &[(&str, &str)],
    ) -> Result<ApiResponse<T>> {
        let url = self.url_for(api_path);
        let payload = serde_json::to_vec(body).map_err(|e| PceError::Invalid(e.to_string()))?;
        let build = || {
            let mut r = self
                .http
                .post(url.clone())
                .header("Content-Type", "application/json")
                .body(payload.clone());
            for (k, v) in headers {
                r = r.header(*k, *v);
            }
            r
        };
        let (status, hdrs, bytes) = self.send(build, false, self.timeout).await?;
        Self::decode(api_path, status, &hdrs, &bytes)
    }

    /// Sends one mutation. Never retried: a 429 or 503 comes back as an
    /// error, because a repeated write could apply a change twice. 400, 406,
    /// 409 and 422 map to [`PceError::Rejected`]; a 204 or empty body decodes
    /// to `Value::Null`. Crate-private: only `pce::write::Writer` sends
    /// mutations, behind a `WriteGuard`.
    pub(crate) async fn write(
        &self,
        method: WriteMethod,
        api_path: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<ApiResponse<serde_json::Value>> {
        let url = self.url_for(api_path);
        let payload = match body {
            Some(b) => Some(serde_json::to_vec(b).map_err(|e| PceError::Invalid(e.to_string()))?),
            None => None,
        };
        let build = || {
            let r = match method {
                WriteMethod::Post => self.http.post(url.clone()),
                WriteMethod::Put => self.http.put(url.clone()),
                WriteMethod::Delete => self.http.delete(url.clone()),
            };
            match &payload {
                Some(p) => r.header("Content-Type", "application/json").body(p.clone()),
                None => r,
            }
        };
        let (status, headers, bytes) = self.send(build, false, self.timeout).await?;
        match status.as_u16() {
            400 | 406 | 409 | 422 => Err(PceError::Rejected {
                status: status.as_u16(),
                body: pce_error_message(&bytes),
            }),
            _ => Self::decode(api_path, status, &headers, &bytes),
        }
    }

    async fn send<F>(
        &self,
        build: F,
        retry: bool,
        timeout: Duration,
    ) -> Result<(StatusCode, HeaderMap, bytes::Bytes)>
    where
        F: Fn() -> reqwest::RequestBuilder,
    {
        let mut attempt: u32 = 0;
        loop {
            self.wait_if_paused().await;
            self.limiter.acquire().await;
            self.stats.calls.fetch_add(1, Ordering::Relaxed);
            let req = build()
                .basic_auth(
                    self.credentials.api_key(),
                    Some(self.credentials.expose_secret()),
                )
                .header(ACCEPT, HeaderValue::from_static("application/json"));
            let resp = req.send().await.map_err(|e| map_reqwest(e, timeout))?;
            let status = resp.status();
            let headers = resp.headers().clone();
            let bytes = resp.bytes().await.map_err(|e| map_reqwest(e, timeout))?;
            self.stats
                .bytes
                .fetch_add(bytes.len() as u64, Ordering::Relaxed);

            let throttled = status == StatusCode::TOO_MANY_REQUESTS
                || status == StatusCode::SERVICE_UNAVAILABLE;
            if throttled && retry && attempt < self.max_retries {
                let delay = parse_retry_after(&headers)
                    .unwrap_or_else(|| {
                        self.backoff_base
                            .saturating_mul(2u32.saturating_pow(attempt))
                    })
                    .min(Duration::from_secs(60));
                self.pause_for(
                    delay
                        + jitter(
                            attempt,
                            (self.backoff_base / 4).min(Duration::from_millis(250)),
                        ),
                );
                attempt += 1;
                continue;
            }
            if status == StatusCode::TOO_MANY_REQUESTS {
                return Err(PceError::RateLimited {
                    retry_after: parse_retry_after(&headers),
                });
            }
            return Ok((status, headers, bytes));
        }
    }

    fn decode<T: DeserializeOwned>(
        api_path: &str,
        status: StatusCode,
        headers: &HeaderMap,
        bytes: &[u8],
    ) -> Result<ApiResponse<T>> {
        match status.as_u16() {
            200..=299 => {}
            401 | 403 => {
                return Err(PceError::Auth {
                    status: status.as_u16(),
                });
            }
            404 => {
                return Err(PceError::NotFound {
                    path: api_path.to_string(),
                });
            }
            406 | 400 => return Err(PceError::InvalidFilter(pce_error_message(bytes))),
            s => {
                return Err(PceError::Server {
                    status: s,
                    body: pce_error_message(bytes),
                });
            }
        }
        let total_count = headers
            .get("X-Total-Count")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse().ok());
        let body = if bytes.is_empty() {
            serde_json::from_str("null")
        } else {
            serde_json::from_slice(bytes)
        }
        .map_err(|e| PceError::Decode(format!("{api_path}: {e}")))?;
        Ok(ApiResponse {
            body,
            status: status.as_u16(),
            total_count,
        })
    }

    fn pause_for(&self, d: Duration) {
        let mut p = self.paused_until.lock().expect("pause mutex poisoned");
        let until = Instant::now() + d;
        if p.is_none_or(|existing| existing < until) {
            *p = Some(until);
        }
    }

    async fn wait_if_paused(&self) {
        let until = *self.paused_until.lock().expect("pause mutex poisoned");
        if let Some(until) = until {
            tokio::time::sleep_until(until).await;
        }
    }
}

/// Parses `Retry-After` when it is a number of seconds. HTTP-date values
/// are ignored (caller falls back to computed backoff).
pub fn parse_retry_after(headers: &HeaderMap) -> Option<Duration> {
    headers
        .get("Retry-After")?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
}

/// Small spread (up to `cap`) so clients that back off together do not
/// retry in lock-step. Not cryptographic, not random; good enough.
fn jitter(attempt: u32, cap: Duration) -> Duration {
    let cap_ms = cap.as_millis() as u64;
    if cap_ms == 0 {
        return Duration::ZERO;
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::from(d.subsec_nanos()))
        .unwrap_or(0);
    Duration::from_millis(nanos.wrapping_add(u64::from(attempt) * 7919) % cap_ms)
}

/// PCE error bodies are `[{"token": "...", "message": "..."}]`. Fall back to
/// the raw text, truncated.
fn pce_error_message(bytes: &[u8]) -> String {
    #[derive(serde::Deserialize)]
    struct Item {
        token: Option<String>,
        message: Option<String>,
    }
    if let Ok(items) = serde_json::from_slice::<Vec<Item>>(bytes) {
        let parts: Vec<String> = items
            .into_iter()
            .map(|i| match (i.token, i.message) {
                (Some(t), Some(m)) => format!("{t}: {m}"),
                (Some(t), None) => t,
                (None, Some(m)) => m,
                (None, None) => String::new(),
            })
            .filter(|s| !s.is_empty())
            .collect();
        if !parts.is_empty() {
            return parts.join("; ");
        }
    }
    let text = String::from_utf8_lossy(bytes);
    text.chars().take(500).collect()
}

fn map_reqwest(e: reqwest::Error, timeout: Duration) -> PceError {
    if e.is_timeout() {
        PceError::Timeout(timeout)
    } else {
        PceError::Network(e.to_string())
    }
}

#[cfg(test)]
mod write_tests {
    use super::*;
    use crate::auth::Credentials;
    use serde_json::json;
    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client_for(server: &MockServer) -> PceClient {
        let url = url::Url::parse(&server.uri()).unwrap();
        let mut cfg = ClientConfig::new(
            url.host_str().unwrap(),
            5,
            Credentials::new("key", "secret"),
        );
        cfg.scheme = "http".into();
        cfg.port = url.port().unwrap();
        cfg.backoff_base = Duration::from_millis(1);
        cfg.requests_per_second = 1000.0;
        cfg.burst = 1000;
        PceClient::new(cfg).unwrap()
    }

    #[tokio::test]
    async fn write_sends_json_body_once_and_decodes_201() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v2/orgs/5/labels"))
            .and(header("Authorization", "Basic a2V5OnNlY3JldA=="))
            .and(header("Content-Type", "application/json"))
            .and(body_json(json!({"key": "app", "value": "web"})))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(
                    json!({"href": "/orgs/5/labels/9", "key": "app", "value": "web"}),
                ),
            )
            .expect(1)
            .mount(&server)
            .await;
        let c = client_for(&server);
        let r = c
            .write(
                WriteMethod::Post,
                &c.org_path("labels"),
                Some(&json!({"key": "app", "value": "web"})),
            )
            .await
            .unwrap();
        assert_eq!(r.status, 201);
        assert_eq!(r.body["href"], "/orgs/5/labels/9");
        assert_eq!(c.stats().calls, 1);
    }

    #[tokio::test]
    async fn write_treats_204_as_null_body() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/api/v2/orgs/5/labels/9"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/api/v2/orgs/5/labels/9"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;
        let c = client_for(&server);
        let r = c
            .write(
                WriteMethod::Put,
                "/orgs/5/labels/9",
                Some(&json!({"value": "x"})),
            )
            .await
            .unwrap();
        assert_eq!(r.status, 204);
        assert!(r.body.is_null());
        let r = c
            .write(WriteMethod::Delete, "/orgs/5/labels/9", None)
            .await
            .unwrap();
        assert_eq!(r.status, 204);
        assert!(r.body.is_null());
    }

    #[tokio::test]
    async fn write_maps_validation_statuses_to_rejected() {
        let server = MockServer::start().await;
        for (p, status) in [("a", 400), ("b", 406), ("c", 409), ("d", 422)] {
            Mock::given(method("POST"))
                .and(path(format!("/api/v2/orgs/5/{p}")))
                .respond_with(ResponseTemplate::new(status).set_body_json(
                    json!([{"token": "name_must_be_unique", "message": "Name must be unique"}]),
                ))
                .mount(&server)
                .await;
        }
        let c = client_for(&server);
        for (p, status) in [("a", 400u16), ("b", 406), ("c", 409), ("d", 422)] {
            let err = c
                .write(WriteMethod::Post, &c.org_path(p), Some(&json!({})))
                .await
                .unwrap_err();
            match err {
                PceError::Rejected { status: s, body } => {
                    assert_eq!(s, status);
                    assert_eq!(body, "name_must_be_unique: Name must be unique");
                }
                other => panic!("{p}: expected Rejected, got {other:?}"),
            }
            assert_eq!(err_kind(&c, p).await, "rejected");
        }
    }

    async fn err_kind(c: &PceClient, p: &str) -> &'static str {
        c.write(WriteMethod::Post, &c.org_path(p), Some(&json!({})))
            .await
            .unwrap_err()
            .kind()
    }

    #[tokio::test]
    async fn write_is_never_retried_on_429_or_503() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/api/v2/orgs/5/x"))
            .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "1"))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/api/v2/orgs/5/y"))
            .respond_with(ResponseTemplate::new(503))
            .expect(1)
            .mount(&server)
            .await;
        let c = client_for(&server);
        let err = c
            .write(WriteMethod::Put, "/orgs/5/x", Some(&json!({})))
            .await
            .unwrap_err();
        assert!(matches!(err, PceError::RateLimited { .. }), "{err:?}");
        // A write 429 sets no client pause (nothing is retried); the DELETE is sent once.
        let err = c
            .write(WriteMethod::Delete, "/orgs/5/y", None)
            .await
            .unwrap_err();
        assert!(
            matches!(err, PceError::Server { status: 503, .. }),
            "{err:?}"
        );
        assert_eq!(c.stats().calls, 2);
    }

    #[tokio::test]
    async fn write_does_not_follow_redirects() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/api/v2/orgs/5/labels/9"))
            .respond_with(
                ResponseTemplate::new(302).insert_header("Location", "/api/v2/orgs/5/labels/9"),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
            .expect(0)
            .mount(&server)
            .await;
        let c = client_for(&server);
        let err = c
            .write(
                WriteMethod::Put,
                "/orgs/5/labels/9",
                Some(&json!({"value": "x"})),
            )
            .await
            .unwrap_err();
        assert!(
            matches!(err, PceError::Server { status: 302, .. }),
            "{err:?}"
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
        assert_eq!(c.stats().calls, 1);
    }
}
