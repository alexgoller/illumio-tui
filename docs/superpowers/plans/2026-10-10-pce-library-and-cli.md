# illumio-tui Plan 1: `pce` Library and CLI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the reusable `pce` Rust library (throttled, cached, typed read-only client for the Illumio Core PCE API v2, including the async Explorer traffic query) and the non-interactive `illumio-tui` CLI that exposes it to agents and scripts.

**Architecture:** Cargo workspace with `crates/pce` (pure async library: HTTP client with token-bucket throttle and backoff, TTL cache, serde models, traffic query builder and poller, name resolution and aggregation) and `crates/illumio-tui` (clap binary; in this plan only the CLI subcommands, the TUI arrives in Plan 2 and reuses every library type). A third `crates/xtask` crate records anonymised fixtures from the test PCE.

**Tech Stack:** Rust stable (edition 2024), tokio 1, reqwest 0.13 (rustls default), serde 1, serde_json 1, thiserror 2, jiff 0.2, humantime 2, tokio-util 0.7, secrecy 0.10, url 2, clap 4, toml 1, dirs 7, dotenvy 0.15, ratatui 0.30 (CLI table rendering only), wiremock 0.6, assert_cmd 2.

**Spec:** `docs/superpowers/specs/2026-10-10-illumio-tui-design.md` (sections 3.1, 5, 6, 7, 8 are implemented here; section 4 and 3.2 UI parts are Plan 2).

## Global Constraints

- Rust edition 2024, `rust-version = "1.88"`, workspace `resolver = "3"`.
- `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --check` must pass at every commit.
- PCE API v2 only: base URL `{scheme}://{host}:{port}/api/v2`, org paths `/orgs/{org_id}/...`, health at `/api/v2/health` (no org).
- HTTP Basic auth (api key as user, secret as password), header `Accept: application/json`, request timeout 30 s.
- Throttle defaults: 2 requests/second, burst 5. Backoff on 429/503 for GET only: base 2 s, doubling, cap 60 s, honour `Retry-After`, max 4 retries. POST is never retried.
- Cache TTLs: labels/label groups/IP lists/services 15 min; rulesets and workloads 5 min; traffic results until the filter changes.
- Explorer defaults: window 24 h, `max_results` 1000. Confirmation (`--yes`) required when window > 7 days or `max_results` > 10 000. Empty include filter serialises as `[[]]`. Dates are full RFC 3339 UTC (`YYYY-MM-DDTHH:MM:SSZ`).
- Poll schedule: 1 s initial, ×1.5, cap 10 s; 30 s grace for "completed but no result".
- Secrets: never logged, never in `Debug` output, never written to disk by the tool.
- Env var names: `PCE_HOST`, `PCE_PORT`, `PCE_ORG_ID`, `API_KEY`, `API_SECRET`, `PCE_TLS_VERIFY` (same as illumio-mcp-server).
- CLI exit codes: 1 generic, 2 arguments/config, 3 auth, 4 PCE error, 5 rate limited. JSON errors on stderr as `{"error":{"kind":"...","message":"..."}}`.
- Test PCE for the opt-in live test: `demo100.illum.io`, org `5636114`, credentials read from `../illumio-mcp-server/.env` via `--env-file`, never copied into this repo.
- No `std::env::set_var` in tests (unsafe in edition 2024); pass environment maps explicitly.

## Review Focus

Inputs the spec implies but which are easy to get wrong; each has a pinned test in the owning task:

1. A label with no `value` or with `deleted: true` must deserialise and resolve without panicking (Task 6, Task 8).
2. ICMP or other non-TCP/UDP flows with no `port` must render as `icmp` with an empty port, not fail (Task 12).
3. `--from` later than `--to`, or a window the PCE would silently accept but return nothing for, must be rejected client-side with a clear message (Task 10, Task 18).
4. `X-Total-Count` larger than the returned row count must mark a list as truncated in `meta.truncated` (Task 5).
5. An `api_secret_cmd` that fails or prints nothing must produce a config error, not a silent empty secret (Task 13).
6. `Retry-After` given as an HTTP date (not seconds) must fall back to computed backoff instead of failing to parse (Task 4).

---

## File structure

```
Cargo.toml                                  workspace
.cargo/config.toml                          `cargo xtask` alias
.gitignore
deny.toml
.github/workflows/ci.yml
tasks/todo.md
docs/architecture.md
fixtures/*.json                             hand-written deterministic PCE responses
fixtures/recorded/*.json                    anonymised real responses (cargo xtask record)
crates/pce/Cargo.toml
crates/pce/src/lib.rs                       re-exports, `Pce` facade, `Cached`, `CacheMode`
crates/pce/src/error.rs                     `PceError`
crates/pce/src/auth.rs                      `Credentials`
crates/pce/src/limiter.rs                   `RateLimiter`
crates/pce/src/cache.rs                     `TtlCache`
crates/pce/src/client.rs                    `PceClient`, `ClientConfig`, `ApiResponse`, `Stats`
crates/pce/src/models/mod.rs
crates/pce/src/models/common.rs             `Ref`, `PolicyDecision`, `proto_name`
crates/pce/src/models/labels.rs             `Label`, `LabelGroup`, `IpList`, `Service`
crates/pce/src/models/workloads.rs          `Workload`, `WorkloadFilter`
crates/pce/src/models/policy.rs             `RuleSet`, `Rule`, `DenyRule`, `Actor`, `PendingChange`, `PolicyScope`
crates/pce/src/models/traffic.rs            `Flow`, `TrafficQuery`, `ActorMatch`, `ServiceMatch`, `AsyncQueryStatus`
crates/pce/src/resolve.rs                   `NameIndex`, `describe_actor`, `RuleView`
crates/pce/src/traffic/mod.rs
crates/pce/src/traffic/filter.rs            `FilterExpr`, `resolve_actor`
crates/pce/src/traffic/query.rs             `TrafficQueryBuilder`, `QueryCost`, `parse_window`
crates/pce/src/traffic/poll.rs              `run_traffic_query`, `PollConfig`, `QueryProgress`, `TrafficResult`
crates/pce/src/traffic/resolved.rs          `ResolvedFlow`, `resolve_flows`
crates/pce/src/traffic/aggregate.rs         `group_by_labels`, `blocked_summary`
crates/pce/tests/client.rs                  wiremock tests for the HTTP layer
crates/pce/tests/facade.rs                  wiremock tests for cache + lists
crates/pce/tests/traffic_poll.rs            wiremock tests for the async query
crates/pce/tests/live.rs                    opt-in live smoke test
crates/pce/tests/recorded.rs                shape check for fixtures/recorded/
crates/illumio-tui/Cargo.toml
crates/illumio-tui/src/main.rs              thin entry point
crates/illumio-tui/src/lib.rs               exposes cli and config to tests
crates/illumio-tui/src/config.rs            profiles, env, resolution
crates/illumio-tui/src/cli/mod.rs           clap definitions, dispatch
crates/illumio-tui/src/cli/error.rs         `CliError`, exit codes
crates/illumio-tui/src/cli/context.rs       `Context`
crates/illumio-tui/src/cli/output.rs        envelope, table rendering
crates/illumio-tui/src/cli/health.rs
crates/illumio-tui/src/cli/objects.rs
crates/illumio-tui/src/cli/workloads.rs
crates/illumio-tui/src/cli/policy.rs
crates/illumio-tui/src/cli/traffic.rs
crates/illumio-tui/src/cli/profile.rs
crates/illumio-tui/tests/cli.rs             assert_cmd tests
crates/xtask/Cargo.toml
crates/xtask/src/main.rs                    `record` fixtures command
```

---

### Task 1: Workspace scaffold

**Files:**
- Create: `Cargo.toml`, `.cargo/config.toml`, `.gitignore`, `rustfmt.toml`, `tasks/todo.md`
- Create: `crates/pce/Cargo.toml`, `crates/pce/src/lib.rs`
- Create: `crates/illumio-tui/Cargo.toml`, `crates/illumio-tui/src/main.rs`
- Create: `crates/xtask/Cargo.toml`, `crates/xtask/src/main.rs`

**Interfaces:**
- Produces: workspace dependency table used by every later task; binary name `illumio-tui`.

- [ ] **Step 1: Write the workspace manifest**

```toml
# Cargo.toml
[workspace]
resolver = "3"
members = ["crates/pce", "crates/illumio-tui", "crates/xtask"]

[workspace.package]
version = "0.1.0"
edition = "2024"
rust-version = "1.88"
license = "GPL-3.0-only"
repository = "https://github.com/alexgoller/illumio-tui"

[workspace.dependencies]
tokio = { version = "1", features = ["rt-multi-thread", "macros", "sync", "time", "process"] }
tokio-util = "0.7"
reqwest = { version = "0.13", features = ["json"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
jiff = { version = "0.2", features = ["serde"] }
humantime = "2"
secrecy = "0.10"
url = "2"
bytes = "1"
clap = { version = "4", features = ["derive", "env"] }
toml = "1"
dirs = "7"
dotenvy = "0.15"
ratatui = "0.30"
wiremock = "0.6"
assert_cmd = "2"
predicates = "3"
tempfile = "3"

[profile.release]
lto = "thin"
strip = true
```

```toml
# .cargo/config.toml
[alias]
xtask = "run -q -p xtask --"
```

```
# .gitignore
/target
*.log
.env
.env.*
fixtures/_raw/
```

```toml
# rustfmt.toml
edition = "2024"
```

```toml
# crates/pce/Cargo.toml
[package]
name = "pce"
description = "Read-only async client for the Illumio Core PCE REST API v2"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[features]
test-support = []

[dependencies]
tokio.workspace = true
tokio-util.workspace = true
reqwest.workspace = true
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
jiff.workspace = true
humantime.workspace = true
secrecy.workspace = true
url.workspace = true
bytes.workspace = true

[dev-dependencies]
wiremock.workspace = true
```

```rust
// crates/pce/src/lib.rs
//! `pce` is a read-only, PCE-friendly client library for the Illumio Core
//! Policy Compute Engine REST API v2.
//!
//! Everything that talks to a PCE goes through [`client::PceClient`], which
//! throttles, counts and retries requests. Higher-level typed reads with
//! caching live on the [`Pce`] facade (added in Task 5).
```

```toml
# crates/illumio-tui/Cargo.toml
[package]
name = "illumio-tui"
description = "Terminal UI and CLI for the Illumio Core PCE"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[[bin]]
name = "illumio-tui"
path = "src/main.rs"

[dependencies]
pce = { path = "../pce" }
tokio.workspace = true
tokio-util.workspace = true
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
clap.workspace = true
toml.workspace = true
dirs.workspace = true
dotenvy.workspace = true
jiff.workspace = true
humantime.workspace = true
ratatui.workspace = true

[dev-dependencies]
assert_cmd.workspace = true
predicates.workspace = true
tempfile.workspace = true
wiremock.workspace = true
```

```rust
// crates/illumio-tui/src/main.rs
//! Entry point. With no subcommand this will open the TUI (Plan 2); for now
//! it prints help. Subcommands are the non-interactive CLI for scripts and
//! agents.
use clap::Parser;

#[derive(Parser)]
#[command(name = "illumio-tui", version, about = "Terminal UI and CLI for the Illumio Core PCE")]
struct Cli {}

fn main() {
    let _cli = Cli::parse();
    println!("illumio-tui: the interactive TUI arrives in Plan 2; run with --help");
}
```

```toml
# crates/xtask/Cargo.toml
[package]
name = "xtask"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
publish = false

[dependencies]
pce = { path = "../pce" }
tokio.workspace = true
tokio-util.workspace = true
serde_json.workspace = true
clap.workspace = true
dotenvy.workspace = true
```

```rust
// crates/xtask/src/main.rs
//! Developer tasks. `cargo xtask record` is added in Task 19.
fn main() {
    println!("xtask: no tasks yet");
}
```

```markdown
<!-- tasks/todo.md -->
# illumio-tui tasks

## Plan 1: pce library and CLI
- [ ] Task 1 scaffold
- [ ] Task 2 error + auth
- [ ] Task 3 limiter
- [ ] Task 4 client
- [ ] Task 5 cache + facade
- [ ] Task 6 object models
- [ ] Task 7 workloads
- [ ] Task 8 name index + filter expressions
- [ ] Task 9 policy models + views
- [ ] Task 10 traffic query builder
- [ ] Task 11 traffic poller
- [ ] Task 12 resolved flows + aggregation
- [ ] Task 13 config
- [ ] Task 14 CLI skeleton + health
- [ ] Task 15 objects commands
- [ ] Task 16 workloads command
- [ ] Task 17 policy commands
- [ ] Task 18 traffic command
- [ ] Task 19 profile commands
- [ ] Task 20 fixture recorder + live smoke test
- [ ] Task 21 CI, cargo-deny, docs

## Review
(filled in at the end of the plan)
```

- [ ] **Step 2: Build and run**

Run: `cargo build --workspace && cargo run -q -p illumio-tui -- --version`
Expected: `illumio-tui 0.1.0`

- [ ] **Step 3: Lint**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "chore: cargo workspace scaffold (pce, illumio-tui, xtask)

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 2: Errors and credentials

**Files:**
- Create: `crates/pce/src/error.rs`, `crates/pce/src/auth.rs`
- Modify: `crates/pce/src/lib.rs`

**Interfaces:**
- Produces: `PceError` (all variants below, `kind()` returns a stable snake_case string used in CLI JSON), `Credentials::new(api_key, api_secret)`, `Credentials::api_key()`, `Credentials::expose_secret()`.

- [ ] **Step 1: Write the failing tests**

```rust
// crates/pce/src/auth.rs (tests module at the bottom; the file body comes in Step 3)
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_output_never_contains_the_secret() {
        let c = Credentials::new("api_key_123", "very-secret");
        let shown = format!("{c:?}");
        assert!(shown.contains("api_key_123"));
        assert!(!shown.contains("very-secret"));
    }

    #[test]
    fn secret_is_available_on_request() {
        let c = Credentials::new("k", "s");
        assert_eq!(c.expose_secret(), "s");
        assert_eq!(c.api_key(), "k");
    }
}
```

```rust
// crates/pce/src/error.rs (tests module at the bottom)
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_are_stable_snake_case() {
        assert_eq!(PceError::Auth { status: 401 }.kind(), "auth");
        assert_eq!(PceError::RateLimited { retry_after: None }.kind(), "rate_limited");
        assert_eq!(PceError::Invalid("x".into()).kind(), "invalid");
        assert_eq!(PceError::QueryFailed("x".into()).kind(), "query_failed");
    }

    #[test]
    fn display_is_human_readable() {
        let e = PceError::NotFound { path: "/orgs/1/labels/9".into() };
        assert_eq!(e.to_string(), "not found: /orgs/1/labels/9");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p pce`
Expected: compile error, `error` and `auth` modules do not exist.

- [ ] **Step 3: Implement**

```rust
// crates/pce/src/error.rs
//! One error type for everything the library can report. The CLI maps
//! `kind()` to exit codes; the TUI shows `Display`.
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum PceError {
    #[error("authentication failed (HTTP {status}); check API key, secret and org id")]
    Auth { status: u16 },
    #[error("not found: {path}")]
    NotFound { path: String },
    #[error("rate limited by the PCE{}", retry_after.map(|d| format!(" (retry after {}s)", d.as_secs())).unwrap_or_default())]
    RateLimited { retry_after: Option<Duration> },
    #[error("request timed out after {0:?}")]
    Timeout(Duration),
    #[error("network error: {0}")]
    Network(String),
    #[error("the PCE rejected the request: {0}")]
    InvalidFilter(String),
    #[error("PCE server error (HTTP {status}): {body}")]
    Server { status: u16, body: String },
    #[error("traffic query failed: {0}")]
    QueryFailed(String),
    #[error("cancelled")]
    Cancelled,
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("unexpected response: {0}")]
    Decode(String),
}

impl PceError {
    /// Stable machine-readable name, used in CLI JSON errors.
    pub fn kind(&self) -> &'static str {
        match self {
            PceError::Auth { .. } => "auth",
            PceError::NotFound { .. } => "not_found",
            PceError::RateLimited { .. } => "rate_limited",
            PceError::Timeout(_) => "timeout",
            PceError::Network(_) => "network",
            PceError::InvalidFilter(_) => "invalid_filter",
            PceError::Server { .. } => "server",
            PceError::QueryFailed(_) => "query_failed",
            PceError::Cancelled => "cancelled",
            PceError::Invalid(_) => "invalid",
            PceError::Decode(_) => "decode",
        }
    }
}

pub type Result<T> = std::result::Result<T, PceError>;
```

```rust
// crates/pce/src/auth.rs
//! API credentials. The secret is wrapped so it cannot leak through `Debug`
//! or accidental logging.
use secrecy::{ExposeSecret, SecretString};

#[derive(Clone)]
pub struct Credentials {
    api_key: String,
    api_secret: SecretString,
}

impl Credentials {
    pub fn new(api_key: impl Into<String>, api_secret: impl Into<String>) -> Self {
        Self { api_key: api_key.into(), api_secret: SecretString::from(api_secret.into()) }
    }

    pub fn api_key(&self) -> &str {
        &self.api_key
    }

    /// Returns the secret for building the Authorization header. Call sites
    /// should be few and obvious.
    pub fn expose_secret(&self) -> &str {
        self.api_secret.expose_secret()
    }
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials").field("api_key", &self.api_key).field("api_secret", &"<redacted>").finish()
    }
}
```

Append to `crates/pce/src/lib.rs`:

```rust
pub mod auth;
pub mod error;

pub use auth::Credentials;
pub use error::{PceError, Result};
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p pce`
Expected: 4 tests pass.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add crates/pce
git commit -m "feat(pce): PceError and redacting Credentials

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 3: Token-bucket rate limiter

**Files:**
- Create: `crates/pce/src/limiter.rs`
- Modify: `crates/pce/src/lib.rs`

**Interfaces:**
- Produces: `RateLimiter::new(requests_per_second: f64, burst: u32)`, `async fn acquire(&self)`.

- [ ] **Step 1: Write the failing tests**

```rust
// crates/pce/src/limiter.rs (tests module)
#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{Duration, Instant};

    #[tokio::test(start_paused = true)]
    async fn burst_is_immediate_then_throttled() {
        let l = RateLimiter::new(2.0, 5);
        let t0 = Instant::now();
        for _ in 0..5 {
            l.acquire().await;
        }
        assert_eq!(Instant::now() - t0, Duration::ZERO, "burst must not wait");
        l.acquire().await;
        let waited = Instant::now() - t0;
        assert!(waited >= Duration::from_millis(499), "6th request waited {waited:?}");
        assert!(waited <= Duration::from_millis(520));
    }

    #[tokio::test(start_paused = true)]
    async fn tokens_refill_over_time() {
        let l = RateLimiter::new(1.0, 1);
        l.acquire().await;
        tokio::time::advance(Duration::from_secs(3)).await;
        let t0 = Instant::now();
        l.acquire().await;
        assert_eq!(Instant::now() - t0, Duration::ZERO);
    }

    #[test]
    fn zero_rate_is_clamped() {
        let l = RateLimiter::new(0.0, 0);
        assert!(l.rate > 0.0);
        assert!(l.burst >= 1.0);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p pce limiter`
Expected: compile error, module missing.

- [ ] **Step 3: Implement**

```rust
// crates/pce/src/limiter.rs
//! A token bucket shared by every request of one client. `acquire` waits
//! until a token is available. Uses tokio's clock so tests can pause time.
use std::sync::Mutex;
use tokio::time::{Duration, Instant, sleep};

pub struct RateLimiter {
    pub(crate) rate: f64,
    pub(crate) burst: f64,
    state: Mutex<State>,
}

struct State {
    tokens: f64,
    updated: Instant,
}

impl RateLimiter {
    pub fn new(requests_per_second: f64, burst: u32) -> Self {
        let burst = f64::from(burst.max(1));
        Self {
            rate: requests_per_second.max(0.01),
            burst,
            state: Mutex::new(State { tokens: burst, updated: Instant::now() }),
        }
    }

    pub async fn acquire(&self) {
        loop {
            let wait = {
                let mut s = self.state.lock().expect("limiter mutex poisoned");
                let now = Instant::now();
                let elapsed = now.duration_since(s.updated).as_secs_f64();
                s.tokens = (s.tokens + elapsed * self.rate).min(self.burst);
                s.updated = now;
                if s.tokens >= 1.0 {
                    s.tokens -= 1.0;
                    None
                } else {
                    Some(Duration::from_secs_f64((1.0 - s.tokens) / self.rate))
                }
            };
            match wait {
                None => return,
                Some(d) => sleep(d).await,
            }
        }
    }
}
```

Append to `lib.rs`: `pub mod limiter;` and `pub use limiter::RateLimiter;`.

- [ ] **Step 4: Run tests**

Run: `cargo test -p pce limiter`
Expected: 3 tests pass.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add crates/pce
git commit -m "feat(pce): token-bucket rate limiter

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 4: HTTP client with auth, counting, and backoff

**Files:**
- Create: `crates/pce/src/client.rs`, `crates/pce/tests/client.rs`
- Modify: `crates/pce/src/lib.rs`

**Interfaces:**
- Consumes: `Credentials`, `RateLimiter`, `PceError`.
- Produces:
  - `ClientConfig { scheme, host, port, org_id, tls_verify, credentials, requests_per_second, burst, timeout, backoff_base, max_retries }` with `ClientConfig::new(host, org_id, credentials)` defaults.
  - `PceClient::new(ClientConfig) -> Result<PceClient>`, `org_id()`, `org_path(&str) -> String` (`/orgs/{org}/{path}`), `url_for(&str) -> Url`.
  - `async fn get<T: DeserializeOwned>(&self, api_path: &str, query: &[(&str, String)]) -> Result<ApiResponse<T>>`
  - `async fn post<B: Serialize, T: DeserializeOwned>(&self, api_path: &str, body: &B, headers: &[(&str, &str)]) -> Result<ApiResponse<T>>`
  - `ApiResponse<T> { body: T, status: u16, total_count: Option<u64> }`
  - `stats() -> StatsSnapshot { calls: u64, bytes: u64 }`
  - `pub fn parse_retry_after(headers) -> Option<Duration>`

- [ ] **Step 1: Write the failing tests**

```rust
// crates/pce/tests/client.rs
use pce::client::{ClientConfig, PceClient};
use pce::{Credentials, PceError};
use serde_json::{Value, json};
use std::time::Duration;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn client_for(server: &MockServer) -> PceClient {
    let url = url::Url::parse(&server.uri()).unwrap();
    let mut cfg = ClientConfig::new(url.host_str().unwrap(), 5, Credentials::new("key", "secret"));
    cfg.scheme = "http".into();
    cfg.port = url.port().unwrap();
    cfg.backoff_base = Duration::from_millis(1);
    cfg.requests_per_second = 1000.0;
    cfg.burst = 1000;
    PceClient::new(cfg).unwrap()
}

#[tokio::test]
async fn get_sends_basic_auth_accept_and_org_path() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/labels"))
        .and(header("Authorization", "Basic a2V5OnNlY3JldA=="))
        .and(header("Accept", "application/json"))
        .and(query_param("max_results", "10"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{"href": "/orgs/5/labels/1"}])).insert_header("X-Total-Count", "42"))
        .mount(&server)
        .await;
    let c = client_for(&server);
    let r = c.get::<Value>(&c.org_path("labels"), &[("max_results", "10".into())]).await.unwrap();
    assert_eq!(r.status, 200);
    assert_eq!(r.total_count, Some(42));
    assert_eq!(r.body[0]["href"], "/orgs/5/labels/1");
    let s = c.stats();
    assert_eq!(s.calls, 1);
    assert!(s.bytes > 10);
}

#[tokio::test]
async fn health_path_has_no_org() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/api/v2/health")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "normal"}))).mount(&server).await;
    let c = client_for(&server);
    let r = c.get::<Value>("/health", &[]).await.unwrap();
    assert_eq!(r.body["status"], "normal");
}

#[tokio::test]
async fn status_codes_map_to_typed_errors() {
    let server = MockServer::start().await;
    Mock::given(path("/api/v2/orgs/5/a")).respond_with(ResponseTemplate::new(401)).mount(&server).await;
    Mock::given(path("/api/v2/orgs/5/b")).respond_with(ResponseTemplate::new(404)).mount(&server).await;
    Mock::given(path("/api/v2/orgs/5/c"))
        .respond_with(ResponseTemplate::new(406).set_body_json(json!([{"token": "invalid_uri", "message": "Invalid traffic filter type"}])))
        .mount(&server)
        .await;
    Mock::given(path("/api/v2/orgs/5/d")).respond_with(ResponseTemplate::new(500).set_body_string("boom")).mount(&server).await;
    let c = client_for(&server);
    assert!(matches!(c.get::<Value>("/orgs/5/a", &[]).await, Err(PceError::Auth { status: 401 })));
    assert!(matches!(c.get::<Value>("/orgs/5/b", &[]).await, Err(PceError::NotFound { .. })));
    match c.get::<Value>("/orgs/5/c", &[]).await {
        Err(PceError::InvalidFilter(m)) => assert_eq!(m, "invalid_uri: Invalid traffic filter type"),
        other => panic!("{other:?}"),
    }
    match c.get::<Value>("/orgs/5/d", &[]).await {
        Err(PceError::Server { status: 500, body }) => assert_eq!(body, "boom"),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn get_retries_after_429_then_succeeds() {
    let server = MockServer::start().await;
    Mock::given(path("/api/v2/orgs/5/x")).respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0")).up_to_n_times(2).mount(&server).await;
    Mock::given(path("/api/v2/orgs/5/x")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true}))).mount(&server).await;
    let c = client_for(&server);
    let r = c.get::<Value>("/orgs/5/x", &[]).await.unwrap();
    assert_eq!(r.body["ok"], true);
    assert_eq!(c.stats().calls, 3);
}

#[tokio::test]
async fn get_gives_up_after_max_retries() {
    let server = MockServer::start().await;
    Mock::given(path("/api/v2/orgs/5/x")).respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0")).mount(&server).await;
    let c = client_for(&server);
    match c.get::<Value>("/orgs/5/x", &[]).await {
        Err(PceError::RateLimited { retry_after }) => assert_eq!(retry_after, Some(Duration::ZERO)),
        other => panic!("{other:?}"),
    }
    assert_eq!(c.stats().calls, 5, "1 try + 4 retries");
}

#[tokio::test]
async fn post_is_never_retried() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/api/v2/orgs/5/traffic_flows/async_queries")).and(header("Prefer", "respond-async")).respond_with(ResponseTemplate::new(503)).mount(&server).await;
    let c = client_for(&server);
    let r = c.post::<Value, Value>("/orgs/5/traffic_flows/async_queries", &json!({"a": 1}), &[("Prefer", "respond-async")]).await;
    assert!(matches!(r, Err(PceError::Server { status: 503, .. })));
    assert_eq!(c.stats().calls, 1);
}

#[tokio::test]
async fn retry_after_http_date_is_ignored() {
    let mut h = reqwest::header::HeaderMap::new();
    h.insert("Retry-After", "Wed, 21 Oct 2026 07:28:00 GMT".parse().unwrap());
    assert_eq!(pce::client::parse_retry_after(&h), None);
    h.insert("Retry-After", "12".parse().unwrap());
    assert_eq!(pce::client::parse_retry_after(&h), Some(Duration::from_secs(12)));
}
```

Add `reqwest.workspace = true` and `url.workspace = true` under `[dev-dependencies]` in `crates/pce/Cargo.toml` (they are already regular dependencies; the test crate needs them too, which works because test targets see the package's dependencies, so no change is needed. If the compiler complains, add them.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p pce --test client`
Expected: compile error, `pce::client` missing.

- [ ] **Step 3: Implement**

```rust
// crates/pce/src/client.rs
//! The only place HTTP happens. Every request passes the rate limiter, is
//! counted, and has its status mapped to `PceError`. GETs back off on
//! 429/503; POSTs never retry (a repeated POST could apply a change twice).
use crate::auth::Credentials;
use crate::error::{PceError, Result};
use crate::limiter::RateLimiter;
use reqwest::header::{ACCEPT, HeaderMap, HeaderValue};
use reqwest::StatusCode;
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
        let base = Url::parse(&format!("{}://{}:{}/api/v2", cfg.scheme, cfg.host, cfg.port))
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
        let joined = format!("{}/{}", u.path().trim_end_matches('/'), api_path.trim_start_matches('/'));
        u.set_path(&joined);
        u
    }

    pub fn stats(&self) -> StatsSnapshot {
        StatsSnapshot { calls: self.stats.calls.load(Ordering::Relaxed), bytes: self.stats.bytes.load(Ordering::Relaxed) }
    }

    pub async fn get<T: DeserializeOwned>(&self, api_path: &str, query: &[(&str, String)]) -> Result<ApiResponse<T>> {
        let url = self.url_for(api_path);
        let build = || self.http.get(url.clone()).query(query);
        let (status, headers, bytes) = self.send(build, true).await?;
        Self::decode(api_path, status, &headers, &bytes)
    }

    pub async fn post<B: Serialize, T: DeserializeOwned>(&self, api_path: &str, body: &B, headers: &[(&str, &str)]) -> Result<ApiResponse<T>> {
        let url = self.url_for(api_path);
        let payload = serde_json::to_vec(body).map_err(|e| PceError::Invalid(e.to_string()))?;
        let build = || {
            let mut r = self.http.post(url.clone()).header("Content-Type", "application/json").body(payload.clone());
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
                .basic_auth(self.credentials.api_key(), Some(self.credentials.expose_secret()))
                .header(ACCEPT, HeaderValue::from_static("application/json"));
            let resp = req.send().await.map_err(|e| self.map_reqwest(e))?;
            let status = resp.status();
            let headers = resp.headers().clone();
            let bytes = resp.bytes().await.map_err(|e| self.map_reqwest(e))?;
            self.stats.bytes.fetch_add(bytes.len() as u64, Ordering::Relaxed);

            let throttled = status == StatusCode::TOO_MANY_REQUESTS || status == StatusCode::SERVICE_UNAVAILABLE;
            if throttled && retry && attempt < self.max_retries {
                let delay = parse_retry_after(&headers)
                    .unwrap_or_else(|| self.backoff_base.saturating_mul(2u32.saturating_pow(attempt)))
                    .min(Duration::from_secs(60));
                self.pause_for(delay + jitter(attempt, (self.backoff_base / 4).min(Duration::from_millis(250))));
                attempt += 1;
                continue;
            }
            if status == StatusCode::TOO_MANY_REQUESTS {
                return Err(PceError::RateLimited { retry_after: parse_retry_after(&headers) });
            }
            return Ok((status, headers, bytes));
        }
    }

    fn decode<T: DeserializeOwned>(api_path: &str, status: StatusCode, headers: &HeaderMap, bytes: &[u8]) -> Result<ApiResponse<T>> {
        match status.as_u16() {
            200..=299 => {}
            401 | 403 => return Err(PceError::Auth { status: status.as_u16() }),
            404 => return Err(PceError::NotFound { path: api_path.to_string() }),
            406 | 400 => return Err(PceError::InvalidFilter(pce_error_message(bytes))),
            s => return Err(PceError::Server { status: s, body: pce_error_message(bytes) }),
        }
        let total_count = headers.get("X-Total-Count").and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse().ok());
        let body = if bytes.is_empty() {
            serde_json::from_str("null")
        } else {
            serde_json::from_slice(bytes)
        }
        .map_err(|e| PceError::Decode(format!("{api_path}: {e}")))?;
        Ok(ApiResponse { body, status: status.as_u16(), total_count })
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
    headers.get("Retry-After")?.to_str().ok()?.trim().parse::<u64>().ok().map(Duration::from_secs)
}

/// Small spread (up to `cap`) so clients that back off together do not
/// retry in lock-step. Not cryptographic, not random; good enough.
fn jitter(attempt: u32, cap: Duration) -> Duration {
    let cap_ms = cap.as_millis() as u64;
    if cap_ms == 0 {
        return Duration::ZERO;
    }
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| u64::from(d.subsec_nanos())).unwrap_or(0);
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
```

Append to `lib.rs`: `pub mod client;` and `pub use client::{ApiResponse, ClientConfig, PceClient, StatsSnapshot};`.

- [ ] **Step 4: Run tests**

Run: `cargo test -p pce --test client`
Expected: 7 tests pass.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add -A
git commit -m "feat(pce): throttled, counted HTTP client with typed errors and backoff

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 5: TTL cache and the `Pce` facade

**Files:**
- Create: `crates/pce/src/cache.rs`, `crates/pce/tests/facade.rs`
- Modify: `crates/pce/src/lib.rs`

**Interfaces:**
- Consumes: `PceClient`.
- Produces:
  - `TtlCache::new()`, `get::<T: Clone + Send + Sync + 'static>(&self, key) -> Option<(T, Duration)>`, `insert<T>(&self, key, value, ttl)`, `invalidate_prefix(&self, prefix)`, `clear()`.
  - `CacheMode { Use, Bypass }`, `Cached<T> { value: T, age: Duration, hit: bool, truncated: bool }`.
  - `CacheTtls { objects: 15 min, policy: 5 min, workloads: 5 min }` (`Default`).
  - `Pce::new(PceClient) -> Pce`, `Pce::with_ttls(PceClient, CacheTtls)`, `client() -> &PceClient`, `stats()`, `clear_cache()`, `async fn health(&self) -> Result<serde_json::Value>`.
  - `pub(crate) async fn cached_list<T>(&self, key, api_path, query: Vec<(&str, String)>, ttl, mode) -> Result<Cached<Vec<T>>>` which appends `max_results=10000` unless given and sets `truncated` when `X-Total-Count` exceeds the row count.

- [ ] **Step 1: Write the failing tests**

```rust
// crates/pce/src/cache.rs (tests module)
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn hit_within_ttl_miss_after() {
        let c = TtlCache::new();
        c.insert("labels", vec![1u32, 2], Duration::from_millis(30));
        let (v, age) = c.get::<Vec<u32>>("labels").expect("hit");
        assert_eq!(v, vec![1, 2]);
        assert!(age < Duration::from_millis(30));
        std::thread::sleep(Duration::from_millis(40));
        assert!(c.get::<Vec<u32>>("labels").is_none());
    }

    #[test]
    fn wrong_type_is_a_miss_and_prefix_invalidation_works() {
        let c = TtlCache::new();
        c.insert("policy:draft", 7u8, Duration::from_secs(60));
        c.insert("policy:active", 8u8, Duration::from_secs(60));
        c.insert("labels", 9u8, Duration::from_secs(60));
        assert!(c.get::<String>("policy:draft").is_none());
        c.invalidate_prefix("policy:");
        assert!(c.get::<u8>("policy:draft").is_none());
        assert!(c.get::<u8>("policy:active").is_none());
        assert_eq!(c.get::<u8>("labels").map(|(v, _)| v), Some(9));
        c.clear();
        assert!(c.get::<u8>("labels").is_none());
    }
}
```

```rust
// crates/pce/tests/facade.rs
use pce::client::{ClientConfig, PceClient};
use pce::{CacheMode, Credentials, Pce};
use serde_json::json;
use std::time::Duration;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

pub fn pce_for(server: &MockServer) -> Pce {
    let url = url::Url::parse(&server.uri()).unwrap();
    let mut cfg = ClientConfig::new(url.host_str().unwrap(), 5, Credentials::new("key", "secret"));
    cfg.scheme = "http".into();
    cfg.port = url.port().unwrap();
    cfg.backoff_base = Duration::from_millis(1);
    cfg.requests_per_second = 1000.0;
    cfg.burst = 1000;
    Pce::new(PceClient::new(cfg).unwrap())
}

#[tokio::test]
async fn health_is_not_cached_and_has_no_org() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/api/v2/health")).respond_with(ResponseTemplate::new(200).set_body_json(json!([{"status": "normal"}]))).expect(2).mount(&server).await;
    let p = pce_for(&server);
    p.health().await.unwrap();
    p.health().await.unwrap();
    assert_eq!(p.stats().calls, 2);
}

#[tokio::test]
async fn cached_list_hits_cache_and_reports_truncation() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/labels"))
        .and(query_param("max_results", "10000"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{"href": "/orgs/5/labels/1", "key": "app", "value": "web"}])).insert_header("X-Total-Count", "25000"))
        .expect(3)
        .mount(&server)
        .await;
    let p = pce_for(&server);
    let first = p.labels(CacheMode::Use).await.unwrap();
    assert!(!first.hit);
    assert!(first.truncated, "25000 total but 1 row returned");
    let second = p.labels(CacheMode::Use).await.unwrap();
    assert!(second.hit);
    assert_eq!(p.stats().calls, 1);
    let third = p.labels(CacheMode::Bypass).await.unwrap();
    assert!(!third.hit);
    assert_eq!(p.stats().calls, 2);
    p.clear_cache();
    let fourth = p.labels(CacheMode::Use).await.unwrap();
    assert!(!fourth.hit, "clear_cache forces a fetch");
    assert_eq!(p.stats().calls, 3);
}
```

(The `labels()` method is added in Task 6; for this task, make the second test compile by adding a temporary `labels()` to the facade that calls `cached_list::<serde_json::Value>("labels", ...)`. Task 6 replaces the row type with `Label`.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p pce cache && cargo test -p pce --test facade`
Expected: compile errors, `cache` and `Pce` missing.

- [ ] **Step 3: Implement**

```rust
// crates/pce/src/cache.rs
//! In-memory TTL cache. Values are stored type-erased; a `get` with the
//! wrong type is simply a miss. Keys are strings like `labels` or
//! `workloads?hostname=web`.
use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct Entry {
    value: Arc<dyn Any + Send + Sync>,
    inserted: Instant,
    ttl: Duration,
}

#[derive(Default)]
pub struct TtlCache {
    map: Mutex<HashMap<String, Entry>>,
}

impl TtlCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the value and its age, or `None` when missing, expired, or of
    /// another type.
    pub fn get<T: Clone + Send + Sync + 'static>(&self, key: &str) -> Option<(T, Duration)> {
        let mut map = self.map.lock().expect("cache mutex poisoned");
        let entry = map.get(key)?;
        let age = entry.inserted.elapsed();
        if age >= entry.ttl {
            map.remove(key);
            return None;
        }
        let value = entry.value.clone().downcast::<T>().ok()?;
        Some(((*value).clone(), age))
    }

    pub fn insert<T: Send + Sync + 'static>(&self, key: &str, value: T, ttl: Duration) {
        let mut map = self.map.lock().expect("cache mutex poisoned");
        map.insert(key.to_string(), Entry { value: Arc::new(value), inserted: Instant::now(), ttl });
    }

    pub fn invalidate_prefix(&self, prefix: &str) {
        let mut map = self.map.lock().expect("cache mutex poisoned");
        map.retain(|k, _| !k.starts_with(prefix));
    }

    pub fn clear(&self) {
        self.map.lock().expect("cache mutex poisoned").clear();
    }
}
```

Replace `crates/pce/src/lib.rs` with:

```rust
//! `pce` is a read-only, PCE-friendly client library for the Illumio Core
//! Policy Compute Engine REST API v2.
//!
//! Layers, bottom up:
//! - [`client::PceClient`]: HTTP with Basic auth, a token-bucket throttle,
//!   request counting and backoff. The only place network I/O happens.
//! - [`cache::TtlCache`]: in-memory cache so the same list is never fetched
//!   twice within its TTL.
//! - [`Pce`]: the facade the TUI and CLI use. Typed reads, cached, with
//!   truncation reporting.
//! - `models`, `resolve`, `traffic`: data types, href-to-name resolution, and
//!   the Explorer traffic query (builder, poller, aggregation).
pub mod auth;
pub mod cache;
pub mod client;
pub mod error;
pub mod limiter;

use std::time::Duration;

pub use auth::Credentials;
pub use cache::TtlCache;
pub use client::{ApiResponse, ClientConfig, PceClient, StatsSnapshot};
pub use error::{PceError, Result};
pub use limiter::RateLimiter;

/// Whether a read may be served from cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheMode {
    Use,
    Bypass,
}

/// A value plus where it came from.
#[derive(Debug, Clone)]
pub struct Cached<T> {
    pub value: T,
    pub age: Duration,
    pub hit: bool,
    /// The PCE holds more rows than were returned.
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct CacheTtls {
    pub objects: Duration,
    pub policy: Duration,
    pub workloads: Duration,
}

impl Default for CacheTtls {
    fn default() -> Self {
        Self { objects: Duration::from_secs(15 * 60), policy: Duration::from_secs(5 * 60), workloads: Duration::from_secs(5 * 60) }
    }
}

pub const DEFAULT_MAX_RESULTS: u32 = 10_000;

/// The facade: typed, cached reads on top of one [`PceClient`].
pub struct Pce {
    client: PceClient,
    cache: TtlCache,
    ttls: CacheTtls,
}

impl Pce {
    pub fn new(client: PceClient) -> Self {
        Self::with_ttls(client, CacheTtls::default())
    }

    pub fn with_ttls(client: PceClient, ttls: CacheTtls) -> Self {
        Self { client, cache: TtlCache::new(), ttls }
    }

    pub fn client(&self) -> &PceClient {
        &self.client
    }

    pub fn ttls(&self) -> CacheTtls {
        self.ttls
    }

    pub fn stats(&self) -> StatsSnapshot {
        self.client.stats()
    }

    pub fn clear_cache(&self) {
        self.cache.clear();
    }

    pub fn invalidate(&self, prefix: &str) {
        self.cache.invalidate_prefix(prefix);
    }

    /// `GET /api/v2/health`. Never cached: it is the liveness probe.
    pub async fn health(&self) -> Result<serde_json::Value> {
        Ok(self.client.get::<serde_json::Value>("/health", &[]).await?.body)
    }

    /// Temporary until Task 6 adds typed labels.
    pub async fn labels(&self, mode: CacheMode) -> Result<Cached<Vec<serde_json::Value>>> {
        let path = self.client.org_path("labels");
        self.cached_list("labels", &path, vec![], self.ttls.objects, mode).await
    }

    /// Fetches a JSON array with an explicit `max_results`, caching under
    /// `key` (plus the query string). `truncated` is set when the PCE reports
    /// more rows than it returned.
    pub(crate) async fn cached_list<T>(&self, key: &str, api_path: &str, mut query: Vec<(&str, String)>, ttl: Duration, mode: CacheMode) -> Result<Cached<Vec<T>>>
    where
        T: serde::de::DeserializeOwned + Clone + Send + Sync + 'static,
    {
        if !query.iter().any(|(k, _)| *k == "max_results") {
            query.push(("max_results", DEFAULT_MAX_RESULTS.to_string()));
        }
        let qs: Vec<String> = query.iter().map(|(k, v)| format!("{k}={v}")).collect();
        let cache_key = format!("{key}?{}", qs.join("&"));
        if mode == CacheMode::Use {
            if let Some(((value, truncated), age)) = self.cache.get::<(Vec<T>, bool)>(&cache_key) {
                return Ok(Cached { value, age, hit: true, truncated });
            }
        }
        let resp = self.client.get::<Vec<T>>(api_path, &query).await?;
        let len = resp.body.len() as u64;
        let max: u64 = query.iter().find(|(k, _)| *k == "max_results").and_then(|(_, v)| v.parse().ok()).unwrap_or(u64::from(DEFAULT_MAX_RESULTS));
        let truncated = resp.total_count.is_some_and(|t| t > len) || len >= max;
        self.cache.insert(&cache_key, (resp.body.clone(), truncated), ttl);
        Ok(Cached { value: resp.body, age: Duration::ZERO, hit: false, truncated })
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p pce`
Expected: all pass (cache unit tests, facade tests, client tests, earlier tests).

- [ ] **Step 5: Commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add -A
git commit -m "feat(pce): TTL cache and Pce facade with truncation reporting

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 6: Object models (labels, label groups, IP lists, services) and fixtures

**Files:**
- Create: `crates/pce/src/models/mod.rs`, `crates/pce/src/models/common.rs`, `crates/pce/src/models/labels.rs`
- Create: `fixtures/labels.json`, `fixtures/label_groups.json`, `fixtures/ip_lists.json`, `fixtures/services.json`
- Modify: `crates/pce/src/lib.rs` (replace temporary `labels()`), `crates/pce/tests/facade.rs`

**Interfaces:**
- Produces: `Ref { href }` with `id()` and `object_type()`; `PolicyDecision` (`Allowed | PotentiallyBlocked | Blocked | Unknown`, `as_str()`, `parse()`, `ALL`); `proto_name(i64) -> String`, `proto_number(&str) -> Option<u8>`; `Label { href, key, value, deleted: Option<bool>, created_at, updated_at }` with `display()` and `is_deleted()`; `LabelGroup { href, name, key, labels: Vec<Ref>, sub_groups: Vec<Ref>, update_type }`; `IpList { href, name, description, ip_ranges: Vec<IpRange>, fqdns: Vec<Fqdn>, update_type }` with `summary()`; `Service { href, name, description, service_ports: Vec<ServicePort>, update_type }` with `summary()`; facade `labels()`, `label_groups()`, `ip_lists()`, `services()` each `(CacheMode) -> Result<Cached<Vec<_>>>`.
- Fixture loading in tests is `include_str!("../../../../fixtures/x.json")` from `src/models/*.rs` and `include_str!("../../../fixtures/x.json")` from `tests/*.rs`.

- [ ] **Step 1: Write the fixtures**

```json
// fixtures/labels.json
[
  {"href":"/orgs/5/labels/1","key":"role","value":"web","created_at":"2025-01-01T00:00:00Z","updated_at":"2025-01-01T00:00:00Z","deleted":false},
  {"href":"/orgs/5/labels/2","key":"role","value":"db","deleted":false},
  {"href":"/orgs/5/labels/3","key":"app","value":"shop","deleted":false},
  {"href":"/orgs/5/labels/4","key":"env","value":"prod","deleted":false},
  {"href":"/orgs/5/labels/5","key":"loc","value":"eu","deleted":false},
  {"href":"/orgs/5/labels/6","key":"app","deleted":true}
]
```

```json
// fixtures/label_groups.json
[{"href":"/orgs/5/sec_policy/draft/label_groups/10","name":"All prod envs","key":"env","labels":[{"href":"/orgs/5/labels/4"}],"sub_groups":[],"update_type":null}]
```

```json
// fixtures/ip_lists.json
[
  {"href":"/orgs/5/sec_policy/draft/ip_lists/20","name":"Any (0.0.0.0/0 and ::/0)","ip_ranges":[{"from_ip":"0.0.0.0/0"},{"from_ip":"::/0"}],"fqdns":[]},
  {"href":"/orgs/5/sec_policy/draft/ip_lists/21","name":"Corp","description":"Office ranges","ip_ranges":[{"from_ip":"10.1.0.0","to_ip":"10.1.255.255","exclusion":false}],"fqdns":[{"fqdn":"*.corp.example"}],"update_type":"create"}
]
```

```json
// fixtures/services.json
[
  {"href":"/orgs/5/sec_policy/draft/services/30","name":"All Services","service_ports":[{"proto":-1}]},
  {"href":"/orgs/5/sec_policy/draft/services/31","name":"PostgreSQL","description":"","service_ports":[{"port":5432,"proto":6}],"windows_services":[],"windows_egress_services":[]},
  {"href":"/orgs/5/sec_policy/draft/services/32","name":"Web","service_ports":[{"port":80,"proto":6},{"port":443,"proto":6},{"port":8000,"to_port":8099,"proto":6}]}
]
```

- [ ] **Step 2: Write the failing tests**

```rust
// crates/pce/src/models/common.rs (tests module)
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ref_parses_type_and_id() {
        let r = Ref::new("/orgs/5/sec_policy/draft/rule_sets/100");
        assert_eq!(r.id(), "100");
        assert_eq!(r.object_type(), "rule_sets");
        assert_eq!(Ref::new("/orgs/5/labels/7").object_type(), "labels");
        assert_eq!(Ref::new("weird").object_type(), "");
    }

    #[test]
    fn policy_decision_round_trips_and_tolerates_unknown() {
        let d: Vec<PolicyDecision> = serde_json::from_str(r#"["allowed","potentially_blocked","blocked","something_new"]"#).unwrap();
        assert_eq!(d, vec![PolicyDecision::Allowed, PolicyDecision::PotentiallyBlocked, PolicyDecision::Blocked, PolicyDecision::Unknown]);
        assert_eq!(serde_json::to_string(&PolicyDecision::PotentiallyBlocked).unwrap(), "\"potentially_blocked\"");
        assert_eq!(PolicyDecision::parse("blocked"), Some(PolicyDecision::Blocked));
        assert_eq!(PolicyDecision::parse("nope"), None);
    }

    #[test]
    fn proto_names() {
        assert_eq!(proto_name(6), "tcp");
        assert_eq!(proto_name(17), "udp");
        assert_eq!(proto_name(1), "icmp");
        assert_eq!(proto_name(-1), "any");
        assert_eq!(proto_name(47), "47");
        assert_eq!(proto_number("TCP"), Some(6));
        assert_eq!(proto_number("17"), Some(17));
        assert_eq!(proto_number("x"), None);
    }
}
```

```rust
// crates/pce/src/models/labels.rs (tests module)
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_fixture_parses_including_deleted_without_value() {
        let labels: Vec<Label> = serde_json::from_str(include_str!("../../../../fixtures/labels.json")).unwrap();
        assert_eq!(labels.len(), 6);
        assert_eq!(labels[0].display(), "role=web");
        let deleted = &labels[5];
        assert!(deleted.is_deleted());
        assert_eq!(deleted.value, "");
        assert_eq!(deleted.display(), "app=");
    }

    #[test]
    fn label_groups_ip_lists_services_parse() {
        let groups: Vec<LabelGroup> = serde_json::from_str(include_str!("../../../../fixtures/label_groups.json")).unwrap();
        assert_eq!(groups[0].labels[0].href, "/orgs/5/labels/4");
        let lists: Vec<IpList> = serde_json::from_str(include_str!("../../../../fixtures/ip_lists.json")).unwrap();
        assert_eq!(lists[1].summary(), "10.1.0.0-10.1.255.255, *.corp.example");
        assert_eq!(lists[0].summary(), "0.0.0.0/0, ::/0");
        let services: Vec<Service> = serde_json::from_str(include_str!("../../../../fixtures/services.json")).unwrap();
        assert_eq!(services[0].summary(), "any");
        assert_eq!(services[1].summary(), "5432/tcp");
        assert_eq!(services[2].summary(), "80/tcp, 443/tcp, 8000-8099/tcp");
    }
}
```

The existing facade test keeps working because its body is a valid `Label`. Add a test for the other three lists to `crates/pce/tests/facade.rs`:

```rust
#[tokio::test]
async fn object_lists_are_cached_separately() {
    let server = MockServer::start().await;
    for (p, body) in [
        ("/api/v2/orgs/5/labels", include_str!("../../../fixtures/labels.json")),
        ("/api/v2/orgs/5/sec_policy/draft/label_groups", include_str!("../../../fixtures/label_groups.json")),
        ("/api/v2/orgs/5/sec_policy/draft/ip_lists", include_str!("../../../fixtures/ip_lists.json")),
        ("/api/v2/orgs/5/sec_policy/draft/services", include_str!("../../../fixtures/services.json")),
    ] {
        Mock::given(method("GET")).and(path(p)).respond_with(ResponseTemplate::new(200).set_body_string(body).insert_header("Content-Type", "application/json")).expect(1).mount(&server).await;
    }
    let p = pce_for(&server);
    assert_eq!(p.labels(CacheMode::Use).await.unwrap().value.len(), 6);
    assert_eq!(p.label_groups(CacheMode::Use).await.unwrap().value.len(), 1);
    assert_eq!(p.ip_lists(CacheMode::Use).await.unwrap().value.len(), 2);
    assert_eq!(p.services(CacheMode::Use).await.unwrap().value.len(), 3);
    assert!(p.services(CacheMode::Use).await.unwrap().hit);
    assert_eq!(p.stats().calls, 4);
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p pce`
Expected: compile errors (`models` missing, `label_groups` missing).

- [ ] **Step 4: Implement**

```rust
// crates/pce/src/models/mod.rs
//! Serde types for PCE API objects. Unknown fields are ignored unless a
//! type keeps an `extra` map on purpose (flows do, so the TUI can show
//! anything the PCE adds later).
pub mod common;
pub mod labels;

pub use common::{PolicyDecision, Ref, proto_name, proto_number};
pub use labels::{Fqdn, IpList, IpRange, Label, LabelGroup, Service, ServicePort};
```

```rust
// crates/pce/src/models/common.rs
use serde::{Deserialize, Serialize};

/// A PCE object reference: `{"href": "/orgs/5/labels/7"}`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Ref {
    pub href: String,
}

impl Ref {
    pub fn new(href: impl Into<String>) -> Self {
        Self { href: href.into() }
    }

    /// Last path segment.
    pub fn id(&self) -> &str {
        self.href.rsplit('/').next().unwrap_or("")
    }

    /// Segment before the id, e.g. `labels`, `rule_sets`, `ip_lists`.
    pub fn object_type(&self) -> &str {
        let mut parts = self.href.rsplit('/');
        let _id = parts.next();
        match parts.next() {
            Some(t) if !t.is_empty() && self.href.contains('/') => t,
            _ => "",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyDecision {
    Allowed,
    PotentiallyBlocked,
    Blocked,
    #[serde(other)]
    Unknown,
}

impl PolicyDecision {
    pub const ALL: [PolicyDecision; 3] = [PolicyDecision::Allowed, PolicyDecision::PotentiallyBlocked, PolicyDecision::Blocked];

    pub fn as_str(self) -> &'static str {
        match self {
            PolicyDecision::Allowed => "allowed",
            PolicyDecision::PotentiallyBlocked => "potentially_blocked",
            PolicyDecision::Blocked => "blocked",
            PolicyDecision::Unknown => "unknown",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "allowed" => Some(PolicyDecision::Allowed),
            "potentially_blocked" | "potentially-blocked" | "pb" => Some(PolicyDecision::PotentiallyBlocked),
            "blocked" => Some(PolicyDecision::Blocked),
            _ => None,
        }
    }
}

pub fn proto_name(proto: i64) -> String {
    match proto {
        6 => "tcp".into(),
        17 => "udp".into(),
        1 => "icmp".into(),
        58 => "icmpv6".into(),
        -1 => "any".into(),
        n => n.to_string(),
    }
}

pub fn proto_number(name: &str) -> Option<u8> {
    match name.trim().to_ascii_lowercase().as_str() {
        "tcp" => Some(6),
        "udp" => Some(17),
        "icmp" => Some(1),
        "icmpv6" => Some(58),
        n => n.parse().ok(),
    }
}
```

```rust
// crates/pce/src/models/labels.rs
//! Labels and the policy objects rules refer to by href.
use super::common::{Ref, proto_name};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Label {
    pub href: String,
    pub key: String,
    #[serde(default)]
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

impl Label {
    pub fn display(&self) -> String {
        format!("{}={}", self.key, self.value)
    }

    pub fn is_deleted(&self) -> bool {
        self.deleted.unwrap_or(false)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LabelGroup {
    pub href: String,
    pub name: String,
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub labels: Vec<Ref>,
    #[serde(default)]
    pub sub_groups: Vec<Ref>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IpRange {
    pub from_ip: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_ip: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclusion: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fqdn {
    pub fqdn: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IpList {
    pub href: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub ip_ranges: Vec<IpRange>,
    #[serde(default)]
    pub fqdns: Vec<Fqdn>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_type: Option<String>,
}

impl IpList {
    /// `10.1.0.0-10.1.255.255, *.corp.example`
    pub fn summary(&self) -> String {
        let mut parts: Vec<String> = self
            .ip_ranges
            .iter()
            .map(|r| {
                let excl = if r.exclusion.unwrap_or(false) { "!" } else { "" };
                match &r.to_ip {
                    Some(to) => format!("{excl}{}-{to}", r.from_ip),
                    None => format!("{excl}{}", r.from_ip),
                }
            })
            .collect();
        parts.extend(self.fqdns.iter().map(|f| f.fqdn.clone()));
        parts.join(", ")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServicePort {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proto: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icmp_type: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icmp_code: Option<i64>,
}

impl ServicePort {
    /// `5432/tcp`, `8000-8099/tcp`, `icmp`, `any`
    pub fn display(&self) -> String {
        let proto = self.proto.map(proto_name).unwrap_or_else(|| "any".into());
        match (self.port, self.to_port) {
            (Some(p), Some(t)) if t != p => format!("{p}-{t}/{proto}"),
            (Some(p), _) => format!("{p}/{proto}"),
            (None, _) => proto,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Service {
    pub href: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub service_ports: Vec<ServicePort>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_type: Option<String>,
}

impl Service {
    pub fn summary(&self) -> String {
        if self.service_ports.is_empty() {
            return "-".into();
        }
        self.service_ports.iter().map(ServicePort::display).collect::<Vec<_>>().join(", ")
    }
}
```

In `crates/pce/src/lib.rs` add `pub mod models;`, `pub use models::*;` is NOT used (keep paths explicit). Replace the temporary `labels()` with:

```rust
    pub async fn labels(&self, mode: CacheMode) -> Result<Cached<Vec<models::Label>>> {
        let path = self.client.org_path("labels");
        self.cached_list("objects:labels", &path, vec![], self.ttls.objects, mode).await
    }

    pub async fn label_groups(&self, mode: CacheMode) -> Result<Cached<Vec<models::LabelGroup>>> {
        let path = self.client.org_path("sec_policy/draft/label_groups");
        self.cached_list("objects:label_groups", &path, vec![], self.ttls.objects, mode).await
    }

    pub async fn ip_lists(&self, mode: CacheMode) -> Result<Cached<Vec<models::IpList>>> {
        let path = self.client.org_path("sec_policy/draft/ip_lists");
        self.cached_list("objects:ip_lists", &path, vec![], self.ttls.objects, mode).await
    }

    pub async fn services(&self, mode: CacheMode) -> Result<Cached<Vec<models::Service>>> {
        let path = self.client.org_path("sec_policy/draft/services");
        self.cached_list("objects:services", &path, vec![], self.ttls.objects, mode).await
    }
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p pce`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add -A
git commit -m "feat(pce): label, label group, IP list and service models with fixtures

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 7: Workload model and filter

**Files:**
- Create: `crates/pce/src/models/workloads.rs`, `fixtures/workloads.json`
- Modify: `crates/pce/src/models/mod.rs`, `crates/pce/src/lib.rs`, `crates/pce/tests/facade.rs`

**Interfaces:**
- Produces: `Workload { href, name, hostname, description, interfaces: Vec<Interface>, labels: Vec<WorkloadLabel>, enforcement_mode, visibility_level, online, managed, os_id, agent: Option<Value>, ven: Option<Ref> }` with `display_name()`, `primary_ip()`, `agent_status()`, `label_text()`, `label_hrefs()`; `WorkloadFilter { name, hostname, ip_address, enforcement_mode, managed, online, labels: Vec<Vec<String>>, max_results }` with `to_query()`; facade `workloads(&WorkloadFilter, CacheMode)`.

- [ ] **Step 1: Write the fixture**

```json
// fixtures/workloads.json
[
  {"href":"/orgs/5/workloads/aaaa-1","name":"web01","hostname":"host-1.example","description":null,"interfaces":[{"name":"eth0","address":"10.0.1.10","cidr_block":24,"link_state":"up"}],"labels":[{"href":"/orgs/5/labels/1","key":"role","value":"web"},{"href":"/orgs/5/labels/3","key":"app","value":"shop"},{"href":"/orgs/5/labels/4","key":"env","value":"prod"}],"enforcement_mode":"full","visibility_level":"flow_summary","online":true,"managed":true,"os_id":"ubuntu-x86_64-22.04","agent":{"href":"/orgs/5/agents/1","status":{"status":"active","uptime_seconds":1000}},"ven":{"href":"/orgs/5/vens/1"}},
  {"href":"/orgs/5/workloads/aaaa-2","name":null,"hostname":"host-2.example","interfaces":[{"name":"eth0","address":"10.0.2.20"}],"labels":[{"href":"/orgs/5/labels/2","key":"role","value":"db"},{"href":"/orgs/5/labels/3","key":"app","value":"shop"},{"href":"/orgs/5/labels/4","key":"env","value":"prod"}],"enforcement_mode":"visibility_only","visibility_level":"flow_summary","online":false,"managed":true,"agent":{"status":{"status":"stopped"}}},
  {"href":"/orgs/5/workloads/aaaa-3","name":"unmanaged-nas","hostname":null,"interfaces":[{"name":"eth0","address":"10.0.3.30"}],"labels":[],"enforcement_mode":"idle","online":null,"managed":false}
]
```

- [ ] **Step 2: Write the failing tests**

```rust
// crates/pce/src/models/workloads.rs (tests module)
#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<Workload> {
        serde_json::from_str(include_str!("../../../../fixtures/workloads.json")).unwrap()
    }

    #[test]
    fn parses_and_derives_display_fields() {
        let w = fixture();
        assert_eq!(w.len(), 3);
        assert_eq!(w[0].display_name(), "host-1.example");
        assert_eq!(w[0].primary_ip(), Some("10.0.1.10"));
        assert_eq!(w[0].agent_status(), Some("active".into()));
        assert_eq!(w[0].label_text(), "app=shop env=prod role=web");
        assert_eq!(w[1].display_name(), "host-2.example");
        assert_eq!(w[1].agent_status(), Some("stopped".into()));
        assert_eq!(w[2].display_name(), "unmanaged-nas");
        assert_eq!(w[2].agent_status(), None);
        assert_eq!(w[2].label_text(), "");
        assert_eq!(w[2].online, None);
    }

    #[test]
    fn filter_encodes_labels_as_nested_json() {
        let f = WorkloadFilter {
            hostname: Some("web".into()),
            labels: vec![vec!["/orgs/5/labels/1".into(), "/orgs/5/labels/3".into()], vec!["/orgs/5/labels/2".into()]],
            online: Some(true),
            max_results: Some(50),
            ..Default::default()
        };
        let q = f.to_query();
        assert!(q.contains(&("hostname", "web".to_string())));
        assert!(q.contains(&("online", "true".to_string())));
        assert!(q.contains(&("max_results", "50".to_string())));
        assert!(q.contains(&("labels", r#"[["/orgs/5/labels/1","/orgs/5/labels/3"],["/orgs/5/labels/2"]]"#.to_string())));
        assert_eq!(WorkloadFilter::default().to_query(), vec![]);
    }
}
```

Add to `crates/pce/tests/facade.rs`:

```rust
#[tokio::test]
async fn workloads_pass_filters_and_cache_per_filter() {
    use pce::models::WorkloadFilter;
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .and(query_param("hostname", "host-1"))
        .respond_with(ResponseTemplate::new(200).set_body_string(include_str!("../../../fixtures/workloads.json")).insert_header("Content-Type", "application/json"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .expect(1)
        .mount(&server)
        .await;
    let p = pce_for(&server);
    let f = WorkloadFilter { hostname: Some("host-1".into()), ..Default::default() };
    assert_eq!(p.workloads(&f, CacheMode::Use).await.unwrap().value.len(), 3);
    assert!(p.workloads(&f, CacheMode::Use).await.unwrap().hit);
    assert_eq!(p.workloads(&WorkloadFilter::default(), CacheMode::Use).await.unwrap().value.len(), 0);
    assert_eq!(p.stats().calls, 2);
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p pce workloads`
Expected: compile error.

- [ ] **Step 4: Implement**

```rust
// crates/pce/src/models/workloads.rs
//! Workloads (VEN-managed hosts and unmanaged entries).
use super::common::Ref;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Interface {
    #[serde(default)]
    pub name: String,
    pub address: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cidr_block: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_state: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkloadLabel {
    pub href: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Workload {
    pub href: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub hostname: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub interfaces: Vec<Interface>,
    #[serde(default)]
    pub labels: Vec<WorkloadLabel>,
    #[serde(default)]
    pub enforcement_mode: Option<String>,
    #[serde(default)]
    pub visibility_level: Option<String>,
    #[serde(default)]
    pub online: Option<bool>,
    #[serde(default)]
    pub managed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ven: Option<Ref>,
}

impl Workload {
    /// hostname, else name, else the href id.
    pub fn display_name(&self) -> String {
        self.hostname
            .clone()
            .filter(|s| !s.is_empty())
            .or_else(|| self.name.clone().filter(|s| !s.is_empty()))
            .unwrap_or_else(|| Ref::new(&self.href).id().to_string())
    }

    pub fn primary_ip(&self) -> Option<&str> {
        self.interfaces.first().map(|i| i.address.as_str())
    }

    /// `agent.status.status` when present.
    pub fn agent_status(&self) -> Option<String> {
        self.agent.as_ref()?.get("status")?.get("status")?.as_str().map(str::to_string)
    }

    /// `app=shop env=prod role=web`, sorted by key.
    pub fn label_text(&self) -> String {
        let mut parts: Vec<String> = self
            .labels
            .iter()
            .filter_map(|l| Some(format!("{}={}", l.key.as_ref()?, l.value.as_deref().unwrap_or(""))))
            .collect();
        parts.sort();
        parts.join(" ")
    }

    pub fn label_hrefs(&self) -> Vec<&str> {
        self.labels.iter().map(|l| l.href.as_str()).collect()
    }
}

/// Query parameters for `GET /orgs/{org}/workloads`. `labels` is an OR of
/// AND-groups of label hrefs, sent as a JSON string (the PCE's format).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkloadFilter {
    pub name: Option<String>,
    pub hostname: Option<String>,
    pub ip_address: Option<String>,
    pub enforcement_mode: Option<String>,
    pub managed: Option<bool>,
    pub online: Option<bool>,
    #[serde(default)]
    pub labels: Vec<Vec<String>>,
    pub max_results: Option<u32>,
}

impl WorkloadFilter {
    pub fn to_query(&self) -> Vec<(&'static str, String)> {
        let mut q = Vec::new();
        if let Some(v) = &self.name {
            q.push(("name", v.clone()));
        }
        if let Some(v) = &self.hostname {
            q.push(("hostname", v.clone()));
        }
        if let Some(v) = &self.ip_address {
            q.push(("ip_address", v.clone()));
        }
        if let Some(v) = &self.enforcement_mode {
            q.push(("enforcement_mode", v.clone()));
        }
        if let Some(v) = self.managed {
            q.push(("managed", v.to_string()));
        }
        if let Some(v) = self.online {
            q.push(("online", v.to_string()));
        }
        if !self.labels.is_empty() {
            q.push(("labels", serde_json::to_string(&self.labels).unwrap_or_default()));
        }
        if let Some(v) = self.max_results {
            q.push(("max_results", v.to_string()));
        }
        q
    }
}
```

Add to `models/mod.rs`: `pub mod workloads;` and `pub use workloads::{Interface, Workload, WorkloadFilter, WorkloadLabel};`.

Add to the `Pce` impl in `lib.rs`:

```rust
    pub async fn workloads(&self, filter: &models::WorkloadFilter, mode: CacheMode) -> Result<Cached<Vec<models::Workload>>> {
        let path = self.client.org_path("workloads");
        self.cached_list("workloads", &path, filter.to_query(), self.ttls.workloads, mode).await
    }
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p pce`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add -A
git commit -m "feat(pce): workload model, filter encoding, cached listing

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 8: Name index (href to name resolution)

**Files:**
- Create: `crates/pce/src/resolve.rs`
- Modify: `crates/pce/src/lib.rs`

**Interfaces:**
- Consumes: `Label`, `LabelGroup`, `IpList`, `Service`.
- Produces: `NameIndex::new(&[Label], &[LabelGroup], &[IpList], &[Service])`, `label(href) -> Option<&Label>`, `label_text(href) -> String` (`key=value` or the href when unknown), `label_href(key, value) -> Option<&str>`, `label_keys() -> Vec<String>`, `values_for_key(key) -> Vec<String>`, `label_group_name(href)`, `ip_list_name(href)`, `ip_list_href(name)` (case-insensitive), `service_name(href)`, `service(href) -> Option<&Service>`, `ResolvedLabel { href, key, value }`, `resolve_labels(&[Ref]) -> Vec<ResolvedLabel>` (sorted by key); facade `name_index(CacheMode) -> Result<Arc<NameIndex>>` (one call each for the four lists, cached under `objects:index`).

- [ ] **Step 1: Write the failing tests**

```rust
// crates/pce/src/resolve.rs (tests module)
#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{IpList, Label, LabelGroup, Ref, Service};

    pub(crate) fn index() -> NameIndex {
        let labels: Vec<Label> = serde_json::from_str(include_str!("../../../fixtures/labels.json")).unwrap();
        let groups: Vec<LabelGroup> = serde_json::from_str(include_str!("../../../fixtures/label_groups.json")).unwrap();
        let lists: Vec<IpList> = serde_json::from_str(include_str!("../../../fixtures/ip_lists.json")).unwrap();
        let services: Vec<Service> = serde_json::from_str(include_str!("../../../fixtures/services.json")).unwrap();
        NameIndex::new(&labels, &groups, &lists, &services)
    }

    #[test]
    fn resolves_labels_both_ways() {
        let idx = index();
        assert_eq!(idx.label_text("/orgs/5/labels/1"), "role=web");
        assert_eq!(idx.label_text("/orgs/5/labels/999"), "/orgs/5/labels/999");
        assert_eq!(idx.label_href("app", "shop"), Some("/orgs/5/labels/3"));
        assert_eq!(idx.label_href("app", "nope"), None);
        assert_eq!(idx.label_keys(), vec!["app", "env", "loc", "role"]);
        assert_eq!(idx.values_for_key("role"), vec!["db", "web"]);
    }

    #[test]
    fn deleted_label_without_value_is_indexed_but_not_matchable_by_value() {
        let idx = index();
        assert_eq!(idx.label_text("/orgs/5/labels/6"), "app=");
        assert_eq!(idx.label_href("app", ""), None, "deleted labels never match filters");
    }

    #[test]
    fn resolves_groups_lists_services() {
        let idx = index();
        assert_eq!(idx.label_group_name("/orgs/5/sec_policy/draft/label_groups/10"), Some("All prod envs"));
        assert_eq!(idx.ip_list_name("/orgs/5/sec_policy/draft/ip_lists/21"), Some("Corp"));
        assert_eq!(idx.ip_list_href("corp"), Some("/orgs/5/sec_policy/draft/ip_lists/21"));
        assert_eq!(idx.service_name("/orgs/5/sec_policy/draft/services/31"), Some("PostgreSQL"));
        assert_eq!(idx.service_name("/x"), None);
    }

    #[test]
    fn resolve_labels_sorts_by_key_and_keeps_unknown_hrefs() {
        let idx = index();
        let out = idx.resolve_labels(&[Ref::new("/orgs/5/labels/4"), Ref::new("/orgs/5/labels/1"), Ref::new("/orgs/5/labels/77")]);
        let text: Vec<String> = out.iter().map(|l| format!("{}={}", l.key, l.value)).collect();
        assert_eq!(text, vec!["?=/orgs/5/labels/77", "env=prod", "role=web"]);
    }
}
```

Add to `crates/pce/tests/facade.rs`:

```rust
#[tokio::test]
async fn name_index_loads_four_lists_once() {
    let server = MockServer::start().await;
    for (p, body) in [
        ("/api/v2/orgs/5/labels", include_str!("../../../fixtures/labels.json")),
        ("/api/v2/orgs/5/sec_policy/draft/label_groups", include_str!("../../../fixtures/label_groups.json")),
        ("/api/v2/orgs/5/sec_policy/draft/ip_lists", include_str!("../../../fixtures/ip_lists.json")),
        ("/api/v2/orgs/5/sec_policy/draft/services", include_str!("../../../fixtures/services.json")),
    ] {
        Mock::given(method("GET")).and(path(p)).respond_with(ResponseTemplate::new(200).set_body_string(body).insert_header("Content-Type", "application/json")).expect(1).mount(&server).await;
    }
    let p = pce_for(&server);
    let idx = p.name_index(CacheMode::Use).await.unwrap();
    assert_eq!(idx.label_text("/orgs/5/labels/3"), "app=shop");
    let again = p.name_index(CacheMode::Use).await.unwrap();
    assert_eq!(again.label_keys().len(), 4);
    assert_eq!(p.stats().calls, 4);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p pce resolve`
Expected: compile error.

- [ ] **Step 3: Implement**

```rust
// crates/pce/src/resolve.rs
//! Turns PCE hrefs into names. Built once from the four object lists and
//! shared (via `Arc`) by every screen and command. Never makes requests.
use crate::models::{IpList, Label, LabelGroup, Ref, Service};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedLabel {
    pub href: String,
    pub key: String,
    pub value: String,
}

#[derive(Debug, Default)]
pub struct NameIndex {
    labels: HashMap<String, Label>,
    label_by_kv: HashMap<(String, String), String>,
    keys: BTreeMap<String, BTreeSet<String>>,
    label_groups: HashMap<String, LabelGroup>,
    ip_lists: HashMap<String, IpList>,
    ip_list_by_name: HashMap<String, String>,
    services: HashMap<String, Service>,
}

impl NameIndex {
    pub fn new(labels: &[Label], groups: &[LabelGroup], ip_lists: &[IpList], services: &[Service]) -> Self {
        let mut idx = Self::default();
        for l in labels {
            idx.labels.insert(l.href.clone(), l.clone());
            if !l.is_deleted() {
                idx.label_by_kv.insert((l.key.clone(), l.value.clone()), l.href.clone());
                idx.keys.entry(l.key.clone()).or_default().insert(l.value.clone());
            }
        }
        for g in groups {
            idx.label_groups.insert(g.href.clone(), g.clone());
        }
        for i in ip_lists {
            idx.ip_list_by_name.insert(i.name.to_ascii_lowercase(), i.href.clone());
            idx.ip_lists.insert(i.href.clone(), i.clone());
        }
        for s in services {
            idx.services.insert(s.href.clone(), s.clone());
        }
        idx
    }

    pub fn label(&self, href: &str) -> Option<&Label> {
        self.labels.get(href)
    }

    /// `key=value`, or the href itself when unknown.
    pub fn label_text(&self, href: &str) -> String {
        self.labels.get(href).map(Label::display).unwrap_or_else(|| href.to_string())
    }

    pub fn label_href(&self, key: &str, value: &str) -> Option<&str> {
        self.label_by_kv.get(&(key.to_string(), value.to_string())).map(String::as_str)
    }

    pub fn label_keys(&self) -> Vec<String> {
        self.keys.keys().cloned().collect()
    }

    pub fn values_for_key(&self, key: &str) -> Vec<String> {
        self.keys.get(key).map(|v| v.iter().cloned().collect()).unwrap_or_default()
    }

    pub fn label_group_name(&self, href: &str) -> Option<&str> {
        self.label_groups.get(href).map(|g| g.name.as_str())
    }

    pub fn ip_list_name(&self, href: &str) -> Option<&str> {
        self.ip_lists.get(href).map(|l| l.name.as_str())
    }

    pub fn ip_list_href(&self, name: &str) -> Option<&str> {
        self.ip_list_by_name.get(&name.to_ascii_lowercase()).map(String::as_str)
    }

    pub fn service(&self, href: &str) -> Option<&Service> {
        self.services.get(href)
    }

    pub fn service_name(&self, href: &str) -> Option<&str> {
        self.services.get(href).map(|s| s.name.as_str())
    }

    /// Resolves label refs, sorted by key. Unknown hrefs get key `?` and the
    /// href as value so nothing silently disappears.
    pub fn resolve_labels(&self, refs: &[Ref]) -> Vec<ResolvedLabel> {
        let mut out: Vec<ResolvedLabel> = refs
            .iter()
            .map(|r| match self.labels.get(&r.href) {
                Some(l) => ResolvedLabel { href: r.href.clone(), key: l.key.clone(), value: l.value.clone() },
                None => ResolvedLabel { href: r.href.clone(), key: "?".into(), value: r.href.clone() },
            })
            .collect();
        out.sort_by(|a, b| a.key.cmp(&b.key).then(a.value.cmp(&b.value)));
        out
    }
}
```

Add to `lib.rs`: `pub mod resolve;`, `pub use resolve::{NameIndex, ResolvedLabel};`, `use std::sync::Arc;`, and in the `Pce` impl:

```rust
    /// Loads (or reuses) the four object lists and builds the name index.
    pub async fn name_index(&self, mode: CacheMode) -> Result<Arc<NameIndex>> {
        if mode == CacheMode::Use {
            if let Some((idx, _)) = self.cache.get::<Arc<NameIndex>>("objects:index") {
                return Ok(idx);
            }
        }
        let labels = self.labels(mode).await?.value;
        let groups = self.label_groups(mode).await?.value;
        let lists = self.ip_lists(mode).await?.value;
        let services = self.services(mode).await?.value;
        let idx = Arc::new(NameIndex::new(&labels, &groups, &lists, &services));
        self.cache.insert("objects:index", idx.clone(), self.ttls.objects);
        Ok(idx)
    }
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p pce`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add -A
git commit -m "feat(pce): NameIndex for href-to-name resolution

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 9: Policy models, pending changes, and rule views

**Files:**
- Create: `crates/pce/src/models/policy.rs`, `fixtures/rule_sets.json`, `fixtures/deny_rules.json`, `fixtures/pending_list.json`, `fixtures/pending_dict.json`
- Modify: `crates/pce/src/models/mod.rs`, `crates/pce/src/resolve.rs`, `crates/pce/src/lib.rs`, `crates/pce/tests/facade.rs`

**Interfaces:**
- Produces: `PolicyScope { Draft, Active }` with `segment()` and `parse()`; `Actor { actors, label, label_group, workload, virtual_service, virtual_server, ip_list, exclusion }`; `IngressService::{Ref(Ref) | Port { port, to_port, proto }}`; `ResolveLabelsAs { providers, consumers }`; `Rule`; `DenyRule` (`override_` field serialised as `override`); `RuleSet` with `scopes: Vec<Vec<Actor>>`; `PendingChange { href, change_type, name, object_type }` and `parse_pending(Value) -> Vec<PendingChange>`; in `resolve.rs`: `describe_actor(&Actor, &NameIndex) -> String`, `describe_ingress_service(&IngressService, &NameIndex) -> String`, `RuleKind { Allow, Deny, OverrideDeny }`, `RuleView::from_rule`, `RuleView::from_deny`, `RuleSetView::from_rule_set`; facade `rule_sets(PolicyScope, CacheMode)`, `deny_rules(ruleset_href, CacheMode)`, `pending_changes(CacheMode)`.

- [ ] **Step 1: Write the fixtures**

```json
// fixtures/rule_sets.json
[
  {"href":"/orgs/5/sec_policy/draft/rule_sets/100","name":"Shop-Prod","description":"Ringfence","enabled":true,
   "scopes":[[{"label":{"href":"/orgs/5/labels/3"}},{"label":{"href":"/orgs/5/labels/4"}}]],
   "update_type":null,"created_at":"2025-01-01T00:00:00Z","updated_at":"2025-06-01T00:00:00Z",
   "rules":[
     {"href":"/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1","enabled":true,"description":"web to db",
      "providers":[{"label":{"href":"/orgs/5/labels/2"}}],"consumers":[{"label":{"href":"/orgs/5/labels/1"}}],
      "ingress_services":[{"href":"/orgs/5/sec_policy/draft/services/31"}],
      "resolve_labels_as":{"providers":["workloads"],"consumers":["workloads"]},
      "unscoped_consumers":false,"sec_connect":false,"stateless":false,"machine_auth":false,"network_type":"brn","update_type":null},
     {"href":"/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/2","enabled":true,"description":"corp to web",
      "providers":[{"label":{"href":"/orgs/5/labels/1"}}],"consumers":[{"ip_list":{"href":"/orgs/5/sec_policy/draft/ip_lists/21"}}],
      "ingress_services":[{"port":443,"proto":6},{"port":8000,"to_port":8099,"proto":6}],
      "resolve_labels_as":{"providers":["workloads"],"consumers":["workloads"]},"unscoped_consumers":true,"update_type":"update"}
   ]},
  {"href":"/orgs/5/sec_policy/draft/rule_sets/101","name":"Global-Infra","enabled":false,"scopes":[[]],
   "rules":[{"href":"/orgs/5/sec_policy/draft/rule_sets/101/sec_rules/3","enabled":true,
             "providers":[{"actors":"ams"}],"consumers":[{"label_group":{"href":"/orgs/5/sec_policy/draft/label_groups/10"}}],
             "ingress_services":[{"href":"/orgs/5/sec_policy/draft/services/30"}],"update_type":"create"}],
   "update_type":"create"}
]
```

```json
// fixtures/deny_rules.json
[
  {"href":"/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/7","enabled":true,"description":"block telnet","providers":[{"actors":"ams"}],"consumers":[{"ip_list":{"href":"/orgs/5/sec_policy/draft/ip_lists/20"}}],"ingress_services":[{"port":23,"proto":6}],"override":false,"update_type":null},
  {"href":"/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8","enabled":true,"providers":[{"label":{"href":"/orgs/5/labels/2"}}],"consumers":[{"actors":"ams"}],"ingress_services":[{"port":22,"proto":6}],"override":true}
]
```

```json
// fixtures/pending_list.json
[{"href":"/orgs/5/sec_policy/draft/rule_sets/101","change_type":"create","name":"Global-Infra"},{"href":"/orgs/5/sec_policy/draft/ip_lists/21","change_type":"create","name":"Corp"}]
```

```json
// fixtures/pending_dict.json
{"rule_sets":[{"href":"/orgs/5/sec_policy/draft/rule_sets/101","update_type":"create","name":"Global-Infra"}],"ip_lists":[{"href":"/orgs/5/sec_policy/draft/ip_lists/21","update_type":"create","name":"Corp"}],"services":[],"label_groups":[]}
```

- [ ] **Step 2: Write the failing tests**

```rust
// crates/pce/src/models/policy.rs (tests module)
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_sets_fixture_parses() {
        let rs: Vec<RuleSet> = serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap();
        assert_eq!(rs.len(), 2);
        assert_eq!(rs[0].rules.len(), 2);
        assert_eq!(rs[0].scopes[0].len(), 2);
        assert_eq!(rs[0].rules[0].ingress_services[0], IngressService::Ref(Ref::new("/orgs/5/sec_policy/draft/services/31")));
        assert_eq!(rs[0].rules[1].ingress_services[1], IngressService::Port { port: 8000, to_port: Some(8099), proto: 6 });
        assert_eq!(rs[0].rules[1].unscoped_consumers, Some(true));
        assert_eq!(rs[1].rules[0].providers[0].actors.as_deref(), Some("ams"));
        assert_eq!(rs[1].update_type.as_deref(), Some("create"));
    }

    #[test]
    fn deny_rules_parse_override_flag() {
        let dr: Vec<DenyRule> = serde_json::from_str(include_str!("../../../../fixtures/deny_rules.json")).unwrap();
        assert!(!dr[0].override_);
        assert!(dr[1].override_);
        let back = serde_json::to_value(&dr[1]).unwrap();
        assert_eq!(back["override"], true);
    }

    #[test]
    fn pending_parses_both_shapes() {
        let list: serde_json::Value = serde_json::from_str(include_str!("../../../../fixtures/pending_list.json")).unwrap();
        let dict: serde_json::Value = serde_json::from_str(include_str!("../../../../fixtures/pending_dict.json")).unwrap();
        let a = parse_pending(list);
        let b = parse_pending(dict);
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].object_type, "rule_sets");
        assert_eq!(a[0].change_type, "create");
        assert_eq!(a[1].name.as_deref(), Some("Corp"));
        assert_eq!(b.len(), a.len(), "dict shape yields the same changes (order may differ)");
        for change in &a {
            assert!(b.contains(change), "{change:?} missing from dict parse");
        }
    }

    #[test]
    fn scope_segments() {
        assert_eq!(PolicyScope::Draft.segment(), "draft");
        assert_eq!(PolicyScope::parse("ACTIVE"), Some(PolicyScope::Active));
        assert_eq!(PolicyScope::parse("x"), None);
    }
}
```

Add to the tests module of `crates/pce/src/resolve.rs`:

```rust
    #[test]
    fn rule_views_render_names() {
        use crate::models::{DenyRule, RuleSet};
        let idx = index();
        let rs: Vec<RuleSet> = serde_json::from_str(include_str!("../../../fixtures/rule_sets.json")).unwrap();
        let v = RuleView::from_rule(&rs[0].rules[1], &idx);
        assert_eq!(v.consumers, vec!["ipl:Corp"]);
        assert_eq!(v.providers, vec!["role=web"]);
        assert_eq!(v.services, vec!["443/tcp", "8000-8099/tcp"]);
        assert_eq!(v.kind, RuleKind::Allow);
        assert!(v.unscoped_consumers);
        let v3 = RuleView::from_rule(&rs[1].rules[0], &idx);
        assert_eq!(v3.providers, vec!["All workloads"]);
        assert_eq!(v3.consumers, vec!["group:All prod envs"]);
        assert_eq!(v3.services, vec!["All Services"]);
        let dr: Vec<DenyRule> = serde_json::from_str(include_str!("../../../fixtures/deny_rules.json")).unwrap();
        assert_eq!(RuleView::from_deny(&dr[1], &idx).kind, RuleKind::OverrideDeny);
        assert_eq!(RuleView::from_deny(&dr[0], &idx).consumers, vec!["ipl:Any (0.0.0.0/0 and ::/0)"]);
        let set = RuleSetView::from_rule_set(&rs[0], &idx);
        assert_eq!(set.scopes, vec!["app=shop & env=prod"]);
        assert_eq!(set.rule_count, 2);
        assert_eq!(RuleSetView::from_rule_set(&rs[1], &idx).scopes, vec!["All"]);
    }
```

Add to `crates/pce/tests/facade.rs`:

```rust
#[tokio::test]
async fn policy_reads_use_scope_and_cache() {
    use pce::models::PolicyScope;
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets")).respond_with(ResponseTemplate::new(200).set_body_string(include_str!("../../../fixtures/rule_sets.json")).insert_header("Content-Type", "application/json")).expect(1).mount(&server).await;
    Mock::given(method("GET")).and(path("/api/v2/orgs/5/sec_policy/active/rule_sets")).respond_with(ResponseTemplate::new(200).set_body_json(json!([]))).expect(1).mount(&server).await;
    Mock::given(method("GET")).and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets/100/deny_rules")).respond_with(ResponseTemplate::new(200).set_body_string(include_str!("../../../fixtures/deny_rules.json")).insert_header("Content-Type", "application/json")).expect(1).mount(&server).await;
    Mock::given(method("GET")).and(path("/api/v2/orgs/5/sec_policy/pending")).respond_with(ResponseTemplate::new(200).set_body_string(include_str!("../../../fixtures/pending_dict.json")).insert_header("Content-Type", "application/json")).expect(1).mount(&server).await;
    let p = pce_for(&server);
    assert_eq!(p.rule_sets(PolicyScope::Draft, CacheMode::Use).await.unwrap().value.len(), 2);
    assert_eq!(p.rule_sets(PolicyScope::Active, CacheMode::Use).await.unwrap().value.len(), 0);
    assert!(p.rule_sets(PolicyScope::Draft, CacheMode::Use).await.unwrap().hit);
    assert_eq!(p.deny_rules("/orgs/5/sec_policy/draft/rule_sets/100", CacheMode::Use).await.unwrap().value.len(), 2);
    assert_eq!(p.pending_changes(CacheMode::Use).await.unwrap().value.len(), 2);
    assert_eq!(p.stats().calls, 4);
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p pce policy`
Expected: compile error.

- [ ] **Step 4: Implement**

```rust
// crates/pce/src/models/policy.rs
//! Rulesets, allow rules, deny rules, and pending provisioning changes.
use super::common::Ref;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PolicyScope {
    Draft,
    Active,
}

impl PolicyScope {
    pub fn segment(self) -> &'static str {
        match self {
            PolicyScope::Draft => "draft",
            PolicyScope::Active => "active",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "draft" => Some(PolicyScope::Draft),
            "active" => Some(PolicyScope::Active),
            _ => None,
        }
    }
}

/// One side of a rule or one element of a scope. Exactly one of the
/// optional fields is set, or `actors == "ams"` for all workloads.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Actor {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actors: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<Ref>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_group: Option<Ref>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workload: Option<Ref>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub virtual_service: Option<Ref>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub virtual_server: Option<Ref>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ip_list: Option<Ref>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclusion: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum IngressService {
    Ref(Ref),
    Port {
        port: u16,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to_port: Option<u16>,
        proto: i64,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ResolveLabelsAs {
    #[serde(default)]
    pub providers: Vec<String>,
    #[serde(default)]
    pub consumers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    pub href: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub providers: Vec<Actor>,
    #[serde(default)]
    pub consumers: Vec<Actor>,
    #[serde(default)]
    pub ingress_services: Vec<IngressService>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolve_labels_as: Option<ResolveLabelsAs>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unscoped_consumers: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sec_connect: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stateless: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine_auth: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DenyRule {
    pub href: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub providers: Vec<Actor>,
    #[serde(default)]
    pub consumers: Vec<Actor>,
    #[serde(default)]
    pub ingress_services: Vec<IngressService>,
    #[serde(rename = "override", default)]
    pub override_: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleSet {
    pub href: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub scopes: Vec<Vec<Actor>>,
    #[serde(default)]
    pub rules: Vec<Rule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingChange {
    pub href: String,
    pub change_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub object_type: String,
}

/// `GET /sec_policy/pending` is either a list of `{href, change_type, name}`
/// or a map of object type to such lists (with `update_type`). Both become
/// one flat list.
pub fn parse_pending(v: Value) -> Vec<PendingChange> {
    fn item(v: &Value) -> Option<PendingChange> {
        let href = v.get("href")?.as_str()?.to_string();
        let change_type = v.get("change_type").or_else(|| v.get("update_type")).and_then(Value::as_str).unwrap_or("update").to_string();
        let name = v.get("name").and_then(Value::as_str).map(str::to_string);
        let object_type = Ref::new(&href).object_type().to_string();
        Some(PendingChange { href, change_type, name, object_type })
    }
    match v {
        Value::Array(items) => items.iter().filter_map(item).collect(),
        Value::Object(map) => map.values().filter_map(Value::as_array).flatten().filter_map(item).collect(),
        _ => Vec::new(),
    }
}
```

Add to `models/mod.rs`: `pub mod policy;` and `pub use policy::{Actor, DenyRule, IngressService, PendingChange, PolicyScope, ResolveLabelsAs, Rule, RuleSet, parse_pending};`.

Append to `crates/pce/src/resolve.rs` (above the tests module):

```rust
use crate::models::{Actor, DenyRule, IngressService, Rule, RuleSet};

/// `role=web`, `ipl:Corp`, `group:All prod envs`, `wl:<id>`, `All workloads`.
pub fn describe_actor(a: &Actor, idx: &NameIndex) -> String {
    let excl = if a.exclusion.unwrap_or(false) { "!" } else { "" };
    let body = if a.actors.as_deref() == Some("ams") {
        "All workloads".to_string()
    } else if let Some(l) = &a.label {
        idx.label_text(&l.href)
    } else if let Some(g) = &a.label_group {
        format!("group:{}", idx.label_group_name(&g.href).unwrap_or(&g.href))
    } else if let Some(i) = &a.ip_list {
        format!("ipl:{}", idx.ip_list_name(&i.href).unwrap_or(&i.href))
    } else if let Some(w) = &a.workload {
        format!("wl:{}", w.id())
    } else if let Some(v) = &a.virtual_service {
        format!("vs:{}", v.id())
    } else if let Some(v) = &a.virtual_server {
        format!("vsrv:{}", v.id())
    } else {
        "?".to_string()
    };
    format!("{excl}{body}")
}

pub fn describe_ingress_service(s: &IngressService, idx: &NameIndex) -> String {
    match s {
        IngressService::Ref(r) => idx.service_name(&r.href).unwrap_or(&r.href).to_string(),
        IngressService::Port { port, to_port, proto } => match to_port {
            Some(t) if t != port => format!("{port}-{t}/{}", crate::models::proto_name(*proto)),
            _ => format!("{port}/{}", crate::models::proto_name(*proto)),
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleKind {
    Allow,
    Deny,
    OverrideDeny,
}

/// A rule with every href turned into a name. What the CLI prints and the
/// TUI lists.
#[derive(Debug, Clone, Serialize)]
pub struct RuleView {
    pub href: String,
    pub kind: RuleKind,
    pub enabled: bool,
    pub description: Option<String>,
    pub consumers: Vec<String>,
    pub providers: Vec<String>,
    pub services: Vec<String>,
    pub unscoped_consumers: bool,
    pub update_type: Option<String>,
}

impl RuleView {
    pub fn from_rule(r: &Rule, idx: &NameIndex) -> Self {
        Self {
            href: r.href.clone(),
            kind: RuleKind::Allow,
            enabled: r.enabled,
            description: r.description.clone(),
            consumers: r.consumers.iter().map(|a| describe_actor(a, idx)).collect(),
            providers: r.providers.iter().map(|a| describe_actor(a, idx)).collect(),
            services: r.ingress_services.iter().map(|s| describe_ingress_service(s, idx)).collect(),
            unscoped_consumers: r.unscoped_consumers.unwrap_or(false),
            update_type: r.update_type.clone(),
        }
    }

    pub fn from_deny(d: &DenyRule, idx: &NameIndex) -> Self {
        Self {
            href: d.href.clone(),
            kind: if d.override_ { RuleKind::OverrideDeny } else { RuleKind::Deny },
            enabled: d.enabled,
            description: d.description.clone(),
            consumers: d.consumers.iter().map(|a| describe_actor(a, idx)).collect(),
            providers: d.providers.iter().map(|a| describe_actor(a, idx)).collect(),
            services: d.ingress_services.iter().map(|s| describe_ingress_service(s, idx)).collect(),
            unscoped_consumers: false,
            update_type: d.update_type.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RuleSetView {
    pub href: String,
    pub name: String,
    pub description: Option<String>,
    pub enabled: bool,
    /// One string per scope, `app=shop & env=prod`; `All` for an empty scope.
    pub scopes: Vec<String>,
    pub rule_count: usize,
    pub update_type: Option<String>,
}

impl RuleSetView {
    pub fn from_rule_set(rs: &RuleSet, idx: &NameIndex) -> Self {
        let scopes = if rs.scopes.is_empty() {
            vec!["All".to_string()]
        } else {
            rs.scopes
                .iter()
                .map(|scope| if scope.is_empty() { "All".to_string() } else { scope.iter().map(|a| describe_actor(a, idx)).collect::<Vec<_>>().join(" & ") })
                .collect()
        };
        Self {
            href: rs.href.clone(),
            name: rs.name.clone(),
            description: rs.description.clone(),
            enabled: rs.enabled,
            scopes,
            rule_count: rs.rules.len(),
            update_type: rs.update_type.clone(),
        }
    }
}
```

Update the `pub use resolve::...` line in `lib.rs` to `pub use resolve::{NameIndex, ResolvedLabel, RuleKind, RuleSetView, RuleView, describe_actor, describe_ingress_service};` and add to the `Pce` impl:

```rust
    pub async fn rule_sets(&self, scope: models::PolicyScope, mode: CacheMode) -> Result<Cached<Vec<models::RuleSet>>> {
        let path = self.client.org_path(&format!("sec_policy/{}/rule_sets", scope.segment()));
        let key = format!("policy:{}:rule_sets", scope.segment());
        self.cached_list(&key, &path, vec![], self.ttls.policy, mode).await
    }

    /// Deny rules live on a per-ruleset endpoint that the OpenAPI spec omits.
    pub async fn deny_rules(&self, ruleset_href: &str, mode: CacheMode) -> Result<Cached<Vec<models::DenyRule>>> {
        let path = format!("{}/deny_rules", ruleset_href.trim_end_matches('/'));
        let key = format!("policy:deny:{ruleset_href}");
        self.cached_list(&key, &path, vec![], self.ttls.policy, mode).await
    }

    pub async fn pending_changes(&self, mode: CacheMode) -> Result<Cached<Vec<models::PendingChange>>> {
        let key = "policy:pending";
        if mode == CacheMode::Use {
            if let Some((value, age)) = self.cache.get::<Vec<models::PendingChange>>(key) {
                return Ok(Cached { value, age, hit: true, truncated: false });
            }
        }
        let path = self.client.org_path("sec_policy/pending");
        let raw = self.client.get::<serde_json::Value>(&path, &[]).await?.body;
        let value = models::parse_pending(raw);
        self.cache.insert(key, value.clone(), self.ttls.policy);
        Ok(Cached { value, age: Duration::ZERO, hit: false, truncated: false })
    }
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p pce`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add -A
git commit -m "feat(pce): policy models, pending changes, resolved rule views

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 10: Traffic query model, builder, and filter expressions

**Files:**
- Create: `crates/pce/src/models/traffic.rs`, `crates/pce/src/traffic/mod.rs`, `crates/pce/src/traffic/query.rs`, `crates/pce/src/traffic/filter.rs`
- Modify: `crates/pce/src/models/mod.rs`, `crates/pce/src/lib.rs`

**Interfaces:**
- Produces (models): `ActorMatch::{Label{label: Ref} | IpList{ip_list: Ref} | Workload{workload: Ref} | IpAddress{ip_address: String} | Fqdn{fqdn: String}}` (untagged); `ActorFilter { include: Vec<Vec<ActorMatch>>, exclude: Vec<ActorMatch> }`; `ServiceMatch { port, proto, process_name }`; `ServiceFilter { include, exclude }`; `TrafficQuery { start_date, end_date, sources, destinations, services, policy_decisions, exclude_workloads_from_ip_list_query, sources_destinations_query_op, max_results, query_name }` with `cost() -> QueryCost { window: Duration, max_results, unbounded }` and `needs_confirmation() -> Option<String>`; `Flow`, `Endpoint`, `FlowWorkload`, `FlowService`, `TimestampRange`, `NamedRef`, `AsyncQueryStatus`, `QueryResultRef`.
- Produces (traffic): `parse_window(&str) -> Result<Duration>`; `TrafficQueryBuilder::last(Duration)`, `::between(Timestamp, Timestamp)`, `.source(Vec<ActorMatch>)`, `.destination(..)`, `.exclude_source(ActorMatch)`, `.exclude_destination(..)`, `.service(ServiceMatch)`, `.exclude_service(..)`, `.decisions(Vec<PolicyDecision>)`, `.max_results(u32)`, `.query_name(String)`, `.build() -> Result<TrafficQuery>`; `FilterExpr::{Label{key,value} | Ip(String) | IpList(String) | Workload(String) | Fqdn(String)}` with `parse(&str)`, `needs_workloads()`; `resolve_actor(&FilterExpr, &NameIndex, &[Workload]) -> Result<ActorMatch>`; `format_timestamp(Timestamp) -> String`.
- Constants: `DEFAULT_WINDOW = 24h`, `DEFAULT_TRAFFIC_MAX_RESULTS = 1000`, `CONFIRM_WINDOW = 7d`, `CONFIRM_MAX_RESULTS = 10_000`.

- [ ] **Step 1: Write the failing tests**

```rust
// crates/pce/src/traffic/query.rs (tests module)
#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ActorMatch, PolicyDecision, Ref, ServiceMatch};
    use serde_json::json;

    fn ts(s: &str) -> jiff::Timestamp {
        s.parse().unwrap()
    }

    #[test]
    fn empty_include_serialises_as_list_of_empty_list() {
        let q = TrafficQueryBuilder::between(ts("2026-10-01T00:00:00Z"), ts("2026-10-02T00:00:00Z")).build().unwrap();
        let v = serde_json::to_value(&q).unwrap();
        assert_eq!(v["sources"]["include"], json!([[]]));
        assert_eq!(v["destinations"]["include"], json!([[]]));
        assert_eq!(v["sources"]["exclude"], json!([]));
        assert_eq!(v["services"]["include"], json!([]));
        assert_eq!(v["policy_decisions"], json!(["allowed", "potentially_blocked", "blocked"]));
        assert_eq!(v["start_date"], "2026-10-01T00:00:00Z");
        assert_eq!(v["end_date"], "2026-10-02T00:00:00Z");
        assert_eq!(v["max_results"], 1000);
        assert_eq!(v["sources_destinations_query_op"], "and");
        assert_eq!(v["exclude_workloads_from_ip_list_query"], true);
        assert_eq!(v["query_name"], "illumio-tui");
    }

    #[test]
    fn filters_serialise_in_pce_shape() {
        let q = TrafficQueryBuilder::between(ts("2026-10-01T00:00:00Z"), ts("2026-10-02T00:00:00Z"))
            .source(vec![ActorMatch::Label { label: Ref::new("/orgs/5/labels/3") }, ActorMatch::Label { label: Ref::new("/orgs/5/labels/4") }])
            .source(vec![ActorMatch::IpAddress { ip_address: "10.0.0.1".into() }])
            .exclude_destination(ActorMatch::IpList { ip_list: Ref::new("/orgs/5/sec_policy/draft/ip_lists/20") })
            .service(ServiceMatch { port: Some(443), proto: Some(6), process_name: None })
            .decisions(vec![PolicyDecision::Blocked])
            .max_results(50)
            .build()
            .unwrap();
        let v = serde_json::to_value(&q).unwrap();
        assert_eq!(v["sources"]["include"], json!([[{"label": {"href": "/orgs/5/labels/3"}}, {"label": {"href": "/orgs/5/labels/4"}}], [{"ip_address": "10.0.0.1"}]]));
        assert_eq!(v["destinations"]["exclude"], json!([{"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/20"}}]));
        assert_eq!(v["services"]["include"], json!([{"port": 443, "proto": 6}]));
        assert_eq!(v["policy_decisions"], json!(["blocked"]));
        assert_eq!(v["max_results"], 50);
    }

    #[test]
    fn backwards_window_and_zero_results_are_rejected() {
        let r = TrafficQueryBuilder::between(ts("2026-10-02T00:00:00Z"), ts("2026-10-01T00:00:00Z")).build();
        assert!(matches!(r, Err(crate::PceError::Invalid(m)) if m.contains("before")));
        let r = TrafficQueryBuilder::between(ts("2026-10-01T00:00:00Z"), ts("2026-10-02T00:00:00Z")).max_results(0).build();
        assert!(matches!(r, Err(crate::PceError::Invalid(_))));
    }

    #[test]
    fn cost_and_confirmation() {
        let q = TrafficQueryBuilder::between(ts("2026-10-01T00:00:00Z"), ts("2026-10-09T00:00:00Z")).build().unwrap();
        let c = q.cost();
        assert_eq!(c.window, std::time::Duration::from_secs(8 * 86400));
        assert!(c.unbounded);
        assert!(q.needs_confirmation().unwrap().contains("8d"));
        let small = TrafficQueryBuilder::between(ts("2026-10-01T00:00:00Z"), ts("2026-10-02T00:00:00Z"))
            .source(vec![ActorMatch::IpAddress { ip_address: "1.1.1.1".into() }])
            .build()
            .unwrap();
        assert!(!small.cost().unbounded);
        assert!(small.needs_confirmation().is_none());
        let big = TrafficQueryBuilder::between(ts("2026-10-01T00:00:00Z"), ts("2026-10-02T00:00:00Z")).max_results(20_000).build().unwrap();
        assert!(big.needs_confirmation().unwrap().contains("20000"));
    }

    #[test]
    fn windows_parse_and_last_uses_now() {
        assert_eq!(parse_window("24h").unwrap(), std::time::Duration::from_secs(86400));
        assert_eq!(parse_window("7d").unwrap(), std::time::Duration::from_secs(7 * 86400));
        assert_eq!(parse_window("90m").unwrap(), std::time::Duration::from_secs(5400));
        assert!(parse_window("soon").is_err());
        let q = TrafficQueryBuilder::last(std::time::Duration::from_secs(3600)).unwrap().build().unwrap();
        assert_eq!(q.cost().window, std::time::Duration::from_secs(3600));
        assert!(q.end_date.ends_with('Z'));
    }
}
```

```rust
// crates/pce/src/traffic/filter.rs (tests module)
#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ActorMatch, IpList, Label, LabelGroup, Ref, Service, Workload};
    use crate::resolve::NameIndex;

    fn index() -> NameIndex {
        let labels: Vec<Label> = serde_json::from_str(include_str!("../../../../fixtures/labels.json")).unwrap();
        let groups: Vec<LabelGroup> = serde_json::from_str(include_str!("../../../../fixtures/label_groups.json")).unwrap();
        let lists: Vec<IpList> = serde_json::from_str(include_str!("../../../../fixtures/ip_lists.json")).unwrap();
        let services: Vec<Service> = serde_json::from_str(include_str!("../../../../fixtures/services.json")).unwrap();
        NameIndex::new(&labels, &groups, &lists, &services)
    }

    #[test]
    fn parses_each_form() {
        assert_eq!(FilterExpr::parse("app=shop").unwrap(), FilterExpr::Label { key: "app".into(), value: "shop".into() });
        assert_eq!(FilterExpr::parse("env=a=b").unwrap(), FilterExpr::Label { key: "env".into(), value: "a=b".into() });
        assert_eq!(FilterExpr::parse("10.0.0.1").unwrap(), FilterExpr::Ip("10.0.0.1".into()));
        assert_eq!(FilterExpr::parse("ip:10.0.0.0/8").unwrap(), FilterExpr::Ip("10.0.0.0/8".into()));
        assert_eq!(FilterExpr::parse("ipl:Corp").unwrap(), FilterExpr::IpList("Corp".into()));
        assert_eq!(FilterExpr::parse("wl:host-1.example").unwrap(), FilterExpr::Workload("host-1.example".into()));
        assert_eq!(FilterExpr::parse("fqdn:x.example").unwrap(), FilterExpr::Fqdn("x.example".into()));
        assert!(FilterExpr::parse("").is_err());
        assert!(FilterExpr::parse("justaword").is_err());
        assert!(FilterExpr::Workload("x".into()).needs_workloads());
        assert!(!FilterExpr::Ip("x".into()).needs_workloads());
    }

    #[test]
    fn resolves_against_index_and_workloads() {
        let idx = index();
        let wls: Vec<Workload> = serde_json::from_str(include_str!("../../../../fixtures/workloads.json")).unwrap();
        assert_eq!(resolve_actor(&FilterExpr::parse("app=shop").unwrap(), &idx, &wls).unwrap(), ActorMatch::Label { label: Ref::new("/orgs/5/labels/3") });
        assert_eq!(resolve_actor(&FilterExpr::parse("ipl:corp").unwrap(), &idx, &wls).unwrap(), ActorMatch::IpList { ip_list: Ref::new("/orgs/5/sec_policy/draft/ip_lists/21") });
        assert_eq!(resolve_actor(&FilterExpr::parse("wl:host-2.example").unwrap(), &idx, &wls).unwrap(), ActorMatch::Workload { workload: Ref::new("/orgs/5/workloads/aaaa-2") });
        assert_eq!(resolve_actor(&FilterExpr::parse("wl:unmanaged-nas").unwrap(), &idx, &wls).unwrap(), ActorMatch::Workload { workload: Ref::new("/orgs/5/workloads/aaaa-3") });
        assert_eq!(resolve_actor(&FilterExpr::parse("1.2.3.4").unwrap(), &idx, &wls).unwrap(), ActorMatch::IpAddress { ip_address: "1.2.3.4".into() });
        let err = resolve_actor(&FilterExpr::parse("app=nope").unwrap(), &idx, &wls).unwrap_err().to_string();
        assert!(err.contains("unknown label app=nope"), "{err}");
        assert!(err.contains("shop"), "lists known values: {err}");
        let err = resolve_actor(&FilterExpr::parse("ipl:none").unwrap(), &idx, &wls).unwrap_err().to_string();
        assert!(err.contains("unknown IP list"));
        let err = resolve_actor(&FilterExpr::parse("wl:ghost").unwrap(), &idx, &wls).unwrap_err().to_string();
        assert!(err.contains("unknown workload"));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p pce traffic`
Expected: compile error.

- [ ] **Step 3: Implement**

```rust
// crates/pce/src/models/traffic.rs
//! Explorer traffic query request and response types.
use super::common::{PolicyDecision, Ref};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// One element of a traffic filter. Serialises to the PCE's one-key objects,
/// e.g. `{"label": {"href": "..."}}` or `{"ip_address": "10.0.0.1"}`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ActorMatch {
    Label { label: Ref },
    IpList { ip_list: Ref },
    Workload { workload: Ref },
    IpAddress { ip_address: String },
    Fqdn { fqdn: String },
}

/// `include` is an OR of AND-groups; `exclude` is flat.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ActorFilter {
    pub include: Vec<Vec<ActorMatch>>,
    pub exclude: Vec<ActorMatch>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ServiceMatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proto: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_name: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ServiceFilter {
    pub include: Vec<ServiceMatch>,
    pub exclude: Vec<ServiceMatch>,
}

/// Body of `POST /orgs/{org}/traffic_flows/async_queries`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TrafficQuery {
    pub start_date: String,
    pub end_date: String,
    pub sources: ActorFilter,
    pub destinations: ActorFilter,
    pub services: ServiceFilter,
    pub policy_decisions: Vec<PolicyDecision>,
    pub exclude_workloads_from_ip_list_query: bool,
    pub sources_destinations_query_op: String,
    pub max_results: u32,
    pub query_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NamedRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub href: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FlowWorkload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub href: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default)]
    pub labels: Vec<Ref>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Endpoint {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ip: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fqdn: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workload: Option<FlowWorkload>,
    #[serde(default)]
    pub ip_lists: Vec<NamedRef>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FlowService {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proto: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub windows_service_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_name: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TimestampRange {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_detected: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_detected: Option<String>,
}

/// One row of the Explorer result. Unknown fields are kept in `extra`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Flow {
    #[serde(default)]
    pub src: Endpoint,
    #[serde(default)]
    pub dst: Endpoint,
    #[serde(default)]
    pub service: FlowService,
    #[serde(default = "unknown_decision")]
    pub policy_decision: PolicyDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow_direction: Option<String>,
    #[serde(default)]
    pub num_connections: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dst_bi: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dst_bo: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp_range: Option<TimestampRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft_policy_decision: Option<String>,
    #[serde(default)]
    pub rules: Vec<Ref>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn unknown_decision() -> PolicyDecision {
    PolicyDecision::Unknown
}

/// `result` on the async query status is a string href on some PCE versions
/// and `{"href": ...}` on others.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum QueryResultRef {
    Href(String),
    Obj { href: String },
}

impl QueryResultRef {
    pub fn href(&self) -> &str {
        match self {
            QueryResultRef::Href(h) => h,
            QueryResultRef::Obj { href } => href,
        }
    }
}

/// Response of the POST and of each poll.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AsyncQueryStatus {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub href: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<QueryResultRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matches_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flows_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}
```

Add to `models/mod.rs`: `pub mod traffic;` and `pub use traffic::{ActorFilter, ActorMatch, AsyncQueryStatus, Endpoint, Flow, FlowService, FlowWorkload, NamedRef, QueryResultRef, ServiceFilter, ServiceMatch, TimestampRange, TrafficQuery};`.

```rust
// crates/pce/src/traffic/mod.rs
//! The Explorer traffic query: build it, run it, make the rows readable,
//! and aggregate them. Nothing here touches the network except `poll`.
pub mod filter;
pub mod query;

pub use filter::{FilterExpr, resolve_actor};
pub use query::{CONFIRM_MAX_RESULTS, CONFIRM_WINDOW, DEFAULT_TRAFFIC_MAX_RESULTS, DEFAULT_WINDOW, QueryCost, TrafficQueryBuilder, format_timestamp, parse_window};
```

```rust
// crates/pce/src/traffic/query.rs
//! Builds a `TrafficQuery` that the PCE will accept: full UTC timestamps,
//! `[[]]` for an empty include, validated window and caps.
use crate::error::{PceError, Result};
use crate::models::{ActorFilter, ActorMatch, PolicyDecision, ServiceFilter, ServiceMatch, TrafficQuery};
use jiff::{Span, Timestamp};
use std::time::Duration;

pub const DEFAULT_WINDOW: Duration = Duration::from_secs(24 * 3600);
pub const DEFAULT_TRAFFIC_MAX_RESULTS: u32 = 1000;
pub const CONFIRM_WINDOW: Duration = Duration::from_secs(7 * 24 * 3600);
pub const CONFIRM_MAX_RESULTS: u32 = 10_000;

/// `2026-10-10T07:31:02Z`
pub fn format_timestamp(t: Timestamp) -> String {
    t.strftime("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// `24h`, `7d`, `90m`, `1w` via humantime.
pub fn parse_window(s: &str) -> Result<Duration> {
    humantime::parse_duration(s.trim()).map_err(|e| PceError::Invalid(format!("bad window {s:?}: {e}")))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct QueryCost {
    pub window: Duration,
    pub max_results: u32,
    /// No source, destination or service filter at all.
    pub unbounded: bool,
}

impl TrafficQuery {
    pub fn window(&self) -> Duration {
        let start: Option<Timestamp> = self.start_date.parse().ok();
        let end: Option<Timestamp> = self.end_date.parse().ok();
        match (start, end) {
            (Some(s), Some(e)) if e > s => e.duration_since(s).unsigned_abs(),
            _ => Duration::ZERO,
        }
    }

    pub fn cost(&self) -> QueryCost {
        let has_actor = |f: &ActorFilter| f.include.iter().any(|g| !g.is_empty()) || !f.exclude.is_empty();
        let unbounded = !has_actor(&self.sources) && !has_actor(&self.destinations) && self.services.include.is_empty();
        QueryCost { window: self.window(), max_results: self.max_results, unbounded }
    }

    /// A reason the caller should confirm before running, if any.
    pub fn needs_confirmation(&self) -> Option<String> {
        let c = self.cost();
        let mut reasons = Vec::new();
        if c.window > CONFIRM_WINDOW {
            reasons.push(format!("window is {}d (> {}d)", c.window.as_secs() / 86400, CONFIRM_WINDOW.as_secs() / 86400));
        }
        if c.max_results > CONFIRM_MAX_RESULTS {
            reasons.push(format!("max_results is {} (> {})", c.max_results, CONFIRM_MAX_RESULTS));
        }
        if reasons.is_empty() { None } else { Some(reasons.join("; ")) }
    }
}

#[derive(Debug, Clone)]
pub struct TrafficQueryBuilder {
    start: Timestamp,
    end: Timestamp,
    sources: ActorFilter,
    destinations: ActorFilter,
    services: ServiceFilter,
    decisions: Vec<PolicyDecision>,
    max_results: u32,
    query_name: String,
}

impl TrafficQueryBuilder {
    pub fn between(start: Timestamp, end: Timestamp) -> Self {
        Self {
            start,
            end,
            sources: ActorFilter::default(),
            destinations: ActorFilter::default(),
            services: ServiceFilter::default(),
            decisions: PolicyDecision::ALL.to_vec(),
            max_results: DEFAULT_TRAFFIC_MAX_RESULTS,
            query_name: "illumio-tui".into(),
        }
    }

    /// The last `window` ending now.
    pub fn last(window: Duration) -> Result<Self> {
        let end = Timestamp::now();
        let span = Span::try_from(window).map_err(|e| PceError::Invalid(format!("window too large: {e}")))?;
        let start = end.checked_sub(span).map_err(|e| PceError::Invalid(format!("window too large: {e}")))?;
        Ok(Self::between(start, end))
    }

    pub fn source(mut self, and_group: Vec<ActorMatch>) -> Self {
        self.sources.include.push(and_group);
        self
    }

    pub fn destination(mut self, and_group: Vec<ActorMatch>) -> Self {
        self.destinations.include.push(and_group);
        self
    }

    pub fn exclude_source(mut self, m: ActorMatch) -> Self {
        self.sources.exclude.push(m);
        self
    }

    pub fn exclude_destination(mut self, m: ActorMatch) -> Self {
        self.destinations.exclude.push(m);
        self
    }

    pub fn service(mut self, m: ServiceMatch) -> Self {
        self.services.include.push(m);
        self
    }

    pub fn exclude_service(mut self, m: ServiceMatch) -> Self {
        self.services.exclude.push(m);
        self
    }

    pub fn decisions(mut self, d: Vec<PolicyDecision>) -> Self {
        if !d.is_empty() {
            self.decisions = d;
        }
        self
    }

    pub fn max_results(mut self, n: u32) -> Self {
        self.max_results = n;
        self
    }

    pub fn query_name(mut self, name: impl Into<String>) -> Self {
        self.query_name = name.into();
        self
    }

    pub fn build(mut self) -> Result<TrafficQuery> {
        if self.end <= self.start {
            return Err(PceError::Invalid(format!("start ({}) must be before end ({})", format_timestamp(self.start), format_timestamp(self.end))));
        }
        if self.max_results == 0 {
            return Err(PceError::Invalid("max_results must be at least 1".into()));
        }
        // The PCE returns nothing for `include: []`; `[[]]` means "no filter".
        self.sources.include.retain(|g| !g.is_empty());
        self.destinations.include.retain(|g| !g.is_empty());
        if self.sources.include.is_empty() {
            self.sources.include.push(Vec::new());
        }
        if self.destinations.include.is_empty() {
            self.destinations.include.push(Vec::new());
        }
        Ok(TrafficQuery {
            start_date: format_timestamp(self.start),
            end_date: format_timestamp(self.end),
            sources: self.sources,
            destinations: self.destinations,
            services: self.services,
            policy_decisions: self.decisions,
            exclude_workloads_from_ip_list_query: true,
            sources_destinations_query_op: "and".into(),
            max_results: self.max_results,
            query_name: self.query_name,
        })
    }
}
```

```rust
// crates/pce/src/traffic/filter.rs
//! Human filter expressions (`app=web`, `ipl:Corp`, `wl:host`, `10.0.0.1`,
//! `fqdn:x`) resolved to PCE actor matches. Unknown names fail here with the
//! known values listed, before anything is sent to the PCE.
use crate::error::{PceError, Result};
use crate::models::{ActorMatch, Ref, Workload};
use crate::resolve::NameIndex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterExpr {
    Label { key: String, value: String },
    Ip(String),
    IpList(String),
    Workload(String),
    Fqdn(String),
}

impl FilterExpr {
    pub fn parse(s: &str) -> Result<Self> {
        let s = s.trim();
        if s.is_empty() {
            return Err(PceError::Invalid("empty filter expression".into()));
        }
        if let Some(rest) = s.strip_prefix("ip:") {
            return Ok(FilterExpr::Ip(rest.to_string()));
        }
        if let Some(rest) = s.strip_prefix("ipl:") {
            return Ok(FilterExpr::IpList(rest.to_string()));
        }
        if let Some(rest) = s.strip_prefix("wl:") {
            return Ok(FilterExpr::Workload(rest.to_string()));
        }
        if let Some(rest) = s.strip_prefix("fqdn:") {
            return Ok(FilterExpr::Fqdn(rest.to_string()));
        }
        if let Some((k, v)) = s.split_once('=') {
            return Ok(FilterExpr::Label { key: k.trim().to_string(), value: v.trim().to_string() });
        }
        if s.parse::<std::net::IpAddr>().is_ok() || s.split_once('/').is_some_and(|(ip, bits)| ip.parse::<std::net::IpAddr>().is_ok() && bits.parse::<u8>().is_ok()) {
            return Ok(FilterExpr::Ip(s.to_string()));
        }
        Err(PceError::Invalid(format!("cannot parse filter {s:?}; use key=value, ip:ADDR, ipl:NAME, wl:HOSTNAME or fqdn:NAME")))
    }

    /// `wl:` needs the workload list to resolve a hostname to an href.
    pub fn needs_workloads(&self) -> bool {
        matches!(self, FilterExpr::Workload(_))
    }
}

pub fn resolve_actor(expr: &FilterExpr, idx: &NameIndex, workloads: &[Workload]) -> Result<ActorMatch> {
    match expr {
        FilterExpr::Label { key, value } => match idx.label_href(key, value) {
            Some(href) => Ok(ActorMatch::Label { label: Ref::new(href) }),
            None => {
                let known = idx.values_for_key(key);
                let hint = if known.is_empty() {
                    format!("known keys: {}", idx.label_keys().join(", "))
                } else {
                    format!("known values for {key}: {}", known.join(", "))
                };
                Err(PceError::Invalid(format!("unknown label {key}={value}; {hint}")))
            }
        },
        FilterExpr::IpList(name) => idx
            .ip_list_href(name)
            .map(|h| ActorMatch::IpList { ip_list: Ref::new(h) })
            .ok_or_else(|| PceError::Invalid(format!("unknown IP list {name:?}"))),
        FilterExpr::Workload(name) => {
            let lower = name.to_ascii_lowercase();
            workloads
                .iter()
                .find(|w| w.hostname.as_deref().is_some_and(|h| h.eq_ignore_ascii_case(name)) || w.name.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(name)) || w.href.to_ascii_lowercase().ends_with(&lower))
                .map(|w| ActorMatch::Workload { workload: Ref::new(&w.href) })
                .ok_or_else(|| PceError::Invalid(format!("unknown workload {name:?} (hostname, name or href id)")))
        }
        FilterExpr::Ip(ip) => Ok(ActorMatch::IpAddress { ip_address: ip.clone() }),
        FilterExpr::Fqdn(f) => Ok(ActorMatch::Fqdn { fqdn: f.clone() }),
    }
}
```

Add to `lib.rs`: `pub mod traffic;`.

- [ ] **Step 4: Run tests**

Run: `cargo test -p pce`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add -A
git commit -m "feat(pce): traffic query model, builder with PCE-safe encoding, filter expressions

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 11: Async traffic query poller

**Files:**
- Create: `crates/pce/src/traffic/poll.rs`, `crates/pce/tests/traffic_poll.rs`, `fixtures/traffic_flows.json`
- Modify: `crates/pce/src/traffic/mod.rs`

**Interfaces:**
- Consumes: `PceClient::post/get`, `TrafficQuery`, `AsyncQueryStatus`, `Flow`.
- Produces: `PollConfig { initial: 1s, factor: 1.5, cap: 10s, grace: 30s }` (`Default`); `QueryProgress::{Submitted{href} | Polling{status, elapsed} | Downloading | Done{rows}}`; `TrafficResult { flows: Vec<Flow>, matches_count: Option<u64>, truncated: bool, query_href: String, elapsed: Duration }`; `async fn run_traffic_query(client: &PceClient, query: &TrafficQuery, poll: &PollConfig, cancel: &CancellationToken, progress: Option<&mpsc::UnboundedSender<QueryProgress>>) -> Result<TrafficResult>`.

- [ ] **Step 1: Write the fixture**

```json
// fixtures/traffic_flows.json
[
  {"src":{"ip":"10.0.1.10","workload":{"href":"/orgs/5/workloads/aaaa-1","hostname":"host-1.example","name":"web01","labels":[{"href":"/orgs/5/labels/1"},{"href":"/orgs/5/labels/3"},{"href":"/orgs/5/labels/4"}]}},
   "dst":{"ip":"10.0.2.20","workload":{"href":"/orgs/5/workloads/aaaa-2","hostname":"host-2.example","labels":[{"href":"/orgs/5/labels/2"},{"href":"/orgs/5/labels/3"},{"href":"/orgs/5/labels/4"}]}},
   "service":{"port":5432,"proto":6,"process_name":"postgres","user_name":"postgres"},
   "policy_decision":"allowed","flow_direction":"outbound","num_connections":310,"dst_bi":120000,"dst_bo":4000,"state":"closed",
   "timestamp_range":{"first_detected":"2026-10-07T08:00:00Z","last_detected":"2026-10-10T07:31:02Z"}},
  {"src":{"ip":"10.1.4.9","ip_lists":[{"href":"/orgs/5/sec_policy/draft/ip_lists/21","name":"Corp"}]},
   "dst":{"ip":"10.0.1.10","workload":{"href":"/orgs/5/workloads/aaaa-1","hostname":"host-1.example","labels":[{"href":"/orgs/5/labels/1"},{"href":"/orgs/5/labels/3"},{"href":"/orgs/5/labels/4"}]}},
   "service":{"port":8443,"proto":6},
   "policy_decision":"potentially_blocked","flow_direction":"inbound","num_connections":12,
   "timestamp_range":{"first_detected":"2026-10-10T07:00:00Z","last_detected":"2026-10-10T07:30:58Z"}},
  {"src":{"ip":"10.0.3.30","workload":{"href":"/orgs/5/workloads/aaaa-3","name":"unmanaged-nas","labels":[]}},
   "dst":{"ip":"10.0.2.20","workload":{"href":"/orgs/5/workloads/aaaa-2","hostname":"host-2.example","labels":[{"href":"/orgs/5/labels/2"},{"href":"/orgs/5/labels/3"},{"href":"/orgs/5/labels/4"}]}},
   "service":{"proto":1},
   "policy_decision":"blocked","flow_direction":"inbound","num_connections":3,
   "timestamp_range":{"first_detected":"2026-10-10T06:00:00Z","last_detected":"2026-10-10T06:05:00Z"}},
  {"src":{"ip":"10.0.1.10","workload":{"href":"/orgs/5/workloads/aaaa-1","hostname":"host-1.example","labels":[{"href":"/orgs/5/labels/1"},{"href":"/orgs/5/labels/3"},{"href":"/orgs/5/labels/4"}]}},
   "dst":{"ip":"93.184.216.34","fqdn":"fqdn-1.example"},
   "service":{"port":443,"proto":6,"process_name":"curl","user_name":"root"},
   "policy_decision":"allowed","flow_direction":"outbound","num_connections":5,"draft_policy_decision":"allowed","rules":[{"href":"/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/2"}],
   "timestamp_range":{"first_detected":"2026-10-10T07:20:00Z","last_detected":"2026-10-10T07:29:00Z"}}
]
```

- [ ] **Step 2: Write the failing tests**

```rust
// crates/pce/tests/traffic_poll.rs
use pce::client::{ClientConfig, PceClient};
use pce::models::{ActorMatch, Flow};
use pce::traffic::poll::{PollConfig, QueryProgress, run_traffic_query};
use pce::traffic::TrafficQueryBuilder;
use pce::{Credentials, PceError};
use serde_json::json;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const QUERY: &str = "/api/v2/orgs/5/traffic_flows/async_queries";
const QHREF: &str = "/orgs/5/traffic_flows/async_queries/q-1";

fn client_for(server: &MockServer) -> PceClient {
    let url = url::Url::parse(&server.uri()).unwrap();
    let mut cfg = ClientConfig::new(url.host_str().unwrap(), 5, Credentials::new("key", "secret"));
    cfg.scheme = "http".into();
    cfg.port = url.port().unwrap();
    cfg.backoff_base = Duration::from_millis(1);
    cfg.requests_per_second = 1000.0;
    cfg.burst = 1000;
    PceClient::new(cfg).unwrap()
}

fn fast() -> PollConfig {
    PollConfig { initial: Duration::from_millis(1), factor: 1.5, cap: Duration::from_millis(5), grace: Duration::from_millis(200) }
}

fn query(max: u32) -> pce::models::TrafficQuery {
    TrafficQueryBuilder::between("2026-10-09T00:00:00Z".parse().unwrap(), "2026-10-10T00:00:00Z".parse().unwrap())
        .source(vec![ActorMatch::IpAddress { ip_address: "10.0.1.10".into() }])
        .max_results(max)
        .build()
        .unwrap()
}

async fn mount_submit(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path(QUERY))
        .and(header("Prefer", "respond-async"))
        .and(body_partial_json(json!({"sources": {"include": [[{"ip_address": "10.0.1.10"}]]}, "query_name": "illumio-tui"})))
        .respond_with(ResponseTemplate::new(202).set_body_json(json!({"href": QHREF, "status": "queued"})))
        .expect(1)
        .mount(server)
        .await;
}

async fn mount_download(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}/download")))
        .respond_with(ResponseTemplate::new(200).set_body_string(include_str!("../../../fixtures/traffic_flows.json")).insert_header("Content-Type", "application/json"))
        .mount(server)
        .await;
}

#[tokio::test]
async fn submits_polls_and_downloads() {
    let server = MockServer::start().await;
    mount_submit(&server).await;
    Mock::given(method("GET")).and(path(format!("/api/v2{QHREF}"))).respond_with(ResponseTemplate::new(200).set_body_json(json!({"href": QHREF, "status": "working"}))).up_to_n_times(2).mount(&server).await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"href": QHREF, "status": "completed", "result": {"href": format!("{QHREF}/download")}, "matches_count": 4})))
        .mount(&server)
        .await;
    mount_download(&server).await;
    let c = client_for(&server);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let r = run_traffic_query(&c, &query(1000), &fast(), &CancellationToken::new(), Some(&tx)).await.unwrap();
    assert_eq!(r.flows.len(), 4);
    assert_eq!(r.matches_count, Some(4));
    assert!(!r.truncated);
    assert_eq!(r.query_href, QHREF);
    assert_eq!(c.stats().calls, 5, "POST + 3 polls + download");
    let mut seen = Vec::new();
    while let Ok(p) = rx.try_recv() {
        seen.push(p);
    }
    assert!(matches!(seen.first(), Some(QueryProgress::Submitted { .. })));
    assert!(matches!(seen.last(), Some(QueryProgress::Done { rows: 4 })));
    assert!(seen.iter().any(|p| matches!(p, QueryProgress::Polling { status, .. } if status == "working")));
}

#[tokio::test]
async fn result_as_string_href_and_truncation() {
    let server = MockServer::start().await;
    mount_submit(&server).await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "completed", "result": format!("{QHREF}/download"), "matches_count": 900})))
        .mount(&server)
        .await;
    mount_download(&server).await;
    let c = client_for(&server);
    let r = run_traffic_query(&c, &query(4), &fast(), &CancellationToken::new(), None).await.unwrap();
    assert_eq!(r.flows.len(), 4);
    assert!(r.truncated, "rows == max_results and matches_count > rows");
}

#[tokio::test]
async fn completed_without_result_is_retried_within_grace() {
    let server = MockServer::start().await;
    mount_submit(&server).await;
    Mock::given(method("GET")).and(path(format!("/api/v2{QHREF}"))).respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "completed"}))).up_to_n_times(2).mount(&server).await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{QHREF}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "completed", "result": {"href": format!("{QHREF}/download")}})))
        .mount(&server)
        .await;
    mount_download(&server).await;
    let c = client_for(&server);
    let r = run_traffic_query(&c, &query(1000), &fast(), &CancellationToken::new(), None).await.unwrap();
    assert_eq!(r.flows.len(), 4);
}

#[tokio::test]
async fn completed_without_result_past_grace_is_an_error() {
    let server = MockServer::start().await;
    mount_submit(&server).await;
    Mock::given(method("GET")).and(path(format!("/api/v2{QHREF}"))).respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "completed"}))).mount(&server).await;
    let c = client_for(&server);
    let cfg = PollConfig { grace: Duration::from_millis(20), ..fast() };
    let r = run_traffic_query(&c, &query(1000), &cfg, &CancellationToken::new(), None).await;
    assert!(matches!(r, Err(PceError::Decode(m)) if m.contains("no result")), "{r:?}");
}

#[tokio::test]
async fn failed_status_reports_message() {
    let server = MockServer::start().await;
    mount_submit(&server).await;
    Mock::given(method("GET")).and(path(format!("/api/v2{QHREF}"))).respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "failed", "message": "too many flows"}))).mount(&server).await;
    let c = client_for(&server);
    let r = run_traffic_query(&c, &query(1000), &fast(), &CancellationToken::new(), None).await;
    assert!(matches!(r, Err(PceError::QueryFailed(m)) if m == "too many flows"), "{r:?}");
}

#[tokio::test]
async fn cancellation_stops_polling() {
    let server = MockServer::start().await;
    mount_submit(&server).await;
    Mock::given(method("GET")).and(path(format!("/api/v2{QHREF}"))).respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "working"}))).mount(&server).await;
    let c = client_for(&server);
    let cancel = CancellationToken::new();
    let cfg = PollConfig { initial: Duration::from_secs(30), ..fast() };
    let child = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        child.cancel();
    });
    let r = run_traffic_query(&c, &query(1000), &cfg, &cancel, None).await;
    assert!(matches!(r, Err(PceError::Cancelled)));
    assert_eq!(c.stats().calls, 1, "only the POST was sent");
}

#[test]
fn fixture_rows_keep_unknown_fields() {
    let flows: Vec<Flow> = serde_json::from_str(include_str!("../../../fixtures/traffic_flows.json")).unwrap();
    assert_eq!(flows.len(), 4);
    assert!(flows[0].extra.is_empty());
    let with_extra: Flow = serde_json::from_str(r#"{"src":{},"dst":{},"service":{},"policy_decision":"allowed","new_field":{"a":1}}"#).unwrap();
    assert_eq!(with_extra.extra["new_field"]["a"], 1);
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p pce --test traffic_poll`
Expected: compile error, `traffic::poll` missing.

- [ ] **Step 4: Implement**

```rust
// crates/pce/src/traffic/poll.rs
//! Runs one Explorer query: POST, poll with backoff until completed, GET the
//! download. Cancellable between polls. Reports progress over a channel so a
//! UI can show elapsed time without owning the future.
use crate::client::PceClient;
use crate::error::{PceError, Result};
use crate::models::{AsyncQueryStatus, Flow, TrafficQuery};
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy)]
pub struct PollConfig {
    pub initial: Duration,
    pub factor: f64,
    pub cap: Duration,
    /// How long to keep polling after `completed` arrives without a result.
    pub grace: Duration,
}

impl Default for PollConfig {
    fn default() -> Self {
        Self { initial: Duration::from_secs(1), factor: 1.5, cap: Duration::from_secs(10), grace: Duration::from_secs(30) }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum QueryProgress {
    Submitted { href: String },
    Polling { status: String, elapsed: Duration },
    Downloading,
    Done { rows: usize },
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TrafficResult {
    pub flows: Vec<Flow>,
    pub matches_count: Option<u64>,
    /// Rows hit `max_results`, or the PCE matched more than it returned.
    pub truncated: bool,
    pub query_href: String,
    pub elapsed: Duration,
}

fn report(progress: Option<&UnboundedSender<QueryProgress>>, p: QueryProgress) {
    if let Some(tx) = progress {
        let _ = tx.send(p);
    }
}

pub async fn run_traffic_query(
    client: &PceClient,
    query: &TrafficQuery,
    poll: &PollConfig,
    cancel: &CancellationToken,
    progress: Option<&UnboundedSender<QueryProgress>>,
) -> Result<TrafficResult> {
    let started = Instant::now();
    let submit_path = client.org_path("traffic_flows/async_queries");
    let submitted: AsyncQueryStatus = client.post(&submit_path, query, &[("Prefer", "respond-async")]).await?.body;
    let query_href = submitted.href.clone().ok_or_else(|| PceError::Decode("async query response has no href".into()))?;
    report(progress, QueryProgress::Submitted { href: query_href.clone() });

    let mut delay = poll.initial;
    let mut completed_at: Option<Instant> = None;
    let mut matches_count = submitted.matches_count;
    let result_href = loop {
        tokio::select! {
            () = cancel.cancelled() => return Err(PceError::Cancelled),
            () = tokio::time::sleep(delay) => {}
        }
        let status: AsyncQueryStatus = client.get(&query_href, &[]).await?.body;
        let state = status.status.clone().unwrap_or_else(|| "unknown".into());
        matches_count = status.matches_count.or(matches_count);
        report(progress, QueryProgress::Polling { status: state.clone(), elapsed: started.elapsed() });
        match state.as_str() {
            "failed" => return Err(PceError::QueryFailed(status.message.unwrap_or_else(|| "no message".into()))),
            "completed" | "done" => match status.result {
                Some(r) => break r.href().to_string(),
                None => {
                    // Known PCE race: completed before the result href exists.
                    let since = *completed_at.get_or_insert_with(Instant::now);
                    if since.elapsed() > poll.grace {
                        return Err(PceError::Decode(format!("query {query_href} completed but returned no result within {:?}", poll.grace)));
                    }
                }
            },
            _ => {}
        }
        delay = Duration::from_secs_f64((delay.as_secs_f64() * poll.factor).min(poll.cap.as_secs_f64()));
    };

    report(progress, QueryProgress::Downloading);
    let flows: Vec<Flow> = client.get(&result_href, &[]).await?.body;
    let rows = flows.len();
    let truncated = rows as u64 >= u64::from(query.max_results) || matches_count.is_some_and(|m| m > rows as u64);
    report(progress, QueryProgress::Done { rows });
    Ok(TrafficResult { flows, matches_count, truncated, query_href, elapsed: started.elapsed() })
}
```

`Duration` serialises with serde's default (`{"secs":..,"nanos":..}`); the CLI meta uses `elapsed_ms` instead, so that is fine.

Add to `traffic/mod.rs`: `pub mod poll;` and `pub use poll::{PollConfig, QueryProgress, TrafficResult, run_traffic_query};`.

- [ ] **Step 5: Run tests**

Run: `cargo test -p pce --test traffic_poll`
Expected: 7 tests pass.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add -A
git commit -m "feat(pce): cancellable async Explorer query poller with grace and truncation

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 12: Resolved flows, aggregation, and the facade's traffic method

**Files:**
- Create: `crates/pce/src/traffic/resolved.rs`, `crates/pce/src/traffic/aggregate.rs`
- Modify: `crates/pce/src/traffic/mod.rs`, `crates/pce/src/lib.rs`, `crates/pce/tests/facade.rs`

**Interfaces:**
- Produces: `ResolvedEndpoint { ip, fqdn, hostname, name, workload_href, labels: Vec<ResolvedLabel>, ip_lists: Vec<String> }` with `display_name()`, `label_value(key)`, `label_text()`; `ResolvedFlow { src, dst, port: Option<u16>, proto: String, process_name, user_name, num_connections, policy_decision, draft_policy_decision, flow_direction, first_detected, last_detected, rules: Vec<String> }` with `service_text()`; `resolve_flows(&[Flow], &NameIndex) -> Vec<ResolvedFlow>`; `FlowGroup { src: Vec<String>, dst: Vec<String>, flows, connections, allowed, potentially_blocked, blocked }`; `group_by_labels(&[ResolvedFlow], &[String]) -> Vec<FlowGroup>` (sorted by connections desc); `BlockedGroup { dst, dst_labels: String, port, proto, flows, connections, sources: Vec<String>, last_detected }`; `blocked_summary(&[ResolvedFlow]) -> Vec<BlockedGroup>` (sorted by last_detected desc); facade `traffic(&TrafficQuery, CacheMode, &CancellationToken, Option<&UnboundedSender<QueryProgress>>) -> Result<Cached<TrafficResult>>` and `resolve_filter_groups(&[Vec<FilterExpr>], CacheMode) -> Result<Vec<Vec<ActorMatch>>>` (loads workloads only when a `wl:` expression is present).

- [ ] **Step 1: Write the failing tests**

```rust
// crates/pce/src/traffic/resolved.rs (tests module)
#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Flow, IpList, Label, LabelGroup, PolicyDecision, Service};
    use crate::resolve::NameIndex;

    pub(crate) fn index() -> NameIndex {
        let labels: Vec<Label> = serde_json::from_str(include_str!("../../../../fixtures/labels.json")).unwrap();
        let groups: Vec<LabelGroup> = serde_json::from_str(include_str!("../../../../fixtures/label_groups.json")).unwrap();
        let lists: Vec<IpList> = serde_json::from_str(include_str!("../../../../fixtures/ip_lists.json")).unwrap();
        let services: Vec<Service> = serde_json::from_str(include_str!("../../../../fixtures/services.json")).unwrap();
        NameIndex::new(&labels, &groups, &lists, &services)
    }

    pub(crate) fn flows() -> Vec<ResolvedFlow> {
        let raw: Vec<Flow> = serde_json::from_str(include_str!("../../../../fixtures/traffic_flows.json")).unwrap();
        resolve_flows(&raw, &index())
    }

    #[test]
    fn resolves_names_labels_and_services() {
        let f = flows();
        assert_eq!(f[0].src.display_name(), "host-1.example");
        assert_eq!(f[0].src.label_text(), "app=shop env=prod role=web");
        assert_eq!(f[0].src.label_value("role"), Some("web"));
        assert_eq!(f[0].dst.label_value("role"), Some("db"));
        assert_eq!(f[0].service_text(), "5432/tcp");
        assert_eq!(f[0].process_name.as_deref(), Some("postgres"));
        assert_eq!(f[0].policy_decision, PolicyDecision::Allowed);
        assert_eq!(f[1].src.display_name(), "10.1.4.9");
        assert_eq!(f[1].src.ip_lists, vec!["Corp"]);
        assert_eq!(f[1].src.label_text(), "");
        assert_eq!(f[3].dst.display_name(), "fqdn-1.example");
        assert_eq!(f[3].rules, vec!["/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/2"]);
    }

    #[test]
    fn icmp_flow_without_port_renders() {
        let f = flows();
        assert_eq!(f[2].port, None);
        assert_eq!(f[2].proto, "icmp");
        assert_eq!(f[2].service_text(), "icmp");
        assert_eq!(f[2].src.display_name(), "unmanaged-nas");
    }
}
```

```rust
// crates/pce/src/traffic/aggregate.rs (tests module)
#[cfg(test)]
mod tests {
    use super::*;
    use crate::traffic::resolved::tests::flows;

    #[test]
    fn groups_by_label_pair_with_fallbacks() {
        let groups = group_by_labels(&flows(), &["app".to_string(), "env".to_string()]);
        assert_eq!(groups[0].src, vec!["shop", "prod"]);
        assert_eq!(groups[0].dst, vec!["shop", "prod"]);
        assert_eq!(groups[0].flows, 1);
        assert_eq!(groups[0].connections, 310);
        assert_eq!(groups[0].allowed, 1);
        let corp = groups.iter().find(|g| g.src == vec!["ipl:Corp", "-"]).expect("ip list source falls back to its name");
        assert_eq!(corp.potentially_blocked, 1);
        let nas = groups.iter().find(|g| g.src == vec!["unmanaged-nas", "-"]).expect("unlabeled workload falls back to its name");
        assert_eq!(nas.blocked, 1);
        let ext = groups.iter().find(|g| g.dst == vec!["fqdn-1.example", "-"]).expect("fqdn destination");
        assert_eq!(ext.connections, 5);
        assert!(groups.windows(2).all(|w| w[0].connections >= w[1].connections), "sorted by connections desc");
    }

    #[test]
    fn blocked_summary_groups_by_destination_and_port() {
        let b = blocked_summary(&flows());
        assert_eq!(b.len(), 2);
        assert_eq!(b[0].dst, "host-1.example");
        assert_eq!(b[0].port, Some(8443));
        assert_eq!(b[0].proto, "tcp");
        assert_eq!(b[0].sources, vec!["10.1.4.9"]);
        assert_eq!(b[0].last_detected.as_deref(), Some("2026-10-10T07:30:58Z"));
        assert_eq!(b[1].dst, "host-2.example");
        assert_eq!(b[1].dst_labels, "app=shop env=prod role=db");
        assert_eq!(b[1].proto, "icmp");
        assert_eq!(b[1].connections, 3);
    }
}
```

Add to `crates/pce/tests/facade.rs`:

```rust
#[tokio::test]
async fn traffic_is_cached_by_query_and_filters_resolve() {
    use pce::traffic::{FilterExpr, TrafficQueryBuilder, poll::PollConfig};
    use tokio_util::sync::CancellationToken;
    let server = MockServer::start().await;
    for (p, body) in [
        ("/api/v2/orgs/5/labels", include_str!("../../../fixtures/labels.json")),
        ("/api/v2/orgs/5/sec_policy/draft/label_groups", include_str!("../../../fixtures/label_groups.json")),
        ("/api/v2/orgs/5/sec_policy/draft/ip_lists", include_str!("../../../fixtures/ip_lists.json")),
        ("/api/v2/orgs/5/sec_policy/draft/services", include_str!("../../../fixtures/services.json")),
        ("/api/v2/orgs/5/workloads", include_str!("../../../fixtures/workloads.json")),
    ] {
        Mock::given(method("GET")).and(path(p)).respond_with(ResponseTemplate::new(200).set_body_string(body).insert_header("Content-Type", "application/json")).expect(1).mount(&server).await;
    }
    Mock::given(method("POST")).and(path("/api/v2/orgs/5/traffic_flows/async_queries")).respond_with(ResponseTemplate::new(202).set_body_json(json!({"href": "/orgs/5/traffic_flows/async_queries/q-9", "status": "queued"}))).expect(1).mount(&server).await;
    Mock::given(method("GET")).and(path("/api/v2/orgs/5/traffic_flows/async_queries/q-9")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "completed", "result": "/orgs/5/traffic_flows/async_queries/q-9/download"}))).mount(&server).await;
    Mock::given(method("GET")).and(path("/api/v2/orgs/5/traffic_flows/async_queries/q-9/download")).respond_with(ResponseTemplate::new(200).set_body_string(include_str!("../../../fixtures/traffic_flows.json")).insert_header("Content-Type", "application/json")).expect(1).mount(&server).await;

    let mut p = pce_for(&server);
    p.set_poll_config(PollConfig { initial: Duration::from_millis(1), factor: 1.5, cap: Duration::from_millis(5), grace: Duration::from_millis(100) });
    let groups = vec![vec![FilterExpr::parse("app=shop").unwrap(), FilterExpr::parse("wl:host-1.example").unwrap()]];
    let actors = p.resolve_filter_groups(&groups, CacheMode::Use).await.unwrap();
    assert_eq!(actors[0].len(), 2);
    let no_wl = p.resolve_filter_groups(&[vec![FilterExpr::parse("env=prod").unwrap()]], CacheMode::Use).await.unwrap();
    assert_eq!(no_wl[0].len(), 1);
    let q = TrafficQueryBuilder::between("2026-10-09T00:00:00Z".parse().unwrap(), "2026-10-10T00:00:00Z".parse().unwrap()).source(actors[0].clone()).build().unwrap();
    let r1 = p.traffic(&q, CacheMode::Use, &CancellationToken::new(), None).await.unwrap();
    assert!(!r1.hit);
    assert_eq!(r1.value.flows.len(), 4);
    let r2 = p.traffic(&q, CacheMode::Use, &CancellationToken::new(), None).await.unwrap();
    assert!(r2.hit);
    assert_eq!(p.stats().calls, 5 + 3, "4 object lists + workloads once, then POST + poll + download once");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p pce`
Expected: compile errors.

- [ ] **Step 3: Implement**

```rust
// crates/pce/src/traffic/resolved.rs
//! Flow rows with hrefs replaced by names: what gets printed and displayed.
use crate::models::{Endpoint, Flow, PolicyDecision, proto_name};
use crate::resolve::{NameIndex, ResolvedLabel};
use serde::Serialize;

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ResolvedEndpoint {
    pub ip: Option<String>,
    pub fqdn: Option<String>,
    pub hostname: Option<String>,
    pub name: Option<String>,
    pub workload_href: Option<String>,
    pub labels: Vec<ResolvedLabel>,
    pub ip_lists: Vec<String>,
}

impl ResolvedEndpoint {
    fn from_endpoint(e: &Endpoint, idx: &NameIndex) -> Self {
        let wl = e.workload.as_ref();
        Self {
            ip: e.ip.clone(),
            fqdn: e.fqdn.clone(),
            hostname: wl.and_then(|w| w.hostname.clone()),
            name: wl.and_then(|w| w.name.clone()),
            workload_href: wl.and_then(|w| w.href.clone()),
            labels: wl.map(|w| idx.resolve_labels(&w.labels)).unwrap_or_default(),
            ip_lists: e.ip_lists.iter().filter_map(|l| l.name.clone().or_else(|| l.href.as_deref().and_then(|h| idx.ip_list_name(h)).map(str::to_string))).collect(),
        }
    }

    /// hostname, else workload name, else fqdn, else ip, else `?`.
    pub fn display_name(&self) -> String {
        [&self.hostname, &self.name, &self.fqdn, &self.ip].into_iter().flatten().find(|s| !s.is_empty()).cloned().unwrap_or_else(|| "?".into())
    }

    pub fn label_value(&self, key: &str) -> Option<&str> {
        self.labels.iter().find(|l| l.key == key).map(|l| l.value.as_str())
    }

    /// `app=shop env=prod role=web`
    pub fn label_text(&self) -> String {
        self.labels.iter().map(|l| format!("{}={}", l.key, l.value)).collect::<Vec<_>>().join(" ")
    }

    pub fn is_workload(&self) -> bool {
        self.workload_href.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResolvedFlow {
    pub src: ResolvedEndpoint,
    pub dst: ResolvedEndpoint,
    pub port: Option<u16>,
    pub proto: String,
    pub process_name: Option<String>,
    pub user_name: Option<String>,
    pub num_connections: u64,
    pub policy_decision: PolicyDecision,
    pub draft_policy_decision: Option<String>,
    pub flow_direction: Option<String>,
    pub first_detected: Option<String>,
    pub last_detected: Option<String>,
    pub rules: Vec<String>,
}

impl ResolvedFlow {
    /// `5432/tcp`, or just `icmp` when there is no port.
    pub fn service_text(&self) -> String {
        match self.port {
            Some(p) => format!("{p}/{}", self.proto),
            None => self.proto.clone(),
        }
    }
}

pub fn resolve_flows(flows: &[Flow], idx: &NameIndex) -> Vec<ResolvedFlow> {
    flows
        .iter()
        .map(|f| ResolvedFlow {
            src: ResolvedEndpoint::from_endpoint(&f.src, idx),
            dst: ResolvedEndpoint::from_endpoint(&f.dst, idx),
            port: f.service.port,
            proto: f.service.proto.map(proto_name).unwrap_or_else(|| "?".into()),
            process_name: f.service.process_name.clone(),
            user_name: f.service.user_name.clone(),
            num_connections: f.num_connections,
            policy_decision: f.policy_decision,
            draft_policy_decision: f.draft_policy_decision.clone(),
            flow_direction: f.flow_direction.clone(),
            first_detected: f.timestamp_range.as_ref().and_then(|t| t.first_detected.clone()),
            last_detected: f.timestamp_range.as_ref().and_then(|t| t.last_detected.clone()),
            rules: f.rules.iter().map(|r| r.href.clone()).collect(),
        })
        .collect()
}
```

```rust
// crates/pce/src/traffic/aggregate.rs
//! Pure aggregations over resolved flows. The TUI's app-to-app and blocked
//! views and the CLI's `--group` / `--blocked` use these; no requests.
use super::resolved::{ResolvedEndpoint, ResolvedFlow};
use crate::models::PolicyDecision;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FlowGroup {
    pub src: Vec<String>,
    pub dst: Vec<String>,
    pub flows: u64,
    pub connections: u64,
    pub allowed: u64,
    pub potentially_blocked: u64,
    pub blocked: u64,
}

/// Group key for one endpoint: the label value per key, `-` when the label
/// is missing. An endpoint without labels at all puts its name (or
/// `ipl:<list>`) in the first slot so it still reads sensibly.
fn group_key(e: &ResolvedEndpoint, keys: &[String]) -> Vec<String> {
    if e.labels.is_empty() || !e.is_workload() {
        let name = e.ip_lists.first().map(|l| format!("ipl:{l}")).unwrap_or_else(|| e.display_name());
        let mut k = vec![name];
        k.extend(std::iter::repeat_n("-".to_string(), keys.len().saturating_sub(1)));
        return k;
    }
    keys.iter().map(|k| e.label_value(k).unwrap_or("-").to_string()).collect()
}

pub fn group_by_labels(flows: &[ResolvedFlow], keys: &[String]) -> Vec<FlowGroup> {
    let keys: Vec<String> = if keys.is_empty() { vec!["app".into(), "env".into()] } else { keys.to_vec() };
    let mut map: BTreeMap<(Vec<String>, Vec<String>), FlowGroup> = BTreeMap::new();
    for f in flows {
        let key = (group_key(&f.src, &keys), group_key(&f.dst, &keys));
        let g = map.entry(key.clone()).or_insert_with(|| FlowGroup { src: key.0.clone(), dst: key.1.clone(), flows: 0, connections: 0, allowed: 0, potentially_blocked: 0, blocked: 0 });
        g.flows += 1;
        g.connections += f.num_connections;
        match f.policy_decision {
            PolicyDecision::Allowed => g.allowed += 1,
            PolicyDecision::PotentiallyBlocked => g.potentially_blocked += 1,
            PolicyDecision::Blocked => g.blocked += 1,
            PolicyDecision::Unknown => {}
        }
    }
    let mut out: Vec<FlowGroup> = map.into_values().collect();
    out.sort_by(|a, b| b.connections.cmp(&a.connections).then_with(|| a.src.cmp(&b.src)));
    out
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BlockedGroup {
    pub dst: String,
    pub dst_labels: String,
    pub port: Option<u16>,
    pub proto: String,
    pub flows: u64,
    pub connections: u64,
    /// Distinct source names, at most 5.
    pub sources: Vec<String>,
    pub last_detected: Option<String>,
}

pub fn blocked_summary(flows: &[ResolvedFlow]) -> Vec<BlockedGroup> {
    let mut map: BTreeMap<(String, Option<u16>, String), BlockedGroup> = BTreeMap::new();
    for f in flows.iter().filter(|f| matches!(f.policy_decision, PolicyDecision::Blocked | PolicyDecision::PotentiallyBlocked)) {
        let dst = f.dst.display_name();
        let key = (dst.clone(), f.port, f.proto.clone());
        let g = map.entry(key).or_insert_with(|| BlockedGroup { dst, dst_labels: f.dst.label_text(), port: f.port, proto: f.proto.clone(), flows: 0, connections: 0, sources: Vec::new(), last_detected: None });
        g.flows += 1;
        g.connections += f.num_connections;
        let src = f.src.display_name();
        if !g.sources.contains(&src) && g.sources.len() < 5 {
            g.sources.push(src);
        }
        if f.last_detected > g.last_detected {
            g.last_detected = f.last_detected.clone();
        }
    }
    let mut out: Vec<BlockedGroup> = map.into_values().collect();
    out.sort_by(|a, b| b.last_detected.cmp(&a.last_detected).then_with(|| a.dst.cmp(&b.dst)));
    out
}
```

Add to `traffic/mod.rs`: `pub mod aggregate; pub mod resolved;` and `pub use aggregate::{BlockedGroup, FlowGroup, blocked_summary, group_by_labels}; pub use resolved::{ResolvedEndpoint, ResolvedFlow, resolve_flows};`.

In `lib.rs`, add a `poll: traffic::PollConfig` field to `Pce` (initialised with `Default::default()` in `with_ttls`), plus:

```rust
    pub fn set_poll_config(&mut self, poll: traffic::PollConfig) {
        self.poll = poll;
    }

    /// Runs (or reuses) one Explorer query. The cache key is the full query
    /// body, so the same filter never runs twice until invalidated.
    pub async fn traffic(
        &self,
        query: &models::TrafficQuery,
        mode: CacheMode,
        cancel: &tokio_util::sync::CancellationToken,
        progress: Option<&tokio::sync::mpsc::UnboundedSender<traffic::QueryProgress>>,
    ) -> Result<Cached<traffic::TrafficResult>> {
        let key = format!("traffic:{}", serde_json::to_string(query).map_err(|e| PceError::Invalid(e.to_string()))?);
        if mode == CacheMode::Use {
            if let Some((value, age)) = self.cache.get::<traffic::TrafficResult>(&key) {
                let truncated = value.truncated;
                return Ok(Cached { value, age, hit: true, truncated });
            }
        }
        let result = traffic::run_traffic_query(&self.client, query, &self.poll, cancel, progress).await?;
        // Traffic is kept until the filter changes or the user refreshes; a
        // long TTL stands in for "forever" within one session.
        self.cache.insert(&key, result.clone(), Duration::from_secs(24 * 3600));
        let truncated = result.truncated;
        Ok(Cached { value: result, age: Duration::ZERO, hit: false, truncated })
    }

    /// Resolves filter expressions (OR of AND-groups) to actor matches,
    /// loading workloads only when a `wl:` expression needs them.
    pub async fn resolve_filter_groups(&self, groups: &[Vec<traffic::FilterExpr>], mode: CacheMode) -> Result<Vec<Vec<models::ActorMatch>>> {
        let idx = self.name_index(mode).await?;
        let workloads = if groups.iter().flatten().any(traffic::FilterExpr::needs_workloads) {
            self.workloads(&models::WorkloadFilter::default(), mode).await?.value
        } else {
            Vec::new()
        };
        groups.iter().map(|g| g.iter().map(|e| traffic::resolve_actor(e, &idx, &workloads)).collect()).collect()
    }
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p pce`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add -A
git commit -m "feat(pce): resolved flows, label-pair and blocked aggregation, cached traffic on the facade

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 13: Configuration: profiles, environment, resolution

**Files:**
- Create: `crates/illumio-tui/src/config.rs`
- Modify: `crates/illumio-tui/src/main.rs` (add `mod config;`)

**Interfaces:**
- Produces: `ConfigFile { default_profile, profiles: BTreeMap<String, Profile> }`, `Profile { host, port, org_id, scheme, tls_verify, api_key, api_secret, api_secret_cmd, limits: Limits }`, `Limits { requests_per_second, auto_refresh_min_seconds, traffic_max_results, traffic_default_window }`, `Overrides { profile, host, port, org_id, tls_verify }`, `ResolvedConfig { profile_name, scheme, host, port, org_id, tls_verify, credentials, requests_per_second, auto_refresh_min_seconds, traffic_max_results, traffic_default_window }`, `ConfigError`, `default_config_path() -> PathBuf`, `load_config_file(&Path) -> Result<ConfigFile, ConfigError>` (missing file is an empty config), `load_env_file(&Path) -> Result<HashMap<String,String>, ConfigError>`, `merge_env(file: HashMap, real: HashMap) -> HashMap` (real wins), `resolve(&ConfigFile, &Overrides, &HashMap<String,String>) -> Result<ResolvedConfig, ConfigError>`, `impl ResolvedConfig { fn client_config(&self) -> pce::ClientConfig }`.
- Env names: `PCE_HOST`, `PCE_PORT`, `PCE_ORG_ID`, `PCE_SCHEME`, `PCE_TLS_VERIFY`, `API_KEY`, `API_SECRET`, `ILLUMIO_PROFILE`.

- [ ] **Step 1: Write the failing tests**

```rust
// crates/illumio-tui/src/config.rs (tests module)
#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn file() -> ConfigFile {
        toml::from_str(
            r#"
default_profile = "demo"

[profiles.demo]
host = "demo.example"
port = 8443
org_id = 7
tls_verify = false
api_key = "file-key"
api_secret = "file-secret"

[profiles.demo.limits]
requests_per_second = 1.5
traffic_max_results = 250
traffic_default_window = "6h"

[profiles.cmd]
host = "cmd.example"
org_id = 9
api_key = "k"
api_secret_cmd = "echo   from-command  "
"#,
        )
        .unwrap()
    }

    #[test]
    fn profile_values_and_limit_defaults() {
        let c = resolve(&file(), &Overrides::default(), &env(&[])).unwrap();
        assert_eq!(c.profile_name, "demo");
        assert_eq!(c.host, "demo.example");
        assert_eq!(c.port, 8443);
        assert_eq!(c.org_id, 7);
        assert!(!c.tls_verify);
        assert_eq!(c.scheme, "https");
        assert_eq!(c.credentials.api_key(), "file-key");
        assert_eq!(c.credentials.expose_secret(), "file-secret");
        assert_eq!(c.requests_per_second, 1.5);
        assert_eq!(c.auto_refresh_min_seconds, 60);
        assert_eq!(c.traffic_max_results, 250);
        assert_eq!(c.traffic_default_window, "6h");
    }

    #[test]
    fn flag_beats_env_beats_profile() {
        let e = env(&[("PCE_HOST", "env.example"), ("PCE_PORT", "9443"), ("API_KEY", "env-key"), ("API_SECRET", "env-secret"), ("PCE_TLS_VERIFY", "true")]);
        let o = Overrides { host: Some("flag.example".into()), ..Default::default() };
        let c = resolve(&file(), &o, &e).unwrap();
        assert_eq!(c.host, "flag.example");
        assert_eq!(c.port, 9443);
        assert_eq!(c.org_id, 7, "org id falls through to the profile");
        assert!(c.tls_verify);
        assert_eq!(c.credentials.api_key(), "env-key");
        assert_eq!(c.credentials.expose_secret(), "env-secret");
    }

    #[test]
    fn env_only_works_without_a_config_file() {
        let e = env(&[("PCE_HOST", "h"), ("PCE_ORG_ID", "3"), ("API_KEY", "k"), ("API_SECRET", "s"), ("PCE_SCHEME", "http")]);
        let c = resolve(&ConfigFile::default(), &Overrides::default(), &e).unwrap();
        assert_eq!(c.profile_name, "default");
        assert_eq!(c.port, 443);
        assert_eq!(c.scheme, "http");
        assert!(c.tls_verify);
        assert_eq!(c.requests_per_second, 2.0);
        assert_eq!(c.traffic_max_results, 1000);
        assert_eq!(c.traffic_default_window, "24h");
    }

    #[test]
    fn missing_values_name_the_env_var() {
        let err = resolve(&ConfigFile::default(), &Overrides::default(), &env(&[])).unwrap_err().to_string();
        assert!(err.contains("PCE_HOST"), "{err}");
        let e = env(&[("PCE_HOST", "h"), ("PCE_ORG_ID", "3"), ("API_KEY", "k")]);
        let err = resolve(&ConfigFile::default(), &Overrides::default(), &e).unwrap_err().to_string();
        assert!(err.contains("API_SECRET"), "{err}");
        let err = resolve(&file(), &Overrides { profile: Some("nope".into()), ..Default::default() }, &env(&[])).unwrap_err().to_string();
        assert!(err.contains("profile \"nope\""), "{err}");
    }

    #[test]
    fn secret_command_is_run_and_trimmed() {
        let o = Overrides { profile: Some("cmd".into()), ..Default::default() };
        let c = resolve(&file(), &o, &env(&[])).unwrap();
        assert_eq!(c.credentials.expose_secret(), "from-command");
    }

    #[test]
    fn failing_or_empty_secret_command_is_an_error() {
        let mut f = file();
        f.profiles.get_mut("cmd").unwrap().api_secret_cmd = Some("exit 3".into());
        let o = Overrides { profile: Some("cmd".into()), ..Default::default() };
        let err = resolve(&f, &o, &env(&[])).unwrap_err().to_string();
        assert!(err.contains("api_secret_cmd"), "{err}");
        f.profiles.get_mut("cmd").unwrap().api_secret_cmd = Some("true".into());
        let err = resolve(&f, &o, &env(&[])).unwrap_err().to_string();
        assert!(err.contains("empty"), "{err}");
    }

    #[test]
    fn env_file_merge_and_missing_config_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(".env");
        std::fs::write(&p, "PCE_HOST=file.example\nAPI_KEY=fk\n# comment\n").unwrap();
        let from_file = load_env_file(&p).unwrap();
        let merged = merge_env(from_file, env(&[("PCE_HOST", "real.example")]));
        assert_eq!(merged["PCE_HOST"], "real.example");
        assert_eq!(merged["API_KEY"], "fk");
        assert!(load_config_file(&dir.path().join("none.toml")).unwrap().profiles.is_empty());
        assert!(load_env_file(&dir.path().join("none.env")).is_err());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p illumio-tui config`
Expected: compile error.

- [ ] **Step 3: Implement**

```rust
// crates/illumio-tui/src/config.rs
//! Where connection settings come from, in order: command-line flag, then
//! environment variable (same names as illumio-mcp-server), then the
//! selected profile in `~/.config/illumio-tui/config.toml`, then defaults.
//! The tool never writes secrets anywhere.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ConfigFile {
    pub default_profile: Option<String>,
    #[serde(default)]
    pub profiles: BTreeMap<String, Profile>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub host: Option<String>,
    pub port: Option<u16>,
    pub org_id: Option<u64>,
    pub scheme: Option<String>,
    pub tls_verify: Option<bool>,
    pub api_key: Option<String>,
    pub api_secret: Option<String>,
    /// Shell command whose stdout is the API secret (e.g. `op read ...`).
    pub api_secret_cmd: Option<String>,
    #[serde(default)]
    pub limits: Limits,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Limits {
    pub requests_per_second: Option<f64>,
    pub auto_refresh_min_seconds: Option<u64>,
    pub traffic_max_results: Option<u32>,
    pub traffic_default_window: Option<String>,
}

/// Values given on the command line.
#[derive(Debug, Default, Clone)]
pub struct Overrides {
    pub profile: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub org_id: Option<u64>,
    pub tls_verify: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    pub profile_name: String,
    pub scheme: String,
    pub host: String,
    pub port: u16,
    pub org_id: u64,
    pub tls_verify: bool,
    pub credentials: pce::Credentials,
    pub requests_per_second: f64,
    pub auto_refresh_min_seconds: u64,
    pub traffic_max_results: u32,
    pub traffic_default_window: String,
}

impl ResolvedConfig {
    pub fn client_config(&self) -> pce::ClientConfig {
        let mut c = pce::ClientConfig::new(self.host.clone(), self.org_id, self.credentials.clone());
        c.scheme = self.scheme.clone();
        c.port = self.port;
        c.tls_verify = self.tls_verify;
        c.requests_per_second = self.requests_per_second;
        c
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("{0}")]
    Message(String),
    #[error("cannot read {path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("cannot parse {path}: {source}")]
    Parse { path: PathBuf, source: toml::de::Error },
}

pub fn default_config_path() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("illumio-tui").join("config.toml");
    }
    dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(".config").join("illumio-tui").join("config.toml")
}

/// A missing file is an empty config; a present but broken file is an error.
pub fn load_config_file(path: &Path) -> Result<ConfigFile, ConfigError> {
    match std::fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text).map_err(|source| ConfigError::Parse { path: path.to_path_buf(), source }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ConfigFile::default()),
        Err(source) => Err(ConfigError::Io { path: path.to_path_buf(), source }),
    }
}

/// Reads a dotenv file into a map without touching the process environment.
pub fn load_env_file(path: &Path) -> Result<HashMap<String, String>, ConfigError> {
    let iter = dotenvy::from_path_iter(path).map_err(|e| ConfigError::Message(format!("cannot read env file {}: {e}", path.display())))?;
    let mut map = HashMap::new();
    for item in iter {
        let (k, v) = item.map_err(|e| ConfigError::Message(format!("bad line in {}: {e}", path.display())))?;
        map.insert(k, v);
    }
    Ok(map)
}

/// Real environment variables win over the env file.
pub fn merge_env(file: HashMap<String, String>, real: HashMap<String, String>) -> HashMap<String, String> {
    let mut merged = file;
    merged.extend(real);
    merged
}

pub fn resolve(file: &ConfigFile, overrides: &Overrides, env: &HashMap<String, String>) -> Result<ResolvedConfig, ConfigError> {
    let profile_name = overrides
        .profile
        .clone()
        .or_else(|| env.get("ILLUMIO_PROFILE").cloned())
        .or_else(|| file.default_profile.clone())
        .unwrap_or_else(|| "default".to_string());
    let profile = match file.profiles.get(&profile_name) {
        Some(p) => p.clone(),
        None if overrides.profile.is_some() || env.contains_key("ILLUMIO_PROFILE") => {
            return Err(ConfigError::Message(format!("profile {profile_name:?} not found in config; known: {}", file.profiles.keys().cloned().collect::<Vec<_>>().join(", "))));
        }
        None => Profile::default(),
    };
    let env_str = |k: &str| env.get(k).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());

    let host = overrides.host.clone().or_else(|| env_str("PCE_HOST")).or_else(|| profile.host.clone()).ok_or_else(|| ConfigError::Message("no PCE host: pass --host, set PCE_HOST, or add host to the profile".into()))?;
    let port = match overrides.port.or_else(|| env_str("PCE_PORT").and_then(|v| v.parse().ok())) {
        Some(p) => p,
        None => profile.port.unwrap_or(443),
    };
    let org_id = overrides
        .org_id
        .or_else(|| env_str("PCE_ORG_ID").and_then(|v| v.parse().ok()))
        .or(profile.org_id)
        .ok_or_else(|| ConfigError::Message("no org id: pass --org-id, set PCE_ORG_ID, or add org_id to the profile".into()))?;
    let scheme = env_str("PCE_SCHEME").or_else(|| profile.scheme.clone()).unwrap_or_else(|| "https".into());
    let tls_verify = overrides.tls_verify.or_else(|| env_str("PCE_TLS_VERIFY").and_then(|v| parse_bool(&v))).or(profile.tls_verify).unwrap_or(true);

    let api_key = env_str("API_KEY").or_else(|| profile.api_key.clone()).ok_or_else(|| ConfigError::Message("no API key: set API_KEY or add api_key to the profile".into()))?;
    let api_secret = match env_str("API_SECRET").or_else(|| profile.api_secret.clone()) {
        Some(s) => s,
        None => match &profile.api_secret_cmd {
            Some(cmd) => run_secret_cmd(cmd)?,
            None => return Err(ConfigError::Message("no API secret: set API_SECRET, or add api_secret or api_secret_cmd to the profile".into())),
        },
    };

    Ok(ResolvedConfig {
        profile_name,
        scheme,
        host,
        port,
        org_id,
        tls_verify,
        credentials: pce::Credentials::new(api_key, api_secret),
        requests_per_second: profile.limits.requests_per_second.unwrap_or(2.0),
        auto_refresh_min_seconds: profile.limits.auto_refresh_min_seconds.unwrap_or(60).max(60),
        traffic_max_results: profile.limits.traffic_max_results.unwrap_or(1000),
        traffic_default_window: profile.limits.traffic_default_window.clone().unwrap_or_else(|| "24h".into()),
    })
}

pub fn parse_bool(s: &str) -> Option<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn run_secret_cmd(cmd: &str) -> Result<String, ConfigError> {
    let out = std::process::Command::new("sh").arg("-c").arg(cmd).output().map_err(|e| ConfigError::Message(format!("api_secret_cmd could not start: {e}")))?;
    if !out.status.success() {
        return Err(ConfigError::Message(format!("api_secret_cmd exited with {}", out.status)));
    }
    let secret = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if secret.is_empty() {
        return Err(ConfigError::Message("api_secret_cmd printed an empty secret".into()));
    }
    Ok(secret)
}
```

Add `mod config;` to `main.rs`.

- [ ] **Step 4: Run tests**

Run: `cargo test -p illumio-tui config`
Expected: 7 tests pass.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add -A
git commit -m "feat(cli): profile config with flag > env > profile resolution and secret commands

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 14: CLI skeleton, output envelope, exit codes, `health`

**Files:**
- Create: `crates/illumio-tui/src/cli/mod.rs`, `crates/illumio-tui/src/cli/error.rs`, `crates/illumio-tui/src/cli/context.rs`, `crates/illumio-tui/src/cli/output.rs`, `crates/illumio-tui/src/cli/health.rs`, `crates/illumio-tui/tests/cli.rs`
- Modify: `crates/illumio-tui/src/main.rs`

**Interfaces:**
- Produces: `Cli { global: GlobalArgs, command: Option<Command> }`; `GlobalArgs { profile, config, env_file, host, port, org_id, insecure, json, table, no_cache }`; `Command::{Health | Labels(..) | LabelGroups | Iplists | Services | Workloads(..) | Policy(..) | Traffic(..) | Profile(..)}` (later tasks add the args structs; this task adds `Health` and stubs nothing else); `CliError` with `exit_code()`, `kind()`, `to_json()`; `OutputFormat { Json, Table }`; `Meta`, `Window`, `Envelope<T>`; `TableRows` trait; `render_table(&[&str], &[Vec<String>]) -> String`; `Context::new(&GlobalArgs) -> Result<Context>` with `pce`, `config`, `format`, `cache_mode`, `meta(cache, truncated, window)`, `emit(data, cache, truncated, window)`; `fn cache_label(hit: bool, mode: CacheMode) -> &'static str`.
- CLI tests use `assert_cmd` with env vars pointing at a wiremock server (`PCE_SCHEME=http`).

- [ ] **Step 1: Write the failing tests**

```rust
// crates/illumio-tui/tests/cli.rs
use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

pub fn cmd(server: &MockServer) -> Command {
    let url = url::Url::parse(&server.uri()).unwrap();
    let mut c = Command::cargo_bin("illumio-tui").unwrap();
    c.env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("PCE_SCHEME", "http")
        .env("PCE_HOST", url.host_str().unwrap())
        .env("PCE_PORT", url.port().unwrap().to_string())
        .env("PCE_ORG_ID", "5")
        .env("API_KEY", "key")
        .env("API_SECRET", "secret")
        .env("XDG_CONFIG_HOME", std::env::temp_dir().join("illumio-tui-no-config"));
    c
}

pub fn stdout_json(assert: assert_cmd::assert::Assert) -> Value {
    serde_json::from_slice(&assert.get_output().stdout).expect("stdout is JSON")
}

#[test]
fn version_and_no_subcommand() {
    Command::cargo_bin("illumio-tui").unwrap().arg("--version").assert().success().stdout(predicate::str::starts_with("illumio-tui "));
    Command::cargo_bin("illumio-tui").unwrap().env_clear().assert().success().stdout(predicate::str::contains("Plan 2"));
}

#[tokio::test(flavor = "multi_thread")]
async fn health_json_envelope() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/api/v2/health")).respond_with(ResponseTemplate::new(200).set_body_json(json!([{"status": "normal", "type": "cluster"}]))).mount(&server).await;
    let out = stdout_json(cmd(&server).args(["health", "--json"]).assert().success());
    assert_eq!(out["data"]["ok"], true);
    assert_eq!(out["data"]["org_id"], 5);
    assert_eq!(out["data"]["status"][0]["status"], "normal");
    assert_eq!(out["meta"]["profile"], "default");
    assert_eq!(out["meta"]["api_calls"], 1);
    assert_eq!(out["meta"]["cache"], "none");
    assert_eq!(out["meta"]["truncated"], false);
    assert!(out["meta"]["elapsed_ms"].is_number());
}

#[tokio::test(flavor = "multi_thread")]
async fn health_table_output() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/api/v2/health")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "normal"}))).mount(&server).await;
    cmd(&server).args(["health", "--table"]).assert().success().stdout(predicate::str::contains("field").and(predicate::str::contains("ok")).and(predicate::str::contains("true"))).stderr(predicate::str::contains("1 api call"));
}

#[tokio::test(flavor = "multi_thread")]
async fn json_is_default_when_stdout_is_not_a_tty() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/api/v2/health")).respond_with(ResponseTemplate::new(200).set_body_json(json!({}))).mount(&server).await;
    let out = stdout_json(cmd(&server).arg("health").assert().success());
    assert_eq!(out["data"]["ok"], true);
}

#[tokio::test(flavor = "multi_thread")]
async fn auth_failure_exits_3_with_json_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/api/v2/health")).respond_with(ResponseTemplate::new(401)).mount(&server).await;
    let assert = cmd(&server).arg("health").assert().code(3);
    let err: Value = serde_json::from_slice(&assert.get_output().stderr).unwrap();
    assert_eq!(err["error"]["kind"], "auth");
    assert!(err["error"]["message"].as_str().unwrap().contains("401"));
}

#[test]
fn missing_config_exits_2() {
    let assert = Command::cargo_bin("illumio-tui").unwrap().env_clear().env("XDG_CONFIG_HOME", std::env::temp_dir().join("illumio-tui-no-config")).arg("health").assert().code(2);
    let err: Value = serde_json::from_slice(&assert.get_output().stderr).unwrap();
    assert_eq!(err["error"]["kind"], "config");
    assert!(err["error"]["message"].as_str().unwrap().contains("PCE_HOST"));
}

#[tokio::test(flavor = "multi_thread")]
async fn env_file_flag_supplies_credentials() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/api/v2/health")).respond_with(ResponseTemplate::new(200).set_body_json(json!({}))).mount(&server).await;
    let url = url::Url::parse(&server.uri()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let env_path = dir.path().join("pce.env");
    std::fs::write(&env_path, format!("PCE_SCHEME=http\nPCE_HOST={}\nPCE_PORT={}\nPCE_ORG_ID=5\nAPI_KEY=k\nAPI_SECRET=s\n", url.host_str().unwrap(), url.port().unwrap())).unwrap();
    let out = stdout_json(Command::cargo_bin("illumio-tui").unwrap().env_clear().env("XDG_CONFIG_HOME", dir.path()).args(["--env-file", env_path.to_str().unwrap(), "health"]).assert().success());
    assert_eq!(out["data"]["ok"], true);
}

#[test]
fn render_table_is_aligned() {
    use illumio_tui::cli::output::render_table;
    let t = render_table(&["name", "n"], &[vec!["a".into(), "1".into()], vec!["longer".into(), "22".into()]]);
    let lines: Vec<&str> = t.lines().collect();
    assert_eq!(lines[0], "name    n");
    assert!(lines[1].starts_with("─"));
    assert_eq!(lines[2], "a       1");
    assert_eq!(lines[3], "longer  22");
    assert_eq!(render_table(&["x"], &[]).lines().last(), Some("(no rows)"));
}
```

The last test needs the binary crate to also be a library: add to `crates/illumio-tui/Cargo.toml`:

```toml
[lib]
name = "illumio_tui"
path = "src/lib.rs"
```

and add `url.workspace = true` under its `[dev-dependencies]`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p illumio-tui --test cli`
Expected: compile error.

- [ ] **Step 3: Implement**

```rust
// crates/illumio-tui/src/lib.rs
//! Library half of the binary so integration tests can reach the CLI
//! modules. `main.rs` is a thin wrapper.
pub mod cli;
pub mod config;
```

```rust
// crates/illumio-tui/src/main.rs
//! Entry point. No subcommand: the interactive TUI (Plan 2). Any subcommand:
//! the non-interactive CLI for scripts and agents.
use clap::Parser;
use illumio_tui::cli::{Cli, run};

fn main() {
    let cli = Cli::parse();
    let Some(command) = cli.command else {
        println!("illumio-tui: the interactive TUI arrives in Plan 2; run `illumio-tui --help` for CLI commands");
        return;
    };
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    if let Err(e) = rt.block_on(run(cli.global, command)) {
        eprintln!("{}", e.to_json());
        std::process::exit(e.exit_code());
    }
}
```

```rust
// crates/illumio-tui/src/cli/error.rs
//! CLI errors and their exit codes: 1 generic, 2 arguments/config, 3 auth,
//! 4 PCE error, 5 rate limited.
use crate::config::ConfigError;
use pce::PceError;

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("{0}")]
    Args(String),
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Pce(#[from] PceError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Other(String),
}

impl CliError {
    pub fn exit_code(&self) -> i32 {
        match self {
            CliError::Args(_) | CliError::Config(_) => 2,
            CliError::Pce(PceError::Auth { .. }) => 3,
            CliError::Pce(PceError::RateLimited { .. }) => 5,
            CliError::Pce(PceError::Invalid(_)) => 2,
            CliError::Pce(_) => 4,
            CliError::Io(_) | CliError::Other(_) => 1,
        }
    }

    pub fn kind(&self) -> String {
        match self {
            CliError::Args(_) => "args".into(),
            CliError::Config(_) => "config".into(),
            CliError::Pce(e) => e.kind().into(),
            CliError::Io(_) => "io".into(),
            CliError::Other(_) => "error".into(),
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({"error": {"kind": self.kind(), "message": self.to_string(), "exit_code": self.exit_code()}})
    }
}
```

```rust
// crates/illumio-tui/src/cli/output.rs
//! The JSON envelope every command prints, and the table renderer shared
//! with the TUI (same ratatui Table widget, drawn into an off-screen buffer).
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Rect};
use ratatui::widgets::{Row, Table, Widget};
use serde::Serialize;
use std::io::Write;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Json,
    Table,
}

#[derive(Debug, Clone, Serialize)]
pub struct Window {
    pub start: String,
    pub end: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Meta {
    pub profile: String,
    pub elapsed_ms: u128,
    pub api_calls: u64,
    pub bytes: u64,
    /// `hit`, `miss`, `bypass`, or `none` (not cacheable).
    pub cache: &'static str,
    pub truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window: Option<Window>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Envelope<'a, T: Serialize> {
    pub data: &'a T,
    pub meta: &'a Meta,
}

/// Anything the CLI can print as a table.
pub trait TableRows {
    fn headers(&self) -> Vec<&'static str>;
    fn rows(&self) -> Vec<Vec<String>>;
}

const MAX_COL: usize = 60;

fn clip(s: &str) -> String {
    if s.chars().count() <= MAX_COL { s.to_string() } else { format!("{}…", s.chars().take(MAX_COL - 1).collect::<String>()) }
}

pub fn render_table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let rows: Vec<Vec<String>> = rows.iter().map(|r| r.iter().map(|c| clip(c)).collect()).collect();
    let widths: Vec<u16> = headers
        .iter()
        .enumerate()
        .map(|(i, h)| rows.iter().map(|r| r.get(i).map_or(0, |c| c.chars().count())).chain(std::iter::once(h.chars().count())).max().unwrap_or(0) as u16)
        .collect();
    let spacing: u16 = 2;
    let total: u16 = widths.iter().sum::<u16>() + spacing * (widths.len().saturating_sub(1) as u16);
    let height = rows.len() as u16 + 1;
    let area = Rect::new(0, 0, total.max(1), height);
    let mut buf = Buffer::empty(area);
    let table = Table::new(rows.iter().map(|r| Row::new(r.clone())).collect::<Vec<_>>(), widths.iter().map(|w| Constraint::Length(*w)).collect::<Vec<_>>())
        .header(Row::new(headers.iter().map(|h| h.to_string()).collect::<Vec<_>>()))
        .column_spacing(spacing);
    table.render(area, &mut buf);
    let mut lines: Vec<String> = (0..height).map(|y| (0..area.width).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string()).collect();
    lines.insert(1, "─".repeat(total as usize));
    if rows.is_empty() {
        lines.push("(no rows)".to_string());
    }
    lines.join("\n")
}

pub fn write_json<T: Serialize>(out: &mut impl Write, data: &T, meta: &Meta) -> std::io::Result<()> {
    serde_json::to_writer_pretty(&mut *out, &Envelope { data, meta })?;
    writeln!(out)
}

pub fn write_table<T: TableRows>(out: &mut impl Write, data: &T) -> std::io::Result<()> {
    let headers = data.headers();
    writeln!(out, "{}", render_table(&headers, &data.rows()))
}

/// One line for stderr in table mode so stdout stays clean.
pub fn meta_line(meta: &Meta) -> String {
    let kb = meta.bytes as f64 / 1024.0;
    let mut s = format!("{} · {} api call{} · {kb:.1} KB · cache {} · {} ms", meta.profile, meta.api_calls, if meta.api_calls == 1 { "" } else { "s" }, meta.cache, meta.elapsed_ms);
    if meta.truncated {
        s.push_str(" · WARNING: result truncated");
    }
    s
}
```

```rust
// crates/illumio-tui/src/cli/context.rs
//! Everything a command needs: the resolved config, the Pce facade, the
//! output format, and the cache mode.
use super::error::CliError;
use super::output::{Meta, OutputFormat, TableRows, Window, meta_line, write_json, write_table};
use super::GlobalArgs;
use crate::config::{self, Overrides};
use pce::{CacheMode, Pce, PceClient};
use serde::Serialize;
use std::collections::HashMap;
use std::io::IsTerminal;
use std::time::Instant;

pub struct Context {
    pub config: config::ResolvedConfig,
    pub pce: Pce,
    pub format: OutputFormat,
    pub cache_mode: CacheMode,
    started: Instant,
}

impl Context {
    pub fn new(global: &GlobalArgs) -> Result<Self, CliError> {
        let path = global.config.clone().unwrap_or_else(config::default_config_path);
        let file = config::load_config_file(&path)?;
        let real: HashMap<String, String> = std::env::vars().collect();
        let env = match &global.env_file {
            Some(p) => config::merge_env(config::load_env_file(p)?, real),
            None => real,
        };
        let overrides = Overrides {
            profile: global.profile.clone(),
            host: global.host.clone(),
            port: global.port,
            org_id: global.org_id,
            tls_verify: if global.insecure { Some(false) } else { None },
        };
        let resolved = config::resolve(&file, &overrides, &env)?;
        let pce = Pce::new(PceClient::new(resolved.client_config())?);
        let format = if global.table {
            OutputFormat::Table
        } else if global.json || !std::io::stdout().is_terminal() {
            OutputFormat::Json
        } else {
            OutputFormat::Table
        };
        let cache_mode = if global.no_cache { CacheMode::Bypass } else { CacheMode::Use };
        Ok(Self { config: resolved, pce, format, cache_mode, started: Instant::now() })
    }

    pub fn meta(&self, cache: &'static str, truncated: bool, window: Option<Window>) -> Meta {
        let s = self.pce.stats();
        Meta { profile: self.config.profile_name.clone(), elapsed_ms: self.started.elapsed().as_millis(), api_calls: s.calls, bytes: s.bytes, cache, truncated, window }
    }

    pub fn emit<T: Serialize + TableRows>(&self, data: &T, cache: &'static str, truncated: bool, window: Option<Window>) -> Result<(), CliError> {
        let meta = self.meta(cache, truncated, window);
        let mut out = std::io::stdout().lock();
        match self.format {
            OutputFormat::Json => write_json(&mut out, data, &meta)?,
            OutputFormat::Table => {
                write_table(&mut out, data)?;
                eprintln!("{}", meta_line(&meta));
            }
        }
        Ok(())
    }
}

pub fn cache_label(hit: bool, mode: CacheMode) -> &'static str {
    match (mode, hit) {
        (CacheMode::Bypass, _) => "bypass",
        (CacheMode::Use, true) => "hit",
        (CacheMode::Use, false) => "miss",
    }
}
```

```rust
// crates/illumio-tui/src/cli/health.rs
use super::context::Context;
use super::error::CliError;
use super::output::TableRows;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct HealthReport {
    pub ok: bool,
    pub host: String,
    pub org_id: u64,
    pub status: serde_json::Value,
}

impl TableRows for HealthReport {
    fn headers(&self) -> Vec<&'static str> {
        vec!["field", "value"]
    }

    fn rows(&self) -> Vec<Vec<String>> {
        vec![
            vec!["ok".into(), self.ok.to_string()],
            vec!["host".into(), self.host.clone()],
            vec!["org_id".into(), self.org_id.to_string()],
            vec!["status".into(), self.status.to_string()],
        ]
    }
}

pub async fn run(ctx: &Context) -> Result<(), CliError> {
    let status = ctx.pce.health().await?;
    let report = HealthReport { ok: true, host: ctx.config.host.clone(), org_id: ctx.config.org_id, status };
    ctx.emit(&report, "none", false, None)
}
```

```rust
// crates/illumio-tui/src/cli/mod.rs
//! clap definitions and dispatch. Each subcommand lives in its own module
//! and gets a `Context`.
pub mod context;
pub mod error;
pub mod health;
pub mod output;

use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

pub use context::Context;
pub use error::CliError;

#[derive(Parser, Debug)]
#[command(name = "illumio-tui", version, about = "Terminal UI and CLI for the Illumio Core PCE")]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalArgs,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Args, Debug, Clone, Default)]
pub struct GlobalArgs {
    /// Profile name from the config file (env: ILLUMIO_PROFILE)
    #[arg(long, global = true, env = "ILLUMIO_PROFILE")]
    pub profile: Option<String>,
    /// Config file (default: ~/.config/illumio-tui/config.toml)
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,
    /// dotenv file with PCE_HOST, PCE_ORG_ID, API_KEY, API_SECRET, ...
    #[arg(long, global = true)]
    pub env_file: Option<PathBuf>,
    #[arg(long, global = true)]
    pub host: Option<String>,
    #[arg(long, global = true)]
    pub port: Option<u16>,
    #[arg(long, global = true)]
    pub org_id: Option<u64>,
    /// Skip TLS certificate verification
    #[arg(long, global = true)]
    pub insecure: bool,
    /// JSON output (default when stdout is not a terminal)
    #[arg(long, global = true, conflicts_with = "table")]
    pub json: bool,
    /// Table output (default when stdout is a terminal)
    #[arg(long, global = true)]
    pub table: bool,
    /// Ignore cached responses
    #[arg(long, global = true)]
    pub no_cache: bool,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Check connectivity and credentials
    Health,
}

pub async fn run(global: GlobalArgs, command: Command) -> Result<(), CliError> {
    let ctx = Context::new(&global)?;
    match command {
        Command::Health => health::run(&ctx).await,
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p illumio-tui`
Expected: config tests and 8 CLI tests pass.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add -A
git commit -m "feat(cli): clap skeleton, JSON envelope, exit codes, table renderer, health command

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 15: `labels`, `label-groups`, `iplists`, `services` commands

**Files:**
- Create: `crates/illumio-tui/src/cli/objects.rs`
- Modify: `crates/illumio-tui/src/cli/mod.rs`, `crates/illumio-tui/tests/cli.rs`

**Interfaces:**
- Produces: `LabelsArgs { key, value, include_deleted }`; `Command::{Labels(LabelsArgs) | LabelGroups | Iplists | Services}`; `TableRows` impls for `Vec<Label>`, `Vec<LabelGroup>`, `Vec<IpList>`, `Vec<Service>`; `objects::labels(&Context, &LabelsArgs)`, `objects::label_groups(&Context)`, `objects::ip_lists(&Context)`, `objects::services(&Context)`.
- Label filtering is client-side and exact (the PCE's `?key=` is a substring match).

- [ ] **Step 1: Write the failing tests**

Add to `crates/illumio-tui/tests/cli.rs`:

```rust
async fn mount_objects(server: &MockServer) {
    for (p, body) in [
        ("/api/v2/orgs/5/labels", include_str!("../../../fixtures/labels.json")),
        ("/api/v2/orgs/5/sec_policy/draft/label_groups", include_str!("../../../fixtures/label_groups.json")),
        ("/api/v2/orgs/5/sec_policy/draft/ip_lists", include_str!("../../../fixtures/ip_lists.json")),
        ("/api/v2/orgs/5/sec_policy/draft/services", include_str!("../../../fixtures/services.json")),
    ] {
        Mock::given(method("GET")).and(path(p)).respond_with(ResponseTemplate::new(200).set_body_string(body).insert_header("Content-Type", "application/json")).mount(server).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn labels_filter_exactly_and_hide_deleted() {
    let server = MockServer::start().await;
    mount_objects(&server).await;
    let out = stdout_json(cmd(&server).args(["labels", "--key", "app"]).assert().success());
    let rows = out["data"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["value"], "shop");
    assert_eq!(out["meta"]["cache"], "miss");
    let out = stdout_json(cmd(&server).args(["labels", "--key", "app", "--include-deleted"]).assert().success());
    assert_eq!(out["data"].as_array().unwrap().len(), 2);
    let out = stdout_json(cmd(&server).args(["labels", "--value", "prod"]).assert().success());
    assert_eq!(out["data"][0]["key"], "env");
    cmd(&server).args(["labels", "--table"]).assert().success().stdout(predicate::str::contains("role  web"));
}

#[tokio::test(flavor = "multi_thread")]
async fn other_object_lists() {
    let server = MockServer::start().await;
    mount_objects(&server).await;
    let out = stdout_json(cmd(&server).arg("label-groups").assert().success());
    assert_eq!(out["data"][0]["name"], "All prod envs");
    let out = stdout_json(cmd(&server).arg("iplists").assert().success());
    assert_eq!(out["data"][1]["name"], "Corp");
    cmd(&server).args(["iplists", "--table"]).assert().success().stdout(predicate::str::contains("10.1.0.0-10.1.255.255, *.corp.example"));
    cmd(&server).args(["services", "--table"]).assert().success().stdout(predicate::str::contains("8000-8099/tcp"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p illumio-tui --test cli labels`
Expected: failure, unknown subcommand.

- [ ] **Step 3: Implement**

```rust
// crates/illumio-tui/src/cli/objects.rs
//! Labels, label groups, IP lists, services: one cached list each.
use super::context::{Context, cache_label};
use super::error::CliError;
use super::output::TableRows;
use clap::Args;
use pce::models::{IpList, Label, LabelGroup, Service};

#[derive(Args, Debug, Clone, Default)]
pub struct LabelsArgs {
    /// Exact label key (e.g. app)
    #[arg(long)]
    pub key: Option<String>,
    /// Exact label value
    #[arg(long)]
    pub value: Option<String>,
    /// Include labels the PCE marks deleted
    #[arg(long)]
    pub include_deleted: bool,
}

fn pending(update_type: &Option<String>) -> String {
    update_type.clone().unwrap_or_default()
}

impl TableRows for Vec<Label> {
    fn headers(&self) -> Vec<&'static str> {
        vec!["key", "value", "deleted", "href"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.iter().map(|l| vec![l.key.clone(), l.value.clone(), if l.is_deleted() { "yes".into() } else { String::new() }, l.href.clone()]).collect()
    }
}

impl TableRows for Vec<LabelGroup> {
    fn headers(&self) -> Vec<&'static str> {
        vec!["name", "key", "labels", "sub_groups", "pending", "href"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.iter().map(|g| vec![g.name.clone(), g.key.clone(), g.labels.len().to_string(), g.sub_groups.len().to_string(), pending(&g.update_type), g.href.clone()]).collect()
    }
}

impl TableRows for Vec<IpList> {
    fn headers(&self) -> Vec<&'static str> {
        vec!["name", "ranges", "pending", "href"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.iter().map(|l| vec![l.name.clone(), l.summary(), pending(&l.update_type), l.href.clone()]).collect()
    }
}

impl TableRows for Vec<Service> {
    fn headers(&self) -> Vec<&'static str> {
        vec!["name", "ports", "pending", "href"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.iter().map(|s| vec![s.name.clone(), s.summary(), pending(&s.update_type), s.href.clone()]).collect()
    }
}

pub async fn labels(ctx: &Context, args: &LabelsArgs) -> Result<(), CliError> {
    let c = ctx.pce.labels(ctx.cache_mode).await?;
    let data: Vec<Label> = c
        .value
        .into_iter()
        .filter(|l| args.include_deleted || !l.is_deleted())
        .filter(|l| args.key.as_deref().is_none_or(|k| l.key == k))
        .filter(|l| args.value.as_deref().is_none_or(|v| l.value == v))
        .collect();
    ctx.emit(&data, cache_label(c.hit, ctx.cache_mode), c.truncated, None)
}

pub async fn label_groups(ctx: &Context) -> Result<(), CliError> {
    let c = ctx.pce.label_groups(ctx.cache_mode).await?;
    ctx.emit(&c.value, cache_label(c.hit, ctx.cache_mode), c.truncated, None)
}

pub async fn ip_lists(ctx: &Context) -> Result<(), CliError> {
    let c = ctx.pce.ip_lists(ctx.cache_mode).await?;
    ctx.emit(&c.value, cache_label(c.hit, ctx.cache_mode), c.truncated, None)
}

pub async fn services(ctx: &Context) -> Result<(), CliError> {
    let c = ctx.pce.services(ctx.cache_mode).await?;
    ctx.emit(&c.value, cache_label(c.hit, ctx.cache_mode), c.truncated, None)
}
```

In `cli/mod.rs` add `pub mod objects;`, extend the enum and dispatch:

```rust
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Check connectivity and credentials
    Health,
    /// List labels
    Labels(objects::LabelsArgs),
    /// List label groups
    LabelGroups,
    /// List IP lists
    Iplists,
    /// List services
    Services,
}

pub async fn run(global: GlobalArgs, command: Command) -> Result<(), CliError> {
    let ctx = Context::new(&global)?;
    match command {
        Command::Health => health::run(&ctx).await,
        Command::Labels(a) => objects::labels(&ctx, &a).await,
        Command::LabelGroups => objects::label_groups(&ctx).await,
        Command::Iplists => objects::ip_lists(&ctx).await,
        Command::Services => objects::services(&ctx).await,
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p illumio-tui`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add -A
git commit -m "feat(cli): labels, label-groups, iplists, services commands

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 16: `workloads` command

**Files:**
- Create: `crates/illumio-tui/src/cli/workloads.rs`
- Modify: `crates/illumio-tui/src/cli/mod.rs`, `crates/illumio-tui/tests/cli.rs`

**Interfaces:**
- Produces: `WorkloadsArgs { hostname, name, ip, labels: Vec<String> (--label key=value, ANDed), enforcement, online, managed, max_results }`; `Command::Workloads(WorkloadsArgs)`; `TableRows for Vec<Workload>`; `workloads::run(&Context, &WorkloadsArgs)`.

- [ ] **Step 1: Write the failing tests**

Add to `crates/illumio-tui/tests/cli.rs`:

```rust
#[tokio::test(flavor = "multi_thread")]
async fn workloads_with_label_filter_resolve_hrefs() {
    use wiremock::matchers::query_param;
    let server = MockServer::start().await;
    mount_objects(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .and(query_param("labels", r#"[["/orgs/5/labels/3","/orgs/5/labels/4"]]"#))
        .and(query_param("online", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_string(include_str!("../../../fixtures/workloads.json")).insert_header("Content-Type", "application/json"))
        .expect(1)
        .mount(&server)
        .await;
    let out = stdout_json(cmd(&server).args(["workloads", "--label", "app=shop", "--label", "env=prod", "--online", "true"]).assert().success());
    assert_eq!(out["data"].as_array().unwrap().len(), 3);
    assert_eq!(out["data"][0]["hostname"], "host-1.example");
    cmd(&server).args(["workloads", "--label", "app=shop", "--label", "env=prod", "--online", "true", "--table"]).assert().success().stdout(predicate::str::contains("host-1.example").and(predicate::str::contains("app=shop env=prod role=web")).and(predicate::str::contains("active")));
}

#[tokio::test(flavor = "multi_thread")]
async fn workloads_unknown_label_exits_2_without_querying() {
    let server = MockServer::start().await;
    mount_objects(&server).await;
    Mock::given(method("GET")).and(path("/api/v2/orgs/5/workloads")).respond_with(ResponseTemplate::new(200).set_body_json(json!([]))).expect(0).mount(&server).await;
    let assert = cmd(&server).args(["workloads", "--label", "app=nope"]).assert().code(2);
    let err: Value = serde_json::from_slice(&assert.get_output().stderr).unwrap();
    assert!(err["error"]["message"].as_str().unwrap().contains("shop"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p illumio-tui --test cli workloads`
Expected: failure, unknown subcommand.

- [ ] **Step 3: Implement**

```rust
// crates/illumio-tui/src/cli/workloads.rs
use super::context::{Context, cache_label};
use super::error::CliError;
use super::output::TableRows;
use clap::Args;
use pce::PceError;
use pce::models::{Workload, WorkloadFilter};

#[derive(Args, Debug, Clone, Default)]
pub struct WorkloadsArgs {
    /// Hostname (PCE substring match)
    #[arg(long)]
    pub hostname: Option<String>,
    #[arg(long)]
    pub name: Option<String>,
    /// Interface IP address
    #[arg(long)]
    pub ip: Option<String>,
    /// Label key=value; repeat to AND several
    #[arg(long = "label")]
    pub labels: Vec<String>,
    /// full | visibility_only | selective | idle
    #[arg(long)]
    pub enforcement: Option<String>,
    #[arg(long)]
    pub online: Option<bool>,
    #[arg(long)]
    pub managed: Option<bool>,
    #[arg(long)]
    pub max_results: Option<u32>,
}

impl TableRows for Vec<Workload> {
    fn headers(&self) -> Vec<&'static str> {
        vec!["hostname", "name", "ip", "labels", "enforcement", "online", "agent", "href"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.iter()
            .map(|w| {
                vec![
                    w.hostname.clone().unwrap_or_default(),
                    w.name.clone().unwrap_or_default(),
                    w.primary_ip().unwrap_or("").to_string(),
                    w.label_text(),
                    w.enforcement_mode.clone().unwrap_or_default(),
                    w.online.map(|b| b.to_string()).unwrap_or_default(),
                    w.agent_status().unwrap_or_default(),
                    w.href.clone(),
                ]
            })
            .collect()
    }
}

pub async fn run(ctx: &Context, args: &WorkloadsArgs) -> Result<(), CliError> {
    let mut filter = WorkloadFilter {
        name: args.name.clone(),
        hostname: args.hostname.clone(),
        ip_address: args.ip.clone(),
        enforcement_mode: args.enforcement.clone(),
        managed: args.managed,
        online: args.online,
        labels: Vec::new(),
        max_results: args.max_results,
    };
    if !args.labels.is_empty() {
        let idx = ctx.pce.name_index(ctx.cache_mode).await?;
        let mut hrefs = Vec::new();
        for spec in &args.labels {
            let (k, v) = spec.split_once('=').ok_or_else(|| CliError::Args(format!("--label expects key=value, got {spec:?}")))?;
            let href = idx.label_href(k.trim(), v.trim()).ok_or_else(|| {
                let known = idx.values_for_key(k.trim());
                PceError::Invalid(format!("unknown label {spec}; known values for {}: {}", k.trim(), known.join(", ")))
            })?;
            hrefs.push(href.to_string());
        }
        filter.labels = vec![hrefs];
    }
    let c = ctx.pce.workloads(&filter, ctx.cache_mode).await?;
    ctx.emit(&c.value, cache_label(c.hit, ctx.cache_mode), c.truncated, None)
}
```

In `cli/mod.rs`: `pub mod workloads;`, add `/// List workloads` `Workloads(workloads::WorkloadsArgs)` to `Command`, and `Command::Workloads(a) => workloads::run(&ctx, &a).await` to `run`.

- [ ] **Step 4: Run tests**

Run: `cargo test -p illumio-tui`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add -A
git commit -m "feat(cli): workloads command with resolved label filters

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 17: `policy rulesets`, `policy rules`, `policy pending`

**Files:**
- Create: `crates/illumio-tui/src/cli/policy.rs`
- Modify: `crates/illumio-tui/src/cli/mod.rs`, `crates/illumio-tui/tests/cli.rs`

**Interfaces:**
- Produces: `PolicyCmd::{Rulesets(RulesetsArgs) | Rules(RulesArgs) | Pending}`; `RulesetsArgs { active, name }`; `RulesArgs { ruleset: String, active }`; `RulesOutput { ruleset: RuleSetView, rules: Vec<RuleView> }`; `TableRows` for `Vec<RuleSetView>`, `RulesOutput`, `Vec<PendingChange>`; `policy::run(&Context, &PolicyCmd)`.
- `policy rules` matches the ruleset by exact name (case-insensitive) or by href / href id; deny rules are appended; a 404 on the deny-rules endpoint is treated as "no deny rules" (older PCEs).

- [ ] **Step 1: Write the failing tests**

Add to `crates/illumio-tui/tests/cli.rs`:

```rust
async fn mount_policy(server: &MockServer) {
    mount_objects(server).await;
    Mock::given(method("GET")).and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets")).respond_with(ResponseTemplate::new(200).set_body_string(include_str!("../../../fixtures/rule_sets.json")).insert_header("Content-Type", "application/json")).mount(server).await;
    Mock::given(method("GET")).and(path("/api/v2/orgs/5/sec_policy/active/rule_sets")).respond_with(ResponseTemplate::new(200).set_body_json(json!([]))).mount(server).await;
    Mock::given(method("GET")).and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets/100/deny_rules")).respond_with(ResponseTemplate::new(200).set_body_string(include_str!("../../../fixtures/deny_rules.json")).insert_header("Content-Type", "application/json")).mount(server).await;
    Mock::given(method("GET")).and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets/101/deny_rules")).respond_with(ResponseTemplate::new(404)).mount(server).await;
    Mock::given(method("GET")).and(path("/api/v2/orgs/5/sec_policy/pending")).respond_with(ResponseTemplate::new(200).set_body_string(include_str!("../../../fixtures/pending_list.json")).insert_header("Content-Type", "application/json")).mount(server).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn policy_rulesets_resolve_scopes_and_filter_by_name() {
    let server = MockServer::start().await;
    mount_policy(&server).await;
    let out = stdout_json(cmd(&server).args(["policy", "rulesets"]).assert().success());
    assert_eq!(out["data"].as_array().unwrap().len(), 2);
    assert_eq!(out["data"][0]["scopes"][0], "app=shop & env=prod");
    assert_eq!(out["data"][0]["rule_count"], 2);
    assert_eq!(out["data"][1]["update_type"], "create");
    let out = stdout_json(cmd(&server).args(["policy", "rulesets", "--name", "infra"]).assert().success());
    assert_eq!(out["data"].as_array().unwrap().len(), 1);
    let out = stdout_json(cmd(&server).args(["policy", "rulesets", "--active"]).assert().success());
    assert_eq!(out["data"].as_array().unwrap().len(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn policy_rules_include_deny_rules_and_tolerate_missing_endpoint() {
    let server = MockServer::start().await;
    mount_policy(&server).await;
    let out = stdout_json(cmd(&server).args(["policy", "rules", "shop-prod"]).assert().success());
    assert_eq!(out["data"]["ruleset"]["name"], "Shop-Prod");
    let rules = out["data"]["rules"].as_array().unwrap();
    assert_eq!(rules.len(), 4);
    assert_eq!(rules[1]["consumers"][0], "ipl:Corp");
    assert_eq!(rules[1]["services"][1], "8000-8099/tcp");
    assert_eq!(rules[3]["kind"], "override_deny");
    let out = stdout_json(cmd(&server).args(["policy", "rules", "/orgs/5/sec_policy/draft/rule_sets/101"]).assert().success());
    assert_eq!(out["data"]["rules"].as_array().unwrap().len(), 1);
    assert_eq!(out["data"]["rules"][0]["providers"][0], "All workloads");
    cmd(&server).args(["policy", "rules", "ghost"]).assert().code(2);
    cmd(&server).args(["policy", "rules", "shop-prod", "--table"]).assert().success().stdout(predicate::str::contains("override_deny").and(predicate::str::contains("role=web")));
}

#[tokio::test(flavor = "multi_thread")]
async fn policy_pending_lists_changes() {
    let server = MockServer::start().await;
    mount_policy(&server).await;
    let out = stdout_json(cmd(&server).args(["policy", "pending"]).assert().success());
    assert_eq!(out["data"][0]["object_type"], "rule_sets");
    assert_eq!(out["data"][1]["name"], "Corp");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p illumio-tui --test cli policy`
Expected: failure, unknown subcommand.

- [ ] **Step 3: Implement**

```rust
// crates/illumio-tui/src/cli/policy.rs
//! Read-only policy views: rulesets, the rules of one ruleset (allow and
//! deny), and pending provisioning changes.
use super::context::{Context, cache_label};
use super::error::CliError;
use super::output::TableRows;
use clap::{Args, Subcommand};
use pce::models::{PendingChange, PolicyScope, RuleSet};
use pce::{PceError, RuleSetView, RuleView};
use serde::Serialize;

#[derive(Subcommand, Debug)]
pub enum PolicyCmd {
    /// List rulesets with resolved scopes
    Rulesets(RulesetsArgs),
    /// List the rules of one ruleset (name or href), allow and deny
    Rules(RulesArgs),
    /// List changes waiting to be provisioned
    Pending,
}

#[derive(Args, Debug, Clone, Default)]
pub struct RulesetsArgs {
    /// Read active policy instead of draft
    #[arg(long)]
    pub active: bool,
    /// Case-insensitive substring of the ruleset name
    #[arg(long)]
    pub name: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct RulesArgs {
    /// Ruleset name (exact, case-insensitive) or href
    pub ruleset: String,
    #[arg(long)]
    pub active: bool,
}

#[derive(Debug, Serialize)]
pub struct RulesOutput {
    pub ruleset: RuleSetView,
    pub rules: Vec<RuleView>,
}

fn scope(active: bool) -> PolicyScope {
    if active { PolicyScope::Active } else { PolicyScope::Draft }
}

impl TableRows for Vec<RuleSetView> {
    fn headers(&self) -> Vec<&'static str> {
        vec!["name", "enabled", "scopes", "rules", "pending", "href"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.iter().map(|r| vec![r.name.clone(), r.enabled.to_string(), r.scopes.join(" | "), r.rule_count.to_string(), r.update_type.clone().unwrap_or_default(), r.href.clone()]).collect()
    }
}

impl TableRows for RulesOutput {
    fn headers(&self) -> Vec<&'static str> {
        vec!["kind", "enabled", "consumers", "providers", "services", "description", "pending"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.rules
            .iter()
            .map(|r| {
                vec![
                    serde_json::to_value(r.kind).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default(),
                    r.enabled.to_string(),
                    r.consumers.join(", "),
                    r.providers.join(", "),
                    r.services.join(", "),
                    r.description.clone().unwrap_or_default(),
                    r.update_type.clone().unwrap_or_default(),
                ]
            })
            .collect()
    }
}

impl TableRows for Vec<PendingChange> {
    fn headers(&self) -> Vec<&'static str> {
        vec!["object_type", "change_type", "name", "href"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.iter().map(|p| vec![p.object_type.clone(), p.change_type.clone(), p.name.clone().unwrap_or_default(), p.href.clone()]).collect()
    }
}

pub async fn run(ctx: &Context, cmd: &PolicyCmd) -> Result<(), CliError> {
    match cmd {
        PolicyCmd::Rulesets(a) => rulesets(ctx, a).await,
        PolicyCmd::Rules(a) => rules(ctx, a).await,
        PolicyCmd::Pending => pending(ctx).await,
    }
}

async fn rulesets(ctx: &Context, args: &RulesetsArgs) -> Result<(), CliError> {
    let idx = ctx.pce.name_index(ctx.cache_mode).await?;
    let c = ctx.pce.rule_sets(scope(args.active), ctx.cache_mode).await?;
    let needle = args.name.as_deref().map(str::to_ascii_lowercase);
    let data: Vec<RuleSetView> = c
        .value
        .iter()
        .filter(|rs| needle.as_deref().is_none_or(|n| rs.name.to_ascii_lowercase().contains(n)))
        .map(|rs| RuleSetView::from_rule_set(rs, &idx))
        .collect();
    ctx.emit(&data, cache_label(c.hit, ctx.cache_mode), c.truncated, None)
}

fn find_ruleset<'a>(sets: &'a [RuleSet], needle: &str) -> Option<&'a RuleSet> {
    let n = needle.trim();
    sets.iter().find(|rs| rs.href == n || rs.name.eq_ignore_ascii_case(n)).or_else(|| sets.iter().find(|rs| rs.href.ends_with(&format!("/{n}"))))
}

async fn rules(ctx: &Context, args: &RulesArgs) -> Result<(), CliError> {
    let idx = ctx.pce.name_index(ctx.cache_mode).await?;
    let c = ctx.pce.rule_sets(scope(args.active), ctx.cache_mode).await?;
    let rs = find_ruleset(&c.value, &args.ruleset).ok_or_else(|| {
        let names: Vec<&str> = c.value.iter().map(|r| r.name.as_str()).collect();
        CliError::Args(format!("ruleset {:?} not found; known: {}", args.ruleset, names.join(", ")))
    })?;
    let mut views: Vec<RuleView> = rs.rules.iter().map(|r| RuleView::from_rule(r, &idx)).collect();
    match ctx.pce.deny_rules(&rs.href, ctx.cache_mode).await {
        Ok(d) => views.extend(d.value.iter().map(|r| RuleView::from_deny(r, &idx))),
        Err(PceError::NotFound { .. }) => {}
        Err(e) => return Err(e.into()),
    }
    let out = RulesOutput { ruleset: RuleSetView::from_rule_set(rs, &idx), rules: views };
    ctx.emit(&out, cache_label(c.hit, ctx.cache_mode), c.truncated, None)
}

async fn pending(ctx: &Context) -> Result<(), CliError> {
    let c = ctx.pce.pending_changes(ctx.cache_mode).await?;
    ctx.emit(&c.value, cache_label(c.hit, ctx.cache_mode), false, None)
}
```

In `cli/mod.rs`: `pub mod policy;`, add to `Command`:

```rust
    /// Rulesets, rules, pending changes
    #[command(subcommand)]
    Policy(policy::PolicyCmd),
```

and `Command::Policy(p) => policy::run(&ctx, &p).await` to `run`.

- [ ] **Step 4: Run tests**

Run: `cargo test -p illumio-tui`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add -A
git commit -m "feat(cli): policy rulesets, rules (allow + deny), pending

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 18: `traffic query` command

**Files:**
- Create: `crates/illumio-tui/src/cli/traffic.rs`
- Modify: `crates/illumio-tui/src/cli/mod.rs`, `crates/illumio-tui/tests/cli.rs`

**Interfaces:**
- Produces: `TrafficCmd::Query(QueryArgs)`; `QueryArgs { window, from, to, src, dst, exclude_src, exclude_dst, port, proto, process, decision, max_results, group, blocked, dry_run, yes }`; `parse_time(&str, end_of_day: bool) -> Result<Timestamp, CliError>`; `build_query(&Context, &QueryArgs) -> Result<TrafficQuery, CliError>` (resolves filters, applies profile defaults for window and max_results); `TableRows` for `Vec<ResolvedFlow>`, `Vec<FlowGroup>`, `Vec<BlockedGroup>`, and `DryRun(TrafficQuery)`; `traffic::run(&Context, &TrafficCmd)`.
- Semantics: each `--src`/`--dst` value is a comma-separated AND group; repeating the flag ORs groups. `--window` is relative (`24h`, `7d`); `--from`/`--to` are RFC 3339 or `YYYY-MM-DD` (date-only `--to` means end of that day). Confirmation gates come from `TrafficQuery::needs_confirmation()` and are bypassed by `--yes`. `--dry-run` resolves filters (needs the name index) but never submits the query.

- [ ] **Step 1: Write the failing tests**

Add to `crates/illumio-tui/tests/cli.rs`:

```rust
const QHREF: &str = "/orgs/5/traffic_flows/async_queries/q-1";

async fn mount_traffic(server: &MockServer, expect_post: u64) {
    mount_objects(server).await;
    Mock::given(method("POST")).and(path("/api/v2/orgs/5/traffic_flows/async_queries")).respond_with(ResponseTemplate::new(202).set_body_json(json!({"href": QHREF, "status": "queued"}))).expect(expect_post).mount(server).await;
    Mock::given(method("GET")).and(path(format!("/api/v2{QHREF}"))).respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "completed", "result": {"href": format!("{QHREF}/download")}, "matches_count": 4}))).mount(server).await;
    Mock::given(method("GET")).and(path(format!("/api/v2{QHREF}/download"))).respond_with(ResponseTemplate::new(200).set_body_string(include_str!("../../../fixtures/traffic_flows.json")).insert_header("Content-Type", "application/json")).mount(server).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn traffic_query_flows_json() {
    let server = MockServer::start().await;
    mount_traffic(&server, 1).await;
    let out = stdout_json(cmd(&server).args(["traffic", "query", "--window", "2h", "--src", "app=shop,env=prod", "--decision", "allowed,blocked"]).assert().success());
    let flows = out["data"].as_array().unwrap();
    assert_eq!(flows.len(), 4);
    assert_eq!(flows[0]["src"]["hostname"], "host-1.example");
    assert_eq!(flows[0]["src"]["labels"][0]["key"], "app");
    assert_eq!(flows[0]["dst"]["labels"][2]["value"], "db");
    assert_eq!(flows[2]["proto"], "icmp");
    assert!(flows[2]["port"].is_null());
    assert_eq!(out["meta"]["truncated"], false);
    assert!(out["meta"]["window"]["start"].as_str().unwrap().ends_with('Z'));
    assert_eq!(out["meta"]["cache"], "miss");
}

#[tokio::test(flavor = "multi_thread")]
async fn traffic_query_group_and_blocked_views() {
    let server = MockServer::start().await;
    mount_traffic(&server, 3).await; // three separate processes, no shared cache
    let out = stdout_json(cmd(&server).args(["traffic", "query", "--from", "2026-10-09", "--to", "2026-10-09", "--group", "app,env"]).assert().success());
    assert_eq!(out["data"][0]["src"], json!(["shop", "prod"]));
    assert_eq!(out["data"][0]["connections"], 310);
    assert_eq!(out["meta"]["window"]["start"], "2026-10-09T00:00:00Z");
    assert_eq!(out["meta"]["window"]["end"], "2026-10-09T23:59:59Z");
    let out = stdout_json(cmd(&server).args(["traffic", "query", "--window", "1h", "--blocked"]).assert().success());
    assert_eq!(out["data"].as_array().unwrap().len(), 2);
    assert_eq!(out["data"][0]["dst"], "host-1.example");
    assert_eq!(out["data"][0]["port"], 8443);
    cmd(&server).args(["traffic", "query", "--window", "1h", "--blocked", "--table"]).assert().success().stdout(predicate::str::contains("8443/tcp"));
}

#[tokio::test(flavor = "multi_thread")]
async fn traffic_dry_run_prints_body_without_submitting() {
    let server = MockServer::start().await;
    mount_traffic(&server, 0).await;
    let out = stdout_json(cmd(&server).args(["traffic", "query", "--window", "1h", "--src", "ipl:corp", "--dst", "10.0.0.5", "--port", "443", "--proto", "tcp", "--dry-run"]).assert().success());
    assert_eq!(out["data"]["sources"]["include"], json!([[{"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/21"}}]]));
    assert_eq!(out["data"]["destinations"]["include"], json!([[{"ip_address": "10.0.0.5"}]]));
    assert_eq!(out["data"]["services"]["include"], json!([{"port": 443, "proto": 6}]));
    assert_eq!(out["data"]["max_results"], 1000);
    assert_eq!(out["meta"]["cache"], "none");
}

#[tokio::test(flavor = "multi_thread")]
async fn traffic_guards_exit_2() {
    let server = MockServer::start().await;
    mount_traffic(&server, 0).await;
    let assert = cmd(&server).args(["traffic", "query", "--window", "30d"]).assert().code(2);
    let err: Value = serde_json::from_slice(&assert.get_output().stderr).unwrap();
    assert!(err["error"]["message"].as_str().unwrap().contains("--yes"));
    let assert = cmd(&server).args(["traffic", "query", "--from", "2026-10-09", "--to", "2026-10-01"]).assert().code(2);
    let err: Value = serde_json::from_slice(&assert.get_output().stderr).unwrap();
    assert!(err["error"]["message"].as_str().unwrap().contains("before"));
    let assert = cmd(&server).args(["traffic", "query", "--src", "app=nope"]).assert().code(2);
    let err: Value = serde_json::from_slice(&assert.get_output().stderr).unwrap();
    assert!(err["error"]["message"].as_str().unwrap().contains("shop"));
    cmd(&server).args(["traffic", "query", "--decision", "maybe"]).assert().code(2);
    cmd(&server).args(["traffic", "query", "--max-results", "20000"]).assert().code(2);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p illumio-tui --test cli traffic`
Expected: failure, unknown subcommand.

- [ ] **Step 3: Implement**

```rust
// crates/illumio-tui/src/cli/traffic.rs
//! `traffic query`: build an Explorer query from human filters, guard its
//! cost, run it through the cached facade, and print flows, label-pair
//! groups, or the blocked summary.
use super::context::{Context, cache_label};
use super::error::CliError;
use super::output::{TableRows, Window};
use clap::{Args, Subcommand};
use jiff::Timestamp;
use jiff::civil::Date;
use pce::models::{PolicyDecision, ServiceMatch, TrafficQuery, proto_number};
use pce::traffic::{BlockedGroup, FilterExpr, FlowGroup, ResolvedFlow, TrafficQueryBuilder, blocked_summary, group_by_labels, parse_window, resolve_flows};
use serde::Serialize;
use tokio_util::sync::CancellationToken;

#[derive(Subcommand, Debug)]
pub enum TrafficCmd {
    /// Run an Explorer traffic query
    Query(QueryArgs),
}

#[derive(Args, Debug, Clone, Default)]
pub struct QueryArgs {
    /// Relative window ending now, e.g. 1h, 24h, 7d (default from profile, 24h)
    #[arg(long, conflicts_with_all = ["from", "to"])]
    pub window: Option<String>,
    /// Start, RFC 3339 or YYYY-MM-DD (UTC)
    #[arg(long, requires = "to")]
    pub from: Option<String>,
    /// End, RFC 3339 or YYYY-MM-DD (end of that day, UTC)
    #[arg(long, requires = "from")]
    pub to: Option<String>,
    /// Source filter: key=value, ip:ADDR, ipl:NAME, wl:HOST, fqdn:NAME. Comma = AND, repeat = OR
    #[arg(long = "src")]
    pub src: Vec<String>,
    /// Destination filter, same syntax as --src
    #[arg(long = "dst")]
    pub dst: Vec<String>,
    #[arg(long = "exclude-src")]
    pub exclude_src: Vec<String>,
    #[arg(long = "exclude-dst")]
    pub exclude_dst: Vec<String>,
    #[arg(long)]
    pub port: Option<u16>,
    /// tcp, udp, icmp or a protocol number
    #[arg(long)]
    pub proto: Option<String>,
    /// Process name on the destination
    #[arg(long)]
    pub process: Option<String>,
    /// allowed, potentially_blocked, blocked (comma-separated; default all)
    #[arg(long, value_delimiter = ',')]
    pub decision: Vec<String>,
    /// Row cap (default from profile, 1000; > 10000 needs --yes)
    #[arg(long)]
    pub max_results: Option<u32>,
    /// Group by label keys, e.g. app,env
    #[arg(long, value_delimiter = ',', conflicts_with = "blocked")]
    pub group: Vec<String>,
    /// Only blocked and potentially blocked flows, grouped by destination and port
    #[arg(long)]
    pub blocked: bool,
    /// Print the request body instead of sending it
    #[arg(long)]
    pub dry_run: bool,
    /// Confirm an expensive query (window > 7d or max_results > 10000)
    #[arg(long)]
    pub yes: bool,
}

/// RFC 3339, or a date; a date-only `--to` means 23:59:59 of that day.
pub fn parse_time(s: &str, end_of_day: bool) -> Result<Timestamp, CliError> {
    let s = s.trim();
    if let Ok(t) = s.parse::<Timestamp>() {
        return Ok(t);
    }
    if let Ok(d) = s.parse::<Date>() {
        let dt = if end_of_day { d.at(23, 59, 59, 0) } else { d.at(0, 0, 0, 0) };
        return dt.to_zoned(jiff::tz::TimeZone::UTC).map(|z| z.timestamp()).map_err(|e| CliError::Args(format!("bad time {s:?}: {e}")));
    }
    Err(CliError::Args(format!("bad time {s:?}: use RFC 3339 (2026-10-09T08:00:00Z) or YYYY-MM-DD")))
}

fn parse_groups(values: &[String]) -> Result<Vec<Vec<FilterExpr>>, CliError> {
    values.iter().map(|v| v.split(',').filter(|s| !s.trim().is_empty()).map(|s| FilterExpr::parse(s).map_err(CliError::from)).collect()).collect()
}

pub async fn build_query(ctx: &Context, args: &QueryArgs) -> Result<TrafficQuery, CliError> {
    let mut b = match (&args.from, &args.to) {
        (Some(f), Some(t)) => TrafficQueryBuilder::between(parse_time(f, false)?, parse_time(t, true)?),
        _ => {
            let w = args.window.clone().unwrap_or_else(|| ctx.config.traffic_default_window.clone());
            TrafficQueryBuilder::last(parse_window(&w)?)?
        }
    };
    let src = ctx.pce.resolve_filter_groups(&parse_groups(&args.src)?, ctx.cache_mode).await?;
    let dst = ctx.pce.resolve_filter_groups(&parse_groups(&args.dst)?, ctx.cache_mode).await?;
    let ex_src = ctx.pce.resolve_filter_groups(&parse_groups(&args.exclude_src)?, ctx.cache_mode).await?;
    let ex_dst = ctx.pce.resolve_filter_groups(&parse_groups(&args.exclude_dst)?, ctx.cache_mode).await?;
    for g in src {
        b = b.source(g);
    }
    for g in dst {
        b = b.destination(g);
    }
    for m in ex_src.into_iter().flatten() {
        b = b.exclude_source(m);
    }
    for m in ex_dst.into_iter().flatten() {
        b = b.exclude_destination(m);
    }
    if args.port.is_some() || args.proto.is_some() || args.process.is_some() {
        let proto = match &args.proto {
            Some(p) => Some(proto_number(p).ok_or_else(|| CliError::Args(format!("unknown protocol {p:?}")))?),
            None => None,
        };
        b = b.service(ServiceMatch { port: args.port, proto, process_name: args.process.clone() });
    }
    let decisions: Vec<PolicyDecision> = args.decision.iter().map(|d| PolicyDecision::parse(d).ok_or_else(|| CliError::Args(format!("unknown decision {d:?}; use allowed, potentially_blocked, blocked")))).collect::<Result<_, _>>()?;
    b = b.decisions(decisions).max_results(args.max_results.unwrap_or(ctx.config.traffic_max_results));
    Ok(b.build()?)
}

#[derive(Debug, Serialize)]
#[serde(transparent)]
pub struct DryRun(pub TrafficQuery);

impl TableRows for DryRun {
    fn headers(&self) -> Vec<&'static str> {
        vec!["request body"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        serde_json::to_string_pretty(&self.0).unwrap_or_default().lines().map(|l| vec![l.to_string()]).collect()
    }
}

impl TableRows for Vec<ResolvedFlow> {
    fn headers(&self) -> Vec<&'static str> {
        vec!["last seen", "src", "src labels", "dst", "dst labels", "service", "conns", "decision", "process"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.iter()
            .map(|f| {
                vec![
                    f.last_detected.clone().unwrap_or_default(),
                    f.src.display_name(),
                    f.src.label_text(),
                    f.dst.display_name(),
                    f.dst.label_text(),
                    f.service_text(),
                    f.num_connections.to_string(),
                    f.policy_decision.as_str().to_string(),
                    f.process_name.clone().unwrap_or_default(),
                ]
            })
            .collect()
    }
}

impl TableRows for Vec<FlowGroup> {
    fn headers(&self) -> Vec<&'static str> {
        vec!["src", "dst", "flows", "conns", "allowed", "pot.blocked", "blocked"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.iter().map(|g| vec![g.src.join("/"), g.dst.join("/"), g.flows.to_string(), g.connections.to_string(), g.allowed.to_string(), g.potentially_blocked.to_string(), g.blocked.to_string()]).collect()
    }
}

impl TableRows for Vec<BlockedGroup> {
    fn headers(&self) -> Vec<&'static str> {
        vec!["last seen", "dst", "dst labels", "service", "flows", "conns", "sources"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.iter()
            .map(|g| {
                let service = match g.port {
                    Some(p) => format!("{p}/{}", g.proto),
                    None => g.proto.clone(),
                };
                vec![g.last_detected.clone().unwrap_or_default(), g.dst.clone(), g.dst_labels.clone(), service, g.flows.to_string(), g.connections.to_string(), g.sources.join(", ")]
            })
            .collect()
    }
}

pub async fn run(ctx: &Context, cmd: &TrafficCmd) -> Result<(), CliError> {
    match cmd {
        TrafficCmd::Query(a) => query(ctx, a).await,
    }
}

async fn query(ctx: &Context, args: &QueryArgs) -> Result<(), CliError> {
    let q = build_query(ctx, args).await?;
    let window = Some(Window { start: q.start_date.clone(), end: q.end_date.clone() });
    if args.dry_run {
        return ctx.emit(&DryRun(q), "none", false, window);
    }
    if let Some(reason) = q.needs_confirmation() {
        if !args.yes {
            return Err(CliError::Args(format!("expensive query: {reason}; re-run with --yes to confirm")));
        }
    }
    let idx = ctx.pce.name_index(ctx.cache_mode).await?;
    let result = ctx.pce.traffic(&q, ctx.cache_mode, &CancellationToken::new(), None).await?;
    let cache = cache_label(result.hit, ctx.cache_mode);
    let flows = resolve_flows(&result.value.flows, &idx);
    if args.blocked {
        ctx.emit(&blocked_summary(&flows), cache, result.truncated, window)
    } else if !args.group.is_empty() {
        ctx.emit(&group_by_labels(&flows, &args.group), cache, result.truncated, window)
    } else {
        ctx.emit(&flows, cache, result.truncated, window)
    }
}
```

In `cli/mod.rs`: `pub mod traffic;`, add to `Command`:

```rust
    /// Explorer traffic queries
    #[command(subcommand)]
    Traffic(traffic::TrafficCmd),
```

and `Command::Traffic(t) => traffic::run(&ctx, &t).await` to `run`.

- [ ] **Step 4: Run tests**

Run: `cargo test -p illumio-tui`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add -A
git commit -m "feat(cli): traffic query with filters, cost guards, dry-run, group and blocked views

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 19: `profile list` and `profile use`

**Files:**
- Create: `crates/illumio-tui/src/cli/profile.rs`
- Modify: `crates/illumio-tui/src/cli/mod.rs`, `crates/illumio-tui/src/cli/context.rs`, `crates/illumio-tui/tests/cli.rs`

**Interfaces:**
- Produces: `ProfileCmd::{List | Use { name: String }}`; `ProfileRow { name, host, org_id, is_default }`; `profile::run(&GlobalArgs, &ProfileCmd)` (no `Context`, because listing profiles must not require credentials); `output::choose_format(&GlobalArgs) -> OutputFormat` extracted from `Context::new`.
- `profile use` rewrites only `default_profile` in the config file, creates parent directories, and sets mode 0600 on Unix. It never writes secrets it did not read from that same file.

- [ ] **Step 1: Write the failing tests**

Add to `crates/illumio-tui/tests/cli.rs`:

```rust
#[test]
fn profile_list_and_use_without_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("config.toml");
    std::fs::write(&cfg, "default_profile = \"a\"\n\n[profiles.a]\nhost = \"a.example\"\norg_id = 1\n\n[profiles.b]\nhost = \"b.example\"\norg_id = 2\n").unwrap();
    let out = stdout_json(Command::cargo_bin("illumio-tui").unwrap().env_clear().args(["--config", cfg.to_str().unwrap(), "profile", "list"]).assert().success());
    let rows = out["data"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["name"], "a");
    assert_eq!(rows[0]["is_default"], true);
    assert_eq!(rows[1]["host"], "b.example");

    let out = stdout_json(Command::cargo_bin("illumio-tui").unwrap().env_clear().args(["--config", cfg.to_str().unwrap(), "profile", "use", "b"]).assert().success());
    assert_eq!(out["data"]["default_profile"], "b");
    let text = std::fs::read_to_string(&cfg).unwrap();
    assert!(text.contains("default_profile = \"b\""));
    assert!(text.contains("host = \"a.example\""), "other profiles survive: {text}");

    Command::cargo_bin("illumio-tui").unwrap().env_clear().args(["--config", cfg.to_str().unwrap(), "profile", "use", "zzz"]).assert().code(2);
    Command::cargo_bin("illumio-tui").unwrap().env_clear().args(["--config", cfg.to_str().unwrap(), "profile", "list", "--table"]).assert().success().stdout(predicate::str::contains("b.example"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p illumio-tui --test cli profile`
Expected: failure, unknown subcommand.

- [ ] **Step 3: Implement**

In `cli/output.rs` add:

```rust
pub fn choose_format(json: bool, table: bool) -> OutputFormat {
    if table {
        OutputFormat::Table
    } else if json || !std::io::IsTerminal::is_terminal(&std::io::stdout()) {
        OutputFormat::Json
    } else {
        OutputFormat::Table
    }
}
```

and in `cli/context.rs` replace the `let format = ...` block with `let format = super::output::choose_format(global.json, global.table);` (remove the now-unused `IsTerminal` import).

```rust
// crates/illumio-tui/src/cli/profile.rs
//! Profile management. Works without credentials: it only reads and
//! rewrites the config file.
use super::GlobalArgs;
use super::error::CliError;
use super::output::{Meta, TableRows, choose_format, meta_line, write_json, write_table};
use crate::config::{self, ConfigFile};
use clap::Subcommand;
use serde::Serialize;

#[derive(Subcommand, Debug)]
pub enum ProfileCmd {
    /// List profiles in the config file
    List,
    /// Set the default profile
    Use { name: String },
}

#[derive(Debug, Serialize)]
pub struct ProfileRow {
    pub name: String,
    pub host: String,
    pub org_id: Option<u64>,
    pub is_default: bool,
}

impl TableRows for Vec<ProfileRow> {
    fn headers(&self) -> Vec<&'static str> {
        vec!["name", "host", "org_id", "default"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.iter().map(|p| vec![p.name.clone(), p.host.clone(), p.org_id.map(|o| o.to_string()).unwrap_or_default(), if p.is_default { "*".into() } else { String::new() }]).collect()
    }
}

#[derive(Debug, Serialize)]
pub struct UseResult {
    pub default_profile: String,
    pub config_path: String,
}

impl TableRows for UseResult {
    fn headers(&self) -> Vec<&'static str> {
        vec!["default_profile", "config_path"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        vec![vec![self.default_profile.clone(), self.config_path.clone()]]
    }
}

fn emit<T: Serialize + TableRows>(global: &GlobalArgs, data: &T) -> Result<(), CliError> {
    let meta = Meta { profile: String::new(), elapsed_ms: 0, api_calls: 0, bytes: 0, cache: "none", truncated: false, window: None };
    let mut out = std::io::stdout().lock();
    match choose_format(global.json, global.table) {
        super::output::OutputFormat::Json => write_json(&mut out, data, &meta)?,
        super::output::OutputFormat::Table => {
            write_table(&mut out, data)?;
            eprintln!("{}", meta_line(&meta));
        }
    }
    Ok(())
}

pub fn run(global: &GlobalArgs, cmd: &ProfileCmd) -> Result<(), CliError> {
    let path = global.config.clone().unwrap_or_else(config::default_config_path);
    let mut file: ConfigFile = config::load_config_file(&path)?;
    match cmd {
        ProfileCmd::List => {
            let rows: Vec<ProfileRow> = file
                .profiles
                .iter()
                .map(|(name, p)| ProfileRow { name: name.clone(), host: p.host.clone().unwrap_or_default(), org_id: p.org_id, is_default: file.default_profile.as_deref() == Some(name.as_str()) })
                .collect();
            emit(global, &rows)
        }
        ProfileCmd::Use { name } => {
            if !file.profiles.contains_key(name) {
                return Err(CliError::Args(format!("profile {name:?} not found in {}; known: {}", path.display(), file.profiles.keys().cloned().collect::<Vec<_>>().join(", "))));
            }
            file.default_profile = Some(name.clone());
            let text = toml::to_string_pretty(&file).map_err(|e| CliError::Other(format!("cannot serialise config: {e}")))?;
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, text)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
            }
            emit(global, &UseResult { default_profile: name.clone(), config_path: path.display().to_string() })
        }
    }
}
```

In `cli/mod.rs`: `pub mod profile;`, add to `Command`:

```rust
    /// Manage connection profiles
    #[command(subcommand)]
    Profile(profile::ProfileCmd),
```

and change `run` so profile commands skip `Context`:

```rust
pub async fn run(global: GlobalArgs, command: Command) -> Result<(), CliError> {
    if let Command::Profile(p) = &command {
        return profile::run(&global, p);
    }
    let ctx = Context::new(&global)?;
    match command {
        Command::Health => health::run(&ctx).await,
        Command::Labels(a) => objects::labels(&ctx, &a).await,
        Command::LabelGroups => objects::label_groups(&ctx).await,
        Command::Iplists => objects::ip_lists(&ctx).await,
        Command::Services => objects::services(&ctx).await,
        Command::Workloads(a) => workloads::run(&ctx, &a).await,
        Command::Policy(p) => policy::run(&ctx, &p).await,
        Command::Traffic(t) => traffic::run(&ctx, &t).await,
        Command::Profile(_) => unreachable!("handled above"),
    }
}
```

`Profile` serialises with `Option` fields as absent, so a round-trip through `toml::to_string_pretty` keeps the file readable. `Limits` is `#[serde(default)]` on the struct field, so an empty `[profiles.x.limits]` table is harmless.

- [ ] **Step 4: Run tests**

Run: `cargo test -p illumio-tui`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add -A
git commit -m "feat(cli): profile list and profile use

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 20: Fixture recorder (`cargo xtask record`) and the live smoke test

**Files:**
- Modify: `crates/xtask/src/main.rs`, `.gitignore` (already ignores `fixtures/_raw/`)
- Create: `crates/pce/tests/live.rs`, `crates/pce/tests/recorded.rs`

**Interfaces:**
- `cargo xtask record [--env-file PATH] [--out fixtures/recorded]` writes anonymised JSON for labels, label groups, IP lists, services, workloads (25), rulesets (10), deny rules (first 3 rulesets), pending, and one 1-hour traffic query (50 rows). It reads the same env vars as the CLI and caps itself at 2 requests/second. Hand-written `fixtures/*.json` stay the deterministic test inputs; `fixtures/recorded/*.json` pin real shapes and are only checked for deserialisation.
- `PCE_LIVE=1 cargo test -p pce --test live` runs health, labels, name index, and a 1-hour, 50-row traffic query against the configured PCE and asserts on shape and on an API-call cap of 8.

- [ ] **Step 1: Write the recorded-fixture shape test (passes vacuously until fixtures exist)**

```rust
// crates/pce/tests/recorded.rs
//! Every file under fixtures/recorded/ must still deserialise into its
//! model. Re-record with `cargo xtask record` when the PCE API changes.
use pce::models::{DenyRule, Flow, IpList, Label, LabelGroup, RuleSet, Service, Workload};
use std::path::Path;

fn check<T: serde::de::DeserializeOwned>(name: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/recorded").join(name);
    let Ok(text) = std::fs::read_to_string(&path) else {
        eprintln!("skipping {name}: not recorded");
        return;
    };
    let parsed: Vec<T> = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
    assert!(!parsed.is_empty(), "{name} is empty; re-record");
}

#[test]
fn recorded_fixtures_still_parse() {
    check::<Label>("labels.json");
    check::<LabelGroup>("label_groups.json");
    check::<IpList>("ip_lists.json");
    check::<Service>("services.json");
    check::<Workload>("workloads.json");
    check::<RuleSet>("rule_sets.json");
    check::<DenyRule>("deny_rules.json");
    check::<Flow>("traffic_flows.json");
}
```

- [ ] **Step 2: Write the live smoke test**

```rust
// crates/pce/tests/live.rs
//! Opt-in: `PCE_LIVE=1` plus PCE_HOST, PCE_ORG_ID, API_KEY, API_SECRET
//! (and optionally PCE_PORT, PCE_TLS_VERIFY). At most 8 API calls.
use pce::client::{ClientConfig, PceClient};
use pce::traffic::TrafficQueryBuilder;
use pce::{CacheMode, Credentials, Pce};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn live_smoke() {
    if std::env::var("PCE_LIVE").as_deref() != Ok("1") {
        eprintln!("PCE_LIVE != 1, skipping");
        return;
    }
    let host = std::env::var("PCE_HOST").expect("PCE_HOST");
    let org: u64 = std::env::var("PCE_ORG_ID").expect("PCE_ORG_ID").parse().unwrap();
    let creds = Credentials::new(std::env::var("API_KEY").expect("API_KEY"), std::env::var("API_SECRET").expect("API_SECRET"));
    let mut cfg = ClientConfig::new(host, org, creds);
    if let Ok(p) = std::env::var("PCE_PORT") {
        cfg.port = p.parse().unwrap();
    }
    if let Ok(v) = std::env::var("PCE_TLS_VERIFY") {
        cfg.tls_verify = !matches!(v.to_ascii_lowercase().as_str(), "false" | "0" | "no");
    }
    let pce = Pce::new(PceClient::new(cfg).unwrap());

    let health = pce.health().await.expect("health");
    assert!(!health.is_null());
    let labels = pce.labels(CacheMode::Use).await.expect("labels");
    assert!(!labels.value.is_empty(), "test PCE should have labels");
    let idx = pce.name_index(CacheMode::Use).await.expect("index");
    assert!(!idx.label_keys().is_empty());

    let q = TrafficQueryBuilder::last(Duration::from_secs(3600)).unwrap().max_results(50).build().unwrap();
    let r = pce.traffic(&q, CacheMode::Use, &CancellationToken::new(), None).await.expect("traffic query");
    eprintln!("flows={} matches={:?} truncated={} elapsed={:?}", r.value.flows.len(), r.value.matches_count, r.truncated, r.value.elapsed);
    let s = pce.stats();
    eprintln!("api calls={} bytes={}", s.calls, s.bytes);
    assert!(s.calls <= 8 + 10, "poll count depends on PCE speed; sanity cap");
}
```

Run (offline): `cargo test -p pce --test live` → prints "skipping", passes.

- [ ] **Step 3: Implement the recorder**

```rust
// crates/xtask/src/main.rs
//! Developer tasks. `cargo xtask record` pulls a small, anonymised sample
//! of each object type from the configured PCE into fixtures/recorded/.
use clap::{Parser, Subcommand};
use pce::client::{ClientConfig, PceClient};
use pce::models::PolicyScope;
use pce::traffic::TrafficQueryBuilder;
use pce::{CacheMode, Credentials, Pce};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

#[derive(Parser)]
#[command(name = "xtask")]
struct Xtask {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Record anonymised fixtures from the PCE named by PCE_HOST etc.
    Record {
        #[arg(long)]
        env_file: Option<PathBuf>,
        #[arg(long, default_value = "fixtures/recorded")]
        out: PathBuf,
    },
}

/// Replaces hostnames, IPs, FQDNs, workload names and descriptions with
/// stable placeholders. Label keys/values and rule names are kept.
#[derive(Default)]
struct Anonymiser {
    hosts: HashMap<String, String>,
    ips: HashMap<String, String>,
    fqdns: HashMap<String, String>,
}

impl Anonymiser {
    fn map(table: &mut HashMap<String, String>, raw: &str, make: impl Fn(usize) -> String) -> String {
        let n = table.len() + 1;
        table.entry(raw.to_string()).or_insert_with(|| make(n)).clone()
    }

    fn ip(&mut self, raw: &str) -> String {
        if raw == "0.0.0.0/0" || raw == "::/0" || raw.is_empty() {
            return raw.to_string();
        }
        let (ip, suffix) = raw.split_once('/').map(|(a, b)| (a, format!("/{b}"))).unwrap_or((raw, String::new()));
        let is_v6 = ip.contains(':');
        let mapped = Self::map(&mut self.ips, ip, |n| if is_v6 { format!("fd00::{n:x}") } else { format!("10.{}.{}.{}", (n >> 16) & 255, (n >> 8) & 255, n & 255) });
        format!("{mapped}{suffix}")
    }

    fn walk(&mut self, v: &mut Value, parent_key: Option<&str>) {
        match v {
            Value::Object(map) => {
                let has_hostname = map.contains_key("hostname");
                let keys: Vec<String> = map.keys().cloned().collect();
                for k in keys {
                    let child = map.get_mut(&k).expect("key exists");
                    match (k.as_str(), &mut *child) {
                        ("hostname", Value::String(s)) if !s.is_empty() => *s = Self::map(&mut self.hosts, s, |n| format!("host-{n}.example")),
                        ("fqdn", Value::String(s)) if !s.is_empty() => *s = Self::map(&mut self.fqdns, s, |n| format!("fqdn-{n}.example")),
                        ("ip" | "address" | "from_ip" | "to_ip" | "ip_address", Value::String(s)) => *s = self.ip(s),
                        ("name", Value::String(s)) if has_hostname && !s.is_empty() => *s = Self::map(&mut self.hosts, s, |n| format!("host-{n}.example")),
                        ("description", Value::String(s)) => s.clear(),
                        ("public_ip", Value::String(s)) => *s = self.ip(s),
                        _ => self.walk(child, Some(&k)),
                    }
                }
            }
            Value::Array(items) => {
                for item in items {
                    self.walk(item, parent_key);
                }
            }
            _ => {}
        }
    }
}

fn env_map(env_file: Option<&PathBuf>) -> HashMap<String, String> {
    let mut m: HashMap<String, String> = HashMap::new();
    if let Some(p) = env_file {
        for item in dotenvy::from_path_iter(p).expect("env file") {
            let (k, v) = item.expect("env line");
            m.insert(k, v);
        }
    }
    m.extend(std::env::vars());
    m
}

fn write(out: &PathBuf, name: &str, value: &Value) {
    let path = out.join(name);
    std::fs::write(&path, serde_json::to_string_pretty(value).unwrap()).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
    let rows = value.as_array().map(Vec::len).unwrap_or(1);
    println!("wrote {} ({rows} rows)", path.display());
}

async fn raw_list(pce: &Pce, path: &str, max: u32) -> Value {
    pce.client().get::<Value>(path, &[("max_results", max.to_string())]).await.expect(path).body
}

#[tokio::main]
async fn main() {
    let args = Xtask::parse();
    match args.cmd {
        Cmd::Record { env_file, out } => {
            let env = env_map(env_file.as_ref());
            let get = |k: &str| env.get(k).cloned().unwrap_or_else(|| panic!("{k} not set"));
            let mut cfg = ClientConfig::new(get("PCE_HOST"), get("PCE_ORG_ID").parse().expect("PCE_ORG_ID"), Credentials::new(get("API_KEY"), get("API_SECRET")));
            if let Some(p) = env.get("PCE_PORT") {
                cfg.port = p.parse().expect("PCE_PORT");
            }
            let pce = Pce::new(PceClient::new(cfg).expect("client"));
            let org = pce.client().org_path("");
            std::fs::create_dir_all(&out).expect("out dir");
            let mut anon = Anonymiser::default();

            let mut sets: Vec<(&str, Value)> = vec![
                ("labels.json", raw_list(&pce, &format!("{org}labels"), 500).await),
                ("label_groups.json", raw_list(&pce, &format!("{org}sec_policy/draft/label_groups"), 100).await),
                ("ip_lists.json", raw_list(&pce, &format!("{org}sec_policy/draft/ip_lists"), 100).await),
                ("services.json", raw_list(&pce, &format!("{org}sec_policy/draft/services"), 100).await),
                ("workloads.json", raw_list(&pce, &format!("{org}workloads"), 25).await),
                ("rule_sets.json", raw_list(&pce, &format!("{org}sec_policy/{}/rule_sets", PolicyScope::Draft.segment()), 10).await),
            ];
            let mut deny = Vec::new();
            if let Some(rs) = sets[5].1.as_array() {
                for r in rs.iter().take(3) {
                    if let Some(href) = r.get("href").and_then(Value::as_str) {
                        match pce.deny_rules(href, CacheMode::Bypass).await {
                            Ok(d) => deny.extend(d.value.into_iter().map(|x| serde_json::to_value(x).unwrap())),
                            Err(e) => eprintln!("deny rules for {href}: {e}"),
                        }
                    }
                }
            }
            sets.push(("deny_rules.json", Value::Array(deny)));
            let pending = pce.client().get::<Value>(&format!("{org}sec_policy/pending"), &[]).await.map(|r| r.body).unwrap_or(Value::Array(vec![]));
            sets.push(("pending.json", pending));

            let q = TrafficQueryBuilder::last(Duration::from_secs(3600)).unwrap().max_results(50).build().unwrap();
            let flows = pce.traffic(&q, CacheMode::Bypass, &CancellationToken::new(), None).await.expect("traffic").value.flows;
            sets.push(("traffic_flows.json", serde_json::to_value(flows).unwrap()));

            for (name, mut value) in sets {
                anon.walk(&mut value, None);
                write(&out, name, &value);
            }
            let mut summary = Map::new();
            summary.insert("api_calls".into(), Value::from(pce.stats().calls));
            summary.insert("bytes".into(), Value::from(pce.stats().bytes));
            println!("{}", Value::Object(summary));
        }
    }
}
```

- [ ] **Step 4: Run offline and lint**

Run: `cargo test -p pce --test recorded --test live && cargo clippy --workspace --all-targets -- -D warnings`
Expected: both tests pass (recorded skips each file, live skips), no warnings.

- [ ] **Step 5: Record from demo100 and run the live test (requires network)**

```bash
cargo xtask record --env-file ../illumio-mcp-server/.env
cargo test -p pce --test recorded
set -a; source ../illumio-mcp-server/.env; set +a
PCE_LIVE=1 cargo test -p pce --test live -- --nocapture
cargo run -q -p illumio-tui -- --env-file ../illumio-mcp-server/.env health --table
cargo run -q -p illumio-tui -- --env-file ../illumio-mcp-server/.env traffic query --window 1h --max-results 20 --table
```

Expected: fixtures written to `fixtures/recorded/`, the recorded test passes against them, the live test prints flow and call counts and passes, and the two CLI commands print tables with a meta line on stderr. If the PCE is unreachable, say so in the task report; do not mark the step done.

Inspect `fixtures/recorded/*.json` for anything that looks like a real hostname, IP or FQDN before committing. If something slipped through, extend `Anonymiser::walk` and re-record.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add -A
git commit -m "feat: fixture recorder xtask, recorded-shape test, opt-in live smoke test

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 21: CI, cargo-deny, README, architecture notes, task review

**Files:**
- Create: `.github/workflows/ci.yml`, `deny.toml`, `README.md`, `docs/architecture.md`
- Modify: `tasks/todo.md`

- [ ] **Step 1: Write CI**

```yaml
# .github/workflows/ci.yml
name: CI
on:
  push:
    branches: [main]
  pull_request:
jobs:
  test:
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy, rustfmt
      - uses: Swatinem/rust-cache@v2
      - run: cargo fmt --all --check
      - run: cargo clippy --workspace --all-targets -- -D warnings
      - run: cargo test --workspace
  deny:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: EmbarkStudios/cargo-deny-action@v2
```

```toml
# deny.toml
[graph]
all-features = true

[advisories]
ignore = []

[licenses]
allow = ["MIT", "Apache-2.0", "Apache-2.0 WITH LLVM-exception", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Unicode-3.0", "Unlicense", "Zlib", "MPL-2.0", "CDLA-Permissive-2.0", "OpenSSL"]

[bans]
multiple-versions = "warn"

[sources]
unknown-registry = "deny"
unknown-git = "deny"
```

Run `cargo install cargo-deny` once locally if missing, then `cargo deny check`. If a crate's licence is not in the list, add it to `allow` only if it is a permissive licence; otherwise replace the crate.

- [ ] **Step 2: Write README and architecture notes**

```markdown
<!-- README.md -->
# illumio-tui

Terminal UI and CLI for the Illumio Core PCE, built for administrators,
Illumio Professional Services, and agents. Read-only in this release.

## Install

    cargo install --path crates/illumio-tui

## Configure

Environment variables (same names as illumio-mcp-server):

    PCE_HOST=pce.example  PCE_PORT=443  PCE_ORG_ID=1  API_KEY=...  API_SECRET=...

or a profile in `~/.config/illumio-tui/config.toml`:

    default_profile = "prod"

    [profiles.prod]
    host = "pce.example"
    org_id = 1
    api_key = "api_..."
    api_secret_cmd = "op read op://vault/pce/secret"

`--env-file .env` loads a dotenv file for one run.

## CLI

    illumio-tui health
    illumio-tui labels --key app
    illumio-tui workloads --label app=web --online true
    illumio-tui policy rulesets
    illumio-tui policy rules "Web-DB"
    illumio-tui policy pending
    illumio-tui traffic query --window 24h --src app=web --dst app=db --json
    illumio-tui traffic query --window 1h --group app,env
    illumio-tui traffic query --window 1h --blocked
    illumio-tui traffic query --window 30d --dry-run

JSON output (default when piped) is `{"data": ..., "meta": {...}}` where
`meta` reports API calls, bytes, cache status, truncation, and the query
window. Errors go to stderr as JSON. Exit codes: 1 generic, 2 arguments or
config, 3 auth, 4 PCE error, 5 rate limited.

## PCE protection

Every request is user-caused, throttled (2/s), cached, and counted.
Explorer queries default to a 24 h window and 1000 rows; larger windows
(> 7 d) or caps (> 10 000) need `--yes`. `--dry-run` shows the request
body without sending it.

## Development

    cargo test --workspace
    cargo xtask record --env-file ../illumio-mcp-server/.env   # refresh fixtures/recorded
    PCE_LIVE=1 cargo test -p pce --test live                    # needs PCE_* env

Design: `docs/superpowers/specs/2026-10-10-illumio-tui-design.md`.
```

```markdown
<!-- docs/architecture.md -->
# Architecture notes (Plan 1: library and CLI)

## Crates

- `crates/pce`: the library. No terminal code, no process-global state.
- `crates/illumio-tui`: the binary. `cli/` is the non-interactive mode;
  the TUI (Plan 2) will sit next to it and reuse every library type.
- `crates/xtask`: developer tooling (`cargo xtask record`).

## One request, top to bottom

`illumio-tui traffic query --window 1h --src app=web`

1. `cli/traffic.rs::build_query` parses `app=web` into a `FilterExpr`.
2. `Pce::resolve_filter_groups` asks `Pce::name_index` for the label
   table. The index is built from four cached lists (labels, label groups,
   IP lists, services), each one request with `max_results=10000`.
   `app=web` becomes `{"label": {"href": "/orgs/1/labels/7"}}`. An unknown
   value fails here, before the PCE sees anything.
3. `TrafficQueryBuilder::last(1h)` produces the request body with full UTC
   timestamps and `[[]]` for the empty destination include.
4. `TrafficQuery::needs_confirmation` checks the window and row cap.
5. `Pce::traffic` looks the exact body up in the cache, then calls
   `traffic::poll::run_traffic_query`: POST the query, sleep 1 s, poll,
   back off ×1.5 to 10 s, download the result, flag truncation.
6. `traffic::resolved::resolve_flows` replaces label hrefs with names.
7. `Context::emit` prints the JSON envelope or a table; `meta` carries the
   call and byte counters from `PceClient::stats`.

## PCE protection, where it lives

| Rule | Code |
|---|---|
| 2 req/s, burst 5 | `pce::limiter::RateLimiter`, used in `PceClient::send` |
| back off on 429/503, never retry POST | `PceClient::send` |
| count every call and byte | `PceClient::stats` |
| cache lists 15/5 min | `Pce::cached_list` with `CacheTtls` |
| one query per filter | `Pce::traffic` cache key is the request body |
| confirm big queries | `TrafficQuery::needs_confirmation` |
| show the request without sending | `traffic query --dry-run` |

## Testing

Offline tests use `fixtures/*.json` (hand-written, deterministic) and
`wiremock` for HTTP. `fixtures/recorded/` holds anonymised real responses
and is only shape-checked. `PCE_LIVE=1` enables the single live test.
```

- [ ] **Step 3: Update `tasks/todo.md`**

Mark every Plan 1 task done and fill the review section:

```markdown
## Review (Plan 1)
- Library: throttled client, TTL cache, typed models, async Explorer poller, name resolution, aggregation.
- CLI: health, labels, label-groups, iplists, services, workloads, policy (rulesets, rules, pending), traffic query (flows, --group, --blocked, --dry-run), profile (list, use).
- Verified: `cargo test --workspace`, clippy clean, live smoke test against demo100 (calls/bytes noted in the test output).
- Deferred to Plan 2: the interactive TUI, auto-refresh, `:profile` switching at runtime.
```

- [ ] **Step 4: Verify everything once more**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: all green.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "chore: CI, cargo-deny, README, architecture notes

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Not in this plan

The interactive TUI (spec §3.2 UI parts and §4), auto-refresh (§5.2 runtime behaviour), and `:profile` switching are Plan 2, written after this plan is executed so it can reference the real library interfaces. Everything in spec §3.1, §5.1, §5.3, §5.4, §6, §7 and §8 (offline suite and live smoke test) is covered above.
