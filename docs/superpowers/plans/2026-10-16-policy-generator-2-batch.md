# Policy Generator (2 of 3): the write batch and its confirm Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** One confirmed write for the generator's whole proposal: `pce::write::batch` plans an ordered list of requests whose later steps name the hrefs earlier ones create (IP lists, rulesets, rule updates, rule creates), with the fresh reads that make it safe (one draft ruleset read that re-runs dedupe and the create-if-missing check, one IP list read, one GET per rule update), and sends it one request at a time; the TUI gets `Cmd::PlanBatch` / `Msg::BatchPlanned`, `Overlay::ConfirmBatch` (every request, `y` sends them all) and `Cmd::ExecuteBatch` / `Msg::BatchDone` (one status line, one reload, one draft re-evaluation per batch).

**Architecture:** The planner lives next to the other planners in `pce::write` and takes the `pce::generate::Proposal` of Plan 1 (`BatchInput { proposal, ruleset_name, picked }`): it calls `Proposal::refresh` on the fresh draft read, so a rule added since the cached read is not duplicated and a ruleset created since is found, then builds `Plan`s through the existing planners (`plan_create_rule`, `plan_update_rule`) or, for a rule into a ruleset the batch creates, a `POST {step:N}/sec_rules`. `Writer::execute_batch` resolves each step's placeholders from the earlier results and calls `Writer::execute`. The TUI keeps Elm style: `update` emits `Cmd::PlanBatch` (Plan 3's `Enter`) and handles the answers; the runtime runs the planner and the executor through the session's write guard. The confirm's text is `Batch::lines()` (pure), drawn with the single confirm's width and wrapping.

**Tech Stack:** Rust 2024 (rust-version 1.88), tokio, ratatui 0.30, crossterm, serde_json, wiremock 0.6, insta (plain snapshot files). No new dependency.

**Spec:** `docs/superpowers/specs/2026-10-16-policy-generator-design.md` (binding), delivery item 2, sections "Writes: one batch, one confirm" and the batch part of "Data model". Builds on Plan 1 (`2026-10-16-policy-generator-1-derive.md`), which must be merged first. Plan 3 (`2026-10-16-policy-generator-3-tui.md`) opens the confirm from the generator and changes where it returns to.

**Tasks:** 3, in this order: 1 `pce::write::batch` (planning and executing, against a mock PCE); 2 the TUI's batch messages, runtime, confirm overlay and answer; 3 tracking and the final gate.

## Global Constraints

- The gate is `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`. Run `cargo fmt --all` first and accept its formatting.
- No new dependencies. `git diff main -- '*Cargo.toml' Cargo.lock` must be empty.
- Never hammer the PCE: every request is caused by the user, throttled, cached where it is a read, counted and visible. Nothing refreshes on its own.
- Writes are gated (profile `writes = true`, audit log writable; the TUI after `:writes on`) and always confirmed; nothing is ever auto-confirmed. CLI `--yes` semantics stay as they are. This plan adds the TUI's batch write path; it is reached only through `Cmd::PlanBatch` (Plan 3's `Enter`) and sends only after `y` on the batch confirm. No CLI write mode (`gen --apply`) exists.
- Tests use recorded fixtures (`fixtures/`, `fixtures/recorded/`) and mocks (wiremock) only. Live smokes on the demo PCE are read-only: never `--yes`, never `y`, writes off; they run with `--env-file ../illumio-mcp-server/.env`. Never read or print that file.
- Verified on demo100 (2026-10-16): IP list consumers in a scoped ruleset appear with `unscoped_consumers: false`; cross-app inbound rules use `unscoped_consumers: true`. The derivation tables rely on both.
- A placeholder `{step:N}` never reaches the wire: the executor puts the earlier step's href in, checks it as the kind it stands for, and refuses the step (the rest not sent) when one is left.
- The batch is sent one request at a time through `Writer::execute` (audit, cache invalidation by `ObjectKind`, re-fetch on a 204, the client's limiter), stops at the first failure, is never retried and never rolled back.
- New rulesets are named by `[profiles.<name>.generator] ruleset_name`, default `"RF-{app}-{env}"` (`RF-shop-prod`). New IP lists are named `<app>-<env> peer <ip>` (`shop-prod peer 10.4.1.9`), one per IP peer.
- `gen` is a reserved keyword in Rust 2024: modules are `generate` (`pce::generate`, `cli::generate`), never `gen`.
- Every commit message ends with these two lines:
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx

## Spec deviations and resolutions (read before Task 1)

- **A ruleset to create whose name exists but does not cover its scope** is refused at plan time (`ruleset RF-crm-prod exists (…) but its scope does not cover crm/prod: rename it or set generator.ruleset_name`). The spec's create-if-missing would answer `Noop` with that ruleset's href and the rules would land in a ruleset scoped elsewhere. A covering ruleset never gets here: `refresh` makes it the target.
- **`plan_create_rule_sets_if_missing(&[RuleSetDraft])`** exists as specified (one read for all). The batch planner already holds the fresh read it dedupes against, so it calls the read-free `plan_rule_set_creates(drafts, sets)` with it: still one read. IP lists the same way (`plan_ip_list_creates(drafts, lists)`). `plan_create_rule_set` and `plan_create_ip_list` keep their behaviour and share the helpers.
- **Refusals at plan time** (nothing sent): a truncated fresh read; a rule to write whose target is ambiguous or has no ruleset (`Proposal::blocker`); every rule already in the draft.
- **Placeholders are spelled with steps counted from 1** (`<href of step 2: create rule_set RF-crm-prod>`), as the confirm numbers its blocks `2/6`.
- **`Msg::BatchDone` carries each step's `ObjectKind`** so the answer can reload the IP lists (the name index follows) only when one was written.
- **Where the confirm and its answer return to** is one function, `batch_closed` (here: the screen). The landing rule is one function, `batch_lands` (here: nothing else open). Plan 3 makes both the generator.
- **Statuses:** `review the requests: y sends them all, n cancels`; `sending 6 requests …`; `wrote 5 of 7 (pending); step 6 failed: …; 1 not sent`; after an unanswered step `step N not answered: it may have applied; reloading`.
- **No live smoke:** this plan's code writes, and the smokes are read-only. The batch is proven against wiremock and the TUI runtime against a mock PCE.

## Review Focus

- **A placeholder that names a step it does not wait for, or a step without an href,** must stop the batch before anything is sent: no request, no audit entry. Pinned in Task 1 (`a_placeholder_naming_a_step_it_does_not_wait_for_is_refused_before_sending`).
- **A rule someone added since the cached read** is found by the fresh read and not created twice. Pinned in Task 1 (`the_fresh_read_dedupes_again_and_a_truncated_one_is_refused`).
- **A ruleset with the new ruleset's name but another scope** is never written into. Pinned in Task 1 (`a_ruleset_named_like_the_new_one_but_scoped_elsewhere_is_refused`).
- **A `y` typed into another overlay while the batch is planned** never confirms it. Pinned in Task 2 (`a_batch_landing_over_another_overlay_is_discarded`).
- **An unanswered step** says it may have applied and reloads the policy once. Pinned in Task 2 (`an_unanswered_step_says_it_may_have_applied_and_reloads`).

---

### Task 1: `pce::write::batch`: plan and execute

**Files:**
- Create: `crates/pce/tests/batch.rs`
- Modify: `crates/pce/src/write/rule_sets.rs`
- Modify: `crates/pce/src/write/ip_lists.rs`
- Modify: `crates/pce/src/write/batch.rs`
- Modify: `crates/pce/src/write/mod.rs`

**Interfaces:**
- Consumes: Plan 1 (`pce::generate::{Proposal, GenActor, RuleAction, Target}`, `Proposal::refresh`, `blocker`, `writes`, `ip_list_step`, `ruleset_step`, `GenRule::to_draft`; `write::batch::{step_placeholder, parse_placeholder}`); `Writer::{execute, plan_create_rule, plan_update_rule}`; `RuleDraft::{validate, to_body}`; `checked_href`, `checked_policy_href`.
- Produces (`pce::write`, re-exported):

```rust
#[derive(Debug, Clone)] pub struct BatchStep { pub plan: Plan, pub kind: ObjectKind, pub needs: Vec<usize> }
#[derive(Debug, Clone, Default)] pub struct Batch { pub steps: Vec<BatchStep>, pub reads: u32 }
impl Batch {
    pub fn sends(&self) -> usize;                 // steps that send a request
    pub fn summary(&self) -> String;              // "6 requests: 1 ip_list, 1 rule_set, 4 rules · planned with 2 reads"
    pub fn spell(&self, text: &str) -> String;    // placeholders for people
    pub fn lines(&self) -> Vec<String>;           // the confirm's text
}
#[derive(Debug)] pub enum StepResult { Written(Written), Skipped { existing_href: String }, Failed(WriteError), NotSent }
#[derive(Debug, Clone, PartialEq, Eq)] pub struct BatchInput { pub proposal: Proposal, pub ruleset_name: String, pub picked: Option<String> }
impl Writer<'_> {
    pub fn plan_rule_set_creates(&self, drafts: &[RuleSetDraft], sets: &[RuleSet]) -> Result<Vec<Plan>, WriteError>;
    pub async fn plan_create_rule_sets_if_missing(&self, drafts: &[RuleSetDraft]) -> Result<Vec<Plan>, WriteError>;
    pub fn plan_ip_list_creates(&self, drafts: &[IpListDraft], lists: &[IpList]) -> Result<Vec<Plan>, WriteError>;
    pub async fn plan_batch(&self, input: BatchInput, idx: &NameIndex) -> Result<Batch, WriteError>;
    pub async fn execute_batch(&self, batch: Batch) -> Vec<StepResult>;
    pub(crate) fn rule_set_create(&self, draft: &RuleSetDraft, existing: Option<&[RuleSet]>) -> Result<Plan, WriteError>;
    pub(crate) fn ip_list_create(&self, draft: &IpListDraft, existing: Option<&[IpList]>) -> Result<Plan, WriteError>;
}
```

The test proposal: the rich fixture's shop/prod with the outbound section on and the IP peer 203.0.113.7 checked: two inbound rules and one outbound IP rule into Shop-Prod (100), a new IP list, a new ruleset RF-crm-prod and one rule into it.

- [ ] **Step 1: Write the failing tests**

Create `crates/pce/tests/batch.rs`:

```rust
//! The generator's write batch against a mock PCE: planning reads, step
//! order, placeholders, stopping, audit. The helpers repeat those of
//! write_contract.rs (test files share no code without a module).
use pce::client::{ClientConfig, PceClient};
use pce::generate::{GenInput, PeerKind, Proposal, Stop, propose, rows};
use pce::models::{Flow, IpList, Label, LabelGroup, PolicyScope, RuleSet, Service};
use pce::traffic::{Focus, parse_app, resolve_flows};
use pce::write::{
    Batch, BatchInput, BatchStep, MemoryAudit, ObjectKind, Plan, StepResult, WriteContext,
    WriteError, WriteGuard, WriteRequest,
};
use pce::{CacheMode, Credentials, NameIndex, Pce, WriteMethod};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn pce_for(server: &MockServer) -> Pce {
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

fn guard_with(audit: Arc<MemoryAudit>) -> WriteGuard {
    WriteGuard::new(
        true,
        Box::new(audit),
        WriteContext {
            profile: "lab".into(),
            host: "pce.example".into(),
            org_id: 5,
            source: "test".into(),
        },
    )
    .unwrap()
}

fn index() -> NameIndex {
    let mut labels: Vec<Label> =
        serde_json::from_str(include_str!("../../../fixtures/labels.json")).unwrap();
    let rich: Vec<Label> =
        serde_json::from_str(include_str!("../../../fixtures/traffic_labels_rich.json")).unwrap();
    labels.extend(rich);
    let groups: Vec<LabelGroup> =
        serde_json::from_str(include_str!("../../../fixtures/label_groups.json")).unwrap();
    let lists: Vec<IpList> =
        serde_json::from_str(include_str!("../../../fixtures/ip_lists.json")).unwrap();
    let services: Vec<Service> =
        serde_json::from_str(include_str!("../../../fixtures/services.json")).unwrap();
    NameIndex::new(&labels, &groups, &lists, &services)
}

fn rule_sets() -> Value {
    serde_json::from_str(include_str!("../../../fixtures/rule_sets.json")).unwrap()
}

/// shop/prod from the rich fixture, outbound on, the IP peer
/// 203.0.113.7 checked: rules into Shop-Prod (100), a new IP list and a
/// new ruleset RF-crm-prod.
fn proposal() -> Proposal {
    let idx = index();
    let raw: Vec<Flow> =
        serde_json::from_str(include_str!("../../../fixtures/traffic_flows_rich.json")).unwrap();
    let flows = resolve_flows(&raw, &idx);
    let focus = Focus::new(parse_app("shop/prod"), vec!["app".into(), "env".into()]);
    let mut r = rows(&flows, &focus, "role", &idx);
    for row in r.iter_mut() {
        if matches!(&row.peer, PeerKind::Ip { ip, .. } if ip == "203.0.113.7") {
            row.checked = true;
        }
    }
    let drafts: Vec<RuleSet> = serde_json::from_value(rule_sets()).unwrap();
    let input = GenInput {
        flows: &flows,
        focus: &focus,
        tier_key: "role",
        drafts: &drafts,
        idx: &idx,
        ruleset_name: "RF-{app}-{env}",
        window: "24h",
        outbound: true,
        picked: None,
    };
    propose(&r, Stop::AppPorts, &input)
}

fn input() -> BatchInput {
    BatchInput {
        proposal: proposal(),
        ruleset_name: "RF-{app}-{env}".into(),
        picked: None,
    }
}

async fn mount_reads(server: &MockServer, rule_sets: Value, ip_lists: Value) {
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(rule_sets))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/ip_lists"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ip_lists))
        .mount(server)
        .await;
}

fn ip_lists() -> Value {
    serde_json::from_str(include_str!("../../../fixtures/ip_lists.json")).unwrap()
}

fn shape(b: &Batch) -> Vec<String> {
    b.steps
        .iter()
        .map(|s| match &s.plan {
            Plan::Send { request, .. } => {
                format!("{} {} {:?}", request.method.as_str(), request.path, s.needs)
            }
            Plan::Noop { existing_href, .. } => format!("reuse {existing_href}"),
        })
        .collect()
}

async fn sent(server: &MockServer) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| format!("{} {}", r.method, r.url.path()))
        .collect()
}

async fn mount_created(server: &MockServer, p: &str, href: &str, times: u64) {
    Mock::given(method("POST"))
        .and(path(format!("/api/v2{p}")))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"href": href})))
        .expect(times)
        .mount(server)
        .await;
}

#[tokio::test]
async fn planning_reads_twice_orders_the_steps_and_sends_nothing() {
    let server = MockServer::start().await;
    mount_reads(&server, rule_sets(), ip_lists()).await;
    let pce = pce_for(&server);
    let audit = MemoryAudit::new();
    let guard = guard_with(audit.clone());
    let b = pce
        .writer(&guard)
        .plan_batch(input(), &index())
        .await
        .unwrap();
    assert_eq!(
        shape(&b),
        vec![
            "POST /orgs/5/sec_policy/draft/ip_lists []",
            "POST /orgs/5/sec_policy/draft/rule_sets []",
            "POST /orgs/5/sec_policy/draft/rule_sets/100/sec_rules []",
            "POST /orgs/5/sec_policy/draft/rule_sets/100/sec_rules []",
            "POST /orgs/5/sec_policy/draft/rule_sets/100/sec_rules [0]",
            "POST {step:1}/sec_rules [1]",
        ]
    );
    assert_eq!(b.reads, 2);
    assert_eq!(
        b.summary(),
        "6 requests: 1 ip_list, 1 rule_set, 4 rules · planned with 2 reads"
    );
    let lines = b.lines();
    assert!(
        lines.contains(
            &"6/6 POST <href of step 2: create rule_set RF-crm-prod>/sec_rules".to_string()
        ),
        "{lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.contains("<href of step 1: create ip_list shop-prod peer 203.0.113.7>"))
    );
    assert_eq!(
        sent(&server).await,
        vec![
            "GET /api/v2/orgs/5/sec_policy/draft/rule_sets",
            "GET /api/v2/orgs/5/sec_policy/draft/ip_lists"
        ],
        "the two fresh reads, no write"
    );
    assert!(audit.entries().is_empty());
}

#[tokio::test]
async fn executing_puts_each_created_href_in_and_audits_every_step() {
    let server = MockServer::start().await;
    mount_reads(&server, rule_sets(), ip_lists()).await;
    mount_created(
        &server,
        "/orgs/5/sec_policy/draft/ip_lists",
        "/orgs/5/sec_policy/draft/ip_lists/77",
        1,
    )
    .await;
    mount_created(
        &server,
        "/orgs/5/sec_policy/draft/rule_sets",
        "/orgs/5/sec_policy/draft/rule_sets/200",
        1,
    )
    .await;
    mount_created(
        &server,
        "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules",
        "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/9",
        3,
    )
    .await;
    Mock::given(method("POST"))
        .and(path(
            "/api/v2/orgs/5/sec_policy/draft/rule_sets/200/sec_rules",
        ))
        .and(body_partial_json(json!({"unscoped_consumers": true})))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(
                json!({"href": "/orgs/5/sec_policy/draft/rule_sets/200/sec_rules/1"}),
            ),
        )
        .expect(1)
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    let audit = MemoryAudit::new();
    let guard = guard_with(audit.clone());
    let w = pce.writer(&guard);
    let b = w.plan_batch(input(), &index()).await.unwrap();
    // Warm both caches: the writes must drop them.
    pce.rule_sets(PolicyScope::Draft, CacheMode::Use)
        .await
        .unwrap();
    let warm = pce
        .rule_sets(PolicyScope::Draft, CacheMode::Use)
        .await
        .unwrap();
    assert!(warm.hit);
    let out = w.execute_batch(b).await;
    assert_eq!(out.len(), 6);
    assert!(
        out.iter().all(|r| matches!(r, StepResult::Written(_))),
        "{out:?}"
    );
    let entries = audit.entries();
    assert_eq!(entries.len(), 6, "one audit entry per step sent");
    let ip_rule = &entries[4];
    assert_eq!(
        ip_rule.body.as_ref().unwrap()["providers"],
        json!([{"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/77"}}]),
        "the IP list step's href, put in before sending"
    );
    assert_eq!(
        entries[5].path,
        "/orgs/5/sec_policy/draft/rule_sets/200/sec_rules"
    );
    assert!(
        entries.iter().all(|e| !e.path.contains("{step:")
            && !e
                .body
                .as_ref()
                .is_some_and(|b| b.to_string().contains("{step:"))),
        "no placeholder reached the wire"
    );
    assert!(
        !pce.rule_sets(PolicyScope::Draft, CacheMode::Use)
            .await
            .unwrap()
            .hit,
        "policy: dropped"
    );
    server.verify().await;
}

#[tokio::test]
async fn an_existing_ip_list_is_reused_through_its_noop_href() {
    let server = MockServer::start().await;
    let mut lists = ip_lists();
    lists.as_array_mut().unwrap().push(json!({
        "href": "/orgs/5/sec_policy/draft/ip_lists/55", "name": "shop-prod peer 203.0.113.7",
        "ip_ranges": [{"from_ip": "203.0.113.7"}]
    }));
    mount_reads(&server, rule_sets(), lists).await;
    mount_created(
        &server,
        "/orgs/5/sec_policy/draft/rule_sets",
        "/orgs/5/sec_policy/draft/rule_sets/200",
        1,
    )
    .await;
    mount_created(
        &server,
        "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules",
        "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/9",
        3,
    )
    .await;
    mount_created(
        &server,
        "/orgs/5/sec_policy/draft/rule_sets/200/sec_rules",
        "/orgs/5/sec_policy/draft/rule_sets/200/sec_rules/1",
        1,
    )
    .await;
    let pce = pce_for(&server);
    let audit = MemoryAudit::new();
    let guard = guard_with(audit.clone());
    let w = pce.writer(&guard);
    let b = w.plan_batch(input(), &index()).await.unwrap();
    assert_eq!(shape(&b)[0], "reuse /orgs/5/sec_policy/draft/ip_lists/55");
    assert!(
        b.summary().ends_with("· reuses 1 object"),
        "{}",
        b.summary()
    );
    let out = w.execute_batch(b).await;
    assert!(
        matches!(&out[0], StepResult::Skipped { existing_href } if existing_href.ends_with("/55"))
    );
    let entries = audit.entries();
    assert_eq!(entries.len(), 5, "the reused list is not sent");
    assert_eq!(
        entries[3].body.as_ref().unwrap()["providers"],
        json!([{"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/55"}}])
    );
    server.verify().await;
}

#[tokio::test]
async fn the_first_failure_stops_the_batch_and_the_rest_is_not_sent() {
    let server = MockServer::start().await;
    mount_reads(&server, rule_sets(), ip_lists()).await;
    mount_created(
        &server,
        "/orgs/5/sec_policy/draft/ip_lists",
        "/orgs/5/sec_policy/draft/ip_lists/77",
        1,
    )
    .await;
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets"))
        .respond_with(ResponseTemplate::new(406).set_body_json(json!([{"token": "name_in_use"}])))
        .expect(1)
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    let audit = MemoryAudit::new();
    let guard = guard_with(audit.clone());
    let w = pce.writer(&guard);
    let b = w.plan_batch(input(), &index()).await.unwrap();
    let out = w.execute_batch(b).await;
    assert!(matches!(out[0], StepResult::Written(_)));
    assert!(
        matches!(out[1], StepResult::Failed(WriteError::Pce(_))),
        "{:?}",
        out[1]
    );
    assert!(out[2..].iter().all(|r| matches!(r, StepResult::NotSent)));
    assert_eq!(audit.entries().len(), 2, "the failure is audited too");
    server.verify().await;
}

#[tokio::test]
async fn an_unanswered_step_stops_the_batch() {
    let server = MockServer::start().await;
    mount_reads(&server, rule_sets(), ip_lists()).await;
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/ip_lists"))
        .respond_with(ResponseTemplate::new(504))
        .expect(1)
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    let audit = MemoryAudit::new();
    let guard = guard_with(audit.clone());
    let w = pce.writer(&guard);
    let b = w.plan_batch(input(), &index()).await.unwrap();
    let out = w.execute_batch(b).await;
    assert!(
        matches!(out[0], StepResult::Failed(WriteError::Unanswered(_))),
        "{:?}",
        out[0]
    );
    assert!(out[1..].iter().all(|r| matches!(r, StepResult::NotSent)));
    assert_eq!(audit.entries()[0].phase.as_deref(), Some("sent"));
    server.verify().await;
}

#[tokio::test]
async fn a_placeholder_naming_a_step_it_does_not_wait_for_is_refused_before_sending() {
    let server = MockServer::start().await;
    let pce = pce_for(&server);
    let audit = MemoryAudit::new();
    let guard = guard_with(audit.clone());
    let step = |path: &str, body: Option<Value>, needs: Vec<usize>| BatchStep {
        plan: Plan::Send {
            request: WriteRequest {
                method: WriteMethod::Post,
                path: path.into(),
                body,
                summary: "test".into(),
                objects: Vec::new(),
            },
            kind: ObjectKind::Rule,
        },
        kind: ObjectKind::Rule,
        needs,
    };
    let b = Batch {
        steps: vec![
            step("{step:0}/sec_rules", None, vec![0]),
            step(
                "/orgs/5/sec_policy/draft/rule_sets/1/sec_rules",
                Some(json!({"x": "{step:0}"})),
                vec![],
            ),
        ],
        reads: 0,
    };
    let out = pce.writer(&guard).execute_batch(b).await;
    let StepResult::Failed(WriteError::Invalid(m)) = &out[0] else {
        panic!("{out:?}")
    };
    assert!(
        m[0].contains("step 1 names step 1, which it does not wait for"),
        "{m:?}"
    );
    assert!(matches!(out[1], StepResult::NotSent));
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "nothing sent"
    );
    assert!(audit.entries().is_empty());
}

#[tokio::test]
async fn several_ruleset_creates_take_one_read() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(rule_sets()))
        .expect(1)
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    let guard = guard_with(MemoryAudit::new());
    let d = |name: &str| pce::write::RuleSetDraft {
        name: name.into(),
        description: None,
        enabled: true,
        scopes: vec![],
    };
    let plans = pce
        .writer(&guard)
        .plan_create_rule_sets_if_missing(&[d("RF-crm-prod"), d("RF-hr-prod"), d("SHOP-prod")])
        .await
        .unwrap();
    assert!(plans[0].request().is_some());
    assert!(plans[1].request().is_some());
    assert!(
        matches!(&plans[2], Plan::Noop { existing_href, .. } if existing_href.ends_with("/100")),
        "found by name, any case"
    );
    server.verify().await;
}

#[tokio::test]
async fn the_fresh_read_dedupes_again_and_a_truncated_one_is_refused() {
    // A rule someone added since: intra-app is in the draft now, the rest
    // still to write.
    let server = MockServer::start().await;
    let mut sets = rule_sets();
    sets[0]["rules"].as_array_mut().unwrap().push(json!({
        "href": "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/50", "enabled": true,
        "providers": [{"actors": "ams"}], "consumers": [{"actors": "ams"}],
        "ingress_services": [{"port": 5432, "proto": 6}], "unscoped_consumers": false
    }));
    mount_reads(&server, sets, ip_lists()).await;
    let pce = pce_for(&server);
    let guard = guard_with(MemoryAudit::new());
    let b = pce
        .writer(&guard)
        .plan_batch(input(), &index())
        .await
        .unwrap();
    assert_eq!(b.sends(), 5, "the intra-app rule exists now: not created");

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(rule_sets())
                .insert_header("X-Total-Count", "5000"),
        )
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    let err = pce
        .writer(&guard)
        .plan_batch(input(), &index())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("too many draft rulesets"), "{err}");
    assert_eq!(
        server.received_requests().await.unwrap().len(),
        1,
        "no IP list read after it"
    );
}

#[tokio::test]
async fn a_ruleset_named_like_the_new_one_but_scoped_elsewhere_is_refused() {
    let server = MockServer::start().await;
    let mut sets = rule_sets();
    sets.as_array_mut().unwrap().push(json!({
        "href": "/orgs/5/sec_policy/draft/rule_sets/300", "name": "RF-crm-prod", "enabled": true,
        "scopes": [[{"label": {"href": "/orgs/5/labels/3"}}]], "rules": []
    }));
    mount_reads(&server, sets, ip_lists()).await;
    let pce = pce_for(&server);
    let guard = guard_with(MemoryAudit::new());
    let err = pce
        .writer(&guard)
        .plan_batch(input(), &index())
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("ruleset RF-crm-prod exists (/orgs/5/sec_policy/draft/rule_sets/300) but its scope does not cover crm/prod"),
        "{err}"
    );
}

#[tokio::test]
async fn added_services_cost_one_get_of_the_rule_and_send_the_list_whole() {
    let server = MockServer::start().await;
    let rule = json!({
        "href": "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/50", "enabled": true,
        "providers": [{"actors": "ams"}], "consumers": [{"actors": "ams"}],
        "ingress_services": [{"port": 443, "proto": 6}], "unscoped_consumers": false
    });
    let mut sets = rule_sets();
    sets[0]["rules"] = json!([rule.clone()]);
    mount_reads(&server, sets, ip_lists()).await;
    Mock::given(method("GET"))
        .and(path(
            "/api/v2/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/50",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(rule))
        .expect(1)
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    let guard = guard_with(MemoryAudit::new());
    let b = pce
        .writer(&guard)
        .plan_batch(input(), &index())
        .await
        .unwrap();
    assert_eq!(b.reads, 3);
    let put = b
        .steps
        .iter()
        .find_map(|s| s.plan.request().filter(|r| r.method == WriteMethod::Put))
        .expect("an update");
    assert_eq!(
        put.path,
        "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/50"
    );
    assert_eq!(
        put.body,
        Some(json!({"ingress_services": [{"port": 443, "proto": 6}, {"port": 5432, "proto": 6}]}))
    );
    assert_eq!(
        shape(&b)[2],
        "PUT /orgs/5/sec_policy/draft/rule_sets/100/sec_rules/50 []",
        "updates before creates"
    );
    server.verify().await;
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p pce --test batch`
Expected: FAIL to compile: "unresolved imports `pce::write::Batch`, `pce::write::BatchInput`, `pce::write::BatchStep`, `pce::write::StepResult`".

- [ ] **Step 3: Write the implementation**

In `crates/pce/src/write/rule_sets.rs`, split `plan_create_rule_set` so the batch can plan several creates against one read:

```rust
        &self,
        draft: &RuleSetDraft,
        if_missing: IfMissing,
    ) -> Result<Plan, WriteError> {
        draft.validate().map_err(WriteError::Invalid)?;
        let name = draft.name.trim();
        if if_missing == IfMissing::SkipExisting {
            let sets = self
                .pce()
                .rule_sets(PolicyScope::Draft, self.bypass())
                .await?;
            if sets.truncated {
                return Err(WriteError::invalid(
                    "too many rulesets to check --if-missing; create without it",
                ));
            }
            let hits: Vec<&RuleSet> = sets
                .value
                .iter()
                .filter(|rs| {
                    rs.update_type.as_deref() != Some("delete")
```

with:

```rust
        &self,
        draft: &RuleSetDraft,
        if_missing: IfMissing,
    ) -> Result<Plan, WriteError> {
        draft.validate().map_err(WriteError::Invalid)?;
        if if_missing == IfMissing::Create {
            return self.rule_set_create(draft, None);
        }
        let sets = self
            .pce()
            .rule_sets(PolicyScope::Draft, self.bypass())
            .await?;
        if sets.truncated {
            return Err(WriteError::invalid(
                "too many rulesets to check --if-missing; create without it",
            ));
        }
        self.rule_set_create(draft, Some(&sets.value))
    }

    /// The create plan; with `existing` (a fresh read), a live ruleset of
    /// the same name (any case) is a `Noop` with its href, several are an
    /// error. No request.
    pub(crate) fn rule_set_create(
        &self,
        draft: &RuleSetDraft,
        existing: Option<&[RuleSet]>,
    ) -> Result<Plan, WriteError> {
        draft.validate().map_err(WriteError::Invalid)?;
        let name = draft.name.trim();
        if let Some(sets) = existing {
            let hits: Vec<&RuleSet> = sets
                .iter()
                .filter(|rs| {
                    rs.update_type.as_deref() != Some("delete")
```

In `crates/pce/src/write/ip_lists.rs`, the same for `plan_create_ip_list`:

```rust
        if_missing: IfMissing,
    ) -> Result<Plan, WriteError> {
        draft.validate().map_err(WriteError::Invalid)?;
        if if_missing == IfMissing::SkipExisting {
            let lists = self.pce().ip_lists(self.bypass()).await?.value;
            if let Some(existing) = lists
                .iter()
                .find(|l| l.update_type.as_deref() != Some("delete") && l.name == draft.name.trim())
            {
                return Ok(Plan::Noop {
                    existing_href: existing.href.clone(),
                    reason: format!("ip_list {} exists", draft.name.trim()),
                });
            }
        }
        Ok(Plan::Send {
            request: WriteRequest {
```

with:

```rust
        if_missing: IfMissing,
    ) -> Result<Plan, WriteError> {
        draft.validate().map_err(WriteError::Invalid)?;
        if if_missing == IfMissing::Create {
            return self.ip_list_create(draft, None);
        }
        let lists = self.pce().ip_lists(self.bypass()).await?.value;
        self.ip_list_create(draft, Some(&lists))
    }

    /// The create plan; with `existing` (a fresh read), a live list of
    /// exactly that name is a `Noop` with its href. No request.
    pub(crate) fn ip_list_create(
        &self,
        draft: &IpListDraft,
        existing: Option<&[IpList]>,
    ) -> Result<Plan, WriteError> {
        draft.validate().map_err(WriteError::Invalid)?;
        if let Some(found) = existing
            .into_iter()
            .flatten()
            .find(|l| l.update_type.as_deref() != Some("delete") && l.name == draft.name.trim())
        {
            return Ok(Plan::Noop {
                existing_href: found.href.clone(),
                reason: format!("ip_list {} exists", draft.name.trim()),
            });
        }
        Ok(Plan::Send {
            request: WriteRequest {
```

Replace the whole of `crates/pce/src/write/batch.rs` with:

```rust
//! A batch of writes whose later steps name what earlier ones create. A
//! step that needs such an href carries the placeholder `{step:N}`; the
//! executor puts the href in just before sending. A placeholder never
//! passes `checked_href`, so it cannot reach the wire.
//!
//! [`Writer::plan_batch`] turns a generator proposal into the batch: one
//! fresh draft ruleset read (dedupe again, create-if-missing for every
//! ruleset), one fresh IP list read when it creates IP lists, one GET per
//! rule update. [`Writer::execute_batch`] sends the steps one at a time
//! through [`Writer::execute`] and stops at the first failure.
use super::ip_lists::IpListDraft;
use super::rule_sets::RuleSetDraft;
use super::rules::RulePatch;
use super::{
    ObjectKind, Plan, WriteError, WriteRequest, Writer, Written, checked_href, checked_policy_href,
};
use crate::CacheMode;
use crate::client::WriteMethod;
use crate::generate::{GenActor, Proposal, RuleAction, Target};
use crate::models::{IpList, PolicyScope, RuleSet};
use crate::resolve::NameIndex;
use serde_json::Value;

/// `{step:3}`: the href step 3 creates (or finds).
pub fn step_placeholder(step: usize) -> String {
    format!("{{step:{step}}}")
}

/// The step a whole placeholder names: `{step:3}` → 3.
pub fn parse_placeholder(s: &str) -> Option<usize> {
    s.strip_prefix("{step:")?.strip_suffix('}')?.parse().ok()
}

/// Every step `s` names, in order of appearance.
fn placeholders_in(s: &str) -> Vec<usize> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(i) = rest.find("{step:") {
        let tail = &rest[i..];
        match tail.find('}') {
            Some(j) => {
                if let Some(n) = parse_placeholder(&tail[..=j]) {
                    out.push(n);
                }
                rest = &tail[j + 1..];
            }
            None => break,
        }
    }
    out
}

/// One write of a batch: its plan, its kind, the earlier steps whose
/// hrefs it carries.
#[derive(Debug, Clone)]
pub struct BatchStep {
    pub plan: Plan,
    pub kind: ObjectKind,
    pub needs: Vec<usize>,
}

/// The steps in sending order (IP lists, rulesets, rule updates, rule
/// creates) and the reads planning took.
#[derive(Debug, Clone, Default)]
pub struct Batch {
    pub steps: Vec<BatchStep>,
    pub reads: u32,
}

/// What became of one step.
#[derive(Debug)]
pub enum StepResult {
    Written(Written),
    /// A `Noop`: the object existed; its href serves later steps.
    Skipped {
        existing_href: String,
    },
    Failed(WriteError),
    /// An earlier step failed.
    NotSent,
}

/// What `Cmd::PlanBatch` carries: the proposal on screen and what its
/// targets need to be resolved again against the fresh read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchInput {
    pub proposal: Proposal,
    /// `generator.ruleset_name`.
    pub ruleset_name: String,
    /// The app ruleset `R` picked among several.
    pub picked: Option<String>,
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

impl Batch {
    /// The steps that send a request (not the `Noop`s).
    pub fn sends(&self) -> usize {
        self.steps
            .iter()
            .filter(|s| s.plan.request().is_some())
            .count()
    }

    /// `7 requests: 1 ip_list, 1 rule_set, 1 rule update, 4 rules ·
    /// planned with 3 reads`, and what is reused.
    pub fn summary(&self) -> String {
        let count = |kind: ObjectKind, method: WriteMethod| {
            self.steps
                .iter()
                .filter(|s| s.kind == kind && s.plan.request().is_some_and(|r| r.method == method))
                .count()
        };
        let parts: Vec<String> = [
            (
                count(ObjectKind::IpList, WriteMethod::Post),
                "ip_list",
                "ip_lists",
            ),
            (
                count(ObjectKind::RuleSet, WriteMethod::Post),
                "rule_set",
                "rule_sets",
            ),
            (
                count(ObjectKind::Rule, WriteMethod::Put),
                "rule update",
                "rule updates",
            ),
            (count(ObjectKind::Rule, WriteMethod::Post), "rule", "rules"),
        ]
        .into_iter()
        .filter(|(n, _, _)| *n > 0)
        .map(|(n, one, many)| plural(n, one, many))
        .collect();
        let mut s = format!(
            "{}: {} · planned with {}",
            plural(self.sends(), "request", "requests"),
            parts.join(", "),
            plural(self.reads as usize, "read", "reads")
        );
        let reused = self.steps.len() - self.sends();
        if reused > 0 {
            s.push_str(&format!(
                " · reuses {}",
                plural(reused, "object", "objects")
            ));
        }
        s
    }

    /// `text` with each placeholder spelled for a person: `<href of step
    /// 2: create rule_set RF-shop-prod>` (steps counted from 1), or the
    /// existing href a `Noop` found.
    pub fn spell(&self, text: &str) -> String {
        let mut out = text.to_string();
        for n in placeholders_in(text) {
            let what = match self.steps.get(n).map(|s| &s.plan) {
                Some(Plan::Send { request, .. }) => {
                    format!("<href of step {}: {}>", n + 1, request.summary)
                }
                Some(Plan::Noop { existing_href, .. }) => existing_href.clone(),
                None => format!("<href of missing step {}>", n + 1),
            };
            out = out.replace(&step_placeholder(n), &what);
        }
        out
    }

    /// The confirm's text: one block per step, `N/M METHOD path`, the
    /// summary and the body, placeholders spelled.
    pub fn lines(&self) -> Vec<String> {
        let n = self.steps.len();
        let mut out = Vec::new();
        for (i, s) in self.steps.iter().enumerate() {
            match &s.plan {
                Plan::Send { request, .. } => {
                    out.push(format!(
                        "{}/{n} {} {}",
                        i + 1,
                        request.method.as_str(),
                        self.spell(&request.path)
                    ));
                    out.push(format!("  {}", self.spell(&request.summary)));
                    match &request.body {
                        Some(b) => out.extend(
                            serde_json::to_string_pretty(b)
                                .unwrap_or_default()
                                .lines()
                                .map(|l| format!("  {}", self.spell(l))),
                        ),
                        None => out.push("  (no body)".into()),
                    }
                }
                Plan::Noop {
                    existing_href,
                    reason,
                } => out.push(format!("{}/{n} reuse {existing_href}: {reason}", i + 1)),
            }
            out.push(String::new());
        }
        out.pop();
        out
    }
}

/// A string with every placeholder replaced by `href_of`'s answer.
fn substitute_str(
    s: &str,
    href_of: &impl Fn(usize) -> Result<String, WriteError>,
) -> Result<String, WriteError> {
    let mut out = s.to_string();
    for n in placeholders_in(s) {
        out = out.replace(&step_placeholder(n), &href_of(n)?);
    }
    Ok(out)
}

fn substitute_value(
    v: &mut Value,
    href_of: &impl Fn(usize) -> Result<String, WriteError>,
) -> Result<(), WriteError> {
    match v {
        Value::String(s) => *s = substitute_str(s, href_of)?,
        Value::Array(a) => {
            for x in a {
                substitute_value(x, href_of)?;
            }
        }
        Value::Object(o) => {
            for x in o.values_mut() {
                substitute_value(x, href_of)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// The href a finished step left: written, or found by a `Noop`.
fn result_href(r: &StepResult) -> Option<&str> {
    match r {
        StepResult::Written(w) => Some(&w.href),
        StepResult::Skipped { existing_href } => Some(existing_href),
        StepResult::Failed(_) | StepResult::NotSent => None,
    }
}

/// Step `i`'s plan with the hrefs of the steps it needs put in, each
/// checked as the kind it stands for; refused when a placeholder names a
/// step it does not wait for, one without an href, or survives.
fn resolve_step(
    i: usize,
    step: BatchStep,
    done: &[StepResult],
    kinds: &[ObjectKind],
    org_id: u64,
) -> Result<Plan, WriteError> {
    let BatchStep { plan, kind, needs } = step;
    let Plan::Send {
        mut request,
        kind: plan_kind,
    } = plan
    else {
        return Ok(plan);
    };
    let href_of = |n: usize| -> Result<String, WriteError> {
        if n >= i || !needs.contains(&n) {
            return Err(WriteError::invalid(format!(
                "step {} names step {}, which it does not wait for: nothing more sent",
                i + 1,
                n + 1
            )));
        }
        let href = done.get(n).and_then(result_href).ok_or_else(|| {
            WriteError::invalid(format!(
                "step {} needs the href of step {}, which has none: nothing more sent",
                i + 1,
                n + 1
            ))
        })?;
        match kinds[n] {
            ObjectKind::RuleSet => checked_policy_href(href, "rule_sets", org_id),
            ObjectKind::IpList => checked_href(href, "ip_lists", org_id),
            other => Err(WriteError::invalid(format!(
                "step {} names a {} href: only rulesets and IP lists are named",
                i + 1,
                other.as_str()
            ))),
        }
    };
    request.path = substitute_str(&request.path, &href_of)?;
    if let Some(b) = request.body.as_mut() {
        substitute_value(b, &href_of)?;
    }
    let left = request.path.contains("{step:")
        || request
            .body
            .as_ref()
            .is_some_and(|b| b.to_string().contains("{step:"));
    if left {
        return Err(WriteError::invalid(format!(
            "step {} still holds a placeholder: nothing more sent",
            i + 1
        )));
    }
    debug_assert_eq!(plan_kind, kind);
    Ok(Plan::Send {
        request,
        kind: plan_kind,
    })
}

impl Writer<'_> {
    /// The create plan of each ruleset against `sets` (one read, already
    /// made): a live ruleset with the same name (any case) is a `Noop`
    /// with its href, several are an error. No request.
    pub fn plan_rule_set_creates(
        &self,
        drafts: &[RuleSetDraft],
        sets: &[RuleSet],
    ) -> Result<Vec<Plan>, WriteError> {
        drafts
            .iter()
            .map(|d| self.rule_set_create(d, Some(sets)))
            .collect()
    }

    /// [`Writer::plan_rule_set_creates`] after one fresh draft ruleset
    /// read for all of them; refused when the read is truncated.
    pub async fn plan_create_rule_sets_if_missing(
        &self,
        drafts: &[RuleSetDraft],
    ) -> Result<Vec<Plan>, WriteError> {
        let sets = self
            .pce()
            .rule_sets(PolicyScope::Draft, CacheMode::Bypass)
            .await?;
        if sets.truncated {
            return Err(WriteError::invalid(
                "too many rulesets to check which exist; nothing planned",
            ));
        }
        self.plan_rule_set_creates(drafts, &sets.value)
    }

    /// The create plan of each IP list against `lists` (one read, already
    /// made): a live list of exactly that name is a `Noop` with its href.
    pub fn plan_ip_list_creates(
        &self,
        drafts: &[IpListDraft],
        lists: &[IpList],
    ) -> Result<Vec<Plan>, WriteError> {
        drafts
            .iter()
            .map(|d| self.ip_list_create(d, Some(lists)))
            .collect()
    }

    /// The batch for `input`: the reads above, then the steps in sending
    /// order. Refused, with nothing sent, when the fresh read is
    /// truncated, a rule's target is ambiguous, nothing is left to write,
    /// or a ruleset to create has the name of one that does not cover its
    /// scope.
    pub async fn plan_batch(
        &self,
        input: BatchInput,
        idx: &NameIndex,
    ) -> Result<Batch, WriteError> {
        let BatchInput {
            mut proposal,
            ruleset_name,
            picked,
        } = input;
        let fresh = self
            .pce()
            .rule_sets(PolicyScope::Draft, CacheMode::Bypass)
            .await?;
        let mut reads = 1;
        if fresh.truncated {
            return Err(WriteError::invalid(
                "too many draft rulesets to prove the rules are not there yet; nothing planned",
            ));
        }
        proposal.refresh(&fresh.value, idx, &ruleset_name, picked.as_deref());
        if let Some(why) = proposal.blocker() {
            return Err(WriteError::invalid(why));
        }
        if !proposal.writes() {
            return Err(WriteError::invalid(
                "every proposed rule is in the draft already; nothing to write",
            ));
        }
        let mut steps = Vec::new();
        if !proposal.ip_lists.is_empty() {
            let lists = self.pce().ip_lists(CacheMode::Bypass).await?.value;
            reads += 1;
            for plan in self.plan_ip_list_creates(&proposal.ip_lists, &lists)? {
                steps.push(BatchStep {
                    plan,
                    kind: ObjectKind::IpList,
                    needs: Vec::new(),
                });
            }
        }
        let creates: Vec<(usize, &RuleSetDraft)> = proposal
            .targets
            .iter()
            .enumerate()
            .filter_map(|(t, x)| match x {
                Target::Create(d) => Some((t, d)),
                _ => None,
            })
            .collect();
        let drafts: Vec<RuleSetDraft> = creates.iter().map(|(_, d)| (*d).clone()).collect();
        for ((t, d), plan) in creates
            .iter()
            .zip(self.plan_rule_set_creates(&drafts, &fresh.value)?)
        {
            if let Plan::Noop { existing_href, .. } = &plan {
                return Err(WriteError::invalid(format!(
                    "ruleset {} exists ({existing_href}) but its scope does not cover {}: rename it or set generator.ruleset_name; nothing planned",
                    d.name,
                    proposal.scopes[*t].text()
                )));
            }
            steps.push(BatchStep {
                plan,
                kind: ObjectKind::RuleSet,
                needs: Vec::new(),
            });
        }
        for r in &proposal.rules {
            if let RuleAction::AddServices { rule_href, add } = &r.action {
                let patch = RulePatch {
                    add_services: add.clone(),
                    ..RulePatch::default()
                };
                let plan = self.plan_update_rule(rule_href, &patch).await?;
                reads += 1;
                steps.push(BatchStep {
                    plan,
                    kind: ObjectKind::Rule,
                    needs: Vec::new(),
                });
            }
        }
        for r in &proposal.rules {
            if r.action != RuleAction::Create {
                continue;
            }
            let draft = r.to_draft(&proposal);
            let mut needs: Vec<usize> = r
                .providers
                .iter()
                .chain(&r.consumers)
                .filter_map(|a| match a {
                    GenActor::NewIpList(i) => Some(proposal.ip_list_step(*i)),
                    GenActor::Ref(_) => None,
                })
                .collect();
            let plan = match (&proposal.targets[r.target], proposal.ruleset_step(r.target)) {
                (Target::Existing { href, .. }, _) => self.plan_create_rule(href, &draft)?,
                (Target::Create(d), Some(step)) => {
                    needs.push(step);
                    draft.validate().map_err(WriteError::Invalid)?;
                    Plan::Send {
                        request: WriteRequest {
                            method: WriteMethod::Post,
                            path: format!("{}/sec_rules", step_placeholder(step)),
                            body: Some(draft.to_body()),
                            summary: format!("create rule in rule_set {}", d.name.trim()),
                            objects: Vec::new(),
                        },
                        kind: ObjectKind::Rule,
                    }
                }
                _ => return Err(WriteError::invalid("a rule has no ruleset to go into")),
            };
            needs.sort_unstable();
            needs.dedup();
            steps.push(BatchStep {
                plan,
                kind: ObjectKind::Rule,
                needs,
            });
        }
        Ok(Batch { steps, reads })
    }

    /// Sends the steps in order, one [`Writer::execute`] each (audit,
    /// cache invalidation, re-fetch on a 204); the first failure stops the
    /// batch and the rest are `NotSent`. Never retried, nothing rolled
    /// back: the steps are draft objects.
    pub async fn execute_batch(&self, batch: Batch) -> Vec<StepResult> {
        let org = self.pce().client().org_id();
        let kinds: Vec<ObjectKind> = batch.steps.iter().map(|s| s.kind).collect();
        let mut out: Vec<StepResult> = Vec::with_capacity(batch.steps.len());
        for (i, step) in batch.steps.into_iter().enumerate() {
            if out.iter().any(|r| matches!(r, StepResult::Failed(_))) {
                out.push(StepResult::NotSent);
                continue;
            }
            let result = match resolve_step(i, step, &out, &kinds, org) {
                Err(e) => StepResult::Failed(e),
                Ok(plan) => match self.execute(plan).await {
                    Ok(super::Outcome::Written(w)) => StepResult::Written(w),
                    Ok(super::Outcome::Skipped { existing_href, .. }) => {
                        StepResult::Skipped { existing_href }
                    }
                    Err(e) => StepResult::Failed(e),
                },
            };
            out.push(result);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_round_trip_and_never_pass_as_hrefs() {
        assert_eq!(step_placeholder(3), "{step:3}");
        assert_eq!(parse_placeholder("{step:3}"), Some(3));
        assert_eq!(parse_placeholder("{step:x}"), None);
        assert_eq!(
            parse_placeholder("/orgs/5/sec_policy/draft/rule_sets/1"),
            None
        );
        assert!(checked_policy_href(&step_placeholder(1), "rule_sets", 5).is_err());
        assert!(checked_href(&step_placeholder(0), "ip_lists", 5).is_err());
        assert_eq!(
            placeholders_in("{step:1}/sec_rules and {step:0}"),
            vec![1, 0]
        );
    }
}
```

In `crates/pce/src/write/mod.rs`, re-export the batch:

```rust

pub use actors::{ActorKind, ActorRef, ServiceRef};
pub use audit::{AuditEntry, AuditSink, JsonlAudit, MemoryAudit, NullAudit};
pub use guard::{WriteContext, WriteGuard, WriteRefused};
pub use ip_lists::{IpListDraft, IpListPatch};
pub use labels::LabelDraft;
```

with:

```rust

pub use actors::{ActorKind, ActorRef, ServiceRef};
pub use audit::{AuditEntry, AuditSink, JsonlAudit, MemoryAudit, NullAudit};
pub use batch::{Batch, BatchInput, BatchStep, StepResult, step_placeholder};
pub use guard::{WriteContext, WriteGuard, WriteRefused};
pub use ip_lists::{IpListDraft, IpListPatch};
pub use labels::LabelDraft;
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p pce --test batch && cargo test -p pce --test policy_writes && cargo test -p pce --test write_contract && cargo test -p pce --lib write`
Expected: PASS: the 10 batch tests, and every existing ruleset and IP list create test (`--if-missing` unchanged).

- [ ] **Step 5: Commit**

```bash
git add crates/pce/src/write crates/pce/tests/batch.rs
git commit -m "pce::write::batch: plan_batch, execute_batch, create-if-missing for many

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

### Task 2: TUI: `Cmd::PlanBatch`, `Overlay::ConfirmBatch`, `Cmd::ExecuteBatch`, `Msg::BatchDone`

**Files:**
- Modify: `crates/illumio-tui/src/tui/test_support.rs`
- Modify: `crates/illumio-tui/src/tui/update.rs`
- Modify: `crates/illumio-tui/tests/tui_runtime.rs`
- Modify: `crates/illumio-tui/tests/tui_snapshots.rs`
- Modify: `crates/illumio-tui/src/tui/msg.rs`
- Modify: `crates/illumio-tui/src/tui/model.rs`
- Modify: `crates/illumio-tui/src/tui/app.rs`
- Modify: `crates/illumio-tui/src/tui/ui/widgets/overlay.rs`
- Modify: `crates/illumio-tui/src/tui/ui/mod.rs`

**Interfaces:**
- Consumes: Task 1 (`Batch`, `BatchInput`, `StepResult`, `Writer::plan_batch`, `execute_batch`, `Batch::{lines, summary, sends}`); `redraft_after_write`, `reload_after_write`, `reload_kind_after_write`, `is_unanswered`, `fail`; the runtime's `writer`, `task_ctx`, `apply` (`task_result` lowers `busy`).
- Produces (Plan 3 relies on these names):

```rust
// tui::msg
Cmd::PlanBatch { input: Box<BatchInput>, index: Arc<pce::NameIndex> }
Cmd::ExecuteBatch(Batch)
Msg::BatchPlanned(Result<Batch, String>)
Msg::BatchDone { kinds: Vec<pce::write::ObjectKind>, results: Vec<StepResult> }
// tui::model
Overlay::ConfirmBatch { batch: pce::write::Batch, scroll: u16 }
// tui::update (private; Plan 3 changes the bodies of the first three)
fn batch_lands(model: &Model) -> bool;                 // here: model.overlay == Overlay::None
fn batch_plan_failed(model: &mut Model, e: &str);      // here: fail(model, "plan", e, false)
fn batch_closed(model: &mut Model) -> Vec<Cmd>;        // here: Overlay::None, no command
fn batch_status(results: &[StepResult]) -> (String, bool);
fn batch_done(model: &mut Model, kinds: &[ObjectKind], results: &[StepResult]) -> Vec<Cmd>;
// tui::ui::widgets::overlay
pub fn render_confirm_batch(model: &Model, batch: &Batch, scroll: u16, f: &mut Frame, area: Rect);
pub fn confirm_batch_last_scroll(batch: &Batch, area: Rect) -> usize;
// tui::test_support
pub fn batch_plan() -> pce::write::Batch;   // ip_list, rule_set, rule into {step:1} naming {step:0}
```

`y` sends exactly one `Cmd::ExecuteBatch`; `n`/`Esc` close it with nothing sent; `j`/`k` scroll to the last line and stop. The guards of the single confirm hold: `writes.in_flight` from `PlanBatch` to `BatchPlanned` and from `y` to `BatchDone`; a plan that lands while another overlay is open is discarded.

- [ ] **Step 1: Write the failing tests**

Append to `crates/illumio-tui/src/tui/test_support.rs` the batch every TUI test uses:

```rust

/// A planned generator batch: an IP list, a ruleset, a rule in that new
/// ruleset (it waits for step 2's href) naming the new IP list.
pub fn batch_plan() -> pce::write::Batch {
    use pce::write::{BatchStep, ObjectKind, Plan, WriteRequest};
    let post = |path: &str, summary: &str, body: serde_json::Value, kind: ObjectKind| Plan::Send {
        request: WriteRequest {
            method: pce::WriteMethod::Post,
            path: path.into(),
            body: Some(body),
            summary: summary.into(),
            objects: Vec::new(),
        },
        kind,
    };
    pce::write::Batch {
        steps: vec![
            BatchStep {
                plan: post(
                    "/orgs/5/sec_policy/draft/ip_lists",
                    "create ip_list shop-prod peer 203.0.113.7",
                    serde_json::json!({"name": "shop-prod peer 203.0.113.7", "ip_ranges": [{"from_ip": "203.0.113.7"}], "fqdns": []}),
                    ObjectKind::IpList,
                ),
                kind: ObjectKind::IpList,
                needs: Vec::new(),
            },
            BatchStep {
                plan: post(
                    "/orgs/5/sec_policy/draft/rule_sets",
                    "create rule_set RF-crm-prod",
                    serde_json::json!({"name": "RF-crm-prod", "enabled": true, "scopes": [[{"label": {"href": "/orgs/5/labels/101"}}, {"label": {"href": "/orgs/5/labels/4"}}]]}),
                    ObjectKind::RuleSet,
                ),
                kind: ObjectKind::RuleSet,
                needs: Vec::new(),
            },
            BatchStep {
                plan: post(
                    "{step:1}/sec_rules",
                    "create rule in rule_set RF-crm-prod",
                    serde_json::json!({"enabled": true, "providers": [{"ip_list": {"href": "{step:0}"}}], "consumers": [{"actors": "ams"}], "ingress_services": [{"port": 443, "proto": 6}], "unscoped_consumers": false}),
                    ObjectKind::Rule,
                ),
                kind: ObjectKind::Rule,
                needs: vec![0, 1],
            },
        ],
        reads: 2,
    }
}
```

Append to `crates/illumio-tui/src/tui/update.rs`:

```rust

#[cfg(test)]
mod batch_tests {
    use super::*;
    use crate::tui::model::{Model, Overlay, StatusKind};
    use crate::tui::test_support::{batch_plan, model_with_flows, with_draft_loaded};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use pce::write::{ObjectKind, StepResult, Written};

    fn key(c: char) -> Msg {
        Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
    }

    fn written(href: &str) -> StepResult {
        StepResult::Written(Written {
            href: href.into(),
            pending: true,
            status: 201,
            object: serde_json::json!({"href": href}),
            dropped_fields: Vec::new(),
            warnings: Vec::new(),
        })
    }

    fn kinds() -> Vec<ObjectKind> {
        vec![ObjectKind::IpList, ObjectKind::RuleSet, ObjectKind::Rule]
    }

    fn armed() -> Model {
        let mut m = model_with_flows();
        m.writes = crate::tui::model::WritesState::on();
        m.writes.in_flight = true; // the plan is on its way
        m
    }

    #[test]
    fn a_planned_batch_opens_its_confirm_with_every_request() {
        let mut m = armed();
        assert_eq!(
            update(&mut m, Msg::BatchPlanned(Ok(batch_plan()))),
            Vec::new()
        );
        assert!(!m.writes.in_flight);
        let Overlay::ConfirmBatch { batch, scroll } = &m.overlay else {
            panic!("{:?}", m.overlay)
        };
        assert_eq!(*scroll, 0);
        assert_eq!(
            batch.summary(),
            "3 requests: 1 ip_list, 1 rule_set, 1 rule · planned with 2 reads"
        );
        assert_eq!(
            m.status.text,
            "review the requests: y sends them all, n cancels"
        );
    }

    #[test]
    fn a_batch_landing_over_another_overlay_is_discarded() {
        let mut m = armed();
        m.overlay = Overlay::Prompt { input: "y".into() };
        update(&mut m, Msg::BatchPlanned(Ok(batch_plan())));
        assert_eq!(m.overlay, Overlay::Prompt { input: "y".into() });
        assert_eq!(m.status.text, "requests discarded, nothing sent");
        assert!(!m.writes.in_flight);
    }

    #[test]
    fn y_sends_exactly_one_execute_batch_and_n_sends_nothing() {
        let mut m = armed();
        update(&mut m, Msg::BatchPlanned(Ok(batch_plan())));
        let cmds = update(&mut m, key('y'));
        assert_eq!(cmds.len(), 1);
        assert!(matches!(&cmds[0], Cmd::ExecuteBatch(b) if b.steps.len() == 3));
        assert!(m.writes.in_flight);
        assert_eq!(m.overlay, Overlay::None);
        assert_eq!(m.status.text, "sending 3 requests …");
        let mut m = armed();
        update(&mut m, Msg::BatchPlanned(Ok(batch_plan())));
        assert_eq!(update(&mut m, key('n')), Vec::new());
        assert_eq!(m.overlay, Overlay::None);
        assert_eq!(m.status.text, "cancelled, nothing sent");
    }

    #[test]
    fn j_scrolls_the_confirm_and_stops_at_the_end() {
        let mut m = armed();
        m.size = (80, 12);
        update(&mut m, Msg::BatchPlanned(Ok(batch_plan())));
        for _ in 0..100 {
            update(&mut m, key('j'));
        }
        let Overlay::ConfirmBatch { batch, scroll } = &m.overlay else {
            panic!()
        };
        let area = ratatui::layout::Rect::new(0, 0, 80, 12);
        assert_eq!(usize::from(*scroll), confirm_batch_last_scroll(batch, area));
        assert!(*scroll > 0);
    }

    #[test]
    fn a_failed_plan_sends_nothing_and_says_why() {
        let mut m = armed();
        assert_eq!(
            update(
                &mut m,
                Msg::BatchPlanned(Err("too many draft rulesets".into()))
            ),
            Vec::new()
        );
        assert_eq!(m.status.text, "plan: too many draft rulesets");
        assert_eq!(m.status.kind, StatusKind::Error);
    }

    #[test]
    fn a_partly_failed_batch_reports_each_step_and_reloads_once() {
        let mut m = with_draft_loaded(armed());
        // A known query: the draft is evaluated again without asking.
        m.traffic.data.as_mut().unwrap().hrefs =
            vec!["/orgs/5/traffic_flows/async_queries/q-1".into()];
        let results = vec![
            written("/orgs/5/sec_policy/draft/ip_lists/77"),
            StepResult::Failed(pce::write::WriteError::invalid("name in use")),
            StepResult::NotSent,
        ];
        let cmds = update(
            &mut m,
            Msg::BatchDone {
                kinds: kinds(),
                results,
            },
        );
        assert_eq!(
            m.status.text.split(" · ").next().unwrap(),
            "wrote 1 of 3 (pending); step 2 failed: name in use; 1 not sent"
        );
        assert_eq!(m.status.kind, StatusKind::Error);
        assert!(!m.writes.in_flight);
        let count = |f: fn(&Cmd) -> bool| cmds.iter().filter(|c| f(c)).count();
        assert_eq!(
            count(|c| matches!(c, Cmd::LoadRuleSets { .. })),
            1,
            "{cmds:?}"
        );
        assert_eq!(
            count(|c| matches!(c, Cmd::LoadIpLists(CacheMode::Bypass))),
            1
        );
        assert_eq!(
            count(|c| matches!(c, Cmd::RunDraft { .. })),
            1,
            "one redraft per batch"
        );
        assert_eq!(m.overlay, Overlay::None);
    }

    #[test]
    fn an_unanswered_step_says_it_may_have_applied_and_reloads() {
        let mut m = armed();
        let results = vec![
            StepResult::Failed(pce::write::WriteError::Unanswered(pce::PceError::Timeout(
                std::time::Duration::from_secs(30),
            ))),
            StepResult::NotSent,
            StepResult::NotSent,
        ];
        let cmds = update(
            &mut m,
            Msg::BatchDone {
                kinds: kinds(),
                results,
            },
        );
        assert!(
            m.status
                .text
                .starts_with("wrote 0 of 3 (pending); step 1 not answered: it may have applied; reloading; 2 not sent"),
            "{}",
            m.status.text
        );
        assert!(cmds.iter().any(|c| matches!(c, Cmd::LoadRuleSets { .. })));
    }

    #[test]
    fn a_batch_that_failed_first_reloads_nothing() {
        let mut m = armed();
        let results = vec![
            StepResult::Failed(pce::write::WriteError::invalid("bad")),
            StepResult::NotSent,
            StepResult::NotSent,
        ];
        let cmds = update(
            &mut m,
            Msg::BatchDone {
                kinds: kinds(),
                results,
            },
        );
        assert_eq!(cmds, Vec::new());
    }
}
```

Append to `crates/illumio-tui/tests/tui_runtime.rs`:

```rust

#[tokio::test]
async fn execute_batch_sends_every_step_in_order_with_the_created_hrefs() {
    use illumio_tui::tui::test_support::batch_plan;
    use wiremock::matchers::body_partial_json;
    let server = MockServer::start().await;
    for (p, href) in [
        (
            "/api/v2/orgs/5/sec_policy/draft/ip_lists",
            "/orgs/5/sec_policy/draft/ip_lists/77",
        ),
        (
            "/api/v2/orgs/5/sec_policy/draft/rule_sets",
            "/orgs/5/sec_policy/draft/rule_sets/200",
        ),
    ] {
        Mock::given(method("POST"))
            .and(path(p))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"href": href})))
            .expect(1)
            .mount(&server)
            .await;
    }
    Mock::given(method("POST"))
        .and(path(
            "/api/v2/orgs/5/sec_policy/draft/rule_sets/200/sec_rules",
        ))
        .and(body_partial_json(
            json!({"providers": [{"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/77"}}]}),
        ))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(
                json!({"href": "/orgs/5/sec_policy/draft/rule_sets/200/sec_rules/1"}),
            ),
        )
        .expect(1)
        .mount(&server)
        .await;
    // One reload each afterwards: the rulesets, the IP lists (one was written).
    for (p, body) in [
        (
            "/api/v2/orgs/5/sec_policy/draft/rule_sets",
            include_str!("../../../fixtures/rule_sets.json"),
        ),
        (
            "/api/v2/orgs/5/sec_policy/draft/ip_lists",
            include_str!("../../../fixtures/ip_lists.json"),
        ),
    ] {
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(body)
                    .insert_header("Content-Type", "application/json"),
            )
            .expect(1)
            .mount(&server)
            .await;
    }
    let audit = MemoryAudit::new();
    let (mut rt, _tx) =
        Runtime::with_writer(pce_for(&server), writes_model(), tui_guard(audit.clone()));
    rt.execute(Cmd::ExecuteBatch(batch_plan()));
    let model = run_loop(&mut rt, Box::pin(wait(500)), &mut |_| {}).await;
    assert!(
        model.status.text.starts_with("wrote 3 of 3 (pending)"),
        "{}",
        model.status.text
    );
    assert_eq!(audit.entries().len(), 3);
    assert!(audit.entries().iter().all(|e| e.source == "tui"));
    assert_eq!(model.busy, 0);
    server.verify().await;
}

#[tokio::test]
async fn plan_batch_needs_the_runtime_writer_and_sends_nothing() {
    use illumio_tui::tui::test_support::fixture_index;
    let server = MockServer::start().await;
    let (mut rt, _tx) = Runtime::new(pce_for(&server), writes_model());
    let input = pce::write::BatchInput {
        proposal: pce::generate::Proposal {
            scopes: Vec::new(),
            targets: Vec::new(),
            ip_lists: Vec::new(),
            rules: Vec::new(),
            coverage: pce::generate::Coverage::default(),
        },
        ruleset_name: "RF-{app}-{env}".into(),
        picked: None,
    };
    rt.execute(Cmd::PlanBatch {
        input: Box::new(input),
        index: fixture_index(),
    });
    let model = run_loop(&mut rt, Box::pin(wait(300)), &mut |_| {}).await;
    assert_eq!(model.status.text, "plan: writes are not enabled");
    assert!(
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty()
    );
}
```

Append to `crates/illumio-tui/tests/tui_snapshots.rs`:

```rust

#[test]
fn confirm_batch_80x24() {
    use illumio_tui::tui::model::Overlay;
    let mut m = model_fixture();
    m.overlay = Overlay::ConfirmBatch {
        batch: illumio_tui::tui::test_support::batch_plan(),
        scroll: 0,
    };
    let out = render_to_string(&m, 80, 24);
    for s in [
        "Send these requests?",
        "3 requests: 1 ip_list, 1 rule_set, 1 rule · planned with 2 reads",
        "1/3 POST /orgs/5/sec_policy/draft/ip_lists",
        "j/k scroll · y send all · n cancel",
    ] {
        assert!(out.contains(s), "missing {s:?}:\n{out}");
    }
    insta::assert_snapshot!("confirm_batch_80x24", out);
}

#[test]
fn confirm_batch_spells_placeholders_120x48() {
    use illumio_tui::tui::model::Overlay;
    let mut m = model_fixture();
    m.overlay = Overlay::ConfirmBatch {
        batch: illumio_tui::tui::test_support::batch_plan(),
        scroll: 0,
    };
    let out = render_to_string(&m, 120, 48);
    assert!(
        out.contains("<href of step 2: create rule_set RF-crm-prod>/sec_rules"),
        "{out}"
    );
    assert!(!out.contains("{step:"), "{out}");
    insta::assert_snapshot!("confirm_batch_120x48", out);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p illumio-tui --lib batch_tests`
Expected: FAIL to compile: "no variant named `BatchPlanned`", "no variant named `ConfirmBatch`", "cannot find function `confirm_batch_last_scroll`".

- [ ] **Step 3: Write the messages, the overlay and the runtime**

In `crates/illumio-tui/src/tui/msg.rs`:

```rust
    Service, Workload, WorkloadFilter,
};
use pce::traffic::{Focus, QueryProgress, ResolvedFlow};
use pce::write::{Outcome, Plan, WriteError};
use pce::{CacheMode, StatsSnapshot};
use std::sync::Arc;
use std::time::{Duration, Instant};
```

with:

```rust
    Service, Workload, WorkloadFilter,
};
use pce::traffic::{Focus, QueryProgress, ResolvedFlow};
use pce::write::{Batch, BatchInput, Outcome, Plan, StepResult, WriteError};
use pce::{CacheMode, StatsSnapshot};
use std::sync::Arc;
use std::time::{Duration, Instant};
```

In `crates/illumio-tui/src/tui/msg.rs`, replace:

```rust
        result: Result<Outcome, WriteError>,
    },
    DiffLoaded(Result<ObjectDiff, String>),
    /// The runtime applied `:writes on|off`: the guard was built (or refused,
    /// with the reason) or dropped.
    WritesSet(WritesState),
```

with:

```rust
        result: Result<Outcome, WriteError>,
    },
    DiffLoaded(Result<ObjectDiff, String>),
    /// A planned write batch (every request, in order) or why it could not
    /// be planned.
    BatchPlanned(Result<Batch, String>),
    /// What became of each step of a sent batch, and each step's kind.
    BatchDone {
        kinds: Vec<pce::write::ObjectKind>,
        results: Vec<StepResult>,
    },
    /// The runtime applied `:writes on|off`: the guard was built (or refused,
    /// with the reason) or dropped.
    WritesSet(WritesState),
```

In `crates/illumio-tui/src/tui/msg.rs`, replace:

```rust
        plan: Plan,
        kind: pce::write::ObjectKind,
    },
    /// The draft and active list of one kind (cached) for a pending IP list, service or ruleset.
    LoadDiff {
        change: PendingChange,
```

with:

```rust
        plan: Plan,
        kind: pce::write::ObjectKind,
    },
    /// Plan a write batch: one fresh draft ruleset read (dedupe again,
    /// create-if-missing), one IP list read when it creates IP lists, one
    /// GET per rule update. Never sends a write. `index` resolves names.
    PlanBatch {
        input: Box<BatchInput>,
        index: Arc<pce::NameIndex>,
    },
    /// `y` on the batch confirm: send every step once, in order, stopping
    /// at the first failure.
    ExecuteBatch(Batch),
    /// The draft and active list of one kind (cached) for a pending IP list, service or ruleset.
    LoadDiff {
        change: PendingChange,
```

In `crates/illumio-tui/src/tui/msg.rs`, replace:

```rust
                Cmd::ExecuteWrite { plan: pa, kind: ka },
                Cmd::ExecuteWrite { plan: pb, kind: kb },
            ) => pa.request() == pb.request() && ka == kb,
            (
                Cmd::LoadDiff {
                    change: ca,
```

with:

```rust
                Cmd::ExecuteWrite { plan: pa, kind: ka },
                Cmd::ExecuteWrite { plan: pb, kind: kb },
            ) => pa.request() == pb.request() && ka == kb,
            (Cmd::PlanBatch { input: a, .. }, Cmd::PlanBatch { input: b, .. }) => a == b,
            (Cmd::ExecuteBatch(a), Cmd::ExecuteBatch(b)) => a.lines() == b.lines(),
            (
                Cmd::LoadDiff {
                    change: ca,
```

In `crates/illumio-tui/src/tui/model.rs`, the overlay and its equality:

```rust
        kind: pce::write::ObjectKind,
        scroll: u16,
    },
    /// A pending IP list or service against its active version.
    Diff {
        diff: ObjectDiff,
```

with:

```rust
        kind: pce::write::ObjectKind,
        scroll: u16,
    },
    /// Every request of a write batch (the generator's); `y` sends them
    /// all, in order; `n`/`Esc` cancel. `scroll` is the first line shown.
    ConfirmBatch {
        batch: pce::write::Batch,
        scroll: u16,
    },
    /// A pending IP list or service against its active version.
    Diff {
        diff: ObjectDiff,
```

In `crates/illumio-tui/src/tui/model.rs`, replace:

```rust
                    scroll: sb,
                },
            ) => pa.request() == pb.request() && ka == kb && sa == sb,
            (
                Overlay::Confirm {
                    reason: ra,
```

with:

```rust
                    scroll: sb,
                },
            ) => pa.request() == pb.request() && ka == kb && sa == sb,
            (
                Overlay::ConfirmBatch {
                    batch: ba,
                    scroll: sa,
                },
                Overlay::ConfirmBatch {
                    batch: bb,
                    scroll: sb,
                },
            ) => ba.lines() == bb.lines() && sa == sb,
            (
                Overlay::Confirm {
                    reason: ra,
```

In `crates/illumio-tui/src/tui/app.rs`, the runtime plans and sends through the session's guard (none: refused, nothing sent):

```rust
                    let _ = tx.send(Msg::Stats(pce.stats()));
                });
            }
            Cmd::LoadDiff {
                change,
                kind,
```

with:

```rust
                    let _ = tx.send(Msg::Stats(pce.stats()));
                });
            }
            Cmd::PlanBatch { input, index } => {
                self.model.busy += 1;
                let (pce, tx) = self.task_ctx();
                let guard = self.writer.clone();
                tokio::spawn(async move {
                    // Reads only: the fresh drafts, the IP lists, a GET per update.
                    let result = match guard {
                        None => Err("writes are not enabled".to_string()),
                        Some(guard) => pce
                            .writer(&guard)
                            .plan_batch(*input, &index)
                            .await
                            .map_err(|e| e.to_string()),
                    };
                    let _ = tx.send(Msg::BatchPlanned(result));
                    let _ = tx.send(Msg::Stats(pce.stats()));
                });
            }
            Cmd::ExecuteBatch(batch) => {
                self.model.busy += 1;
                let (pce, tx) = self.task_ctx();
                let guard = self.writer.clone();
                let kinds: Vec<pce::write::ObjectKind> =
                    batch.steps.iter().map(|s| s.kind).collect();
                tokio::spawn(async move {
                    let results = match &guard {
                        Some(guard) => pce.writer(guard).execute_batch(batch).await,
                        None => kinds
                            .iter()
                            .enumerate()
                            .map(|(i, _)| {
                                if i == 0 {
                                    pce::write::StepResult::Failed(WriteError::Invalid(vec![
                                        WRITES_OFF.into(),
                                    ]))
                                } else {
                                    pce::write::StepResult::NotSent
                                }
                            })
                            .collect(),
                    };
                    // Pending changes are not under the kinds' prefixes.
                    pce.invalidate("policy:pending");
                    let _ = tx.send(Msg::BatchDone { kinds, results });
                    let _ = tx.send(Msg::Stats(pce.stats()));
                });
            }
            Cmd::LoadDiff {
                change,
                kind,
```

and in `Runtime::apply`, both answers are task results (they lower `busy`):

```rust
                | Msg::Planned(_)
                | Msg::WriteDone { .. }
                | Msg::DiffLoaded(_)
        );
        let cmds = update(&mut self.model, msg);
        if task_result {
```

with:

```rust
                | Msg::Planned(_)
                | Msg::WriteDone { .. }
                | Msg::DiffLoaded(_)
                | Msg::BatchPlanned(_)
                | Msg::BatchDone { .. }
        );
        let cmds = update(&mut self.model, msg);
        if task_result {
```

- [ ] **Step 4: Write the keys, the answers and the confirm**

In `crates/illumio-tui/src/tui/update.rs`:

```rust
    Cmd, DraftFailure, DraftOutcome, FocusedOutcome, Msg, ObjectsPayload, QueryOutcome,
};
use crate::tui::picker::{KeyPicker, PickerOutcome};
use crate::tui::ui::widgets::overlay::{confirm_write_last_scroll, help_last_scroll};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use pce::models::{EventFilter, PolicyDecision, PolicyScope, Workload};
use pce::traffic::treemap::Weight;
```

with:

```rust
    Cmd, DraftFailure, DraftOutcome, FocusedOutcome, Msg, ObjectsPayload, QueryOutcome,
};
use crate::tui::picker::{KeyPicker, PickerOutcome};
use crate::tui::ui::widgets::overlay::{
    confirm_batch_last_scroll, confirm_write_last_scroll, help_last_scroll,
};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use pce::models::{EventFilter, PolicyDecision, PolicyScope, Workload};
use pce::traffic::treemap::Weight;
```

the plan's answer, and the batch's, in `on_msg`:

```rust
                }
            }
        }
        Msg::Planned(result) => {
            model.writes.in_flight = false;
            let flow_form = model.traffic.rule_form.take();
```

with:

```rust
                }
            }
        }
        Msg::BatchPlanned(result) => {
            model.writes.in_flight = false;
            match result {
                // Keys typed while the batch was planned went elsewhere; a
                // `y` there must not confirm this.
                Ok(_) if !batch_lands(model) => {
                    model.status = StatusLine::warn("requests discarded, nothing sent");
                }
                Ok(batch) => {
                    model.status = StatusLine::info(format!(
                        "review the {}: y sends them all, n cancels",
                        if batch.sends() == 1 {
                            "request"
                        } else {
                            "requests"
                        }
                    ));
                    model.overlay = Overlay::ConfirmBatch { batch, scroll: 0 };
                }
                Err(e) => batch_plan_failed(model, &e),
            }
            Vec::new()
        }
        Msg::BatchDone { kinds, results } => batch_done(model, &kinds, &results),
        Msg::Planned(result) => {
            model.writes.in_flight = false;
            let flow_form = model.traffic.rule_form.take();
```

the confirm's keys, in `on_key`:

```rust
                _ => Vec::new(),
            }
        }
        Overlay::Pending => {
            let n = model.policy.pending.as_ref().map_or(0, Vec::len);
            match k.code {
```

with:

```rust
                _ => Vec::new(),
            }
        }
        Overlay::ConfirmBatch { batch, scroll } => {
            let area = ratatui::layout::Rect::new(0, 0, model.size.0, model.size.1);
            let last = u16::try_from(confirm_batch_last_scroll(&batch, area)).unwrap_or(u16::MAX);
            match k.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    model.overlay = Overlay::None;
                    model.status = StatusLine::info(format!(
                        "sending {} …",
                        if batch.sends() == 1 {
                            "1 request".to_string()
                        } else {
                            format!("{} requests", batch.sends())
                        }
                    ));
                    model.writes.in_flight = true;
                    vec![Cmd::ExecuteBatch(batch)]
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    let cmds = batch_closed(model);
                    model.status = StatusLine::info("cancelled, nothing sent");
                    cmds
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    model.overlay = Overlay::ConfirmBatch {
                        batch,
                        scroll: (scroll + 1).min(last),
                    };
                    Vec::new()
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    model.overlay = Overlay::ConfirmBatch {
                        batch,
                        scroll: scroll.saturating_sub(1),
                    };
                    Vec::new()
                }
                _ => Vec::new(),
            }
        }
        Overlay::Pending => {
            let n = model.policy.pending.as_ref().map_or(0, Vec::len);
            match k.code {
```

and the helpers, before `reload_after_write`:

```rust
    }
}

/// One reload of the written kind's table, past the cache (the writer
/// already invalidated it).
fn reload_after_write(model: &mut Model, kind: pce::write::ObjectKind) -> Vec<Cmd> {
```

with:

```rust
    }
}

/// Whether a planned batch may open its confirm: only over the screen it
/// was planned from (nothing else open).
fn batch_lands(model: &Model) -> bool {
    model.overlay == Overlay::None
}

/// A batch could not be planned: nothing was sent.
fn batch_plan_failed(model: &mut Model, e: &str) {
    fail(model, "plan", e, false);
}

/// The batch confirm or its answer is done: back to the screen. What it
/// reopens may need to load (none here).
fn batch_closed(model: &mut Model) -> Vec<Cmd> {
    model.overlay = Overlay::None;
    Vec::new()
}

/// `wrote 5 of 7 (pending); step 6 failed: …; 1 not sent`, and whether a
/// step failed.
fn batch_status(results: &[pce::write::StepResult]) -> (String, bool) {
    use pce::write::StepResult as R;
    let count = |f: fn(&R) -> bool| results.iter().filter(|r| f(r)).count();
    let written = count(|r| matches!(r, R::Written(_)));
    let reused = count(|r| matches!(r, R::Skipped { .. }));
    let not_sent = count(|r| matches!(r, R::NotSent));
    let mut s = format!("wrote {written} of {} (pending)", results.len());
    if reused > 0 {
        s.push_str(&format!("; reused {reused}"));
    }
    let failed = results.iter().enumerate().find_map(|(i, r)| match r {
        R::Failed(e) => Some((i, e)),
        _ => None,
    });
    if let Some((i, e)) = failed {
        if is_unanswered(e) {
            s.push_str(&format!(
                "; step {} not answered: it may have applied; reloading",
                i + 1
            ));
        } else {
            s.push_str(&format!("; step {} failed: {e}", i + 1));
        }
    }
    if not_sent > 0 {
        s.push_str(&format!("; {not_sent} not sent"));
    }
    (s, failed.is_some())
}

/// A sent batch's answer: the status line, then one reload of the
/// rulesets (and of the IP lists when one was written, so the index
/// follows) and one draft re-evaluation, whatever the step count.
fn batch_done(
    model: &mut Model,
    kinds: &[pce::write::ObjectKind],
    results: &[pce::write::StepResult],
) -> Vec<Cmd> {
    use pce::write::StepResult as R;
    model.writes.in_flight = false;
    // The runtime dropped `policy:pending`; the next `p` fetches again.
    model.policy.pending = None;
    model.policy.pending_marked.clear();
    let (text, failed) = batch_status(results);
    model.status = if failed {
        StatusLine::error(text)
    } else {
        StatusLine::info(text)
    };
    let touched = |r: &R| match r {
        R::Written(_) => true,
        R::Failed(e) => is_unanswered(e),
        R::Skipped { .. } | R::NotSent => false,
    };
    let mut cmds = Vec::new();
    if results.iter().any(touched) {
        let redraft = redraft_after_write(model, true, false);
        let lists = kinds
            .iter()
            .zip(results)
            .any(|(k, r)| *k == pce::write::ObjectKind::IpList && touched(r));
        if lists {
            cmds.extend(reload_kind_after_write(
                model,
                pce::write::ObjectKind::IpList,
            ));
        }
        cmds.extend(reload_after_write(model, pce::write::ObjectKind::Rule));
        cmds.extend(redraft);
    }
    cmds.extend(batch_closed(model));
    cmds
}

/// One reload of the written kind's table, past the cache (the writer
/// already invalidated it).
fn reload_after_write(model: &mut Model, kind: pce::write::ObjectKind) -> Vec<Cmd> {
```

In `crates/illumio-tui/src/tui/ui/widgets/overlay.rs`:

```rust
use crate::tui::ui::widgets::panel::panel;
use crate::tui::update::REVERT_WORD;
use pce::RuleView;
use pce::write::{ObjectKind, Plan, WriteRequest};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
```

with:

```rust
use crate::tui::ui::widgets::panel::panel;
use crate::tui::update::REVERT_WORD;
use pce::RuleView;
use pce::write::{Batch, ObjectKind, Plan, WriteRequest};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
```

In `crates/illumio-tui/src/tui/ui/widgets/overlay.rs`, replace:

```rust
    );
}

/// Longest message drawn; the panel holds five wrapped lines.
const ERROR_MAX_CHARS: usize = 300;

```

with:

```rust
    );
}

/// The batch confirm's rows around its scrolled lines: borders and the
/// summary line.
const BATCH_CHROME: u16 = 3;

/// The batch confirm as drawn: every line wrapped to the inner width (the
/// box is the dry run: nothing may be clipped), and the rows that fit.
fn confirm_batch_layout(batch: &Batch, area: Rect) -> (Rect, Vec<String>, usize) {
    let width = usize::from(CONFIRM_WIDTH.min(area.width).saturating_sub(2));
    let content: Vec<String> = batch
        .lines()
        .iter()
        .flat_map(|l| wrap_chars(&plain(l), width))
        .collect();
    let height = u16::try_from(content.len())
        .unwrap_or(u16::MAX)
        .saturating_add(BATCH_CHROME);
    let rect = centered(area, CONFIRM_WIDTH, height);
    let rows = usize::from(rect.height.saturating_sub(BATCH_CHROME));
    (rect, content, rows)
}

/// The first line shown when the batch confirm is scrolled to the end.
pub fn confirm_batch_last_scroll(batch: &Batch, area: Rect) -> usize {
    let (_, content, rows) = confirm_batch_layout(batch, area);
    content.len().saturating_sub(rows)
}

/// `Send these requests?`: the batch's summary pinned, then one block per
/// request (`N/M METHOD path`, its summary, its body), scrolled.
pub fn render_confirm_batch(model: &Model, batch: &Batch, scroll: u16, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let (r, content, rows) = confirm_batch_layout(batch, area);
    f.render_widget(Clear, r);
    let first = (scroll as usize).min(content.len().saturating_sub(rows));
    let meta = if content.len() > rows {
        "j/k scroll · y send all · n cancel"
    } else {
        "y send all · n cancel"
    };
    let mut lines = vec![Line::from(Span::styled(
        plain(&batch.summary()),
        Style::new().fg(t.warn),
    ))];
    lines.extend(content.into_iter().skip(first).take(rows).map(|s| {
        let head = s.split_once(' ').is_some_and(|(n, _)| {
            n.contains('/') && n.chars().next().is_some_and(|c| c.is_ascii_digit())
        });
        Line::from(Span::styled(
            s,
            if head { t.header_style() } else { t.base() },
        ))
    }));
    f.render_widget(
        Paragraph::new(lines)
            .block(panel("Send these requests?", meta, t, true))
            .style(t.base()),
        r,
    );
}

/// Longest message drawn; the panel holds five wrapped lines.
const ERROR_MAX_CHARS: usize = 300;

```

In `crates/illumio-tui/src/tui/ui/mod.rs`, draw it:

```rust
        Overlay::ConfirmWrite { plan, kind, scroll } => {
            widgets::overlay::render_confirm_write(model, plan, *kind, *scroll, f, area)
        }
        Overlay::Diff { diff, scroll } => {
            widgets::overlay::render_diff(model, diff, *scroll, f, area)
        }
```

with:

```rust
        Overlay::ConfirmWrite { plan, kind, scroll } => {
            widgets::overlay::render_confirm_write(model, plan, *kind, *scroll, f, area)
        }
        Overlay::ConfirmBatch { batch, scroll } => {
            widgets::overlay::render_confirm_batch(model, batch, *scroll, f, area)
        }
        Overlay::Diff { diff, scroll } => {
            widgets::overlay::render_diff(model, diff, *scroll, f, area)
        }
```

- [ ] **Step 5: Run the tests and write the snapshots**

Run: `cargo test -p illumio-tui --lib batch_tests && cargo test -p illumio-tui --test tui_runtime batch && INSTA_UPDATE=always cargo test -p illumio-tui --test tui_snapshots confirm_batch && find crates/illumio-tui/tests/snapshots -name '*.snap.new' -delete`
Expected: PASS (8 + 2 + 2 tests). Read the two new `.snap` files: every block numbered `N/3`, the summary pinned, `<href of step 2: create rule_set RF-crm-prod>/sec_rules` and no `{step:` anywhere.

- [ ] **Step 6: Commit**

```bash
git add crates/illumio-tui/src crates/illumio-tui/tests
git commit -m "tui: the batch confirm (PlanBatch, ConfirmBatch, ExecuteBatch, BatchDone)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

### Task 3: Tracking and the final gate

**Files:**
- Modify: `tasks/todo.md`

**Interfaces:**
- Consumes: everything above.
- Produces: the tracking section.

- [ ] **Step 1: Track it**

Append to `tasks/todo.md`:

```markdown

## Policy generator 2: the write batch (plan docs/superpowers/plans/2026-10-16-policy-generator-2-batch.md)
- [x] Task 1 pce::write::batch: plan_batch, execute_batch
- [x] Task 2 TUI: PlanBatch, ConfirmBatch, ExecuteBatch, BatchDone
- [x] Task 3 gate
```

- [ ] **Step 2: Run the final gate**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && git diff main -- '*Cargo.toml' Cargo.lock`
Expected: every test passes, no warning, the diff is empty.

- [ ] **Step 3: Write the review section**

Append `## Review (Policy generator 2)` to `tasks/todo.md`: the batch's order and reads, the guards, the test count, "no live smoke: this plan's code writes; nothing was sent to any PCE".

- [ ] **Step 4: Commit**

```bash
git add tasks/todo.md
git commit -m "docs: track policy generator 2

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

## Self-review notes

- Spec coverage: plan-time reads and refusals, step order, placeholders (`{step:N}` in a ruleset path and an IP list actor), created objects, the confirm (blocks, header, keys), the guards, `execute_batch` (sequential, stop, never retried, `BatchOutcome` as `Vec<StepResult>`), the status line, one reload and one `redraft_after_write` per batch: Tasks 1 and 2. The batch tests the spec lists are in `crates/pce/tests/batch.rs` (order, `Written` and `Noop` placeholders, stop, `Unanswered`, audit per step, cache prefixes, nothing before `execute_batch`, a leftover placeholder, one ruleset read for several creates).
- Left to Plan 3: `Enter` in the generator emits `Cmd::PlanBatch`; `batch_lands`, `batch_plan_failed` and `batch_closed` return to the generator, and the generator reopens on the reloaded drafts.
- Verification: every step of this plan was applied in order to a clean checkout of main (after the plans before it); each task built clean under `cargo clippy --workspace --all-targets -- -D warnings` with its own tests passing, and the gate passed at the end.
