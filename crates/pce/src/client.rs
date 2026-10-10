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
        let url = self.url_for(api_path);
        let build = || self.http.get(url.clone()).query(query);
        let (status, headers, bytes) = self.send(build, true).await?;
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
        let (status, hdrs, bytes) = self.send(build, false).await?;
        Self::decode(api_path, status, &hdrs, &bytes)
    }

    async fn send<F>(&self, build: F, retry: bool) -> Result<(StatusCode, HeaderMap, bytes::Bytes)>
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
            let resp = req.send().await.map_err(|e| self.map_reqwest(e))?;
            let status = resp.status();
            let headers = resp.headers().clone();
            let bytes = resp.bytes().await.map_err(|e| self.map_reqwest(e))?;
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

    fn map_reqwest(&self, e: reqwest::Error) -> PceError {
        if e.is_timeout() {
            PceError::Timeout(self.timeout)
        } else {
            PceError::Network(e.to_string())
        }
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
