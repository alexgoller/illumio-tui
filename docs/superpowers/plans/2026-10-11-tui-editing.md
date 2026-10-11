# TUI Editing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a PCE admin create, edit and delete labels, IP lists, services and unmanaged workloads from the TUI through the existing `pce::write` guard and planners, with the exact request shown before `y` sends it once, a header marker while writes are on, and a navigable pending list that diffs a draft IP list or service against its active version.

**Architecture:** The model knows only `WritesState` (enabled + reason); the runtime owns `Option<Arc<WriteGuard>>`, built in `run()` and in the `:profile` switch from the same inputs as the CLI (`source: "tui"`). `n`/`e`/`x` produce an `EditIntent`; `tui/edit.rs` turns it into a pre-filled `Form` (`tui/form.rs`, `Overlay::Form`) or, for deletes, straight into a `WriteIntent`; a submitted form becomes a library draft or patch (pure adapters). `Cmd::PlanWrite` runs the matching `plan_*` in a task and returns `Msg::Planned`; `Overlay::ConfirmWrite` shows summary, `METHOD path` and pretty JSON (the TUI's dry run); `y` emits `Cmd::ExecuteWrite`, whose task calls `Writer::execute` once and returns `Msg::WriteDone`, after which the kind's table reloads once with `CacheMode::Bypass`. The pending overlay gains a selection; `Enter` emits `Cmd::LoadDiff`, whose task reads the draft and the new active lists through the cache and computes an `ObjectDiff` with pure functions in `tui/diff.rs`.

**Tech Stack:** Rust 2024 (rust-version 1.88), tokio, ratatui, crossterm, serde_json, insta (snapshots), wiremock 0.6 (runtime and library tests), tempfile; the existing `pce` crate (`pce::write`, `pce::models`, `pce::NameIndex`).

**Spec:** `docs/superpowers/specs/2026-10-11-tui-editing-design.md` (Plan 3 spec `docs/superpowers/specs/2026-10-10-pce-writes-design.md` for the write layer's rules).

## Global Constraints

- edition 2024, rust-version 1.88; no new dependencies.
- Every PCE request is user-caused (no timers, no refresh on its own).
- POST/PUT/DELETE are sent exactly once and never retried.
- The TUI never bypasses `WriteGuard`: every write is planned by a `pce.writer(&guard).plan_*` call and sent by `Writer::execute`; the TUI never builds a `WriteRequest` path or body itself (the one exception, the confirm summary of a delete, rewrites `summary` only, which is never sent or audited).
- Nothing in the plan runs against a real PCE except the final read-only/request-free smoke in Task 9; `PCE_LIVE_WRITES=1` must never be set. No write may be sent to demo100.
- The CLI and its envelope are untouched (`crates/illumio-tui/src/cli/**` does not change).
- Existing runtime tests keep passing unchanged (new `Runtime::with_writer` constructor, `Runtime::new` keeps `writer = None`). The only edit to `tests/tui_runtime.rs` outside new tests is the sixth argument of its `model()` helper (Task 2).
- The help overlay must still fit 80x24 or scroll; both themes snapshot (every new overlay gets a dark text snapshot and a light styled snapshot).
- The TUI's `ObjectKind` (Objects tabs, `tui/model.rs`) and `pce::write::ObjectKind` collide: the latter is always written fully qualified, never imported or aliased.
- Commit trailer on every commit, exactly (after a blank line): `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`.
- Gate before every commit: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p <crate>` (the task names the crate; Task 9 runs `cargo test --workspace`).
- Edition 2024 clippy may rewrite nested `if`/`if let` into let-chains: accept and list it as a deviation in the task report. If clippy reports `large_enum_variant` on `Overlay`, `Cmd` or `Msg`, box the new payload (for example `Overlay::Form(Box<Form>)`) and list it as a deviation.
- Snapshots: new ones are created with `INSTA_UPDATE=always cargo test -p illumio-tui --test tui_snapshots`; afterwards `git status --short crates/illumio-tui/tests/snapshots` must list exactly the files the task names (new or intentionally changed), each read before committing.
- Spec refinements this plan makes (reviewers: these are intended):
  - `WriteIntent::Delete*` carry `name` next to `href`, so the confirm summary names the object (spec 5.1 asks for the name; the library's delete summary carries the href).
  - `Msg::WriteDone` carries `method: Option<pce::WriteMethod>` so the status line can say created / updated / deleted.
  - `Cmd::LoadDiff` carries the whole `PendingChange` (href, `change_type`, name) instead of only the href, so created / updated / deleted and the "not in the list" text do not depend on cache freshness.
  - `PolicyState.diff_loading: bool` keeps one diff task in flight (one task per resource, Plan 2 rule).
  - The help scroll lives in `Model.help_scroll` (not in `Overlay::Help`), so the twelve existing `Overlay::Help` uses stay as they are.
  - The guard helper is `build_writer(ctx: &Context)`: `Context` already carries `allow_writes` from `GlobalArgs` (`cli/context.rs` ~line 56).
  - With writes on, the Objects and Workloads hints become `n:new e:edit x:del  Tab type  Enter detail` and `n:new e:edit x:del  t/T traffic  Enter detail`: appending to today's hints would leave no room for the status text at 80 columns.
  - `active_ip_lists` / `active_services` use the policy TTL (5 minutes, spec 6.2) under `objects:` keys.
  - Three arms are staged and replaced by the next task: Task 3's form `Enter` closes the form, Task 5's `Cmd::PlanWrite` arm only sets a status line, Task 6's `Cmd::ExecuteWrite` arm only sets a status line. None of them sends a request.

## Review Focus

1. A form submitted exactly as pre-filled (any kind). Expected: status `no changes`, the form closes, no `Cmd`, no request. Tests: Task 4 `ip_list_edit_adds_and_removes_only_what_changed` (first assertion), Task 5 `e_opens_a_prefilled_form_and_enter_plans_only_the_change`.
2. `e` on IP list Corp adding `10.2.0.0/16` to the Ranges field. Expected: one GET of the list at plan time, one PUT whose body is exactly `{"ip_ranges": [existing range, new range]}` (no `name`, `description`, `fqdns`), one list GET to reload. Test: Task 7 `update_ip_list_puts_only_the_changed_ranges`.
3. `x` on a service, `y`, PCE answers 406. Expected: one DELETE, one audit entry with status 406, `Overlay::Error` containing the PCE's message, no reload GET, never a retry. Test: Task 7 `delete_service_rejected_shows_the_body_and_does_not_reload`.
4. `n`/`e`/`x` with writes off, and after `:profile` to a profile without writes. Expected: the reason in the status line as a warning, no form, the request counter unchanged, `rt.writer` is `None`, no `✎ writes` in the header. Tests: Task 7 `writes_disabled_editing_keys_send_nothing`, Task 7 `switching_to_a_profile_without_writes_drops_the_guard_and_the_marker`.
5. `p`, `Enter` on a pending IP list, `Esc`, `Enter` again. Expected: exactly one `GET sec_policy/active/ip_lists`, the draft list served from the cache, the second `Enter` costs nothing. Test: Task 8 `pending_diff_fetches_the_active_list_once`.

---

## File structure

| File | Responsibility |
| --- | --- |
| `crates/pce/src/lib.rs` | `Pce::active_ip_lists`, `Pce::active_services` |
| `crates/pce/tests/facade.rs` | wiremock test for the active lists |
| `crates/illumio-tui/src/tui/model.rs` | `WritesState`, `Model.writes`, `Model.help_scroll`, `Overlay::{Form, ConfirmWrite, Diff}`, `PolicyState.{pending_selected, diff_loading}` |
| `crates/illumio-tui/src/tui/msg.rs` | `Cmd::{PlanWrite, ExecuteWrite, LoadDiff}`, `Msg::{Planned, WriteDone, DiffLoaded}` |
| `crates/illumio-tui/src/tui/form.rs` (new) | `Form`, `Field`, `FormEvent`, `visible_tail` |
| `crates/illumio-tui/src/tui/edit.rs` (new) | `EditIntent`, `WriteIntent`, `Start`, `start`, `form_for`, `build_intent`, `request_body_lines` |
| `crates/illumio-tui/src/tui/diff.rs` (new) | `ObjectDiff`, `DiffKind`, `Sign`, `DiffLine`, `diff_ip_list`, `diff_service`, `ip_list_diff_for`, `service_diff_for` |
| `crates/illumio-tui/src/tui/update.rs` | editing keys, form/confirm/diff/pending/help keys, `Planned`, `WriteDone`, `DiffLoaded` |
| `crates/illumio-tui/src/tui/app.rs` | `Runtime.writer`, `with_writer`, `build_writer`, `PlanWrite`/`ExecuteWrite`/`LoadDiff` tasks, `task_result` list |
| `crates/illumio-tui/src/tui/ui/mod.rs` | header marker, overlay dispatch |
| `crates/illumio-tui/src/tui/ui/widgets/overlay.rs` | `render_form`, `render_confirm_write`, `render_diff`, scrollable help, pending selection, hints |
| `crates/illumio-tui/src/tui/ui/widgets/status.rs` | hint with writes |
| `crates/illumio-tui/src/tui/test_support.rs` | `model_fixture` writes, `label_post_plan`, `ip_list_delete_plan` |
| `crates/illumio-tui/tests/tui_snapshots.rs`, `tests/tui_runtime.rs` | snapshots and wiremock runtime tests |
| `README.md`, `docs/architecture.md`, `docs/index.html`, `tasks/todo.md` | docs and tracking |

---

### Task 1: Library: active IP lists and services

**Files:**
- Modify: `crates/pce/src/lib.rs` (after `services`, ~line 158)
- Test: `crates/pce/tests/facade.rs` (append)

**Interfaces:**
- Consumes: `Pce::cached_list` (`lib.rs` ~line 336), `CacheTtls.policy` (5 min), `pce::write::ObjectKind::cache_prefix` (`write/mod.rs` ~line 40), `pce_for` in `tests/facade.rs` (~line 8).
- Produces:

```rust
impl Pce {
    pub async fn active_ip_lists(&self, mode: CacheMode) -> Result<Cached<Vec<models::IpList>>>;  // key objects:active_ip_lists, GET sec_policy/active/ip_lists
    pub async fn active_services(&self, mode: CacheMode) -> Result<Cached<Vec<models::Service>>>;  // key objects:active_services, GET sec_policy/active/services
}
```

- [ ] **Step 1: Write the failing test** (append to `crates/pce/tests/facade.rs`)

```rust
#[tokio::test]
async fn active_lists_are_cached_under_objects_and_dropped_by_a_write() {
    let server = MockServer::start().await;
    for (p, body) in [
        (
            "/api/v2/orgs/5/sec_policy/active/ip_lists",
            json!([{"href": "/orgs/5/sec_policy/active/ip_lists/20", "name": "Any", "ip_ranges": [{"from_ip": "0.0.0.0/0"}]}]),
        ),
        (
            "/api/v2/orgs/5/sec_policy/active/services",
            json!([{"href": "/orgs/5/sec_policy/active/services/32", "name": "Web", "service_ports": [{"port": 443, "proto": 6}]}]),
        ),
    ] {
        Mock::given(method("GET"))
            .and(path(p))
            .and(query_param("max_results", "10000"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(2)
            .mount(&server)
            .await;
    }
    let p = pce_for(&server);
    let lists = p.active_ip_lists(CacheMode::Use).await.unwrap();
    assert!(!lists.hit);
    assert_eq!(lists.value[0].name, "Any");
    assert!(p.active_ip_lists(CacheMode::Use).await.unwrap().hit);
    let services = p.active_services(CacheMode::Use).await.unwrap();
    assert_eq!(services.value[0].service_ports[0].port, Some(443));
    assert!(p.active_services(CacheMode::Use).await.unwrap().hit);
    assert_eq!(p.stats().calls, 2, "one GET per list");
    // `Writer::execute` invalidates the kind's prefix; for IP lists and
    // services that is `objects:`, which must cover the active lists too.
    p.invalidate(pce::write::ObjectKind::IpList.cache_prefix());
    assert!(!p.active_ip_lists(CacheMode::Use).await.unwrap().hit);
    assert!(!p.active_services(CacheMode::Use).await.unwrap().hit);
    assert_eq!(p.stats().calls, 4);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p pce --test facade active_lists`
Expected: FAIL, compile error `no method named active_ip_lists found for struct Pce`.

- [ ] **Step 3: Implement** (in `crates/pce/src/lib.rs`, directly after `pub async fn services`)

```rust
    /// Provisioned IP lists, for the TUI's pending diff. Cached under
    /// `objects:` so a write invalidates them with the draft lists; kept five
    /// minutes like the rest of the active policy.
    pub async fn active_ip_lists(&self, mode: CacheMode) -> Result<Cached<Vec<models::IpList>>> {
        let path = self.client.org_path("sec_policy/active/ip_lists");
        self.cached_list(
            "objects:active_ip_lists",
            &path,
            vec![],
            self.ttls.policy,
            mode,
        )
        .await
    }

    /// Provisioned services, for the TUI's pending diff (see `active_ip_lists`).
    pub async fn active_services(&self, mode: CacheMode) -> Result<Cached<Vec<models::Service>>> {
        let path = self.client.org_path("sec_policy/active/services");
        self.cached_list(
            "objects:active_services",
            &path,
            vec![],
            self.ttls.policy,
            mode,
        )
        .await
    }
```

- [ ] **Step 4: Run the test**

Run: `cargo test -p pce --test facade`
Expected: PASS (all facade tests, including `active_lists_are_cached_under_objects_and_dropped_by_a_write`).

- [ ] **Step 5: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p pce
git add crates/pce/src/lib.rs crates/pce/tests/facade.rs
git commit -m "feat(pce): cached reads of the active IP lists and services

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 2: Writes state, runtime guard and header marker

**Files:**
- Modify: `crates/illumio-tui/src/tui/model.rs` (new `WritesState` after `StatusLine`'s impl ~line 100; `Model` field ~line 1674; `Model::new_connecting` ~line 1701)
- Modify: `crates/illumio-tui/src/tui/app.rs` (imports ~line 4-20; `SwitchResult` ~line 44; `Runtime` ~line 46; `new`/`with_args`/`build` ~lines 60-93; the `:profile` `spawn_blocking` closure ~line 123; `finish_switch` ~line 307; `run_loop`'s `mem::replace` ~line 409; `run` ~line 445; tests ~line 486)
- Modify: `crates/illumio-tui/src/tui/ui/mod.rs` (`render_header` ~line 63)
- Modify: `crates/illumio-tui/src/tui/test_support.rs` (`model_fixture` ~line 29)
- Modify: `crates/illumio-tui/src/tui/update.rs` (test `startup_connect_flow` ~line 1824: sixth argument)
- Modify: `crates/illumio-tui/tests/tui_runtime.rs` (`model()` helper ~line 34: sixth argument; import)
- Test: `crates/illumio-tui/tests/tui_snapshots.rs` (append), `app.rs` unit tests

**Interfaces:**
- Consumes: `WriteGuard::{check, new, context}`, `WriteContext`, `WriteRefused` (`pce/src/write/guard.rs`), `JsonlAudit::{new, default_path}`, `MemoryAudit::new` (`pce/src/write/audit.rs`), `Context { config.writes, config.profile_name, config.host, config.org_id, allow_writes, pce }` (`cli/context.rs`), `ResolvedConfig.writes` (`config.rs` ~line 106).
- Produces:

```rust
// tui/model.rs
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WritesState { pub enabled: bool, pub reason: Option<String> }
impl WritesState {
    pub fn on() -> Self;
    pub fn off(reason: impl Into<String>) -> Self;
    pub fn refusal(&self) -> String;   // reason, or "writes are not enabled"
}
pub struct Model { /* … */ pub writes: WritesState }
impl Model {
    pub fn new_connecting(profile: ProfileInfo, theme_name: String, auto_refresh_min_seconds: u64,
        traffic_max_results: u32, traffic_default_window: String, writes: WritesState) -> Self;
}
// tui/app.rs
type SwitchResult = Result<(Pce, ProfileInfo, Limits, Option<WriteGuard>, WritesState), String>;
pub struct Runtime { /* … */ pub writer: Option<Arc<WriteGuard>> }
impl Runtime {
    pub fn with_writer(pce: Arc<Pce>, model: Model, writer: WriteGuard) -> (Self, UnboundedSender<Msg>);
}
fn build_writer(ctx: &Context) -> (Option<WriteGuard>, WritesState);
fn writer_from(profile_allows: bool, flag_allows: bool, audit_path: Option<PathBuf>, ctx: WriteContext)
    -> (Option<WriteGuard>, WritesState);
```

- [ ] **Step 1: Write the failing tests**

Append to the `#[cfg(test)] mod tests` in `crates/illumio-tui/src/tui/app.rs`:

```rust
    fn write_ctx() -> WriteContext {
        WriteContext {
            profile: "demo".into(),
            host: "pce.example".into(),
            org_id: 5,
            source: "tui".into(),
        }
    }

    #[test]
    fn writer_needs_both_switches_and_a_writable_audit_log() {
        let dir = tempfile::tempdir().unwrap();
        let path = || Some(dir.path().join("illumio-tui").join("writes.jsonl"));
        let (g, w) = writer_from(false, true, path(), write_ctx());
        assert!(g.is_none());
        assert!(!w.enabled);
        assert!(
            w.reason.as_deref().unwrap().contains("not enabled for this profile"),
            "{w:?}"
        );
        let (g, w) = writer_from(true, false, path(), write_ctx());
        assert!(g.is_none());
        assert_eq!(w.reason.as_deref(), Some("writes need --allow-writes"));
        let (g, w) = writer_from(true, true, None, write_ctx());
        assert!(g.is_none());
        assert_eq!(w.reason.as_deref(), Some(AUDIT_PATH_UNKNOWN));
        let blocker = dir.path().join("a-file");
        std::fs::write(&blocker, "x").unwrap();
        let (g, w) = writer_from(true, true, Some(blocker.join("writes.jsonl")), write_ctx());
        assert!(g.is_none());
        assert!(
            w.reason.as_deref().unwrap().starts_with("audit log is not writable"),
            "{w:?}"
        );
        let (g, w) = writer_from(true, true, path(), write_ctx());
        assert_eq!(w, WritesState::on());
        assert_eq!(g.unwrap().context().source, "tui");
    }

    #[tokio::test]
    async fn a_switch_without_a_guard_drops_it_and_clears_the_marker() {
        use pce::write::MemoryAudit;
        let pce = || {
            Pce::new(
                pce::PceClient::new(pce::ClientConfig::new(
                    "pce.invalid",
                    5,
                    pce::Credentials::new("k", "s"),
                ))
                .unwrap(),
            )
        };
        let guard =
            WriteGuard::new(true, true, Box::new(MemoryAudit::new()), write_ctx()).unwrap();
        let mut model = crate::tui::test_support::model_fixture();
        model.writes = WritesState::on();
        let (mut rt, _tx) = Runtime::with_writer(Arc::new(pce()), model, guard);
        assert!(rt.writer.is_some());
        // A failed switch keeps the guard and the marker.
        rt.finish_switch(Err("profile \"nope\" not found".into()));
        assert!(rt.writer.is_some());
        assert!(rt.model.writes.enabled);
        rt.finish_switch(Ok((
            pce(),
            ProfileInfo {
                name: "ro".into(),
                host: "pce.invalid".into(),
                org_id: 5,
            },
            Limits {
                auto_refresh_min_seconds: 60,
                traffic_max_results: 1000,
                traffic_default_window: "24h".into(),
            },
            None,
            WritesState::off("writes need --allow-writes"),
        )));
        assert!(rt.writer.is_none());
        assert!(!rt.model.writes.enabled);
        assert_eq!(
            rt.model.writes.reason.as_deref(),
            Some("writes need --allow-writes")
        );
    }
```

Append to `crates/illumio-tui/tests/tui_snapshots.rs`:

```rust
#[test]
fn header_shows_the_writes_marker_only_when_enabled() {
    use illumio_tui::tui::model::WritesState;
    let mut m = model_fixture();
    let first = |m: &illumio_tui::tui::Model| {
        render_to_string(m, 80, 24)
            .lines()
            .next()
            .unwrap_or("")
            .to_string()
    };
    assert!(!first(&m).contains("writes"), "{}", first(&m));
    m.writes = WritesState::on();
    assert!(first(&m).contains("● · ✎ writes"), "{}", first(&m));
    insta::assert_snapshot!("header_writes_dark_styled_80x24", render_styled(&m, 80, 24));
    m.set_theme("light");
    insta::assert_snapshot!("header_writes_light_styled_80x24", render_styled(&m, 80, 24));
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p illumio-tui --lib tui::app::tests`
Expected: FAIL, compile errors (`writer_from`, `WritesState`, `Runtime::with_writer`, `AUDIT_PATH_UNKNOWN` not found).

- [ ] **Step 3: Implement the model state** (`crates/illumio-tui/src/tui/model.rs`)

After `impl StatusLine { … }`:

```rust
/// Whether editing keys may start a write flow. The guard itself lives in
/// the runtime (`Runtime::writer`); the model never holds it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WritesState {
    /// Both switches on and the audit sink writable.
    pub enabled: bool,
    /// Why not, for the status line.
    pub reason: Option<String>,
}

impl WritesState {
    pub fn on() -> Self {
        Self {
            enabled: true,
            reason: None,
        }
    }

    pub fn off(reason: impl Into<String>) -> Self {
        Self {
            enabled: false,
            reason: Some(reason.into()),
        }
    }

    /// The status-line text for an editing key pressed while writes are off.
    pub fn refusal(&self) -> String {
        self.reason
            .clone()
            .unwrap_or_else(|| "writes are not enabled".into())
    }
}
```

In `pub struct Model`, after `pub status: StatusLine,`:

```rust
    /// Set at startup and on every profile switch from the runtime's guard.
    pub writes: WritesState,
```

`Model::new_connecting` gains the sixth parameter and stores it:

```rust
    pub fn new_connecting(
        profile: ProfileInfo,
        theme_name: String,
        auto_refresh_min_seconds: u64,
        traffic_max_results: u32,
        traffic_default_window: String,
        writes: WritesState,
    ) -> Self {
```

and in the `Self { … }` literal, after `status: StatusLine::info("connecting…"),`:

```rust
            writes,
```

- [ ] **Step 4: Update every caller of `new_connecting`**

`crates/illumio-tui/src/tui/test_support.rs`, `model_fixture`:

```rust
    let mut m = Model::new_connecting(
        profile,
        "dark".into(),
        60,
        1000,
        "24h".into(),
        crate::tui::model::WritesState::off("started without --allow-writes"),
    );
```

`crates/illumio-tui/src/tui/update.rs`, test `startup_connect_flow`:

```rust
        let mut m = Model::new_connecting(
            model_fixture().profile,
            model_fixture().theme_name.clone(),
            60,
            1000,
            "24h".into(),
            crate::tui::model::WritesState::default(),
        );
```

`crates/illumio-tui/tests/tui_runtime.rs`: change the import to `use illumio_tui::tui::model::{Connection, Model, ProfileInfo, WritesState};` and the helper:

```rust
fn model() -> Model {
    Model::new_connecting(
        ProfileInfo {
            name: "t".into(),
            host: "h".into(),
            org_id: 5,
        },
        "dark".into(),
        60,
        1000,
        "24h".into(),
        WritesState::default(),
    )
}
```

- [ ] **Step 5: Implement the runtime guard** (`crates/illumio-tui/src/tui/app.rs`)

Imports: change `use crate::tui::model::{Limits, Model, ProfileInfo};` to `use crate::tui::model::{Limits, Model, ProfileInfo, WritesState};` and add:

```rust
use pce::write::{JsonlAudit, WriteContext, WriteGuard};
use std::path::PathBuf;
```

Replace `SwitchResult`:

```rust
/// What a `:profile` switch built off the event loop: the new client, what
/// the model needs to know about it and the new profile's write guard, or
/// the error to show.
type SwitchResult = Result<(Pce, ProfileInfo, Limits, Option<WriteGuard>, WritesState), String>;
```

In `pub struct Runtime`, after `pub model: Model,`:

```rust
    /// The current profile's write guard; `None` while writes are off. Only
    /// the runtime holds it: `update` sees `model.writes` and nothing else.
    pub writer: Option<Arc<WriteGuard>>,
```

Replace `new`, `with_args` and `build`, and add `with_writer`:

```rust
    pub fn new(pce: Arc<Pce>, model: Model) -> (Self, UnboundedSender<Msg>) {
        Self::build(pce, model, None, None)
    }

    pub fn with_args(
        pce: Arc<Pce>,
        model: Model,
        global: GlobalArgs,
    ) -> (Self, UnboundedSender<Msg>) {
        Self::build(pce, model, Some(global), None)
    }

    /// A runtime that may write (tests; `run` builds the guard from config).
    pub fn with_writer(
        pce: Arc<Pce>,
        model: Model,
        writer: WriteGuard,
    ) -> (Self, UnboundedSender<Msg>) {
        Self::build(pce, model, None, Some(Arc::new(writer)))
    }

    fn build(
        pce: Arc<Pce>,
        model: Model,
        global: Option<GlobalArgs>,
        writer: Option<Arc<WriteGuard>>,
    ) -> (Self, UnboundedSender<Msg>) {
        let (tx, rx) = unbounded_channel();
        let (switch_tx, switch_rx) = unbounded_channel();
        (
            Self {
                pce,
                model,
                writer,
                global,
                generation: Arc::new(AtomicU64::new(0)),
                tx: tx.clone(),
                rx,
                switch_tx,
                switch_rx,
            },
            tx,
        )
    }
```

In `Cmd::SwitchProfile`, replace the `Context::new(&global).map(|ctx| { … })` closure body so it also builds the guard (`global.allow_writes` is kept, like the other global flags):

```rust
                    let result = Context::new(&global)
                        .map(|ctx| {
                            let info = ProfileInfo {
                                name: ctx.config.profile_name.clone(),
                                host: ctx.config.host.clone(),
                                org_id: ctx.config.org_id,
                            };
                            let limits = Limits {
                                auto_refresh_min_seconds: ctx.config.auto_refresh_min_seconds,
                                traffic_max_results: ctx.config.traffic_max_results,
                                traffic_default_window: ctx.config.traffic_default_window.clone(),
                            };
                            let (writer, writes) = build_writer(&ctx);
                            (ctx.pce, info, limits, writer, writes)
                        })
                        .map_err(|e| e.to_string());
```

In `finish_switch`, replace the `Ok` arm:

```rust
            Ok((pce, info, limits, writer, writes)) => {
                self.pce = Arc::new(pce);
                self.writer = writer.map(Arc::new);
                self.model.writes = writes;
                self.generation.fetch_add(1, Ordering::SeqCst);
                Msg::ProfileSwitched(Ok((info, limits)))
            }
```

In `run_loop`'s `std::mem::replace`, add the sixth argument `WritesState::default(),` after `"24h".into(),`.

In `run`, build the guard before the model and install it:

```rust
pub async fn run(global: GlobalArgs) -> Result<(), CliError> {
    let ctx = Context::new(&global)?;
    let (writer, writes) = build_writer(&ctx);
    let cfg = &ctx.config;
    let profile = ProfileInfo {
        name: cfg.profile_name.clone(),
        host: cfg.host.clone(),
        org_id: cfg.org_id,
    };
    let theme = global.theme.clone().unwrap_or_else(|| "dark".into());
    let model = Model::new_connecting(
        profile,
        theme,
        cfg.auto_refresh_min_seconds,
        cfg.traffic_max_results,
        cfg.traffic_default_window.clone(),
        writes,
    );
    let (mut rt, _tx) = Runtime::with_args(Arc::new(ctx.pce), model, global);
    rt.writer = writer.map(Arc::new);
    rt.execute(Cmd::Connect);
```

(the rest of `run` is unchanged). Add, above `to_msg`:

```rust
const AUDIT_PATH_UNKNOWN: &str =
    "audit log path unknown: no $XDG_STATE_HOME and no home directory";

/// The TUI's write guard for one profile, from the same inputs as the CLI's
/// (`cli/write.rs`, `write_guard`): the profile's `writes`, the
/// `--allow-writes` the TUI was started with, and the JSONL audit log, with
/// `source: "tui"`. Any refusal leaves writes off with its reason.
fn build_writer(ctx: &Context) -> (Option<WriteGuard>, WritesState) {
    writer_from(
        ctx.config.writes,
        ctx.allow_writes,
        JsonlAudit::default_path(),
        WriteContext {
            profile: ctx.config.profile_name.clone(),
            host: ctx.config.host.clone(),
            org_id: ctx.config.org_id,
            source: "tui".into(),
        },
    )
}

fn writer_from(
    profile_allows: bool,
    flag_allows: bool,
    audit_path: Option<PathBuf>,
    ctx: WriteContext,
) -> (Option<WriteGuard>, WritesState) {
    // Profile and flag first: a run without them never touches the audit path.
    if let Err(refused) = WriteGuard::check(profile_allows, flag_allows) {
        return (None, WritesState::off(refused.to_string()));
    }
    let Some(path) = audit_path else {
        return (None, WritesState::off(AUDIT_PATH_UNKNOWN));
    };
    match WriteGuard::new(
        profile_allows,
        flag_allows,
        Box::new(JsonlAudit::new(path)),
        ctx,
    ) {
        Ok(guard) => (Some(guard), WritesState::on()),
        Err(refused) => (None, WritesState::off(refused.to_string())),
    }
}
```

- [ ] **Step 6: Implement the header marker** (`crates/illumio-tui/src/tui/ui/mod.rs`, `render_header`)

Replace the `let left = Line::from(vec![ … ]);` statement:

```rust
    let mut spans = vec![
        Span::styled(" illumio-tui ", t.title_style()),
        Span::styled(
            format!("{} · org {} ", model.profile.host, model.profile.org_id),
            Style::new().fg(t.fg),
        ),
        conn,
    ];
    if model.writes.enabled {
        spans.push(Span::styled(" · ✎ writes", Style::new().fg(t.warn)));
    }
    let left = Line::from(spans);
```

- [ ] **Step 7: Run the tests, create the snapshots**

Run: `cargo test -p illumio-tui --lib tui::app::tests`
Expected: PASS (4 tests: the two existing, `writer_needs_both_switches_and_a_writable_audit_log`, `a_switch_without_a_guard_drops_it_and_clears_the_marker`).

Run: `INSTA_UPDATE=always cargo test -p illumio-tui --test tui_snapshots header_shows_the_writes_marker_only_when_enabled && git status --short crates/illumio-tui/tests/snapshots`
Expected: PASS; exactly two new files, `tui_snapshots__header_writes_dark_styled_80x24.snap` and `tui_snapshots__header_writes_light_styled_80x24.snap`. Read both: line 1 ends with `● · ✎ writes` and its run list shows the warn colour (`Rgb(229, 192, 123)` dark, `Rgb(176, 120, 0)` light) at the marker.

Run: `cargo test -p illumio-tui`
Expected: PASS, every existing snapshot and runtime test unchanged.

- [ ] **Step 8: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p illumio-tui
git add crates/illumio-tui/src/tui/model.rs crates/illumio-tui/src/tui/app.rs crates/illumio-tui/src/tui/ui/mod.rs crates/illumio-tui/src/tui/test_support.rs crates/illumio-tui/src/tui/update.rs crates/illumio-tui/tests/tui_runtime.rs crates/illumio-tui/tests/tui_snapshots.rs crates/illumio-tui/tests/snapshots
git commit -m "feat(tui): writes capability: runtime guard per profile and header marker

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---
### Task 3: Form overlay

**Files:**
- Create: `crates/illumio-tui/src/tui/form.rs`
- Create: `crates/illumio-tui/src/tui/edit.rs` (only `EditIntent` in this task; Task 4 completes the module)
- Modify: `crates/illumio-tui/src/tui/mod.rs` (`pub mod edit;`, `pub mod form;`)
- Modify: `crates/illumio-tui/src/tui/model.rs` (`Overlay` enum ~line 104: `Form(Form)`; `PartialEq for Overlay` ~line 138)
- Modify: `crates/illumio-tui/src/tui/update.rs` (`on_key` overlay match ~line 490: new `Overlay::Form` arm; imports)
- Modify: `crates/illumio-tui/src/tui/ui/mod.rs` (overlay dispatch ~line 40)
- Modify: `crates/illumio-tui/src/tui/ui/widgets/overlay.rs` (new `render_form` after `render_filter_editor` ~line 144)
- Test: unit tests in `form.rs` and `update.rs`; `crates/illumio-tui/tests/tui_snapshots.rs` (append)

**Interfaces:**
- Consumes: `pce::models::{Label, IpList, Service, Workload}` (all `Clone + PartialEq`, `models/labels.rs`, `models/workloads.rs`), `centered` (`ui/layout.rs`), `panel` (`ui/widgets/panel.rs`), `plain` (`ui/screens/workloads.rs` ~line 175, `pub(crate)`).
- Produces:

```rust
// tui/edit.rs
#[derive(Debug, Clone, PartialEq)]
pub enum EditIntent {
    CreateLabel, UpdateLabel(Label), DeleteLabel(Label),
    CreateIpList, UpdateIpList(IpList), DeleteIpList(IpList),
    CreateService, UpdateService(Service), DeleteService(Service),
    CreateWorkload, UpdateWorkload(Workload), DeleteWorkload(Workload),
}
// tui/form.rs
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field { pub label: &'static str, pub value: String, pub hint: &'static str, pub required: bool }
impl Field { pub fn new(label: &'static str, value: impl Into<String>, hint: &'static str, required: bool) -> Self; }
#[derive(Debug, Clone, PartialEq)]
pub struct Form { pub title: String, pub fields: Vec<Field>, pub active: usize, pub error: Option<String>, pub intent: EditIntent }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormEvent { Continue, Submit, Cancel }
impl Form {
    pub fn new(title: impl Into<String>, fields: Vec<Field>, intent: EditIntent) -> Self;
    pub fn value(&self, label: &str) -> &str;
    pub fn on_key(&mut self, k: KeyEvent) -> FormEvent;
}
pub fn visible_tail(value: &str, width: usize) -> String;
// tui/model.rs
pub enum Overlay { /* … */ Form(Form) }
// ui/widgets/overlay.rs
pub fn render_form(model: &Model, form: &Form, f: &mut Frame, area: Rect);
```

- [ ] **Step 1: Write the failing tests**

Create `crates/illumio-tui/src/tui/form.rs` with only its tests first (the module compiles once Step 3 adds the code above them):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::edit::EditIntent;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn k(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn label_form() -> Form {
        Form::new(
            "New label",
            vec![
                Field::new("Key", "", "app", true),
                Field::new("Value", "", "web", true),
                Field::new("Note", "", "optional", false),
            ],
            EditIntent::CreateLabel,
        )
    }

    #[test]
    fn tab_and_arrows_move_between_fields_and_wrap() {
        let mut f = label_form();
        assert_eq!(f.on_key(k(KeyCode::Tab)), FormEvent::Continue);
        assert_eq!(f.active, 1);
        f.on_key(k(KeyCode::Down));
        assert_eq!(f.active, 2);
        f.on_key(k(KeyCode::Tab));
        assert_eq!(f.active, 0, "wraps forward");
        f.on_key(k(KeyCode::BackTab));
        assert_eq!(f.active, 2, "wraps backward");
        f.on_key(k(KeyCode::Up));
        assert_eq!(f.active, 1);
    }

    #[test]
    fn typing_backspace_and_ctrl_u_edit_only_the_active_field() {
        let mut f = label_form();
        for c in "apq".chars() {
            f.on_key(k(KeyCode::Char(c)));
        }
        f.on_key(k(KeyCode::Backspace));
        f.on_key(k(KeyCode::Char('p')));
        assert_eq!(f.value("Key"), "app");
        assert_eq!(f.value("Value"), "");
        f.on_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL));
        assert_eq!(f.value("Key"), "app", "other control keys are not typed");
        f.on_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert_eq!(f.value("Key"), "", "Ctrl-U clears the field");
        f.on_key(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT));
        assert_eq!(f.value("Key"), "A", "shifted characters are typed");
        assert_eq!(f.value("Missing"), "");
    }

    #[test]
    fn enter_checks_required_fields_and_esc_cancels() {
        let mut f = label_form();
        f.on_key(k(KeyCode::Char('a')));
        f.on_key(k(KeyCode::Tab));
        f.on_key(k(KeyCode::Tab));
        assert_eq!(f.on_key(k(KeyCode::Enter)), FormEvent::Continue);
        assert_eq!(f.error.as_deref(), Some("Value is required"));
        assert_eq!(f.active, 1, "the empty required field takes the focus");
        f.on_key(k(KeyCode::Char(' ')));
        assert_eq!(
            f.on_key(k(KeyCode::Enter)),
            FormEvent::Continue,
            "blank is empty"
        );
        f.on_key(k(KeyCode::Char('w')));
        assert_eq!(f.on_key(k(KeyCode::Enter)), FormEvent::Submit);
        assert_eq!(f.error, None);
        assert_eq!(f.on_key(k(KeyCode::Esc)), FormEvent::Cancel);
    }

    #[test]
    fn visible_tail_keeps_the_end_where_the_cursor_is() {
        assert_eq!(visible_tail("10.0.0.0/8", 20), "10.0.0.0/8");
        assert_eq!(visible_tail("abcdefghij", 5), "…ghij");
        assert_eq!(visible_tail("ab", 0), "");
        assert_eq!(visible_tail("äöüß", 3), "…üß", "counts characters, not bytes");
    }
}
```

Append to the `#[cfg(test)] mod tests` in `crates/illumio-tui/src/tui/update.rs`:

```rust
    #[test]
    fn a_form_overlay_takes_every_key_and_esc_sends_nothing() {
        use crate::tui::edit::EditIntent;
        use crate::tui::form::{Field, Form};
        let mut m = model_fixture();
        m.overlay = Overlay::Form(Form::new(
            "New label",
            vec![
                Field::new("Key", "", "app", true),
                Field::new("Value", "", "web", true),
            ],
            EditIntent::CreateLabel,
        ));
        for c in "q2?".chars() {
            assert!(update(&mut m, key(c)).is_empty());
        }
        assert!(!m.quit, "q is typed, not a quit");
        assert_eq!(m.screen, Screen::Traffic);
        let Overlay::Form(f) = &m.overlay else {
            panic!("{:?}", m.overlay)
        };
        assert_eq!(f.value("Key"), "q2?");
        assert!(update(&mut m, kc(KeyCode::Esc)).is_empty());
        assert_eq!(m.overlay, Overlay::None);
        assert_eq!(m.status.text, "cancelled, nothing sent");
    }
```

Append to `crates/illumio-tui/tests/tui_snapshots.rs`:

```rust
#[test]
fn ip_list_form_80x24_scrolls_the_active_field() {
    use illumio_tui::tui::edit::EditIntent;
    use illumio_tui::tui::form::{Field, Form};
    use illumio_tui::tui::model::{Overlay, Screen};
    use illumio_tui::tui::test_support::model_with_objects;
    let mut m = model_with_objects();
    m.screen = Screen::Objects;
    let mut form = Form::new(
        "New IP list",
        vec![
            Field::new("Name", "Corp", "Corp", true),
            Field::new("Description", "Office ranges", "optional", false),
            Field::new(
                "Ranges",
                "10.1.0.0-10.1.255.255, 10.2.0.0/16, !10.1.5.0/24, 192.168.0.0/16, 172.16.0.0/12",
                "10.0.0.0/8, 10.1.0.0-10.1.255.255, !10.0.5.0/24",
                false,
            ),
            Field::new(
                "FQDNs",
                "*.corp.example",
                "*.corp.example, api.example.com",
                false,
            ),
        ],
        EditIntent::CreateIpList,
    );
    form.active = 2;
    m.overlay = Overlay::Form(form);
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("New IP list"), "{out}");
    assert!(out.contains("Name*"), "required fields are marked: {out}");
    assert!(
        out.contains("…") && out.contains("172.16.0.0/12▏"),
        "the end of the active field stays visible: {out}"
    );
    insta::assert_snapshot!("form_ip_list_80x24", out);
    m.set_theme("light");
    insta::assert_snapshot!("form_ip_list_light_styled_80x24", render_styled(&m, 80, 24));
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p illumio-tui --lib tui::form`
Expected: FAIL, compile error (`mod form` not declared / `Form`, `Field`, `EditIntent` not found).

- [ ] **Step 3: Implement `EditIntent`** (create `crates/illumio-tui/src/tui/edit.rs`)

```rust
//! Editing in the TUI. A key produces an [`EditIntent`]: a pure value that
//! names what the user wants, with the selected object cloned in.
use pce::models::{IpList, Label, Service, Workload};

/// What `n`, `e` or `x` on Objects or Workloads asks for.
#[derive(Debug, Clone, PartialEq)]
pub enum EditIntent {
    CreateLabel,
    UpdateLabel(Label),
    DeleteLabel(Label),
    CreateIpList,
    UpdateIpList(IpList),
    DeleteIpList(IpList),
    CreateService,
    UpdateService(Service),
    DeleteService(Service),
    CreateWorkload,
    UpdateWorkload(Workload),
    DeleteWorkload(Workload),
}
```

In `crates/illumio-tui/src/tui/mod.rs`, add `pub mod edit;` after `pub mod commands;` and `pub mod form;` after `pub mod filter;`.

- [ ] **Step 4: Implement the form** (top of `crates/illumio-tui/src/tui/form.rs`, above the tests)

```rust
//! One generic form for every editable kind, held in `Overlay::Form`. Pure:
//! keys change the form; `update` decides what a submit means. Fields are
//! single-line and the cursor is always at the end of the active field.
use crate::tui::edit::EditIntent;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// "Name", "Ranges".
    pub label: &'static str,
    pub value: String,
    /// Example input shown under the field.
    pub hint: &'static str,
    pub required: bool,
}

impl Field {
    pub fn new(
        label: &'static str,
        value: impl Into<String>,
        hint: &'static str,
        required: bool,
    ) -> Self {
        Self {
            label,
            value: value.into(),
            hint,
            required,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Form {
    /// "New IP list", "Edit service Web".
    pub title: String,
    pub fields: Vec<Field>,
    pub active: usize,
    /// The last validation failure, shown in red.
    pub error: Option<String>,
    /// What to build on Enter.
    pub intent: EditIntent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormEvent {
    Continue,
    /// Enter with every required field filled.
    Submit,
    /// Esc: close without a request.
    Cancel,
}

impl Form {
    pub fn new(title: impl Into<String>, fields: Vec<Field>, intent: EditIntent) -> Self {
        Self {
            title: title.into(),
            fields,
            active: 0,
            error: None,
            intent,
        }
    }

    /// The value of the field with this label ("" when there is none).
    pub fn value(&self, label: &str) -> &str {
        self.fields
            .iter()
            .find(|f| f.label == label)
            .map_or("", |f| f.value.as_str())
    }

    pub fn on_key(&mut self, k: KeyEvent) -> FormEvent {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let n = self.fields.len().max(1);
        match k.code {
            KeyCode::Esc => return FormEvent::Cancel,
            KeyCode::Enter => {
                if let Some(i) = self
                    .fields
                    .iter()
                    .position(|f| f.required && f.value.trim().is_empty())
                {
                    self.active = i;
                    self.error = Some(format!("{} is required", self.fields[i].label));
                    return FormEvent::Continue;
                }
                self.error = None;
                return FormEvent::Submit;
            }
            KeyCode::Tab | KeyCode::Down => self.active = (self.active + 1) % n,
            KeyCode::BackTab | KeyCode::Up => self.active = (self.active + n - 1) % n,
            KeyCode::Backspace => {
                if let Some(f) = self.fields.get_mut(self.active) {
                    f.value.pop();
                }
            }
            KeyCode::Char('u') if ctrl => {
                if let Some(f) = self.fields.get_mut(self.active) {
                    f.value.clear();
                }
            }
            KeyCode::Char(c) if !ctrl => {
                if let Some(f) = self.fields.get_mut(self.active) {
                    f.value.push(c);
                }
            }
            _ => {}
        }
        FormEvent::Continue
    }
}

/// The end of `value` that fits in `width` cells, with `…` in front when the
/// start is cut, so the cursor (always at the end) stays visible.
pub fn visible_tail(value: &str, width: usize) -> String {
    let n = value.chars().count();
    if n <= width {
        return value.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let tail: String = value.chars().skip(n - (width - 1)).collect();
    format!("…{tail}")
}
```

- [ ] **Step 5: Add the overlay** (`crates/illumio-tui/src/tui/model.rs`)

Add `use crate::tui::form::Form;` to the imports. In `pub enum Overlay`, after `EventDetail { … },`:

```rust
    /// Create or edit form for a label, IP list, service or workload.
    Form(Form),
```

In `impl PartialEq for Overlay`, before `_ => false,`:

```rust
            (Overlay::Form(a), Overlay::Form(b)) => a == b,
```

- [ ] **Step 6: Route keys to the form** (`crates/illumio-tui/src/tui/update.rs`)

Add `use crate::tui::form::FormEvent;` to the imports. In `on_key`, before the `Overlay::RuleDetail(_) | Overlay::Pending | Overlay::Error(_) => {` arm:

```rust
        Overlay::Form(mut form) => {
            match form.on_key(k) {
                FormEvent::Continue => model.overlay = Overlay::Form(form),
                // Submit is wired to planning in Task 5; until then it sends nothing.
                FormEvent::Cancel | FormEvent::Submit => {
                    model.overlay = Overlay::None;
                    model.status = StatusLine::info("cancelled, nothing sent");
                }
            }
            Vec::new()
        }
```

- [ ] **Step 7: Render the form** (`crates/illumio-tui/src/tui/ui/widgets/overlay.rs`)

Add `use crate::tui::form::{Form, visible_tail};` and `use crate::tui::ui::screens::workloads::plain;` to the imports, and after `render_filter_editor`:

```rust
/// Label column width inside the form (the `*` of a required field included).
const FORM_LABEL_WIDTH: usize = 14;

pub fn render_form(model: &Model, form: &Form, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    // Two rows per field (value, hint), the error row and the borders.
    let r = centered(area, 70, (form.fields.len() as u16) * 2 + 3);
    f.render_widget(Clear, r);
    let block = panel(&form.title, "Tab next · Enter review · Esc cancel", t, true);
    // Borders (2), the focus marker (2), the label column and the cursor (1).
    let width = (r.width as usize).saturating_sub(2 + 2 + FORM_LABEL_WIDTH + 1);
    let mut lines: Vec<Line> = Vec::new();
    for (i, field) in form.fields.iter().enumerate() {
        let on = i == form.active;
        let label = if field.required {
            format!("{}*", field.label)
        } else {
            field.label.to_string()
        };
        let shown = plain(&visible_tail(&field.value, width));
        lines.push(Line::from(vec![
            Span::styled(if on { "▸ " } else { "  " }, Style::new().fg(t.accent)),
            Span::styled(
                format!("{label:<w$}", w = FORM_LABEL_WIDTH),
                t.header_style(),
            ),
            Span::styled(
                format!("{shown}{}", if on { "▏" } else { "" }),
                if on {
                    Style::new().fg(t.fg).bg(t.selected_bg)
                } else {
                    t.base()
                },
            ),
        ]));
        lines.push(Line::from(Span::styled(
            format!("{:w$}{}", "", field.hint, w = 2 + FORM_LABEL_WIDTH),
            t.dim_style(),
        )));
    }
    lines.push(match &form.error {
        Some(e) => Line::from(Span::styled(plain(e), Style::new().fg(t.bad))),
        None => Line::from(Span::styled("* required", t.dim_style())),
    });
    f.render_widget(
        Paragraph::new(lines)
            .block(block)
            .style(t.base())
            .wrap(Wrap { trim: false }),
        r,
    );
}
```

In `crates/illumio-tui/src/tui/ui/mod.rs`, in the overlay `match`, after the `Overlay::EventDetail` arm:

```rust
        Overlay::Form(form) => widgets::overlay::render_form(model, form, f, area),
```

- [ ] **Step 8: Run the tests, create the snapshots**

Run: `cargo test -p illumio-tui --lib tui::form && cargo test -p illumio-tui --lib a_form_overlay_takes_every_key`
Expected: PASS (4 form tests, 1 update test).

Run: `INSTA_UPDATE=always cargo test -p illumio-tui --test tui_snapshots ip_list_form_80x24 && git status --short crates/illumio-tui/tests/snapshots`
Expected: PASS; exactly two new files, `tui_snapshots__form_ip_list_80x24.snap` and `tui_snapshots__form_ip_list_light_styled_80x24.snap`. Read the dark one: a 70-wide box titled `New IP list`, four fields with hints, the Ranges line starting with `…` and ending in `172.16.0.0/12▏`, `* required` on the last row.

- [ ] **Step 9: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p illumio-tui
git add crates/illumio-tui/src/tui crates/illumio-tui/tests/tui_snapshots.rs crates/illumio-tui/tests/snapshots
git commit -m "feat(tui): form overlay with field navigation, editing keys and horizontal scroll

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 4: Edit intents and form adapters

**Files:**
- Modify: `crates/illumio-tui/src/tui/edit.rs` (replace the whole file; `EditIntent` is unchanged)
- Test: unit tests in `edit.rs`

**Interfaces:**
- Consumes: `Form`, `Field` (Task 3); `LabelDraft::{new, validate}` (`pce/src/write/labels.rs`); `IpListDraft { name, description, ranges: Vec<IpRangeSpec>, fqdns: Vec<Fqdn> }`, `IpListPatch { name, description, add_ranges, remove_ranges, add_fqdns: Vec<String>, remove_fqdns: Vec<String> }` with `validate`/`is_empty` (`write/ip_lists.rs`); `ServiceDraft { name, description, ports: Vec<PortSpec> }`, `ServicePatch { name, description, add_ports, remove_ports }` (`write/services.rs`); `WorkloadDraft { name, hostname, description, interfaces: Vec<InterfaceSpec>, labels: Vec<String> }`, `WorkloadPatch { name, hostname, description, add_addresses: Vec<String>, remove_addresses: Vec<String>, set_labels: Option<Vec<String>>, add_labels, remove_labels }` (`write/workloads.rs`); `IpRangeSpec::{parse, from_range, display, same_range}`, `PortSpec::{parse, from_service_port}` (`PartialEq`), `InterfaceSpec::parse(s, name)` (`write/specs.rs`); `ServicePort::display` (`models/labels.rs` ~line 150), `Fqdn::new` (`models/labels.rs` ~line 64), `Label::display`, `Workload::display_name` (`models/workloads.rs` ~line 176); `NameIndex::{label, label_href, values_for_key}` (`pce/src/resolve.rs` ~lines 72-97); `fixture_index` (`test_support.rs`).
- Produces:

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum WriteIntent {
    CreateLabel(LabelDraft), UpdateLabel { href: String, value: String }, DeleteLabel { href: String, name: String },
    CreateIpList(IpListDraft), UpdateIpList { href: String, patch: IpListPatch }, DeleteIpList { href: String, name: String },
    CreateService(ServiceDraft), UpdateService { href: String, patch: ServicePatch }, DeleteService { href: String, name: String },
    CreateWorkload(WorkloadDraft), UpdateWorkload { href: String, patch: WorkloadPatch }, DeleteWorkload { href: String, name: String },
}
impl WriteIntent { pub fn kind(&self) -> pce::write::ObjectKind; }
#[derive(Debug)]
pub enum Start { Form(Form), Plan(WriteIntent) }
pub fn start(intent: EditIntent, index: Option<&NameIndex>) -> Start;
pub fn form_for(intent: EditIntent, index: Option<&NameIndex>) -> Option<Form>;
pub fn build_intent(form: &Form, index: Option<&NameIndex>) -> Result<Option<WriteIntent>, String>;  // Ok(None): no changes
```

- [ ] **Step 1: Write the failing tests** (the `#[cfg(test)] mod tests` at the end of the new `edit.rs`; Step 3 writes the code above it)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::test_support::fixture_index;

    fn labels() -> Vec<Label> {
        serde_json::from_str(include_str!("../../../../fixtures/labels.json")).unwrap()
    }
    fn ip_lists() -> Vec<IpList> {
        serde_json::from_str(include_str!("../../../../fixtures/ip_lists.json")).unwrap()
    }
    fn services() -> Vec<Service> {
        serde_json::from_str(include_str!("../../../../fixtures/services.json")).unwrap()
    }
    fn workloads() -> Vec<Workload> {
        serde_json::from_str(include_str!("../../../../fixtures/workloads.json")).unwrap()
    }

    fn form(intent: EditIntent) -> Form {
        form_for(intent, Some(&*fixture_index())).expect("a create or update intent")
    }

    fn set(f: &mut Form, label: &str, value: &str) {
        f.fields
            .iter_mut()
            .find(|x| x.label == label)
            .unwrap_or_else(|| panic!("no field {label}"))
            .value = value.into();
    }

    fn build(f: &Form) -> Result<Option<WriteIntent>, String> {
        build_intent(f, Some(&*fixture_index()))
    }

    #[test]
    fn label_create_and_value_edit() {
        let mut f = form(EditIntent::CreateLabel);
        assert_eq!(
            f.fields.iter().map(|x| x.label).collect::<Vec<_>>(),
            vec!["Key", "Value"]
        );
        set(&mut f, "Key", " app ");
        set(&mut f, "Value", "web2");
        assert_eq!(
            build(&f).unwrap(),
            Some(WriteIntent::CreateLabel(LabelDraft::new("app", "web2")))
        );
        let mut f = form(EditIntent::UpdateLabel(labels()[0].clone())); // role=web
        assert_eq!(f.title, "Edit label role=web");
        assert_eq!(f.fields.len(), 1, "the key is not editable");
        assert_eq!(build(&f).unwrap(), None, "unchanged");
        set(&mut f, "Value", "www");
        assert_eq!(
            build(&f).unwrap(),
            Some(WriteIntent::UpdateLabel {
                href: "/orgs/5/labels/1".into(),
                value: "www".into()
            })
        );
    }

    #[test]
    fn ip_list_form_prefills_and_create_keeps_exclusions() {
        let f = form(EditIntent::UpdateIpList(ip_lists()[1].clone()));
        assert_eq!(f.title, "Edit IP list Corp");
        assert_eq!(f.value("Name"), "Corp");
        assert_eq!(f.value("Description"), "Office ranges");
        assert_eq!(f.value("Ranges"), "10.1.0.0-10.1.255.255");
        assert_eq!(f.value("FQDNs"), "*.corp.example");
        let mut f = form(EditIntent::CreateIpList);
        set(&mut f, "Name", "Lab");
        set(&mut f, "Ranges", "10.0.0.0/8, !10.0.5.0/24");
        set(&mut f, "FQDNs", "*.lab.example");
        let Some(WriteIntent::CreateIpList(d)) = build(&f).unwrap() else {
            panic!("not a create")
        };
        assert_eq!(d.name, "Lab");
        assert_eq!(d.description, None);
        assert_eq!(
            d.ranges,
            vec![
                IpRangeSpec::parse("10.0.0.0/8", false).unwrap(),
                IpRangeSpec::parse("10.0.5.0/24", true).unwrap(),
            ]
        );
        assert_eq!(d.fqdns, vec![Fqdn::new("*.lab.example")]);
        set(&mut f, "Ranges", "10.0.0.300");
        let err = build(&f).unwrap_err();
        assert!(err.starts_with("Ranges:"), "{err}");
    }

    #[test]
    fn ip_list_edit_adds_and_removes_only_what_changed() {
        let corp = ip_lists()[1].clone();
        let mut f = form(EditIntent::UpdateIpList(corp.clone()));
        assert_eq!(build(&f).unwrap(), None, "a pre-filled form changes nothing");
        set(&mut f, "Description", "Office");
        set(&mut f, "Ranges", "10.2.0.0/16");
        set(&mut f, "FQDNs", "*.corp.example, api.example.com");
        assert_eq!(
            build(&f).unwrap(),
            Some(WriteIntent::UpdateIpList {
                href: "/orgs/5/sec_policy/draft/ip_lists/21".into(),
                patch: IpListPatch {
                    name: None,
                    description: Some("Office".into()),
                    add_ranges: vec![IpRangeSpec::parse("10.2.0.0/16", false).unwrap()],
                    remove_ranges: vec![IpRangeSpec::from_range(&corp.ip_ranges[0])],
                    add_fqdns: vec!["api.example.com".into()],
                    remove_fqdns: vec![],
                },
            })
        );
    }

    #[test]
    fn service_bare_port_is_refused_and_edits_diff_by_port() {
        let mut f = form(EditIntent::CreateService);
        set(&mut f, "Name", "SSH");
        set(&mut f, "Ports", "22");
        let err = build(&f).unwrap_err();
        assert!(
            err.contains("a bare number is ambiguous") && err.contains("22/tcp"),
            "{err}"
        );
        set(&mut f, "Ports", "22/tcp, icmp");
        let Some(WriteIntent::CreateService(d)) = build(&f).unwrap() else {
            panic!("not a create")
        };
        assert_eq!(
            d.ports,
            vec![
                PortSpec::parse("22/tcp").unwrap(),
                PortSpec::parse("icmp").unwrap()
            ]
        );
        let mut f = form(EditIntent::UpdateService(services()[2].clone())); // Web
        assert_eq!(f.value("Ports"), "80/tcp, 443/tcp, 8000-8099/tcp");
        assert_eq!(build(&f).unwrap(), None);
        set(&mut f, "Ports", "443/TCP, 8000-8099/tcp, 53/udp");
        let Some(WriteIntent::UpdateService { href, patch }) = build(&f).unwrap() else {
            panic!("not an update")
        };
        assert_eq!(href, "/orgs/5/sec_policy/draft/services/32");
        assert_eq!(patch.add_ports, vec![PortSpec::parse("53/udp").unwrap()]);
        assert_eq!(patch.remove_ports, vec![PortSpec::parse("80/tcp").unwrap()]);
        // `any` cannot be parsed back, but spelled as shown it is kept.
        let all = form(EditIntent::UpdateService(services()[0].clone()));
        assert_eq!(all.value("Ports"), "any");
        assert_eq!(build(&all).unwrap(), None);
    }

    #[test]
    fn workload_create_resolves_labels_through_the_index() {
        let mut f = form(EditIntent::CreateWorkload);
        set(&mut f, "Name", "printer-1");
        set(&mut f, "Addresses", "10.0.0.50/24, 10.0.0.51");
        set(&mut f, "Labels", "app=shop, env=prod");
        let Some(WriteIntent::CreateWorkload(d)) = build(&f).unwrap() else {
            panic!("not a create")
        };
        assert_eq!(
            d.interfaces,
            vec![
                InterfaceSpec::parse("10.0.0.50/24", "eth0".into()).unwrap(),
                InterfaceSpec::parse("10.0.0.51", "eth1".into()).unwrap(),
            ]
        );
        assert_eq!(d.labels, vec!["/orgs/5/labels/3", "/orgs/5/labels/4"]);
        assert_eq!(d.hostname, None);
        let host1 = form(EditIntent::UpdateWorkload(workloads()[0].clone()));
        assert_eq!(host1.value("Labels"), "role=web, app=shop, env=prod");
        assert_eq!(host1.value("Addresses"), "10.0.1.10/24");
    }

    #[test]
    fn unknown_labels_name_the_known_values_and_a_missing_index_refuses() {
        let mut f = form(EditIntent::CreateWorkload);
        set(&mut f, "Name", "printer-1");
        set(&mut f, "Labels", "app=nope");
        let err = build(&f).unwrap_err();
        assert!(err.contains("known values for app: shop"), "{err}");
        set(&mut f, "Labels", "tier=1");
        let err = build(&f).unwrap_err();
        assert!(err.contains("no labels have key tier"), "{err}");
        set(&mut f, "Labels", "app=shop");
        let err = build_intent(&f, None).unwrap_err();
        assert!(
            err.contains("label index not loaded, press r on Objects"),
            "{err}"
        );
        set(&mut f, "Labels", "");
        assert!(
            build_intent(&f, None).unwrap().is_some(),
            "no labels need no index"
        );
    }

    #[test]
    fn workload_edit_diffs_addresses_and_sets_the_full_label_list() {
        let nas = workloads()[2].clone(); // unmanaged-nas, eth0 10.0.3.30, no labels
        let mut f = form(EditIntent::UpdateWorkload(nas));
        assert_eq!(f.title, "Edit workload unmanaged-nas");
        assert_eq!(f.value("Addresses"), "10.0.3.30");
        assert_eq!(f.value("Labels"), "");
        assert_eq!(build(&f).unwrap(), None);
        set(&mut f, "Addresses", "10.0.3.30, 10.0.3.31/24");
        set(&mut f, "Labels", "env=prod");
        assert_eq!(
            build(&f).unwrap(),
            Some(WriteIntent::UpdateWorkload {
                href: "/orgs/5/workloads/aaaa-3".into(),
                patch: WorkloadPatch {
                    add_addresses: vec!["10.0.3.31/24".into()],
                    set_labels: Some(vec!["/orgs/5/labels/4".into()]),
                    ..WorkloadPatch::default()
                },
            })
        );
        set(&mut f, "Addresses", "10.0.3.31");
        set(&mut f, "Labels", "");
        let Some(WriteIntent::UpdateWorkload { patch, .. }) = build(&f).unwrap() else {
            panic!("not an update")
        };
        assert_eq!(patch.remove_addresses, vec!["10.0.3.30"]);
        assert_eq!(patch.add_addresses, vec!["10.0.3.31"]);
        assert_eq!(patch.set_labels, None, "labels unchanged");
    }

    #[test]
    fn deletes_skip_the_form_and_carry_the_name() {
        let Start::Plan(w) = start(EditIntent::DeleteIpList(ip_lists()[1].clone()), None) else {
            panic!("a delete opens no form")
        };
        assert_eq!(
            w,
            WriteIntent::DeleteIpList {
                href: "/orgs/5/sec_policy/draft/ip_lists/21".into(),
                name: "Corp".into()
            }
        );
        assert_eq!(w.kind(), pce::write::ObjectKind::IpList);
        let Start::Plan(w) = start(EditIntent::DeleteLabel(labels()[0].clone()), None) else {
            panic!("a delete opens no form")
        };
        assert_eq!(
            w,
            WriteIntent::DeleteLabel {
                href: "/orgs/5/labels/1".into(),
                name: "role=web".into()
            }
        );
        assert!(form_for(EditIntent::DeleteWorkload(workloads()[2].clone()), None).is_none());
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p illumio-tui --lib tui::edit`
Expected: FAIL, compile errors (`WriteIntent`, `form_for`, `start`, `build_intent` not found).

- [ ] **Step 3: Implement** (replace everything above the tests in `crates/illumio-tui/src/tui/edit.rs`)

```rust
//! Editing in the TUI. A key produces an [`EditIntent`]; [`start`] turns it
//! into a pre-filled [`Form`] (create, update) or straight into a
//! [`WriteIntent`] (delete); [`build_intent`] turns a submitted form into the
//! draft or patch the library plans. Pure: nothing here reaches the PCE.
use crate::tui::form::{Field, Form};
use pce::NameIndex;
use pce::models::{Fqdn, IpList, Label, Service, Workload};
use pce::write::{
    InterfaceSpec, IpListDraft, IpListPatch, IpRangeSpec, LabelDraft, PortSpec, ServiceDraft,
    ServicePatch, WorkloadDraft, WorkloadPatch,
};

/// What `n`, `e` or `x` on Objects or Workloads asks for.
#[derive(Debug, Clone, PartialEq)]
pub enum EditIntent {
    CreateLabel,
    UpdateLabel(Label),
    DeleteLabel(Label),
    CreateIpList,
    UpdateIpList(IpList),
    DeleteIpList(IpList),
    CreateService,
    UpdateService(Service),
    DeleteService(Service),
    CreateWorkload,
    UpdateWorkload(Workload),
    DeleteWorkload(Workload),
}

/// What `Cmd::PlanWrite` carries: the library draft or patch, or the href
/// (and, for the confirm text, the name) of the object to delete.
#[derive(Debug, Clone, PartialEq)]
pub enum WriteIntent {
    CreateLabel(LabelDraft),
    UpdateLabel { href: String, value: String },
    DeleteLabel { href: String, name: String },
    CreateIpList(IpListDraft),
    UpdateIpList { href: String, patch: IpListPatch },
    DeleteIpList { href: String, name: String },
    CreateService(ServiceDraft),
    UpdateService { href: String, patch: ServicePatch },
    DeleteService { href: String, name: String },
    CreateWorkload(WorkloadDraft),
    UpdateWorkload { href: String, patch: WorkloadPatch },
    DeleteWorkload { href: String, name: String },
}

impl WriteIntent {
    pub fn kind(&self) -> pce::write::ObjectKind {
        match self {
            WriteIntent::CreateLabel(_)
            | WriteIntent::UpdateLabel { .. }
            | WriteIntent::DeleteLabel { .. } => pce::write::ObjectKind::Label,
            WriteIntent::CreateIpList(_)
            | WriteIntent::UpdateIpList { .. }
            | WriteIntent::DeleteIpList { .. } => pce::write::ObjectKind::IpList,
            WriteIntent::CreateService(_)
            | WriteIntent::UpdateService { .. }
            | WriteIntent::DeleteService { .. } => pce::write::ObjectKind::Service,
            WriteIntent::CreateWorkload(_)
            | WriteIntent::UpdateWorkload { .. }
            | WriteIntent::DeleteWorkload { .. } => pce::write::ObjectKind::Workload,
        }
    }
}

/// Where an editing key leads: a form to fill, or (delete) a plan at once.
#[derive(Debug)]
pub enum Start {
    Form(Form),
    Plan(WriteIntent),
}

const RANGES_HINT: &str = "10.0.0.0/8, 10.1.0.0-10.1.255.255, !10.0.5.0/24";
const FQDNS_HINT: &str = "*.corp.example, api.example.com";
const PORTS_HINT: &str = "443/tcp, 8000-8100/udp, icmp";
const ADDRESSES_HINT: &str = "10.0.0.5, 10.0.0.6/24";
const LABELS_HINT: &str = "app=web, env=prod";

/// The form for a create or update (pre-filled from the object; the key of a
/// label and the href of anything are never editable), or the delete intent.
pub fn start(intent: EditIntent, index: Option<&NameIndex>) -> Start {
    let form = |title: String, fields: Vec<Field>, intent: EditIntent| {
        Start::Form(Form::new(title, fields, intent))
    };
    match intent {
        EditIntent::CreateLabel => form(
            "New label".into(),
            vec![
                Field::new("Key", "", "app, env, role, loc or your own", true),
                Field::new("Value", "", "web", true),
            ],
            EditIntent::CreateLabel,
        ),
        EditIntent::UpdateLabel(l) => {
            let title = format!("Edit label {}", l.display());
            let fields = vec![Field::new("Value", l.value.clone(), "web", true)];
            form(title, fields, EditIntent::UpdateLabel(l))
        }
        EditIntent::DeleteLabel(l) => Start::Plan(WriteIntent::DeleteLabel {
            name: l.display(),
            href: l.href,
        }),
        EditIntent::CreateIpList => form(
            "New IP list".into(),
            ip_list_fields(None),
            EditIntent::CreateIpList,
        ),
        EditIntent::UpdateIpList(l) => {
            let title = format!("Edit IP list {}", l.name);
            let fields = ip_list_fields(Some(&l));
            form(title, fields, EditIntent::UpdateIpList(l))
        }
        EditIntent::DeleteIpList(l) => Start::Plan(WriteIntent::DeleteIpList {
            name: l.name,
            href: l.href,
        }),
        EditIntent::CreateService => form(
            "New service".into(),
            service_fields(None),
            EditIntent::CreateService,
        ),
        EditIntent::UpdateService(s) => {
            let title = format!("Edit service {}", s.name);
            let fields = service_fields(Some(&s));
            form(title, fields, EditIntent::UpdateService(s))
        }
        EditIntent::DeleteService(s) => Start::Plan(WriteIntent::DeleteService {
            name: s.name,
            href: s.href,
        }),
        EditIntent::CreateWorkload => form(
            "New workload".into(),
            workload_fields(None, index),
            EditIntent::CreateWorkload,
        ),
        EditIntent::UpdateWorkload(w) => {
            let title = format!("Edit workload {}", w.display_name());
            let fields = workload_fields(Some(&w), index);
            form(title, fields, EditIntent::UpdateWorkload(w))
        }
        EditIntent::DeleteWorkload(w) => Start::Plan(WriteIntent::DeleteWorkload {
            name: w.display_name(),
            href: w.href,
        }),
    }
}

/// The form a create or update intent opens; `None` for a delete.
pub fn form_for(intent: EditIntent, index: Option<&NameIndex>) -> Option<Form> {
    match start(intent, index) {
        Start::Form(f) => Some(f),
        Start::Plan(_) => None,
    }
}

fn ip_list_fields(l: Option<&IpList>) -> Vec<Field> {
    vec![
        Field::new("Name", l.map(|l| l.name.clone()).unwrap_or_default(), "Corp", true),
        Field::new(
            "Description",
            l.and_then(|l| l.description.clone()).unwrap_or_default(),
            "optional",
            false,
        ),
        Field::new("Ranges", l.map(ranges_text).unwrap_or_default(), RANGES_HINT, false),
        Field::new("FQDNs", l.map(fqdns_text).unwrap_or_default(), FQDNS_HINT, false),
    ]
}

fn service_fields(s: Option<&Service>) -> Vec<Field> {
    vec![
        Field::new("Name", s.map(|s| s.name.clone()).unwrap_or_default(), "Web", true),
        Field::new(
            "Description",
            s.and_then(|s| s.description.clone()).unwrap_or_default(),
            "optional",
            false,
        ),
        Field::new("Ports", s.map(ports_text).unwrap_or_default(), PORTS_HINT, true),
    ]
}

fn workload_fields(w: Option<&Workload>, index: Option<&NameIndex>) -> Vec<Field> {
    let text = |f: fn(&Workload) -> Option<String>| w.and_then(f).unwrap_or_default();
    vec![
        // A workload may have no name (hostname only); only a new one needs it.
        Field::new("Name", text(|w| w.name.clone()), "printer-1", w.is_none()),
        Field::new(
            "Hostname",
            text(|w| w.hostname.clone()),
            "defaults to the name",
            false,
        ),
        Field::new(
            "Description",
            text(|w| w.description.clone()),
            "optional",
            false,
        ),
        Field::new(
            "Addresses",
            w.map(addresses_text).unwrap_or_default(),
            ADDRESSES_HINT,
            false,
        ),
        Field::new(
            "Labels",
            w.map(|w| labels_text(w, index)).unwrap_or_default(),
            LABELS_HINT,
            false,
        ),
    ]
}

fn join(items: impl Iterator<Item = String>) -> String {
    items.collect::<Vec<_>>().join(", ")
}

fn ranges_text(l: &IpList) -> String {
    join(l.ip_ranges.iter().map(|r| IpRangeSpec::from_range(r).display()))
}

fn fqdns_text(l: &IpList) -> String {
    join(l.fqdns.iter().map(|f| f.fqdn.clone()))
}

fn ports_text(s: &Service) -> String {
    join(s.service_ports.iter().map(|p| p.display()))
}

fn addresses_text(w: &Workload) -> String {
    join(w.interfaces.iter().map(|i| match i.cidr_block {
        Some(c) => format!("{}/{c}", i.address),
        None => i.address.clone(),
    }))
}

/// `key=value` from the workload's own label entry, else from the index,
/// else the href (which `resolve_labels` accepts back as it is).
fn labels_text(w: &Workload, index: Option<&NameIndex>) -> String {
    join(w.labels.iter().map(|l| match (&l.key, &l.value) {
        (Some(k), Some(v)) => format!("{k}={v}"),
        _ => index
            .and_then(|i| i.label(&l.href))
            .map(Label::display)
            .unwrap_or_else(|| l.href.clone()),
    }))
}

/// Comma-separated entries, trimmed, empty ones dropped.
fn items(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

fn non_empty(s: String) -> Option<String> {
    (!s.is_empty()).then_some(s)
}

/// `Some(new)` when a text field no longer equals the original.
fn changed(old: &str, new: String) -> Option<String> {
    (new != old.trim()).then_some(new)
}

/// Ranges in the CLI grammar (`!` marks an exclusion). An entry spelled
/// exactly as an existing range keeps that range, description and unknown
/// fields included.
fn parse_ranges(text: &str, existing: &[IpRangeSpec]) -> Result<Vec<IpRangeSpec>, String> {
    items(text)
        .iter()
        .map(|t| {
            if let Some(e) = existing.iter().find(|e| e.display() == *t) {
                return Ok(e.clone());
            }
            let (exclusion, body) = match t.strip_prefix('!') {
                Some(rest) => (true, rest.trim()),
                None => (false, t.as_str()),
            };
            IpRangeSpec::parse(body, exclusion).map_err(|e| format!("Ranges: {e}"))
        })
        .collect()
}

/// Ports added and removed by the edited list. An entry spelled exactly as
/// an existing port keeps it, so ports this tool cannot parse back (`any`,
/// ICMP types) survive an edit that does not touch them.
fn port_changes(s: &Service, tokens: &[String]) -> Result<(Vec<PortSpec>, Vec<PortSpec>), String> {
    let existing: Vec<Option<PortSpec>> = s
        .service_ports
        .iter()
        .map(PortSpec::from_service_port)
        .collect();
    let mut kept: Vec<PortSpec> = Vec::new();
    let mut add: Vec<PortSpec> = Vec::new();
    for t in tokens {
        if let Some(i) = s.service_ports.iter().position(|p| p.display() == *t) {
            if let Some(spec) = &existing[i] {
                kept.push(spec.clone());
            }
            continue;
        }
        let spec = PortSpec::parse(t).map_err(|e| format!("Ports: {e}"))?;
        if existing.iter().flatten().any(|o| *o == spec) {
            kept.push(spec);
        } else if !add.contains(&spec) {
            add.push(spec);
        }
    }
    let mut remove = Vec::new();
    for (p, spec) in s.service_ports.iter().zip(&existing) {
        if tokens.contains(&p.display()) {
            continue;
        }
        match spec {
            Some(spec) if !kept.contains(spec) => remove.push(spec.clone()),
            Some(_) => {}
            None => {
                return Err(format!(
                    "Ports: {} has no protocol and cannot be removed here; edit it in the PCE UI",
                    p.display()
                ));
            }
        }
    }
    Ok((add, remove))
}

/// Addresses added (as typed) and removed (by address). A typed address
/// that exists keeps its interface; with a new prefix it is an add, which
/// `WorkloadPatch::apply` turns into a prefix change of that interface.
fn address_changes(w: &Workload, tokens: &[String]) -> Result<(Vec<String>, Vec<String>), String> {
    let mut parsed = Vec::new();
    for t in tokens {
        let spec =
            InterfaceSpec::parse(t, String::new()).map_err(|e| format!("Addresses: {e}"))?;
        parsed.push((t.clone(), spec));
    }
    let add = parsed
        .iter()
        .filter(|(_, s)| {
            !w.interfaces.iter().any(|i| {
                i.address == s.address && (s.cidr_block.is_none() || s.cidr_block == i.cidr_block)
            })
        })
        .map(|(t, _)| t.clone())
        .collect();
    let remove = w
        .interfaces
        .iter()
        .filter(|i| !parsed.iter().any(|(_, s)| s.address == i.address))
        .map(|i| i.address.clone())
        .collect();
    Ok((add, remove))
}

/// `key=value` pairs (or hrefs) to label hrefs through the loaded index. No
/// request is ever made to resolve a label.
fn resolve_labels(text: &str, index: Option<&NameIndex>) -> Result<Vec<String>, String> {
    let wanted = items(text);
    if wanted.is_empty() {
        return Ok(Vec::new());
    }
    let Some(idx) = index else {
        return Err("Labels: label index not loaded, press r on Objects".into());
    };
    wanted
        .iter()
        .map(|t| {
            if t.starts_with("/orgs/") {
                return Ok(t.clone());
            }
            let (k, v) = t
                .split_once('=')
                .ok_or_else(|| format!("Labels: {t:?} is not key=value"))?;
            let (k, v) = (k.trim(), v.trim());
            idx.label_href(k, v).map(str::to_string).ok_or_else(|| {
                let known = idx.values_for_key(k);
                if known.is_empty() {
                    format!("Labels: unknown label {k}={v}; no labels have key {k}")
                } else {
                    format!(
                        "Labels: unknown label {k}={v}; known values for {k}: {}",
                        known.join(", ")
                    )
                }
            })
        })
        .collect()
}

/// The submitted form as a write: `Ok(None)` when an edit changes nothing,
/// `Err` (shown in the form) when a field does not parse or validate.
pub fn build_intent(form: &Form, index: Option<&NameIndex>) -> Result<Option<WriteIntent>, String> {
    let v = |label: &str| form.value(label).trim().to_string();
    let invalid = |e: Vec<String>| e.join("; ");
    match &form.intent {
        EditIntent::CreateLabel => {
            let d = LabelDraft::new(v("Key"), v("Value"));
            d.validate().map_err(invalid)?;
            Ok(Some(WriteIntent::CreateLabel(d)))
        }
        EditIntent::UpdateLabel(l) => {
            let value = v("Value");
            if value == l.value.trim() {
                return Ok(None);
            }
            if value.is_empty() {
                return Err("label value must not be empty".into());
            }
            Ok(Some(WriteIntent::UpdateLabel {
                href: l.href.clone(),
                value,
            }))
        }
        EditIntent::CreateIpList => {
            let d = IpListDraft {
                name: v("Name"),
                description: non_empty(v("Description")),
                ranges: parse_ranges(&v("Ranges"), &[])?,
                fqdns: items(&v("FQDNs")).iter().map(Fqdn::new).collect(),
            };
            d.validate().map_err(invalid)?;
            Ok(Some(WriteIntent::CreateIpList(d)))
        }
        EditIntent::UpdateIpList(l) => {
            let old: Vec<IpRangeSpec> = l.ip_ranges.iter().map(IpRangeSpec::from_range).collect();
            let new = parse_ranges(&v("Ranges"), &old)?;
            let old_fqdns: Vec<String> = l.fqdns.iter().map(|f| f.fqdn.clone()).collect();
            let new_fqdns = items(&v("FQDNs"));
            let patch = IpListPatch {
                name: changed(&l.name, v("Name")),
                description: changed(l.description.as_deref().unwrap_or(""), v("Description")),
                add_ranges: new
                    .iter()
                    .filter(|n| !old.iter().any(|o| o.same_range(n)))
                    .cloned()
                    .collect(),
                remove_ranges: old
                    .iter()
                    .filter(|o| !new.iter().any(|n| n.same_range(o)))
                    .cloned()
                    .collect(),
                add_fqdns: new_fqdns
                    .iter()
                    .filter(|f| !old_fqdns.contains(f))
                    .cloned()
                    .collect(),
                remove_fqdns: old_fqdns
                    .iter()
                    .filter(|f| !new_fqdns.contains(f))
                    .cloned()
                    .collect(),
            };
            if patch.is_empty() {
                Ok(None)
            } else {
                Ok(Some(WriteIntent::UpdateIpList {
                    href: l.href.clone(),
                    patch,
                }))
            }
        }
        EditIntent::CreateService => {
            let ports = items(&v("Ports"))
                .iter()
                .map(|p| PortSpec::parse(p).map_err(|e| format!("Ports: {e}")))
                .collect::<Result<Vec<_>, _>>()?;
            let d = ServiceDraft {
                name: v("Name"),
                description: non_empty(v("Description")),
                ports,
            };
            d.validate().map_err(invalid)?;
            Ok(Some(WriteIntent::CreateService(d)))
        }
        EditIntent::UpdateService(s) => {
            let (add_ports, remove_ports) = port_changes(s, &items(&v("Ports")))?;
            let patch = ServicePatch {
                name: changed(&s.name, v("Name")),
                description: changed(s.description.as_deref().unwrap_or(""), v("Description")),
                add_ports,
                remove_ports,
            };
            if patch.is_empty() {
                Ok(None)
            } else {
                Ok(Some(WriteIntent::UpdateService {
                    href: s.href.clone(),
                    patch,
                }))
            }
        }
        EditIntent::CreateWorkload => {
            let interfaces = items(&v("Addresses"))
                .iter()
                .enumerate()
                .map(|(i, a)| {
                    InterfaceSpec::parse(a, format!("eth{i}")).map_err(|e| format!("Addresses: {e}"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let d = WorkloadDraft {
                name: v("Name"),
                hostname: non_empty(v("Hostname")),
                description: non_empty(v("Description")),
                interfaces,
                labels: resolve_labels(&v("Labels"), index)?,
            };
            d.validate().map_err(invalid)?;
            Ok(Some(WriteIntent::CreateWorkload(d)))
        }
        EditIntent::UpdateWorkload(w) => {
            let (add_addresses, remove_addresses) = address_changes(w, &items(&v("Addresses")))?;
            let labels = v("Labels");
            // The field shows the full list, so a changed field replaces it.
            let set_labels = if items(&labels) == items(&labels_text(w, index)) {
                None
            } else {
                Some(resolve_labels(&labels, index)?)
            };
            let patch = WorkloadPatch {
                name: changed(w.name.as_deref().unwrap_or(""), v("Name")),
                hostname: changed(w.hostname.as_deref().unwrap_or(""), v("Hostname")),
                description: changed(w.description.as_deref().unwrap_or(""), v("Description")),
                add_addresses,
                remove_addresses,
                set_labels,
                ..WorkloadPatch::default()
            };
            if patch.is_empty() {
                Ok(None)
            } else {
                Ok(Some(WriteIntent::UpdateWorkload {
                    href: w.href.clone(),
                    patch,
                }))
            }
        }
        // Deletes never open a form (`start`); a form never carries one.
        EditIntent::DeleteLabel(_)
        | EditIntent::DeleteIpList(_)
        | EditIntent::DeleteService(_)
        | EditIntent::DeleteWorkload(_) => Err("a delete has no form".into()),
    }
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p illumio-tui --lib tui::edit`
Expected: PASS (8 tests).

- [ ] **Step 5: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p illumio-tui
git add crates/illumio-tui/src/tui/edit.rs
git commit -m "feat(tui): edit intents, pre-filled forms and form-to-draft/patch adapters

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---
### Task 5: Editing keys, scrollable help, hints

**Files:**
- Modify: `crates/illumio-tui/src/tui/msg.rs` (`Cmd` ~line 92: `PlanWrite`; `PartialEq for Cmd` ~line 127)
- Modify: `crates/illumio-tui/src/tui/model.rs` (`Model.help_scroll`, init in `new_connecting`)
- Modify: `crates/illumio-tui/src/tui/update.rs` (imports; `on_key` `Overlay::Help` arm ~line 491 and the Task 3 `Overlay::Form` arm; `on_screen_key` `?` ~line 730; `workloads_key` ~line 1166; `objects_key` ~line 1328; `on_command` `Command::Help` ~line 1407; new helpers after `objects_key`)
- Modify: `crates/illumio-tui/src/tui/app.rs` (`execute`: staged `Cmd::PlanWrite` arm)
- Modify: `crates/illumio-tui/src/tui/ui/widgets/overlay.rs` (`HELP_LINES` ~line 13, `render_help` ~line 56, new `help_last_scroll`, `screen_hint` ~line 385)
- Modify: `crates/illumio-tui/src/tui/ui/widgets/status.rs` (line 17)
- Test: unit tests in `update.rs`; `tests/tui_snapshots.rs` (`help_overlay_80x24` ~line 118 rewritten, one new test)

**Interfaces:**
- Consumes: `edit::{EditIntent, WriteIntent, Start, start, build_intent}` (Task 4), `FormEvent` (Task 3), `WritesState::refusal` (Task 2), `TableView::selected_item` (`tui/table.rs` ~line 109), `WorkloadRowView.workload` (`model.rs`), `Model::index` (`model.rs`).
- Produces:

```rust
// tui/msg.rs
pub enum Cmd { /* … */ PlanWrite(WriteIntent) }
// tui/model.rs
pub struct Model { /* … */ pub help_scroll: usize }
// ui/widgets/overlay.rs
pub fn help_last_scroll(height: u16) -> usize;
pub fn screen_hint(screen: Screen, writes: bool) -> &'static str;
// tui/update.rs (private)
fn writes_refused(model: &mut Model) -> bool;
fn begin_edit(model: &mut Model, intent: EditIntent) -> Vec<Cmd>;
fn objects_edit_key(model: &mut Model, c: char) -> Vec<Cmd>;
fn workloads_edit_key(model: &mut Model, c: char) -> Vec<Cmd>;
```

- [ ] **Step 1: Write the failing tests**

Append to the `#[cfg(test)] mod tests` in `crates/illumio-tui/src/tui/update.rs`:

```rust
    // ---- Editing keys (Plan 4) ----

    fn writes_on(mut m: Model) -> Model {
        m.writes = crate::tui::model::WritesState::on();
        m
    }

    #[test]
    fn editing_keys_with_writes_off_only_say_why() {
        use crate::tui::test_support::{model_with_objects, model_with_workloads};
        let mut m = model_with_objects();
        m.screen = Screen::Objects;
        for c in ['n', 'e', 'x'] {
            assert!(update(&mut m, key(c)).is_empty(), "{c}");
            assert_eq!(m.status.kind, StatusKind::Warn);
            assert_eq!(m.status.text, "started without --allow-writes");
            assert_eq!(m.overlay, Overlay::None);
        }
        let mut m = model_with_workloads();
        m.screen = Screen::Workloads;
        for c in ['n', 'e', 'x'] {
            assert!(update(&mut m, key(c)).is_empty(), "{c}");
            assert_eq!(m.status.text, "started without --allow-writes");
            assert_eq!(m.overlay, Overlay::None);
        }
    }

    #[test]
    fn label_groups_and_managed_workloads_are_refused_before_any_form() {
        use crate::tui::edit::WriteIntent;
        use crate::tui::model::ObjectKind;
        use crate::tui::test_support::{model_with_objects, model_with_workloads};
        let mut m = writes_on(model_with_objects());
        m.screen = Screen::Objects;
        m.objects.kind = ObjectKind::LabelGroups;
        for c in ['n', 'e', 'x'] {
            assert!(update(&mut m, key(c)).is_empty(), "{c}");
            assert_eq!(m.status.text, "label groups are read-only in this version");
            assert_eq!(m.overlay, Overlay::None);
        }
        let mut m = writes_on(model_with_workloads());
        m.screen = Screen::Workloads;
        // Row 0 is host-1.example, VEN-managed.
        for c in ['e', 'x'] {
            assert!(update(&mut m, key(c)).is_empty(), "{c}");
            assert_eq!(m.status.text, "only unmanaged workloads can be edited here");
            assert_eq!(m.overlay, Overlay::None);
        }
        update(&mut m, key('j'));
        update(&mut m, key('j')); // unmanaged-nas
        assert_eq!(
            update(&mut m, key('x')),
            vec![Cmd::PlanWrite(WriteIntent::DeleteWorkload {
                href: "/orgs/5/workloads/aaaa-3".into(),
                name: "unmanaged-nas".into(),
            })],
            "a delete plans at once, the confirm overlay comes next"
        );
        assert!(update(&mut m, key('e')).is_empty());
        assert!(
            matches!(&m.overlay, Overlay::Form(f) if f.title == "Edit workload unmanaged-nas"),
            "{:?}",
            m.overlay
        );
    }

    #[test]
    fn e_opens_a_prefilled_form_and_enter_plans_only_the_change() {
        use crate::tui::edit::WriteIntent;
        use crate::tui::model::ObjectKind;
        use crate::tui::test_support::model_with_objects;
        let mut m = writes_on(model_with_objects());
        m.screen = Screen::Objects;
        m.objects.kind = ObjectKind::IpLists;
        update(&mut m, key('j')); // Corp
        assert!(update(&mut m, key('e')).is_empty());
        let Overlay::Form(f) = &m.overlay else {
            panic!("{:?}", m.overlay)
        };
        assert_eq!(f.value("Ranges"), "10.1.0.0-10.1.255.255");
        // Unchanged: the form closes, nothing is planned.
        assert!(update(&mut m, enter()).is_empty());
        assert_eq!(m.status.text, "no changes");
        assert_eq!(m.overlay, Overlay::None);
        update(&mut m, key('e'));
        update(&mut m, kc(KeyCode::Tab));
        update(&mut m, kc(KeyCode::Tab));
        for c in ", 10.2.0.0/16".chars() {
            update(&mut m, key(c));
        }
        let cmds = update(&mut m, enter());
        assert_eq!(
            cmds,
            vec![Cmd::PlanWrite(WriteIntent::UpdateIpList {
                href: "/orgs/5/sec_policy/draft/ip_lists/21".into(),
                patch: pce::write::IpListPatch {
                    add_ranges: vec![pce::write::IpRangeSpec::parse("10.2.0.0/16", false).unwrap()],
                    ..Default::default()
                },
            })]
        );
        assert_eq!(m.overlay, Overlay::None);
        assert_eq!(m.status.text, "planning…");
    }

    #[test]
    fn n_opens_an_empty_form_that_validates_and_esc_cancels() {
        use crate::tui::model::ObjectKind;
        use crate::tui::test_support::model_with_objects;
        let mut m = writes_on(model_with_objects());
        m.screen = Screen::Objects;
        m.objects.kind = ObjectKind::Services;
        assert!(update(&mut m, key('n')).is_empty());
        assert!(update(&mut m, enter()).is_empty());
        let Overlay::Form(f) = &m.overlay else {
            panic!("{:?}", m.overlay)
        };
        assert_eq!(f.error.as_deref(), Some("Name is required"));
        assert_eq!(f.active, 0);
        for c in "SSH".chars() {
            update(&mut m, key(c));
        }
        update(&mut m, kc(KeyCode::Tab));
        update(&mut m, kc(KeyCode::Tab));
        for c in "22".chars() {
            update(&mut m, key(c));
        }
        assert!(
            update(&mut m, enter()).is_empty(),
            "a bare port is refused locally"
        );
        let Overlay::Form(f) = &m.overlay else {
            panic!("{:?}", m.overlay)
        };
        assert!(f.error.as_deref().unwrap().contains("22/tcp"), "{:?}", f.error);
        assert!(update(&mut m, kc(KeyCode::Esc)).is_empty());
        assert_eq!(m.overlay, Overlay::None);
        assert_eq!(m.status.text, "cancelled, nothing sent");
    }

    #[test]
    fn help_scrolls_to_its_last_line_and_reopens_at_the_top() {
        use crate::tui::ui::widgets::overlay::{HELP_LINES, help_last_scroll};
        let mut m = model_fixture(); // 80x24
        assert_eq!(HELP_LINES.len(), 25);
        update(&mut m, key('?'));
        let last = help_last_scroll(24);
        assert_eq!(last, 3, "25 lines, 22 rows inside the box");
        for _ in 0..40 {
            update(&mut m, key('j'));
        }
        assert_eq!(m.help_scroll, last);
        update(&mut m, key('k'));
        assert_eq!(m.help_scroll, last - 1);
        update(&mut m, key('G'));
        assert_eq!(m.help_scroll, last);
        update(&mut m, key('g'));
        assert_eq!(m.help_scroll, 0);
        update(&mut m, key('G'));
        update(&mut m, kc(KeyCode::Esc));
        update(&mut m, key('?'));
        assert_eq!(m.help_scroll, 0, "the help reopens at the top");
    }
```

Rewrite `help_overlay_80x24` in `crates/illumio-tui/tests/tui_snapshots.rs` and append the hint test:

```rust
#[test]
fn help_overlay_80x24() {
    use illumio_tui::tui::model::Overlay;
    use illumio_tui::tui::ui::widgets::overlay::help_last_scroll;
    let mut m = model_fixture();
    m.overlay = Overlay::Help;
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("1 2 3 4 5"), "first page: {out}");
    assert!(out.contains("move in the current table"), "{out}");
    assert!(out.contains("j/k scroll"), "{out}");
    insta::assert_snapshot!("help_overlay_80x24", out);
    m.help_scroll = help_last_scroll(24);
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("close overlay"), "last line visible: {out}");
    assert!(out.contains("send / cancel a shown request"), "{out}");
    insta::assert_snapshot!("help_overlay_end_80x24", out);
    m.set_theme("light");
    insta::assert_snapshot!(
        "help_overlay_end_light_styled_80x24",
        render_styled(&m, 80, 24)
    );
}

#[test]
fn status_hint_lists_the_edit_keys_only_with_writes() {
    use illumio_tui::tui::model::WritesState;
    use illumio_tui::tui::test_support::model_with_objects;
    let mut m = model_with_objects();
    m.screen = Screen::Objects;
    let last = |m: &illumio_tui::tui::Model| {
        render_to_string(m, 80, 24)
            .lines()
            .last()
            .unwrap_or("")
            .to_string()
    };
    assert!(!last(&m).contains("n:new"), "{}", last(&m));
    m.writes = WritesState::on();
    assert!(last(&m).contains("n:new e:edit x:del"), "{}", last(&m));
    m.screen = Screen::Workloads;
    assert!(last(&m).contains("n:new e:edit x:del"), "{}", last(&m));
    m.screen = Screen::Traffic;
    assert!(!last(&m).contains("n:new"), "{}", last(&m));
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p illumio-tui --lib tui::update::tests::editing_keys`
Expected: FAIL, compile errors (`Cmd::PlanWrite`, `help_scroll`, `help_last_scroll` not found).

- [ ] **Step 3: Add the command** (`crates/illumio-tui/src/tui/msg.rs`)

Add `use crate::tui::edit::WriteIntent;` to the imports. In `pub enum Cmd`, before `Quit,`:

```rust
    /// Plan a write (one GET for IP lists, services and workloads; none for
    /// labels) and show it in the confirm overlay. Never sends a write.
    PlanWrite(WriteIntent),
```

In `impl PartialEq for Cmd`, before `_ => false,`:

```rust
            (Cmd::PlanWrite(a), Cmd::PlanWrite(b)) => a == b,
```

- [ ] **Step 4: Stage the runtime arm** (`crates/illumio-tui/src/tui/app.rs`, `execute`, after `Cmd::Quit => …,`)

```rust
            // Planning is wired in Task 6; until then nothing is planned or sent.
            Cmd::PlanWrite(_) => {
                self.model.status =
                    crate::tui::model::StatusLine::warn("planning is not available yet; nothing sent");
            }
```

- [ ] **Step 5: Help scroll state** (`crates/illumio-tui/src/tui/model.rs`)

In `pub struct Model`, after `pub writes: WritesState,`:

```rust
    /// First help line shown; `Overlay::Help` is reopened at the top.
    pub help_scroll: usize,
```

and in `new_connecting`'s `Self { … }`, after `writes,`: `help_scroll: 0,`.

- [ ] **Step 6: Help lines, scrolling and hints** (`crates/illumio-tui/src/tui/ui/widgets/overlay.rs`)

In `HELP_LINES`, insert before `("Esc", "close overlay / cancel"),`:

```rust
    (
        "n / e / x",
        "new / edit / delete (Objects, Workloads; needs writes)",
    ),
    (
        "Pending  j/k Enter",
        "select · diff of a pending IP list or service",
    ),
    ("y / n", "send / cancel a shown request"),
```

Replace `render_help` and add `help_last_scroll`:

```rust
/// The first help line shown when scrolled to the end, for a terminal
/// `height` rows tall (the box is as tall as the lines allow, borders included).
pub fn help_last_scroll(height: u16) -> usize {
    let rows = (HELP_LINES.len() + 2)
        .min(height as usize)
        .saturating_sub(2);
    HELP_LINES.len().saturating_sub(rows)
}

pub fn render_help(model: &Model, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let total = HELP_LINES.len();
    let r = centered(area, area.width.min(84), (total as u16) + 2);
    f.render_widget(Clear, r);
    let rows = r.height.saturating_sub(2) as usize;
    let first = model.help_scroll.min(help_last_scroll(area.height));
    let meta = if total > rows {
        format!(
            "{}-{} of {total} · j/k scroll · Esc closes",
            first + 1,
            (first + rows).min(total)
        )
    } else {
        "Esc closes".to_string()
    };
    let block = panel("Keys", &meta, t, true);
    let lines: Vec<Line> = HELP_LINES
        .iter()
        .skip(first)
        .take(rows)
        .map(|(k, d)| {
            Line::from(vec![
                Span::styled(format!("{k:<24}"), t.header_style()),
                Span::styled(*d, t.base()),
            ])
        })
        .collect();
    f.render_widget(Paragraph::new(lines).block(block).style(t.base()), r);
}
```

Replace `screen_hint`:

```rust
/// Key hints for the status line. With writes on, Objects and Workloads lead
/// with the edit keys and drop their least used hints so the status text
/// keeps room at 80 columns.
pub fn screen_hint(screen: Screen, writes: bool) -> &'static str {
    match (screen, writes) {
        (Screen::Objects, true) => "n:new e:edit x:del  Tab type  Enter detail",
        (Screen::Workloads, true) => "n:new e:edit x:del  t/T traffic  Enter detail",
        (Screen::Traffic, _) => "f filter  r run  a auto  v view  s sort  / search",
        (Screen::Policy, _) => "d scope  p pending  Enter detail  Tab pane  a auto",
        (Screen::Objects, false) => "Tab type  Enter detail  r refresh  a auto  / search",
        (Screen::Workloads, false) => "t/T traffic  h/H health  Enter detail  r refresh",
        (Screen::Events, _) => "f filter  r refresh  a auto  S severity  Enter detail",
    }
}
```

In `crates/illumio-tui/src/tui/ui/widgets/status.rs` line 17:

```rust
    let hints = format!(
        " {}  : cmd  ? help ",
        screen_hint(model.screen, model.writes.enabled)
    );
```

- [ ] **Step 7: Keys** (`crates/illumio-tui/src/tui/update.rs`)

Imports: add `use crate::tui::edit::{self, EditIntent, Start};` and `use crate::tui::ui::widgets::overlay::help_last_scroll;`.

Replace the `Overlay::Help => { … }` arm in `on_key`:

```rust
        Overlay::Help => {
            let last = help_last_scroll(model.size.1);
            match k.code {
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') => {
                    model.overlay = Overlay::None;
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    model.help_scroll = (model.help_scroll + 1).min(last);
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    model.help_scroll = model.help_scroll.saturating_sub(1);
                }
                KeyCode::End | KeyCode::Char('G') => model.help_scroll = last,
                KeyCode::Home | KeyCode::Char('g') => model.help_scroll = 0,
                _ => {}
            }
            Vec::new()
        }
```

Replace the Task 3 `Overlay::Form(mut form) => { … }` arm:

```rust
        Overlay::Form(mut form) => match form.on_key(k) {
            FormEvent::Continue => {
                model.overlay = Overlay::Form(form);
                Vec::new()
            }
            FormEvent::Cancel => {
                model.overlay = Overlay::None;
                model.status = StatusLine::info("cancelled, nothing sent");
                Vec::new()
            }
            FormEvent::Submit => {
                match edit::build_intent(&form, model.index().map(|i| i.as_ref())) {
                    Ok(Some(intent)) => {
                        model.overlay = Overlay::None;
                        model.status = StatusLine::info("planning…");
                        vec![Cmd::PlanWrite(intent)]
                    }
                    Ok(None) => {
                        model.overlay = Overlay::None;
                        model.status = StatusLine::info("no changes");
                        Vec::new()
                    }
                    Err(e) => {
                        form.error = Some(e);
                        model.overlay = Overlay::Form(form);
                        Vec::new()
                    }
                }
            }
        },
```

In `on_screen_key`, the `?` arm, and in `on_command`, the `Command::Help` arm, set the scroll before opening:

```rust
        (KeyCode::Char('?'), _) => {
            model.help_scroll = 0;
            model.overlay = Overlay::Help;
            Vec::new()
        }
```

```rust
        Command::Help => {
            model.help_scroll = 0;
            model.overlay = Overlay::Help;
            Vec::new()
        }
```

In `objects_key`, inside `if !model.objects.is_searching() { … match k.code {`, before `_ => {}`:

```rust
            KeyCode::Char(c @ ('n' | 'e' | 'x')) => return objects_edit_key(model, c),
```

In `workloads_key`, inside `if !model.workloads.table.is_searching() { match k.code {`, before `_ => {}`:

```rust
            KeyCode::Char(c @ ('n' | 'e' | 'x')) => return workloads_edit_key(model, c),
```

Add after `objects_key`:

```rust
/// Writes off: the reason goes to the status line and the key does nothing else.
fn writes_refused(model: &mut Model) -> bool {
    if model.writes.enabled {
        return false;
    }
    model.status = StatusLine::warn(model.writes.refusal());
    true
}

/// A create or update opens its form; a delete goes straight to planning.
fn begin_edit(model: &mut Model, intent: EditIntent) -> Vec<Cmd> {
    match edit::start(intent, model.index().map(|i| i.as_ref())) {
        Start::Form(f) => {
            model.overlay = Overlay::Form(f);
            Vec::new()
        }
        Start::Plan(w) => {
            model.status = StatusLine::info("planning…");
            vec![Cmd::PlanWrite(w)]
        }
    }
}

/// `n` / `e` / `x` on Objects: labels, IP lists, services; label groups are read-only.
fn objects_edit_key(model: &mut Model, c: char) -> Vec<Cmd> {
    if writes_refused(model) {
        return Vec::new();
    }
    let o = &model.objects;
    let intent = match (o.kind, c) {
        (ObjectKind::LabelGroups, _) => {
            model.status = StatusLine::warn("label groups are read-only in this version");
            return Vec::new();
        }
        (ObjectKind::Labels, 'n') => Some(EditIntent::CreateLabel),
        (ObjectKind::IpLists, 'n') => Some(EditIntent::CreateIpList),
        (ObjectKind::Services, 'n') => Some(EditIntent::CreateService),
        (ObjectKind::Labels, 'e') => o.labels.selected_item().cloned().map(EditIntent::UpdateLabel),
        (ObjectKind::Labels, _) => o.labels.selected_item().cloned().map(EditIntent::DeleteLabel),
        (ObjectKind::IpLists, 'e') => o
            .ip_lists
            .selected_item()
            .cloned()
            .map(EditIntent::UpdateIpList),
        (ObjectKind::IpLists, _) => o
            .ip_lists
            .selected_item()
            .cloned()
            .map(EditIntent::DeleteIpList),
        (ObjectKind::Services, 'e') => o
            .services
            .selected_item()
            .cloned()
            .map(EditIntent::UpdateService),
        (ObjectKind::Services, _) => o
            .services
            .selected_item()
            .cloned()
            .map(EditIntent::DeleteService),
    };
    let Some(intent) = intent else {
        model.status = StatusLine::warn("no row selected");
        return Vec::new();
    };
    begin_edit(model, intent)
}

/// `n` / `e` / `x` on Workloads. Edit and delete only for workloads the PCE
/// reports as unmanaged with no VEN (the library's rule, checked again there).
fn workloads_edit_key(model: &mut Model, c: char) -> Vec<Cmd> {
    if writes_refused(model) {
        return Vec::new();
    }
    if c == 'n' {
        return begin_edit(model, EditIntent::CreateWorkload);
    }
    let Some(w) = model
        .workloads
        .table
        .selected_item()
        .map(|r| r.workload.clone())
    else {
        model.status = StatusLine::warn("no row selected");
        return Vec::new();
    };
    if w.managed != Some(false) || w.ven.is_some() {
        model.status = StatusLine::warn("only unmanaged workloads can be edited here");
        return Vec::new();
    }
    let intent = if c == 'e' {
        EditIntent::UpdateWorkload(w)
    } else {
        EditIntent::DeleteWorkload(w)
    };
    begin_edit(model, intent)
}
```

- [ ] **Step 8: Run the tests, update the help snapshot**

Run: `cargo test -p illumio-tui --lib tui::update`
Expected: PASS (all existing update tests plus the five new ones).

Run: `INSTA_UPDATE=always cargo test -p illumio-tui --test tui_snapshots && git status --short crates/illumio-tui/tests/snapshots`
Expected: PASS; exactly `tui_snapshots__help_overlay_80x24.snap` modified (meta `1-22 of 25 · j/k scroll · Esc closes`; the first 22 lines, so `Esc  close overlay / cancel` is no longer on the first page) and two new files, `tui_snapshots__help_overlay_end_80x24.snap` (ends with `y / n` and `Esc  close overlay / cancel`) and `tui_snapshots__help_overlay_end_light_styled_80x24.snap`. No other snapshot changes (writes are off in every other fixture, so no hint changes).

- [ ] **Step 9: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p illumio-tui
git add crates/illumio-tui/src/tui crates/illumio-tui/tests/tui_snapshots.rs crates/illumio-tui/tests/snapshots
git commit -m "feat(tui): n/e/x editing keys with refusals, form submit to PlanWrite, scrollable help

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 6: Plan and confirm

**Files:**
- Modify: `crates/illumio-tui/src/tui/msg.rs` (`Msg::Planned`, `Cmd::ExecuteWrite`, `PartialEq for Cmd`)
- Modify: `crates/illumio-tui/src/tui/model.rs` (`Overlay::ConfirmWrite`, `PartialEq for Overlay`)
- Modify: `crates/illumio-tui/src/tui/edit.rs` (`request_body_lines`)
- Modify: `crates/illumio-tui/src/tui/update.rs` (`Msg::Planned` arm in `update` after `Msg::PendingLoaded` ~line 414; `Overlay::ConfirmWrite` arm in `on_key`)
- Modify: `crates/illumio-tui/src/tui/app.rs` (`execute`: `Cmd::PlanWrite` task replacing the Task 5 arm, staged `Cmd::ExecuteWrite` arm; `plan_intent`, `named` helpers; `apply` `task_result` list ~line 327)
- Modify: `crates/illumio-tui/src/tui/ui/widgets/overlay.rs` (`render_confirm_write` after `render_confirm` ~line 240), `crates/illumio-tui/src/tui/ui/mod.rs` (dispatch)
- Modify: `crates/illumio-tui/src/tui/test_support.rs` (`label_post_plan`, `ip_list_delete_plan`)
- Test: unit tests in `update.rs`; `tests/tui_snapshots.rs`, `tests/tui_runtime.rs` (append)

**Interfaces:**
- Consumes: `Writer::{plan_create_label, plan_update_label, plan_delete_label}` (`write/labels.rs`), `plan_create_ip_list`/`plan_update_ip_list`/`plan_delete_ip_list` (`write/ip_lists.rs`), `plan_create_service`/`plan_update_service`/`plan_delete_service` (`write/services.rs`), `plan_create_workload`/`plan_update_workload(href, patch, &NameIndex)`/`plan_delete_workload` (`write/workloads.rs`), `IfMissing::Create`, `Plan::{Send, Noop}`, `Plan::request`, `WriteRequest { method, path, body, summary }`, `WriteError` (`write/mod.rs`), `Pce::writer` (`write/mod.rs` ~line 246), `WriteMethod::as_str` (`pce/src/client.rs` ~line 74), `fail` (`update.rs` ~line 439).
- Produces:

```rust
// tui/msg.rs
pub enum Msg { /* … */ Planned(Result<(Plan, pce::write::ObjectKind), String>) }
pub enum Cmd { /* … */ ExecuteWrite { plan: Plan, kind: pce::write::ObjectKind } }
// tui/model.rs
pub enum Overlay { /* … */ ConfirmWrite { plan: Plan, kind: pce::write::ObjectKind, scroll: u16 } }
// tui/edit.rs
pub fn request_body_lines(req: &WriteRequest) -> Vec<String>;   // pretty JSON lines, or ["(no body)"]
// tui/app.rs (private)
async fn plan_intent(pce: &Pce, guard: &WriteGuard, intent: WriteIntent) -> Result<(Plan, pce::write::ObjectKind), WriteError>;
fn named(plan: Plan, kind: &str, name: &str) -> Plan;
// ui/widgets/overlay.rs
pub fn render_confirm_write(model: &Model, plan: &Plan, scroll: u16, f: &mut Frame, area: Rect);
// tui/test_support.rs
pub fn label_post_plan() -> pce::write::Plan;      // POST /orgs/5/labels {"key":"app","value":"web2"}
pub fn ip_list_delete_plan() -> pce::write::Plan;  // DELETE /orgs/5/sec_policy/draft/ip_lists/21, summary "delete ip_list Corp"
```

- [ ] **Step 1: Write the failing tests**

Append to `crates/illumio-tui/src/tui/test_support.rs`:

```rust
/// The plan the confirm overlay shows for a new label `app=web2`.
pub fn label_post_plan() -> pce::write::Plan {
    pce::write::Plan::Send {
        request: pce::write::WriteRequest {
            method: pce::WriteMethod::Post,
            path: "/orgs/5/labels".into(),
            body: Some(serde_json::json!({"key": "app", "value": "web2"})),
            summary: "create label app=web2".into(),
        },
        kind: pce::write::ObjectKind::Label,
    }
}

/// The plan for `x` on the Corp IP list, with the TUI's named summary.
pub fn ip_list_delete_plan() -> pce::write::Plan {
    pce::write::Plan::Send {
        request: pce::write::WriteRequest {
            method: pce::WriteMethod::Delete,
            path: "/orgs/5/sec_policy/draft/ip_lists/21".into(),
            body: None,
            summary: "delete ip_list Corp".into(),
        },
        kind: pce::write::ObjectKind::IpList,
    }
}
```

Append to the tests in `crates/illumio-tui/src/tui/update.rs`:

```rust
    #[test]
    fn planned_opens_the_confirm_overlay_and_y_sends_once() {
        use crate::tui::test_support::label_post_plan;
        let mut m = writes_on(model_fixture());
        let planned = Msg::Planned(Ok((label_post_plan(), pce::write::ObjectKind::Label)));
        assert!(update(&mut m, planned).is_empty());
        assert!(matches!(
            &m.overlay,
            Overlay::ConfirmWrite { scroll: 0, .. }
        ));
        update(&mut m, key('j'));
        update(&mut m, key('j'));
        assert!(matches!(&m.overlay, Overlay::ConfirmWrite { scroll: 2, .. }));
        for _ in 0..10 {
            update(&mut m, key('j'));
        }
        // The body is four lines: `{`, key, value, `}`.
        assert!(
            matches!(&m.overlay, Overlay::ConfirmWrite { scroll: 3, .. }),
            "stops at the last body line: {:?}",
            m.overlay
        );
        update(&mut m, key('k'));
        assert!(matches!(&m.overlay, Overlay::ConfirmWrite { scroll: 2, .. }));
        assert!(update(&mut m, key('q')).is_empty());
        assert!(!m.quit, "q does not quit from the confirm overlay");
        let cmds = update(&mut m, key('y'));
        assert_eq!(
            cmds,
            vec![Cmd::ExecuteWrite {
                plan: label_post_plan(),
                kind: pce::write::ObjectKind::Label
            }]
        );
        assert_eq!(m.overlay, Overlay::None);
        assert_eq!(m.status.text, "sending …");
        assert!(
            update(&mut m, key('y')).is_empty(),
            "a second y has no overlay to answer"
        );
    }

    #[test]
    fn n_or_esc_cancels_and_a_failed_plan_reports() {
        use crate::tui::test_support::ip_list_delete_plan;
        for code in [KeyCode::Char('n'), KeyCode::Esc] {
            let mut m = writes_on(model_fixture());
            update(
                &mut m,
                Msg::Planned(Ok((ip_list_delete_plan(), pce::write::ObjectKind::IpList))),
            );
            assert!(update(&mut m, kc(code)).is_empty());
            assert_eq!(m.overlay, Overlay::None);
            assert_eq!(m.status.text, "cancelled, nothing sent");
        }
        let mut m = writes_on(model_fixture());
        update(
            &mut m,
            Msg::Planned(Err("not found: /orgs/5/sec_policy/draft/ip_lists/9".into())),
        );
        assert_eq!(m.status.kind, StatusKind::Error);
        assert_eq!(
            m.status.text,
            "write: not found: /orgs/5/sec_policy/draft/ip_lists/9"
        );
        assert_eq!(m.overlay, Overlay::None, "short errors stay in the status line");
        let long = "range 10.9.0.0/16 is not in Corp; an IP list needs at least one range or FQDN";
        update(&mut m, Msg::Planned(Err(long.into())));
        assert_eq!(m.overlay, Overlay::Error(format!("write: {long}")));
        let mut m = writes_on(model_fixture());
        let noop = pce::write::Plan::Noop {
            existing_href: "/orgs/5/labels/1".into(),
            reason: "label role=web exists".into(),
        };
        update(&mut m, Msg::Planned(Ok((noop, pce::write::ObjectKind::Label))));
        assert_eq!(m.status.kind, StatusKind::Error);
        assert!(
            m.status.text.contains("nothing to send (label role=web exists)"),
            "{}",
            m.status.text
        );
        assert_eq!(m.overlay, Overlay::None);
    }
```

Append to `crates/illumio-tui/tests/tui_snapshots.rs`:

```rust
#[test]
fn confirm_write_overlay_shows_method_path_and_body() {
    use illumio_tui::tui::model::{Overlay, Screen};
    use illumio_tui::tui::test_support::{ip_list_delete_plan, label_post_plan, model_with_objects};
    let mut m = model_with_objects();
    m.screen = Screen::Objects;
    m.overlay = Overlay::ConfirmWrite {
        plan: label_post_plan(),
        kind: pce::write::ObjectKind::Label,
        scroll: 0,
    };
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("create label app=web2"), "{out}");
    assert!(out.contains("POST /orgs/5/labels"), "{out}");
    assert!(out.contains("\"value\": \"web2\""), "{out}");
    assert!(out.contains("y send · n cancel"), "{out}");
    insta::assert_snapshot!("confirm_label_post_80x24", out);
    let mut l = model_with_objects();
    l.screen = Screen::Objects;
    l.set_theme("light");
    l.overlay = m.overlay.clone();
    insta::assert_snapshot!("confirm_label_post_light_styled_80x24", render_styled(&l, 80, 24));
    m.overlay = Overlay::ConfirmWrite {
        plan: ip_list_delete_plan(),
        kind: pce::write::ObjectKind::IpList,
        scroll: 0,
    };
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("delete ip_list Corp"), "{out}");
    assert!(out.contains("DELETE /orgs/5/sec_policy/draft/ip_lists/21"), "{out}");
    assert!(out.contains("(no body)"), "{out}");
    insta::assert_snapshot!("confirm_ip_list_delete_80x24", out);
    l.overlay = m.overlay.clone();
    insta::assert_snapshot!(
        "confirm_ip_list_delete_light_styled_80x24",
        render_styled(&l, 80, 24)
    );
}
```

Append to `crates/illumio-tui/tests/tui_runtime.rs` (and add `use pce::write::{MemoryAudit, WriteContext, WriteGuard};` to the imports):

```rust
/// The test model with writes on (the runtime guard is given separately).
fn writes_model() -> Model {
    let mut m = model();
    m.writes = WritesState::on();
    m
}

/// A guard like the one `run` builds, auditing into memory.
fn tui_guard(audit: Arc<MemoryAudit>) -> WriteGuard {
    WriteGuard::new(
        true,
        true,
        Box::new(audit),
        WriteContext {
            profile: "t".into(),
            host: "h".into(),
            org_id: 5,
            source: "tui".into(),
        },
    )
    .unwrap()
}

/// One key no screen or overlay uses, after `ms`: lets queued tasks finish.
fn wait(ms: u64) -> impl futures::Stream<Item = Event> {
    stream::once(async move {
        tokio::time::sleep(Duration::from_millis(ms)).await;
        Event::Key(KeyEvent::new(KeyCode::F(12), KeyModifiers::NONE))
    })
}

#[tokio::test]
async fn plan_write_needs_the_runtime_writer_and_sends_nothing() {
    use illumio_tui::tui::edit::WriteIntent;
    use illumio_tui::tui::model::Overlay;
    use pce::write::LabelDraft;
    let server = MockServer::start().await;
    let intent = WriteIntent::CreateLabel(LabelDraft::new("app", "web2"));
    // No writer: refused, although the model says writes are on.
    let (mut rt, _tx) = Runtime::new(pce_for(&server), writes_model());
    rt.execute(Cmd::PlanWrite(intent.clone()));
    let model = run_loop(&mut rt, Box::pin(wait(300)), &mut |_| {}).await;
    assert_eq!(
        model.status.text, "write: writes are not enabled",
        "{:?}",
        model.status
    );
    assert_eq!(model.overlay, Overlay::None);
    assert_eq!(model.busy, 0);
    // With a writer: the confirm overlay shows the POST; planning a label sends nothing.
    let audit = MemoryAudit::new();
    let (mut rt, _tx) =
        Runtime::with_writer(pce_for(&server), writes_model(), tui_guard(audit.clone()));
    rt.execute(Cmd::PlanWrite(intent));
    let model = run_loop(&mut rt, Box::pin(wait(300)), &mut |_| {}).await;
    match &model.overlay {
        Overlay::ConfirmWrite { plan, kind, scroll } => {
            let req = plan.request().expect("a request");
            assert_eq!(req.method, pce::WriteMethod::Post);
            assert_eq!(req.path, "/orgs/5/labels");
            assert_eq!(*kind, pce::write::ObjectKind::Label);
            assert_eq!(*scroll, 0);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(model.busy, 0);
    assert!(
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty(),
        "planning a label makes no request"
    );
    assert!(audit.entries().is_empty(), "nothing sent, nothing audited");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p illumio-tui --lib tui::update::tests::planned`
Expected: FAIL, compile errors (`Msg::Planned`, `Overlay::ConfirmWrite`, `Cmd::ExecuteWrite`, `label_post_plan` not found).

- [ ] **Step 3: Messages and commands** (`crates/illumio-tui/src/tui/msg.rs`)

Add `use pce::write::Plan;` to the imports. In `pub enum Msg`, after `ProfileSwitched(…),`:

```rust
    /// A planned write (the exact request) or why it could not be planned.
    Planned(Result<(Plan, pce::write::ObjectKind), String>),
```

In `pub enum Cmd`, after `PlanWrite(WriteIntent),`:

```rust
    /// `y` on the confirm overlay: send this plan once.
    ExecuteWrite {
        plan: Plan,
        kind: pce::write::ObjectKind,
    },
```

In `impl PartialEq for Cmd`, before `_ => false,` (`Plan` is not `PartialEq`; a plan is its request):

```rust
            (
                Cmd::ExecuteWrite { plan: pa, kind: ka },
                Cmd::ExecuteWrite { plan: pb, kind: kb },
            ) => pa.request() == pb.request() && ka == kb,
```

- [ ] **Step 4: The overlay** (`crates/illumio-tui/src/tui/model.rs`)

Add `use pce::write::Plan;` to the imports. In `pub enum Overlay`, after `Form(Form),`:

```rust
    /// The exact request a write would send; `y` sends it, `n`/`Esc` cancel.
    /// `scroll` is the first body line shown.
    ConfirmWrite {
        plan: Plan,
        kind: pce::write::ObjectKind,
        scroll: u16,
    },
```

In `impl PartialEq for Overlay`, before `_ => false,`:

```rust
            (
                Overlay::ConfirmWrite {
                    plan: pa,
                    kind: ka,
                    scroll: sa,
                },
                Overlay::ConfirmWrite {
                    plan: pb,
                    kind: kb,
                    scroll: sb,
                },
            ) => pa.request() == pb.request() && ka == kb && sa == sb,
```

- [ ] **Step 5: Body lines** (`crates/illumio-tui/src/tui/edit.rs`, after `build_intent`; add `WriteRequest` to the `pce::write` import)

```rust
/// The request body as the confirm overlay shows it: pretty JSON, one entry
/// per line, or `(no body)` for a delete.
pub fn request_body_lines(req: &WriteRequest) -> Vec<String> {
    match &req.body {
        Some(b) => serde_json::to_string_pretty(b)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect(),
        None => vec!["(no body)".to_string()],
    }
}
```

- [ ] **Step 6: Update** (`crates/illumio-tui/src/tui/update.rs`)

Add `use pce::write::Plan;` to the imports. In `update`, after the `Msg::PendingLoaded(result) => { … }` arm:

```rust
        Msg::Planned(result) => {
            match result {
                Ok((plan, kind)) if plan.request().is_some() => {
                    model.status = StatusLine::info("review the request: y sends it, n cancels");
                    model.overlay = Overlay::ConfirmWrite {
                        plan,
                        kind,
                        scroll: 0,
                    };
                }
                // Cannot happen with `IfMissing::Create`; reported, never sent.
                Ok((plan, _)) => {
                    let reason = match plan {
                        Plan::Noop { reason, .. } => reason,
                        Plan::Send { .. } => String::new(),
                    };
                    fail(model, "write", &format!("nothing to send ({reason})"), false);
                }
                Err(e) => fail(model, "write", &e, false),
            }
            Vec::new()
        }
```

In `on_key`, before the `Overlay::RuleDetail(_) | …` arm:

```rust
        Overlay::ConfirmWrite { plan, kind, scroll } => {
            let lines = plan.request().map_or(1, |r| edit::request_body_lines(r).len());
            let last = u16::try_from(lines.saturating_sub(1)).unwrap_or(u16::MAX);
            match k.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    model.overlay = Overlay::None;
                    model.status = StatusLine::info("sending …");
                    vec![Cmd::ExecuteWrite { plan, kind }]
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    model.overlay = Overlay::None;
                    model.status = StatusLine::info("cancelled, nothing sent");
                    Vec::new()
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    model.overlay = Overlay::ConfirmWrite {
                        plan,
                        kind,
                        scroll: (scroll + 1).min(last),
                    };
                    Vec::new()
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    model.overlay = Overlay::ConfirmWrite {
                        plan,
                        kind,
                        scroll: scroll.saturating_sub(1),
                    };
                    Vec::new()
                }
                _ => Vec::new(),
            }
        }
```

- [ ] **Step 7: Runtime** (`crates/illumio-tui/src/tui/app.rs`)

Imports: add `use crate::tui::edit::WriteIntent;` and extend the write import to `use pce::write::{IfMissing, JsonlAudit, Plan, WriteContext, WriteError, WriteGuard};`; add `use pce::NameIndex;`.

Replace the Task 5 `Cmd::PlanWrite(_) => { … }` arm and add the staged `ExecuteWrite` arm:

```rust
            Cmd::PlanWrite(intent) => {
                self.model.busy += 1;
                let (pce, tx) = self.task_ctx();
                let guard = self.writer.clone();
                tokio::spawn(async move {
                    let result = match guard {
                        None => Err("writes are not enabled".to_string()),
                        Some(guard) => plan_intent(&pce, &guard, intent)
                            .await
                            .map_err(|e| e.to_string()),
                    };
                    let _ = tx.send(Msg::Planned(result));
                    let _ = tx.send(Msg::Stats(pce.stats()));
                });
            }
            // Sending is wired in Task 7; until then `y` sends nothing.
            Cmd::ExecuteWrite { .. } => {
                self.model.status =
                    crate::tui::model::StatusLine::warn("sending is not available yet; nothing sent");
            }
```

In `apply`, add `| Msg::Planned(_)` to the `task_result` list (after `| Msg::ProfileSwitched(_)`).

Add after `writer_from`:

```rust
/// The library planner for one intent, through the guard. Creates use
/// `IfMissing::Create`; IP list, service and workload updates and workload
/// deletes fetch the object once; label plans make no request.
async fn plan_intent(
    pce: &Pce,
    guard: &WriteGuard,
    intent: WriteIntent,
) -> Result<(Plan, pce::write::ObjectKind), WriteError> {
    let w = pce.writer(guard);
    let kind = intent.kind();
    let plan = match intent {
        WriteIntent::CreateLabel(d) => w.plan_create_label(&d, IfMissing::Create).await?,
        WriteIntent::UpdateLabel { href, value } => w.plan_update_label(&href, &value)?,
        WriteIntent::DeleteLabel { href, name } => {
            named(w.plan_delete_label(&href)?, "label", &name)
        }
        WriteIntent::CreateIpList(d) => w.plan_create_ip_list(&d, IfMissing::Create).await?,
        WriteIntent::UpdateIpList { href, patch } => w.plan_update_ip_list(&href, &patch).await?,
        WriteIntent::DeleteIpList { href, name } => {
            named(w.plan_delete_ip_list(&href)?, "ip_list", &name)
        }
        WriteIntent::CreateService(d) => w.plan_create_service(&d, IfMissing::Create).await?,
        WriteIntent::UpdateService { href, patch } => {
            w.plan_update_service(&href, &patch).await?
        }
        WriteIntent::DeleteService { href, name } => {
            named(w.plan_delete_service(&href)?, "service", &name)
        }
        WriteIntent::CreateWorkload(d) => w.plan_create_workload(&d, IfMissing::Create).await?,
        // The TUI only sends `set_labels`, which `WorkloadPatch::apply` never
        // resolves through the index (only add/remove do), so an empty index
        // costs no request.
        WriteIntent::UpdateWorkload { href, patch } => {
            w.plan_update_workload(&href, &patch, &NameIndex::default())
                .await?
        }
        // The library's summary already names the workload.
        WriteIntent::DeleteWorkload { href, .. } => w.plan_delete_workload(&href).await?,
    };
    Ok((plan, kind))
}

/// The library's delete summary names the href; the confirm overlay names
/// the object. Only `summary` changes: it is neither sent nor audited.
fn named(mut plan: Plan, kind: &str, name: &str) -> Plan {
    if let Plan::Send { request, .. } = &mut plan {
        request.summary = format!("delete {kind} {name}");
    }
    plan
}
```

- [ ] **Step 8: Render** (`crates/illumio-tui/src/tui/ui/widgets/overlay.rs`)

Add `use pce::write::Plan;` to the imports and, after `render_confirm`:

```rust
pub fn render_confirm_write(model: &Model, plan: &Plan, scroll: u16, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let Some(req) = plan.request() else {
        return;
    };
    let body = crate::tui::edit::request_body_lines(req);
    // Borders, the summary, `METHOD path` and a blank line around the body.
    let r = centered(area, 76, (body.len() as u16).saturating_add(5));
    f.render_widget(Clear, r);
    let rows = r.height.saturating_sub(5) as usize;
    let first = (scroll as usize).min(body.len().saturating_sub(rows));
    let mut lines = vec![
        Line::from(Span::styled(plain(&req.summary), Style::new().fg(t.warn))),
        Line::from(Span::styled(
            format!("{} {}", req.method.as_str(), plain(&req.path)),
            t.header_style(),
        )),
        Line::from(""),
    ];
    lines.extend(
        body.iter()
            .skip(first)
            .take(rows)
            .map(|l| Line::from(Span::styled(plain(l), t.base()))),
    );
    let meta = if body.len() > rows {
        "j/k scroll · y send · n cancel"
    } else {
        "y send · n cancel"
    };
    f.render_widget(
        Paragraph::new(lines)
            .block(panel("Send this request?", meta, t, true))
            .style(t.base()),
        r,
    );
}
```

In `crates/illumio-tui/src/tui/ui/mod.rs`, after the `Overlay::Form` dispatch arm:

```rust
        Overlay::ConfirmWrite { plan, scroll, .. } => {
            widgets::overlay::render_confirm_write(model, plan, *scroll, f, area)
        }
```

- [ ] **Step 9: Run the tests, create the snapshots**

Run: `cargo test -p illumio-tui --lib tui::update && cargo test -p illumio-tui --test tui_runtime plan_write_needs`
Expected: PASS.

Run: `INSTA_UPDATE=always cargo test -p illumio-tui --test tui_snapshots confirm_write && git status --short crates/illumio-tui/tests/snapshots`
Expected: PASS; exactly four new files: `tui_snapshots__confirm_label_post_80x24.snap`, `tui_snapshots__confirm_label_post_light_styled_80x24.snap`, `tui_snapshots__confirm_ip_list_delete_80x24.snap`, `tui_snapshots__confirm_ip_list_delete_light_styled_80x24.snap`. Read the dark ones: title `Send this request?`, summary, `POST /orgs/5/labels` / `DELETE /orgs/5/sec_policy/draft/ip_lists/21`, then the body or `(no body)`.

- [ ] **Step 10: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p illumio-tui
git add crates/illumio-tui/src/tui crates/illumio-tui/tests crates/illumio-tui/tests/snapshots
git commit -m "feat(tui): plan writes through the guard and confirm the exact request

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---
### Task 7: Execute, report and reload

**Files:**
- Modify: `crates/illumio-tui/src/tui/msg.rs` (`Msg::WriteDone`)
- Modify: `crates/illumio-tui/src/tui/update.rs` (`Msg::WriteDone` arm after `Msg::Planned`; new `reload_after_write` after `load_workloads` ~line 1121; imports)
- Modify: `crates/illumio-tui/src/tui/app.rs` (`execute`: `Cmd::ExecuteWrite` task replacing the Task 6 arm; `apply` `task_result` list)
- Test: unit tests in `update.rs`; `crates/illumio-tui/tests/tui_runtime.rs` (append)

**Interfaces:**
- Consumes: `Writer::execute(plan) -> Result<Outcome, WriteError>` (`write/mod.rs` ~line 270, sends once, audits, invalidates `kind.cache_prefix()`, re-fetches after a 204), `Outcome::{Written, Skipped}`, `Written { href, pending, status, object, dropped_fields, warnings }`, `WriteError::{Unanswered, Pce, AuditFailed, Refused, Invalid}`, `WriteRefused::FlagMissing` (`write/guard.rs`), `Pce::invalidate` (`lib.rs` ~line 117), `load_objects` / `load_workloads` (`update.rs` ~lines 1105, 1121), `MemoryAudit::entries` and `AuditEntry { method, status, source, … }` (`write/audit.rs`), `two_profile_config`, `mount_objects`, `pce_for` (`tests/tui_runtime.rs`), `writes_model`, `tui_guard`, `wait` (Task 6).
- Produces:

```rust
// tui/msg.rs
pub enum Msg { /* … */ WriteDone { kind: pce::write::ObjectKind, method: Option<pce::WriteMethod>, result: Result<Outcome, WriteError> } }
// tui/update.rs (private)
fn reload_after_write(model: &mut Model, kind: pce::write::ObjectKind) -> Vec<Cmd>;
```

- [ ] **Step 1: Write the failing tests**

Append to the tests in `crates/illumio-tui/src/tui/update.rs`:

```rust
    fn written(href: &str, pending: bool) -> pce::write::Written {
        pce::write::Written {
            href: href.into(),
            pending,
            status: 201,
            object: serde_json::json!({ "href": href }),
            dropped_fields: vec![],
            warnings: vec![],
        }
    }

    #[test]
    fn write_done_reports_reloads_the_kind_once_and_forgets_pending() {
        use crate::tui::model::ObjectKind;
        use crate::tui::test_support::model_with_objects;
        use pce::write::Outcome;
        let mut m = model_with_objects();
        m.policy.pending = Some(vec![]);
        let cmds = update(
            &mut m,
            Msg::WriteDone {
                kind: pce::write::ObjectKind::IpList,
                method: Some(pce::WriteMethod::Post),
                result: Ok(Outcome::Written(written(
                    "/orgs/5/sec_policy/draft/ip_lists/9",
                    true,
                ))),
            },
        );
        assert_eq!(
            cmds,
            vec![Cmd::LoadObjects {
                kind: ObjectKind::IpLists,
                mode: pce::CacheMode::Bypass
            }]
        );
        assert_eq!(
            m.status.text,
            "created /orgs/5/sec_policy/draft/ip_lists/9 (pending)"
        );
        assert_eq!(m.status.kind, StatusKind::Info);
        assert!(m.policy.pending.is_none(), "the next p fetches again");
        let mut w = written("/orgs/5/labels/9", false);
        w.warnings = vec!["audit entry could not be written to x: denied".into()];
        let cmds = update(
            &mut m,
            Msg::WriteDone {
                kind: pce::write::ObjectKind::Label,
                method: Some(pce::WriteMethod::Put),
                result: Ok(Outcome::Written(w)),
            },
        );
        assert_eq!(
            cmds,
            vec![Cmd::LoadObjects {
                kind: ObjectKind::Labels,
                mode: pce::CacheMode::Bypass
            }]
        );
        assert_eq!(m.status.kind, StatusKind::Warn);
        assert_eq!(
            m.status.text,
            "updated /orgs/5/labels/9; audit entry could not be written to x: denied"
        );
        let cmds = update(
            &mut m,
            Msg::WriteDone {
                kind: pce::write::ObjectKind::Workload,
                method: Some(pce::WriteMethod::Delete),
                result: Ok(Outcome::Written(written("/orgs/5/workloads/aaaa-3", false))),
            },
        );
        assert_eq!(cmds, vec![Cmd::LoadWorkloads(pce::CacheMode::Bypass)]);
        assert_eq!(m.status.text, "deleted /orgs/5/workloads/aaaa-3");
        let cmds = update(
            &mut m,
            Msg::WriteDone {
                kind: pce::write::ObjectKind::Label,
                method: None,
                result: Ok(Outcome::Skipped {
                    existing_href: "/orgs/5/labels/1".into(),
                    reason: "label role=web exists".into(),
                }),
            },
        );
        assert!(cmds.is_empty());
        assert_eq!(m.status.text, "skipped: label role=web exists");
    }

    #[test]
    fn write_errors_open_the_overlay_and_only_unanswered_reloads() {
        use crate::tui::model::ObjectKind;
        use crate::tui::test_support::model_with_objects;
        use pce::PceError;
        use pce::write::WriteError;
        let mut m = model_with_objects();
        let cmds = update(
            &mut m,
            Msg::WriteDone {
                kind: pce::write::ObjectKind::Service,
                method: Some(pce::WriteMethod::Delete),
                result: Err(WriteError::Pce(PceError::Rejected {
                    status: 406,
                    body: "object_in_use: referenced by a rule".into(),
                })),
            },
        );
        assert!(cmds.is_empty(), "a rejected write reloads nothing");
        match &m.overlay {
            Overlay::Error(t) => assert!(
                t.starts_with("write failed: ") && t.contains("object_in_use: referenced by a rule"),
                "{t}"
            ),
            other => panic!("{other:?}"),
        }
        assert_eq!(m.status.kind, StatusKind::Error);
        m.overlay = Overlay::None;
        let cmds = update(
            &mut m,
            Msg::WriteDone {
                kind: pce::write::ObjectKind::Service,
                method: Some(pce::WriteMethod::Put),
                result: Err(WriteError::Unanswered(PceError::Timeout(
                    std::time::Duration::from_secs(30),
                ))),
            },
        );
        assert_eq!(
            cmds,
            vec![Cmd::LoadObjects {
                kind: ObjectKind::Services,
                mode: pce::CacheMode::Bypass
            }],
            "the user sees the real state"
        );
        match &m.overlay {
            Overlay::Error(t) => assert!(
                t.starts_with("the request was sent but not answered; check the PCE before retrying"),
                "{t}"
            ),
            other => panic!("{other:?}"),
        }
    }
```

Append to `crates/illumio-tui/tests/tui_runtime.rs` (add `body_json` to the `wiremock::matchers` import):

```rust
/// The four index lists, each expected `n` times (startup plus reloads).
async fn mount_lists(server: &MockServer, labels: u64, ip_lists: u64, services: u64) {
    for (p, body, n) in [
        (
            "/api/v2/orgs/5/labels",
            include_str!("../../../fixtures/labels.json"),
            labels,
        ),
        (
            "/api/v2/orgs/5/sec_policy/draft/label_groups",
            include_str!("../../../fixtures/label_groups.json"),
            1,
        ),
        (
            "/api/v2/orgs/5/sec_policy/draft/ip_lists",
            include_str!("../../../fixtures/ip_lists.json"),
            ip_lists,
        ),
        (
            "/api/v2/orgs/5/sec_policy/draft/services",
            include_str!("../../../fixtures/services.json"),
            services,
        ),
    ] {
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(body)
                    .insert_header("Content-Type", "application/json"),
            )
            .expect(n)
            .mount(server)
            .await;
    }
}

/// Each key after its delay in ms.
fn press(keys: Vec<(u64, KeyCode)>) -> impl futures::Stream<Item = Event> {
    stream::iter(keys).then(|(ms, code)| async move {
        tokio::time::sleep(Duration::from_millis(ms)).await;
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    })
}

fn ch(c: char) -> KeyCode {
    KeyCode::Char(c)
}

/// Typing `text` into a form, 20 ms per character.
fn typed(text: &str) -> Vec<(u64, KeyCode)> {
    text.chars().map(|c| (20, KeyCode::Char(c))).collect()
}

/// Connects (health is mounted here), presses `keys`, returns the model.
async fn key_session(
    server: &MockServer,
    model: Model,
    writer: Option<WriteGuard>,
    keys: Vec<(u64, KeyCode)>,
) -> Model {
    Mock::given(method("GET"))
        .and(path("/api/v2/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "normal"})))
        .expect(1)
        .mount(server)
        .await;
    let pce = pce_for(server);
    let (mut rt, _tx) = match writer {
        Some(g) => Runtime::with_writer(pce, model, g),
        None => Runtime::new(pce, model),
    };
    rt.execute(Cmd::Connect);
    run_loop(&mut rt, Box::pin(press(keys)), &mut |_| {}).await
}

/// Methods the mock PCE received other than GET.
async fn writes_seen(server: &MockServer) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .map(|r| r.method.to_string())
        .filter(|m| m != "GET")
        .collect()
}

#[tokio::test]
async fn writes_disabled_editing_keys_send_nothing() {
    use illumio_tui::tui::model::{Overlay, StatusKind};
    let server = MockServer::start().await;
    mount_objects(&server).await;
    let keys = vec![
        (500, ch('3')),
        (50, ch('n')),
        (50, ch('e')),
        (50, ch('x')),
        (50, KeyCode::Tab),
        (50, KeyCode::Tab),
        (50, ch('e')),
        (200, KeyCode::F(12)),
    ];
    let model = key_session(&server, model(), None, keys).await;
    assert_eq!(model.stats.calls, 5, "health + four lists");
    assert!(writes_seen(&server).await.is_empty());
    assert_eq!(model.status.text, "writes are not enabled");
    assert_eq!(model.status.kind, StatusKind::Warn);
    assert_eq!(model.overlay, Overlay::None);
}

#[tokio::test]
async fn create_label_posts_once_audits_as_tui_and_reloads_labels_once() {
    let server = MockServer::start().await;
    mount_lists(&server, 2, 1, 1).await;
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/labels"))
        .and(body_json(json!({"key": "app", "value": "web2"})))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_json(json!({"href": "/orgs/5/labels/9", "key": "app", "value": "web2"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let audit = MemoryAudit::new();
    let mut keys = vec![(500, ch('3')), (50, ch('n'))];
    keys.extend(typed("app"));
    keys.push((20, KeyCode::Tab));
    keys.extend(typed("web2"));
    keys.extend([(50, KeyCode::Enter), (300, ch('y')), (400, KeyCode::F(12))]);
    let model = key_session(
        &server,
        writes_model(),
        Some(tui_guard(audit.clone())),
        keys,
    )
    .await;
    assert_eq!(model.status.text, "created /orgs/5/labels/9");
    assert_eq!(
        model.stats.calls, 7,
        "health + four lists + POST + one labels reload"
    );
    assert_eq!(writes_seen(&server).await, vec!["POST"]);
    let entries = audit.entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].source, "tui");
    assert_eq!(entries[0].method, pce::WriteMethod::Post);
    assert_eq!(entries[0].status, Some(201));
    assert_eq!(model.busy, 0);
}

#[tokio::test]
async fn update_ip_list_puts_only_the_changed_ranges() {
    let server = MockServer::start().await;
    mount_lists(&server, 1, 2, 1).await;
    let corp = json!({
        "href": "/orgs/5/sec_policy/draft/ip_lists/21", "name": "Corp", "description": "Office ranges",
        "ip_ranges": [{"from_ip": "10.1.0.0", "to_ip": "10.1.255.255", "exclusion": false}],
        "fqdns": [{"fqdn": "*.corp.example"}], "update_type": "create"
    });
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/ip_lists/21"))
        .respond_with(ResponseTemplate::new(200).set_body_json(corp.clone()))
        .expect(1)
        .mount(&server)
        .await;
    // A real PCE answers 204 here and the writer re-fetches (one more GET);
    // answering with the object keeps the count to the plan's GET.
    Mock::given(method("PUT"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/ip_lists/21"))
        .and(body_json(json!({"ip_ranges": [
            {"from_ip": "10.1.0.0", "to_ip": "10.1.255.255"},
            {"from_ip": "10.2.0.0/16"}
        ]})))
        .respond_with(ResponseTemplate::new(200).set_body_json(corp))
        .expect(1)
        .mount(&server)
        .await;
    let audit = MemoryAudit::new();
    let mut keys = vec![
        (500, ch('3')),
        (50, KeyCode::Tab),
        (50, KeyCode::Tab),
        (50, ch('j')),
        (50, ch('e')),
        (20, KeyCode::Tab),
        (20, KeyCode::Tab),
    ];
    keys.extend(typed(", 10.2.0.0/16"));
    keys.extend([(50, KeyCode::Enter), (300, ch('y')), (400, KeyCode::F(12))]);
    let model = key_session(
        &server,
        writes_model(),
        Some(tui_guard(audit.clone())),
        keys,
    )
    .await;
    assert_eq!(
        model.status.text,
        "updated /orgs/5/sec_policy/draft/ip_lists/21 (pending)"
    );
    assert_eq!(
        model.stats.calls, 8,
        "health + four lists + plan GET + PUT + one IP lists reload"
    );
    assert_eq!(writes_seen(&server).await, vec!["PUT"]);
    assert_eq!(audit.entries().len(), 1);
    assert_eq!(model.busy, 0);
}

#[tokio::test]
async fn delete_service_rejected_shows_the_body_and_does_not_reload() {
    use illumio_tui::tui::model::Overlay;
    let server = MockServer::start().await;
    mount_lists(&server, 1, 1, 1).await; // services: the startup load only
    Mock::given(method("DELETE"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/services/30"))
        .respond_with(ResponseTemplate::new(406).set_body_json(json!([
            {"token": "object_in_use", "message": "service is referenced by a rule"}
        ])))
        .expect(1)
        .mount(&server)
        .await;
    let audit = MemoryAudit::new();
    let keys = vec![
        (500, ch('3')),
        (50, KeyCode::Tab),
        (50, KeyCode::Tab),
        (50, KeyCode::Tab),
        (50, ch('x')), // All Services
        (300, ch('y')),
        (400, KeyCode::F(12)),
    ];
    let model = key_session(
        &server,
        writes_model(),
        Some(tui_guard(audit.clone())),
        keys,
    )
    .await;
    match &model.overlay {
        Overlay::Error(t) => assert!(
            t.contains("object_in_use: service is referenced by a rule"),
            "{t}"
        ),
        other => panic!("{other:?}"),
    }
    assert_eq!(writes_seen(&server).await, vec!["DELETE"], "sent once, never retried");
    assert_eq!(model.stats.calls, 6, "health + four lists + DELETE, no reload");
    let entries = audit.entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].status, Some(406));
    assert_eq!(model.busy, 0);
}

#[tokio::test]
async fn cancelled_confirm_sends_nothing() {
    let server = MockServer::start().await;
    mount_objects(&server).await;
    let audit = MemoryAudit::new();
    let mut keys = vec![(500, ch('3')), (50, ch('n'))];
    keys.extend(typed("app"));
    keys.push((20, KeyCode::Tab));
    keys.extend(typed("web2"));
    keys.extend([(50, KeyCode::Enter), (300, ch('n')), (200, KeyCode::F(12))]);
    let model = key_session(
        &server,
        writes_model(),
        Some(tui_guard(audit.clone())),
        keys,
    )
    .await;
    assert_eq!(model.status.text, "cancelled, nothing sent");
    assert!(writes_seen(&server).await.is_empty());
    assert!(audit.entries().is_empty());
    assert_eq!(model.stats.calls, 5);
}

#[tokio::test]
async fn switching_to_a_profile_without_writes_drops_the_guard_and_the_marker() {
    use illumio_tui::cli::GlobalArgs;
    use illumio_tui::tui::model::{Overlay, StatusKind};
    use illumio_tui::tui::msg::Msg;
    use illumio_tui::tui::test_support::render_to_string;
    let a = MockServer::start().await;
    let b = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&b)
        .await;
    mount_objects(&b).await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = two_profile_config(dir.path(), &b);
    let global = GlobalArgs {
        config: Some(cfg),
        profile: Some("a".into()),
        ..Default::default()
    };
    let (mut rt, tx) = Runtime::with_args(pce_for(&a), writes_model(), global);
    rt.writer = Some(Arc::new(tui_guard(MemoryAudit::new())));
    let header = |m: &Model| {
        render_to_string(m, 80, 24)
            .lines()
            .next()
            .unwrap_or("")
            .to_string()
    };
    assert!(header(&rt.model).contains("✎ writes"));
    tx.send(Msg::Command(illumio_tui::tui::commands::Command::Profile(
        "b".into(),
    )))
    .unwrap();
    let keys = vec![(700, ch('3')), (100, ch('n')), (200, KeyCode::F(12))];
    let model = run_loop(&mut rt, Box::pin(press(keys)), &mut |_| {}).await;
    assert_eq!(model.profile.name, "b");
    assert!(rt.writer.is_none(), "the old profile's guard is gone");
    assert!(!model.writes.enabled);
    assert!(!header(&model).contains("writes"), "{}", header(&model));
    assert_eq!(model.status.kind, StatusKind::Warn);
    assert_eq!(model.status.text, model.writes.refusal());
    assert_eq!(model.overlay, Overlay::None);
    assert_eq!(model.stats.calls, 5, "health + four lists; n made no request");
    assert!(a.received_requests().await.unwrap_or_default().is_empty());
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p illumio-tui --lib tui::update::tests::write_done`
Expected: FAIL, compile error (`Msg::WriteDone` not found).

- [ ] **Step 3: The message** (`crates/illumio-tui/src/tui/msg.rs`)

Extend the import to `use pce::write::{Outcome, Plan, WriteError};`. In `pub enum Msg`, after `Planned(…),`:

```rust
    /// The result of one sent write. `method` names what was sent (None for
    /// a plan that was a no-op).
    WriteDone {
        kind: pce::write::ObjectKind,
        method: Option<pce::WriteMethod>,
        result: Result<Outcome, WriteError>,
    },
```

- [ ] **Step 4: Update** (`crates/illumio-tui/src/tui/update.rs`)

Extend imports: `use pce::write::{Outcome, Plan, WriteError};` and `use pce::{CacheMode, WriteMethod};` (replacing `use pce::CacheMode;`). After the `Msg::Planned(result) => { … }` arm:

```rust
        Msg::WriteDone {
            kind,
            method,
            result,
        } => {
            // The runtime dropped `policy:pending`; the next `p` fetches again.
            model.policy.pending = None;
            match result {
                Ok(Outcome::Written(w)) => {
                    let verb = match method {
                        Some(WriteMethod::Post) => "created",
                        Some(WriteMethod::Put) => "updated",
                        Some(WriteMethod::Delete) => "deleted",
                        None => "wrote",
                    };
                    let mut text = format!("{verb} {}", w.href);
                    if w.pending {
                        text.push_str(" (pending)");
                    }
                    if !w.dropped_fields.is_empty() {
                        text.push_str(&format!(
                            "; the PCE dropped {}",
                            w.dropped_fields.join(", ")
                        ));
                    }
                    for warning in &w.warnings {
                        text.push_str("; ");
                        text.push_str(warning);
                    }
                    model.status = if w.warnings.is_empty() && w.dropped_fields.is_empty() {
                        StatusLine::info(text)
                    } else {
                        StatusLine::warn(text)
                    };
                    reload_after_write(model, kind)
                }
                Ok(Outcome::Skipped { reason, .. }) => {
                    model.status = StatusLine::info(format!("skipped: {reason}"));
                    Vec::new()
                }
                Err(e @ WriteError::Unanswered(_)) => {
                    model.status = StatusLine::error("write not answered; reloading");
                    model.overlay = Overlay::Error(format!(
                        "the request was sent but not answered; check the PCE before retrying: {e}"
                    ));
                    reload_after_write(model, kind)
                }
                Err(e) => {
                    model.status = StatusLine::error("write failed; nothing reloaded");
                    model.overlay = Overlay::Error(format!("write failed: {e}"));
                    Vec::new()
                }
            }
        }
```

After `load_workloads`:

```rust
/// One reload of the written kind's table, past the cache (the writer
/// already invalidated it).
fn reload_after_write(model: &mut Model, kind: pce::write::ObjectKind) -> Vec<Cmd> {
    match kind {
        pce::write::ObjectKind::Label => load_objects(model, ObjectKind::Labels, CacheMode::Bypass),
        pce::write::ObjectKind::IpList => load_objects(model, ObjectKind::IpLists, CacheMode::Bypass),
        pce::write::ObjectKind::Service => {
            load_objects(model, ObjectKind::Services, CacheMode::Bypass)
        }
        pce::write::ObjectKind::Workload => load_workloads(model, CacheMode::Bypass),
    }
}
```

- [ ] **Step 5: Runtime** (`crates/illumio-tui/src/tui/app.rs`)

Extend the write import with `WriteRefused`. Replace the Task 6 `Cmd::ExecuteWrite { .. } => { … }` arm:

```rust
            Cmd::ExecuteWrite { plan, kind } => {
                self.model.busy += 1;
                let (pce, tx) = self.task_ctx();
                let guard = self.writer.clone();
                let method = plan.request().map(|r| r.method);
                tokio::spawn(async move {
                    // `execute` sends once, audits, and invalidates the kind's cache prefix.
                    let result = match &guard {
                        Some(guard) => pce.writer(guard).execute(plan).await,
                        None => Err(WriteError::Refused(WriteRefused::FlagMissing)),
                    };
                    // Pending changes are not under the kind's prefix.
                    pce.invalidate("policy:pending");
                    let _ = tx.send(Msg::WriteDone {
                        kind,
                        method,
                        result,
                    });
                    let _ = tx.send(Msg::Stats(pce.stats()));
                });
            }
```

In `apply`, add `| Msg::WriteDone { .. }` to the `task_result` list.

- [ ] **Step 6: Run the tests**

Run: `cargo test -p illumio-tui --lib tui::update`
Expected: PASS.

Run: `cargo test -p illumio-tui --test tui_runtime`
Expected: PASS: every existing runtime test plus `plan_write_needs_the_runtime_writer_and_sends_nothing`, `writes_disabled_editing_keys_send_nothing`, `create_label_posts_once_audits_as_tui_and_reloads_labels_once`, `update_ip_list_puts_only_the_changed_ranges`, `delete_service_rejected_shows_the_body_and_does_not_reload`, `cancelled_confirm_sends_nothing`, `switching_to_a_profile_without_writes_drops_the_guard_and_the_marker`. Wiremock's `.expect(n)` verifies the counts when each server drops.

- [ ] **Step 7: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p illumio-tui
git add crates/illumio-tui/src/tui crates/illumio-tui/tests/tui_runtime.rs
git commit -m "feat(tui): send a confirmed write once, report it, reload its table

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---
### Task 8: Pending diff

**Files:**
- Create: `crates/illumio-tui/src/tui/diff.rs`
- Modify: `crates/illumio-tui/src/tui/mod.rs` (`pub mod diff;` after `pub mod commands;`)
- Modify: `crates/illumio-tui/src/tui/model.rs` (`PolicyState` ~line 844 and `PolicyState::new` ~line 866: `pending_selected`, `diff_loading`; `Overlay::Diff`; `PartialEq for Overlay`)
- Modify: `crates/illumio-tui/src/tui/msg.rs` (`Cmd::LoadDiff`, `Msg::DiffLoaded`, `PartialEq for Cmd`)
- Modify: `crates/illumio-tui/src/tui/update.rs` (`Msg::PendingLoaded` ~line 414 resets the selection; `Msg::DiffLoaded` arm; `on_key`: `Overlay::Pending` gets its own arm out of the `RuleDetail | Pending | Error` arm ~line 683, new `Overlay::Diff` arm; new `pending_diff` helper)
- Modify: `crates/illumio-tui/src/tui/app.rs` (`execute`: `Cmd::LoadDiff`; `load_diff` helper; `apply` `task_result` list)
- Modify: `crates/illumio-tui/src/tui/ui/widgets/overlay.rs` (`render_pending` ~line 348 with selection; new `render_diff`), `crates/illumio-tui/src/tui/ui/mod.rs` (dispatch)
- Test: unit tests in `diff.rs`, `update.rs`; `tests/tui_snapshots.rs` (append; `policy_pending_80x24` snapshot changes), `tests/tui_runtime.rs` (append)

**Interfaces:**
- Consumes: `Pce::{ip_lists, services}` and `Pce::{active_ip_lists, active_services}` (Task 1), `PendingChange { href, change_type, name, object_type }` (`models/policy.rs` ~line 147, `Clone + PartialEq + Eq`), `parse_pending` (`models/policy.rs` ~line 158), `IpRangeSpec::{from_range, display}`, `ServicePort::display`, `Ref::object_type` (`models/common.rs` ~line 20), `plain`, `panel`, `centered`.
- Produces:

```rust
// tui/diff.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum DiffKind { Created, Updated, Deleted }
impl DiffKind { pub fn from_change_type(change_type: &str) -> Self; pub fn as_str(self) -> &'static str; }
#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum Sign { Same, Added, Removed }
#[derive(Debug, Clone, PartialEq, Eq)] pub struct DiffLine { pub sign: Sign, pub text: String }
#[derive(Debug, Clone, PartialEq, Eq)] pub struct ObjectDiff { pub name: String, pub href: String, pub change: DiffKind, pub lines: Vec<DiffLine> }
pub fn diff_ip_list(active: Option<&IpList>, draft: Option<&IpList>) -> Vec<DiffLine>;
pub fn diff_service(active: Option<&Service>, draft: Option<&Service>) -> Vec<DiffLine>;
pub fn ip_list_diff_for(change: &PendingChange, draft: &[IpList], active: &[IpList]) -> ObjectDiff;
pub fn service_diff_for(change: &PendingChange, draft: &[Service], active: &[Service]) -> ObjectDiff;
// tui/model.rs
pub struct PolicyState { /* … */ pub pending_selected: usize, pub diff_loading: bool }
pub enum Overlay { /* … */ Diff { diff: ObjectDiff, scroll: u16 } }
// tui/msg.rs
pub enum Cmd { /* … */ LoadDiff { change: PendingChange, kind: pce::write::ObjectKind } }
pub enum Msg { /* … */ DiffLoaded(Result<ObjectDiff, String>) }
// ui/widgets/overlay.rs
pub fn render_diff(model: &Model, diff: &ObjectDiff, scroll: u16, f: &mut Frame, area: Rect);
```

- [ ] **Step 1: Write the failing tests**

Create `crates/illumio-tui/src/tui/diff.rs` with its tests (Step 3 adds the code above them):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn corp() -> IpList {
        serde_json::from_value(json!({
            "href": "/orgs/5/sec_policy/draft/ip_lists/21", "name": "Corp", "description": "Office ranges",
            "ip_ranges": [{"from_ip": "10.1.0.0", "to_ip": "10.1.255.255"}],
            "fqdns": [{"fqdn": "*.corp.example"}]
        }))
        .unwrap()
    }

    fn line(sign: Sign, text: &str) -> DiffLine {
        DiffLine {
            sign,
            text: text.into(),
        }
    }

    fn change(href: &str, change_type: &str) -> PendingChange {
        PendingChange {
            href: href.into(),
            change_type: change_type.into(),
            name: Some("Corp".into()),
            object_type: "ip_lists".into(),
        }
    }

    #[test]
    fn an_updated_ip_list_shows_added_removed_and_unchanged_entries() {
        let mut active = corp();
        active.href = "/orgs/5/sec_policy/active/ip_lists/21".into();
        let mut draft = corp();
        draft.description = Some("Office".into());
        draft
            .ip_ranges
            .push(serde_json::from_value(json!({"from_ip": "10.2.0.0/16"})).unwrap());
        draft.fqdns.clear();
        assert_eq!(
            diff_ip_list(Some(&active), Some(&draft)),
            vec![
                line(Sign::Same, "name: Corp"),
                line(Sign::Removed, "description: Office ranges"),
                line(Sign::Added, "description: Office"),
                line(Sign::Same, "range 10.1.0.0-10.1.255.255"),
                line(Sign::Added, "range 10.2.0.0/16"),
                line(Sign::Removed, "fqdn *.corp.example"),
            ]
        );
        let d = ip_list_diff_for(
            &change("/orgs/5/sec_policy/draft/ip_lists/21", "update"),
            &[draft],
            &[active],
        );
        assert_eq!(d.change, DiffKind::Updated);
        assert_eq!(d.name, "Corp");
        assert_eq!(d.lines.len(), 6, "the active href matches its draft href");
    }

    #[test]
    fn created_is_all_added_and_deleted_all_removed() {
        let c = corp();
        let created = diff_ip_list(None, Some(&c));
        assert_eq!(created.len(), 4, "name, description, range, fqdn");
        assert!(created.iter().all(|l| l.sign == Sign::Added));
        assert!(
            diff_ip_list(Some(&c), None)
                .iter()
                .all(|l| l.sign == Sign::Removed)
        );
        let d = ip_list_diff_for(&change(&c.href, "delete"), &[c.clone()], &[c.clone()]);
        assert_eq!(d.change, DiffKind::Deleted);
        assert!(
            d.lines.iter().all(|l| l.sign == Sign::Removed),
            "a delete has no draft side"
        );
    }

    #[test]
    fn a_missing_side_is_said_not_shown_as_an_empty_diff() {
        let c = corp();
        let d = ip_list_diff_for(&change(&c.href, "update"), &[], &[c.clone()]);
        assert_eq!(d.lines, vec![line(Sign::Same, "Corp is not in the draft list")]);
        let d = ip_list_diff_for(&change(&c.href, "update"), &[c.clone()], &[]);
        assert_eq!(d.lines, vec![line(Sign::Same, "Corp is not in the active list")]);
        let d = ip_list_diff_for(&change(&c.href, "create"), &[c.clone()], &[]);
        assert_eq!(d.change, DiffKind::Created);
        assert!(d.lines.iter().all(|l| l.sign == Sign::Added));
        let d = ip_list_diff_for(&change(&c.href, "create"), &[], &[]);
        assert_eq!(d.lines, vec![line(Sign::Same, "Corp is not in the draft list")]);
    }

    #[test]
    fn services_diff_by_port_and_fall_back_to_the_object_name() {
        let active: Service = serde_json::from_value(json!({
            "href": "/orgs/5/sec_policy/active/services/32", "name": "Web",
            "service_ports": [{"port": 80, "proto": 6}, {"port": 443, "proto": 6}]
        }))
        .unwrap();
        let draft: Service = serde_json::from_value(json!({
            "href": "/orgs/5/sec_policy/draft/services/32", "name": "Web",
            "service_ports": [{"port": 443, "proto": 6}, {"port": 8443, "proto": 6}]
        }))
        .unwrap();
        assert_eq!(
            diff_service(Some(&active), Some(&draft)),
            vec![
                line(Sign::Same, "name: Web"),
                line(Sign::Same, "port 443/tcp"),
                line(Sign::Added, "port 8443/tcp"),
                line(Sign::Removed, "port 80/tcp"),
            ]
        );
        let unnamed = PendingChange {
            href: draft.href.clone(),
            change_type: "update".into(),
            name: None,
            object_type: "services".into(),
        };
        let d = service_diff_for(&unnamed, &[draft], &[active]);
        assert_eq!(d.name, "Web");
        assert_eq!(d.change, DiffKind::Updated);
    }
}
```

Append to the tests in `crates/illumio-tui/src/tui/update.rs`:

```rust
    #[test]
    fn pending_list_selects_with_j_k_and_enter_asks_for_one_diff() {
        use crate::tui::test_support::model_with_policy;
        let mut m = model_with_policy();
        m.screen = Screen::Policy;
        let mut pending = pce::models::parse_pending(
            serde_json::from_str(include_str!("../../../../fixtures/pending_list.json")).unwrap(),
        );
        pending.push(pce::models::PendingChange {
            href: "/orgs/5/sec_policy/draft/label_groups/7".into(),
            change_type: "update".into(),
            name: Some("G".into()),
            object_type: "label_groups".into(),
        });
        update(&mut m, Msg::PendingLoaded(Ok(pending.clone())));
        assert_eq!(m.policy.pending_selected, 0);
        assert!(update(&mut m, enter()).is_empty(), "entry 0 is a ruleset");
        assert_eq!(m.status.text, "rule diffs arrive with Plan 5");
        assert_eq!(m.overlay, Overlay::Pending);
        update(&mut m, key('j')); // Corp
        let cmds = update(&mut m, enter());
        assert_eq!(
            cmds,
            vec![Cmd::LoadDiff {
                change: pending[1].clone(),
                kind: pce::write::ObjectKind::IpList
            }]
        );
        assert!(m.policy.diff_loading);
        assert!(update(&mut m, enter()).is_empty(), "one diff in flight at a time");
        assert_eq!(m.status.text, "diff already loading");
        m.policy.diff_loading = false;
        update(&mut m, key('j'));
        update(&mut m, key('j'));
        assert_eq!(m.policy.pending_selected, 2, "stops at the last entry");
        assert!(update(&mut m, enter()).is_empty());
        assert_eq!(m.status.text, "no diff for label_groups");
        update(&mut m, key('k'));
        assert_eq!(m.policy.pending_selected, 1);
        update(&mut m, kc(KeyCode::Esc));
        assert_eq!(m.overlay, Overlay::None);
    }

    #[test]
    fn diff_loaded_opens_the_overlay_esc_returns_and_q_closes() {
        use crate::tui::diff::{DiffKind, DiffLine, ObjectDiff, Sign};
        use crate::tui::test_support::model_with_policy;
        let mut m = model_with_policy();
        m.policy.pending = Some(vec![]);
        m.overlay = Overlay::Pending;
        m.policy.diff_loading = true;
        let diff = ObjectDiff {
            name: "Corp".into(),
            href: "/orgs/5/sec_policy/draft/ip_lists/21".into(),
            change: DiffKind::Created,
            lines: vec![
                DiffLine {
                    sign: Sign::Added,
                    text: "name: Corp".into(),
                },
                DiffLine {
                    sign: Sign::Added,
                    text: "range 10.1.0.0-10.1.255.255".into(),
                },
            ],
        };
        assert!(update(&mut m, Msg::DiffLoaded(Ok(diff.clone()))).is_empty());
        assert!(!m.policy.diff_loading);
        assert_eq!(
            m.overlay,
            Overlay::Diff {
                diff: diff.clone(),
                scroll: 0
            }
        );
        update(&mut m, key('j'));
        update(&mut m, key('j'));
        assert_eq!(
            m.overlay,
            Overlay::Diff {
                diff: diff.clone(),
                scroll: 1
            },
            "stops at the last line"
        );
        update(&mut m, kc(KeyCode::Esc));
        assert_eq!(m.overlay, Overlay::Pending, "Esc returns to the list");
        update(&mut m, Msg::DiffLoaded(Ok(diff)));
        update(&mut m, key('q'));
        assert_eq!(m.overlay, Overlay::None, "q closes both");
        m.overlay = Overlay::Pending;
        update(
            &mut m,
            Msg::DiffLoaded(Err("not found: /orgs/5/sec_policy/active/ip_lists".into())),
        );
        assert_eq!(m.status.kind, StatusKind::Error);
        assert_eq!(m.overlay, Overlay::Pending);
    }
```

Append to `crates/illumio-tui/tests/tui_snapshots.rs`:

```rust
#[test]
fn pending_diff_overlay_80x24() {
    use illumio_tui::tui::diff::{DiffKind, ObjectDiff, diff_ip_list};
    use illumio_tui::tui::model::{Overlay, Screen};
    use illumio_tui::tui::test_support::model_with_policy;
    let mut m = model_with_policy();
    m.screen = Screen::Policy;
    let lists: Vec<pce::models::IpList> =
        serde_json::from_str(include_str!("../../../fixtures/ip_lists.json")).unwrap();
    let mut draft = lists[1].clone();
    draft.description = Some("Office and lab".into());
    draft
        .ip_ranges
        .push(serde_json::from_value(serde_json::json!({"from_ip": "10.2.0.0/16"})).unwrap());
    m.overlay = Overlay::Diff {
        diff: ObjectDiff {
            name: "Corp".into(),
            href: draft.href.clone(),
            change: DiffKind::Updated,
            lines: diff_ip_list(Some(&lists[1]), Some(&draft)),
        },
        scroll: 0,
    };
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("IP list Corp · updated"), "{out}");
    assert!(out.contains("+ range 10.2.0.0/16"), "{out}");
    assert!(out.contains("- description: Office ranges"), "{out}");
    insta::assert_snapshot!("pending_diff_80x24", out);
    m.set_theme("light");
    insta::assert_snapshot!("pending_diff_light_styled_80x24", render_styled(&m, 80, 24));
}
```

Append to `crates/illumio-tui/tests/tui_runtime.rs`:

```rust
#[tokio::test]
async fn pending_diff_fetches_the_active_list_once() {
    use illumio_tui::tui::diff::{DiffKind, Sign};
    use illumio_tui::tui::model::Overlay;
    let server = MockServer::start().await;
    mount_objects(&server).await; // the draft list: the startup load only
    for (p, body, n) in [
        (
            "/api/v2/orgs/5/sec_policy/draft/rule_sets",
            include_str!("../../../fixtures/rule_sets.json").to_string(),
            1,
        ),
        (
            "/api/v2/orgs/5/sec_policy/pending",
            include_str!("../../../fixtures/pending_list.json").to_string(),
            1,
        ),
        (
            "/api/v2/orgs/5/sec_policy/active/ip_lists",
            json!([{"href": "/orgs/5/sec_policy/active/ip_lists/20", "name": "Any (0.0.0.0/0 and ::/0)",
                    "ip_ranges": [{"from_ip": "0.0.0.0/0"}, {"from_ip": "::/0"}]}])
            .to_string(),
            1,
        ),
        (
            "/api/v2/orgs/5/sec_policy/draft/rule_sets/100/deny_rules",
            "[]".to_string(),
            0,
        ),
    ] {
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(body)
                    .insert_header("Content-Type", "application/json"),
            )
            .expect(n)
            .mount(&server)
            .await;
    }
    let keys = vec![
        (500, ch('2')),
        (300, ch('p')),
        (300, ch('j')), // Corp, a pending create
        (50, KeyCode::Enter),
        (300, KeyCode::Esc),
        (50, KeyCode::Enter),
        (300, KeyCode::F(12)),
    ];
    let model = key_session(&server, model(), None, keys).await;
    match &model.overlay {
        Overlay::Diff { diff, scroll } => {
            assert_eq!(diff.name, "Corp");
            assert_eq!(diff.change, DiffKind::Created);
            assert!(diff.lines.iter().all(|l| l.sign == Sign::Added), "{diff:?}");
            assert_eq!(*scroll, 0);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        model.stats.calls, 8,
        "health + four lists + rulesets + pending + one active list; the second Enter is free"
    );
    assert_eq!(model.busy, 0);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p illumio-tui --lib tui::diff`
Expected: FAIL, compile error (`mod diff` not declared / `diff_ip_list` not found).

- [ ] **Step 3: Implement the diff** (top of `crates/illumio-tui/src/tui/diff.rs`; add `pub mod diff;` to `crates/illumio-tui/src/tui/mod.rs`)

```rust
//! What a pending IP list or service changes against its active version.
//! Pure: the runtime loads both lists, these functions compare them. Every
//! entry is listed, unchanged ones with `Sign::Same`, so the reader sees the
//! whole object.
use pce::models::{IpList, PendingChange, Service};
use pce::write::IpRangeSpec;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffKind {
    Created,
    Updated,
    Deleted,
}

impl DiffKind {
    pub fn from_change_type(change_type: &str) -> Self {
        match change_type {
            "create" => DiffKind::Created,
            "delete" => DiffKind::Deleted,
            _ => DiffKind::Updated,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            DiffKind::Created => "created",
            DiffKind::Updated => "updated",
            DiffKind::Deleted => "deleted",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sign {
    Same,
    Added,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub sign: Sign,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectDiff {
    pub name: String,
    pub href: String,
    pub change: DiffKind,
    pub lines: Vec<DiffLine>,
}

/// One `label: value` line, or a removed and an added line when it changed.
fn field(out: &mut Vec<DiffLine>, label: &str, active: Option<&str>, draft: Option<&str>) {
    let line = |sign, v: &str| DiffLine {
        sign,
        text: format!("{label}: {v}"),
    };
    match (active, draft) {
        (Some(a), Some(d)) if a == d => out.push(line(Sign::Same, d)),
        (a, d) => {
            if let Some(a) = a {
                out.push(line(Sign::Removed, a));
            }
            if let Some(d) = d {
                out.push(line(Sign::Added, d));
            }
        }
    }
}

/// Draft entries in order (same or added), then entries only the active side has.
fn entries(out: &mut Vec<DiffLine>, label: &str, active: &[String], draft: &[String]) {
    for d in draft {
        let sign = if active.contains(d) {
            Sign::Same
        } else {
            Sign::Added
        };
        out.push(DiffLine {
            sign,
            text: format!("{label} {d}"),
        });
    }
    for a in active.iter().filter(|a| !draft.contains(a)) {
        out.push(DiffLine {
            sign: Sign::Removed,
            text: format!("{label} {a}"),
        });
    }
}

fn non_empty(s: &Option<String>) -> Option<&str> {
    s.as_deref().filter(|d| !d.is_empty())
}

fn ranges(l: &IpList) -> Vec<String> {
    l.ip_ranges
        .iter()
        .map(|r| IpRangeSpec::from_range(r).display())
        .collect()
}

fn fqdns(l: &IpList) -> Vec<String> {
    l.fqdns.iter().map(|f| f.fqdn.clone()).collect()
}

fn ports(s: &Service) -> Vec<String> {
    s.service_ports.iter().map(|p| p.display()).collect()
}

/// `None` on one side: a created (no active) or deleted (no draft) object.
pub fn diff_ip_list(active: Option<&IpList>, draft: Option<&IpList>) -> Vec<DiffLine> {
    let mut out = Vec::new();
    field(
        &mut out,
        "name",
        active.map(|l| l.name.as_str()),
        draft.map(|l| l.name.as_str()),
    );
    field(
        &mut out,
        "description",
        active.and_then(|l| non_empty(&l.description)),
        draft.and_then(|l| non_empty(&l.description)),
    );
    entries(
        &mut out,
        "range",
        &active.map(ranges).unwrap_or_default(),
        &draft.map(ranges).unwrap_or_default(),
    );
    entries(
        &mut out,
        "fqdn",
        &active.map(fqdns).unwrap_or_default(),
        &draft.map(fqdns).unwrap_or_default(),
    );
    out
}

pub fn diff_service(active: Option<&Service>, draft: Option<&Service>) -> Vec<DiffLine> {
    let mut out = Vec::new();
    field(
        &mut out,
        "name",
        active.map(|s| s.name.as_str()),
        draft.map(|s| s.name.as_str()),
    );
    field(
        &mut out,
        "description",
        active.and_then(|s| non_empty(&s.description)),
        draft.and_then(|s| non_empty(&s.description)),
    );
    entries(
        &mut out,
        "port",
        &active.map(ports).unwrap_or_default(),
        &draft.map(ports).unwrap_or_default(),
    );
    out
}

/// The same object in both scopes: active hrefs differ from draft hrefs only
/// by the `sec_policy/<scope>` segment.
fn draft_href(href: &str) -> String {
    href.replacen("/sec_policy/active/", "/sec_policy/draft/", 1)
}

trait Diffable {
    fn href(&self) -> &str;
    fn name(&self) -> &str;
}

impl Diffable for IpList {
    fn href(&self) -> &str {
        &self.href
    }
    fn name(&self) -> &str {
        &self.name
    }
}

impl Diffable for Service {
    fn href(&self) -> &str {
        &self.href
    }
    fn name(&self) -> &str {
        &self.name
    }
}

fn diff_for<T: Diffable>(
    change: &PendingChange,
    draft: &[T],
    active: &[T],
    diff: fn(Option<&T>, Option<&T>) -> Vec<DiffLine>,
) -> ObjectDiff {
    let key = draft_href(&change.href);
    let d = draft.iter().find(|x| draft_href(x.href()) == key);
    let a = active.iter().find(|x| draft_href(x.href()) == key);
    let kind = DiffKind::from_change_type(&change.change_type);
    let name = change
        .name
        .clone()
        .or_else(|| d.map(|x| x.name().to_string()))
        .or_else(|| a.map(|x| x.name().to_string()))
        .unwrap_or_else(|| change.href.clone());
    let missing = match kind {
        DiffKind::Created | DiffKind::Updated if d.is_none() => Some("draft"),
        DiffKind::Deleted | DiffKind::Updated if a.is_none() => Some("active"),
        _ => None,
    };
    let lines = match (missing, kind) {
        (Some(side), _) => vec![DiffLine {
            sign: Sign::Same,
            text: format!("{name} is not in the {side} list"),
        }],
        (None, DiffKind::Created) => diff(None, d),
        (None, DiffKind::Deleted) => diff(a, None),
        (None, DiffKind::Updated) => diff(a, d),
    };
    ObjectDiff {
        name,
        href: change.href.clone(),
        change: kind,
        lines,
    }
}

pub fn ip_list_diff_for(change: &PendingChange, draft: &[IpList], active: &[IpList]) -> ObjectDiff {
    diff_for(change, draft, active, diff_ip_list)
}

pub fn service_diff_for(
    change: &PendingChange,
    draft: &[Service],
    active: &[Service],
) -> ObjectDiff {
    diff_for(change, draft, active, diff_service)
}
```

- [ ] **Step 4: Model, messages, commands**

`crates/illumio-tui/src/tui/model.rs`: add `use crate::tui::diff::ObjectDiff;`. In `pub struct PolicyState`, after `pub pending_loading: bool,`:

```rust
    /// The highlighted entry of the pending overlay.
    pub pending_selected: usize,
    /// A diff request is in flight (`Enter` does not send another).
    pub diff_loading: bool,
```

and in `PolicyState::new`, after `pending_loading: false,`: `pending_selected: 0,` and `diff_loading: false,`. In `pub enum Overlay`, after `ConfirmWrite { … },`:

```rust
    /// A pending IP list or service against its active version.
    Diff {
        diff: ObjectDiff,
        scroll: u16,
    },
```

and in `impl PartialEq for Overlay`, before `_ => false,`:

```rust
            (
                Overlay::Diff {
                    diff: a,
                    scroll: sa,
                },
                Overlay::Diff {
                    diff: b,
                    scroll: sb,
                },
            ) => a == b && sa == sb,
```

`crates/illumio-tui/src/tui/msg.rs`: add `use crate::tui::diff::ObjectDiff;`. In `pub enum Msg`, after `WriteDone { … },`:

```rust
    DiffLoaded(Result<ObjectDiff, String>),
```

In `pub enum Cmd`, after `ExecuteWrite { … },`:

```rust
    /// The draft and active list of one kind (cached) for a pending entry.
    LoadDiff {
        change: PendingChange,
        kind: pce::write::ObjectKind,
    },
```

In `impl PartialEq for Cmd`, before `_ => false,`:

```rust
            (
                Cmd::LoadDiff { change: ca, kind: ka },
                Cmd::LoadDiff { change: cb, kind: kb },
            ) => ca == cb && ka == kb,
```

- [ ] **Step 5: Update** (`crates/illumio-tui/src/tui/update.rs`)

In the `Msg::PendingLoaded` arm, inside `Ok(p) => {`, before `model.policy.pending = Some(p);`:

```rust
                    model.policy.pending_selected = 0;
```

After the `Msg::WriteDone { … }` arm:

```rust
        Msg::DiffLoaded(result) => {
            model.policy.diff_loading = false;
            match result {
                Ok(diff) => model.overlay = Overlay::Diff { diff, scroll: 0 },
                Err(e) => fail(model, "diff", &e, false),
            }
            Vec::new()
        }
```

In `on_key`, change `Overlay::RuleDetail(_) | Overlay::Pending | Overlay::Error(_) => {` to `Overlay::RuleDetail(_) | Overlay::Error(_) => {` and add before it:

```rust
        Overlay::Pending => {
            let n = model.policy.pending.as_ref().map_or(0, Vec::len);
            match k.code {
                KeyCode::Esc | KeyCode::Char('q') => model.overlay = Overlay::None,
                KeyCode::Down | KeyCode::Char('j') => {
                    model.policy.pending_selected =
                        (model.policy.pending_selected + 1).min(n.saturating_sub(1));
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    model.policy.pending_selected = model.policy.pending_selected.saturating_sub(1);
                }
                KeyCode::Enter => return pending_diff(model),
                _ => {}
            }
            Vec::new()
        }
        Overlay::Diff { diff, scroll } => {
            let last = u16::try_from(diff.lines.len().saturating_sub(1)).unwrap_or(u16::MAX);
            match k.code {
                KeyCode::Esc => model.overlay = Overlay::Pending,
                KeyCode::Char('q') => model.overlay = Overlay::None,
                KeyCode::Down | KeyCode::Char('j') => {
                    model.overlay = Overlay::Diff {
                        diff,
                        scroll: (scroll + 1).min(last),
                    };
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    model.overlay = Overlay::Diff {
                        diff,
                        scroll: scroll.saturating_sub(1),
                    };
                }
                _ => {}
            }
            Vec::new()
        }
```

Add after `reload_after_write`:

```rust
/// `Enter` on the pending overlay: one diff request for an IP list or a
/// service; anything else says why there is none.
fn pending_diff(model: &mut Model) -> Vec<Cmd> {
    let Some(change) = model
        .policy
        .pending
        .as_ref()
        .and_then(|p| p.get(model.policy.pending_selected))
        .cloned()
    else {
        return Vec::new();
    };
    let kind = match change.object_type.as_str() {
        "ip_lists" => pce::write::ObjectKind::IpList,
        "services" => pce::write::ObjectKind::Service,
        "rule_sets" | "sec_rules" | "deny_rules" => {
            model.status = StatusLine::info("rule diffs arrive with Plan 5");
            return Vec::new();
        }
        other => {
            model.status = StatusLine::info(format!("no diff for {other}"));
            return Vec::new();
        }
    };
    if model.policy.diff_loading {
        model.status = StatusLine::info("diff already loading");
        return Vec::new();
    }
    model.policy.diff_loading = true;
    model.status = StatusLine::info(format!(
        "loading the active version of {}",
        change.name.as_deref().unwrap_or(&change.href)
    ));
    vec![Cmd::LoadDiff { change, kind }]
}
```

- [ ] **Step 6: Runtime** (`crates/illumio-tui/src/tui/app.rs`)

Imports: add `use crate::tui::diff::{self, ObjectDiff};` and `use pce::models::PendingChange;`. In `execute`, after the `Cmd::ExecuteWrite` arm:

```rust
            Cmd::LoadDiff { change, kind } => {
                self.model.busy += 1;
                let (pce, tx) = self.task_ctx();
                tokio::spawn(async move {
                    let result = load_diff(&pce, &change, kind)
                        .await
                        .map_err(|e| e.to_string());
                    let _ = tx.send(Msg::DiffLoaded(result));
                    let _ = tx.send(Msg::Stats(pce.stats()));
                });
            }
```

In `apply`, add `| Msg::DiffLoaded(_)` to the `task_result` list. Add after `named`:

```rust
/// The draft and active list of one kind through the cache, compared
/// locally. The draft list is usually cached by the name index, so the
/// common cost is one GET for the active list, and none when both are fresh.
async fn load_diff(
    pce: &Pce,
    change: &PendingChange,
    kind: pce::write::ObjectKind,
) -> pce::Result<ObjectDiff> {
    match kind {
        pce::write::ObjectKind::IpList => {
            let draft = pce.ip_lists(CacheMode::Use).await?.value;
            let active = pce.active_ip_lists(CacheMode::Use).await?.value;
            Ok(diff::ip_list_diff_for(change, &draft, &active))
        }
        pce::write::ObjectKind::Service => {
            let draft = pce.services(CacheMode::Use).await?.value;
            let active = pce.active_services(CacheMode::Use).await?.value;
            Ok(diff::service_diff_for(change, &draft, &active))
        }
        other => Err(pce::PceError::Invalid(format!(
            "no diff for {}",
            other.as_str()
        ))),
    }
}
```

- [ ] **Step 7: Render** (`crates/illumio-tui/src/tui/ui/widgets/overlay.rs`)

Add `use crate::tui::diff::{ObjectDiff, Sign};` to the imports. Replace `render_pending`:

```rust
pub fn render_pending(model: &Model, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let r = centered(area, 70, 16);
    f.render_widget(Clear, r);
    let pending = model.policy.pending.as_deref().unwrap_or(&[]);
    let rows = r.height.saturating_sub(2) as usize;
    let sel = model
        .policy
        .pending_selected
        .min(pending.len().saturating_sub(1));
    // Keep the selection in view.
    let first = sel.saturating_sub(rows.saturating_sub(1));
    let lines: Vec<Line> = if pending.is_empty() {
        vec![Line::from(Span::styled("none", t.dim_style()))]
    } else {
        pending
            .iter()
            .enumerate()
            .skip(first)
            .take(rows)
            .map(|(i, c)| {
                Line::from(vec![
                    Span::styled(if i == sel { "▸ " } else { "  " }, Style::new().fg(t.accent)),
                    Span::styled(format!("{:<16}", c.object_type), t.header_style()),
                    Span::styled(format!("{:<8}", c.change_type), Style::new().fg(t.warn)),
                    Span::styled(c.name.clone().unwrap_or_else(|| c.href.clone()), t.base()),
                ])
            })
            .collect()
    };
    let meta = if pending.is_empty() {
        "0 · Esc closes".to_string()
    } else {
        format!("{}/{} · Enter diff · Esc closes", sel + 1, pending.len())
    };
    f.render_widget(
        Paragraph::new(lines)
            .block(panel("Pending changes", &meta, t, true))
            .style(t.base()),
        r,
    );
}

pub fn render_diff(model: &Model, diff: &ObjectDiff, scroll: u16, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let r = centered(area, 76, (diff.lines.len() as u16).saturating_add(2).max(5));
    f.render_widget(Clear, r);
    let rows = r.height.saturating_sub(2) as usize;
    let first = (scroll as usize).min(diff.lines.len().saturating_sub(rows));
    let lines: Vec<Line> = diff
        .lines
        .iter()
        .skip(first)
        .take(rows)
        .map(|l| {
            let (sign, style) = match l.sign {
                Sign::Added => ("+ ", Style::new().fg(t.ok)),
                Sign::Removed => ("- ", Style::new().fg(t.bad)),
                Sign::Same => ("  ", t.dim_style()),
            };
            Line::from(Span::styled(format!("{sign}{}", plain(&l.text)), style))
        })
        .collect();
    let kind = match pce::models::Ref::new(&diff.href).object_type() {
        "ip_lists" => "IP list",
        "services" => "service",
        other => other,
    };
    let title = format!("{kind} {} · {}", plain(&diff.name), diff.change.as_str());
    let meta = if diff.lines.len() > rows {
        "j/k scroll · Esc back · q close"
    } else {
        "Esc back · q close"
    };
    f.render_widget(
        Paragraph::new(lines)
            .block(panel(&title, meta, t, true))
            .style(t.base()),
        r,
    );
}
```

In `crates/illumio-tui/src/tui/ui/mod.rs`, after the `Overlay::ConfirmWrite` dispatch arm:

```rust
        Overlay::Diff { diff, scroll } => {
            widgets::overlay::render_diff(model, diff, *scroll, f, area)
        }
```

- [ ] **Step 8: Run the tests, create and update the snapshots**

Run: `cargo test -p illumio-tui --lib tui::diff && cargo test -p illumio-tui --lib tui::update`
Expected: PASS (4 diff tests; all update tests including the two new ones).

Run: `cargo test -p illumio-tui --test tui_runtime pending_diff_fetches_the_active_list_once`
Expected: PASS; the active-list mock's `.expect(1)` and the deny-rules mock's `.expect(0)` hold.

Run: `INSTA_UPDATE=always cargo test -p illumio-tui --test tui_snapshots && git status --short crates/illumio-tui/tests/snapshots`
Expected: PASS; exactly `tui_snapshots__policy_pending_80x24.snap` modified (`▸ ` marker on the first entry, meta `1/2 · Enter diff · Esc closes`) and two new files, `tui_snapshots__pending_diff_80x24.snap` (title `IP list Corp · updated`, `+`/`-` lines) and `tui_snapshots__pending_diff_light_styled_80x24.snap` (ok colour on `+` lines, bad colour on `-` lines).

- [ ] **Step 9: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p illumio-tui
git add crates/illumio-tui/src/tui crates/illumio-tui/tests crates/illumio-tui/tests/snapshots
git commit -m "feat(tui): navigable pending list with a diff of draft against active

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---
### Task 9: Docs, tracking, final gate and request-free smoke

**Files:**
- Modify: `README.md` (`## Writes` section ~line 133: new `### Editing in the TUI` subsection before `## PCE protection` ~line 185)
- Modify: `docs/architecture.md` (`## Writes` ~line 90: new `### TUI editing` subsection before `## Testing` ~line 115; the TUI module table ~line 42: three rows)
- Modify: `docs/index.html` (the writes `<ul>` ~line 93: one `<li>`)
- Modify: `tasks/todo.md` (append the Plan 4 checklist and review)
- `tasks/lessons.md`: no change (it has no placeholder line; checked with `grep -n 'placeholder\|TBD' tasks/lessons.md`, no output)
- Create (scratchpad only, never committed): `$SCRATCH/plan4_smoke.py`, where `$SCRATCH` is the executing session's scratchpad directory

**Interfaces:**
- Consumes: the built binary `target/debug/illumio-tui`; `../illumio-mcp-server/.env` (credentials of the read-only test PCE demo100; read by `--env-file`, never copied, printed or committed).
- Produces: documentation only; no code changes.

- [ ] **Step 1: README** (insert before `## PCE protection`)

```markdown
### Editing in the TUI

With writes enabled for the profile and `illumio-tui --allow-writes`, the
header shows `· ✎ writes` and two screens take three keys: on Labels &
objects (labels, IP lists, services) and Workloads (unmanaged workloads only)
`n` creates, `e` edits the selected row and `x` deletes it. Without either
switch the keys print the reason in the status line and send nothing. Label
groups are read-only.

A form takes one line per field; lists are comma-separated in the CLI
grammar: ranges `10.0.0.0/8, 10.1.0.0-10.1.255.255, !10.0.5.0/24`, FQDNs
`*.corp.example`, ports `443/tcp, 8000-8100/udp, icmp`, addresses
`10.0.0.5, 10.0.0.6/24`, labels `app=web, env=prod` (resolved through the
loaded index, never with a request; a label created in the TUI joins the
index after the next `r` on Labels & objects). `Tab`/`Shift-Tab` move between fields,
`Ctrl-U` clears one, `Enter` checks the form, `Esc` leaves without a request.
An edit sends only what changed; an edit that changes nothing says
`no changes`.

The confirm overlay is the TUI's dry run: it shows the summary,
`METHOD path` and the JSON body (`j`/`k` scroll). `y` sends it once, `n` or
`Esc` cancels with `cancelled, nothing sent`. The status line then reports
`created <href> (pending)`, `updated <href>` or `deleted <href>`, and that
table reloads once. A rejected write (400/406/409/422) shows the PCE's
message and reloads nothing; an unanswered one says to check the PCE before
retrying and reloads. TUI writes go to the same audit log with
`"source": "tui"`.

On the Policy screen `p` lists pending changes; `j`/`k` select one, and
`Enter` on a pending IP list or service shows what it changes against the
active version (`+` added, `-` removed, unchanged entries dimmed). That costs
one GET for the active list of the kind, cached for five minutes.
```

- [ ] **Step 2: Architecture** (`docs/architecture.md`)

Add three rows to the TUI module table, after the `table` row:

```markdown
| `form` | the generic edit form (fields, keys, horizontal scroll) |
| `edit` | edit intents, pre-filled forms, form to library draft or patch |
| `diff` | draft against active for a pending IP list or service |
```

Insert before `## Testing`:

```markdown
### TUI editing

The model knows only `WritesState` (enabled, reason); the runtime owns
`Option<Arc<WriteGuard>>`, built in `run` and again on every `:profile`
switch from the same inputs as the CLI (profile `writes`, `--allow-writes`,
the JSONL audit log) with `source: "tui"`. A switch to a profile without
writes drops the guard and the header marker.

One edit, top to bottom: `e` on an IP list.

1. `update` checks `model.writes`, clones the selected list into
   `EditIntent::UpdateIpList` and opens `Overlay::Form` pre-filled by
   `edit::start`.
2. `Enter`: `edit::build_intent` diffs the fields against the original into
   an `IpListPatch` (`Ok(None)` when nothing changed) and returns
   `Cmd::PlanWrite`.
3. `app` spawns a task: `pce.writer(&guard).plan_update_ip_list` fetches the
   list once and returns the `Plan`; `Msg::Planned` opens
   `Overlay::ConfirmWrite` with summary, `METHOD path` and the pretty body.
4. `y`: `Cmd::ExecuteWrite`; the task calls `Writer::execute` once (audit,
   cache prefix invalidated) and drops `policy:pending`; `Msg::WriteDone`
   sets the status line and reloads the IP lists once with `Bypass`.

`n`/`Esc` on the confirm overlay or the form sends nothing. The pending
overlay keeps a selection; `Enter` sends `Cmd::LoadDiff` with the
`PendingChange`; the task reads the draft list and
`Pce::active_ip_lists`/`active_services` through the cache and
`tui/diff.rs` computes `+`/`-`/same lines, so a second diff of the same kind
within five minutes costs nothing.
```

- [ ] **Step 3: Website** (`docs/index.html`, in the writes `<ul>` after the CLI writes `<li>` ~line 94)

```html
    <li>Edits the same objects in the TUI: <code>n</code> new, <code>e</code> edit, <code>x</code> delete, a header marker while writes are on, the exact request on screen before <code>y</code> sends it once, and a pending-changes diff of a draft IP list or service against active.</li>
```

- [ ] **Step 4: Tracking** (append to `tasks/todo.md`)

```markdown
## Plan 4: TUI editing (labels, IP lists, services, unmanaged workloads; pending diff)
Spec: docs/superpowers/specs/2026-10-11-tui-editing-design.md · Plan: docs/superpowers/plans/2026-10-11-tui-editing.md
- [x] Task 1 Library: active IP lists and services
- [x] Task 2 Writes state, runtime guard and header marker
- [x] Task 3 Form overlay
- [x] Task 4 Edit intents and form adapters
- [x] Task 5 Editing keys, scrollable help, hints
- [x] Task 6 Plan and confirm
- [x] Task 7 Execute, report and reload
- [x] Task 8 Pending diff
- [x] Task 9 Docs, tracking, final gate and request-free smoke
```

Tasks 1-8 have landed when this task runs; Task 9 is ticked in the same commit as this file (Step 8).

- [ ] **Step 5: Final gate**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: PASS; note the test count for the review.

Run: `git diff --stat "$(git log --format=%H -1 --grep='cached reads of the active IP lists')^" HEAD -- crates/illumio-tui/src/cli crates/illumio-tui/tests/cli.rs crates/illumio-tui/tests/cli_writes.rs`
Expected: empty output (the CLI, its envelope and its tests are untouched by Plan 4).

- [ ] **Step 6: Request-free smoke against demo100**

No write may be sent to demo100. `PCE_LIVE_WRITES` stays unset, the script never sends the byte `y` (it asserts so before starting the binary), and every run uses a scratch `XDG_STATE_HOME`, so the real audit log is never touched.

Set up once (`$SCRATCH` is this session's scratchpad directory):

```bash
cargo build -p illumio-tui
python3 -m venv "$SCRATCH/venv" && "$SCRATCH/venv/bin/pip" install pyte
mkdir -p "$SCRATCH/state"
```

Create `$SCRATCH/plan4_smoke.py`:

```python
#!/usr/bin/env python3
"""Plan 4 smoke against the read-only test PCE. Sends no write: no step
contains 'y', and the audit log in the scratch state dir stays empty."""
import fcntl, os, pty, re, select, struct, sys, termios, time
import pyte

BIN, ENV_FILE, STATE = sys.argv[1], sys.argv[2], sys.argv[3]
COLS, ROWS = 100, 30
AUDIT = os.path.join(STATE, "illumio-tui", "writes.jsonl")


def run(args, extra_env, steps):
    for _, keys in steps:
        assert "y" not in keys and "Y" not in keys, "the smoke never presses y"
    pid, fd = pty.fork()
    if pid == 0:
        env = {k: v for k, v in os.environ.items() if k not in ("ILLUMIO_WRITES", "PCE_LIVE_WRITES")}
        env.update(extra_env)
        env["XDG_STATE_HOME"] = STATE
        os.execve(BIN, [BIN, "--env-file", ENV_FILE, *args], env)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
    screen = pyte.Screen(COLS, ROWS)
    stream = pyte.ByteStream(screen)

    def pump(seconds):
        end = time.time() + seconds
        while time.time() < end:
            ready, _, _ = select.select([fd], [], [], 0.05)
            if ready:
                try:
                    stream.feed(os.read(fd, 65536))
                except OSError:
                    return

    def calls():
        m = re.search(r"api (\d+) calls", screen.display[0])
        return int(m.group(1)) if m else -1

    pump(5)
    seen = [(calls(), list(screen.display))]
    for wait, keys in steps:
        for k in keys:
            os.write(fd, k.encode())
            pump(0.15)
        pump(wait)
        seen.append((calls(), list(screen.display)))
    for _ in range(3):
        os.write(fd, b"\x1b")
        pump(0.3)
    os.write(fd, b"q")
    pump(1)
    os.waitpid(pid, 0)
    return seen


def status(display):
    return display[ROWS - 1]


def audit_lines():
    if not os.path.exists(AUDIT):
        return 0
    with open(AUDIT) as f:
        return sum(1 for _ in f)


# A: profile allows (env), flag missing.
a = run([], {"ILLUMIO_WRITES": "1"}, [(1.5, "3"), (1.0, "n")])
assert "writes need --allow-writes" in status(a[2][1]), status(a[2][1])
assert a[1][0] == a[2][0], ("n made a request", a[1][0], a[2][0])
assert "✎ writes" not in a[2][1][0]
print("A ok: flag missing refused, calls", a[2][0])

# B: flag given, profile does not allow.
b = run(["--allow-writes"], {}, [(1.5, "3"), (1.0, "n")])
assert "writes are not enabled" in status(b[2][1]), status(b[2][1])
assert b[1][0] == b[2][0], ("n made a request", b[1][0], b[2][0])
print("B ok: profile refused, calls", b[2][0])

# C: writes on. Form opened and cancelled; a label planned (no request) and
# its confirm overlay cancelled with n; then the pending diff. Never y.
c = run(
    ["--allow-writes"],
    {"ILLUMIO_WRITES": "1"},
    [
        (1.5, "3"),
        (0.5, "n"),
        (0.5, "\x1b"),
        (1.0, "n" + "smoke-plan4" + "\t" + "never-sent" + "\r"),
        (0.5, "n"),
        (2.0, "2"),
        (2.0, "p"),
        (2.0, "\r"),
        (0.5, "\x1b"),
    ],
)
assert "✎ writes" in c[1][1][0], c[1][1][0]
assert any("New label" in line for line in c[2][1]), "form did not open"
assert "cancelled, nothing sent" in status(c[3][1]), status(c[3][1])
confirm = "\n".join(c[4][1])
assert "POST /orgs/" in confirm and "smoke-plan4" in confirm, confirm
assert "cancelled, nothing sent" in status(c[5][1]), status(c[5][1])
assert c[1][0] == c[5][0], ("the form or the plan made a request", c[1][0], c[5][0])
print("C ok: form, confirm and cancel cost no request; calls before Policy", c[5][0])
print("C pending diff screen:\n" + "\n".join(c[8][1]))
print("C final calls", c[-1][0])
assert audit_lines() == 0, "an audit line exists: something was sent"
print("audit log lines:", audit_lines())
```

Run it:

```bash
"$SCRATCH/venv/bin/python" -I "$SCRATCH/plan4_smoke.py" target/debug/illumio-tui ../illumio-mcp-server/.env "$SCRATCH/state"
```

Expected: `A ok`, `B ok`, `C ok`, the pending overlay or a diff overlay printed for step 8 (whatever demo100 has pending; a rule set entry prints `rule diffs arrive with Plan 5` in the status line instead), and `audit log lines: 0`. If any assertion fails, stop and debug (systematic-debugging); do not retry against the PCE in a loop.

- [ ] **Step 7: Review section** (append to `tasks/todo.md` under the Plan 4 checklist)

Write `## Review (Plan 4)` with: what shipped (one line per user-facing feature), the final gate's test count from Step 5, the smoke's request counts for A, B and C from Step 6 output, the statement that no write was sent to any PCE (the TUI write path is proven by the wiremock runtime tests in `tests/tui_runtime.rs` only), and any deviation reported by a task.

- [ ] **Step 8: Commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add README.md docs/architecture.md docs/index.html tasks/todo.md
git commit -m "docs: TUI editing, confirm overlay and pending diff; Plan 4 tracking and review

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```
