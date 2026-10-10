# PCE Writes (Plan 3: labels, IP lists, services, unmanaged workloads) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a guarded, audited, never-retried write layer to the `pce` library and agent-facing CLI subcommands that create, update and delete labels, IP lists, services and unmanaged workloads.

**Architecture:** `PceClient::write` is the one HTTP path for mutations (no retry, write-specific error mapping). `pce::write` holds typed drafts with `validate()`/`to_body()`, a `WriteGuard` (profile + flag + audit sink) that is the only way to get a `Writer`, and a two-phase API: pure `plan_*` functions produce a `Plan` (the exact request, or a no-op), `Writer::execute` sends it once, audits it, invalidates the cache, and re-fetches when the PCE answers 204. The CLI adds `writes = true` / `--allow-writes`, `--dry-run` / `--yes`, exit code 6, and `add`/`update`/`delete` subcommands under the existing `labels`, `iplists`, `services` and `workloads` commands.

**Tech Stack:** Rust 2024 (rust-version 1.88), tokio, reqwest 0.13, serde/serde_json, thiserror 2, jiff 0.2, clap 4 (derive), wiremock 0.6, assert_cmd, tempfile, predicates.

**Spec:** `docs/superpowers/specs/2026-10-10-pce-writes-design.md` (and GitHub issue #1 for the safety model).

## Global Constraints

- Every PCE request is user-caused. No write command issues a request before the guard, the `--yes` check and local validation have passed. Lookups for `--if-missing` and name resolution are the only reads a write command makes.
- POST, PUT and DELETE are sent exactly once. `PceClient::write` passes `retry = false` to `send`; a 429 or 503 on a write is an error, never a retry.
- Draft is the only policy target: hrefs containing `/sec_policy/active/` are rewritten to `/sec_policy/draft/`; any other `sec_policy/<scope>/` is refused.
- Audit log path: `$XDG_STATE_HOME/illumio-tui/writes.jsonl`, else `~/.local/state/illumio-tui/writes.jsonl`; created with mode 0600 on Unix; one JSON object per line; dry runs and no-ops are never logged.
- Exit codes: 0 ok / dry run / no-op; 2 args, config, validation, unknown or ambiguous name, missing `--yes`, managed workload; 3 auth; 4 PCE error incl. `rejected` and `write_unanswered`; 5 rate limited; 6 write refused by the guard. Codes 1–5 and every existing command, flag and envelope field stay unchanged.
- The PCE's `?name=`, `?key=`, `?hostname=` filters are substring matches; every "exists?" decision compares exactly, client-side.
- A service body carries exactly one port list key, `service_ports`; `windows_services` is never sent. Deny-rule and `resolve_labels_as` rules from issue #1 do not apply to this plan (no rules yet).
- No output, log line, audit entry or error may contain the API secret (`Credentials` Debug is redacted; request bodies never carry credentials).
- Commit trailer on every commit, exactly: `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`.
- Gate before every commit: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` (or the narrower `-p pce` / `-p illumio-tui` where the task says so, with the full workspace gate in the final task).

## Review Focus

1. A gateway status (502, 503, 504) on a write: the PCE may have applied the change. Expected: the error says `write may have applied; check the PCE before retrying`, the audit entry is written, and nothing is retried. Test: Task 3 `gateway_status_on_a_write_is_reported_as_unanswered`.
2. `--if-missing` when the only exact match is a label the PCE marks `deleted: true`. Expected: the label is created, not skipped. Test: Task 5 `if_missing_ignores_deleted_labels`.
3. A label value that contains `=` (`labels add app=a=b`). Expected: key `app`, value `a=b` (split on the first `=`). Test: Task 10 `label_spec_splits_on_the_first_equals`.
4. `iplists update --remove-range` that removes every range of a list with no FQDNs, or `workloads update --remove-ip` that removes the last interface of a workload without a hostname. Expected: refused locally with exit 2, nothing sent. Tests: Task 6 `removing_the_last_range_is_refused`, Task 8 `removing_the_last_interface_without_a_hostname_is_refused`.
5. A CIDR range (`--range 10.0.0.0/8`). Expected: sent as `{"from_ip": "10.0.0.0/8"}` with no `to_ip`, round-trips through `--remove-range 10.0.0.0/8`. Test: Task 4 `cidr_stays_in_from_ip`, Task 6 `remove_range_matches_a_cidr`.

---

## File structure

| File | Responsibility |
| --- | --- |
| `crates/pce/src/client.rs` | add `WriteMethod` and `PceClient::write` (no retry, write status mapping) |
| `crates/pce/src/error.rs` | add `PceError::Rejected { status, body }` (kind `rejected`) |
| `crates/pce/src/write/mod.rs` | `WriteRequest`, `Plan`, `Written`, `Outcome`, `ObjectKind`, `WriteError`, `IfMissing`, `draft_href`, `Writer` + `execute`, `Pce::writer`, re-exports |
| `crates/pce/src/write/audit.rs` | `AuditEntry`, `AuditSink`, `JsonlAudit`, `NullAudit`, `MemoryAudit` |
| `crates/pce/src/write/guard.rs` | `WriteContext`, `WriteGuard`, `WriteRefused` |
| `crates/pce/src/write/specs.rs` | `IpRangeSpec`, `PortSpec`, `InterfaceSpec` parsers and JSON forms |
| `crates/pce/src/write/labels.rs` | `LabelDraft`, `Writer::plan_create_label / plan_update_label / plan_delete_label` |
| `crates/pce/src/write/ip_lists.rs` | `IpListDraft`, `IpListPatch`, `Writer::plan_create_ip_list / plan_update_ip_list / plan_delete_ip_list` |
| `crates/pce/src/write/services.rs` | `ServiceDraft`, `ServicePatch`, `Writer::plan_create_service / plan_update_service / plan_delete_service` |
| `crates/pce/src/write/workloads.rs` | `WorkloadDraft`, `WorkloadPatch`, `LabelRemoval`, `Writer::find_workload / plan_create_workload / plan_update_workload / plan_delete_workload` |
| `crates/pce/src/resolve.rs` | add exact-name lookups `ip_lists_named`, `services_named` |
| `crates/pce/src/lib.rs` | `pub mod write;` and `Pce::writer` |
| `crates/pce/tests/write_contract.rs` | wiremock contract tests for the write layer |
| `crates/pce/tests/live_writes.rs` | opt-in live smoke (`PCE_LIVE_WRITES=1`) |
| `crates/illumio-tui/src/config.rs` | `Profile.writes`, `ILLUMIO_WRITES`, `ResolvedConfig.writes` |
| `crates/illumio-tui/src/cli/mod.rs` | `--allow-writes`, subcommand enums for `labels`, `iplists`, `services`, `workloads` |
| `crates/illumio-tui/src/cli/error.rs` | `CliError::Write(WriteError)` → exit 6 / 2 / 4 |
| `crates/illumio-tui/src/cli/context.rs` | `Context.allow_writes`, `emit_write` |
| `crates/illumio-tui/src/cli/output.rs` | `Meta.write: Option<WriteMeta>` |
| `crates/illumio-tui/src/cli/write.rs` | `WriteFlags`, `write_guard`, `check_flags`, `WriteOutput`, `run_plan`, name resolvers, `parse_kv` |
| `crates/illumio-tui/src/cli/health.rs` | `HealthReport.writes` |
| `crates/illumio-tui/src/cli/objects.rs` | `LabelsCmd`, `IplistsCmd`, `ServicesCmd` with `add`/`update`/`delete` |
| `crates/illumio-tui/src/cli/workloads.rs` | `WorkloadsCmd` with `add`/`update`/`delete` |
| `crates/illumio-tui/tests/cli_writes.rs` | assert_cmd + wiremock tests for every write command |
| `README.md`, `docs/architecture.md`, `docs/index.html`, `tasks/todo.md`, `tasks/lessons.md` | docs and tracking |

---

### Task 1: `PceClient::write` and `PceError::Rejected`

**Files:**
- Modify: `crates/pce/src/client.rs` (after `post`, ~line 150; `decode` ~line 230)
- Modify: `crates/pce/src/error.rs`
- Test: `crates/pce/tests/client.rs`

**Interfaces:**
- Consumes: `PceClient::send(build, retry)`, `pce_error_message`, `ApiResponse<T>`.
- Produces: `pub enum WriteMethod { Post, Put, Delete }` with `as_str()`, serde `"POST"|"PUT"|"DELETE"`; `pub async fn write(&self, method: WriteMethod, api_path: &str, body: Option<&serde_json::Value>) -> Result<ApiResponse<serde_json::Value>>`; `PceError::Rejected { status: u16, body: String }` with kind `"rejected"`.

- [ ] **Step 1: Write the failing tests** (append to `crates/pce/tests/client.rs`)

```rust
use pce::client::WriteMethod;

#[tokio::test]
async fn write_sends_json_body_once_and_decodes_201() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/labels"))
        .and(header("Authorization", "Basic a2V5OnNlY3JldA=="))
        .and(header("Content-Type", "application/json"))
        .and(wiremock::matchers::body_json(json!({"key": "app", "value": "web"})))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_json(json!({"href": "/orgs/5/labels/9", "key": "app", "value": "web"})),
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
        .write(WriteMethod::Put, "/orgs/5/labels/9", Some(&json!({"value": "x"})))
        .await
        .unwrap();
    assert_eq!(r.status, 204);
    assert!(r.body.is_null());
    let r = c.write(WriteMethod::Delete, "/orgs/5/labels/9", None).await.unwrap();
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
    let err = c.write(WriteMethod::Delete, "/orgs/5/y", None).await.unwrap_err();
    assert!(matches!(err, PceError::Server { status: 503, .. }), "{err:?}");
    assert_eq!(c.stats().calls, 2);
}
```

Note: `client_for` in this file sets `backoff_base` to 1 ms and the limiter to 1000 rps; keep using it. `header` and `body_json` come from `wiremock::matchers`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p pce --test client write_`
Expected: compile error, `no variant named Rejected` / `no method named write`.

- [ ] **Step 3: Add `Rejected` to `crates/pce/src/error.rs`**

After the `Server` variant:

```rust
    /// A write the PCE refused on validation grounds (HTTP 400, 406, 409, 422).
    #[error("the PCE rejected the write (HTTP {status}): {body}")]
    Rejected { status: u16, body: String },
```

In `kind()` add `PceError::Rejected { .. } => "rejected",`. Extend the `kinds_are_stable_snake_case` unit test with
`assert_eq!(PceError::Rejected { status: 406, body: "x".into() }.kind(), "rejected");`.

- [ ] **Step 4: Add `WriteMethod` and `write` to `crates/pce/src/client.rs`**

Near the top, after `StatsSnapshot`:

```rust
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
```

After `post`:

```rust
    /// Sends one mutation. Never retried: a 429 or 503 comes back as an
    /// error, because a repeated write could apply a change twice. 400, 406,
    /// 409 and 422 map to [`PceError::Rejected`]; a 204 or empty body decodes
    /// to `Value::Null`.
    pub async fn write(
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
                Some(p) => r
                    .header("Content-Type", "application/json")
                    .body(p.clone()),
                None => r,
            }
        };
        let (status, headers, bytes) = self.send(build, false).await?;
        match status.as_u16() {
            400 | 406 | 409 | 422 => Err(PceError::Rejected {
                status: status.as_u16(),
                body: pce_error_message(&bytes),
            }),
            _ => Self::decode(api_path, status, &headers, &bytes),
        }
    }
```

`decode` already turns an empty body into `null`, 401/403 into `Auth`, 404 into `NotFound`, and other non-2xx into `Server`.

- [ ] **Step 5: Export and run**

In `crates/pce/src/lib.rs` change the client re-export to
`pub use client::{ApiResponse, ClientConfig, PceClient, StatsSnapshot, WriteMethod};`.

Run: `cargo test -p pce --test client` and `cargo test -p pce --lib error`
Expected: all pass, including the four new tests.

- [ ] **Step 6: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy -p pce --all-targets -- -D warnings && cargo test -p pce
git add crates/pce/src/client.rs crates/pce/src/error.rs crates/pce/src/lib.rs crates/pce/tests/client.rs
git commit -m "feat(pce): PceClient::write with no retry and a Rejected error for 400/406/409/422

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 2: Audit sink

**Files:**
- Create: `crates/pce/src/write/audit.rs`
- Create: `crates/pce/src/write/mod.rs` (minimal: `pub mod audit;` plus re-exports; Task 3 fills it)
- Modify: `crates/pce/src/lib.rs` (`pub mod write;`)
- Modify: `crates/pce/Cargo.toml` (add `tempfile` to dev-dependencies: `tempfile.workspace = true`)

**Interfaces:**
- Consumes: `WriteMethod` (Task 1), `jiff::Timestamp`.
- Produces:

```rust
pub struct AuditEntry { pub ts: String, pub profile: String, pub host: String, pub org_id: u64,
    pub method: WriteMethod, pub path: String, pub body: Option<Value>, pub status: Option<u16>,
    pub href: Option<String>, pub error: Option<String>, pub phase: Option<String>, pub source: String }
pub trait AuditSink: Send + Sync {
    fn check_writable(&self) -> std::io::Result<()>;
    fn append(&self, entry: &AuditEntry) -> std::io::Result<()>;
    fn location(&self) -> String;
}
pub struct JsonlAudit;  // JsonlAudit::new(path), JsonlAudit::default_path() -> PathBuf
pub struct NullAudit;   // for the TUI tests and dry runs; location() == "none"
pub struct MemoryAudit; // MemoryAudit::new() -> Arc<MemoryAudit>; entries() -> Vec<AuditEntry>
```

- [ ] **Step 1: Write the failing unit tests** (inside `audit.rs`, `#[cfg(test)] mod tests`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry() -> AuditEntry {
        AuditEntry {
            ts: "2026-10-10T10:00:00Z".into(),
            profile: "lab".into(),
            host: "pce.example".into(),
            org_id: 5,
            method: WriteMethod::Post,
            path: "/orgs/5/labels".into(),
            body: Some(json!({"key": "app", "value": "web"})),
            status: Some(201),
            href: Some("/orgs/5/labels/9".into()),
            error: None,
            phase: None,
            source: "cli".into(),
        }
    }

    #[test]
    fn jsonl_appends_one_line_per_entry_and_creates_parents() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state").join("illumio-tui").join("writes.jsonl");
        let audit = JsonlAudit::new(path.clone());
        audit.check_writable().unwrap();
        audit.append(&entry()).unwrap();
        audit.append(&entry()).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        let v: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(v["method"], "POST");
        assert_eq!(v["status"], 201);
        assert_eq!(v["body"]["key"], "app");
        assert!(v.get("error").is_none(), "absent fields are omitted");
        assert_eq!(audit.location(), path.display().to_string());
    }

    #[cfg(unix)]
    #[test]
    fn jsonl_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("writes.jsonl");
        JsonlAudit::new(path.clone()).append(&entry()).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn unwritable_path_fails_check() {
        let dir = tempfile::tempdir().unwrap();
        // A file where the parent directory should be.
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, b"x").unwrap();
        let audit = JsonlAudit::new(blocker.join("writes.jsonl"));
        assert!(audit.check_writable().is_err());
    }

    #[test]
    fn default_path_honours_xdg_state_home() {
        let p = JsonlAudit::path_from(Some(std::path::Path::new("/tmp/xdg-state")), None);
        assert_eq!(p, std::path::PathBuf::from("/tmp/xdg-state/illumio-tui/writes.jsonl"));
        let p = JsonlAudit::path_from(None, Some(std::path::Path::new("/home/u")));
        assert_eq!(
            p,
            std::path::PathBuf::from("/home/u/.local/state/illumio-tui/writes.jsonl")
        );
    }

    #[test]
    fn memory_audit_records_entries() {
        let m = MemoryAudit::new();
        m.append(&entry()).unwrap();
        assert_eq!(m.entries().len(), 1);
        assert_eq!(NullAudit.location(), "none");
        assert!(NullAudit.append(&entry()).is_ok());
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p pce --lib write::audit`
Expected: compile error (module missing).

- [ ] **Step 3: Implement `crates/pce/src/write/audit.rs`**

```rust
//! The local audit log of writes: one JSON line per sent request, written
//! after the response (or with `phase: "sent"` when none arrived). Dry runs
//! and no-ops never reach the log. Request bodies carry no credentials.
use crate::client::WriteMethod;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuditEntry {
    pub ts: String,
    pub profile: String,
    pub host: String,
    pub org_id: u64,
    pub method: WriteMethod,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub href: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// `"sent"` when the request left but no response arrived.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    /// `cli` or `tui`.
    pub source: String,
}

pub trait AuditSink: Send + Sync {
    /// Called before a request is sent; a failure refuses the write.
    fn check_writable(&self) -> std::io::Result<()>;
    fn append(&self, entry: &AuditEntry) -> std::io::Result<()>;
    /// Where entries go, for `meta.write.audit`.
    fn location(&self) -> String;
}

/// Append-only JSON lines file, private to the user.
#[derive(Debug, Clone)]
pub struct JsonlAudit {
    path: PathBuf,
}

impl JsonlAudit {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// `$XDG_STATE_HOME/illumio-tui/writes.jsonl`, else
    /// `~/.local/state/illumio-tui/writes.jsonl`.
    pub fn default_path() -> PathBuf {
        let xdg = std::env::var_os("XDG_STATE_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from);
        let home = std::env::var_os("HOME").map(PathBuf::from);
        Self::path_from(xdg.as_deref(), home.as_deref())
    }

    pub fn path_from(xdg_state_home: Option<&Path>, home: Option<&Path>) -> PathBuf {
        let base = match (xdg_state_home, home) {
            (Some(x), _) => x.to_path_buf(),
            (None, Some(h)) => h.join(".local").join("state"),
            (None, None) => PathBuf::from(".local").join("state"),
        };
        base.join("illumio-tui").join("writes.jsonl")
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn open(&self) -> std::io::Result<File> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut opts = OpenOptions::new();
        opts.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        opts.open(&self.path)
    }
}

impl AuditSink for JsonlAudit {
    fn check_writable(&self) -> std::io::Result<()> {
        self.open().map(|_| ())
    }

    fn append(&self, entry: &AuditEntry) -> std::io::Result<()> {
        let mut f = self.open()?;
        let line = serde_json::to_string(entry).map_err(std::io::Error::other)?;
        f.write_all(line.as_bytes())?;
        f.write_all(b"\n")?;
        f.flush()
    }

    fn location(&self) -> String {
        self.path.display().to_string()
    }
}

/// Discards entries. For tests and for the TUI until it writes.
#[derive(Debug, Default, Clone, Copy)]
pub struct NullAudit;

impl AuditSink for NullAudit {
    fn check_writable(&self) -> std::io::Result<()> {
        Ok(())
    }
    fn append(&self, _entry: &AuditEntry) -> std::io::Result<()> {
        Ok(())
    }
    fn location(&self) -> String {
        "none".into()
    }
}

/// Keeps entries in memory so tests can assert on them.
#[derive(Debug, Default)]
pub struct MemoryAudit {
    entries: Mutex<Vec<AuditEntry>>,
}

impl MemoryAudit {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn entries(&self) -> Vec<AuditEntry> {
        self.entries.lock().expect("audit mutex poisoned").clone()
    }
}

impl AuditSink for MemoryAudit {
    fn check_writable(&self) -> std::io::Result<()> {
        Ok(())
    }
    fn append(&self, entry: &AuditEntry) -> std::io::Result<()> {
        self.entries
            .lock()
            .expect("audit mutex poisoned")
            .push(entry.clone());
        Ok(())
    }
    fn location(&self) -> String {
        "memory".into()
    }
}

impl<T: AuditSink + ?Sized> AuditSink for Arc<T> {
    fn check_writable(&self) -> std::io::Result<()> {
        (**self).check_writable()
    }
    fn append(&self, entry: &AuditEntry) -> std::io::Result<()> {
        (**self).append(entry)
    }
    fn location(&self) -> String {
        (**self).location()
    }
}
```

Create `crates/pce/src/write/mod.rs` with, for now:

```rust
//! Writes to the PCE: guarded, audited, sent once. See the Plan 3 spec.
pub mod audit;

pub use audit::{AuditEntry, AuditSink, JsonlAudit, MemoryAudit, NullAudit};
```

Add `pub mod write;` to `crates/pce/src/lib.rs` (after `pub mod traffic;`) and `tempfile.workspace = true` under `[dev-dependencies]` in `crates/pce/Cargo.toml`. Also update the crate doc comment's first line in `lib.rs` from "read-only" to "a PCE-friendly client library" (the `write` module makes "read-only" false) and the `description` in `crates/pce/Cargo.toml` to `"Async client for the Illumio Core PCE REST API v2: cached reads, guarded writes"`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p pce --lib write::audit`
Expected: 5 tests pass (4 on non-Unix).

- [ ] **Step 5: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy -p pce --all-targets -- -D warnings && cargo test -p pce
git add crates/pce/src/write crates/pce/src/lib.rs crates/pce/Cargo.toml Cargo.lock
git commit -m "feat(pce): JSONL audit sink for writes (0600, XDG state dir, memory and null sinks)

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 3: Guard, plan types and `Writer::execute`

**Files:**
- Create: `crates/pce/src/write/guard.rs`
- Modify: `crates/pce/src/write/mod.rs` (full content below)
- Modify: `crates/pce/src/lib.rs` (`Pce::writer`)
- Test: `crates/pce/tests/write_contract.rs` (new)

**Interfaces:**
- Consumes: `PceClient::write`, `WriteMethod`, `PceError::Rejected`, `AuditSink`, `AuditEntry`, `Pce::invalidate`, `Pce::client()`.
- Produces (all `pub` from `pce::write`):

```rust
pub struct WriteContext { pub profile: String, pub host: String, pub org_id: u64, pub source: String }
pub enum WriteRefused { ProfileDenies, FlagMissing, AuditUnwritable(String) }
pub struct WriteGuard;  // WriteGuard::new(profile_allows, flag_allows, audit: Box<dyn AuditSink>, ctx) -> Result<Self, WriteRefused>; audit_location()
pub enum ObjectKind { Label, IpList, Service, Workload }  // cache_prefix(), pending(), as_str()
pub struct WriteRequest { pub method: WriteMethod, pub path: String, pub body: Option<Value>, pub summary: String }
pub enum Plan { Send { request: WriteRequest, kind: ObjectKind }, Noop { existing_href: String, reason: String } }
pub struct Written { pub href: String, pub pending: bool, pub status: u16, pub object: Value, pub dropped_fields: Vec<String> }
pub enum Outcome { Written(Written), Skipped { existing_href: String, reason: String } }
pub enum WriteError { Refused(WriteRefused), Invalid(Vec<String>), Unanswered(PceError), Pce(PceError) }  // kind()
pub enum IfMissing { Create, SkipExisting }
pub fn draft_href(href: &str) -> Result<String, WriteError>
pub struct Writer<'a>;  // Pce::writer(&self, guard: &WriteGuard) -> Writer<'_>; execute(plan) -> Result<Outcome, WriteError>; fetch(href) -> Result<Value, WriteError>; pce() -> &Pce
```

- [ ] **Step 1: Write the failing contract tests** (`crates/pce/tests/write_contract.rs`)

```rust
use pce::client::{ClientConfig, PceClient};
use pce::write::{
    IfMissing, MemoryAudit, ObjectKind, Outcome, Plan, WriteContext, WriteError, WriteGuard,
    WriteRefused, WriteRequest,
};
use pce::{CacheMode, Credentials, Pce, PceError, WriteMethod};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

pub fn pce_for(server: &MockServer) -> Pce {
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
    cfg.timeout = Duration::from_millis(500);
    Pce::new(PceClient::new(cfg).unwrap())
}

pub fn ctx() -> WriteContext {
    WriteContext {
        profile: "lab".into(),
        host: "pce.example".into(),
        org_id: 5,
        source: "test".into(),
    }
}

pub fn guard_with(audit: Arc<MemoryAudit>) -> WriteGuard {
    WriteGuard::new(true, true, Box::new(audit), ctx()).unwrap()
}

fn send_plan(method: WriteMethod, path: &str, body: Option<Value>, kind: ObjectKind) -> Plan {
    Plan::Send {
        request: WriteRequest {
            method,
            path: path.into(),
            body,
            summary: "test".into(),
        },
        kind,
    }
}

#[test]
fn guard_refuses_without_profile_or_flag() {
    let a = MemoryAudit::new();
    assert!(matches!(
        WriteGuard::new(false, true, Box::new(a.clone()), ctx()).unwrap_err(),
        WriteRefused::ProfileDenies
    ));
    assert!(matches!(
        WriteGuard::new(true, false, Box::new(a.clone()), ctx()).unwrap_err(),
        WriteRefused::FlagMissing
    ));
    assert!(WriteGuard::new(true, true, Box::new(a), ctx()).is_ok());
}

#[tokio::test]
async fn execute_posts_once_audits_and_invalidates_objects() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/labels"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/labels"))
        .and(body_json(json!({"key": "app", "value": "web"})))
        .respond_with(ResponseTemplate::new(201).set_body_json(
            json!({"href": "/orgs/5/labels/9", "key": "app", "value": "web"}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    pce.labels(CacheMode::Use).await.unwrap(); // warm the cache
    let audit = MemoryAudit::new();
    let guard = guard_with(audit.clone());
    let out = pce
        .writer(&guard)
        .execute(send_plan(
            WriteMethod::Post,
            "/orgs/5/labels",
            Some(json!({"key": "app", "value": "web"})),
            ObjectKind::Label,
        ))
        .await
        .unwrap();
    let Outcome::Written(w) = out else {
        panic!("expected Written")
    };
    assert_eq!(w.href, "/orgs/5/labels/9");
    assert_eq!(w.status, 201);
    assert!(!w.pending, "labels are not provisionable");
    assert_eq!(w.object["key"], "app");
    assert!(w.dropped_fields.is_empty());
    let entries = audit.entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].method, WriteMethod::Post);
    assert_eq!(entries[0].path, "/orgs/5/labels");
    assert_eq!(entries[0].status, Some(201));
    assert_eq!(entries[0].href.as_deref(), Some("/orgs/5/labels/9"));
    assert_eq!(entries[0].profile, "lab");
    assert_eq!(entries[0].source, "test");
    assert!(entries[0].phase.is_none());
    // The objects cache was invalidated: the next read hits the PCE again.
    let c = pce.labels(CacheMode::Use).await.unwrap();
    assert!(!c.hit);
}

#[tokio::test]
async fn execute_refetches_after_204_and_marks_policy_objects_pending() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/ip_lists/3"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/ip_lists/3"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "href": "/orgs/5/sec_policy/draft/ip_lists/3", "name": "Corp",
            "ip_ranges": [{"from_ip": "10.0.0.0/8"}], "fqdns": [], "update_type": "update"
        })))
        .expect(1)
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    let audit = MemoryAudit::new();
    let guard = guard_with(audit.clone());
    let out = pce
        .writer(&guard)
        .execute(send_plan(
            WriteMethod::Put,
            "/orgs/5/sec_policy/draft/ip_lists/3",
            Some(json!({"name": "Corp", "ip_ranges": [{"from_ip": "10.0.0.0/8"}], "fqdns": []})),
            ObjectKind::IpList,
        ))
        .await
        .unwrap();
    let Outcome::Written(w) = out else {
        panic!("expected Written")
    };
    assert_eq!(w.status, 204);
    assert!(w.pending);
    assert_eq!(w.object["name"], "Corp");
    assert_eq!(w.href, "/orgs/5/sec_policy/draft/ip_lists/3");
    assert_eq!(audit.entries()[0].status, Some(204));
}

#[tokio::test]
async fn execute_reports_fields_the_pce_dropped() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/services"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "href": "/orgs/5/sec_policy/draft/services/7", "name": "PG", "service_ports": []
        })))
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    let guard = guard_with(MemoryAudit::new());
    let out = pce
        .writer(&guard)
        .execute(send_plan(
            WriteMethod::Post,
            "/orgs/5/sec_policy/draft/services",
            Some(json!({"name": "PG", "service_ports": [{"port": 5432, "proto": 6}]})),
            ObjectKind::Service,
        ))
        .await
        .unwrap();
    let Outcome::Written(w) = out else {
        panic!("expected Written")
    };
    assert_eq!(w.dropped_fields, vec!["service_ports".to_string()]);
}

#[tokio::test]
async fn delete_returns_the_request_href_and_no_object() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/v2/orgs/5/labels/9"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    let audit = MemoryAudit::new();
    let guard = guard_with(audit.clone());
    let out = pce
        .writer(&guard)
        .execute(send_plan(WriteMethod::Delete, "/orgs/5/labels/9", None, ObjectKind::Label))
        .await
        .unwrap();
    let Outcome::Written(w) = out else {
        panic!("expected Written")
    };
    assert_eq!(w.href, "/orgs/5/labels/9");
    assert!(w.object.is_null());
    assert_eq!(server.received_requests().await.unwrap().len(), 1, "no re-fetch after delete");
    assert_eq!(audit.entries()[0].method, WriteMethod::Delete);
}

#[tokio::test]
async fn rejected_write_is_audited_with_the_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/labels"))
        .respond_with(ResponseTemplate::new(406).set_body_json(
            json!([{"token": "label_in_use", "message": "Label is in use"}]),
        ))
        .expect(1)
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    let audit = MemoryAudit::new();
    let guard = guard_with(audit.clone());
    let err = pce
        .writer(&guard)
        .execute(send_plan(WriteMethod::Post, "/orgs/5/labels", Some(json!({})), ObjectKind::Label))
        .await
        .unwrap_err();
    assert!(matches!(err, WriteError::Pce(PceError::Rejected { status: 406, .. })), "{err:?}");
    assert_eq!(err.kind(), "rejected");
    let e = &audit.entries()[0];
    assert_eq!(e.status, Some(406));
    assert!(e.error.as_deref().unwrap().contains("label_in_use"));
}

#[tokio::test]
async fn gateway_status_on_a_write_is_reported_as_unanswered() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/labels"))
        .respond_with(ResponseTemplate::new(502).set_body_string("bad gateway"))
        .expect(1)
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    let audit = MemoryAudit::new();
    let guard = guard_with(audit.clone());
    let err = pce
        .writer(&guard)
        .execute(send_plan(WriteMethod::Post, "/orgs/5/labels", Some(json!({})), ObjectKind::Label))
        .await
        .unwrap_err();
    assert!(matches!(err, WriteError::Unanswered(_)), "{err:?}");
    assert!(err.to_string().contains("write may have applied"));
    let e = &audit.entries()[0];
    assert_eq!(e.phase.as_deref(), Some("sent"));
    assert_eq!(e.status, Some(502));
}

#[tokio::test]
async fn timeout_on_a_write_is_unanswered_and_audited_as_sent() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/labels"))
        .respond_with(ResponseTemplate::new(201).set_delay(Duration::from_secs(3)))
        .expect(1)
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    let audit = MemoryAudit::new();
    let guard = guard_with(audit.clone());
    let err = pce
        .writer(&guard)
        .execute(send_plan(WriteMethod::Post, "/orgs/5/labels", Some(json!({})), ObjectKind::Label))
        .await
        .unwrap_err();
    assert!(matches!(err, WriteError::Unanswered(PceError::Timeout(_))), "{err:?}");
    assert_eq!(err.kind(), "write_unanswered");
    let e = &audit.entries()[0];
    assert_eq!(e.phase.as_deref(), Some("sent"));
    assert!(e.status.is_none());
    assert!(e.error.is_some());
}

#[tokio::test]
async fn noop_plan_sends_nothing_and_logs_nothing() {
    let server = MockServer::start().await;
    let pce = pce_for(&server);
    let audit = MemoryAudit::new();
    let guard = guard_with(audit.clone());
    let out = pce
        .writer(&guard)
        .execute(Plan::Noop {
            existing_href: "/orgs/5/labels/1".into(),
            reason: "label app=web exists".into(),
        })
        .await
        .unwrap();
    assert!(matches!(out, Outcome::Skipped { .. }));
    assert!(server.received_requests().await.unwrap().is_empty());
    assert!(audit.entries().is_empty());
}

#[tokio::test]
async fn unwritable_audit_refuses_before_sending() {
    struct Broken;
    impl pce::write::AuditSink for Broken {
        fn check_writable(&self) -> std::io::Result<()> {
            Err(std::io::Error::other("disk full"))
        }
        fn append(&self, _: &pce::write::AuditEntry) -> std::io::Result<()> {
            Ok(())
        }
        fn location(&self) -> String {
            "broken".into()
        }
    }
    let server = MockServer::start().await;
    let pce = pce_for(&server);
    let err = WriteGuard::new(true, true, Box::new(Broken), ctx()).unwrap_err();
    assert!(matches!(err, WriteRefused::AuditUnwritable(_)), "{err:?}");
    assert!(server.received_requests().await.unwrap().is_empty());
    let _ = IfMissing::SkipExisting; // used by later tasks; keeps the import honest
    let _ = pce;
}

#[test]
fn draft_href_rewrites_active_and_refuses_other_scopes() {
    use pce::write::draft_href;
    assert_eq!(
        draft_href("/orgs/5/sec_policy/active/ip_lists/3").unwrap(),
        "/orgs/5/sec_policy/draft/ip_lists/3"
    );
    assert_eq!(draft_href("/orgs/5/labels/1").unwrap(), "/orgs/5/labels/1");
    assert!(matches!(
        draft_href("/orgs/5/sec_policy/42/ip_lists/3").unwrap_err(),
        WriteError::Invalid(_)
    ));
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p pce --test write_contract`
Expected: compile errors (types missing).

- [ ] **Step 3: Implement `crates/pce/src/write/guard.rs`**

```rust
//! The write guard: writes are impossible without one, and one cannot be
//! built unless the profile allows writes, the caller asked for them, and
//! the audit log is writable.
use super::audit::AuditSink;

/// Who is writing, for audit entries.
#[derive(Debug, Clone)]
pub struct WriteContext {
    pub profile: String,
    pub host: String,
    pub org_id: u64,
    /// `cli` or `tui`.
    pub source: String,
}

#[derive(Debug, thiserror::Error)]
pub enum WriteRefused {
    #[error("writes are not enabled for this profile; set writes = true in the profile or ILLUMIO_WRITES=1")]
    ProfileDenies,
    #[error("writes need --allow-writes")]
    FlagMissing,
    #[error("audit log is not writable: {0}")]
    AuditUnwritable(String),
}

pub struct WriteGuard {
    audit: Box<dyn AuditSink>,
    ctx: WriteContext,
}

impl WriteGuard {
    pub fn new(
        profile_allows: bool,
        flag_allows: bool,
        audit: Box<dyn AuditSink>,
        ctx: WriteContext,
    ) -> Result<Self, WriteRefused> {
        if !profile_allows {
            return Err(WriteRefused::ProfileDenies);
        }
        if !flag_allows {
            return Err(WriteRefused::FlagMissing);
        }
        audit
            .check_writable()
            .map_err(|e| WriteRefused::AuditUnwritable(format!("{}: {e}", audit.location())))?;
        Ok(Self { audit, ctx })
    }

    pub fn audit(&self) -> &dyn AuditSink {
        self.audit.as_ref()
    }

    pub fn audit_location(&self) -> String {
        self.audit.location()
    }

    pub fn context(&self) -> &WriteContext {
        &self.ctx
    }
}

impl std::fmt::Debug for WriteGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WriteGuard")
            .field("audit", &self.audit.location())
            .field("ctx", &self.ctx)
            .finish()
    }
}
```

- [ ] **Step 4: Replace `crates/pce/src/write/mod.rs`**

```rust
//! Writes to the PCE. Two phases: `plan_*` functions (in the per-kind
//! modules) build a [`Plan`] without sending anything; [`Writer::execute`]
//! sends it exactly once, audits it, invalidates the cache and re-fetches
//! the object when the PCE answers with no body. A [`Writer`] exists only
//! through [`crate::Pce::writer`] with a [`WriteGuard`].
pub mod audit;
pub mod guard;

use crate::client::WriteMethod;
use crate::error::PceError;
use crate::{CacheMode, Pce};
use serde::Serialize;
use serde_json::Value;

pub use audit::{AuditEntry, AuditSink, JsonlAudit, MemoryAudit, NullAudit};
pub use guard::{WriteContext, WriteGuard, WriteRefused};

/// What kind of object a plan touches; decides the cache prefix and whether
/// the result waits for provisioning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectKind {
    Label,
    IpList,
    Service,
    Workload,
}

impl ObjectKind {
    pub fn cache_prefix(self) -> &'static str {
        match self {
            ObjectKind::Label | ObjectKind::IpList | ObjectKind::Service => "objects:",
            ObjectKind::Workload => "workloads",
        }
    }

    /// Policy objects live in draft until provisioned.
    pub fn pending(self) -> bool {
        matches!(self, ObjectKind::IpList | ObjectKind::Service)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ObjectKind::Label => "label",
            ObjectKind::IpList => "ip_list",
            ObjectKind::Service => "service",
            ObjectKind::Workload => "workload",
        }
    }
}

/// The exact request a plan would send. Printed by `--dry-run`.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct WriteRequest {
    pub method: WriteMethod,
    /// API path, e.g. `/orgs/5/labels/9` (same form as a PCE href).
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<Value>,
    /// One line for humans: `create label app=web`.
    pub summary: String,
}

#[derive(Debug, Clone)]
pub enum Plan {
    Send { request: WriteRequest, kind: ObjectKind },
    /// Nothing to do (`--if-missing` found an exact match).
    Noop { existing_href: String, reason: String },
}

impl Plan {
    pub fn request(&self) -> Option<&WriteRequest> {
        match self {
            Plan::Send { request, .. } => Some(request),
            Plan::Noop { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Written {
    pub href: String,
    /// True for policy objects: the change waits in draft for provisioning.
    pub pending: bool,
    pub status: u16,
    /// The object as the PCE returned it (re-fetched after a 204); `null`
    /// after a delete.
    pub object: Value,
    /// Keys sent with a value that came back empty or missing.
    pub dropped_fields: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum Outcome {
    Written(Written),
    Skipped { existing_href: String, reason: String },
}

#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    #[error(transparent)]
    Refused(#[from] WriteRefused),
    #[error("{}", .0.join("; "))]
    Invalid(Vec<String>),
    /// The request was sent but the outcome is unknown (timeout, connection
    /// loss, or a gateway status). Never retried automatically.
    #[error("{0}; write may have applied; check the PCE before retrying")]
    Unanswered(PceError),
    #[error(transparent)]
    Pce(PceError),
}

impl WriteError {
    /// Stable machine-readable name for CLI JSON errors.
    pub fn kind(&self) -> &'static str {
        match self {
            WriteError::Refused(_) => "write_refused",
            WriteError::Invalid(_) => "invalid",
            WriteError::Unanswered(_) => "write_unanswered",
            WriteError::Pce(e) => e.kind(),
        }
    }

    pub fn invalid(msg: impl Into<String>) -> Self {
        WriteError::Invalid(vec![msg.into()])
    }
}

impl From<PceError> for WriteError {
    fn from(e: PceError) -> Self {
        WriteError::Pce(e)
    }
}

/// `--if-missing`: skip the create when an exact match already exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IfMissing {
    Create,
    SkipExisting,
}

/// Rewrites an active policy href to its draft twin; refuses any other
/// `sec_policy` scope (draft is the only write target).
pub fn draft_href(href: &str) -> Result<String, WriteError> {
    const ACTIVE: &str = "/sec_policy/active/";
    const DRAFT: &str = "/sec_policy/draft/";
    if href.contains(ACTIVE) {
        return Ok(href.replacen(ACTIVE, DRAFT, 1));
    }
    if let Some(rest) = href.split_once("/sec_policy/").map(|(_, r)| r)
        && !rest.starts_with("draft/")
    {
        return Err(WriteError::invalid(format!(
            "{href}: only draft policy objects can be written"
        )));
    }
    Ok(href.to_string())
}

/// A value counts as "present" for `dropped_fields` when it is not null,
/// not an empty array, object or string.
fn present(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

fn dropped_fields(sent: Option<&Value>, got: &Value) -> Vec<String> {
    let Some(Value::Object(sent)) = sent else {
        return Vec::new();
    };
    let Value::Object(got) = got else {
        return Vec::new();
    };
    sent.iter()
        .filter(|(_, v)| present(v))
        .filter(|(k, _)| !got.get(k.as_str()).is_some_and(present))
        .map(|(k, _)| k.clone())
        .collect()
}

/// The only thing that sends writes.
pub struct Writer<'a> {
    pce: &'a Pce,
    guard: &'a WriteGuard,
}

impl Pce {
    pub fn writer<'a>(&'a self, guard: &'a WriteGuard) -> Writer<'a> {
        Writer { pce: self, guard }
    }
}

impl<'a> Writer<'a> {
    pub fn pce(&self) -> &'a Pce {
        self.pce
    }

    pub fn guard(&self) -> &'a WriteGuard {
        self.guard
    }

    /// One uncached GET of a single object by href.
    pub async fn fetch(&self, href: &str) -> Result<Value, WriteError> {
        Ok(self.pce.client().get::<Value>(href, &[]).await?.body)
    }

    /// The cache mode for an `--if-missing` lookup: always fresh.
    pub fn bypass(&self) -> CacheMode {
        CacheMode::Bypass
    }

    pub async fn execute(&self, plan: Plan) -> Result<Outcome, WriteError> {
        let (request, kind) = match plan {
            Plan::Noop {
                existing_href,
                reason,
            } => {
                return Ok(Outcome::Skipped {
                    existing_href,
                    reason,
                });
            }
            Plan::Send { request, kind } => (request, kind),
        };
        let audit = self.guard.audit();
        audit
            .check_writable()
            .map_err(|e| WriteRefused::AuditUnwritable(format!("{}: {e}", audit.location())))?;

        let result = self
            .pce
            .client()
            .write(request.method, &request.path, request.body.as_ref())
            .await;

        let mut entry = self.entry(&request);
        let resp = match result {
            Ok(resp) => resp,
            Err(e) => {
                let unanswered = matches!(
                    e,
                    PceError::Timeout(_)
                        | PceError::Network(_)
                        | PceError::Server {
                            status: 502..=504,
                            ..
                        }
                );
                if let PceError::Server { status, .. } | PceError::Rejected { status, .. } = &e {
                    entry.status = Some(*status);
                }
                entry.error = Some(e.to_string());
                if unanswered {
                    entry.phase = Some("sent".into());
                }
                let _ = audit.append(&entry);
                return Err(if unanswered {
                    WriteError::Unanswered(e)
                } else {
                    WriteError::Pce(e)
                });
            }
        };

        let href = resp
            .body
            .get("href")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| request.path.clone());
        entry.status = Some(resp.status);
        entry.href = Some(href.clone());
        let _ = audit.append(&entry);
        self.pce.invalidate(kind.cache_prefix());

        let object = if request.method == WriteMethod::Delete {
            Value::Null
        } else if resp.body.is_null() {
            self.fetch(&href).await?
        } else {
            resp.body
        };
        let dropped = if request.method == WriteMethod::Delete {
            Vec::new()
        } else {
            dropped_fields(request.body.as_ref(), &object)
        };
        Ok(Outcome::Written(Written {
            href,
            pending: kind.pending(),
            status: resp.status,
            object,
            dropped_fields: dropped,
        }))
    }

    fn entry(&self, request: &WriteRequest) -> AuditEntry {
        let c = self.guard.context();
        AuditEntry {
            ts: jiff::Timestamp::now().to_string(),
            profile: c.profile.clone(),
            host: c.host.clone(),
            org_id: c.org_id,
            method: request.method,
            path: request.path.clone(),
            body: request.body.clone(),
            status: None,
            href: None,
            error: None,
            phase: None,
            source: c.source.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn dropped_fields_flags_sent_values_that_came_back_empty() {
        let sent = json!({"name": "PG", "service_ports": [{"port": 1}], "description": ""});
        let got = json!({"name": "PG", "service_ports": [], "href": "/x"});
        assert_eq!(dropped_fields(Some(&sent), &got), vec!["service_ports".to_string()]);
        assert!(dropped_fields(None, &got).is_empty());
        assert!(dropped_fields(Some(&sent), &Value::Null).is_empty());
    }

    #[test]
    fn object_kind_prefixes_and_pending() {
        assert_eq!(ObjectKind::Label.cache_prefix(), "objects:");
        assert_eq!(ObjectKind::Workload.cache_prefix(), "workloads");
        assert!(ObjectKind::IpList.pending());
        assert!(!ObjectKind::Label.pending());
        assert!(!ObjectKind::Workload.pending());
    }
}
```

`Pce::invalidate("objects:")` also drops `objects:index`, so the name index is rebuilt on the next read. Note `PceError::Server { status: 502..=504, .. }` uses a range pattern on `u16`; it is stable.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p pce --test write_contract` and `cargo test -p pce --lib write`
Expected: all pass. If the `PceClient::write` `429` test from Task 1 and these run in parallel they use separate mock servers; no interference.

- [ ] **Step 6: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy -p pce --all-targets -- -D warnings && cargo test -p pce
git add crates/pce/src/write crates/pce/src/lib.rs crates/pce/tests/write_contract.rs
git commit -m "feat(pce): write guard, plans, and Writer::execute (send once, audit, invalidate, re-fetch)

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 4: Spec parsers: IP ranges, ports, interfaces

**Files:**
- Create: `crates/pce/src/write/specs.rs`
- Modify: `crates/pce/src/write/mod.rs` (`pub mod specs;` and `pub use specs::{IpRangeSpec, PortSpec, InterfaceSpec};`)

**Interfaces:**
- Consumes: `models::{IpRange, ServicePort, Interface}`, `models::proto_number`, `models::proto_name`.
- Produces:

```rust
pub struct IpRangeSpec { pub from_ip: String, pub to_ip: Option<String>, pub exclusion: bool }
impl IpRangeSpec { pub fn parse(s: &str, exclusion: bool) -> Result<Self, String>; pub fn from_range(r: &IpRange) -> Self; pub fn to_json(&self) -> Value; pub fn display(&self) -> String; pub fn same_range(&self, other: &Self) -> bool }
pub struct PortSpec { pub port: Option<u16>, pub to_port: Option<u16>, pub proto: i64 }
impl PortSpec { pub fn parse(s: &str) -> Result<Self, String>; pub fn from_service_port(p: &ServicePort) -> Option<Self>; pub fn to_json(&self) -> Value; pub fn display(&self) -> String }
pub struct InterfaceSpec { pub name: String, pub address: String, pub cidr_block: Option<u8> }
impl InterfaceSpec { pub fn parse(s: &str, name: String) -> Result<Self, String>; pub fn from_interface(i: &Interface) -> Self; pub fn to_json(&self) -> Value }
```

- [ ] **Step 1: Write the failing unit tests** (in `specs.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ip_ranges_parse_single_range_and_cidr() {
        let r = IpRangeSpec::parse("10.1.2.3", false).unwrap();
        assert_eq!(r.to_json(), json!({"from_ip": "10.1.2.3"}));
        let r = IpRangeSpec::parse("10.1.0.0-10.1.255.255", false).unwrap();
        assert_eq!(r.to_json(), json!({"from_ip": "10.1.0.0", "to_ip": "10.1.255.255"}));
        let r = IpRangeSpec::parse(" 2001:db8::1 ", true).unwrap();
        assert_eq!(r.to_json(), json!({"from_ip": "2001:db8::1", "exclusion": true}));
        assert_eq!(r.display(), "!2001:db8::1");
    }

    #[test]
    fn cidr_stays_in_from_ip() {
        let r = IpRangeSpec::parse("10.0.0.0/8", false).unwrap();
        assert_eq!(r.to_json(), json!({"from_ip": "10.0.0.0/8"}));
        assert_eq!(r.display(), "10.0.0.0/8");
        assert!(IpRangeSpec::parse("10.0.0.0/33", false).is_err());
        assert!(IpRangeSpec::parse("2001:db8::/129", false).is_err());
    }

    #[test]
    fn bad_ip_ranges_are_rejected_with_the_input_in_the_message() {
        for bad in ["", "10.1", "10.1.0.0-", "a-b", "10.0.0.2-10.0.0.1", "10.0.0.0/8-10.0.0.9"] {
            let err = IpRangeSpec::parse(bad, false).unwrap_err();
            assert!(err.contains(bad.trim()) || bad.is_empty(), "{bad:?}: {err}");
        }
    }

    #[test]
    fn same_range_ignores_nothing() {
        let a = IpRangeSpec::parse("10.0.0.0/8", false).unwrap();
        let b = IpRangeSpec::parse("10.0.0.0/8", true).unwrap();
        assert!(!a.same_range(&b), "exclusion is part of identity");
        assert!(a.same_range(&IpRangeSpec::parse("10.0.0.0/8", false).unwrap()));
        let from = IpRangeSpec::from_range(&crate::models::IpRange {
            from_ip: "10.0.0.0/8".into(),
            to_ip: None,
            exclusion: Some(false),
            description: None,
        });
        assert!(a.same_range(&from));
    }

    #[test]
    fn ports_parse_every_form() {
        assert_eq!(PortSpec::parse("443/tcp").unwrap().to_json(), json!({"port": 443, "proto": 6}));
        assert_eq!(
            PortSpec::parse("8000-8100/udp").unwrap().to_json(),
            json!({"port": 8000, "to_port": 8100, "proto": 17})
        );
        assert_eq!(PortSpec::parse("icmp").unwrap().to_json(), json!({"proto": 1}));
        assert_eq!(PortSpec::parse("ICMPv6").unwrap().to_json(), json!({"proto": 58}));
        assert_eq!(PortSpec::parse("tcp").unwrap().to_json(), json!({"proto": 6}));
        assert_eq!(PortSpec::parse("47").unwrap().to_json(), json!({"proto": 47}));
        assert_eq!(PortSpec::parse("53/17").unwrap().display(), "53/udp");
        assert_eq!(PortSpec::parse("8000-8100/tcp").unwrap().display(), "8000-8100/tcp");
    }

    #[test]
    fn bad_ports_are_rejected() {
        for bad in ["", "443", "443/", "/tcp", "443/sctpx", "9-1/tcp", "70000/tcp", "443/icmp"] {
            assert!(PortSpec::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn port_round_trips_from_service_port() {
        let sp = crate::models::ServicePort {
            port: Some(5432),
            to_port: None,
            proto: Some(6),
            icmp_type: None,
            icmp_code: None,
        };
        let p = PortSpec::from_service_port(&sp).unwrap();
        assert_eq!(p.display(), "5432/tcp");
        assert_eq!(p, PortSpec::parse("5432/tcp").unwrap());
        let none = crate::models::ServicePort { proto: None, ..sp };
        assert!(PortSpec::from_service_port(&none).is_none());
    }

    #[test]
    fn interfaces_parse_address_and_prefix() {
        let i = InterfaceSpec::parse("10.0.0.5/24", "eth0".into()).unwrap();
        assert_eq!(i.to_json(), json!({"name": "eth0", "address": "10.0.0.5", "cidr_block": 24}));
        let i = InterfaceSpec::parse("fe80::1", "eth1".into()).unwrap();
        assert_eq!(i.to_json(), json!({"name": "eth1", "address": "fe80::1"}));
        assert!(InterfaceSpec::parse("10.0.0.5/33", "eth0".into()).is_err());
        assert!(InterfaceSpec::parse("host", "eth0".into()).is_err());
        let from = InterfaceSpec::from_interface(&crate::models::Interface {
            name: "eth0".into(),
            address: "10.0.0.5".into(),
            cidr_block: Some(24),
            link_state: Some("up".into()),
        });
        assert_eq!(from.to_json()["cidr_block"], 24);
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p pce --lib write::specs`
Expected: compile error.

- [ ] **Step 3: Implement `crates/pce/src/write/specs.rs`**

```rust
//! Human spellings of IP ranges, service ports and workload interfaces, and
//! their PCE JSON forms. Parsers never touch the network.
use crate::models::{Interface, IpRange, ServicePort, proto_name, proto_number};
use serde_json::{Value, json};
use std::net::IpAddr;

fn parse_ip(s: &str) -> Result<IpAddr, String> {
    s.parse::<IpAddr>()
        .map_err(|_| format!("{s:?} is not an IP address"))
}

/// `addr/N` → (addr, N) with N checked against the address family.
fn parse_cidr(s: &str) -> Result<(IpAddr, u8), String> {
    let (a, n) = s
        .split_once('/')
        .ok_or_else(|| format!("{s:?} is not a CIDR"))?;
    let ip = parse_ip(a)?;
    let bits: u8 = n
        .parse()
        .map_err(|_| format!("{s:?}: prefix length is not a number"))?;
    let max = if ip.is_ipv4() { 32 } else { 128 };
    if bits > max {
        return Err(format!("{s:?}: prefix length must be at most {max}"));
    }
    Ok((ip, bits))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpRangeSpec {
    /// A single address or a CIDR (`10.0.0.0/8`), as the PCE stores it.
    pub from_ip: String,
    pub to_ip: Option<String>,
    pub exclusion: bool,
}

impl IpRangeSpec {
    /// `A`, `A-B` or `A/N`, IPv4 or IPv6.
    pub fn parse(s: &str, exclusion: bool) -> Result<Self, String> {
        let s = s.trim();
        if s.is_empty() {
            return Err("empty IP range".into());
        }
        if s.contains('/') {
            if s.contains('-') {
                return Err(format!("{s:?}: a CIDR cannot also be a range"));
            }
            let (ip, bits) = parse_cidr(s)?;
            return Ok(Self {
                from_ip: format!("{ip}/{bits}"),
                to_ip: None,
                exclusion,
            });
        }
        if let Some((a, b)) = s.split_once('-') {
            let from = parse_ip(a.trim()).map_err(|e| format!("{s:?}: {e}"))?;
            let to = parse_ip(b.trim()).map_err(|e| format!("{s:?}: {e}"))?;
            if from.is_ipv4() != to.is_ipv4() {
                return Err(format!("{s:?}: mixed address families"));
            }
            if to < from {
                return Err(format!("{s:?}: range end is before its start"));
            }
            return Ok(Self {
                from_ip: from.to_string(),
                to_ip: Some(to.to_string()),
                exclusion,
            });
        }
        let ip = parse_ip(s)?;
        Ok(Self {
            from_ip: ip.to_string(),
            to_ip: None,
            exclusion,
        })
    }

    pub fn from_range(r: &IpRange) -> Self {
        Self {
            from_ip: r.from_ip.clone(),
            to_ip: r.to_ip.clone(),
            exclusion: r.exclusion.unwrap_or(false),
        }
    }

    pub fn to_json(&self) -> Value {
        let mut v = json!({"from_ip": self.from_ip});
        if let Some(to) = &self.to_ip {
            v["to_ip"] = json!(to);
        }
        if self.exclusion {
            v["exclusion"] = json!(true);
        }
        v
    }

    pub fn display(&self) -> String {
        let excl = if self.exclusion { "!" } else { "" };
        match &self.to_ip {
            Some(to) => format!("{excl}{}-{to}", self.from_ip),
            None => format!("{excl}{}", self.from_ip),
        }
    }

    pub fn same_range(&self, other: &Self) -> bool {
        self == other
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortSpec {
    pub port: Option<u16>,
    pub to_port: Option<u16>,
    pub proto: i64,
}

impl PortSpec {
    /// `P/PROTO`, `P-Q/PROTO`, or a bare protocol (`icmp`, `udp`, `47`).
    pub fn parse(s: &str) -> Result<Self, String> {
        let s = s.trim();
        if s.is_empty() {
            return Err("empty port spec".into());
        }
        let (ports, proto) = match s.split_once('/') {
            Some((p, pr)) => (Some(p.trim()), pr.trim()),
            None => (None, s),
        };
        if proto.is_empty() {
            return Err(format!("{s:?}: missing protocol"));
        }
        let proto = i64::from(
            proto_number(proto).ok_or_else(|| format!("{s:?}: unknown protocol {proto:?}"))?,
        );
        let Some(ports) = ports else {
            return Ok(Self {
                port: None,
                to_port: None,
                proto,
            });
        };
        if ports.is_empty() {
            return Err(format!("{s:?}: missing port"));
        }
        if matches!(proto, 1 | 58) {
            return Err(format!("{s:?}: ICMP has no ports"));
        }
        let parse_port = |p: &str| {
            p.trim()
                .parse::<u16>()
                .map_err(|_| format!("{s:?}: {p:?} is not a port (0-65535)"))
        };
        let (port, to_port) = match ports.split_once('-') {
            Some((a, b)) => {
                let a = parse_port(a)?;
                let b = parse_port(b)?;
                if b < a {
                    return Err(format!("{s:?}: port range end is before its start"));
                }
                (a, if b == a { None } else { Some(b) })
            }
            None => (parse_port(ports)?, None),
        };
        Ok(Self {
            port: Some(port),
            to_port,
            proto,
        })
    }

    pub fn from_service_port(p: &ServicePort) -> Option<Self> {
        Some(Self {
            port: p.port,
            to_port: p.to_port.filter(|t| Some(*t) != p.port),
            proto: p.proto?,
        })
    }

    pub fn to_json(&self) -> Value {
        let mut v = json!({"proto": self.proto});
        if let Some(p) = self.port {
            v["port"] = json!(p);
        }
        if let Some(t) = self.to_port {
            v["to_port"] = json!(t);
        }
        v
    }

    pub fn display(&self) -> String {
        let proto = proto_name(self.proto);
        match (self.port, self.to_port) {
            (Some(p), Some(t)) => format!("{p}-{t}/{proto}"),
            (Some(p), None) => format!("{p}/{proto}"),
            (None, _) => proto,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfaceSpec {
    pub name: String,
    pub address: String,
    pub cidr_block: Option<u8>,
}

impl InterfaceSpec {
    /// `ADDR` or `ADDR/N`.
    pub fn parse(s: &str, name: String) -> Result<Self, String> {
        let s = s.trim();
        if s.contains('/') {
            let (ip, bits) = parse_cidr(s)?;
            return Ok(Self {
                name,
                address: ip.to_string(),
                cidr_block: Some(bits),
            });
        }
        let ip = parse_ip(s)?;
        Ok(Self {
            name,
            address: ip.to_string(),
            cidr_block: None,
        })
    }

    pub fn from_interface(i: &Interface) -> Self {
        Self {
            name: i.name.clone(),
            address: i.address.clone(),
            cidr_block: i.cidr_block,
        }
    }

    pub fn to_json(&self) -> Value {
        let mut v = json!({"name": self.name, "address": self.address});
        if let Some(c) = self.cidr_block {
            v["cidr_block"] = json!(c);
        }
        v
    }
}
```

Note on bare numbers in `PortSpec::parse`: `proto_number` parses a bare number as `u8`, so `"443"` alone fails with "unknown protocol" (it is in the `bad_ports_are_rejected` list) while `"47"` is protocol 47.

Add to `crates/pce/src/write/mod.rs`: `pub mod specs;` and `pub use specs::{InterfaceSpec, IpRangeSpec, PortSpec};`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p pce --lib write::specs`
Expected: 8 tests pass.

- [ ] **Step 5: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy -p pce --all-targets -- -D warnings && cargo test -p pce
git add crates/pce/src/write
git commit -m "feat(pce): parsers for IP ranges, service ports and workload interfaces

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 5: Labels: draft and planners

**Files:**
- Create: `crates/pce/src/write/labels.rs`
- Modify: `crates/pce/src/write/mod.rs` (`pub mod labels;`, `pub use labels::LabelDraft;`, add `checked_href`)
- Test: `crates/pce/tests/write_contract.rs` (append)

**Interfaces:**
- Consumes: `Writer`, `Plan`, `WriteRequest`, `ObjectKind::Label`, `IfMissing`, `Pce::labels(CacheMode::Bypass)`, `Label::is_deleted`.
- Produces:

```rust
pub struct LabelDraft { pub key: String, pub value: String }   // LabelDraft::new(key, value) trims; validate(); display() -> "key=value"
pub fn checked_href(href: &str, segment: &str) -> Result<String, WriteError>   // in mod.rs
impl Writer<'_> {
    pub async fn plan_create_label(&self, draft: &LabelDraft, if_missing: IfMissing) -> Result<Plan, WriteError>;
    pub fn plan_update_label(&self, href: &str, value: &str) -> Result<Plan, WriteError>;
    pub fn plan_delete_label(&self, href: &str) -> Result<Plan, WriteError>;
}
```

- [ ] **Step 1: Write the failing tests** (append to `crates/pce/tests/write_contract.rs`)

```rust
mod labels {
    use super::*;
    use pce::write::LabelDraft;

    #[test]
    fn label_draft_validates_and_trims() {
        let d = LabelDraft::new(" app ", " web ");
        assert_eq!(d.display(), "app=web");
        assert!(d.validate().is_ok());
        let errs = LabelDraft::new("", " ").validate().unwrap_err();
        assert_eq!(errs.len(), 2, "{errs:?}");
    }

    #[tokio::test]
    async fn create_plan_without_if_missing_makes_no_request() {
        let server = MockServer::start().await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let plan = pce
            .writer(&guard)
            .plan_create_label(&LabelDraft::new("app", "web"), IfMissing::Create)
            .await
            .unwrap();
        let req = plan.request().expect("send plan").clone();
        assert_eq!(req.method, WriteMethod::Post);
        assert_eq!(req.path, "/orgs/5/labels");
        assert_eq!(req.body, Some(json!({"key": "app", "value": "web"})));
        assert_eq!(req.summary, "create label app=web");
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn if_missing_bypasses_the_cache_and_skips_an_exact_match() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/labels"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {"href": "/orgs/5/labels/1", "key": "app", "value": "web"},
                {"href": "/orgs/5/labels/2", "key": "app", "value": "webshop"}
            ])))
            .expect(3)
            .mount(&server)
            .await;
        let pce = pce_for(&server);
        pce.labels(CacheMode::Use).await.unwrap(); // warm: the writer must not use this
        let guard = guard_with(MemoryAudit::new());
        let plan = pce
            .writer(&guard)
            .plan_create_label(&LabelDraft::new("app", "web"), IfMissing::SkipExisting)
            .await
            .unwrap();
        match plan {
            Plan::Noop { existing_href, .. } => assert_eq!(existing_href, "/orgs/5/labels/1"),
            other => panic!("expected Noop, got {other:?}"),
        }
        // "web" is a substring of "webshop": only the exact match counts.
        let plan = pce
            .writer(&guard)
            .plan_create_label(&LabelDraft::new("app", "we"), IfMissing::SkipExisting)
            .await
            .unwrap();
        assert!(plan.request().is_some());
    }

    #[tokio::test]
    async fn if_missing_ignores_deleted_labels() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/labels"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {"href": "/orgs/5/labels/1", "key": "app", "value": "web", "deleted": true}
            ])))
            .mount(&server)
            .await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let plan = pce
            .writer(&guard)
            .plan_create_label(&LabelDraft::new("app", "web"), IfMissing::SkipExisting)
            .await
            .unwrap();
        assert!(plan.request().is_some(), "a deleted label does not count as existing");
    }

    #[tokio::test]
    async fn update_and_delete_plans() {
        let server = MockServer::start().await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let w = pce.writer(&guard);
        let up = w.plan_update_label("/orgs/5/labels/9", "prod").unwrap();
        let req = up.request().unwrap();
        assert_eq!(req.method, WriteMethod::Put);
        assert_eq!(req.path, "/orgs/5/labels/9");
        assert_eq!(req.body, Some(json!({"value": "prod"})));
        let del = w.plan_delete_label("/orgs/5/labels/9").unwrap();
        let req = del.request().unwrap();
        assert_eq!(req.method, WriteMethod::Delete);
        assert_eq!(req.body, None);
        assert_eq!(req.summary, "delete label /orgs/5/labels/9");
        assert!(matches!(
            w.plan_delete_label("/orgs/5/sec_policy/draft/ip_lists/1").unwrap_err(),
            WriteError::Invalid(_)
        ));
        assert!(matches!(
            w.plan_update_label("/orgs/5/labels/9", " ").unwrap_err(),
            WriteError::Invalid(_)
        ));
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p pce --test write_contract labels`
Expected: compile error.

- [ ] **Step 3: Add `checked_href` to `crates/pce/src/write/mod.rs`** (after `draft_href`)

```rust
/// An href of the expected kind (`labels`, `ip_lists`, `services`,
/// `workloads`), rewritten to draft where that applies.
pub fn checked_href(href: &str, segment: &str) -> Result<String, WriteError> {
    let href = href.trim().trim_end_matches('/');
    if !href.starts_with("/orgs/") || !href.contains(&format!("/{segment}/")) {
        return Err(WriteError::invalid(format!(
            "{href:?} is not a {segment} href (expected /orgs/<org>/.../{segment}/<id>)"
        )));
    }
    draft_href(href)
}
```

- [ ] **Step 4: Implement `crates/pce/src/write/labels.rs`**

```rust
//! Labels: `POST /orgs/{org}/labels`, `PUT|DELETE /orgs/{org}/labels/{id}`.
//! Not provisionable, so a written label is live at once.
use super::{IfMissing, ObjectKind, Plan, WriteError, WriteRequest, Writer, checked_href};
use crate::client::WriteMethod;
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelDraft {
    pub key: String,
    pub value: String,
}

impl LabelDraft {
    pub fn new(key: impl AsRef<str>, value: impl AsRef<str>) -> Self {
        Self {
            key: key.as_ref().trim().to_string(),
            value: value.as_ref().trim().to_string(),
        }
    }

    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errs = Vec::new();
        if self.key.is_empty() {
            errs.push("label key must not be empty".into());
        }
        if self.value.is_empty() {
            errs.push("label value must not be empty".into());
        }
        if errs.is_empty() { Ok(()) } else { Err(errs) }
    }

    pub fn display(&self) -> String {
        format!("{}={}", self.key, self.value)
    }

    pub(crate) fn to_body(&self) -> Value {
        json!({"key": self.key, "value": self.value})
    }
}

impl Writer<'_> {
    pub async fn plan_create_label(
        &self,
        draft: &LabelDraft,
        if_missing: IfMissing,
    ) -> Result<Plan, WriteError> {
        draft.validate().map_err(WriteError::Invalid)?;
        if if_missing == IfMissing::SkipExisting {
            let labels = self.pce().labels(self.bypass()).await?.value;
            if let Some(existing) = labels
                .iter()
                .find(|l| !l.is_deleted() && l.key == draft.key && l.value == draft.value)
            {
                return Ok(Plan::Noop {
                    existing_href: existing.href.clone(),
                    reason: format!("label {} exists", draft.display()),
                });
            }
        }
        Ok(Plan::Send {
            request: WriteRequest {
                method: WriteMethod::Post,
                path: self.pce().client().org_path("labels"),
                body: Some(draft.to_body()),
                summary: format!("create label {}", draft.display()),
            },
            kind: ObjectKind::Label,
        })
    }

    pub fn plan_update_label(&self, href: &str, value: &str) -> Result<Plan, WriteError> {
        let href = checked_href(href, "labels")?;
        let value = value.trim();
        if value.is_empty() {
            return Err(WriteError::invalid("label value must not be empty"));
        }
        Ok(Plan::Send {
            request: WriteRequest {
                method: WriteMethod::Put,
                path: href.clone(),
                body: Some(json!({"value": value})),
                summary: format!("update label {href} value to {value}"),
            },
            kind: ObjectKind::Label,
        })
    }

    pub fn plan_delete_label(&self, href: &str) -> Result<Plan, WriteError> {
        let href = checked_href(href, "labels")?;
        Ok(Plan::Send {
            request: WriteRequest {
                method: WriteMethod::Delete,
                path: href.clone(),
                body: None,
                summary: format!("delete label {href}"),
            },
            kind: ObjectKind::Label,
        })
    }
}
```

Add to `mod.rs`: `pub mod labels;` and `pub use labels::LabelDraft;`.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p pce --test write_contract`
Expected: all pass (`if_missing_bypasses_the_cache_and_skips_an_exact_match` expects exactly three GETs: the warm-up and one bypass per `plan_create_label`).

- [ ] **Step 6: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy -p pce --all-targets -- -D warnings && cargo test -p pce
git add crates/pce/src/write crates/pce/tests/write_contract.rs
git commit -m "feat(pce): label drafts and create/update/delete plans with exact-match --if-missing

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 6: IP lists: draft, patch and planners

**Files:**
- Create: `crates/pce/src/write/ip_lists.rs`
- Modify: `crates/pce/src/write/mod.rs` (`pub mod ip_lists;`, `pub use ip_lists::{IpListDraft, IpListPatch};`)
- Test: `crates/pce/tests/write_contract.rs` (append)

**Interfaces:**
- Consumes: `IpRangeSpec`, `models::IpList`, `Writer::fetch`, `checked_href`, `Pce::ip_lists(CacheMode::Bypass)`.
- Produces:

```rust
pub struct IpListDraft { pub name: String, pub description: Option<String>, pub ranges: Vec<IpRangeSpec>, pub fqdns: Vec<String> }  // validate(), from_ip_list(&IpList)
pub struct IpListPatch { pub name: Option<String>, pub description: Option<String>, pub add_ranges: Vec<IpRangeSpec>, pub remove_ranges: Vec<IpRangeSpec>, pub add_fqdns: Vec<String>, pub remove_fqdns: Vec<String> }  // is_empty(), apply(&IpList) -> Result<IpListDraft, Vec<String>>
impl Writer<'_> {
    pub async fn plan_create_ip_list(&self, draft: &IpListDraft, if_missing: IfMissing) -> Result<Plan, WriteError>;
    pub async fn plan_update_ip_list(&self, href: &str, patch: &IpListPatch) -> Result<Plan, WriteError>;
    pub fn plan_delete_ip_list(&self, href: &str) -> Result<Plan, WriteError>;
}
```

- [ ] **Step 1: Write the failing tests** (append to `crates/pce/tests/write_contract.rs`)

```rust
mod ip_lists {
    use super::*;
    use pce::write::{IpListDraft, IpListPatch, IpRangeSpec};

    fn corp() -> Value {
        json!({
            "href": "/orgs/5/sec_policy/draft/ip_lists/3", "name": "Corp", "description": "corp nets",
            "ip_ranges": [{"from_ip": "10.0.0.0/8"}, {"from_ip": "192.168.1.1", "to_ip": "192.168.1.9", "exclusion": true}],
            "fqdns": [{"fqdn": "*.corp.example"}]
        })
    }

    #[test]
    fn draft_validates_and_builds_the_body() {
        let d = IpListDraft {
            name: "Corp".into(),
            description: None,
            ranges: vec![IpRangeSpec::parse("10.0.0.0/8", false).unwrap()],
            fqdns: vec!["*.corp.example".into()],
        };
        assert!(d.validate().is_ok());
        let empty = IpListDraft { name: " ".into(), description: None, ranges: vec![], fqdns: vec![] };
        let errs = empty.validate().unwrap_err();
        assert_eq!(errs.len(), 2, "{errs:?}");
    }

    #[tokio::test]
    async fn create_plan_body_uses_pce_field_names() {
        let server = MockServer::start().await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let d = IpListDraft {
            name: "Corp".into(),
            description: Some("corp nets".into()),
            ranges: vec![
                IpRangeSpec::parse("10.0.0.0/8", false).unwrap(),
                IpRangeSpec::parse("192.168.1.1-192.168.1.9", true).unwrap(),
            ],
            fqdns: vec!["*.corp.example".into()],
        };
        let plan = pce.writer(&guard).plan_create_ip_list(&d, IfMissing::Create).await.unwrap();
        let req = plan.request().unwrap();
        assert_eq!(req.method, WriteMethod::Post);
        assert_eq!(req.path, "/orgs/5/sec_policy/draft/ip_lists");
        assert_eq!(
            req.body,
            Some(json!({
                "name": "Corp", "description": "corp nets",
                "ip_ranges": [{"from_ip": "10.0.0.0/8"}, {"from_ip": "192.168.1.1", "to_ip": "192.168.1.9", "exclusion": true}],
                "fqdns": [{"fqdn": "*.corp.example"}]
            }))
        );
        assert_eq!(req.summary, "create ip_list Corp");
    }

    #[tokio::test]
    async fn if_missing_matches_names_exactly_and_case_sensitively() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/sec_policy/draft/ip_lists"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([corp()])))
            .mount(&server)
            .await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let d = |name: &str| IpListDraft {
            name: name.into(),
            description: None,
            ranges: vec![IpRangeSpec::parse("10.0.0.0/8", false).unwrap()],
            fqdns: vec![],
        };
        let plan = pce.writer(&guard).plan_create_ip_list(&d("Corp"), IfMissing::SkipExisting).await.unwrap();
        assert!(matches!(plan, Plan::Noop { .. }));
        let plan = pce.writer(&guard).plan_create_ip_list(&d("corp"), IfMissing::SkipExisting).await.unwrap();
        assert!(plan.request().is_some(), "case differs: not the same list");
    }

    #[test]
    fn patch_applies_adds_and_removes_to_the_current_list() {
        let current: pce::models::IpList = serde_json::from_value(corp()).unwrap();
        let patch = IpListPatch {
            name: None,
            description: Some("updated".into()),
            add_ranges: vec![IpRangeSpec::parse("10.2.0.0/16", false).unwrap()],
            remove_ranges: vec![IpRangeSpec::parse("192.168.1.1-192.168.1.9", true).unwrap()],
            add_fqdns: vec!["db.corp.example".into()],
            remove_fqdns: vec!["*.corp.example".into()],
        };
        let d = patch.apply(&current).unwrap();
        assert_eq!(d.name, "Corp");
        assert_eq!(d.description.as_deref(), Some("updated"));
        assert_eq!(
            d.ranges.iter().map(IpRangeSpec::display).collect::<Vec<_>>(),
            vec!["10.0.0.0/8", "10.2.0.0/16"]
        );
        assert_eq!(d.fqdns, vec!["db.corp.example"]);
        // Adding a duplicate is a no-op, not a second entry.
        let dup = IpListPatch { add_ranges: vec![IpRangeSpec::parse("10.0.0.0/8", false).unwrap()], ..IpListPatch::default() };
        assert_eq!(dup.apply(&current).unwrap().ranges.len(), 2);
    }

    #[test]
    fn remove_range_matches_a_cidr() {
        let current: pce::models::IpList = serde_json::from_value(corp()).unwrap();
        let patch = IpListPatch { remove_ranges: vec![IpRangeSpec::parse("10.0.0.0/8", false).unwrap()], ..IpListPatch::default() };
        let d = patch.apply(&current).unwrap();
        assert_eq!(d.ranges.len(), 1);
        assert_eq!(d.ranges[0].display(), "!192.168.1.1-192.168.1.9");
    }

    #[test]
    fn removing_something_absent_or_the_last_range_is_refused() {
        let current: pce::models::IpList = serde_json::from_value(corp()).unwrap();
        let patch = IpListPatch { remove_ranges: vec![IpRangeSpec::parse("172.16.0.1", false).unwrap()], ..IpListPatch::default() };
        let errs = patch.apply(&current).unwrap_err();
        assert!(errs[0].contains("172.16.0.1"), "{errs:?}");
        let patch = IpListPatch { remove_fqdns: vec!["nope".into()], ..IpListPatch::default() };
        assert!(patch.apply(&current).is_err());
        // Remove everything: the draft no longer validates.
        let patch = IpListPatch {
            remove_ranges: vec![
                IpRangeSpec::parse("10.0.0.0/8", false).unwrap(),
                IpRangeSpec::parse("192.168.1.1-192.168.1.9", true).unwrap(),
            ],
            remove_fqdns: vec!["*.corp.example".into()],
            ..IpListPatch::default()
        };
        let errs = patch.apply(&current).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("at least one")), "{errs:?}");
        assert!(IpListPatch::default().is_empty());
    }

    #[tokio::test]
    async fn update_plan_fetches_current_and_puts_the_full_list() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/sec_policy/draft/ip_lists/3"))
            .respond_with(ResponseTemplate::new(200).set_body_json(corp()))
            .expect(1)
            .mount(&server)
            .await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let patch = IpListPatch { add_fqdns: vec!["db.corp.example".into()], ..IpListPatch::default() };
        let plan = pce
            .writer(&guard)
            .plan_update_ip_list("/orgs/5/sec_policy/active/ip_lists/3", &patch)
            .await
            .unwrap();
        let req = plan.request().unwrap();
        assert_eq!(req.method, WriteMethod::Put);
        assert_eq!(req.path, "/orgs/5/sec_policy/draft/ip_lists/3", "active is rewritten to draft");
        let body = req.body.clone().unwrap();
        assert_eq!(body["name"], "Corp");
        assert_eq!(body["ip_ranges"].as_array().unwrap().len(), 2);
        assert_eq!(body["fqdns"], json!([{"fqdn": "*.corp.example"}, {"fqdn": "db.corp.example"}]));
        let err = pce
            .writer(&guard)
            .plan_update_ip_list("/orgs/5/sec_policy/draft/ip_lists/3", &IpListPatch::default())
            .await
            .unwrap_err();
        assert!(matches!(err, WriteError::Invalid(_)), "an empty patch is refused: {err:?}");
    }

    #[tokio::test]
    async fn delete_plan() {
        let server = MockServer::start().await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let plan = pce.writer(&guard).plan_delete_ip_list("/orgs/5/sec_policy/draft/ip_lists/3").unwrap();
        let req = plan.request().unwrap();
        assert_eq!(req.method, WriteMethod::Delete);
        assert_eq!(req.path, "/orgs/5/sec_policy/draft/ip_lists/3");
        assert!(pce.writer(&guard).plan_delete_ip_list("/orgs/5/labels/3").is_err());
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p pce --test write_contract ip_lists`
Expected: compile error.

- [ ] **Step 3: Implement `crates/pce/src/write/ip_lists.rs`**

```rust
//! IP lists under `/sec_policy/draft/ip_lists`. Updates are a full `PUT`
//! computed locally: the PCE replaces `ip_ranges` and `fqdns` wholesale.
use super::specs::IpRangeSpec;
use super::{IfMissing, ObjectKind, Plan, WriteError, WriteRequest, Writer, checked_href};
use crate::client::WriteMethod;
use crate::models::IpList;
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpListDraft {
    pub name: String,
    pub description: Option<String>,
    pub ranges: Vec<IpRangeSpec>,
    pub fqdns: Vec<String>,
}

impl IpListDraft {
    pub fn from_ip_list(l: &IpList) -> Self {
        Self {
            name: l.name.clone(),
            description: l.description.clone(),
            ranges: l.ip_ranges.iter().map(IpRangeSpec::from_range).collect(),
            fqdns: l.fqdns.iter().map(|f| f.fqdn.clone()).collect(),
        }
    }

    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errs = Vec::new();
        if self.name.trim().is_empty() {
            errs.push("IP list name must not be empty".into());
        }
        if self.ranges.is_empty() && self.fqdns.is_empty() {
            errs.push("an IP list needs at least one range or FQDN".into());
        }
        if self.fqdns.iter().any(|f| f.trim().is_empty()) {
            errs.push("FQDN must not be empty".into());
        }
        if errs.is_empty() { Ok(()) } else { Err(errs) }
    }

    pub(crate) fn to_body(&self) -> Value {
        let mut v = json!({
            "name": self.name.trim(),
            "ip_ranges": self.ranges.iter().map(IpRangeSpec::to_json).collect::<Vec<_>>(),
            "fqdns": self.fqdns.iter().map(|f| json!({"fqdn": f.trim()})).collect::<Vec<_>>(),
        });
        if let Some(d) = &self.description {
            v["description"] = json!(d);
        }
        v
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IpListPatch {
    pub name: Option<String>,
    pub description: Option<String>,
    pub add_ranges: Vec<IpRangeSpec>,
    pub remove_ranges: Vec<IpRangeSpec>,
    pub add_fqdns: Vec<String>,
    pub remove_fqdns: Vec<String>,
}

impl IpListPatch {
    pub fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.description.is_none()
            && self.add_ranges.is_empty()
            && self.remove_ranges.is_empty()
            && self.add_fqdns.is_empty()
            && self.remove_fqdns.is_empty()
    }

    /// The full list after this patch, validated. A remove that matches
    /// nothing is an error so a typo never silently leaves a range behind.
    pub fn apply(&self, current: &IpList) -> Result<IpListDraft, Vec<String>> {
        let mut d = IpListDraft::from_ip_list(current);
        let mut errs = Vec::new();
        for r in &self.remove_ranges {
            let before = d.ranges.len();
            d.ranges.retain(|x| !x.same_range(r));
            if d.ranges.len() == before {
                errs.push(format!("range {} is not in {}", r.display(), current.name));
            }
        }
        for r in &self.add_ranges {
            if !d.ranges.iter().any(|x| x.same_range(r)) {
                d.ranges.push(r.clone());
            }
        }
        for f in &self.remove_fqdns {
            let before = d.fqdns.len();
            d.fqdns.retain(|x| x != f);
            if d.fqdns.len() == before {
                errs.push(format!("fqdn {f} is not in {}", current.name));
            }
        }
        for f in &self.add_fqdns {
            if !d.fqdns.contains(f) {
                d.fqdns.push(f.clone());
            }
        }
        if let Some(n) = &self.name {
            d.name = n.clone();
        }
        if let Some(desc) = &self.description {
            d.description = Some(desc.clone());
        }
        if let Err(mut e) = d.validate() {
            errs.append(&mut e);
        }
        if errs.is_empty() { Ok(d) } else { Err(errs) }
    }
}

impl Writer<'_> {
    pub async fn plan_create_ip_list(
        &self,
        draft: &IpListDraft,
        if_missing: IfMissing,
    ) -> Result<Plan, WriteError> {
        draft.validate().map_err(WriteError::Invalid)?;
        if if_missing == IfMissing::SkipExisting {
            let lists = self.pce().ip_lists(self.bypass()).await?.value;
            if let Some(existing) = lists.iter().find(|l| l.name == draft.name.trim()) {
                return Ok(Plan::Noop {
                    existing_href: existing.href.clone(),
                    reason: format!("ip_list {} exists", draft.name.trim()),
                });
            }
        }
        Ok(Plan::Send {
            request: WriteRequest {
                method: WriteMethod::Post,
                path: self.pce().client().org_path("sec_policy/draft/ip_lists"),
                body: Some(draft.to_body()),
                summary: format!("create ip_list {}", draft.name.trim()),
            },
            kind: ObjectKind::IpList,
        })
    }

    pub async fn plan_update_ip_list(
        &self,
        href: &str,
        patch: &IpListPatch,
    ) -> Result<Plan, WriteError> {
        let href = checked_href(href, "ip_lists")?;
        if patch.is_empty() {
            return Err(WriteError::invalid("nothing to change"));
        }
        let current: IpList = serde_json::from_value(self.fetch(&href).await?)
            .map_err(|e| WriteError::Pce(crate::PceError::Decode(format!("{href}: {e}"))))?;
        let draft = patch.apply(&current).map_err(WriteError::Invalid)?;
        Ok(Plan::Send {
            request: WriteRequest {
                method: WriteMethod::Put,
                path: href.clone(),
                body: Some(draft.to_body()),
                summary: format!("update ip_list {}", current.name),
            },
            kind: ObjectKind::IpList,
        })
    }

    pub fn plan_delete_ip_list(&self, href: &str) -> Result<Plan, WriteError> {
        let href = checked_href(href, "ip_lists")?;
        Ok(Plan::Send {
            request: WriteRequest {
                method: WriteMethod::Delete,
                path: href.clone(),
                body: None,
                summary: format!("delete ip_list {href}"),
            },
            kind: ObjectKind::IpList,
        })
    }
}
```

Add to `mod.rs`: `pub mod ip_lists;` and `pub use ip_lists::{IpListDraft, IpListPatch};`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p pce --test write_contract`
Expected: all pass.

- [ ] **Step 5: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy -p pce --all-targets -- -D warnings && cargo test -p pce
git add crates/pce/src/write crates/pce/tests/write_contract.rs
git commit -m "feat(pce): IP list drafts, local patch application and create/update/delete plans

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 7: Services: draft, patch and planners

**Files:**
- Create: `crates/pce/src/write/services.rs`
- Modify: `crates/pce/src/write/mod.rs` (`pub mod services;`, `pub use services::{ServiceDraft, ServicePatch};`)
- Test: `crates/pce/tests/write_contract.rs` (append)

**Interfaces:**
- Consumes: `PortSpec`, `models::Service`, `Writer::fetch`, `checked_href`, `Pce::services(CacheMode::Bypass)`.
- Produces:

```rust
pub struct ServiceDraft { pub name: String, pub description: Option<String>, pub ports: Vec<PortSpec> }  // validate(), from_service(&Service)
pub struct ServicePatch { pub name: Option<String>, pub description: Option<String>, pub add_ports: Vec<PortSpec>, pub remove_ports: Vec<PortSpec> }  // is_empty(), apply(&Service) -> Result<ServiceDraft, Vec<String>>
impl Writer<'_> {
    pub async fn plan_create_service(&self, draft: &ServiceDraft, if_missing: IfMissing) -> Result<Plan, WriteError>;
    pub async fn plan_update_service(&self, href: &str, patch: &ServicePatch) -> Result<Plan, WriteError>;
    pub fn plan_delete_service(&self, href: &str) -> Result<Plan, WriteError>;
}
```

- [ ] **Step 1: Write the failing tests** (append to `crates/pce/tests/write_contract.rs`)

```rust
mod services {
    use super::*;
    use pce::write::{PortSpec, ServiceDraft, ServicePatch};

    fn pg() -> Value {
        json!({
            "href": "/orgs/5/sec_policy/draft/services/7", "name": "PostgreSQL",
            "service_ports": [{"port": 5432, "proto": 6}]
        })
    }

    #[test]
    fn draft_validates_and_sends_only_service_ports() {
        let d = ServiceDraft { name: "PG".into(), description: None, ports: vec![] };
        assert!(d.validate().is_err(), "a service needs a port");
        let d = ServiceDraft {
            name: "PG".into(),
            description: Some("db".into()),
            ports: vec![PortSpec::parse("5432/tcp").unwrap(), PortSpec::parse("icmp").unwrap()],
        };
        assert!(d.validate().is_ok());
    }

    #[tokio::test]
    async fn create_plan_body() {
        let server = MockServer::start().await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let d = ServiceDraft {
            name: "PostgreSQL".into(),
            description: Some("db".into()),
            ports: vec![PortSpec::parse("5432/tcp").unwrap(), PortSpec::parse("8000-8100/udp").unwrap()],
        };
        let plan = pce.writer(&guard).plan_create_service(&d, IfMissing::Create).await.unwrap();
        let req = plan.request().unwrap();
        assert_eq!(req.method, WriteMethod::Post);
        assert_eq!(req.path, "/orgs/5/sec_policy/draft/services");
        let body = req.body.clone().unwrap();
        assert_eq!(
            body,
            json!({"name": "PostgreSQL", "description": "db",
                   "service_ports": [{"port": 5432, "proto": 6}, {"port": 8000, "to_port": 8100, "proto": 17}]})
        );
        assert!(body.get("windows_services").is_none(), "exactly one port list key");
        assert_eq!(req.summary, "create service PostgreSQL");
    }

    #[tokio::test]
    async fn if_missing_skips_exact_name() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/sec_policy/draft/services"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([pg()])))
            .mount(&server)
            .await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let d = |n: &str| ServiceDraft { name: n.into(), description: None, ports: vec![PortSpec::parse("1/tcp").unwrap()] };
        assert!(matches!(
            pce.writer(&guard).plan_create_service(&d("PostgreSQL"), IfMissing::SkipExisting).await.unwrap(),
            Plan::Noop { .. }
        ));
        assert!(pce.writer(&guard).plan_create_service(&d("Postgres"), IfMissing::SkipExisting).await.unwrap().request().is_some());
    }

    #[test]
    fn patch_adds_and_removes_ports() {
        let current: pce::models::Service = serde_json::from_value(pg()).unwrap();
        let patch = ServicePatch {
            name: Some("PG".into()),
            description: None,
            add_ports: vec![PortSpec::parse("5433/tcp").unwrap(), PortSpec::parse("5432/tcp").unwrap()],
            remove_ports: vec![],
        };
        let d = patch.apply(&current).unwrap();
        assert_eq!(d.name, "PG");
        assert_eq!(d.ports.iter().map(PortSpec::display).collect::<Vec<_>>(), vec!["5432/tcp", "5433/tcp"]);
        let patch = ServicePatch { remove_ports: vec![PortSpec::parse("5432/tcp").unwrap()], ..ServicePatch::default() };
        let errs = patch.apply(&current).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("at least one")), "removing the last port: {errs:?}");
        let patch = ServicePatch { remove_ports: vec![PortSpec::parse("99/tcp").unwrap()], ..ServicePatch::default() };
        assert!(patch.apply(&current).unwrap_err()[0].contains("99/tcp"));
    }

    #[tokio::test]
    async fn update_and_delete_plans() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/sec_policy/draft/services/7"))
            .respond_with(ResponseTemplate::new(200).set_body_json(pg()))
            .expect(1)
            .mount(&server)
            .await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let patch = ServicePatch { add_ports: vec![PortSpec::parse("5433/tcp").unwrap()], ..ServicePatch::default() };
        let plan = pce.writer(&guard).plan_update_service("/orgs/5/sec_policy/draft/services/7", &patch).await.unwrap();
        let req = plan.request().unwrap();
        assert_eq!(req.method, WriteMethod::Put);
        assert_eq!(
            req.body.clone().unwrap()["service_ports"],
            json!([{"port": 5432, "proto": 6}, {"port": 5433, "proto": 6}])
        );
        let del = pce.writer(&guard).plan_delete_service("/orgs/5/sec_policy/active/services/7").unwrap();
        assert_eq!(del.request().unwrap().path, "/orgs/5/sec_policy/draft/services/7");
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p pce --test write_contract services`
Expected: compile error.

- [ ] **Step 3: Implement `crates/pce/src/write/services.rs`**

```rust
//! Services under `/sec_policy/draft/services`. The body carries exactly
//! one port list, `service_ports`; the PCE silently drops a list it does
//! not accept and answers 201, which `Written::dropped_fields` reports.
use super::specs::PortSpec;
use super::{IfMissing, ObjectKind, Plan, WriteError, WriteRequest, Writer, checked_href};
use crate::client::WriteMethod;
use crate::models::Service;
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceDraft {
    pub name: String,
    pub description: Option<String>,
    pub ports: Vec<PortSpec>,
}

impl ServiceDraft {
    pub fn from_service(s: &Service) -> Self {
        Self {
            name: s.name.clone(),
            description: s.description.clone(),
            ports: s.service_ports.iter().filter_map(PortSpec::from_service_port).collect(),
        }
    }

    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errs = Vec::new();
        if self.name.trim().is_empty() {
            errs.push("service name must not be empty".into());
        }
        if self.ports.is_empty() {
            errs.push("a service needs at least one port or protocol".into());
        }
        if errs.is_empty() { Ok(()) } else { Err(errs) }
    }

    pub(crate) fn to_body(&self) -> Value {
        let mut v = json!({
            "name": self.name.trim(),
            "service_ports": self.ports.iter().map(PortSpec::to_json).collect::<Vec<_>>(),
        });
        if let Some(d) = &self.description {
            v["description"] = json!(d);
        }
        v
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServicePatch {
    pub name: Option<String>,
    pub description: Option<String>,
    pub add_ports: Vec<PortSpec>,
    pub remove_ports: Vec<PortSpec>,
}

impl ServicePatch {
    pub fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.description.is_none()
            && self.add_ports.is_empty()
            && self.remove_ports.is_empty()
    }

    pub fn apply(&self, current: &Service) -> Result<ServiceDraft, Vec<String>> {
        let mut d = ServiceDraft::from_service(current);
        let mut errs = Vec::new();
        for p in &self.remove_ports {
            let before = d.ports.len();
            d.ports.retain(|x| x != p);
            if d.ports.len() == before {
                errs.push(format!("port {} is not in {}", p.display(), current.name));
            }
        }
        for p in &self.add_ports {
            if !d.ports.contains(p) {
                d.ports.push(p.clone());
            }
        }
        if let Some(n) = &self.name {
            d.name = n.clone();
        }
        if let Some(desc) = &self.description {
            d.description = Some(desc.clone());
        }
        if let Err(mut e) = d.validate() {
            errs.append(&mut e);
        }
        if errs.is_empty() { Ok(d) } else { Err(errs) }
    }
}

impl Writer<'_> {
    pub async fn plan_create_service(
        &self,
        draft: &ServiceDraft,
        if_missing: IfMissing,
    ) -> Result<Plan, WriteError> {
        draft.validate().map_err(WriteError::Invalid)?;
        if if_missing == IfMissing::SkipExisting {
            let services = self.pce().services(self.bypass()).await?.value;
            if let Some(existing) = services.iter().find(|s| s.name == draft.name.trim()) {
                return Ok(Plan::Noop {
                    existing_href: existing.href.clone(),
                    reason: format!("service {} exists", draft.name.trim()),
                });
            }
        }
        Ok(Plan::Send {
            request: WriteRequest {
                method: WriteMethod::Post,
                path: self.pce().client().org_path("sec_policy/draft/services"),
                body: Some(draft.to_body()),
                summary: format!("create service {}", draft.name.trim()),
            },
            kind: ObjectKind::Service,
        })
    }

    pub async fn plan_update_service(
        &self,
        href: &str,
        patch: &ServicePatch,
    ) -> Result<Plan, WriteError> {
        let href = checked_href(href, "services")?;
        if patch.is_empty() {
            return Err(WriteError::invalid("nothing to change"));
        }
        let current: Service = serde_json::from_value(self.fetch(&href).await?)
            .map_err(|e| WriteError::Pce(crate::PceError::Decode(format!("{href}: {e}"))))?;
        let draft = patch.apply(&current).map_err(WriteError::Invalid)?;
        Ok(Plan::Send {
            request: WriteRequest {
                method: WriteMethod::Put,
                path: href.clone(),
                body: Some(draft.to_body()),
                summary: format!("update service {}", current.name),
            },
            kind: ObjectKind::Service,
        })
    }

    pub fn plan_delete_service(&self, href: &str) -> Result<Plan, WriteError> {
        let href = checked_href(href, "services")?;
        Ok(Plan::Send {
            request: WriteRequest {
                method: WriteMethod::Delete,
                path: href.clone(),
                body: None,
                summary: format!("delete service {href}"),
            },
            kind: ObjectKind::Service,
        })
    }
}
```

Add to `mod.rs`: `pub mod services;` and `pub use services::{ServiceDraft, ServicePatch};`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p pce --test write_contract`
Expected: all pass.

- [ ] **Step 5: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy -p pce --all-targets -- -D warnings && cargo test -p pce
git add crates/pce/src/write crates/pce/tests/write_contract.rs
git commit -m "feat(pce): service drafts with a single service_ports list and create/update/delete plans

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 8: Unmanaged workloads: draft, patch, lookup and planners

**Files:**
- Create: `crates/pce/src/write/workloads.rs`
- Modify: `crates/pce/src/write/mod.rs` (`pub mod workloads;`, `pub use workloads::{LabelRemoval, WorkloadDraft, WorkloadPatch};`)
- Test: `crates/pce/tests/write_contract.rs` (append)

**Interfaces:**
- Consumes: `InterfaceSpec`, `models::{Workload, WorkloadFilter, WorkloadLabel}`, `NameIndex::label`, `Pce::workloads(filter, CacheMode::Bypass)`, `Writer::fetch`, `checked_href`.
- Produces:

```rust
pub struct WorkloadDraft { pub name: String, pub hostname: Option<String>, pub description: Option<String>, pub interfaces: Vec<InterfaceSpec>, pub labels: Vec<String> /* hrefs */ }  // validate()
pub struct LabelRemoval { pub key: String, pub value: Option<String> }
pub struct WorkloadPatch { pub name: Option<String>, pub hostname: Option<String>, pub description: Option<String>, pub add_addresses: Vec<String>, pub remove_addresses: Vec<String>, pub set_labels: Option<Vec<String>>, pub add_labels: Vec<String>, pub remove_labels: Vec<LabelRemoval> }
//   is_empty(); apply(&Workload, &NameIndex) -> Result<Value /* PUT body with only the changed keys */, Vec<String>>
impl Writer<'_> {
    pub async fn find_workload(&self, name_or_hostname: &str) -> Result<Workload, WriteError>;   // exact hostname (case-insensitive), then exact name; 0 or >1 is Invalid
    pub async fn plan_create_workload(&self, draft: &WorkloadDraft, if_missing: IfMissing) -> Result<Plan, WriteError>;
    pub async fn plan_update_workload(&self, href: &str, patch: &WorkloadPatch, idx: &NameIndex) -> Result<Plan, WriteError>;
    pub async fn plan_delete_workload(&self, href: &str) -> Result<Plan, WriteError>;
}
```

- [ ] **Step 1: Write the failing tests** (append to `crates/pce/tests/write_contract.rs`)

```rust
mod workloads {
    use super::*;
    use pce::NameIndex;
    use pce::models::Label;
    use pce::write::{InterfaceSpec, LabelRemoval, WorkloadDraft, WorkloadPatch};

    fn printer() -> Value {
        json!({
            "href": "/orgs/5/workloads/u1", "name": "printer-1", "hostname": "printer-1.lab",
            "managed": false, "interfaces": [{"name": "eth0", "address": "10.0.0.50", "cidr_block": 24}],
            "labels": [{"href": "/orgs/5/labels/1", "key": "app", "value": "print"}]
        })
    }

    fn managed() -> Value {
        json!({"href": "/orgs/5/workloads/m1", "hostname": "web-1", "managed": true, "interfaces": [], "labels": []})
    }

    fn idx() -> NameIndex {
        let labels: Vec<Label> = serde_json::from_value(json!([
            {"href": "/orgs/5/labels/1", "key": "app", "value": "print"},
            {"href": "/orgs/5/labels/2", "key": "env", "value": "lab"},
            {"href": "/orgs/5/labels/3", "key": "app", "value": "scan"}
        ]))
        .unwrap();
        NameIndex::new(&labels, &[], &[], &[])
    }

    #[test]
    fn draft_validates_and_defaults_hostname_to_name() {
        let d = WorkloadDraft { name: "printer-1".into(), hostname: None, description: None, interfaces: vec![], labels: vec![] };
        assert!(d.validate().is_ok(), "hostname defaults to the name, which satisfies the rule");
        let d = WorkloadDraft { name: " ".into(), ..d };
        assert!(d.validate().is_err());
    }

    #[tokio::test]
    async fn create_plan_body() {
        let server = MockServer::start().await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let d = WorkloadDraft {
            name: "printer-1".into(),
            hostname: None,
            description: Some("lobby printer".into()),
            interfaces: vec![InterfaceSpec::parse("10.0.0.50/24", "eth0".into()).unwrap()],
            labels: vec!["/orgs/5/labels/1".into()],
        };
        let plan = pce.writer(&guard).plan_create_workload(&d, IfMissing::Create).await.unwrap();
        let req = plan.request().unwrap();
        assert_eq!(req.method, WriteMethod::Post);
        assert_eq!(req.path, "/orgs/5/workloads");
        assert_eq!(
            req.body,
            Some(json!({
                "name": "printer-1", "hostname": "printer-1", "description": "lobby printer",
                "interfaces": [{"name": "eth0", "address": "10.0.0.50", "cidr_block": 24}],
                "labels": [{"href": "/orgs/5/labels/1"}]
            }))
        );
        assert_eq!(req.summary, "create workload printer-1");
    }

    #[tokio::test]
    async fn find_workload_matches_hostname_then_name_exactly() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/workloads"))
            .and(wiremock::matchers::query_param("hostname", "printer-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([printer(), {
                "href": "/orgs/5/workloads/u2", "hostname": "printer-10.lab", "managed": false
            }])))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/workloads"))
            .and(wiremock::matchers::query_param("name", "printer-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([printer()])))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/workloads"))
            .and(wiremock::matchers::query_param("hostname", "PRINTER-1.LAB"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([printer()])))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/workloads"))
            .and(wiremock::matchers::query_param("hostname", "nope"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/workloads"))
            .and(wiremock::matchers::query_param("name", "nope"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let w = pce.writer(&guard);
        // No hostname equals "printer-1" exactly, so the name lookup finds it.
        assert_eq!(w.find_workload("printer-1").await.unwrap().href, "/orgs/5/workloads/u1");
        assert_eq!(w.find_workload("PRINTER-1.LAB").await.unwrap().href, "/orgs/5/workloads/u1");
        let err = w.find_workload("nope").await.unwrap_err();
        assert!(matches!(err, WriteError::Invalid(_)), "{err:?}");
        assert!(err.to_string().contains("nope"));
    }

    #[tokio::test]
    async fn find_workload_refuses_ambiguity_and_lists_candidates() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/workloads"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {"href": "/orgs/5/workloads/a", "hostname": "dup", "managed": false},
                {"href": "/orgs/5/workloads/b", "hostname": "dup", "managed": false}
            ])))
            .mount(&server)
            .await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let err = pce.writer(&guard).find_workload("dup").await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("/orgs/5/workloads/a") && msg.contains("/orgs/5/workloads/b"), "{msg}");
    }

    #[test]
    fn patch_sends_only_changed_keys() {
        let current: pce::models::Workload = serde_json::from_value(printer()).unwrap();
        let patch = WorkloadPatch { description: Some("moved".into()), ..WorkloadPatch::default() };
        assert_eq!(patch.apply(&current, &idx()).unwrap(), json!({"description": "moved"}));
        let patch = WorkloadPatch {
            add_addresses: vec!["10.0.1.5".into()],
            remove_addresses: vec!["10.0.0.50".into()],
            add_labels: vec!["/orgs/5/labels/2".into()],
            ..WorkloadPatch::default()
        };
        let body = patch.apply(&current, &idx()).unwrap();
        // Removes run before adds, so the freed name is reused.
        assert_eq!(body["interfaces"], json!([{"name": "eth0", "address": "10.0.1.5"}]));
        assert_eq!(body["labels"], json!([{"href": "/orgs/5/labels/1"}, {"href": "/orgs/5/labels/2"}]));
        assert!(body.get("name").is_none() && body.get("hostname").is_none());
    }

    #[test]
    fn label_removal_by_key_or_pair_and_set_labels() {
        let current: pce::models::Workload = serde_json::from_value(printer()).unwrap();
        let patch = WorkloadPatch {
            remove_labels: vec![LabelRemoval { key: "app".into(), value: None }],
            add_labels: vec!["/orgs/5/labels/3".into()],
            ..WorkloadPatch::default()
        };
        let body = patch.apply(&current, &idx()).unwrap();
        assert_eq!(body["labels"], json!([{"href": "/orgs/5/labels/3"}]));
        let patch = WorkloadPatch {
            remove_labels: vec![LabelRemoval { key: "app".into(), value: Some("scan".into()) }],
            ..WorkloadPatch::default()
        };
        let errs = patch.apply(&current, &idx()).unwrap_err();
        assert!(errs[0].contains("app=scan"), "{errs:?}");
        let patch = WorkloadPatch { set_labels: Some(vec![]), ..WorkloadPatch::default() };
        assert_eq!(patch.apply(&current, &idx()).unwrap(), json!({"labels": []}));
        assert!(WorkloadPatch::default().is_empty());
    }

    #[test]
    fn removing_the_last_interface_without_a_hostname_is_refused() {
        let mut v = printer();
        v["hostname"] = Value::Null;
        let current: pce::models::Workload = serde_json::from_value(v).unwrap();
        let patch = WorkloadPatch { remove_addresses: vec!["10.0.0.50".into()], ..WorkloadPatch::default() };
        let errs = patch.apply(&current, &idx()).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("hostname") || e.contains("interface")), "{errs:?}");
        let patch = WorkloadPatch { remove_addresses: vec!["1.2.3.4".into()], ..WorkloadPatch::default() };
        assert!(patch.apply(&current, &idx()).unwrap_err()[0].contains("1.2.3.4"));
    }

    #[tokio::test]
    async fn update_and_delete_refuse_managed_workloads() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/workloads/m1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(managed()))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/workloads/u1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(printer()))
            .mount(&server)
            .await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let w = pce.writer(&guard);
        let patch = WorkloadPatch { description: Some("x".into()), ..WorkloadPatch::default() };
        let err = w.plan_update_workload("/orgs/5/workloads/m1", &patch, &idx()).await.unwrap_err();
        assert!(err.to_string().contains("VEN-managed"), "{err}");
        let err = w.plan_delete_workload("/orgs/5/workloads/m1").await.unwrap_err();
        assert!(err.to_string().contains("VEN-managed"), "{err}");
        let plan = w.plan_update_workload("/orgs/5/workloads/u1", &patch, &idx()).await.unwrap();
        let req = plan.request().unwrap();
        assert_eq!(req.method, WriteMethod::Put);
        assert_eq!(req.path, "/orgs/5/workloads/u1");
        assert_eq!(req.body, Some(json!({"description": "x"})));
        let plan = w.plan_delete_workload("/orgs/5/workloads/u1").await.unwrap();
        assert_eq!(plan.request().unwrap().method, WriteMethod::Delete);
        assert_eq!(plan.request().unwrap().summary, "delete workload printer-1.lab");
    }

    #[tokio::test]
    async fn if_missing_skips_an_existing_hostname() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/workloads"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([printer()])))
            .mount(&server)
            .await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let d = WorkloadDraft { name: "printer-1".into(), hostname: Some("printer-1.lab".into()), description: None, interfaces: vec![], labels: vec![] };
        let plan = pce.writer(&guard).plan_create_workload(&d, IfMissing::SkipExisting).await.unwrap();
        assert!(matches!(plan, Plan::Noop { .. }));
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p pce --test write_contract workloads`
Expected: compile error.

- [ ] **Step 3: Implement `crates/pce/src/write/workloads.rs`**

```rust
//! Unmanaged workloads: `POST /orgs/{org}/workloads`, `PUT|DELETE
//! /orgs/{org}/workloads/{id}`. A VEN-managed workload is refused locally:
//! the VEN owns its hostname and interfaces, and label changes on managed
//! workloads are a later plan. `labels` and `interfaces` are replaced
//! wholesale by the PCE, so a patch sends the full list it computed.
use super::specs::InterfaceSpec;
use super::{IfMissing, ObjectKind, Plan, WriteError, WriteRequest, Writer, checked_href};
use crate::client::WriteMethod;
use crate::models::{Workload, WorkloadFilter};
use crate::resolve::NameIndex;
use serde_json::{Map, Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkloadDraft {
    pub name: String,
    pub hostname: Option<String>,
    pub description: Option<String>,
    pub interfaces: Vec<InterfaceSpec>,
    /// Label hrefs.
    pub labels: Vec<String>,
}

impl WorkloadDraft {
    pub fn hostname(&self) -> String {
        self.hostname
            .as_deref()
            .map(str::trim)
            .filter(|h| !h.is_empty())
            .unwrap_or(self.name.trim())
            .to_string()
    }

    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errs = Vec::new();
        if self.name.trim().is_empty() {
            errs.push("workload name must not be empty".into());
        }
        if self.hostname().is_empty() && self.interfaces.is_empty() {
            errs.push("a workload needs a hostname or at least one interface".into());
        }
        if errs.is_empty() { Ok(()) } else { Err(errs) }
    }

    pub(crate) fn to_body(&self) -> Value {
        let mut v = json!({
            "name": self.name.trim(),
            "hostname": self.hostname(),
            "interfaces": self.interfaces.iter().map(InterfaceSpec::to_json).collect::<Vec<_>>(),
            "labels": self.labels.iter().map(|h| json!({"href": h})).collect::<Vec<_>>(),
        });
        if let Some(d) = &self.description {
            v["description"] = json!(d);
        }
        v
    }
}

/// `--remove-label app` (any value) or `--remove-label app=web`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelRemoval {
    pub key: String,
    pub value: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkloadPatch {
    pub name: Option<String>,
    pub hostname: Option<String>,
    pub description: Option<String>,
    /// `ADDR[/N]` strings; interface names are assigned on apply.
    pub add_addresses: Vec<String>,
    pub remove_addresses: Vec<String>,
    /// Replace the whole label set (hrefs). Conflicts with add/remove.
    pub set_labels: Option<Vec<String>>,
    pub add_labels: Vec<String>,
    pub remove_labels: Vec<LabelRemoval>,
}

fn next_interface_name(existing: &[InterfaceSpec]) -> String {
    let mut n = 0;
    loop {
        let name = format!("eth{n}");
        if !existing.iter().any(|i| i.name == name) {
            return name;
        }
        n += 1;
    }
}

impl WorkloadPatch {
    pub fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.hostname.is_none()
            && self.description.is_none()
            && self.add_addresses.is_empty()
            && self.remove_addresses.is_empty()
            && self.set_labels.is_none()
            && self.add_labels.is_empty()
            && self.remove_labels.is_empty()
    }

    /// The `PUT` body: only the keys this patch changes. `idx` supplies a
    /// label's key when the workload's own label entry lacks it.
    pub fn apply(&self, current: &Workload, idx: &NameIndex) -> Result<Value, Vec<String>> {
        let mut body = Map::new();
        let mut errs = Vec::new();
        let who = current
            .hostname
            .clone()
            .or_else(|| current.name.clone())
            .unwrap_or_else(|| current.href.clone());

        if let Some(n) = &self.name {
            body.insert("name".into(), json!(n));
        }
        if let Some(h) = &self.hostname {
            body.insert("hostname".into(), json!(h));
        }
        if let Some(d) = &self.description {
            body.insert("description".into(), json!(d));
        }

        if !self.add_addresses.is_empty() || !self.remove_addresses.is_empty() {
            let mut ifaces: Vec<InterfaceSpec> =
                current.interfaces.iter().map(InterfaceSpec::from_interface).collect();
            for a in &self.remove_addresses {
                let addr = a.split('/').next().unwrap_or(a).trim();
                let before = ifaces.len();
                ifaces.retain(|i| i.address != addr);
                if ifaces.len() == before {
                    errs.push(format!("address {addr} is not an interface of {who}"));
                }
            }
            for a in &self.add_addresses {
                match InterfaceSpec::parse(a, next_interface_name(&ifaces)) {
                    Ok(spec) => {
                        if !ifaces.iter().any(|i| i.address == spec.address) {
                            ifaces.push(spec);
                        }
                    }
                    Err(e) => errs.push(e),
                }
            }
            let hostname = self
                .hostname
                .clone()
                .or_else(|| current.hostname.clone())
                .unwrap_or_default();
            if ifaces.is_empty() && hostname.trim().is_empty() {
                errs.push(format!(
                    "{who} would have no interface and no hostname; keep one interface or set --hostname"
                ));
            }
            body.insert(
                "interfaces".into(),
                Value::Array(ifaces.iter().map(InterfaceSpec::to_json).collect()),
            );
        }

        if let Some(set) = &self.set_labels {
            body.insert(
                "labels".into(),
                Value::Array(set.iter().map(|h| json!({"href": h})).collect()),
            );
        } else if !self.add_labels.is_empty() || !self.remove_labels.is_empty() {
            // (href, key, value) for every current label.
            let mut labels: Vec<(String, String, String)> = current
                .labels
                .iter()
                .map(|l| {
                    let from_idx = idx.label(&l.href);
                    (
                        l.href.clone(),
                        l.key
                            .clone()
                            .or_else(|| from_idx.map(|x| x.key.clone()))
                            .unwrap_or_default(),
                        l.value
                            .clone()
                            .or_else(|| from_idx.map(|x| x.value.clone()))
                            .unwrap_or_default(),
                    )
                })
                .collect();
            for r in &self.remove_labels {
                let before = labels.len();
                labels.retain(|(_, k, v)| {
                    !(k == &r.key && r.value.as_deref().is_none_or(|rv| rv == v))
                });
                if labels.len() == before {
                    let what = match &r.value {
                        Some(v) => format!("{}={v}", r.key),
                        None => r.key.clone(),
                    };
                    errs.push(format!("{who} has no label {what}"));
                }
            }
            for h in &self.add_labels {
                if !labels.iter().any(|(x, _, _)| x == h) {
                    let (k, v) = idx
                        .label(h)
                        .map(|l| (l.key.clone(), l.value.clone()))
                        .unwrap_or_default();
                    labels.push((h.clone(), k, v));
                }
            }
            body.insert(
                "labels".into(),
                Value::Array(labels.iter().map(|(h, _, _)| json!({"href": h})).collect()),
            );
        }

        if errs.is_empty() { Ok(Value::Object(body)) } else { Err(errs) }
    }
}

fn require_unmanaged(w: &Workload) -> Result<(), WriteError> {
    if w.managed == Some(true) {
        let who = w.hostname.clone().or_else(|| w.name.clone()).unwrap_or_else(|| w.href.clone());
        return Err(WriteError::invalid(format!(
            "{who} is VEN-managed; this version writes unmanaged workloads only"
        )));
    }
    Ok(())
}

impl Writer<'_> {
    /// Exact match on hostname (case-insensitive), then on name. The PCE's
    /// `?hostname=` and `?name=` are substring filters, so the result is
    /// narrowed here; none or several matches is an error listing them.
    pub async fn find_workload(&self, name_or_hostname: &str) -> Result<Workload, WriteError> {
        let needle = name_or_hostname.trim();
        let by_host = self
            .pce()
            .workloads(
                &WorkloadFilter {
                    hostname: Some(needle.to_string()),
                    ..WorkloadFilter::default()
                },
                self.bypass(),
            )
            .await?
            .value;
        let mut hits: Vec<Workload> = by_host
            .into_iter()
            .filter(|w| w.hostname.as_deref().is_some_and(|h| h.eq_ignore_ascii_case(needle)))
            .collect();
        if hits.is_empty() {
            let by_name = self
                .pce()
                .workloads(
                    &WorkloadFilter {
                        name: Some(needle.to_string()),
                        ..WorkloadFilter::default()
                    },
                    self.bypass(),
                )
                .await?
                .value;
            hits = by_name
                .into_iter()
                .filter(|w| w.name.as_deref() == Some(needle))
                .collect();
        }
        match hits.len() {
            0 => Err(WriteError::invalid(format!(
                "no workload with hostname or name {needle:?}"
            ))),
            1 => Ok(hits.remove(0)),
            _ => Err(WriteError::invalid(format!(
                "{needle:?} matches several workloads: {}; use the href",
                hits.iter().map(|w| w.href.as_str()).collect::<Vec<_>>().join(", ")
            ))),
        }
    }

    pub async fn plan_create_workload(
        &self,
        draft: &WorkloadDraft,
        if_missing: IfMissing,
    ) -> Result<Plan, WriteError> {
        draft.validate().map_err(WriteError::Invalid)?;
        if if_missing == IfMissing::SkipExisting
            && let Ok(existing) = self.find_workload(&draft.hostname()).await
        {
            return Ok(Plan::Noop {
                existing_href: existing.href.clone(),
                reason: format!("workload {} exists", draft.hostname()),
            });
        }
        Ok(Plan::Send {
            request: WriteRequest {
                method: WriteMethod::Post,
                path: self.pce().client().org_path("workloads"),
                body: Some(draft.to_body()),
                summary: format!("create workload {}", draft.name.trim()),
            },
            kind: ObjectKind::Workload,
        })
    }

    async fn fetch_workload(&self, href: &str) -> Result<Workload, WriteError> {
        serde_json::from_value(self.fetch(href).await?)
            .map_err(|e| WriteError::Pce(crate::PceError::Decode(format!("{href}: {e}"))))
    }

    pub async fn plan_update_workload(
        &self,
        href: &str,
        patch: &WorkloadPatch,
        idx: &NameIndex,
    ) -> Result<Plan, WriteError> {
        let href = checked_href(href, "workloads")?;
        if patch.is_empty() {
            return Err(WriteError::invalid("nothing to change"));
        }
        let current = self.fetch_workload(&href).await?;
        require_unmanaged(&current)?;
        let body = patch.apply(&current, idx).map_err(WriteError::Invalid)?;
        let who = current.hostname.clone().or(current.name.clone()).unwrap_or(href.clone());
        Ok(Plan::Send {
            request: WriteRequest {
                method: WriteMethod::Put,
                path: href,
                body: Some(body),
                summary: format!("update workload {who}"),
            },
            kind: ObjectKind::Workload,
        })
    }

    pub async fn plan_delete_workload(&self, href: &str) -> Result<Plan, WriteError> {
        let href = checked_href(href, "workloads")?;
        let current = self.fetch_workload(&href).await?;
        require_unmanaged(&current)?;
        let who = current.hostname.clone().or(current.name.clone()).unwrap_or(href.clone());
        Ok(Plan::Send {
            request: WriteRequest {
                method: WriteMethod::Delete,
                path: href,
                body: None,
                summary: format!("delete workload {who}"),
            },
            kind: ObjectKind::Workload,
        })
    }
}
```

Note: `find_workload` in `plan_create_workload` with `IfMissing::SkipExisting` turns an ambiguous match into "exists" only when exactly one matches; an ambiguous hostname is an `Err` from `find_workload`, which `let Ok(...)` ignores, so the create proceeds. That is the intended behaviour (the PCE allows duplicate hostnames for unmanaged workloads).

Add to `mod.rs`: `pub mod workloads;` and `pub use workloads::{LabelRemoval, WorkloadDraft, WorkloadPatch};`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p pce --test write_contract`
Expected: all pass.

- [ ] **Step 5: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy -p pce --all-targets -- -D warnings && cargo test -p pce
git add crates/pce/src/write crates/pce/tests/write_contract.rs
git commit -m "feat(pce): unmanaged workload drafts, patches, exact lookup and plans; managed workloads refused

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 9: CLI plumbing: `writes` setting, `--allow-writes`, exit code 6, envelope and shared write helpers

**Files:**
- Modify: `crates/illumio-tui/src/config.rs` (`Profile`, its `Debug`, `ResolvedConfig`, `resolve`, tests)
- Modify: `crates/illumio-tui/src/cli/mod.rs` (`GlobalArgs.allow_writes`, `pub mod write;`)
- Modify: `crates/illumio-tui/src/cli/error.rs`
- Modify: `crates/illumio-tui/src/cli/context.rs` (`allow_writes`, `emit_write`)
- Modify: `crates/illumio-tui/src/cli/output.rs` (`WriteMeta`, `Meta.write`, `meta_line`)
- Modify: `crates/illumio-tui/src/cli/profile.rs:64` (`write: None` in the `Meta` literal)
- Modify: `crates/illumio-tui/src/cli/health.rs` (`writes`)
- Modify: `crates/pce/src/resolve.rs` (`ip_lists_named`, `services_named`)
- Create: `crates/illumio-tui/src/cli/write.rs`
- Create: `crates/illumio-tui/tests/cli_writes.rs`

**Interfaces:**
- Consumes: `pce::write::{WriteGuard, WriteContext, WriteRefused, WriteError, JsonlAudit, Plan, Outcome, Written, WriteRequest}`, `pce::WriteMethod`, `NameIndex`.
- Produces:

```rust
// config.rs
pub struct Profile { ..., pub writes: Option<bool>, ... }
pub struct ResolvedConfig { ..., pub writes: bool }
// cli/mod.rs
pub struct GlobalArgs { ..., pub allow_writes: bool }
// cli/error.rs
pub enum CliError { ..., Write(pce::write::WriteError) }   // exit: Refused 6, Invalid 2, Unanswered 4, Pce(e) as Pce(e)
// cli/output.rs
pub struct WriteMeta { pub method: WriteMethod, pub path: String, pub status: u16, pub audit: String }
pub struct Meta { ..., pub write: Option<WriteMeta> }
// cli/context.rs
pub struct Context { ..., pub allow_writes: bool }
impl Context { pub fn emit_write(&self, data: &WriteOutput, write: Option<WriteMeta>) -> Result<(), CliError> }
// cli/write.rs
pub struct WriteFlags { pub dry_run: bool, pub yes: bool }          // clap Args, flattened into every write command
pub fn prepare(ctx: &Context, flags: &WriteFlags) -> Result<WriteGuard, CliError>   // guard, then the --yes check
pub enum WriteOutput { DryRun(WriteRequest), Written(Written), Skipped { skipped: bool, href: String, reason: String } }
pub async fn run_plan(ctx: &Context, flags: &WriteFlags, guard: &WriteGuard, plan: Plan) -> Result<(), CliError>
pub fn parse_kv(spec: &str) -> Result<(String, String), CliError>
pub fn is_href(s: &str) -> bool
pub async fn resolve_label(ctx: &Context, spec: &str) -> Result<String, CliError>
pub async fn resolve_labels(ctx: &Context, specs: &[String]) -> Result<Vec<String>, CliError>
pub async fn resolve_ip_list(ctx: &Context, name: &str) -> Result<String, CliError>
pub async fn resolve_service(ctx: &Context, name: &str) -> Result<String, CliError>
// pce resolve.rs
impl NameIndex { pub fn ip_lists_named(&self, name: &str) -> Vec<&IpList>; pub fn services_named(&self, name: &str) -> Vec<&Service> }
```

- [ ] **Step 1: Write the failing tests**

Config, in `crates/illumio-tui/src/config.rs` `mod tests` (the `file()` helper and `env()` already exist there):

```rust
    #[test]
    fn writes_default_off_profile_enables_env_outranks() {
        let f = file();
        let r = resolve(&f, &Overrides::default(), &env(&[])).unwrap();
        assert!(!r.writes);
        let mut f2 = f.clone();
        f2.profiles.get_mut("demo").unwrap().writes = Some(true);
        assert!(resolve(&f2, &Overrides::default(), &env(&[])).unwrap().writes);
        let r = resolve(&f2, &Overrides::default(), &env(&[("ILLUMIO_WRITES", "false")])).unwrap();
        assert!(!r.writes, "env outranks the profile");
        let r = resolve(&f, &Overrides::default(), &env(&[("ILLUMIO_WRITES", "1")])).unwrap();
        assert!(r.writes);
        let shown = format!("{:?}", f2.profiles["demo"]);
        assert!(shown.contains("writes: Some(true)"), "{shown}");
        assert!(!shown.contains("file-secret"));
    }
```

`crates/pce/src/resolve.rs` `mod tests` (add; look at how the existing tests build a `NameIndex` and reuse their fixtures):

```rust
    #[test]
    fn exact_name_lookups_are_case_sensitive_and_list_duplicates() {
        let lists: Vec<IpList> = serde_json::from_str(r#"[
            {"href": "/orgs/5/sec_policy/draft/ip_lists/1", "name": "Corp"},
            {"href": "/orgs/5/sec_policy/draft/ip_lists/2", "name": "Corp"},
            {"href": "/orgs/5/sec_policy/draft/ip_lists/3", "name": "corp"}
        ]"#).unwrap();
        let services: Vec<Service> = serde_json::from_str(r#"[
            {"href": "/orgs/5/sec_policy/draft/services/1", "name": "PG"}
        ]"#).unwrap();
        let idx = NameIndex::new(&[], &[], &lists, &services);
        let hits = idx.ip_lists_named("Corp");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].href, "/orgs/5/sec_policy/draft/ip_lists/1", "sorted by href");
        assert_eq!(idx.ip_lists_named("corp").len(), 1);
        assert_eq!(idx.services_named("PG").len(), 1);
        assert!(idx.services_named("pg").is_empty());
    }
```

Unit tests inside `crates/illumio-tui/src/cli/write.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_kv_splits_on_the_first_equals() {
        assert_eq!(parse_kv("app=a=b").unwrap(), ("app".into(), "a=b".into()));
        assert_eq!(parse_kv(" env = prod ").unwrap(), ("env".into(), "prod".into()));
        assert!(parse_kv("app").is_err());
        assert!(parse_kv("=x").is_err());
        assert!(parse_kv("app=").is_err());
        assert!(is_href("/orgs/5/labels/1"));
        assert!(!is_href("app=web"));
    }

    #[test]
    fn write_output_renders_key_value_rows() {
        let out = WriteOutput::DryRun(WriteRequest {
            method: pce::WriteMethod::Post,
            path: "/orgs/5/labels".into(),
            body: Some(json!({"key": "app"})),
            summary: "create label app=web".into(),
        });
        assert_eq!(out.headers(), vec!["field", "value"]);
        let rows = out.rows();
        assert_eq!(rows[0], vec!["method", "POST"]);
        assert_eq!(rows[1], vec!["path", "/orgs/5/labels"]);
        assert_eq!(rows[2], vec!["summary", "create label app=web"]);
        assert_eq!(rows[3], vec!["body", r#"{"key":"app"}"#]);
        let out = WriteOutput::Skipped { skipped: true, href: "/x".into(), reason: "exists".into() };
        assert_eq!(out.rows()[0], vec!["skipped", "true"]);
        let json = serde_json::to_value(&out).unwrap();
        assert_eq!(json, json!({"skipped": true, "href": "/x", "reason": "exists"}));
    }

    #[test]
    fn missing_yes_is_an_args_error_unless_dry_run() {
        assert!(check_flags(&WriteFlags { dry_run: false, yes: false }).is_err());
        assert!(check_flags(&WriteFlags { dry_run: true, yes: false }).is_ok());
        assert!(check_flags(&WriteFlags { dry_run: false, yes: true }).is_ok());
    }
}
```

`crates/illumio-tui/tests/cli_writes.rs` (new; the helpers are repeated from `tests/cli.rs` because integration tests are separate crates):

```rust
//! Write commands: guard, --dry-run/--yes, envelope, audit, exit codes.
use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::{Value, json};
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TEST_SECRET: &str = "s3cr3t-cli-test-9f2a";

/// A command with credentials but writes neither enabled nor allowed.
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
        .env("API_SECRET", TEST_SECRET)
        .env("XDG_CONFIG_HOME", std::env::temp_dir().join("illumio-tui-no-config"));
    c
}

/// Writes enabled by env, allowed by flag, audit log in `state`.
pub fn writer(server: &MockServer, state: &std::path::Path) -> Command {
    let mut c = cmd(server);
    c.env("ILLUMIO_WRITES", "1")
        .env("XDG_STATE_HOME", state)
        .arg("--allow-writes")
        .arg("--json");
    c
}

pub fn stdout_json(assert: assert_cmd::assert::Assert) -> Value {
    serde_json::from_slice(&assert.get_output().stdout).expect("stdout is JSON")
}

pub fn stderr_json(assert: &assert_cmd::assert::Assert) -> Value {
    serde_json::from_slice(&assert.get_output().stderr).expect("stderr is JSON")
}

pub fn assert_no_secret(assert: &assert_cmd::assert::Assert) {
    let out = assert.get_output();
    for (name, bytes) in [("stdout", &out.stdout), ("stderr", &out.stderr)] {
        let text = String::from_utf8_lossy(bytes);
        assert!(!text.contains(TEST_SECRET), "{name} leaks the secret: {text}");
    }
}

pub fn audit_lines(state: &std::path::Path) -> Vec<Value> {
    let p = state.join("illumio-tui").join("writes.jsonl");
    match std::fs::read_to_string(p) {
        Ok(t) => t.lines().map(|l| serde_json::from_str(l).unwrap()).collect(),
        Err(_) => Vec::new(),
    }
}

/// The four lists behind the name index.
pub async fn mount_index(server: &MockServer, labels: Value, ip_lists: Value, services: Value) {
    for (p, body) in [
        ("/api/v2/orgs/5/labels", labels),
        ("/api/v2/orgs/5/sec_policy/draft/label_groups", json!([])),
        ("/api/v2/orgs/5/sec_policy/draft/ip_lists", ip_lists),
        ("/api/v2/orgs/5/sec_policy/draft/services", services),
    ] {
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(server)
            .await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn health_reports_the_writes_setting() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "normal"})))
        .mount(&server)
        .await;
    let out = stdout_json(cmd(&server).args(["health", "--json"]).assert().success());
    assert_eq!(out["data"]["writes"], false);
    let out = stdout_json(
        cmd(&server)
            .env("ILLUMIO_WRITES", "true")
            .args(["health", "--json"])
            .assert()
            .success(),
    );
    assert_eq!(out["data"]["writes"], true);
    assert!(out["meta"].get("write").is_none(), "reads carry no write meta");
}

#[test]
fn allow_writes_is_a_global_flag_with_help() {
    let out = Command::cargo_bin("illumio-tui")
        .unwrap()
        .env_clear()
        .args(["--help"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("--allow-writes"), "{text}");
}
```

Keep `body_json`, `stderr_json`, `audit_lines`, `writer`, `assert_no_secret` and `mount_index` even though only Task 10 onwards uses them; add `#![allow(dead_code)]` at the top of the file so the gate stays green until then.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p illumio-tui --test cli_writes` and `cargo test -p illumio-tui --lib config`
Expected: compile errors (`writes` fields missing, `mod write` missing).

- [ ] **Step 3: Config**

In `crates/illumio-tui/src/config.rs`:

- `Profile`: after `api_secret_cmd`, add
  ```rust
      /// Allow write commands against this PCE (the CLI also needs --allow-writes).
      pub writes: Option<bool>,
  ```
  and in the manual `Debug`, after `.field("api_secret_cmd", ...)`, add `.field("writes", &self.writes)`.
- `ResolvedConfig`: add `pub writes: bool,` after `traffic_default_window`.
- In `resolve`, in the `Ok(ResolvedConfig { ... })` literal add
  ```rust
          writes: env_str("ILLUMIO_WRITES")
              .and_then(|v| parse_bool(&v))
              .or(profile.writes)
              .unwrap_or(false),
  ```
- In the doc comment at the top of the file, after the precedence sentence, add: "`writes` (profile) or `ILLUMIO_WRITES` (env) enables write commands; nothing is written unless `--allow-writes` is also given."

Check `parse_bool` accepts `1|true|yes|on` and `0|false|no|off` (it exists at ~line 310); if it accepts fewer forms, do not change it; the test uses `1`, `true`, `false`.

- [ ] **Step 4: `NameIndex` exact lookups** (`crates/pce/src/resolve.rs`, in `impl NameIndex` after `ip_list_names`)

```rust
    /// IP lists whose name equals `name` exactly (case-sensitive), by href.
    pub fn ip_lists_named(&self, name: &str) -> Vec<&IpList> {
        let mut v: Vec<&IpList> = self.ip_lists.values().filter(|l| l.name == name).collect();
        v.sort_by(|a, b| a.href.cmp(&b.href));
        v
    }

    /// Services whose name equals `name` exactly (case-sensitive), by href.
    pub fn services_named(&self, name: &str) -> Vec<&Service> {
        let mut v: Vec<&Service> = self.services.values().filter(|s| s.name == name).collect();
        v.sort_by(|a, b| a.href.cmp(&b.href));
        v
    }
```

- [ ] **Step 5: CLI error, output, context, health, profile, global flag**

`crates/illumio-tui/src/cli/error.rs`: add the variant and mappings.

```rust
use pce::write::{WriteError, WriteRefused};

    #[error(transparent)]
    Write(#[from] WriteError),
```

In `exit_code`, before the `CliError::Io` arm, delegate to a helper (the `AuditFailed` variant, added in Task 3's fix round, wraps another `WriteError` and takes its code and kind):

```rust
            CliError::Write(e) => write_exit_code(e),
```

```rust
/// Exit code of a write error; `AuditFailed` reports as the write error it wraps.
fn write_exit_code(e: &WriteError) -> i32 {
    match e {
        WriteError::Refused(_) => 6,
        WriteError::Invalid(_) => 2,
        WriteError::Unanswered(_) => 4,
        WriteError::AuditFailed { write, .. } => write_exit_code(write),
        WriteError::Pce(e) => match e {
            PceError::Auth { .. } => 3,
            PceError::RateLimited { .. } => 5,
            PceError::Invalid(_) => 2,
            _ => 4,
        },
    }
}
```

In `kind`: `CliError::Write(e) => e.kind().into(),`. Update the module doc comment to list `6 write refused`. Add a unit test in `error.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_errors_map_to_exit_codes() {
        assert_eq!(CliError::Write(WriteError::Refused(WriteRefused::FlagMissing)).exit_code(), 6);
        assert_eq!(CliError::Write(WriteError::Refused(WriteRefused::FlagMissing)).kind(), "write_refused");
        assert_eq!(CliError::Write(WriteError::Invalid(vec!["x".into()])).exit_code(), 2);
        assert_eq!(CliError::Write(WriteError::Unanswered(PceError::Cancelled)).exit_code(), 4);
        assert_eq!(CliError::Write(WriteError::Pce(PceError::Auth { status: 401 })).exit_code(), 3);
        assert_eq!(
            CliError::Write(WriteError::Pce(PceError::Rejected { status: 406, body: "x".into() })).exit_code(),
            4
        );
        let wrapped = CliError::Write(WriteError::AuditFailed {
            write: Box::new(WriteError::Refused(WriteRefused::FlagMissing)),
            error: "disk full".into(),
        });
        assert_eq!(wrapped.exit_code(), 6);
        assert_eq!(wrapped.kind(), "write_refused");
        assert!(wrapped.to_string().contains("audit"));
    }
}
```

`crates/illumio-tui/src/cli/output.rs`:

```rust
use pce::WriteMethod;

/// What a write command did, for `meta.write`.
#[derive(Debug, Clone, Serialize)]
pub struct WriteMeta {
    pub method: WriteMethod,
    pub path: String,
    pub status: u16,
    /// Audit log location.
    pub audit: String,
}
```

Add to `Meta`: `#[serde(skip_serializing_if = "Option::is_none")] pub write: Option<WriteMeta>,`. In `meta_line`, after the truncated check:

```rust
    if let Some(w) = &meta.write {
        s.push_str(&format!(" · {} {} → {}", w.method.as_str(), w.path, w.status));
    }
```

`crates/illumio-tui/src/cli/profile.rs`: add `write: None,` to the `Meta` literal.

`crates/illumio-tui/src/cli/context.rs`: add `pub allow_writes: bool,` to `Context`, set it from `global.allow_writes` in `new`, add `write: None` to `meta()`, and add:

```rust
    /// Envelope for a write command: `cache` is always `none`.
    pub fn emit_write(
        &self,
        data: &super::write::WriteOutput,
        write: Option<WriteMeta>,
    ) -> Result<(), CliError> {
        let mut meta = self.meta("none", false, None);
        meta.write = write;
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
```

(import `WriteMeta` from `super::output`).

`crates/illumio-tui/src/cli/health.rs`: add `pub writes: bool,` to `HealthReport`, a row `vec!["writes".into(), self.writes.to_string()]`, and `writes: ctx.config.writes` in `run`.

`crates/illumio-tui/src/cli/mod.rs`: add `pub mod write;` and, in `GlobalArgs` after `no_cache`:

```rust
    /// Allow write commands; the profile must also set writes = true (or ILLUMIO_WRITES=1)
    #[arg(long, global = true)]
    pub allow_writes: bool,
```

- [ ] **Step 6: Create `crates/illumio-tui/src/cli/write.rs`**

```rust
//! What every write command shares: the guard (profile + --allow-writes +
//! audit log), the --dry-run / --yes contract, exact name resolution, and
//! the envelope data. Commands build a `Plan` with the library and hand it
//! to `run_plan`.
use super::context::Context;
use super::error::CliError;
use super::output::{TableRows, WriteMeta};
use clap::Args;
use pce::write::{
    JsonlAudit, Outcome, Plan, WriteContext, WriteError, WriteGuard, WriteRequest, Written,
};
use serde::Serialize;

#[derive(Args, Debug, Clone, Default)]
pub struct WriteFlags {
    /// Print the request that would be sent, send nothing
    #[arg(long)]
    pub dry_run: bool,
    /// Send the request (required unless --dry-run)
    #[arg(long)]
    pub yes: bool,
}

pub fn check_flags(flags: &WriteFlags) -> Result<(), CliError> {
    if !flags.dry_run && !flags.yes {
        return Err(CliError::Args(
            "this command changes the PCE; re-run with --yes to send it, or --dry-run to see the request"
                .into(),
        ));
    }
    Ok(())
}

fn write_guard(ctx: &Context) -> Result<WriteGuard, CliError> {
    let audit = JsonlAudit::new(JsonlAudit::default_path());
    let guard = WriteGuard::new(
        ctx.config.writes,
        ctx.allow_writes,
        Box::new(audit),
        WriteContext {
            profile: ctx.config.profile_name.clone(),
            host: ctx.config.host.clone(),
            org_id: ctx.config.org_id,
            source: "cli".into(),
        },
    )
    .map_err(WriteError::from)?;
    Ok(guard)
}

/// Guard first, then the flag check, before any request: an agent learns
/// about a missing `--allow-writes` (exit 6) before a missing `--yes` (exit 2).
pub fn prepare(ctx: &Context, flags: &WriteFlags) -> Result<WriteGuard, CliError> {
    let guard = write_guard(ctx)?;
    check_flags(flags)?;
    Ok(guard)
}

/// The `data` of a write command's envelope.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum WriteOutput {
    DryRun(WriteRequest),
    Written(Written),
    Skipped {
        skipped: bool,
        href: String,
        reason: String,
    },
}

fn compact(v: &serde_json::Value) -> String {
    serde_json::to_string(v).unwrap_or_default()
}

impl TableRows for WriteOutput {
    fn headers(&self) -> Vec<&'static str> {
        vec!["field", "value"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        let row = |k: &str, v: String| vec![k.to_string(), v];
        match self {
            WriteOutput::DryRun(r) => vec![
                row("method", r.method.as_str().into()),
                row("path", r.path.clone()),
                row("summary", r.summary.clone()),
                row("body", r.body.as_ref().map(compact).unwrap_or_default()),
            ],
            WriteOutput::Written(w) => vec![
                row("href", w.href.clone()),
                row("pending", w.pending.to_string()),
                row("status", w.status.to_string()),
                row("dropped_fields", w.dropped_fields.join(", ")),
                row("warnings", w.warnings.join("; ")),
                row("object", compact(&w.object)),
            ],
            WriteOutput::Skipped {
                skipped,
                href,
                reason,
            } => vec![
                row("skipped", skipped.to_string()),
                row("href", href.clone()),
                row("reason", reason.clone()),
            ],
        }
    }
}

/// Dry run prints the request; otherwise the plan is executed once and the
/// result printed with `meta.write`.
pub async fn run_plan(
    ctx: &Context,
    flags: &WriteFlags,
    guard: &WriteGuard,
    plan: Plan,
) -> Result<(), CliError> {
    let (request, kind) = match plan {
        Plan::Noop {
            existing_href,
            reason,
        } => {
            return ctx.emit_write(
                &WriteOutput::Skipped {
                    skipped: true,
                    href: existing_href,
                    reason,
                },
                None,
            );
        }
        Plan::Send { request, kind } => (request, kind),
    };
    if flags.dry_run {
        return ctx.emit_write(&WriteOutput::DryRun(request), None);
    }
    let method = request.method;
    let path = request.path.clone();
    match ctx
        .pce
        .writer(guard)
        .execute(Plan::Send { request, kind })
        .await?
    {
        Outcome::Written(w) => {
            let meta = WriteMeta {
                method,
                path,
                status: w.status,
                audit: guard.audit_location(),
            };
            ctx.emit_write(&WriteOutput::Written(w), Some(meta))
        }
        Outcome::Skipped {
            existing_href,
            reason,
        } => ctx.emit_write(
            &WriteOutput::Skipped {
                skipped: true,
                href: existing_href,
                reason,
            },
            None,
        ),
    }
}

/// `key=value`, split on the first `=`; both sides trimmed and non-empty.
pub fn parse_kv(spec: &str) -> Result<(String, String), CliError> {
    let (k, v) = spec
        .split_once('=')
        .ok_or_else(|| CliError::Args(format!("expected key=value, got {spec:?}")))?;
    let (k, v) = (k.trim(), v.trim());
    if k.is_empty() || v.is_empty() {
        return Err(CliError::Args(format!(
            "expected key=value with both sides non-empty, got {spec:?}"
        )));
    }
    Ok((k.to_string(), v.to_string()))
}

pub fn is_href(s: &str) -> bool {
    s.trim_start().starts_with("/orgs/")
}

/// An href as given, or `key=value` resolved exactly through the name index.
pub async fn resolve_label(ctx: &Context, spec: &str) -> Result<String, CliError> {
    if is_href(spec) {
        return Ok(spec.trim().to_string());
    }
    let (k, v) = parse_kv(spec)?;
    let idx = ctx.pce.name_index(ctx.cache_mode).await?;
    idx.label_href(&k, &v).map(str::to_string).ok_or_else(|| {
        let known = idx.values_for_key(&k);
        CliError::Args(if known.is_empty() {
            format!("unknown label {k}={v}; no labels have key {k}")
        } else {
            format!("unknown label {k}={v}; known values for {k}: {}", known.join(", "))
        })
    })
}

pub async fn resolve_labels(ctx: &Context, specs: &[String]) -> Result<Vec<String>, CliError> {
    let mut out = Vec::with_capacity(specs.len());
    for s in specs {
        out.push(resolve_label(ctx, s).await?);
    }
    Ok(out)
}

fn pick_one(kind: &str, name: &str, hrefs: Vec<&str>) -> Result<String, CliError> {
    match hrefs.len() {
        0 => Err(CliError::Args(format!("no {kind} named {name:?}"))),
        1 => Ok(hrefs[0].to_string()),
        _ => Err(CliError::Args(format!(
            "{name:?} names several {kind}s: {}; use the href",
            hrefs.join(", ")
        ))),
    }
}

pub async fn resolve_ip_list(ctx: &Context, name: &str) -> Result<String, CliError> {
    if is_href(name) {
        return Ok(name.trim().to_string());
    }
    let idx = ctx.pce.name_index(ctx.cache_mode).await?;
    let hrefs: Vec<&str> = idx.ip_lists_named(name.trim()).iter().map(|l| l.href.as_str()).collect();
    pick_one("IP list", name.trim(), hrefs)
}

pub async fn resolve_service(ctx: &Context, name: &str) -> Result<String, CliError> {
    if is_href(name) {
        return Ok(name.trim().to_string());
    }
    let idx = ctx.pce.name_index(ctx.cache_mode).await?;
    let hrefs: Vec<&str> = idx.services_named(name.trim()).iter().map(|s| s.href.as_str()).collect();
    pick_one("service", name.trim(), hrefs)
}
```

Then paste the unit tests from Step 1 at the bottom.

- [ ] **Step 7: Run everything**

Run: `cargo test -p pce && cargo test -p illumio-tui`
Expected: all green, including the existing `tests/cli.rs` (the `health_json_envelope` test still passes; the new `writes` key is additive).

- [ ] **Step 8: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add crates/pce/src/resolve.rs crates/illumio-tui/src crates/illumio-tui/tests/cli_writes.rs
git commit -m "feat(cli): writes setting, --allow-writes, exit code 6, write envelope and shared write helpers

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 10: `labels add|update|delete`

**Files:**
- Modify: `crates/illumio-tui/src/cli/objects.rs`
- Modify: `crates/illumio-tui/src/cli/mod.rs` (`Labels(objects::LabelsCmd)`, dispatch)
- Modify: `crates/illumio-tui/tests/cli.rs` (`every_flag_has_help_text` list)
- Test: `crates/illumio-tui/tests/cli_writes.rs` (append)

**Interfaces:**
- Consumes: `prepare`, `run_plan`, `parse_kv`, `resolve_label`, `WriteFlags`, `LabelDraft`, `IfMissing`, `Writer::plan_create_label / plan_update_label / plan_delete_label`.
- Produces: `objects::LabelsCmd { action: Option<LabelsAction>, list: LabelsArgs }`, `objects::labels_cmd(ctx, &LabelsCmd)`.

- [ ] **Step 1: Write the failing tests** (append to `tests/cli_writes.rs`)

```rust
mod labels {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn add_is_refused_without_allow_writes_or_profile_setting() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        // Flag present, profile denies.
        let a = cmd(&server)
            .env("XDG_STATE_HOME", state.path())
            .args(["--allow-writes", "--json", "labels", "add", "app=web", "--yes"])
            .assert()
            .code(6);
        let err = stderr_json(&a);
        assert_eq!(err["error"]["kind"], "write_refused");
        assert_eq!(err["error"]["exit_code"], 6);
        assert!(err["error"]["message"].as_str().unwrap().contains("writes = true"));
        // Profile allows, flag missing.
        let a = cmd(&server)
            .env("ILLUMIO_WRITES", "1")
            .env("XDG_STATE_HOME", state.path())
            .args(["--json", "labels", "add", "app=web", "--yes"])
            .assert()
            .code(6);
        assert!(stderr_json(&a)["error"]["message"].as_str().unwrap().contains("--allow-writes"));
        // Even a dry run needs the guard.
        cmd(&server)
            .args(["--json", "labels", "add", "app=web", "--dry-run"])
            .assert()
            .code(6);
        assert!(server.received_requests().await.unwrap().is_empty());
        assert!(audit_lines(state.path()).is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn add_without_yes_is_exit_2_and_sends_nothing() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        let a = writer(&server, state.path())
            .args(["labels", "add", "app=web"])
            .assert()
            .code(2);
        let err = stderr_json(&a);
        assert_eq!(err["error"]["kind"], "args");
        assert!(err["error"]["message"].as_str().unwrap().contains("--yes"));
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn add_dry_run_prints_the_request_and_sends_nothing() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        let a = writer(&server, state.path())
            .args(["labels", "add", "app=web", "--dry-run"])
            .assert()
            .success();
        assert_no_secret(&a);
        let out = stdout_json(a);
        assert_eq!(out["data"]["method"], "POST");
        assert_eq!(out["data"]["path"], "/orgs/5/labels");
        assert_eq!(out["data"]["body"], json!({"key": "app", "value": "web"}));
        assert_eq!(out["data"]["summary"], "create label app=web");
        assert_eq!(out["meta"]["cache"], "none");
        assert_eq!(out["meta"]["api_calls"], 0);
        assert!(out["meta"].get("write").is_none());
        assert!(server.received_requests().await.unwrap().is_empty());
        assert!(audit_lines(state.path()).is_empty(), "dry runs are not audited");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn label_spec_splits_on_the_first_equals() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        let out = stdout_json(
            writer(&server, state.path())
                .args(["labels", "add", "app=a=b", "--dry-run"])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["body"], json!({"key": "app", "value": "a=b"}));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn add_sends_once_and_audits() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        Mock::given(method("POST"))
            .and(path("/api/v2/orgs/5/labels"))
            .and(body_json(json!({"key": "app", "value": "web"})))
            .respond_with(ResponseTemplate::new(201).set_body_json(
                json!({"href": "/orgs/5/labels/9", "key": "app", "value": "web"}),
            ))
            .expect(1)
            .mount(&server)
            .await;
        let a = writer(&server, state.path())
            .args(["labels", "add", "app=web", "--yes"])
            .assert()
            .success();
        assert_no_secret(&a);
        let out = stdout_json(a);
        assert_eq!(out["data"]["href"], "/orgs/5/labels/9");
        assert_eq!(out["data"]["pending"], false);
        assert_eq!(out["data"]["status"], 201);
        assert_eq!(out["data"]["object"]["value"], "web");
        assert_eq!(out["data"]["dropped_fields"], json!([]));
        assert_eq!(out["meta"]["write"]["method"], "POST");
        assert_eq!(out["meta"]["write"]["path"], "/orgs/5/labels");
        assert_eq!(out["meta"]["write"]["status"], 201);
        assert!(out["meta"]["write"]["audit"].as_str().unwrap().ends_with("writes.jsonl"));
        assert_eq!(out["meta"]["api_calls"], 1);
        let lines = audit_lines(state.path());
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0]["method"], "POST");
        assert_eq!(lines[0]["status"], 201);
        assert_eq!(lines[0]["href"], "/orgs/5/labels/9");
        assert_eq!(lines[0]["source"], "cli");
        assert_eq!(lines[0]["org_id"], 5);
        assert!(!serde_json::to_string(&lines[0]).unwrap().contains(TEST_SECRET));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn add_if_missing_skips_an_exact_match() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/labels"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!([{"href": "/orgs/5/labels/1", "key": "app", "value": "web"}]),
            ))
            .expect(1)
            .mount(&server)
            .await;
        let out = stdout_json(
            writer(&server, state.path())
                .args(["labels", "add", "app=web", "--if-missing", "--yes"])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["skipped"], true);
        assert_eq!(out["data"]["href"], "/orgs/5/labels/1");
        assert!(out["meta"].get("write").is_none());
        assert!(audit_lines(state.path()).is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rejected_write_is_exit_4_with_the_pce_message() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        Mock::given(method("POST"))
            .and(path("/api/v2/orgs/5/labels"))
            .respond_with(ResponseTemplate::new(406).set_body_json(
                json!([{"token": "label_key_value_must_be_unique", "message": "exists"}]),
            ))
            .expect(1)
            .mount(&server)
            .await;
        let a = writer(&server, state.path())
            .args(["labels", "add", "app=web", "--yes"])
            .assert()
            .code(4);
        let err = stderr_json(&a);
        assert_eq!(err["error"]["kind"], "rejected");
        assert!(err["error"]["message"].as_str().unwrap().contains("label_key_value_must_be_unique"));
        assert_eq!(audit_lines(state.path())[0]["status"], 406);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn update_resolves_key_value_and_refetches_after_204() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_index(
            &server,
            json!([{"href": "/orgs/5/labels/1", "key": "env", "value": "test"}]),
            json!([]),
            json!([]),
        )
        .await;
        Mock::given(method("PUT"))
            .and(path("/api/v2/orgs/5/labels/1"))
            .and(body_json(json!({"value": "prod"})))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/labels/1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"href": "/orgs/5/labels/1", "key": "env", "value": "prod"}),
            ))
            .expect(1)
            .mount(&server)
            .await;
        let out = stdout_json(
            writer(&server, state.path())
                .args(["labels", "update", "env=test", "--value", "prod", "--yes"])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["href"], "/orgs/5/labels/1");
        assert_eq!(out["data"]["status"], 204);
        assert_eq!(out["data"]["object"]["value"], "prod");
        // Unknown label: exit 2 listing known values, nothing written.
        let a = writer(&server, state.path())
            .args(["labels", "update", "env=nope", "--value", "x", "--yes"])
            .assert()
            .code(2);
        assert!(stderr_json(&a)["error"]["message"].as_str().unwrap().contains("test"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn delete_by_href_and_table_output() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        Mock::given(method("DELETE"))
            .and(path("/api/v2/orgs/5/labels/9"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        let mut c = writer(&server, state.path());
        c.args(["labels", "delete", "/orgs/5/labels/9", "--yes"]);
        let out = stdout_json(c.assert().success());
        assert_eq!(out["data"]["href"], "/orgs/5/labels/9");
        assert!(out["data"]["object"].is_null());
        // Table form of a dry run (built from cmd(): writer() already passes --json).
        let mut t = cmd(&server);
        t.env("ILLUMIO_WRITES", "1").env("XDG_STATE_HOME", state.path());
        t.args(["--allow-writes", "--table", "labels", "delete", "/orgs/5/labels/9", "--dry-run"])
            .assert()
            .success()
            .stdout(predicate::str::contains("method").and(predicate::str::contains("DELETE")))
            .stderr(predicate::str::contains("cache none"));
        assert_eq!(audit_lines(state.path()).len(), 1);
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p illumio-tui --test cli_writes labels`
Expected: failures (`add` is not a subcommand yet).

- [ ] **Step 3: Implement the subcommands in `crates/illumio-tui/src/cli/objects.rs`**

Add imports:

```rust
use super::write::{self, WriteFlags, parse_kv, resolve_label, run_plan};
use clap::Subcommand;
use pce::write::{IfMissing, LabelDraft};
```

Add after `LabelsArgs`:

```rust
/// `labels` lists by default; `add`, `update` and `delete` write.
#[derive(Args, Debug)]
#[command(args_conflicts_with_subcommands = true)]
pub struct LabelsCmd {
    #[command(subcommand)]
    pub action: Option<LabelsAction>,
    #[command(flatten)]
    pub list: LabelsArgs,
}

#[derive(Subcommand, Debug)]
pub enum LabelsAction {
    /// Create a label (needs writes enabled and --allow-writes)
    Add(LabelAddArgs),
    /// Change a label's value
    Update(LabelUpdateArgs),
    /// Delete a label
    Delete(LabelDeleteArgs),
}

#[derive(Args, Debug)]
pub struct LabelAddArgs {
    /// key=value
    pub label: String,
    /// Do nothing when a label with exactly this key and value exists
    #[arg(long)]
    pub if_missing: bool,
    #[command(flatten)]
    pub write: WriteFlags,
}

#[derive(Args, Debug)]
pub struct LabelUpdateArgs {
    /// Label href or key=value
    pub label: String,
    /// New value
    #[arg(long)]
    pub value: String,
    #[command(flatten)]
    pub write: WriteFlags,
}

#[derive(Args, Debug)]
pub struct LabelDeleteArgs {
    /// Label href or key=value
    pub label: String,
    #[command(flatten)]
    pub write: WriteFlags,
}

pub async fn labels_cmd(ctx: &Context, cmd: &LabelsCmd) -> Result<(), CliError> {
    match &cmd.action {
        None => labels(ctx, &cmd.list).await,
        Some(LabelsAction::Add(a)) => label_add(ctx, a).await,
        Some(LabelsAction::Update(a)) => label_update(ctx, a).await,
        Some(LabelsAction::Delete(a)) => label_delete(ctx, a).await,
    }
}

async fn label_add(ctx: &Context, a: &LabelAddArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let (k, v) = parse_kv(&a.label)?;
    let draft = LabelDraft::new(k, v);
    let if_missing = if a.if_missing {
        IfMissing::SkipExisting
    } else {
        IfMissing::Create
    };
    let plan = ctx
        .pce
        .writer(&guard)
        .plan_create_label(&draft, if_missing)
        .await?;
    run_plan(ctx, &a.write, &guard, plan).await
}

async fn label_update(ctx: &Context, a: &LabelUpdateArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let href = resolve_label(ctx, &a.label).await?;
    let plan = ctx.pce.writer(&guard).plan_update_label(&href, &a.value)?;
    run_plan(ctx, &a.write, &guard, plan).await
}

async fn label_delete(ctx: &Context, a: &LabelDeleteArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let href = resolve_label(ctx, &a.label).await?;
    let plan = ctx.pce.writer(&guard).plan_delete_label(&href)?;
    run_plan(ctx, &a.write, &guard, plan).await
}
```

In `cli/mod.rs`: `Labels(objects::LabelsCmd)` with doc `/// List labels; add, update or delete one`, and dispatch `Command::Labels(c) => objects::labels_cmd(&ctx, &c).await`.

In `tests/cli.rs` `every_flag_has_help_text`, add `vec!["labels", "add", "--help"]`, `vec!["labels", "update", "--help"]`, `vec!["labels", "delete", "--help"]`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p illumio-tui`
Expected: all pass, including the existing `labels --key` listing tests in `tests/cli.rs`.

- [ ] **Step 5: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add crates/illumio-tui/src/cli crates/illumio-tui/tests
git commit -m "feat(cli): labels add/update/delete with --dry-run, --yes, --if-missing and audit

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 11: `iplists add|update|delete`

**Files:**
- Modify: `crates/illumio-tui/src/cli/objects.rs`
- Modify: `crates/illumio-tui/src/cli/mod.rs` (`Iplists(objects::IplistsCmd)`, dispatch)
- Modify: `crates/illumio-tui/tests/cli.rs` (`every_flag_has_help_text` list)
- Test: `crates/illumio-tui/tests/cli_writes.rs` (append)

**Interfaces:**
- Consumes: `prepare`, `run_plan`, `resolve_ip_list`, `WriteFlags`, `IpListDraft`, `IpListPatch`, `IpRangeSpec`, `IfMissing`, `Writer::plan_create_ip_list / plan_update_ip_list / plan_delete_ip_list`.
- Produces: `objects::IplistsCmd { action: Option<IplistsAction> }`, `objects::ip_lists_cmd(ctx, &IplistsCmd)`.

- [ ] **Step 1: Write the failing tests** (append to `tests/cli_writes.rs`)

```rust
mod iplists {
    use super::*;

    fn corp() -> Value {
        json!({
            "href": "/orgs/5/sec_policy/draft/ip_lists/3", "name": "Corp",
            "ip_ranges": [{"from_ip": "10.0.0.0/8"}], "fqdns": [{"fqdn": "*.corp.example"}]
        })
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn add_dry_run_body() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        let out = stdout_json(
            writer(&server, state.path())
                .args([
                    "iplists", "add", "Corp",
                    "--range", "10.0.0.0/8", "--range", "192.168.1.1-192.168.1.9",
                    "--exclude", "10.0.5.0/24", "--fqdn", "*.corp.example",
                    "--description", "corp nets", "--dry-run",
                ])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["method"], "POST");
        assert_eq!(out["data"]["path"], "/orgs/5/sec_policy/draft/ip_lists");
        assert_eq!(
            out["data"]["body"],
            json!({
                "name": "Corp", "description": "corp nets",
                "ip_ranges": [
                    {"from_ip": "10.0.0.0/8"},
                    {"from_ip": "192.168.1.1", "to_ip": "192.168.1.9"},
                    {"from_ip": "10.0.5.0/24", "exclusion": true}
                ],
                "fqdns": [{"fqdn": "*.corp.example"}]
            })
        );
        // A bad range is exit 2 before anything else.
        let a = writer(&server, state.path())
            .args(["iplists", "add", "Corp", "--range", "10.0.0.0/99", "--dry-run"])
            .assert()
            .code(2);
        assert!(stderr_json(&a)["error"]["message"].as_str().unwrap().contains("10.0.0.0/99"));
        // Neither a range nor an FQDN is exit 2.
        writer(&server, state.path())
            .args(["iplists", "add", "Empty", "--dry-run"])
            .assert()
            .code(2);
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn add_sends_and_marks_pending() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        Mock::given(method("POST"))
            .and(path("/api/v2/orgs/5/sec_policy/draft/ip_lists"))
            .respond_with(ResponseTemplate::new(201).set_body_json(corp()))
            .expect(1)
            .mount(&server)
            .await;
        let out = stdout_json(
            writer(&server, state.path())
                .args(["iplists", "add", "Corp", "--range", "10.0.0.0/8", "--fqdn", "*.corp.example", "--yes"])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["pending"], true);
        assert_eq!(out["data"]["href"], "/orgs/5/sec_policy/draft/ip_lists/3");
        assert_eq!(audit_lines(state.path()).len(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn update_by_name_puts_the_full_list() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_index(&server, json!([]), json!([corp()]), json!([])).await;
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/sec_policy/draft/ip_lists/3"))
            .respond_with(ResponseTemplate::new(200).set_body_json(corp()))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/api/v2/orgs/5/sec_policy/draft/ip_lists/3"))
            .and(body_json(json!({
                "name": "Corp",
                "ip_ranges": [{"from_ip": "10.0.0.0/8"}, {"from_ip": "10.2.0.0/16"}],
                "fqdns": []
            })))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        let out = stdout_json(
            writer(&server, state.path())
                .args([
                    "iplists", "update", "Corp",
                    "--add-range", "10.2.0.0/16", "--remove-fqdn", "*.corp.example", "--yes",
                ])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["status"], 204);
        assert_eq!(out["data"]["pending"], true);
        assert_eq!(out["meta"]["write"]["method"], "PUT");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn removing_the_last_range_is_refused() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        let mut only_range = corp();
        only_range["fqdns"] = json!([]);
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/sec_policy/draft/ip_lists/3"))
            .respond_with(ResponseTemplate::new(200).set_body_json(only_range))
            .mount(&server)
            .await;
        let a = writer(&server, state.path())
            .args([
                "iplists", "update", "/orgs/5/sec_policy/draft/ip_lists/3",
                "--remove-range", "10.0.0.0/8", "--yes",
            ])
            .assert()
            .code(2);
        assert_eq!(stderr_json(&a)["error"]["kind"], "invalid");
        let sent = server.received_requests().await.unwrap();
        assert!(sent.iter().all(|r| r.method.as_str() == "GET"), "nothing was written");
        assert!(audit_lines(state.path()).is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ambiguous_name_lists_candidates_and_delete_by_href_works() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        let mut dup = corp();
        dup["href"] = json!("/orgs/5/sec_policy/draft/ip_lists/4");
        mount_index(&server, json!([]), json!([corp(), dup]), json!([])).await;
        let a = writer(&server, state.path())
            .args(["iplists", "delete", "Corp", "--yes"])
            .assert()
            .code(2);
        let msg = stderr_json(&a)["error"]["message"].as_str().unwrap().to_string();
        assert!(msg.contains("ip_lists/3") && msg.contains("ip_lists/4"), "{msg}");
        Mock::given(method("DELETE"))
            .and(path("/api/v2/orgs/5/sec_policy/draft/ip_lists/4"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        writer(&server, state.path())
            .args(["iplists", "delete", "/orgs/5/sec_policy/active/ip_lists/4", "--yes"])
            .assert()
            .success();
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p illumio-tui --test cli_writes iplists`
Expected: failures.

- [ ] **Step 3: Implement in `crates/illumio-tui/src/cli/objects.rs`**

Imports to add: `use super::write::resolve_ip_list; use pce::write::{IpListDraft, IpListPatch, IpRangeSpec};`

```rust
#[derive(Args, Debug)]
pub struct IplistsCmd {
    #[command(subcommand)]
    pub action: Option<IplistsAction>,
}

#[derive(Subcommand, Debug)]
pub enum IplistsAction {
    /// Create an IP list in draft (needs writes enabled and --allow-writes)
    Add(IpListAddArgs),
    /// Add or remove ranges and FQDNs; the full list is re-sent
    Update(IpListUpdateArgs),
    /// Delete an IP list from draft
    Delete(IpListDeleteArgs),
}

#[derive(Args, Debug)]
pub struct IpListAddArgs {
    /// IP list name
    pub name: String,
    /// Range: A, A-B or A/N (repeatable)
    #[arg(long = "range")]
    pub ranges: Vec<String>,
    /// Excluded range, same forms (repeatable)
    #[arg(long = "exclude")]
    pub excludes: Vec<String>,
    /// FQDN, wildcards allowed (repeatable)
    #[arg(long = "fqdn")]
    pub fqdns: Vec<String>,
    /// Description
    #[arg(long)]
    pub description: Option<String>,
    /// Do nothing when an IP list with exactly this name exists
    #[arg(long)]
    pub if_missing: bool,
    #[command(flatten)]
    pub write: WriteFlags,
}

#[derive(Args, Debug)]
pub struct IpListUpdateArgs {
    /// IP list name or href
    pub target: String,
    /// New name
    #[arg(long)]
    pub name: Option<String>,
    /// New description
    #[arg(long)]
    pub description: Option<String>,
    /// Range to add (repeatable)
    #[arg(long = "add-range")]
    pub add_ranges: Vec<String>,
    /// Range to remove; must match an existing range exactly (repeatable)
    #[arg(long = "remove-range")]
    pub remove_ranges: Vec<String>,
    /// Excluded range to add (repeatable)
    #[arg(long = "add-exclude")]
    pub add_excludes: Vec<String>,
    /// Excluded range to remove (repeatable)
    #[arg(long = "remove-exclude")]
    pub remove_excludes: Vec<String>,
    /// FQDN to add (repeatable)
    #[arg(long = "add-fqdn")]
    pub add_fqdns: Vec<String>,
    /// FQDN to remove (repeatable)
    #[arg(long = "remove-fqdn")]
    pub remove_fqdns: Vec<String>,
    #[command(flatten)]
    pub write: WriteFlags,
}

#[derive(Args, Debug)]
pub struct IpListDeleteArgs {
    /// IP list name or href
    pub target: String,
    #[command(flatten)]
    pub write: WriteFlags,
}

fn parse_ranges(specs: &[String], exclusion: bool) -> Result<Vec<IpRangeSpec>, CliError> {
    specs
        .iter()
        .map(|s| IpRangeSpec::parse(s, exclusion).map_err(CliError::Args))
        .collect()
}

pub async fn ip_lists_cmd(ctx: &Context, cmd: &IplistsCmd) -> Result<(), CliError> {
    match &cmd.action {
        None => ip_lists(ctx).await,
        Some(IplistsAction::Add(a)) => ip_list_add(ctx, a).await,
        Some(IplistsAction::Update(a)) => ip_list_update(ctx, a).await,
        Some(IplistsAction::Delete(a)) => ip_list_delete(ctx, a).await,
    }
}

async fn ip_list_add(ctx: &Context, a: &IpListAddArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let mut ranges = parse_ranges(&a.ranges, false)?;
    ranges.extend(parse_ranges(&a.excludes, true)?);
    let draft = IpListDraft {
        name: a.name.clone(),
        description: a.description.clone(),
        ranges,
        fqdns: a.fqdns.clone(),
    };
    let if_missing = if a.if_missing {
        IfMissing::SkipExisting
    } else {
        IfMissing::Create
    };
    let plan = ctx
        .pce
        .writer(&guard)
        .plan_create_ip_list(&draft, if_missing)
        .await?;
    run_plan(ctx, &a.write, &guard, plan).await
}

async fn ip_list_update(ctx: &Context, a: &IpListUpdateArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let mut add_ranges = parse_ranges(&a.add_ranges, false)?;
    add_ranges.extend(parse_ranges(&a.add_excludes, true)?);
    let mut remove_ranges = parse_ranges(&a.remove_ranges, false)?;
    remove_ranges.extend(parse_ranges(&a.remove_excludes, true)?);
    let patch = IpListPatch {
        name: a.name.clone(),
        description: a.description.clone(),
        add_ranges,
        remove_ranges,
        add_fqdns: a.add_fqdns.clone(),
        remove_fqdns: a.remove_fqdns.clone(),
    };
    let href = resolve_ip_list(ctx, &a.target).await?;
    let plan = ctx
        .pce
        .writer(&guard)
        .plan_update_ip_list(&href, &patch)
        .await?;
    run_plan(ctx, &a.write, &guard, plan).await
}

async fn ip_list_delete(ctx: &Context, a: &IpListDeleteArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let href = resolve_ip_list(ctx, &a.target).await?;
    let plan = ctx.pce.writer(&guard).plan_delete_ip_list(&href)?;
    run_plan(ctx, &a.write, &guard, plan).await
}
```

In `cli/mod.rs`: `Iplists(objects::IplistsCmd)` with doc `/// List IP lists; add, update or delete one`, dispatch `Command::Iplists(c) => objects::ip_lists_cmd(&ctx, &c).await`. Add the three `iplists ... --help` entries to `every_flag_has_help_text`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p illumio-tui`
Expected: all pass (the bare `iplists` listing test in `tests/cli.rs` still passes).

- [ ] **Step 5: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add crates/illumio-tui/src/cli crates/illumio-tui/tests
git commit -m "feat(cli): iplists add/update/delete with full-list updates computed locally

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 12: `services add|update|delete`

**Files:**
- Modify: `crates/illumio-tui/src/cli/objects.rs`
- Modify: `crates/illumio-tui/src/cli/mod.rs` (`Services(objects::ServicesCmd)`, dispatch)
- Modify: `crates/illumio-tui/tests/cli.rs` (`every_flag_has_help_text` list)
- Test: `crates/illumio-tui/tests/cli_writes.rs` (append)

**Interfaces:**
- Consumes: `prepare`, `run_plan`, `resolve_service`, `WriteFlags`, `ServiceDraft`, `ServicePatch`, `PortSpec`, `IfMissing`, `Writer::plan_create_service / plan_update_service / plan_delete_service`.
- Produces: `objects::ServicesCmd { action: Option<ServicesAction> }`, `objects::services_cmd(ctx, &ServicesCmd)`.

- [ ] **Step 1: Write the failing tests** (append to `tests/cli_writes.rs`)

```rust
mod services {
    use super::*;

    fn pg() -> Value {
        json!({"href": "/orgs/5/sec_policy/draft/services/7", "name": "PostgreSQL",
               "service_ports": [{"port": 5432, "proto": 6}]})
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn add_dry_run_body_has_only_service_ports() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        let out = stdout_json(
            writer(&server, state.path())
                .args([
                    "services", "add", "PostgreSQL", "--port", "5432/tcp", "--port", "8000-8100/udp",
                    "--port", "icmp", "--description", "db", "--dry-run",
                ])
                .assert()
                .success(),
        );
        assert_eq!(
            out["data"]["body"],
            json!({"name": "PostgreSQL", "description": "db",
                   "service_ports": [{"port": 5432, "proto": 6}, {"port": 8000, "to_port": 8100, "proto": 17}, {"proto": 1}]})
        );
        assert!(out["data"]["body"].get("windows_services").is_none());
        let a = writer(&server, state.path())
            .args(["services", "add", "X", "--port", "443", "--dry-run"])
            .assert()
            .code(2);
        assert!(stderr_json(&a)["error"]["message"].as_str().unwrap().contains("443"));
        // --port is required.
        writer(&server, state.path())
            .args(["services", "add", "X", "--dry-run"])
            .assert()
            .code(2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn add_reports_dropped_fields() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        let mut dropped = pg();
        dropped["service_ports"] = json!([]);
        Mock::given(method("POST"))
            .and(path("/api/v2/orgs/5/sec_policy/draft/services"))
            .respond_with(ResponseTemplate::new(201).set_body_json(dropped))
            .expect(1)
            .mount(&server)
            .await;
        let out = stdout_json(
            writer(&server, state.path())
                .args(["services", "add", "PostgreSQL", "--port", "5432/tcp", "--yes"])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["dropped_fields"], json!(["service_ports"]));
        assert_eq!(out["data"]["pending"], true);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn update_by_name_and_delete() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_index(&server, json!([]), json!([]), json!([pg()])).await;
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/sec_policy/draft/services/7"))
            .respond_with(ResponseTemplate::new(200).set_body_json(pg()))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/api/v2/orgs/5/sec_policy/draft/services/7"))
            .and(body_json(json!({"name": "PostgreSQL",
                 "service_ports": [{"port": 5432, "proto": 6}, {"port": 5433, "proto": 6}]})))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        let out = stdout_json(
            writer(&server, state.path())
                .args(["services", "update", "PostgreSQL", "--add-port", "5433/tcp", "--yes"])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["status"], 204);
        let a = writer(&server, state.path())
            .args(["services", "update", "PostgreSQL", "--remove-port", "5432/tcp", "--yes"])
            .assert()
            .code(2);
        assert!(stderr_json(&a)["error"]["message"].as_str().unwrap().contains("at least one"));
        Mock::given(method("DELETE"))
            .and(path("/api/v2/orgs/5/sec_policy/draft/services/7"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        writer(&server, state.path())
            .args(["services", "delete", "PostgreSQL", "--yes"])
            .assert()
            .success();
        assert_eq!(audit_lines(state.path()).len(), 2);
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p illumio-tui --test cli_writes services`
Expected: failures.

- [ ] **Step 3: Implement in `crates/illumio-tui/src/cli/objects.rs`**

Imports: `use super::write::resolve_service; use pce::write::{PortSpec, ServiceDraft, ServicePatch};`

```rust
#[derive(Args, Debug)]
pub struct ServicesCmd {
    #[command(subcommand)]
    pub action: Option<ServicesAction>,
}

#[derive(Subcommand, Debug)]
pub enum ServicesAction {
    /// Create a service in draft (needs writes enabled and --allow-writes)
    Add(ServiceAddArgs),
    /// Add or remove ports; the full port list is re-sent
    Update(ServiceUpdateArgs),
    /// Delete a service from draft
    Delete(ServiceDeleteArgs),
}

#[derive(Args, Debug)]
pub struct ServiceAddArgs {
    /// Service name
    pub name: String,
    /// Port: P/PROTO, P-Q/PROTO, icmp, or a bare protocol (repeatable, at least one)
    #[arg(long = "port", required = true)]
    pub ports: Vec<String>,
    /// Description
    #[arg(long)]
    pub description: Option<String>,
    /// Do nothing when a service with exactly this name exists
    #[arg(long)]
    pub if_missing: bool,
    #[command(flatten)]
    pub write: WriteFlags,
}

#[derive(Args, Debug)]
pub struct ServiceUpdateArgs {
    /// Service name or href
    pub target: String,
    /// New name
    #[arg(long)]
    pub name: Option<String>,
    /// New description
    #[arg(long)]
    pub description: Option<String>,
    /// Port to add (repeatable)
    #[arg(long = "add-port")]
    pub add_ports: Vec<String>,
    /// Port to remove; must match an existing entry exactly (repeatable)
    #[arg(long = "remove-port")]
    pub remove_ports: Vec<String>,
    #[command(flatten)]
    pub write: WriteFlags,
}

#[derive(Args, Debug)]
pub struct ServiceDeleteArgs {
    /// Service name or href
    pub target: String,
    #[command(flatten)]
    pub write: WriteFlags,
}

fn parse_ports(specs: &[String]) -> Result<Vec<PortSpec>, CliError> {
    specs
        .iter()
        .map(|s| PortSpec::parse(s).map_err(CliError::Args))
        .collect()
}

pub async fn services_cmd(ctx: &Context, cmd: &ServicesCmd) -> Result<(), CliError> {
    match &cmd.action {
        None => services(ctx).await,
        Some(ServicesAction::Add(a)) => service_add(ctx, a).await,
        Some(ServicesAction::Update(a)) => service_update(ctx, a).await,
        Some(ServicesAction::Delete(a)) => service_delete(ctx, a).await,
    }
}

async fn service_add(ctx: &Context, a: &ServiceAddArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let draft = ServiceDraft {
        name: a.name.clone(),
        description: a.description.clone(),
        ports: parse_ports(&a.ports)?,
    };
    let if_missing = if a.if_missing {
        IfMissing::SkipExisting
    } else {
        IfMissing::Create
    };
    let plan = ctx
        .pce
        .writer(&guard)
        .plan_create_service(&draft, if_missing)
        .await?;
    run_plan(ctx, &a.write, &guard, plan).await
}

async fn service_update(ctx: &Context, a: &ServiceUpdateArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let patch = ServicePatch {
        name: a.name.clone(),
        description: a.description.clone(),
        add_ports: parse_ports(&a.add_ports)?,
        remove_ports: parse_ports(&a.remove_ports)?,
    };
    let href = resolve_service(ctx, &a.target).await?;
    let plan = ctx
        .pce
        .writer(&guard)
        .plan_update_service(&href, &patch)
        .await?;
    run_plan(ctx, &a.write, &guard, plan).await
}

async fn service_delete(ctx: &Context, a: &ServiceDeleteArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let href = resolve_service(ctx, &a.target).await?;
    let plan = ctx.pce.writer(&guard).plan_delete_service(&href)?;
    run_plan(ctx, &a.write, &guard, plan).await
}
```

In `cli/mod.rs`: `Services(objects::ServicesCmd)` with doc `/// List services; add, update or delete one`, dispatch `Command::Services(c) => objects::services_cmd(&ctx, &c).await`. Add the `services ... --help` entries to `every_flag_has_help_text`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p illumio-tui`
Expected: all pass.

- [ ] **Step 5: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add crates/illumio-tui/src/cli crates/illumio-tui/tests
git commit -m "feat(cli): services add/update/delete with one service_ports list and dropped-field reporting

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 13: `workloads add|update|delete` (unmanaged)

**Files:**
- Modify: `crates/illumio-tui/src/cli/workloads.rs`
- Modify: `crates/illumio-tui/src/cli/mod.rs` (`Workloads(workloads::WorkloadsCmd)`, dispatch)
- Modify: `crates/illumio-tui/tests/cli.rs` (`every_flag_has_help_text` list)
- Test: `crates/illumio-tui/tests/cli_writes.rs` (append)

**Interfaces:**
- Consumes: `prepare`, `run_plan`, `resolve_labels`, `parse_kv`, `is_href`, `WriteFlags`, `WorkloadDraft`, `WorkloadPatch`, `LabelRemoval`, `InterfaceSpec`, `IfMissing`, `Writer::find_workload / plan_create_workload / plan_update_workload / plan_delete_workload`, `Pce::name_index`.
- Produces: `workloads::WorkloadsCmd { action: Option<WorkloadsAction>, list: WorkloadsArgs }`, `workloads::workloads_cmd(ctx, &WorkloadsCmd)`.

- [ ] **Step 1: Write the failing tests** (append to `tests/cli_writes.rs`)

```rust
mod workloads {
    use super::*;

    fn printer() -> Value {
        json!({
            "href": "/orgs/5/workloads/u1", "name": "printer-1", "hostname": "printer-1.lab",
            "managed": false, "interfaces": [{"name": "eth0", "address": "10.0.0.50", "cidr_block": 24}],
            "labels": [{"href": "/orgs/5/labels/1", "key": "app", "value": "print"}]
        })
    }

    fn labels() -> Value {
        json!([
            {"href": "/orgs/5/labels/1", "key": "app", "value": "print"},
            {"href": "/orgs/5/labels/2", "key": "env", "value": "lab"}
        ])
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn add_dry_run_resolves_labels() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_index(&server, labels(), json!([]), json!([])).await;
        let out = stdout_json(
            writer(&server, state.path())
                .args([
                    "workloads", "add", "printer-1", "--hostname", "printer-1.lab",
                    "--ip", "10.0.0.50/24", "--ip", "fe80::1", "--label", "app=print", "--label", "env=lab",
                    "--description", "lobby", "--dry-run",
                ])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["method"], "POST");
        assert_eq!(out["data"]["path"], "/orgs/5/workloads");
        assert_eq!(
            out["data"]["body"],
            json!({
                "name": "printer-1", "hostname": "printer-1.lab", "description": "lobby",
                "interfaces": [{"name": "eth0", "address": "10.0.0.50", "cidr_block": 24}, {"name": "eth1", "address": "fe80::1"}],
                "labels": [{"href": "/orgs/5/labels/1"}, {"href": "/orgs/5/labels/2"}]
            })
        );
        let a = writer(&server, state.path())
            .args(["workloads", "add", "p", "--label", "app=nope", "--dry-run"])
            .assert()
            .code(2);
        assert!(stderr_json(&a)["error"]["message"].as_str().unwrap().contains("print"));
        assert!(server.received_requests().await.unwrap().iter().all(|r| r.method.as_str() == "GET"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn add_sends_and_is_not_pending() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        Mock::given(method("POST"))
            .and(path("/api/v2/orgs/5/workloads"))
            .and(body_json(json!({"name": "printer-1", "hostname": "printer-1", "interfaces": [], "labels": []})))
            .respond_with(ResponseTemplate::new(201).set_body_json(printer()))
            .expect(1)
            .mount(&server)
            .await;
        let out = stdout_json(
            writer(&server, state.path())
                .args(["workloads", "add", "printer-1", "--yes"])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["pending"], false);
        assert_eq!(out["data"]["href"], "/orgs/5/workloads/u1");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn update_by_hostname_sends_only_changed_keys() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_index(&server, labels(), json!([]), json!([])).await;
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/workloads"))
            .and(wiremock::matchers::query_param("hostname", "printer-1.lab"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([printer()])))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/workloads/u1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(printer()))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/api/v2/orgs/5/workloads/u1"))
            .and(body_json(json!({
                "description": "moved",
                "labels": [{"href": "/orgs/5/labels/1"}, {"href": "/orgs/5/labels/2"}]
            })))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        let out = stdout_json(
            writer(&server, state.path())
                .args([
                    "workloads", "update", "printer-1.lab", "--description", "moved",
                    "--add-label", "env=lab", "--yes",
                ])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["status"], 204);
        assert_eq!(out["data"]["object"]["hostname"], "printer-1.lab");
        // --set-labels conflicts with --add-label (clap, exit 2).
        writer(&server, state.path())
            .args(["workloads", "update", "printer-1.lab", "--set-labels", "env=lab", "--add-label", "env=lab", "--yes"])
            .assert()
            .code(2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn managed_workloads_are_refused_locally() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/workloads/m1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"href": "/orgs/5/workloads/m1", "hostname": "web-1", "managed": true}),
            ))
            .mount(&server)
            .await;
        let a = writer(&server, state.path())
            .args(["workloads", "delete", "/orgs/5/workloads/m1", "--yes"])
            .assert()
            .code(2);
        assert!(stderr_json(&a)["error"]["message"].as_str().unwrap().contains("VEN-managed"));
        let a = writer(&server, state.path())
            .args(["workloads", "update", "/orgs/5/workloads/m1", "--description", "x", "--yes"])
            .assert()
            .code(2);
        assert_eq!(stderr_json(&a)["error"]["kind"], "invalid");
        assert!(server.received_requests().await.unwrap().iter().all(|r| r.method.as_str() == "GET"));
        assert!(audit_lines(state.path()).is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn removing_the_last_interface_without_a_hostname_is_refused() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        let mut no_host = printer();
        no_host["hostname"] = Value::Null;
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/workloads/u1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(no_host))
            .mount(&server)
            .await;
        mount_index(&server, labels(), json!([]), json!([])).await;
        let a = writer(&server, state.path())
            .args(["workloads", "update", "/orgs/5/workloads/u1", "--remove-ip", "10.0.0.50", "--yes"])
            .assert()
            .code(2);
        assert_eq!(stderr_json(&a)["error"]["kind"], "invalid");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn delete_by_href() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/workloads/u1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(printer()))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/api/v2/orgs/5/workloads/u1"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        let out = stdout_json(
            writer(&server, state.path())
                .args(["workloads", "delete", "/orgs/5/workloads/u1", "--yes"])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["href"], "/orgs/5/workloads/u1");
        assert_eq!(audit_lines(state.path())[0]["method"], "DELETE");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn listing_still_works_without_a_subcommand() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/workloads"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([printer()])))
            .mount(&server)
            .await;
        let out = stdout_json(cmd(&server).args(["workloads", "--managed", "false", "--json"]).assert().success());
        assert_eq!(out["data"][0]["hostname"], "printer-1.lab");
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p illumio-tui --test cli_writes workloads`
Expected: failures.

- [ ] **Step 3: Implement in `crates/illumio-tui/src/cli/workloads.rs`**

Imports to add:

```rust
use super::write::{self, WriteFlags, is_href, parse_kv, resolve_labels, run_plan};
use clap::Subcommand;
use pce::write::{IfMissing, InterfaceSpec, LabelRemoval, WorkloadDraft, WorkloadPatch};
```

After `WorkloadsArgs`:

```rust
/// `workloads` lists by default; `add`, `update` and `delete` write
/// unmanaged workloads.
#[derive(Args, Debug)]
#[command(args_conflicts_with_subcommands = true)]
pub struct WorkloadsCmd {
    #[command(subcommand)]
    pub action: Option<WorkloadsAction>,
    #[command(flatten)]
    pub list: WorkloadsArgs,
}

#[derive(Subcommand, Debug)]
pub enum WorkloadsAction {
    /// Create an unmanaged workload (needs writes enabled and --allow-writes)
    Add(WorkloadAddArgs),
    /// Change an unmanaged workload's name, hostname, interfaces or labels
    Update(WorkloadUpdateArgs),
    /// Delete an unmanaged workload
    Delete(WorkloadDeleteArgs),
}

#[derive(Args, Debug)]
pub struct WorkloadAddArgs {
    /// Workload name (also the hostname unless --hostname is given)
    pub name: String,
    /// Hostname
    #[arg(long)]
    pub hostname: Option<String>,
    /// Interface address, ADDR or ADDR/N (repeatable; named eth0, eth1, ...)
    #[arg(long = "ip")]
    pub ips: Vec<String>,
    /// Label key=value (repeatable)
    #[arg(long = "label")]
    pub labels: Vec<String>,
    /// Description
    #[arg(long)]
    pub description: Option<String>,
    /// Do nothing when a workload with exactly this hostname exists
    #[arg(long)]
    pub if_missing: bool,
    #[command(flatten)]
    pub write: WriteFlags,
}

#[derive(Args, Debug)]
pub struct WorkloadUpdateArgs {
    /// Hostname, name or href
    pub target: String,
    /// New name
    #[arg(long)]
    pub name: Option<String>,
    /// New hostname
    #[arg(long)]
    pub hostname: Option<String>,
    /// New description
    #[arg(long)]
    pub description: Option<String>,
    /// Interface address to add, ADDR or ADDR/N (repeatable)
    #[arg(long = "add-ip")]
    pub add_ips: Vec<String>,
    /// Interface address to remove (repeatable)
    #[arg(long = "remove-ip")]
    pub remove_ips: Vec<String>,
    /// Replace all labels: key=value,key=value (conflicts with --add-label/--remove-label)
    #[arg(long, value_delimiter = ',', conflicts_with_all = ["add_labels", "remove_labels"])]
    pub set_labels: Option<Vec<String>>,
    /// Label key=value to add (repeatable)
    #[arg(long = "add-label")]
    pub add_labels: Vec<String>,
    /// Label to remove: key (any value) or key=value (repeatable)
    #[arg(long = "remove-label")]
    pub remove_labels: Vec<String>,
    #[command(flatten)]
    pub write: WriteFlags,
}

#[derive(Args, Debug)]
pub struct WorkloadDeleteArgs {
    /// Hostname, name or href
    pub target: String,
    #[command(flatten)]
    pub write: WriteFlags,
}

pub async fn workloads_cmd(ctx: &Context, cmd: &WorkloadsCmd) -> Result<(), CliError> {
    match &cmd.action {
        None => run(ctx, &cmd.list).await,
        Some(WorkloadsAction::Add(a)) => workload_add(ctx, a).await,
        Some(WorkloadsAction::Update(a)) => workload_update(ctx, a).await,
        Some(WorkloadsAction::Delete(a)) => workload_delete(ctx, a).await,
    }
}

fn parse_interfaces(specs: &[String]) -> Result<Vec<InterfaceSpec>, CliError> {
    specs
        .iter()
        .enumerate()
        .map(|(i, s)| InterfaceSpec::parse(s, format!("eth{i}")).map_err(CliError::Args))
        .collect()
}

async fn workload_add(ctx: &Context, a: &WorkloadAddArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let draft = WorkloadDraft {
        name: a.name.clone(),
        hostname: a.hostname.clone(),
        description: a.description.clone(),
        interfaces: parse_interfaces(&a.ips)?,
        labels: resolve_labels(ctx, &a.labels).await?,
    };
    let if_missing = if a.if_missing {
        IfMissing::SkipExisting
    } else {
        IfMissing::Create
    };
    let plan = ctx
        .pce
        .writer(&guard)
        .plan_create_workload(&draft, if_missing)
        .await?;
    run_plan(ctx, &a.write, &guard, plan).await
}

/// An href as given, or an exact hostname/name match (one or two reads).
async fn workload_href(ctx: &Context, guard: &pce::write::WriteGuard, target: &str) -> Result<String, CliError> {
    if is_href(target) {
        return Ok(target.trim().to_string());
    }
    Ok(ctx.pce.writer(guard).find_workload(target).await?.href)
}

async fn workload_update(ctx: &Context, a: &WorkloadUpdateArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let remove_labels = a
        .remove_labels
        .iter()
        .map(|s| match s.split_once('=') {
            Some(_) => parse_kv(s).map(|(k, v)| LabelRemoval { key: k, value: Some(v) }),
            None if s.trim().is_empty() => Err(CliError::Args("--remove-label needs a key".into())),
            None => Ok(LabelRemoval { key: s.trim().to_string(), value: None }),
        })
        .collect::<Result<Vec<_>, _>>()?;
    // `--set-labels ""` clears every label: clap yields `Some(vec![""])`.
    let set_labels = match &a.set_labels {
        Some(specs) => {
            let specs: Vec<String> = specs
                .iter()
                .filter(|s| !s.trim().is_empty())
                .cloned()
                .collect();
            Some(resolve_labels(ctx, &specs).await?)
        }
        None => None,
    };
    let patch = WorkloadPatch {
        name: a.name.clone(),
        hostname: a.hostname.clone(),
        description: a.description.clone(),
        add_addresses: a.add_ips.clone(),
        remove_addresses: a.remove_ips.clone(),
        set_labels,
        add_labels: resolve_labels(ctx, &a.add_labels).await?,
        remove_labels,
    };
    let href = workload_href(ctx, &guard, &a.target).await?;
    let idx = ctx.pce.name_index(ctx.cache_mode).await?;
    let plan = ctx
        .pce
        .writer(&guard)
        .plan_update_workload(&href, &patch, &idx)
        .await?;
    run_plan(ctx, &a.write, &guard, plan).await
}

async fn workload_delete(ctx: &Context, a: &WorkloadDeleteArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let href = workload_href(ctx, &guard, &a.target).await?;
    let plan = ctx.pce.writer(&guard).plan_delete_workload(&href).await?;
    run_plan(ctx, &a.write, &guard, plan).await
}
```

In `cli/mod.rs`: `Workloads(workloads::WorkloadsCmd)` with doc `/// List workloads; add, update or delete an unmanaged one`, dispatch `Command::Workloads(c) => workloads::workloads_cmd(&ctx, &c).await`. Add the three `workloads ... --help` entries to `every_flag_has_help_text`. Check the TUI does not construct `WorkloadsArgs` through `Command::Workloads` (grep `Command::Workloads` in `crates/illumio-tui/src`); it does not as of 19fdf85.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p illumio-tui`
Expected: all pass, including the existing `workloads` listing tests in `tests/cli.rs`.

- [ ] **Step 5: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add crates/illumio-tui/src/cli crates/illumio-tui/tests
git commit -m "feat(cli): workloads add/update/delete for unmanaged workloads; managed ones refused locally

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 14: Opt-in live smoke, docs, tracking, final gate

**Files:**
- Create: `crates/pce/tests/live_writes.rs`
- Modify: `README.md` (new section "Writes" after "CLI"), `docs/architecture.md` (new section "Writes" after "PCE protection, where it lives"; update "Testing"), `docs/index.html` (section 1 and the CLI example block), `docs/superpowers/specs/2026-10-10-pce-writes-design.md` (section 8 file table: `specs.rs` instead of `drafts.rs`), `tasks/todo.md`, `tasks/lessons.md`

- [ ] **Step 1: Live write smoke** (`crates/pce/tests/live_writes.rs`)

```rust
//! Opt-in, destructive on purpose and therefore doubly gated: `PCE_LIVE_WRITES=1`
//! plus the usual PCE_HOST, PCE_ORG_ID, API_KEY, API_SECRET (optionally
//! PCE_PORT, PCE_TLS_VERIFY). Creates one label `illumio-tui-smoke-<ts>` and
//! deletes it again: 4 API calls. Run it only against a PCE you designate
//! as non-production. Never in CI.
use pce::client::{ClientConfig, PceClient};
use pce::write::{IfMissing, LabelDraft, MemoryAudit, Outcome, WriteContext, WriteGuard};
use pce::{Credentials, Pce};

#[tokio::test]
async fn live_label_create_and_delete() {
    if std::env::var("PCE_LIVE_WRITES").as_deref() != Ok("1") {
        eprintln!("PCE_LIVE_WRITES != 1, skipping");
        return;
    }
    let host = std::env::var("PCE_HOST").expect("PCE_HOST");
    let org: u64 = std::env::var("PCE_ORG_ID").expect("PCE_ORG_ID").parse().unwrap();
    let creds = Credentials::new(
        std::env::var("API_KEY").expect("API_KEY"),
        std::env::var("API_SECRET").expect("API_SECRET"),
    );
    let mut cfg = ClientConfig::new(host.clone(), org, creds);
    if let Ok(p) = std::env::var("PCE_PORT") {
        cfg.port = p.parse().unwrap();
    }
    if let Ok(v) = std::env::var("PCE_TLS_VERIFY") {
        cfg.tls_verify = !matches!(v.to_ascii_lowercase().as_str(), "false" | "0" | "no");
    }
    let pce = Pce::new(PceClient::new(cfg).unwrap());
    let audit = MemoryAudit::new();
    let guard = WriteGuard::new(
        true,
        true,
        Box::new(audit.clone()),
        WriteContext { profile: "live".into(), host, org_id: org, source: "test".into() },
    )
    .unwrap();
    let w = pce.writer(&guard);

    let value = format!("illumio-tui-smoke-{}", jiff::Timestamp::now().as_second());
    let draft = LabelDraft::new("illumio-tui-smoke", &value);
    let plan = w.plan_create_label(&draft, IfMissing::SkipExisting).await.unwrap();
    let Outcome::Written(created) = w.execute(plan).await.unwrap() else {
        panic!("smoke label already existed; clean up the PCE")
    };
    assert_eq!(created.status, 201);
    assert!(created.href.contains("/labels/"));

    let plan = w.plan_delete_label(&created.href).unwrap();
    let Outcome::Written(deleted) = w.execute(plan).await.unwrap() else {
        panic!("delete produced no result")
    };
    assert_eq!(deleted.status, 204);
    assert_eq!(audit.entries().len(), 2);
    assert!(pce.stats().calls <= 4, "{} calls", pce.stats().calls);
}
```

Run: `cargo test -p pce --test live_writes` without the env: prints the skip line and passes. **Do not run it with `PCE_LIVE_WRITES=1` against demo100** (reads only were approved there).

- [ ] **Step 2: README** (insert after the "CLI" section, before "PCE protection")

````markdown
## Writes

Writes are off until two things say yes: the profile (`writes = true`, or
`ILLUMIO_WRITES=1` in the environment) and the invocation (`--allow-writes`).
Every write command prints the exact request with `--dry-run` and needs
`--yes` to send it. Each sent request is appended to
`~/.local/state/illumio-tui/writes.jsonl` (or `$XDG_STATE_HOME/illumio-tui/`),
mode 0600, with the body sent and the status received. Nothing is ever retried.
IP lists and services are written to draft and wait for provisioning
(`"pending": true`); labels and workloads take effect at once.

```sh
illumio-tui --allow-writes labels add app=web --yes
illumio-tui --allow-writes labels add app=web --if-missing --yes     # no-op when it exists
illumio-tui --allow-writes iplists add Corp --range 10.0.0.0/8 --fqdn '*.corp.example' --yes
illumio-tui --allow-writes iplists update Corp --add-range 10.2.0.0/16 --dry-run
illumio-tui --allow-writes services add PostgreSQL --port 5432/tcp --yes
illumio-tui --allow-writes workloads add printer-1 --ip 10.0.0.50/24 --label app=print --yes
illumio-tui --allow-writes workloads update printer-1 --add-label env=lab --yes
illumio-tui --allow-writes workloads delete printer-1 --yes
```

Names resolve by exact match; an ambiguous or unknown name is exit code 2
with the candidates listed, before any request. Update commands re-fetch the
object and send the full list the PCE expects (`ip_ranges`, `service_ports`,
`labels`), so `--remove-range` and friends must match an existing entry.
Only unmanaged workloads can be written; a VEN-managed one is refused locally.
Exit code 6 means the guard refused (profile, flag, or audit log not writable).
Rulesets, rules and provisioning are the next stage.
````

Also add `6 | write refused by the guard` to the exit-code list in the CLI section if the README has one, and `writes = true` to the profile example in "Configure" with a comment.

- [ ] **Step 3: `docs/architecture.md`**

Add after "PCE protection, where it lives":

```markdown
## Writes

`pce::write` is the only path that mutates the PCE. A `Writer` exists only
through `Pce::writer(&WriteGuard)`, and a `WriteGuard` cannot be built unless
the profile allows writes, the caller passed `--allow-writes`, and the audit
log opens for append. Writes are two-phase: `plan_create_*`, `plan_update_*`
and `plan_delete_*` build a `Plan` (the exact `WriteRequest`, or a `Noop` when
`--if-missing` found an exact match) without sending anything; `execute` sends
it once through `PceClient::write` (no retry, 400/406/409/422 → `Rejected`),
appends the audit entry, invalidates the cache prefix of the kind (`objects:`
or `workloads`), and re-fetches the object after a 204. A timeout, connection
loss or gateway status after the request left is reported as `write_unanswered`
("may have applied") and audited with `phase: "sent"`. Updates of IP lists,
services and workload labels compute the full replacement list locally because
the PCE replaces those arrays wholesale. The CLI layer (`cli/write.rs`) owns
`--dry-run`/`--yes`, exact name resolution and the envelope (`meta.write`).
```

In "Testing", add a line: "`crates/pce/tests/write_contract.rs` pins every write body with wiremock; `crates/illumio-tui/tests/cli_writes.rs` covers the guard, exit codes 2/4/6, dry runs and the audit file. `live_writes.rs` is opt-in (`PCE_LIVE_WRITES=1`) and only ever creates and deletes one label."

- [ ] **Step 4: `docs/index.html`**

In section 1 ("What it does today") add one list item: `Writes labels, IP lists, services and unmanaged workloads from the CLI: profile opt-in plus <code>--allow-writes</code>, <code>--dry-run</code> shows the request, <code>--yes</code> sends it once, every write lands in a local audit log.` Add two lines to the CLI example `<pre>` block there: `illumio-tui --allow-writes labels add app=web --yes` and `illumio-tui --allow-writes iplists update Corp --add-range 10.2.0.0/16 --dry-run`. Keep the page's existing structure and classes; no new sections, no script.

- [ ] **Step 5: Spec file table, `tasks/todo.md`, `tasks/lessons.md`**

- In the spec's section 8 replace `drafts.rs` with `specs.rs` so the table matches the code.
- Append to `tasks/todo.md` a "Plan 3: writes" checklist with the 14 tasks checked, and a "Review (Plan 3)" section: what shipped, the gate command, test counts, the note that the live write smoke was not run against the shared demo PCE.
- Append to `tasks/lessons.md` any correction made during this plan's execution (reviewer findings that needed a fix round, PCE write pitfalls confirmed or refuted).

- [ ] **Step 6: Final gate**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
cargo run -p illumio-tui -- --help | grep -- --allow-writes
cargo run -p illumio-tui -- labels add --help
cargo run -p illumio-tui -- workloads update --help
```

Expected: green; the help texts list every flag with a description.

- [ ] **Step 7: Commit**

```bash
git add crates/pce/tests/live_writes.rs README.md docs tasks
git commit -m "docs: writes section, architecture notes, opt-in live write smoke, Plan 3 review

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```
