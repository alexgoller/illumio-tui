# Policy Writes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add rulesets, allow rules, deny rules, provisioning and label changes on VEN-managed workloads to the `pce` write layer and the CLI, with the Plan 3 guard, dry run, single send and audit line unchanged.

**Architecture:** The policy models gain `extra` flatten maps so nothing the PCE returns is lost on a round trip. `pce::write` gains `actors.rs` (typed actors and services for drafts), `rule_sets.rs`, `rules.rs` (nested hrefs, list patches applied to the rule JSON as the PCE returned it) and `provision.rs` (one pending GET, an explicit `change_subset`); `ObjectKind` grows four kinds with a list of cache prefixes each. The CLI adds `policy rulesets|rules add|update|delete`, `policy deny-rules add|update|delete`, `policy provision` and `workloads update --managed-labels`; a pure token grammar (`ActorToken`, `ServiceToken`) is parsed before any request and resolved against object lists fetched at most once per invocation.

**Tech Stack:** Rust 2024 (rust-version 1.88), tokio, reqwest 0.13, serde/serde_json, thiserror 2, clap 4 (derive), wiremock 0.6, assert_cmd, tempfile, predicates. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-10-11-policy-writes-design.md` (Plan 3 spec `docs/superpowers/specs/2026-10-10-pce-writes-design.md` for the write layer's rules).

## Global Constraints

- edition 2024, rust-version 1.88; no new dependencies.
- Every PCE request is user-caused.
- POST/PUT/DELETE are sent exactly once and never retried (the library's `Writer::execute` guarantees it; planners never send).
- Draft is the only policy write target: `active` hrefs are rewritten to draft; provisioning is the only POST to `sec_policy`.
- hrefs are parsed strictly by component, never by substring.
- A round trip never drops fields the models do not know (`extra` flatten maps), and a changed list is sent whole with untouched entries carried as returned.
- Name resolution happens in the CLI with at most one list GET per kind per invocation, never inside library planners (except the single pending GET in `plan_provision` and the single object GET in update planners).
- The CLI envelope and exit codes of Plan 3 are unchanged (2 invalid, 4 unanswered, 6 refused).
- Existing tests keep passing.
- The TUI is untouched (`crates/illumio-tui/src/tui/**` does not change) except where a changed library signature forces a call-site update. The one such update: Task 1 adds the arm `RuleSet | Rule | DenyRule | Provision => Vec::new()` to the exhaustive `match kind` in `reload_after_write` (`crates/illumio-tui/src/tui/update.rs` ~line 1376), because `pce::write::ObjectKind` gains four variants. The TUI never plans these kinds in Plan 5.
- `PCE_LIVE_WRITES=1` is never set by this plan and never run against demo100. The opt-in live test is extended in Task 10 and is NOT run.
- Commit trailer on every commit, exactly (after a blank line): `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`.
- Gate before every commit: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p <crate>` (the task names the crate; Task 1 runs `-p pce -p illumio-tui` because it touches both; Task 10 runs `cargo test --workspace`).
- Run `cargo fmt --all` before the gate. Code in this plan is not always laid out the way rustfmt wants it; formatting changes are not deviations.
- Edition 2024 clippy may rewrite nested `if`/`if let` into let-chains: accept it and list it as a deviation in the task report.
- In zsh, backticks inside `echo "…"` run as commands: write ledger and todo lines with a quoted heredoc.
- Spec refinements this plan makes (reviewers: these are intended):
  - `checked_policy_href(href, segment, org_id)` (Task 1) is `checked_href` plus "must be the `/sec_policy/<scope>/` form". Policy planners use it where the spec says `checked_href(href, "rule_sets")`, because `checked_href` also accepts the flat `/orgs/N/rule_sets/ID`, which is not a ruleset.
  - `apply_rule_patch` and `apply_deny_rule_patch` take the fetched rule as JSON (`&serde_json::Value`), not `&Rule`. `IngressService` is unchanged (spec 3.1) and has no `extra`, so an untouched inline service with `icmp_type`/`icmp_code` would lose those keys through the typed model. Untouched providers, consumers and services are therefore copied verbatim from the PCE's JSON. A service entry with keys other than `href`/`port`/`to_port`/`proto` is carried untouched but never matched by a remove (fail closed: "is not among the services").
  - An update whose every field equals the current value becomes `Plan::Noop { reason: "<kind> already matches" }` (CLI `Skipped`, exit 0) instead of an empty PUT. This applies to rulesets, rules and deny rules. An empty patch is still `Invalid` before any request.
  - `RuleSetPatch.set_scopes` always sends `scopes` (an explicit replacement) and is not diffed.
  - A rule create body always carries `unscoped_consumers` (default `false`), and a deny rule create body always carries `override`.
  - Provisioning groups pending hrefs by their object type segment generically (`rule_sets`, `ip_lists`, `services`, `label_groups`, `virtual_services`, `firewall_settings`, … whatever `sec_policy/pending` lists), so `--all` always means all; the PCE decides which keys of `change_subset` it accepts and a rejection surfaces as `Rejected` with the body. A rule or deny-rule href, pending or requested, maps to its ruleset through `parent_href`. With explicit hrefs, unrelated pending entries are ignored.
  - An unscoped ruleset is sent as `scopes: [[]]` (one empty scope), which is how every unscoped ruleset on a real PCE is returned (`fixtures/recorded/rule_sets.json`); the spec's `[]` is superseded.
  - `Plan::Noop` for "nothing pending" carries `existing_href: /orgs/N/sec_policy/pending`.
  - `Writer::execute` reports no `dropped_fields` for `ObjectKind::Provision`: the PCE answers a provision with the new policy version, which echoes none of the request's keys.
  - CLI: `ActorToken::parse` and `ServiceToken::parse` are pure. `Lists<'a>` holds the `WriteGuard`, so `workload:HOST` resolves through `Writer::find_workload` (`write/workloads.rs` ~line 316, one or two reads of at most 500 rows per token). Labels, label groups, IP lists and services are each fetched at most once per invocation.
  - CLI: the end-to-end tests for token resolution live in Task 8, because no command consumes the resolver before then. Task 7 unit-tests the parsing.
  - CLI: `--resolve-providers-as` without `--resolve-consumers-as` (or the reverse) fills the other side with `workloads`, so the library's "both sides" rule holds.
  - CLI: the `policy rules` list positional becomes optional (`Option<String>`) so that `add|update|delete` can sit beside it. A missing ruleset is exit 2. A ruleset literally named `add`, `update` or `delete` must be given by href.
  - CLI: a ruleset given as an href (to `rules add`, `deny-rules add`, `rulesets update|delete`) costs no list GET; the planner checks it.
  - CLI: `rules add` and `deny-rules add` with no provider, consumer or service are refused with the library's `Invalid` messages (kind `invalid`, exit 2) before any request, even when the ruleset is given by name.
  - CLI: `policy provision` in table mode prints the rows `provisioned <version href>` and `counts ip_lists=1, rule_sets=3`, followed by the usual written rows.
  - CLI: `workloads update --managed-labels` refuses other changes before any request. A label change on a managed workload still reads the workload (one GET) before its PUT. Without the flag, a managed workload is refused after that GET, exactly as in Plan 3.

## Review Focus

1. Rule update that adds one provider while the existing provider carries an unknown key (`future_flag`) and an inline service carries `icmp_type`. Expected: one GET, a PUT body that is exactly `{"providers": [<existing entry byte-for-byte>, <new entry>]}`, and no `ingress_services` key. Test: Task 4 `update_add_provider_keeps_extras_and_sends_providers_whole`.
2. Provision `All` when the pending list holds a rule href (`.../rule_sets/100/sec_rules/1`), a deny-rule href of the same ruleset and the ruleset itself. Expected: `change_subset.rule_sets` is `[{"href": ".../rule_sets/100"}]` exactly once, and no `sec_rules`/`deny_rules` text anywhere in the body. Test: Task 5 `rule_hrefs_map_to_their_ruleset_once`.
3. A flat ruleset href, a deny-rule href or a dot segment where a rule href is expected (`policy rules update /orgs/5/sec_policy/draft/rule_sets/100 --description x`). Expected: `Invalid` exit 2 naming the expected shape `/orgs/<org>/sec_policy/draft/rule_sets/<id>/sec_rules/<id>`, zero requests. Tests: Task 1 `nested_dot_and_empty_segments_are_refused`, Task 8 `rules_update_refuses_the_wrong_href_shape_before_any_request`.
4. `workloads update <managed> --add-label loc=eu --description x --managed-labels`. Expected: exit 2 `--managed-labels allows label changes only`, zero requests. With labels only: a PUT whose body is exactly `{"labels": [...]}`. Tests: Task 9 `managed_labels_refuses_other_changes_before_any_request`, Task 6 `relabel_of_a_managed_workload_sends_only_labels`.
5. An executed provision. Expected: one POST to `/orgs/5/sec_policy`, `Written.href` is the version href, `pending: false`, `dropped_fields` empty, one audit entry carrying the body, and both `policy:` and `objects:` caches invalidated (a cached ruleset list and a cached IP list refetch). Test: Task 5 `execute_provision_posts_once_and_invalidates_policy_and_objects`.

---

## File structure

| File | Responsibility |
| --- | --- |
| `crates/pce/src/models/policy.rs` | `extra` on `Actor`, `Rule`, `DenyRule`, `RuleSet` |
| `crates/pce/src/write/mod.rs` | `ObjectKind::{RuleSet, Rule, DenyRule, Provision}`, `cache_prefixes`, `checked_nested_href`, `checked_policy_href`, `parent_href`, `decode`, execute invalidates every prefix, no `dropped_fields` for a provision, module wiring |
| `crates/pce/src/write/actors.rs` (new) | `ActorKind`, `ActorRef`, `ServiceRef`, conversions, validation, href kinds |
| `crates/pce/src/write/rule_sets.rs` (new) | `RuleSetDraft`, `RuleSetPatch`, ruleset planners |
| `crates/pce/src/write/rules.rs` (new) | `RuleDraft`, `RulePatch`, `DenyRuleDraft`, `DenyRulePatch`, `apply_rule_patch`, `apply_deny_rule_patch`, rule planners |
| `crates/pce/src/write/provision.rs` (new) | `ProvisionSubset`, `ProvisionRequest`, `provision_counts`, `plan_provision` |
| `crates/pce/src/write/workloads.rs` | `LabelsPatch`, `plan_relabel_workload` |
| `crates/pce/src/resolve.rs` | `NameIndex::label_group_href` |
| `crates/pce/tests/policy_writes.rs` (new) | wiremock contract tests for every policy planner and provisioning |
| `crates/pce/tests/facade.rs` | `cache_prefix` call site moves to `cache_prefixes` |
| `crates/pce/tests/live_writes.rs` | opt-in ruleset/rule lifecycle (not run) |
| `crates/illumio-tui/src/tui/update.rs` | one match arm in `reload_after_write` (forced by the new kinds) |
| `crates/illumio-tui/src/cli/write.rs` | `ActorToken`, `ServiceToken`, `Lists`, resolvers, `resolve_workload`, `run_provision_plan`, provision output variants |
| `crates/illumio-tui/src/cli/policy.rs` | `RulesetsCmd`, `RulesCmd`, `DenyRulesAction`, `ProvisionArgs` and their handlers |
| `crates/illumio-tui/src/cli/workloads.rs` | `--managed-labels`; `workload_href` replaced by `write::resolve_workload` |
| `crates/illumio-tui/src/cli/mod.rs` | `Policy` doc line |
| `crates/illumio-tui/tests/cli_writes.rs` | CLI tests for every new command; `ALL_WRITES` grows to 23 |
| `crates/illumio-tui/tests/cli.rs` | help-text checker covers the new subcommands |
| `README.md`, `docs/architecture.md`, `docs/index.html`, `tasks/todo.md` | docs and tracking |

---

### Task 1: Models and kinds

**Files:**
- Modify: `crates/pce/src/models/policy.rs` (`Actor` ~line 30-50, `Rule` ~74-101, `DenyRule` ~103-120, `RuleSet` ~122-144; `mod tests` ~188)
- Modify: `crates/pce/src/write/mod.rs` (`ObjectKind` ~line 28-60; after `checked_href` ~line 211; `Writer::execute` invalidation ~line 352; `mod tests` ~405-430)
- Modify: `crates/pce/tests/facade.rs` (~line 532, the only `cache_prefix` caller outside `write/mod.rs`; `grep -rn cache_prefix crates docs` finds no other)
- Modify: `crates/illumio-tui/src/tui/update.rs` (`reload_after_write` ~line 1376-1387: one arm)
- Create: `crates/pce/tests/policy_writes.rs`

**Interfaces:**
- Consumes: `checked_href` (`write/mod.rs` ~177), `WriteError::invalid` (~152), `Pce::invalidate` (`lib.rs` ~117), `Pce::rule_sets` (`lib.rs` ~239, cache key `policy:draft:rule_sets`).
- Produces:

```rust
// models/policy.rs: on Actor, Rule, DenyRule and RuleSet
#[serde(flatten)]
pub extra: serde_json::Map<String, serde_json::Value>,

// write/mod.rs
pub enum ObjectKind { Label, IpList, Service, Workload, RuleSet, Rule, DenyRule, Provision }
impl ObjectKind {
    pub fn cache_prefixes(self) -> &'static [&'static str];
    pub fn pending(self) -> bool;
    pub fn as_str(self) -> &'static str; // "rule_set", "rule", "deny_rule", "provision" for the new kinds
}
pub fn checked_nested_href(href: &str, parent: &str, child: &str, org_id: u64) -> Result<String, WriteError>;
pub fn checked_policy_href(href: &str, segment: &str, org_id: u64) -> Result<String, WriteError>;
pub fn parent_href(nested: &str) -> String;
```

- [ ] **Step 1: Write the failing tests**

Append to `mod tests` in `crates/pce/src/models/policy.rs`:

```rust
    #[test]
    fn unknown_fields_survive_a_round_trip() {
        let v = serde_json::json!({
            "href": "/orgs/5/sec_policy/draft/rule_sets/100", "name": "Shop-Prod", "enabled": true,
            "caps": ["write"], "ip_tables_rules": [],
            "scopes": [[{"label": {"href": "/orgs/5/labels/3"}, "exclusion": false, "future": 1}]],
            "rules": [{
                "href": "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1", "enabled": true,
                "providers": [{"label": {"href": "/orgs/5/labels/2"}, "future": "x"}],
                "consumers": [{"actors": "ams"}],
                "ingress_services": [{"port": 443, "proto": 6}],
                "sec_connect": false, "egress_services": [], "use_workload_subnets": ["providers"],
                "consuming_security_principals": []
            }],
            "deny_rules": [{
                "href": "/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8", "enabled": true,
                "providers": [{"actors": "ams"}], "consumers": [{"actors": "ams"}],
                "ingress_services": [{"port": 22, "proto": 6}], "override": true, "network_type": "brn"
            }]
        });
        let rs: RuleSet = serde_json::from_value(v.clone()).unwrap();
        assert_eq!(rs.extra["caps"], serde_json::json!(["write"]));
        assert_eq!(rs.scopes[0][0].extra["future"], 1);
        assert_eq!(
            rs.rules[0].extra["use_workload_subnets"],
            serde_json::json!(["providers"])
        );
        assert_eq!(rs.rules[0].providers[0].extra["future"], "x");
        assert_eq!(rs.deny_rules[0].extra["network_type"], "brn");
        assert_eq!(rs.rules[0].sec_connect, Some(false), "named fields stay named");
        assert!(!rs.rules[0].extra.contains_key("sec_connect"));
        let back = serde_json::to_value(&rs).unwrap();
        assert_eq!(back, v, "nothing lost, nothing added");
    }
```

Replace the test `object_kind_prefixes_and_pending` in `mod tests` of `crates/pce/src/write/mod.rs` and append the href tests:

```rust
    #[test]
    fn object_kind_prefixes_and_pending() {
        assert_eq!(ObjectKind::Label.cache_prefixes(), &["objects:"]);
        assert_eq!(ObjectKind::IpList.cache_prefixes(), &["objects:"]);
        assert_eq!(ObjectKind::Workload.cache_prefixes(), &["workloads"]);
        assert_eq!(ObjectKind::RuleSet.cache_prefixes(), &["policy:"]);
        assert_eq!(ObjectKind::Rule.cache_prefixes(), &["policy:"]);
        assert_eq!(ObjectKind::DenyRule.cache_prefixes(), &["policy:"]);
        assert_eq!(
            ObjectKind::Provision.cache_prefixes(),
            &["policy:", "objects:"]
        );
        for k in [
            ObjectKind::IpList,
            ObjectKind::Service,
            ObjectKind::RuleSet,
            ObjectKind::Rule,
            ObjectKind::DenyRule,
        ] {
            assert!(k.pending(), "{k:?}");
        }
        for k in [ObjectKind::Label, ObjectKind::Workload, ObjectKind::Provision] {
            assert!(!k.pending(), "{k:?}");
        }
        assert_eq!(ObjectKind::DenyRule.as_str(), "deny_rule");
        assert_eq!(ObjectKind::Provision.as_str(), "provision");
        assert_eq!(
            serde_json::to_value(ObjectKind::RuleSet).unwrap(),
            json!("rule_set")
        );
    }

    fn nested_err(href: &str, child: &str) -> String {
        match checked_nested_href(href, "rule_sets", child, 5).unwrap_err() {
            WriteError::Invalid(m) => m.join("; "),
            other => panic!("{href}: expected Invalid, got {other:?}"),
        }
    }

    #[test]
    fn nested_hrefs_pass_and_active_is_rewritten() {
        assert_eq!(
            checked_nested_href(
                " /orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1/ ",
                "rule_sets",
                "sec_rules",
                5
            )
            .unwrap(),
            "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1"
        );
        assert_eq!(
            checked_nested_href(
                "/orgs/5/sec_policy/active/rule_sets/24206847997116627/deny_rules/0a-B",
                "rule_sets",
                "deny_rules",
                5
            )
            .unwrap(),
            "/orgs/5/sec_policy/draft/rule_sets/24206847997116627/deny_rules/0a-B"
        );
    }

    #[test]
    fn nested_other_scopes_are_refused() {
        for scope in ["42", "Draft", "", "pending"] {
            let m = nested_err(
                &format!("/orgs/5/sec_policy/{scope}/rule_sets/100/sec_rules/1"),
                "sec_rules",
            );
            assert!(!m.is_empty(), "{scope}");
        }
    }

    #[test]
    fn nested_dot_and_empty_segments_are_refused() {
        for href in [
            "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/..",
            "/orgs/5/sec_policy/draft/rule_sets/../sec_rules/1",
            "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1/../2",
            "/orgs/5/sec_policy/draft/rule_sets/./100/sec_rules/1",
            "/orgs/5/sec_policy/draft/rule_sets//sec_rules/1",
            "/orgs/5/sec_policy/draft/rule_sets/100//1",
            "//orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1",
            "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/%2e%2e",
            "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1_2",
            "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1.2",
            "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/",
        ] {
            let m = nested_err(href, "sec_rules");
            assert!(
                m.contains("expected /orgs/<org>/sec_policy/draft/rule_sets/<id>/sec_rules/<id>"),
                "{href}: {m}"
            );
        }
    }

    #[test]
    fn nested_wrong_parent_child_or_flat_is_refused() {
        nested_err(
            "/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/7",
            "sec_rules",
        );
        nested_err("/orgs/5/sec_policy/draft/ip_lists/100/sec_rules/1", "sec_rules");
        let m = nested_err("/orgs/5/sec_policy/draft/rule_sets/100", "sec_rules");
        assert!(m.contains("is not a sec_rules href"), "{m}");
        // checked_href keeps refusing nested paths.
        assert!(
            checked_href(
                "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1",
                "rule_sets",
                5
            )
            .is_err()
        );
        assert!(
            checked_href(
                "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1",
                "sec_rules",
                5
            )
            .is_err()
        );
    }

    #[test]
    fn nested_foreign_org_is_refused_with_both_ids() {
        let m = nested_err(
            "/orgs/7/sec_policy/draft/rule_sets/100/sec_rules/1",
            "sec_rules",
        );
        assert!(m.contains("org 7") && m.contains("org 5"), "{m}");
    }

    #[test]
    fn parent_href_strips_the_rule_part() {
        assert_eq!(
            parent_href("/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1"),
            "/orgs/5/sec_policy/draft/rule_sets/100"
        );
        assert_eq!(
            parent_href("/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8/"),
            "/orgs/5/sec_policy/draft/rule_sets/100"
        );
    }

    #[test]
    fn policy_hrefs_refuse_the_flat_form() {
        assert_eq!(
            checked_policy_href("/orgs/5/sec_policy/active/rule_sets/100", "rule_sets", 5)
                .unwrap(),
            "/orgs/5/sec_policy/draft/rule_sets/100"
        );
        let err = checked_policy_href("/orgs/5/rule_sets/100", "rule_sets", 5).unwrap_err();
        assert!(
            err.to_string()
                .contains("expected /orgs/<org>/sec_policy/draft/rule_sets/<id>"),
            "{err}"
        );
    }
```

Create `crates/pce/tests/policy_writes.rs`:

```rust
//! Policy writes: rulesets, rules, deny rules, provisioning, managed
//! labels. The helpers repeat those of write_contract.rs (test files share
//! no code without a module).
#![allow(dead_code)]
use pce::client::{ClientConfig, PceClient};
use pce::models::PolicyScope;
use pce::write::{
    MemoryAudit, ObjectKind, Outcome, Plan, WriteContext, WriteError, WriteGuard, WriteRequest,
};
use pce::{CacheMode, Credentials, Pce, WriteMethod};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use wiremock::matchers::{method, path};
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

fn ctx() -> WriteContext {
    WriteContext {
        profile: "lab".into(),
        host: "pce.example".into(),
        org_id: 5,
        source: "test".into(),
    }
}

fn guard_with(audit: Arc<MemoryAudit>) -> WriteGuard {
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

/// The messages of an `Invalid`, joined; panics on any other error.
fn invalid(err: WriteError) -> String {
    match err {
        WriteError::Invalid(m) => m.join("; "),
        other => panic!("expected Invalid, got {other:?}"),
    }
}

fn rule_sets() -> Value {
    serde_json::from_str(include_str!("../../../fixtures/rule_sets.json")).unwrap()
}

async fn request_count(server: &MockServer) -> usize {
    server.received_requests().await.unwrap().len()
}

#[tokio::test]
async fn execute_of_a_rule_set_plan_invalidates_policy_and_marks_pending() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(rule_sets()))
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets"))
        .respond_with(ResponseTemplate::new(201).set_body_json(
            json!({"href": "/orgs/5/sec_policy/draft/rule_sets/102", "name": "New"}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    pce.rule_sets(PolicyScope::Draft, CacheMode::Use)
        .await
        .unwrap(); // warm the cache
    let audit = MemoryAudit::new();
    let guard = guard_with(audit.clone());
    let out = pce
        .writer(&guard)
        .execute(send_plan(
            WriteMethod::Post,
            "/orgs/5/sec_policy/draft/rule_sets",
            Some(json!({"name": "New"})),
            ObjectKind::RuleSet,
        ))
        .await
        .unwrap();
    let Outcome::Written(w) = out else {
        panic!("expected Written")
    };
    assert!(w.pending, "rulesets wait for provisioning");
    assert_eq!(w.href, "/orgs/5/sec_policy/draft/rule_sets/102");
    let again = pce
        .rule_sets(PolicyScope::Draft, CacheMode::Use)
        .await
        .unwrap();
    assert!(!again.hit, "policy: was invalidated");
    assert_eq!(audit.entries().len(), 1);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p pce`
Expected: compile errors: `no field 'extra' on type 'RuleSet'`, `no function or associated item named 'cache_prefixes'`, `cannot find function 'checked_nested_href'`, `no variant named 'RuleSet' found for enum 'ObjectKind'`.

- [ ] **Step 3: Implement the models**

In `crates/pce/src/models/policy.rs` change the import line to `use serde_json::{Map, Value};` and add this field as the last field of `Actor`, `Rule`, `DenyRule` and `RuleSet`:

```rust
    /// Fields this model does not name (`sec_connect` is named; `caps`,
    /// `egress_services`, `use_workload_subnets` and future ones are not),
    /// carried through a round trip.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
```

`Actor` keeps `#[derive(Default)]` (`Map` is `Default`); the only struct literal of a policy model in the workspace, `Actor { .., ..Default::default() }` in `resolve.rs` tests (~line 622), keeps compiling.

- [ ] **Step 4: Implement the kinds and hrefs**

In `crates/pce/src/write/mod.rs` replace the `ObjectKind` enum and impl (~lines 28-60) with:

```rust
/// What kind of object a plan touches; decides the cache prefixes and
/// whether the result waits for provisioning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectKind {
    Label,
    IpList,
    Service,
    Workload,
    RuleSet,
    Rule,
    DenyRule,
    /// `POST sec_policy`: changes active policy and therefore every
    /// policy and object list the cache holds.
    Provision,
}

impl ObjectKind {
    pub fn cache_prefixes(self) -> &'static [&'static str] {
        match self {
            ObjectKind::Label | ObjectKind::IpList | ObjectKind::Service => &["objects:"],
            ObjectKind::Workload => &["workloads"],
            ObjectKind::RuleSet | ObjectKind::Rule | ObjectKind::DenyRule => &["policy:"],
            ObjectKind::Provision => &["policy:", "objects:"],
        }
    }

    /// Policy objects live in draft until provisioned.
    pub fn pending(self) -> bool {
        matches!(
            self,
            ObjectKind::IpList
                | ObjectKind::Service
                | ObjectKind::RuleSet
                | ObjectKind::Rule
                | ObjectKind::DenyRule
        )
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ObjectKind::Label => "label",
            ObjectKind::IpList => "ip_list",
            ObjectKind::Service => "service",
            ObjectKind::Workload => "workload",
            ObjectKind::RuleSet => "rule_set",
            ObjectKind::Rule => "rule",
            ObjectKind::DenyRule => "deny_rule",
            ObjectKind::Provision => "provision",
        }
    }
}
```

Insert after `checked_href` (~line 211):

```rust
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// A rule or deny rule href inside a ruleset, parsed strictly by component:
/// `/orgs/<org>/sec_policy/<draft|active>/<parent>/<id>/<child>/<id>`, each
/// id `[A-Za-z0-9-]+`, `active` rewritten to `draft`, a foreign org refused.
pub fn checked_nested_href(
    href: &str,
    parent: &str,
    child: &str,
    org_id: u64,
) -> Result<String, WriteError> {
    let href = href.trim().trim_end_matches('/');
    let bad = || {
        WriteError::invalid(format!(
            "{href:?} is not a {child} href (expected /orgs/<org>/sec_policy/draft/{parent}/<id>/{child}/<id>)"
        ))
    };
    let parts: Vec<&str> = href.split('/').collect();
    let ["", "orgs", org, "sec_policy", scope, p, pid, c, cid] = parts.as_slice() else {
        return Err(bad());
    };
    let org_ok = !org.is_empty() && org.bytes().all(|b| b.is_ascii_digit());
    if *p != parent || *c != child || !valid_id(pid) || !valid_id(cid) || !org_ok {
        return Err(bad());
    }
    let org: u64 = org.parse().map_err(|_| bad())?;
    if org != org_id {
        return Err(WriteError::invalid(format!(
            "{href} belongs to org {org}, profile is org {org_id}"
        )));
    }
    match *scope {
        "draft" | "active" => Ok(format!(
            "/orgs/{org}/sec_policy/draft/{parent}/{pid}/{child}/{cid}"
        )),
        _ => Err(WriteError::invalid(format!(
            "{href}: only draft policy objects can be written"
        ))),
    }
}

/// [`checked_href`] for policy objects: the `/sec_policy/<scope>/` form
/// only. `checked_href` alone also accepts `/orgs/N/rule_sets/ID`, which
/// names no ruleset.
pub fn checked_policy_href(href: &str, segment: &str, org_id: u64) -> Result<String, WriteError> {
    let out = checked_href(href, segment, org_id)?;
    if out.split('/').nth(3) == Some("sec_policy") {
        Ok(out)
    } else {
        Err(WriteError::invalid(format!(
            "{:?} is not a {segment} href (expected /orgs/<org>/sec_policy/draft/{segment}/<id>)",
            href.trim()
        )))
    }
}

/// The ruleset href of a rule or deny rule href (the first seven
/// components). Call it on an href that passed [`checked_nested_href`].
pub fn parent_href(nested: &str) -> String {
    let parts: Vec<&str> = nested.trim().trim_end_matches('/').split('/').collect();
    parts[..parts.len().min(7)].join("/")
}
```

In `Writer::execute` replace `self.pce.invalidate(kind.cache_prefix());` (~line 352) with:

```rust
        for prefix in kind.cache_prefixes() {
            self.pce.invalidate(prefix);
        }
```

- [ ] **Step 5: Move the remaining caller and the TUI match**

In `crates/pce/tests/facade.rs` (~line 532) replace `p.invalidate(pce::write::ObjectKind::IpList.cache_prefix());` with:

```rust
    for prefix in pce::write::ObjectKind::IpList.cache_prefixes() {
        p.invalidate(prefix);
    }
```

In `crates/illumio-tui/src/tui/update.rs`, `reload_after_write` (~line 1385), add after the `pce::write::ObjectKind::Workload => load_workloads(model, CacheMode::Bypass),` arm:

```rust
        // Policy kinds are written by the CLI only (Plan 6 brings them here).
        pce::write::ObjectKind::RuleSet
        | pce::write::ObjectKind::Rule
        | pce::write::ObjectKind::DenyRule
        | pce::write::ObjectKind::Provision => Vec::new(),
```

This arm is the only change under `crates/illumio-tui/src/tui/` in this plan.

- [ ] **Step 6: Run the tests**

Run: `cargo test -p pce && cargo test -p illumio-tui`
Expected: PASS, including `unknown_fields_survive_a_round_trip`, the seven href tests, `object_kind_prefixes_and_pending`, `execute_of_a_rule_set_plan_invalidates_policy_and_marks_pending`, and every existing test (the facade active-lists test still sees two refetches).

- [ ] **Step 7: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p pce -p illumio-tui
git add crates/pce/src/models/policy.rs crates/pce/src/write/mod.rs crates/pce/tests/facade.rs crates/pce/tests/policy_writes.rs crates/illumio-tui/src/tui/update.rs
git commit -m "feat(pce): policy object kinds, nested rule hrefs, extra fields on policy models

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 2: Actors and services

**Files:**
- Create: `crates/pce/src/write/actors.rs`
- Modify: `crates/pce/src/write/mod.rs` (module list ~lines 6-12: `pub mod actors;`; re-exports ~lines 20-26: `pub use actors::{ActorKind, ActorRef, ServiceRef};`)

**Interfaces:**
- Consumes: `models::{Actor, IngressService, Ref}` (`models/policy.rs` ~30-64, `models/common.rs` ~5), `PortSpec` (`write/specs.rs` ~130: `parse`, `to_json`, `display`, derives `PartialEq, Eq`), `checked_href`, `checked_policy_href` (Task 1), `WriteError`.
- Produces:

```rust
pub enum ActorKind { Label, LabelGroup, IpList, Workload, All }
impl ActorKind { pub fn as_str(self) -> &'static str; pub fn excludable(self) -> bool; }
pub struct ActorRef { pub kind: ActorKind, pub href: Option<String>, pub exclusion: bool }
impl ActorRef {
    pub fn new(kind: ActorKind, href: impl Into<String>) -> Self;
    pub fn all() -> Self;
    pub fn excluded(self) -> Self;
    pub fn validate(&self) -> Result<(), String>;
    pub fn to_actor(&self) -> Actor;
    pub fn to_json(&self) -> serde_json::Value;
    pub fn from_actor(a: &Actor) -> Result<Self, String>;
    pub fn from_href(href: &str, org_id: u64) -> Result<Self, WriteError>;
    pub fn same_actor(&self, other: &ActorRef) -> bool;
    pub fn display(&self) -> String;
}
pub enum ServiceRef { Href(String), Port(PortSpec) }
impl ServiceRef {
    pub fn to_ingress(&self) -> IngressService;
    pub fn from_ingress(s: &IngressService) -> Self;
    pub fn from_href(href: &str, org_id: u64) -> Result<Self, WriteError>;
    pub fn same_service(&self, other: &Self) -> bool;
    pub fn to_json(&self) -> serde_json::Value;
    pub fn display(&self) -> String;
}
```

- [ ] **Step 1: Write the failing tests**

Create `crates/pce/src/write/actors.rs` holding only its doc comment and test module (Step 3 inserts the code above the module):

```rust
//! Actors (rule providers and consumers, scope members) and rule services
//! as drafts carry them, with the conversions to and from the PCE models.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::RuleSet;
    use serde_json::json;

    fn fixture() -> Vec<RuleSet> {
        serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap()
    }

    fn port(s: &str) -> ServiceRef {
        ServiceRef::Port(PortSpec::parse(s).unwrap())
    }

    #[test]
    fn actors_serialise_in_pce_form() {
        assert_eq!(ActorRef::all().to_json(), json!({"actors": "ams"}));
        assert_eq!(
            ActorRef::new(ActorKind::Label, "/orgs/5/labels/1").to_json(),
            json!({"label": {"href": "/orgs/5/labels/1"}})
        );
        assert_eq!(
            ActorRef::new(
                ActorKind::LabelGroup,
                "/orgs/5/sec_policy/draft/label_groups/10"
            )
            .to_json(),
            json!({"label_group": {"href": "/orgs/5/sec_policy/draft/label_groups/10"}})
        );
        assert_eq!(
            ActorRef::new(ActorKind::IpList, "/orgs/5/sec_policy/draft/ip_lists/20")
                .excluded()
                .to_json(),
            json!({"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/20"}, "exclusion": true})
        );
        assert_eq!(
            ActorRef::new(ActorKind::Workload, "/orgs/5/workloads/aaaa-1").to_json(),
            json!({"workload": {"href": "/orgs/5/workloads/aaaa-1"}})
        );
    }

    #[test]
    fn fixture_actors_convert_both_ways() {
        let rs = fixture();
        let ams = ActorRef::from_actor(&rs[1].rules[0].providers[0]).unwrap();
        assert_eq!(ams, ActorRef::all());
        let group = ActorRef::from_actor(&rs[1].rules[0].consumers[0]).unwrap();
        assert_eq!(
            group,
            ActorRef::new(
                ActorKind::LabelGroup,
                "/orgs/5/sec_policy/draft/label_groups/10"
            )
        );
        let ipl = ActorRef::from_actor(&rs[0].rules[1].consumers[0]).unwrap();
        assert_eq!(
            ipl,
            ActorRef::new(ActorKind::IpList, "/orgs/5/sec_policy/draft/ip_lists/21")
        );
        for a in [&ams, &group, &ipl] {
            assert_eq!(&ActorRef::from_actor(&a.to_actor()).unwrap(), a);
        }
        let scope = ActorRef::from_actor(&rs[0].scopes[0][0]).unwrap();
        assert_eq!(scope, ActorRef::new(ActorKind::Label, "/orgs/5/labels/3"));
    }

    #[test]
    fn unsupported_actors_are_refused_with_the_spec_message() {
        let vs: Actor = serde_json::from_value(
            json!({"virtual_service": {"href": "/orgs/5/sec_policy/draft/virtual_services/1"}}),
        )
        .unwrap();
        assert_eq!(
            ActorRef::from_actor(&vs).unwrap_err(),
            "virtual_service actors are not supported in this version"
        );
        let vsrv: Actor = serde_json::from_value(
            json!({"virtual_server": {"href": "/orgs/5/sec_policy/draft/virtual_servers/1"}}),
        )
        .unwrap();
        assert_eq!(
            ActorRef::from_actor(&vsrv).unwrap_err(),
            "virtual_server actors are not supported in this version"
        );
        let odd: Actor = serde_json::from_value(json!({"actors": "container_hosts"})).unwrap();
        assert!(
            ActorRef::from_actor(&odd)
                .unwrap_err()
                .contains("container_hosts")
        );
        assert!(ActorRef::from_actor(&Actor::default()).is_err());
    }

    #[test]
    fn same_actor_compares_kind_href_and_exclusion() {
        let a = ActorRef::new(ActorKind::Label, "/orgs/5/labels/1");
        assert!(a.same_actor(&a.clone()));
        assert!(!a.same_actor(&a.clone().excluded()));
        assert!(!a.same_actor(&ActorRef::new(ActorKind::Label, "/orgs/5/labels/2")));
        assert!(
            !ActorRef::new(ActorKind::LabelGroup, "/x/1")
                .same_actor(&ActorRef::new(ActorKind::IpList, "/x/1"))
        );
        // An explicit "exclusion": false from the PCE equals no flag.
        let explicit: Actor = serde_json::from_value(
            json!({"label": {"href": "/orgs/5/labels/1"}, "exclusion": false}),
        )
        .unwrap();
        assert!(ActorRef::from_actor(&explicit).unwrap().same_actor(&a));
    }

    #[test]
    fn validation_refuses_excluded_all_and_excluded_workloads() {
        assert_eq!(
            ActorRef::all().excluded().validate().unwrap_err(),
            "all cannot be excluded"
        );
        assert_eq!(
            ActorRef::new(ActorKind::Workload, "/orgs/5/workloads/aaaa-1")
                .excluded()
                .validate()
                .unwrap_err(),
            "only labels, label groups and IP lists can be excluded, not a workload"
        );
        assert!(
            ActorRef::new(ActorKind::IpList, "/orgs/5/sec_policy/draft/ip_lists/20")
                .excluded()
                .validate()
                .is_ok()
        );
        let no_href = ActorRef {
            kind: ActorKind::Label,
            href: None,
            exclusion: false,
        };
        assert_eq!(no_href.validate().unwrap_err(), "a label actor needs an href");
        let all_href = ActorRef {
            kind: ActorKind::All,
            href: Some("/x".into()),
            exclusion: false,
        };
        assert_eq!(
            all_href.validate().unwrap_err(),
            "the all actor takes no href (got /x)"
        );
    }

    #[test]
    fn hrefs_pick_the_actor_kind_by_component() {
        assert_eq!(
            ActorRef::from_href("/orgs/5/labels/1", 5).unwrap(),
            ActorRef::new(ActorKind::Label, "/orgs/5/labels/1")
        );
        assert_eq!(
            ActorRef::from_href("/orgs/5/workloads/aaaa-1", 5)
                .unwrap()
                .kind,
            ActorKind::Workload
        );
        assert_eq!(
            ActorRef::from_href("/orgs/5/sec_policy/active/ip_lists/21", 5).unwrap(),
            ActorRef::new(ActorKind::IpList, "/orgs/5/sec_policy/draft/ip_lists/21")
        );
        assert_eq!(
            ActorRef::from_href("/orgs/5/sec_policy/draft/label_groups/10", 5)
                .unwrap()
                .kind,
            ActorKind::LabelGroup
        );
        for bad in [
            "/orgs/5/sec_policy/draft/labels/1",
            "/orgs/5/ip_lists/21",
            "/orgs/5/sec_policy/draft/services/31",
            "/orgs/5/labels/../workloads/1",
            "/orgs/5/labels/1/2",
            "/orgs/5/labels/%2e%2e",
        ] {
            assert!(ActorRef::from_href(bad, 5).is_err(), "{bad}");
        }
        let err = ActorRef::from_href("/orgs/7/labels/1", 5).unwrap_err();
        assert!(err.to_string().contains("org 7"), "{err}");
    }

    #[test]
    fn services_convert_and_compare_by_port_spec() {
        let rs = fixture();
        let inline = &rs[0].rules[1].ingress_services;
        assert_eq!(ServiceRef::from_ingress(&inline[0]), port("443/tcp"));
        assert!(ServiceRef::from_ingress(&inline[1]).same_service(&port("8000-8099/tcp")));
        assert!(!ServiceRef::from_ingress(&inline[1]).same_service(&port("8000-8099/udp")));
        let href = ServiceRef::from_ingress(&rs[0].rules[0].ingress_services[0]);
        assert_eq!(
            href,
            ServiceRef::Href("/orgs/5/sec_policy/draft/services/31".into())
        );
        for s in [&href, &port("icmp"), &port("8000-8099/tcp")] {
            assert_eq!(&ServiceRef::from_ingress(&s.to_ingress()), s);
        }
        // A to_port equal to port is the single port.
        let same = IngressService::Port {
            port: Some(53),
            to_port: Some(53),
            proto: 17,
        };
        assert_eq!(ServiceRef::from_ingress(&same), port("53/udp"));
        assert_eq!(
            port("8000-8099/tcp").to_json(),
            json!({"port": 8000, "to_port": 8099, "proto": 6})
        );
        assert_eq!(
            href.to_json(),
            json!({"href": "/orgs/5/sec_policy/draft/services/31"})
        );
        assert_eq!(port("443/tcp").display(), "443/tcp");
        assert_eq!(
            href.display(),
            "service /orgs/5/sec_policy/draft/services/31"
        );
        assert_eq!(
            ServiceRef::from_href("/orgs/5/sec_policy/active/services/32", 5).unwrap(),
            ServiceRef::Href("/orgs/5/sec_policy/draft/services/32".into())
        );
        assert!(ServiceRef::from_href("/orgs/5/services/32", 5).is_err());
    }
}
```

Add `pub mod actors;` to `crates/pce/src/write/mod.rs`.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p pce --lib write::actors`
Expected: compile errors: `cannot find type 'ActorRef' in this scope`, `cannot find type 'ServiceRef' in this scope`, `cannot find type 'Actor' in this scope`.

- [ ] **Step 3: Implement** (insert above `#[cfg(test)]` in `crates/pce/src/write/actors.rs`)

```rust
use super::specs::PortSpec;
use super::{WriteError, checked_href, checked_policy_href};
use crate::models::{Actor, IngressService, Ref};
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorKind {
    Label,
    LabelGroup,
    IpList,
    Workload,
    /// Every workload: `{"actors": "ams"}`.
    All,
}

impl ActorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ActorKind::Label => "label",
            ActorKind::LabelGroup => "label_group",
            ActorKind::IpList => "ip_list",
            ActorKind::Workload => "workload",
            ActorKind::All => "all",
        }
    }

    /// The PCE excludes labels, label groups and IP lists only.
    pub fn excludable(self) -> bool {
        matches!(
            self,
            ActorKind::Label | ActorKind::LabelGroup | ActorKind::IpList
        )
    }
}

/// One actor of a draft: a provider, a consumer or a scope member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorRef {
    pub kind: ActorKind,
    /// `None` only for [`ActorKind::All`].
    pub href: Option<String>,
    pub exclusion: bool,
}

impl ActorRef {
    pub fn new(kind: ActorKind, href: impl Into<String>) -> Self {
        Self {
            kind,
            href: Some(href.into()),
            exclusion: false,
        }
    }

    pub fn all() -> Self {
        Self {
            kind: ActorKind::All,
            href: None,
            exclusion: false,
        }
    }

    pub fn excluded(mut self) -> Self {
        self.exclusion = true;
        self
    }

    pub fn validate(&self) -> Result<(), String> {
        match (self.kind, &self.href) {
            (ActorKind::All, Some(h)) => {
                return Err(format!("the all actor takes no href (got {h})"));
            }
            (ActorKind::All, None) => {}
            (k, None) => return Err(format!("a {} actor needs an href", k.as_str())),
            (_, Some(_)) => {}
        }
        if self.exclusion && self.kind == ActorKind::All {
            return Err("all cannot be excluded".into());
        }
        if self.exclusion && !self.kind.excludable() {
            return Err(format!(
                "only labels, label groups and IP lists can be excluded, not a {}",
                self.kind.as_str()
            ));
        }
        Ok(())
    }

    pub fn to_actor(&self) -> Actor {
        let r = || self.href.clone().map(Ref::new);
        let mut a = Actor::default();
        match self.kind {
            ActorKind::All => a.actors = Some("ams".into()),
            ActorKind::Label => a.label = r(),
            ActorKind::LabelGroup => a.label_group = r(),
            ActorKind::IpList => a.ip_list = r(),
            ActorKind::Workload => a.workload = r(),
        }
        if self.exclusion {
            a.exclusion = Some(true);
        }
        a
    }

    /// The PCE JSON for a request body.
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self.to_actor()).unwrap_or(Value::Null)
    }

    /// An actor as the PCE returned it. Virtual services, virtual servers
    /// and `actors` values other than `ams` are refused.
    pub fn from_actor(a: &Actor) -> Result<Self, String> {
        let exclusion = a.exclusion.unwrap_or(false);
        let with = |kind: ActorKind, r: &Ref| -> Result<Self, String> {
            Ok(Self {
                kind,
                href: Some(r.href.clone()),
                exclusion,
            })
        };
        if let Some(s) = &a.actors {
            return if s == "ams" {
                Ok(Self {
                    kind: ActorKind::All,
                    href: None,
                    exclusion,
                })
            } else {
                Err(format!("actors {s:?} is not supported in this version"))
            };
        }
        if a.virtual_service.is_some() {
            return Err("virtual_service actors are not supported in this version".into());
        }
        if a.virtual_server.is_some() {
            return Err("virtual_server actors are not supported in this version".into());
        }
        if let Some(r) = &a.label {
            return with(ActorKind::Label, r);
        }
        if let Some(r) = &a.label_group {
            return with(ActorKind::LabelGroup, r);
        }
        if let Some(r) = &a.ip_list {
            return with(ActorKind::IpList, r);
        }
        if let Some(r) = &a.workload {
            return with(ActorKind::Workload, r);
        }
        Err("actor names no label, label group, IP list, workload or all".into())
    }

    /// An href typed by its components: `/orgs/N/labels/ID`,
    /// `/orgs/N/workloads/ID`, `/orgs/N/sec_policy/<scope>/label_groups/ID`
    /// or `/orgs/N/sec_policy/<scope>/ip_lists/ID`, then checked strictly
    /// (`active` becomes `draft`).
    pub fn from_href(href: &str, org_id: u64) -> Result<Self, WriteError> {
        let parts: Vec<&str> = href.trim().trim_end_matches('/').split('/').collect();
        let (kind, segment) = match parts.as_slice() {
            ["", "orgs", _, "labels", _] => (ActorKind::Label, "labels"),
            ["", "orgs", _, "workloads", _] => (ActorKind::Workload, "workloads"),
            ["", "orgs", _, "sec_policy", _, "label_groups", _] => {
                (ActorKind::LabelGroup, "label_groups")
            }
            ["", "orgs", _, "sec_policy", _, "ip_lists", _] => (ActorKind::IpList, "ip_lists"),
            _ => {
                return Err(WriteError::invalid(format!(
                    "{:?} is not a label, label group, IP list or workload href",
                    href.trim()
                )));
            }
        };
        Ok(Self::new(kind, checked_href(href, segment, org_id)?))
    }

    pub fn same_actor(&self, other: &ActorRef) -> bool {
        self.kind == other.kind && self.href == other.href && self.exclusion == other.exclusion
    }

    /// `label /orgs/5/labels/1`, `!ip_list /orgs/..`, `all`: for messages.
    pub fn display(&self) -> String {
        let base = match &self.href {
            Some(h) => format!("{} {h}", self.kind.as_str()),
            None => self.kind.as_str().to_string(),
        };
        if self.exclusion {
            format!("!{base}")
        } else {
            base
        }
    }
}

/// One service of a rule: a service object or an inline port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceRef {
    Href(String),
    Port(PortSpec),
}

impl ServiceRef {
    pub fn to_ingress(&self) -> IngressService {
        match self {
            ServiceRef::Href(h) => IngressService::Ref(Ref::new(h.clone())),
            ServiceRef::Port(p) => IngressService::Port {
                port: p.port,
                to_port: p.to_port,
                proto: p.proto,
            },
        }
    }

    pub fn from_ingress(s: &IngressService) -> Self {
        match s {
            IngressService::Ref(r) => ServiceRef::Href(r.href.clone()),
            IngressService::Port {
                port,
                to_port,
                proto,
            } => ServiceRef::Port(PortSpec {
                port: *port,
                to_port: to_port.filter(|t| Some(*t) != *port),
                proto: *proto,
            }),
        }
    }

    pub fn from_href(href: &str, org_id: u64) -> Result<Self, WriteError> {
        checked_policy_href(href, "services", org_id).map(ServiceRef::Href)
    }

    /// Hrefs compare exactly; inline ports through [`PortSpec`].
    pub fn same_service(&self, other: &Self) -> bool {
        self == other
    }

    pub fn to_json(&self) -> Value {
        match self {
            ServiceRef::Href(h) => serde_json::json!({ "href": h }),
            ServiceRef::Port(p) => p.to_json(),
        }
    }

    pub fn display(&self) -> String {
        match self {
            ServiceRef::Href(h) => format!("service {h}"),
            ServiceRef::Port(p) => p.display(),
        }
    }
}
```

Add `pub use actors::{ActorKind, ActorRef, ServiceRef};` to the re-exports in `crates/pce/src/write/mod.rs`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p pce --lib write::actors`
Expected: PASS, 7 tests.

- [ ] **Step 5: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p pce
git add crates/pce/src/write/actors.rs crates/pce/src/write/mod.rs
git commit -m "feat(pce): typed actors and rule services for policy drafts

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 3: Rulesets

**Files:**
- Create: `crates/pce/src/write/rule_sets.rs`
- Modify: `crates/pce/src/write/mod.rs` (`pub mod rule_sets;`, `pub use rule_sets::{RuleSetDraft, RuleSetPatch};`, and the `decode` helper after `parent_href`)
- Test: `crates/pce/tests/policy_writes.rs` (append `mod rule_sets`)

**Interfaces:**
- Consumes: `ActorKind`, `ActorRef` (Task 2), `checked_policy_href` (Task 1), `Writer::fetch` (`write/mod.rs` ~261), `Writer::bypass` (~266), `Pce::rule_sets(PolicyScope::Draft, CacheMode::Bypass)` (`lib.rs` ~239: one `GET sec_policy/draft/rule_sets?max_results=10000` through `cached_list`, which refreshes the `policy:draft:rule_sets` entry and reports `truncated`), `models::RuleSet` (with `update_type`), `IfMissing`, `Plan`, `WriteRequest`, `WriteMethod`.
- Decision on the `SkipExisting` lookup: `Pce::rule_sets(PolicyScope::Draft, self.bypass())`. It is exactly one GET, it bypasses the cache like every other `--if-missing` lookup, and it leaves a fresh list in the cache for the CLI's own name resolution. A hand-built fetch of `sec_policy/draft/rule_sets` would duplicate `cached_list`.
- Produces:

```rust
// write/mod.rs
pub(crate) fn decode<T: serde::de::DeserializeOwned>(href: &str, v: serde_json::Value) -> Result<T, WriteError>;

// write/rule_sets.rs
pub struct RuleSetDraft { pub name: String, pub description: Option<String>, pub enabled: bool, pub scopes: Vec<Vec<ActorRef>> }
impl RuleSetDraft { pub fn validate(&self) -> Result<(), Vec<String>>; }
pub struct RuleSetPatch { pub name: Option<String>, pub description: Option<String>, pub enabled: Option<bool>, pub set_scopes: Option<Vec<Vec<ActorRef>>> }
impl RuleSetPatch {
    pub fn is_empty(&self) -> bool;
    pub fn validate(&self) -> Result<(), Vec<String>>;
    pub fn body(&self, current: &RuleSet) -> serde_json::Value;
}
impl Writer<'_> {
    pub async fn plan_create_rule_set(&self, draft: &RuleSetDraft, if_missing: IfMissing) -> Result<Plan, WriteError>;
    pub async fn plan_update_rule_set(&self, href: &str, patch: &RuleSetPatch) -> Result<Plan, WriteError>;
    pub fn plan_delete_rule_set(&self, href: &str) -> Result<Plan, WriteError>;
}
```

- [ ] **Step 1: Write the failing tests** (append to `crates/pce/tests/policy_writes.rs`)

```rust
mod rule_sets {
    use super::*;
    use pce::write::{ActorKind, ActorRef, IfMissing, RuleSetDraft, RuleSetPatch};

    fn label(id: u32) -> ActorRef {
        ActorRef::new(ActorKind::Label, format!("/orgs/5/labels/{id}"))
    }

    fn group() -> ActorRef {
        ActorRef::new(
            ActorKind::LabelGroup,
            "/orgs/5/sec_policy/draft/label_groups/10",
        )
    }

    fn named(name: &str) -> RuleSetDraft {
        RuleSetDraft {
            name: name.into(),
            description: None,
            enabled: true,
            scopes: vec![],
        }
    }

    async fn mount_list(server: &MockServer, body: Value) {
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(server)
            .await;
    }

    async fn mount_shop_prod(server: &MockServer) {
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets/100"))
            .respond_with(ResponseTemplate::new(200).set_body_json(rule_sets()[0].clone()))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn create_body_has_two_scopes_and_unscoped_sends_one_empty_scope() {
        let server = MockServer::start().await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let d = RuleSetDraft {
            name: " Web-Tier ".into(),
            description: Some("web".into()),
            enabled: true,
            scopes: vec![vec![label(3), label(4)], vec![group()]],
        };
        let plan = pce
            .writer(&guard)
            .plan_create_rule_set(&d, IfMissing::Create)
            .await
            .unwrap();
        let req = plan.request().unwrap();
        assert_eq!(req.method, WriteMethod::Post);
        assert_eq!(req.path, "/orgs/5/sec_policy/draft/rule_sets");
        assert_eq!(
            req.body,
            Some(json!({
                "name": "Web-Tier", "description": "web", "enabled": true,
                "scopes": [
                    [{"label": {"href": "/orgs/5/labels/3"}}, {"label": {"href": "/orgs/5/labels/4"}}],
                    [{"label_group": {"href": "/orgs/5/sec_policy/draft/label_groups/10"}}]
                ]
            }))
        );
        assert_eq!(req.summary, "create rule_set Web-Tier");
        let unscoped = RuleSetDraft {
            enabled: false,
            ..named("Global")
        };
        let plan = pce
            .writer(&guard)
            .plan_create_rule_set(&unscoped, IfMissing::Create)
            .await
            .unwrap();
        assert_eq!(
            plan.request().unwrap().body,
            Some(json!({"name": "Global", "enabled": false, "scopes": [[]]}))
        );
        assert_eq!(request_count(&server).await, 0);
    }

    #[tokio::test]
    async fn a_scope_with_an_ip_list_or_an_empty_name_is_refused_locally() {
        let server = MockServer::start().await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let d = RuleSetDraft {
            scopes: vec![vec![
                label(3),
                ActorRef::new(ActorKind::IpList, "/orgs/5/sec_policy/draft/ip_lists/21"),
            ]],
            ..named("X")
        };
        let m = invalid(
            pce.writer(&guard)
                .plan_create_rule_set(&d, IfMissing::SkipExisting)
                .await
                .unwrap_err(),
        );
        assert_eq!(m, "scopes take labels and label groups only");
        let m = invalid(
            pce.writer(&guard)
                .plan_create_rule_set(&named("  "), IfMissing::SkipExisting)
                .await
                .unwrap_err(),
        );
        assert_eq!(m, "ruleset name must not be empty");
        let patch = RuleSetPatch {
            set_scopes: Some(vec![vec![ActorRef::all()]]),
            ..RuleSetPatch::default()
        };
        let m = invalid(
            pce.writer(&guard)
                .plan_update_rule_set("/orgs/5/sec_policy/draft/rule_sets/100", &patch)
                .await
                .unwrap_err(),
        );
        assert_eq!(m, "scopes take labels and label groups only");
        assert_eq!(request_count(&server).await, 0, "refused before any request");
    }

    #[tokio::test]
    async fn if_missing_matches_names_case_insensitively_and_skips_pending_deletes() {
        let server = MockServer::start().await;
        mount_list(&server, rule_sets()).await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let plan = pce
            .writer(&guard)
            .plan_create_rule_set(&named("shop-prod"), IfMissing::SkipExisting)
            .await
            .unwrap();
        let Plan::Noop {
            existing_href,
            reason,
        } = plan
        else {
            panic!("expected Noop")
        };
        assert_eq!(existing_href, "/orgs/5/sec_policy/draft/rule_sets/100");
        assert_eq!(reason, "rule_set shop-prod exists");
        let plan = pce
            .writer(&guard)
            .plan_create_rule_set(&named("Web-Tier"), IfMissing::SkipExisting)
            .await
            .unwrap();
        assert!(plan.request().is_some());
        assert_eq!(request_count(&server).await, 2, "one GET per lookup, never cached");
    }

    #[tokio::test]
    async fn if_missing_refuses_ambiguity() {
        let server = MockServer::start().await;
        mount_list(
            &server,
            json!([
                {"href": "/orgs/5/sec_policy/draft/rule_sets/200", "name": "Dup"},
                {"href": "/orgs/5/sec_policy/draft/rule_sets/201", "name": "dup"},
                {"href": "/orgs/5/sec_policy/draft/rule_sets/202", "name": "Gone", "update_type": "delete"}
            ]),
        )
        .await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let m = invalid(
            pce.writer(&guard)
                .plan_create_rule_set(&named("DUP"), IfMissing::SkipExisting)
                .await
                .unwrap_err(),
        );
        assert!(
            m.contains("/orgs/5/sec_policy/draft/rule_sets/200")
                && m.contains("/orgs/5/sec_policy/draft/rule_sets/201"),
            "{m}"
        );
        let plan = pce
            .writer(&guard)
            .plan_create_rule_set(&named("gone"), IfMissing::SkipExisting)
            .await
            .unwrap();
        assert!(plan.request().is_some(), "a pending delete does not count");
    }

    #[tokio::test]
    async fn update_sends_only_changed_keys_and_never_rules() {
        let server = MockServer::start().await;
        mount_shop_prod(&server).await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let w = pce.writer(&guard);
        let patch = RuleSetPatch {
            name: Some("Shop-Prod".into()),
            description: Some("Ringfence v2".into()),
            enabled: Some(true),
            set_scopes: None,
        };
        let plan = w
            .plan_update_rule_set("/orgs/5/sec_policy/active/rule_sets/100", &patch)
            .await
            .unwrap();
        let req = plan.request().unwrap();
        assert_eq!(req.method, WriteMethod::Put);
        assert_eq!(req.path, "/orgs/5/sec_policy/draft/rule_sets/100");
        assert_eq!(req.body, Some(json!({"description": "Ringfence v2"})));
        assert_eq!(req.summary, "update rule_set Shop-Prod");
        let patch = RuleSetPatch {
            set_scopes: Some(vec![vec![label(5)]]),
            ..RuleSetPatch::default()
        };
        let plan = w
            .plan_update_rule_set("/orgs/5/sec_policy/draft/rule_sets/100", &patch)
            .await
            .unwrap();
        assert_eq!(
            plan.request().unwrap().body,
            Some(json!({"scopes": [[{"label": {"href": "/orgs/5/labels/5"}}]]}))
        );
        let patch = RuleSetPatch {
            enabled: Some(false),
            set_scopes: Some(vec![]),
            ..RuleSetPatch::default()
        };
        let plan = w
            .plan_update_rule_set("/orgs/5/sec_policy/draft/rule_sets/100", &patch)
            .await
            .unwrap();
        assert_eq!(
            plan.request().unwrap().body,
            Some(json!({"enabled": false, "scopes": [[]]}))
        );
        assert_eq!(request_count(&server).await, 3, "one GET per plan");
    }

    #[tokio::test]
    async fn an_update_that_changes_nothing_is_a_noop_and_an_empty_patch_is_refused() {
        let server = MockServer::start().await;
        mount_shop_prod(&server).await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let patch = RuleSetPatch {
            name: Some("Shop-Prod".into()),
            enabled: Some(true),
            ..RuleSetPatch::default()
        };
        let plan = pce
            .writer(&guard)
            .plan_update_rule_set("/orgs/5/sec_policy/draft/rule_sets/100", &patch)
            .await
            .unwrap();
        let Plan::Noop {
            existing_href,
            reason,
        } = plan
        else {
            panic!("expected Noop")
        };
        assert_eq!(existing_href, "/orgs/5/sec_policy/draft/rule_sets/100");
        assert_eq!(reason, "rule_set Shop-Prod already matches");
        let m = invalid(
            pce.writer(&guard)
                .plan_update_rule_set(
                    "/orgs/5/sec_policy/draft/rule_sets/100",
                    &RuleSetPatch::default(),
                )
                .await
                .unwrap_err(),
        );
        assert_eq!(m, "nothing to change");
        assert_eq!(request_count(&server).await, 1);
    }

    #[tokio::test]
    async fn delete_and_href_shapes() {
        let server = MockServer::start().await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let w = pce.writer(&guard);
        let plan = w
            .plan_delete_rule_set("/orgs/5/sec_policy/active/rule_sets/100")
            .unwrap();
        let req = plan.request().unwrap();
        assert_eq!(req.method, WriteMethod::Delete);
        assert_eq!(req.path, "/orgs/5/sec_policy/draft/rule_sets/100");
        assert!(req.body.is_none());
        assert_eq!(
            req.summary,
            "delete rule_set /orgs/5/sec_policy/draft/rule_sets/100"
        );
        let patch = RuleSetPatch {
            description: Some("x".into()),
            ..RuleSetPatch::default()
        };
        for bad in [
            "/orgs/5/rule_sets/100",
            "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1",
            "/orgs/5/sec_policy/draft/ip_lists/21",
            "/orgs/5/sec_policy/draft/rule_sets/../labels/1",
        ] {
            invalid(w.plan_delete_rule_set(bad).unwrap_err());
            invalid(w.plan_update_rule_set(bad, &patch).await.unwrap_err());
        }
        assert_eq!(request_count(&server).await, 0);
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p pce --test policy_writes rule_sets`
Expected: compile error: `unresolved imports pce::write::RuleSetDraft, pce::write::RuleSetPatch`.

- [ ] **Step 3: Add `decode` to `crates/pce/src/write/mod.rs`** (after `parent_href`)

```rust
/// A fetched object decoded into its model; the error names the href.
pub(crate) fn decode<T: serde::de::DeserializeOwned>(href: &str, v: Value) -> Result<T, WriteError> {
    serde_json::from_value(v).map_err(|e| WriteError::Pce(PceError::Decode(format!("{href}: {e}"))))
}
```

Add `pub mod rule_sets;` and `pub use rule_sets::{RuleSetDraft, RuleSetPatch};`.

- [ ] **Step 4: Implement `crates/pce/src/write/rule_sets.rs`**

```rust
//! Rulesets under `/sec_policy/draft/rule_sets`. A ruleset `PUT` carries
//! only the keys that change and never `rules` or `deny_rules`: rules have
//! their own nested hrefs (`write/rules.rs`).
use super::actors::{ActorKind, ActorRef};
use super::{
    IfMissing, ObjectKind, Plan, WriteError, WriteRequest, Writer, checked_policy_href, decode,
};
use crate::client::WriteMethod;
use crate::models::{PolicyScope, RuleSet};
use serde_json::{Map, Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleSetDraft {
    pub name: String,
    pub description: Option<String>,
    pub enabled: bool,
    /// Alternatives; each inner list is one scope of labels or label
    /// groups. Empty: the ruleset applies everywhere.
    pub scopes: Vec<Vec<ActorRef>>,
}

const SCOPE_KINDS: &str = "scopes take labels and label groups only";

/// At most one label per dimension in a scope is not checked here (the
/// library has no label index); the PCE refuses it.
fn check_scopes(scopes: &[Vec<ActorRef>], errs: &mut Vec<String>) {
    for a in scopes.iter().flatten() {
        if !matches!(a.kind, ActorKind::Label | ActorKind::LabelGroup) {
            if !errs.iter().any(|e| e == SCOPE_KINDS) {
                errs.push(SCOPE_KINDS.into());
            }
        } else if let Err(e) = a.validate() {
            errs.push(e);
        }
    }
}

fn check_name(name: &str, errs: &mut Vec<String>) {
    if name.trim().is_empty() {
        errs.push("ruleset name must not be empty".into());
    }
}

fn done(errs: Vec<String>) -> Result<(), Vec<String>> {
    if errs.is_empty() { Ok(()) } else { Err(errs) }
}

/// The PCE represents "applies everywhere" as one empty scope (`[[]]`),
/// which is what every unscoped ruleset on a real PCE returns.
fn scopes_json(scopes: &[Vec<ActorRef>]) -> Value {
    if scopes.is_empty() {
        return json!([[]]);
    }
    Value::Array(
        scopes
            .iter()
            .map(|s| Value::Array(s.iter().map(ActorRef::to_json).collect()))
            .collect(),
    )
}

impl RuleSetDraft {
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errs = Vec::new();
        check_name(&self.name, &mut errs);
        check_scopes(&self.scopes, &mut errs);
        done(errs)
    }

    pub(crate) fn to_body(&self) -> Value {
        let mut v = json!({
            "name": self.name.trim(),
            "enabled": self.enabled,
            "scopes": scopes_json(&self.scopes),
        });
        if let Some(d) = &self.description {
            v["description"] = json!(d);
        }
        v
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuleSetPatch {
    pub name: Option<String>,
    pub description: Option<String>,
    pub enabled: Option<bool>,
    /// Replaces every scope; `Some(vec![])` makes the ruleset apply everywhere.
    pub set_scopes: Option<Vec<Vec<ActorRef>>>,
}

impl RuleSetPatch {
    pub fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.description.is_none()
            && self.enabled.is_none()
            && self.set_scopes.is_none()
    }

    /// The errors that need no current ruleset.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errs = Vec::new();
        if let Some(n) = &self.name {
            check_name(n, &mut errs);
        }
        if let Some(s) = &self.set_scopes {
            check_scopes(s, &mut errs);
        }
        done(errs)
    }

    /// The `PUT` body against `current`: a name, description or state equal
    /// to the current one contributes nothing; `set_scopes` is always sent.
    pub fn body(&self, current: &RuleSet) -> Value {
        let mut v = Map::new();
        if let Some(n) = &self.name
            && n.trim() != current.name
        {
            v.insert("name".into(), json!(n.trim()));
        }
        if let Some(d) = &self.description
            && current.description.as_deref().unwrap_or("") != d.as_str()
        {
            v.insert("description".into(), json!(d));
        }
        if let Some(e) = self.enabled
            && e != current.enabled
        {
            v.insert("enabled".into(), json!(e));
        }
        if let Some(s) = &self.set_scopes {
            v.insert("scopes".into(), scopes_json(s));
        }
        Value::Object(v)
    }
}

impl Writer<'_> {
    pub async fn plan_create_rule_set(
        &self,
        draft: &RuleSetDraft,
        if_missing: IfMissing,
    ) -> Result<Plan, WriteError> {
        draft.validate().map_err(WriteError::Invalid)?;
        let name = draft.name.trim();
        if if_missing == IfMissing::SkipExisting {
            let sets = self.pce().rule_sets(PolicyScope::Draft, self.bypass()).await?;
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
                        && rs.name.eq_ignore_ascii_case(name)
                })
                .collect();
            match hits.as_slice() {
                [] => {}
                [one] => {
                    return Ok(Plan::Noop {
                        existing_href: one.href.clone(),
                        reason: format!("rule_set {name} exists"),
                    });
                }
                many => {
                    return Err(WriteError::invalid(format!(
                        "{name:?} matches several rulesets: {}",
                        many.iter()
                            .map(|rs| rs.href.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )));
                }
            }
        }
        Ok(Plan::Send {
            request: WriteRequest {
                method: WriteMethod::Post,
                path: self.pce().client().org_path("sec_policy/draft/rule_sets"),
                body: Some(draft.to_body()),
                summary: format!("create rule_set {name}"),
            },
            kind: ObjectKind::RuleSet,
        })
    }

    pub async fn plan_update_rule_set(
        &self,
        href: &str,
        patch: &RuleSetPatch,
    ) -> Result<Plan, WriteError> {
        let href = checked_policy_href(href, "rule_sets", self.pce().client().org_id())?;
        if patch.is_empty() {
            return Err(WriteError::invalid("nothing to change"));
        }
        patch.validate().map_err(WriteError::Invalid)?;
        let current: RuleSet = decode(&href, self.fetch(&href).await?)?;
        let body = patch.body(&current);
        if body.as_object().is_some_and(Map::is_empty) {
            return Ok(Plan::Noop {
                existing_href: href,
                reason: format!("rule_set {} already matches", current.name),
            });
        }
        Ok(Plan::Send {
            request: WriteRequest {
                method: WriteMethod::Put,
                path: href,
                body: Some(body),
                summary: format!("update rule_set {}", current.name),
            },
            kind: ObjectKind::RuleSet,
        })
    }

    pub fn plan_delete_rule_set(&self, href: &str) -> Result<Plan, WriteError> {
        let href = checked_policy_href(href, "rule_sets", self.pce().client().org_id())?;
        Ok(Plan::Send {
            request: WriteRequest {
                method: WriteMethod::Delete,
                path: href.clone(),
                body: None,
                summary: format!("delete rule_set {href}"),
            },
            kind: ObjectKind::RuleSet,
        })
    }
}
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p pce --test policy_writes`
Expected: PASS (7 new tests in `rule_sets`, 1 from Task 1).

- [ ] **Step 6: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p pce
git add crates/pce/src/write/rule_sets.rs crates/pce/src/write/mod.rs crates/pce/tests/policy_writes.rs
git commit -m "feat(pce): ruleset drafts, changed-keys patches and create/update/delete plans

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 4: Rules and deny rules

**Files:**
- Create: `crates/pce/src/write/rules.rs`
- Modify: `crates/pce/src/write/mod.rs` (`pub mod rules;`, `pub use rules::{DenyRuleDraft, DenyRulePatch, RuleDraft, RulePatch, apply_deny_rule_patch, apply_rule_patch};`)
- Test: `crates/pce/tests/policy_writes.rs` (append `mod rules`)

**Interfaces:**
- Consumes: `ActorRef`, `ServiceRef`, `ActorKind` (Task 2), `checked_nested_href`, `checked_policy_href` (Task 1), `Writer::fetch`, `models::{Actor, IngressService, ResolveLabelsAs}` (`ResolveLabelsAs` derives `Default, PartialEq`, not `Eq`), `PortSpec` (tests).
- Produces:

```rust
pub struct RuleDraft {
    pub description: Option<String>, pub enabled: bool,
    pub providers: Vec<ActorRef>, pub consumers: Vec<ActorRef>, pub services: Vec<ServiceRef>,
    pub unscoped_consumers: bool, pub resolve_labels_as: Option<ResolveLabelsAs>,
}
impl RuleDraft { pub fn validate(&self) -> Result<(), Vec<String>>; }
#[derive(Default)]
pub struct RulePatch {
    pub description: Option<String>, pub enabled: Option<bool>,
    pub add_providers: Vec<ActorRef>, pub remove_providers: Vec<ActorRef>,
    pub add_consumers: Vec<ActorRef>, pub remove_consumers: Vec<ActorRef>,
    pub add_services: Vec<ServiceRef>, pub remove_services: Vec<ServiceRef>,
    pub unscoped_consumers: Option<bool>, pub resolve_labels_as: Option<ResolveLabelsAs>,
}
impl RulePatch { pub fn is_empty(&self) -> bool; pub fn check(&self) -> Result<(), Vec<String>>; }
pub struct DenyRuleDraft { pub description: Option<String>, pub enabled: bool, pub providers: Vec<ActorRef>, pub consumers: Vec<ActorRef>, pub services: Vec<ServiceRef>, pub override_: bool }
impl DenyRuleDraft { pub fn validate(&self) -> Result<(), Vec<String>>; }
#[derive(Default)]
pub struct DenyRulePatch {
    pub description: Option<String>, pub enabled: Option<bool>,
    pub add_providers: Vec<ActorRef>, pub remove_providers: Vec<ActorRef>,
    pub add_consumers: Vec<ActorRef>, pub remove_consumers: Vec<ActorRef>,
    pub add_services: Vec<ServiceRef>, pub remove_services: Vec<ServiceRef>,
    pub override_: Option<bool>,
}
impl DenyRulePatch { pub fn is_empty(&self) -> bool; pub fn check(&self) -> Result<(), Vec<String>>; }
/// `current` is the rule exactly as the PCE returned it (see the spec refinement).
pub fn apply_rule_patch(current: &serde_json::Value, patch: &RulePatch) -> Result<serde_json::Value, Vec<String>>;
pub fn apply_deny_rule_patch(current: &serde_json::Value, patch: &DenyRulePatch) -> Result<serde_json::Value, Vec<String>>;
impl Writer<'_> {
    pub fn plan_create_rule(&self, ruleset_href: &str, draft: &RuleDraft) -> Result<Plan, WriteError>;
    pub async fn plan_update_rule(&self, href: &str, patch: &RulePatch) -> Result<Plan, WriteError>;
    pub fn plan_delete_rule(&self, href: &str) -> Result<Plan, WriteError>;
    pub fn plan_create_deny_rule(&self, ruleset_href: &str, draft: &DenyRuleDraft) -> Result<Plan, WriteError>;
    pub async fn plan_update_deny_rule(&self, href: &str, patch: &DenyRulePatch) -> Result<Plan, WriteError>;
    pub fn plan_delete_deny_rule(&self, href: &str) -> Result<Plan, WriteError>;
}
```

- [ ] **Step 1: Write the failing tests**

Create `crates/pce/src/write/rules.rs` holding only its doc comment and this test module (Step 3 inserts the code above it):

```rust
//! Allow rules (`sec_rules`) and deny rules inside a ruleset.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::write::actors::ActorKind;
    use crate::write::specs::PortSpec;

    fn rule2() -> Value {
        json!({
            "href": "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/2", "enabled": true,
            "description": "corp to web",
            "providers": [{"label": {"href": "/orgs/5/labels/1"}}],
            "consumers": [{"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/21"}}],
            "ingress_services": [
                {"port": 443, "proto": 6},
                {"port": 8000, "to_port": 8099, "proto": 6},
                {"proto": 1, "icmp_type": 8}
            ],
            "resolve_labels_as": {"providers": ["workloads"], "consumers": ["workloads"]},
            "unscoped_consumers": true
        })
    }

    fn port(s: &str) -> ServiceRef {
        ServiceRef::Port(PortSpec::parse(s).unwrap())
    }

    fn label(id: u32) -> ActorRef {
        ActorRef::new(ActorKind::Label, format!("/orgs/5/labels/{id}"))
    }

    #[test]
    fn removing_a_range_by_its_spec_keeps_the_other_entries_verbatim() {
        let patch = RulePatch {
            remove_services: vec![port("8000-8099/tcp")],
            ..RulePatch::default()
        };
        assert_eq!(
            apply_rule_patch(&rule2(), &patch).unwrap(),
            json!({"ingress_services": [{"port": 443, "proto": 6}, {"proto": 1, "icmp_type": 8}]})
        );
    }

    #[test]
    fn an_entry_with_unknown_keys_is_never_matched_by_a_remove() {
        let patch = RulePatch {
            remove_services: vec![port("icmp")],
            ..RulePatch::default()
        };
        assert_eq!(
            apply_rule_patch(&rule2(), &patch).unwrap_err(),
            vec!["icmp is not among the services"]
        );
    }

    #[test]
    fn adding_an_existing_entry_changes_nothing_and_a_swap_is_one_list() {
        let same = RulePatch {
            add_services: vec![port("443/tcp")],
            add_consumers: vec![ActorRef::new(
                ActorKind::IpList,
                "/orgs/5/sec_policy/draft/ip_lists/21",
            )],
            ..RulePatch::default()
        };
        assert_eq!(apply_rule_patch(&rule2(), &same).unwrap(), json!({}));
        let swap = RulePatch {
            remove_providers: vec![label(1)],
            add_providers: vec![label(2)],
            ..RulePatch::default()
        };
        assert_eq!(
            apply_rule_patch(&rule2(), &swap).unwrap(),
            json!({"providers": [{"label": {"href": "/orgs/5/labels/2"}}]})
        );
    }

    #[test]
    fn removing_the_last_provider_or_an_absent_consumer_is_refused() {
        let last = RulePatch {
            remove_providers: vec![label(1)],
            ..RulePatch::default()
        };
        assert_eq!(
            apply_rule_patch(&rule2(), &last).unwrap_err(),
            vec!["a rule needs at least one provider"]
        );
        let absent = RulePatch {
            remove_consumers: vec![ActorRef::all()],
            ..RulePatch::default()
        };
        assert_eq!(
            apply_rule_patch(&rule2(), &absent).unwrap_err(),
            vec!["all is not among the consumers"]
        );
    }

    #[test]
    fn text_and_bool_fields_equal_to_the_current_value_send_nothing() {
        let patch = RulePatch {
            description: Some("corp to web".into()),
            enabled: Some(true),
            unscoped_consumers: Some(false),
            resolve_labels_as: Some(ResolveLabelsAs {
                providers: vec!["workloads".into()],
                consumers: vec!["workloads".into()],
            }),
            ..RulePatch::default()
        };
        assert_eq!(
            apply_rule_patch(&rule2(), &patch).unwrap(),
            json!({"unscoped_consumers": false})
        );
    }

    #[test]
    fn deny_patch_toggles_override_and_changes_only_what_differs() {
        let current = json!({
            "href": "/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8", "enabled": true,
            "providers": [{"label": {"href": "/orgs/5/labels/2"}}], "consumers": [{"actors": "ams"}],
            "ingress_services": [{"port": 22, "proto": 6}], "override": true
        });
        let off = DenyRulePatch {
            override_: Some(false),
            ..DenyRulePatch::default()
        };
        assert_eq!(
            apply_deny_rule_patch(&current, &off).unwrap(),
            json!({"override": false})
        );
        let same = DenyRulePatch {
            override_: Some(true),
            description: Some("ssh".into()),
            ..DenyRulePatch::default()
        };
        assert_eq!(
            apply_deny_rule_patch(&current, &same).unwrap(),
            json!({"description": "ssh"})
        );
    }

    #[test]
    fn excluded_all_and_bad_resolve_labels_as_are_refused() {
        let p = RulePatch {
            add_consumers: vec![ActorRef::all().excluded()],
            ..RulePatch::default()
        };
        assert_eq!(p.check().unwrap_err(), vec!["all cannot be excluded"]);
        let d = RuleDraft {
            description: None,
            enabled: true,
            providers: vec![ActorRef::all()],
            consumers: vec![ActorRef::all()],
            services: vec![port("443/tcp")],
            unscoped_consumers: false,
            resolve_labels_as: Some(ResolveLabelsAs {
                providers: vec![],
                consumers: vec!["vms".into()],
            }),
        };
        assert_eq!(
            d.validate().unwrap_err(),
            vec![
                "resolve_labels_as providers must name workloads or virtual_services",
                "resolve_labels_as consumers: \"vms\" is not workloads or virtual_services",
            ]
        );
        assert!(RulePatch::default().is_empty());
        assert!(DenyRulePatch::default().is_empty());
    }
}
```

Add `pub mod rules;` to `crates/pce/src/write/mod.rs` so the test module compiles (and fails).

Append to `crates/pce/tests/policy_writes.rs`:

```rust
mod rules {
    use super::*;
    use pce::models::ResolveLabelsAs;
    use pce::write::{
        ActorKind, ActorRef, DenyRuleDraft, DenyRulePatch, PortSpec, RuleDraft, RulePatch,
        ServiceRef,
    };
    use wiremock::matchers::body_json;

    const RULE_1: &str = "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1";

    fn label(id: u32) -> ActorRef {
        ActorRef::new(ActorKind::Label, format!("/orgs/5/labels/{id}"))
    }

    fn port(s: &str) -> ServiceRef {
        ServiceRef::Port(PortSpec::parse(s).unwrap())
    }

    fn mixed() -> RuleDraft {
        RuleDraft {
            description: Some("web tier".into()),
            enabled: true,
            providers: vec![
                label(1),
                ActorRef::new(
                    ActorKind::LabelGroup,
                    "/orgs/5/sec_policy/draft/label_groups/10",
                ),
            ],
            consumers: vec![
                ActorRef::all(),
                ActorRef::new(ActorKind::IpList, "/orgs/5/sec_policy/draft/ip_lists/20")
                    .excluded(),
            ],
            services: vec![
                ServiceRef::Href("/orgs/5/sec_policy/draft/services/31".into()),
                port("443/tcp"),
                port("8000-8099/tcp"),
            ],
            unscoped_consumers: false,
            resolve_labels_as: Some(ResolveLabelsAs {
                providers: vec!["workloads".into()],
                consumers: vec!["workloads".into()],
            }),
        }
    }

    fn mixed_body() -> Value {
        json!({
            "enabled": true, "description": "web tier",
            "providers": [
                {"label": {"href": "/orgs/5/labels/1"}},
                {"label_group": {"href": "/orgs/5/sec_policy/draft/label_groups/10"}}
            ],
            "consumers": [
                {"actors": "ams"},
                {"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/20"}, "exclusion": true}
            ],
            "ingress_services": [
                {"href": "/orgs/5/sec_policy/draft/services/31"},
                {"port": 443, "proto": 6},
                {"port": 8000, "to_port": 8099, "proto": 6}
            ],
            "unscoped_consumers": false,
            "resolve_labels_as": {"providers": ["workloads"], "consumers": ["workloads"]}
        })
    }

    /// Rule 1 with fields the models do not name, an extra key on a
    /// provider and an inline ICMP service the typed model cannot hold.
    fn rule_1() -> Value {
        json!({
            "href": RULE_1, "enabled": true, "description": "web to db",
            "providers": [{"label": {"href": "/orgs/5/labels/2"}, "exclusion": false, "future_flag": "x"}],
            "consumers": [{"label": {"href": "/orgs/5/labels/1"}}],
            "ingress_services": [
                {"href": "/orgs/5/sec_policy/draft/services/31"},
                {"proto": 1, "icmp_type": 8, "icmp_code": 0}
            ],
            "sec_connect": false, "egress_services": [], "use_workload_subnets": ["providers"]
        })
    }

    async fn mount_rule_1(server: &MockServer) {
        Mock::given(method("GET"))
            .and(path(format!("/api/v2{RULE_1}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(rule_1()))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn create_posts_under_the_ruleset_with_mixed_actors_and_sends_once() {
        let server = MockServer::start().await;
        let mut created = mixed_body();
        created["href"] = json!("/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/9");
        Mock::given(method("POST"))
            .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets/100/sec_rules"))
            .and(body_json(mixed_body()))
            .respond_with(ResponseTemplate::new(201).set_body_json(created))
            .expect(1)
            .mount(&server)
            .await;
        let pce = pce_for(&server);
        let audit = MemoryAudit::new();
        let guard = guard_with(audit.clone());
        let w = pce.writer(&guard);
        let plan = w
            .plan_create_rule("/orgs/5/sec_policy/active/rule_sets/100", &mixed())
            .unwrap();
        let req = plan.request().unwrap();
        assert_eq!(req.method, WriteMethod::Post);
        assert_eq!(req.path, "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules");
        assert_eq!(req.body, Some(mixed_body()));
        assert_eq!(
            req.summary,
            "create rule in rule_set /orgs/5/sec_policy/draft/rule_sets/100"
        );
        assert_eq!(request_count(&server).await, 0, "planning sends nothing");
        let Outcome::Written(out) = w.execute(plan).await.unwrap() else {
            panic!("expected Written")
        };
        assert_eq!(out.href, "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/9");
        assert!(out.pending);
        assert!(out.dropped_fields.is_empty());
        assert_eq!(audit.entries().len(), 1);
    }

    #[tokio::test]
    async fn create_validation_errors_send_nothing() {
        let server = MockServer::start().await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let w = pce.writer(&guard);
        let empty = RuleDraft {
            providers: vec![],
            consumers: vec![],
            services: vec![],
            resolve_labels_as: None,
            ..mixed()
        };
        let rs = "/orgs/5/sec_policy/draft/rule_sets/100";
        assert_eq!(
            invalid(w.plan_create_rule(rs, &empty).unwrap_err()),
            "a rule needs at least one provider; a rule needs at least one consumer; a rule needs at least one service"
        );
        let excluded_all = RuleDraft {
            consumers: vec![ActorRef::all().excluded()],
            ..mixed()
        };
        assert_eq!(
            invalid(w.plan_create_rule(rs, &excluded_all).unwrap_err()),
            "all cannot be excluded"
        );
        let m = invalid(w.plan_create_rule(RULE_1, &mixed()).unwrap_err());
        assert!(m.contains("is not a rule_sets href"), "{m}");
        let deny = DenyRuleDraft {
            description: None,
            enabled: true,
            providers: vec![ActorRef::all()],
            consumers: vec![],
            services: vec![port("22/tcp")],
            override_: false,
        };
        assert_eq!(
            invalid(w.plan_create_deny_rule(rs, &deny).unwrap_err()),
            "a rule needs at least one consumer"
        );
        assert_eq!(request_count(&server).await, 0);
    }

    #[tokio::test]
    async fn update_add_provider_keeps_extras_and_sends_providers_whole() {
        let server = MockServer::start().await;
        mount_rule_1(&server).await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let patch = RulePatch {
            add_providers: vec![label(4)],
            ..RulePatch::default()
        };
        let plan = pce
            .writer(&guard)
            .plan_update_rule(
                "/orgs/5/sec_policy/active/rule_sets/100/sec_rules/1",
                &patch,
            )
            .await
            .unwrap();
        let req = plan.request().unwrap();
        assert_eq!(req.method, WriteMethod::Put);
        assert_eq!(req.path, RULE_1);
        assert_eq!(
            req.body,
            Some(json!({"providers": [
                {"label": {"href": "/orgs/5/labels/2"}, "exclusion": false, "future_flag": "x"},
                {"label": {"href": "/orgs/5/labels/4"}}
            ]}))
        );
        assert_eq!(req.summary, format!("update rule {RULE_1}"));
        assert_eq!(request_count(&server).await, 1, "one GET of the rule");
    }

    #[tokio::test]
    async fn description_only_patch_sends_only_the_description() {
        let server = MockServer::start().await;
        mount_rule_1(&server).await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let patch = RulePatch {
            description: Some("web to db, v2".into()),
            ..RulePatch::default()
        };
        let plan = pce
            .writer(&guard)
            .plan_update_rule(RULE_1, &patch)
            .await
            .unwrap();
        assert_eq!(
            plan.request().unwrap().body,
            Some(json!({"description": "web to db, v2"}))
        );
    }

    #[tokio::test]
    async fn removing_an_absent_consumer_is_invalid_and_nothing_is_sent() {
        let server = MockServer::start().await;
        mount_rule_1(&server).await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let patch = RulePatch {
            remove_consumers: vec![ActorRef::new(
                ActorKind::IpList,
                "/orgs/5/sec_policy/draft/ip_lists/99",
            )],
            ..RulePatch::default()
        };
        let m = invalid(
            pce.writer(&guard)
                .plan_update_rule(RULE_1, &patch)
                .await
                .unwrap_err(),
        );
        assert_eq!(
            m,
            "ip_list /orgs/5/sec_policy/draft/ip_lists/99 is not among the consumers"
        );
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method.as_str() == "GET")
        );
    }

    #[tokio::test]
    async fn empty_patches_and_wrong_href_shapes_are_refused_before_any_request() {
        let server = MockServer::start().await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let w = pce.writer(&guard);
        assert_eq!(
            invalid(
                w.plan_update_rule(RULE_1, &RulePatch::default())
                    .await
                    .unwrap_err()
            ),
            "nothing to change"
        );
        let patch = RulePatch {
            description: Some("x".into()),
            ..RulePatch::default()
        };
        for bad in [
            "/orgs/5/sec_policy/draft/rule_sets/100",
            "/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/7",
            "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/../1",
            "/orgs/7/sec_policy/draft/rule_sets/100/sec_rules/1",
        ] {
            invalid(w.plan_update_rule(bad, &patch).await.unwrap_err());
            invalid(w.plan_delete_rule(bad).unwrap_err());
        }
        let deny_patch = DenyRulePatch {
            override_: Some(true),
            ..DenyRulePatch::default()
        };
        let m = invalid(w.plan_update_deny_rule(RULE_1, &deny_patch).await.unwrap_err());
        assert!(
            m.contains("expected /orgs/<org>/sec_policy/draft/rule_sets/<id>/deny_rules/<id>"),
            "{m}"
        );
        assert_eq!(request_count(&server).await, 0);
    }

    #[tokio::test]
    async fn deny_rule_with_override_create_update_and_delete() {
        let server = MockServer::start().await;
        let deny_rules: Value =
            serde_json::from_str(include_str!("../../../fixtures/deny_rules.json")).unwrap();
        Mock::given(method("GET"))
            .and(path(
                "/api/v2/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(deny_rules[1].clone()))
            .expect(1)
            .mount(&server)
            .await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let w = pce.writer(&guard);
        let draft = DenyRuleDraft {
            description: Some("no telnet".into()),
            enabled: true,
            providers: vec![ActorRef::all()],
            consumers: vec![ActorRef::new(
                ActorKind::IpList,
                "/orgs/5/sec_policy/draft/ip_lists/20",
            )],
            services: vec![port("23/tcp")],
            override_: true,
        };
        let plan = w
            .plan_create_deny_rule("/orgs/5/sec_policy/draft/rule_sets/100", &draft)
            .unwrap();
        let req = plan.request().unwrap();
        assert_eq!(req.path, "/orgs/5/sec_policy/draft/rule_sets/100/deny_rules");
        assert_eq!(
            req.body,
            Some(json!({
                "enabled": true, "description": "no telnet",
                "providers": [{"actors": "ams"}],
                "consumers": [{"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/20"}}],
                "ingress_services": [{"port": 23, "proto": 6}],
                "override": true
            }))
        );
        let patch = DenyRulePatch {
            override_: Some(false),
            ..DenyRulePatch::default()
        };
        let plan = w
            .plan_update_deny_rule(
                "/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8",
                &patch,
            )
            .await
            .unwrap();
        let req = plan.request().unwrap();
        assert_eq!(req.body, Some(json!({"override": false})));
        assert_eq!(
            req.summary,
            "update deny_rule /orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8"
        );
        let plan = w
            .plan_delete_deny_rule("/orgs/5/sec_policy/active/rule_sets/100/deny_rules/8")
            .unwrap();
        let req = plan.request().unwrap();
        assert_eq!(req.method, WriteMethod::Delete);
        assert_eq!(req.path, "/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8");
        assert!(matches!(
            plan,
            Plan::Send {
                kind: ObjectKind::DenyRule,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn delete_of_a_nested_rule_href() {
        let server = MockServer::start().await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let plan = pce
            .writer(&guard)
            .plan_delete_rule("/orgs/5/sec_policy/active/rule_sets/100/sec_rules/1")
            .unwrap();
        let req = plan.request().unwrap();
        assert_eq!(req.method, WriteMethod::Delete);
        assert_eq!(req.path, RULE_1);
        assert!(req.body.is_none());
        assert_eq!(req.summary, format!("delete rule {RULE_1}"));
        assert!(matches!(
            plan,
            Plan::Send {
                kind: ObjectKind::Rule,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn a_rule_that_already_matches_is_a_noop() {
        let server = MockServer::start().await;
        mount_rule_1(&server).await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let patch = RulePatch {
            enabled: Some(true),
            add_consumers: vec![label(1)],
            ..RulePatch::default()
        };
        let plan = pce
            .writer(&guard)
            .plan_update_rule(RULE_1, &patch)
            .await
            .unwrap();
        let Plan::Noop {
            existing_href,
            reason,
        } = plan
        else {
            panic!("expected Noop")
        };
        assert_eq!(existing_href, RULE_1);
        assert_eq!(reason, "rule already matches");
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p pce --lib write::rules && cargo test -p pce --test policy_writes rules`
Expected: compile errors: `cannot find type 'RulePatch' in this scope` (unit tests), `unresolved imports pce::write::DenyRuleDraft ...` (contract tests).

- [ ] **Step 3: Implement** (insert above `#[cfg(test)]` in `crates/pce/src/write/rules.rs`, and extend the doc comment)

```rust
//! Create is a `POST` under the ruleset's href; update and delete address
//! the nested href. An update `PUT`s only the keys it changes: a text or
//! bool field equal to the current value contributes nothing, and a
//! changed provider, consumer or service list is sent whole with the
//! untouched entries exactly as the PCE returned them.
use super::actors::{ActorRef, ServiceRef};
use super::{
    ObjectKind, Plan, WriteError, WriteRequest, Writer, checked_nested_href, checked_policy_href,
};
use crate::client::WriteMethod;
use crate::models::{Actor, IngressService, ResolveLabelsAs};
use serde_json::{Map, Value, json};

#[derive(Debug, Clone, PartialEq)]
pub struct RuleDraft {
    pub description: Option<String>,
    pub enabled: bool,
    pub providers: Vec<ActorRef>,
    pub consumers: Vec<ActorRef>,
    pub services: Vec<ServiceRef>,
    pub unscoped_consumers: bool,
    /// Absent: omitted from the body, so the PCE applies its default.
    pub resolve_labels_as: Option<ResolveLabelsAs>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RulePatch {
    pub description: Option<String>,
    pub enabled: Option<bool>,
    pub add_providers: Vec<ActorRef>,
    pub remove_providers: Vec<ActorRef>,
    pub add_consumers: Vec<ActorRef>,
    pub remove_consumers: Vec<ActorRef>,
    pub add_services: Vec<ServiceRef>,
    pub remove_services: Vec<ServiceRef>,
    pub unscoped_consumers: Option<bool>,
    pub resolve_labels_as: Option<ResolveLabelsAs>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DenyRuleDraft {
    pub description: Option<String>,
    pub enabled: bool,
    pub providers: Vec<ActorRef>,
    pub consumers: Vec<ActorRef>,
    pub services: Vec<ServiceRef>,
    /// Sent as `override`: an override deny is evaluated before allow rules.
    pub override_: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DenyRulePatch {
    pub description: Option<String>,
    pub enabled: Option<bool>,
    pub add_providers: Vec<ActorRef>,
    pub remove_providers: Vec<ActorRef>,
    pub add_consumers: Vec<ActorRef>,
    pub remove_consumers: Vec<ActorRef>,
    pub add_services: Vec<ServiceRef>,
    pub remove_services: Vec<ServiceRef>,
    pub override_: Option<bool>,
}

fn done(errs: Vec<String>) -> Result<(), Vec<String>> {
    if errs.is_empty() { Ok(()) } else { Err(errs) }
}

fn check_actors<'a>(actors: impl Iterator<Item = &'a ActorRef>, errs: &mut Vec<String>) {
    for a in actors {
        if let Err(e) = a.validate()
            && !errs.contains(&e)
        {
            errs.push(e);
        }
    }
}

fn check_sides(
    providers: &[ActorRef],
    consumers: &[ActorRef],
    services: &[ServiceRef],
    errs: &mut Vec<String>,
) {
    if providers.is_empty() {
        errs.push("a rule needs at least one provider".into());
    }
    if consumers.is_empty() {
        errs.push("a rule needs at least one consumer".into());
    }
    if services.is_empty() {
        errs.push("a rule needs at least one service".into());
    }
    check_actors(providers.iter().chain(consumers), errs);
}

fn check_resolve(r: &ResolveLabelsAs, errs: &mut Vec<String>) {
    for (side, values) in [("providers", &r.providers), ("consumers", &r.consumers)] {
        if values.is_empty() {
            errs.push(format!(
                "resolve_labels_as {side} must name workloads or virtual_services"
            ));
        }
        for v in values {
            if !matches!(v.as_str(), "workloads" | "virtual_services") {
                errs.push(format!(
                    "resolve_labels_as {side}: {v:?} is not workloads or virtual_services"
                ));
            }
        }
    }
}

fn actors_json(a: &[ActorRef]) -> Value {
    Value::Array(a.iter().map(ActorRef::to_json).collect())
}

fn services_json(s: &[ServiceRef]) -> Value {
    Value::Array(s.iter().map(ServiceRef::to_json).collect())
}

impl RuleDraft {
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errs = Vec::new();
        check_sides(&self.providers, &self.consumers, &self.services, &mut errs);
        if let Some(r) = &self.resolve_labels_as {
            check_resolve(r, &mut errs);
        }
        done(errs)
    }

    pub(crate) fn to_body(&self) -> Value {
        let mut v = json!({
            "enabled": self.enabled,
            "providers": actors_json(&self.providers),
            "consumers": actors_json(&self.consumers),
            "ingress_services": services_json(&self.services),
            "unscoped_consumers": self.unscoped_consumers,
        });
        if let Some(d) = &self.description {
            v["description"] = json!(d);
        }
        if let Some(r) = &self.resolve_labels_as {
            v["resolve_labels_as"] = json!({"providers": r.providers, "consumers": r.consumers});
        }
        v
    }
}

impl DenyRuleDraft {
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errs = Vec::new();
        check_sides(&self.providers, &self.consumers, &self.services, &mut errs);
        done(errs)
    }

    pub(crate) fn to_body(&self) -> Value {
        let mut v = json!({
            "enabled": self.enabled,
            "providers": actors_json(&self.providers),
            "consumers": actors_json(&self.consumers),
            "ingress_services": services_json(&self.services),
            "override": self.override_,
        });
        if let Some(d) = &self.description {
            v["description"] = json!(d);
        }
        v
    }
}

/// One entry of a provider, consumer or service list.
trait Entry: Clone {
    /// The entry as the PCE returned it; `None` when it cannot be matched
    /// (it is then carried untouched and no remove can hit it).
    fn parse(v: &Value) -> Option<Self>;
    fn same(&self, other: &Self) -> bool;
    fn json(&self) -> Value;
    fn label(&self) -> String;
}

impl Entry for ActorRef {
    fn parse(v: &Value) -> Option<Self> {
        let a: Actor = serde_json::from_value(v.clone()).ok()?;
        ActorRef::from_actor(&a).ok()
    }
    fn same(&self, other: &Self) -> bool {
        self.same_actor(other)
    }
    fn json(&self) -> Value {
        self.to_json()
    }
    fn label(&self) -> String {
        self.display()
    }
}

impl Entry for ServiceRef {
    fn parse(v: &Value) -> Option<Self> {
        let known = v.as_object()?.keys().all(|k| {
            matches!(k.as_str(), "href" | "port" | "to_port" | "proto")
        });
        if !known {
            return None;
        }
        let s: IngressService = serde_json::from_value(v.clone()).ok()?;
        Some(ServiceRef::from_ingress(&s))
    }
    fn same(&self, other: &Self) -> bool {
        self.same_service(other)
    }
    fn json(&self) -> Value {
        self.to_json()
    }
    fn label(&self) -> String {
        self.display()
    }
}

/// Removes, then adds. Returns the new list only when it changed; every
/// entry not removed is the PCE's JSON, unchanged.
fn patch_list<T: Entry>(
    what: &str,
    current: &[Value],
    add: &[T],
    remove: &[T],
    errs: &mut Vec<String>,
) -> Option<Vec<Value>> {
    let mut entries: Vec<(Option<T>, Value)> =
        current.iter().map(|v| (T::parse(v), v.clone())).collect();
    let mut changed = false;
    for r in remove {
        let before = entries.len();
        entries.retain(|(e, _)| !e.as_ref().is_some_and(|e| e.same(r)));
        if entries.len() == before {
            errs.push(format!("{} is not among the {what}", r.label()));
        } else {
            changed = true;
        }
    }
    for a in add {
        if !entries
            .iter()
            .any(|(e, _)| e.as_ref().is_some_and(|e| e.same(a)))
        {
            entries.push((Some(a.clone()), a.json()));
            changed = true;
        }
    }
    changed.then(|| entries.into_iter().map(|(_, v)| v).collect())
}

fn list_of<'v>(current: &'v Value, key: &str) -> &'v [Value] {
    current
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn text_change(body: &mut Map<String, Value>, current: &Value, key: &str, new: Option<&str>) {
    if let Some(new) = new
        && current.get(key).and_then(Value::as_str).unwrap_or("") != new
    {
        body.insert(key.into(), json!(new));
    }
}

fn bool_change(body: &mut Map<String, Value>, current: &Value, key: &str, new: Option<bool>) {
    if let Some(new) = new
        && current.get(key).and_then(Value::as_bool).unwrap_or(false) != new
    {
        body.insert(key.into(), json!(new));
    }
}

/// The list edits a rule patch and a deny-rule patch share.
struct Sides<'a> {
    add_providers: &'a [ActorRef],
    remove_providers: &'a [ActorRef],
    add_consumers: &'a [ActorRef],
    remove_consumers: &'a [ActorRef],
    add_services: &'a [ServiceRef],
    remove_services: &'a [ServiceRef],
}

impl Sides<'_> {
    fn is_empty(&self) -> bool {
        self.add_providers.is_empty()
            && self.remove_providers.is_empty()
            && self.add_consumers.is_empty()
            && self.remove_consumers.is_empty()
            && self.add_services.is_empty()
            && self.remove_services.is_empty()
    }

    fn check(&self, errs: &mut Vec<String>) {
        check_actors(self.add_providers.iter().chain(self.add_consumers), errs);
    }

    fn apply(&self, current: &Value, body: &mut Map<String, Value>, errs: &mut Vec<String>) {
        for (key, add, remove, empty) in [
            (
                "providers",
                self.add_providers,
                self.remove_providers,
                "a rule needs at least one provider",
            ),
            (
                "consumers",
                self.add_consumers,
                self.remove_consumers,
                "a rule needs at least one consumer",
            ),
        ] {
            if let Some(list) = patch_list(key, list_of(current, key), add, remove, errs) {
                if list.is_empty() {
                    errs.push(empty.into());
                }
                body.insert(key.into(), Value::Array(list));
            }
        }
        if let Some(list) = patch_list(
            "services",
            list_of(current, "ingress_services"),
            self.add_services,
            self.remove_services,
            errs,
        ) {
            if list.is_empty() {
                errs.push("a rule needs at least one service".into());
            }
            body.insert("ingress_services".into(), Value::Array(list));
        }
    }
}

impl RulePatch {
    fn sides(&self) -> Sides<'_> {
        Sides {
            add_providers: &self.add_providers,
            remove_providers: &self.remove_providers,
            add_consumers: &self.add_consumers,
            remove_consumers: &self.remove_consumers,
            add_services: &self.add_services,
            remove_services: &self.remove_services,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.description.is_none()
            && self.enabled.is_none()
            && self.unscoped_consumers.is_none()
            && self.resolve_labels_as.is_none()
            && self.sides().is_empty()
    }

    /// The errors that need no current rule.
    pub fn check(&self) -> Result<(), Vec<String>> {
        let mut errs = Vec::new();
        self.sides().check(&mut errs);
        if let Some(r) = &self.resolve_labels_as {
            check_resolve(r, &mut errs);
        }
        done(errs)
    }
}

impl DenyRulePatch {
    fn sides(&self) -> Sides<'_> {
        Sides {
            add_providers: &self.add_providers,
            remove_providers: &self.remove_providers,
            add_consumers: &self.add_consumers,
            remove_consumers: &self.remove_consumers,
            add_services: &self.add_services,
            remove_services: &self.remove_services,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.description.is_none()
            && self.enabled.is_none()
            && self.override_.is_none()
            && self.sides().is_empty()
    }

    pub fn check(&self) -> Result<(), Vec<String>> {
        let mut errs = Vec::new();
        self.sides().check(&mut errs);
        done(errs)
    }
}

/// The `PUT` body for `patch` against the rule as the PCE returned it:
/// only the keys that change.
pub fn apply_rule_patch(current: &Value, patch: &RulePatch) -> Result<Value, Vec<String>> {
    let mut errs = patch.check().err().unwrap_or_default();
    let mut body = Map::new();
    text_change(&mut body, current, "description", patch.description.as_deref());
    bool_change(&mut body, current, "enabled", patch.enabled);
    bool_change(&mut body, current, "unscoped_consumers", patch.unscoped_consumers);
    if let Some(r) = &patch.resolve_labels_as {
        let new = json!({"providers": r.providers, "consumers": r.consumers});
        if current.get("resolve_labels_as") != Some(&new) {
            body.insert("resolve_labels_as".into(), new);
        }
    }
    patch.sides().apply(current, &mut body, &mut errs);
    if errs.is_empty() {
        Ok(Value::Object(body))
    } else {
        Err(errs)
    }
}

/// [`apply_rule_patch`] for a deny rule (`override` instead of
/// `unscoped_consumers` and `resolve_labels_as`).
pub fn apply_deny_rule_patch(current: &Value, patch: &DenyRulePatch) -> Result<Value, Vec<String>> {
    let mut errs = patch.check().err().unwrap_or_default();
    let mut body = Map::new();
    text_change(&mut body, current, "description", patch.description.as_deref());
    bool_change(&mut body, current, "enabled", patch.enabled);
    bool_change(&mut body, current, "override", patch.override_);
    patch.sides().apply(current, &mut body, &mut errs);
    if errs.is_empty() {
        Ok(Value::Object(body))
    } else {
        Err(errs)
    }
}

fn create(ruleset: &str, child: &str, what: &str, body: Value, kind: ObjectKind) -> Plan {
    Plan::Send {
        request: WriteRequest {
            method: WriteMethod::Post,
            path: format!("{ruleset}/{child}"),
            body: Some(body),
            summary: format!("create {what} in rule_set {ruleset}"),
        },
        kind,
    }
}

fn update(href: String, what: &str, body: Value, kind: ObjectKind) -> Plan {
    if body.as_object().is_some_and(Map::is_empty) {
        return Plan::Noop {
            existing_href: href,
            reason: format!("{what} already matches"),
        };
    }
    Plan::Send {
        request: WriteRequest {
            method: WriteMethod::Put,
            path: href.clone(),
            body: Some(body),
            summary: format!("update {what} {href}"),
        },
        kind,
    }
}

fn delete(href: String, what: &str, kind: ObjectKind) -> Plan {
    Plan::Send {
        request: WriteRequest {
            method: WriteMethod::Delete,
            path: href.clone(),
            body: None,
            summary: format!("delete {what} {href}"),
        },
        kind,
    }
}

impl Writer<'_> {
    /// One GET of a rule or deny rule, kept as JSON (see `apply_rule_patch`).
    async fn fetch_rule(&self, href: &str) -> Result<Value, WriteError> {
        let v = self.fetch(href).await?;
        if v.is_object() {
            Ok(v)
        } else {
            Err(WriteError::Pce(crate::PceError::Decode(format!(
                "{href}: expected a rule object"
            ))))
        }
    }

    pub fn plan_create_rule(&self, ruleset_href: &str, draft: &RuleDraft) -> Result<Plan, WriteError> {
        let rs = checked_policy_href(ruleset_href, "rule_sets", self.pce().client().org_id())?;
        draft.validate().map_err(WriteError::Invalid)?;
        Ok(create(&rs, "sec_rules", "rule", draft.to_body(), ObjectKind::Rule))
    }

    pub async fn plan_update_rule(&self, href: &str, patch: &RulePatch) -> Result<Plan, WriteError> {
        let href = checked_nested_href(href, "rule_sets", "sec_rules", self.pce().client().org_id())?;
        if patch.is_empty() {
            return Err(WriteError::invalid("nothing to change"));
        }
        patch.check().map_err(WriteError::Invalid)?;
        let current = self.fetch_rule(&href).await?;
        let body = apply_rule_patch(&current, patch).map_err(WriteError::Invalid)?;
        Ok(update(href, "rule", body, ObjectKind::Rule))
    }

    pub fn plan_delete_rule(&self, href: &str) -> Result<Plan, WriteError> {
        let href = checked_nested_href(href, "rule_sets", "sec_rules", self.pce().client().org_id())?;
        Ok(delete(href, "rule", ObjectKind::Rule))
    }

    pub fn plan_create_deny_rule(
        &self,
        ruleset_href: &str,
        draft: &DenyRuleDraft,
    ) -> Result<Plan, WriteError> {
        let rs = checked_policy_href(ruleset_href, "rule_sets", self.pce().client().org_id())?;
        draft.validate().map_err(WriteError::Invalid)?;
        Ok(create(&rs, "deny_rules", "deny_rule", draft.to_body(), ObjectKind::DenyRule))
    }

    pub async fn plan_update_deny_rule(
        &self,
        href: &str,
        patch: &DenyRulePatch,
    ) -> Result<Plan, WriteError> {
        let href = checked_nested_href(href, "rule_sets", "deny_rules", self.pce().client().org_id())?;
        if patch.is_empty() {
            return Err(WriteError::invalid("nothing to change"));
        }
        patch.check().map_err(WriteError::Invalid)?;
        let current = self.fetch_rule(&href).await?;
        let body = apply_deny_rule_patch(&current, patch).map_err(WriteError::Invalid)?;
        Ok(update(href, "deny_rule", body, ObjectKind::DenyRule))
    }

    pub fn plan_delete_deny_rule(&self, href: &str) -> Result<Plan, WriteError> {
        let href = checked_nested_href(href, "rule_sets", "deny_rules", self.pce().client().org_id())?;
        Ok(delete(href, "deny_rule", ObjectKind::DenyRule))
    }
}
```

The deny-rule GET in the contract test uses the fixture entry `deny_rules.json[1]` (`override: true`, provider label 2, consumer `ams`, port 22/tcp).

Add `pub use rules::{DenyRuleDraft, DenyRulePatch, RuleDraft, RulePatch, apply_deny_rule_patch, apply_rule_patch};` to `crates/pce/src/write/mod.rs`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p pce --lib write::rules && cargo test -p pce --test policy_writes`
Expected: PASS (7 unit tests, 9 new contract tests).

- [ ] **Step 5: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p pce
git add crates/pce/src/write/rules.rs crates/pce/src/write/mod.rs crates/pce/tests/policy_writes.rs
git commit -m "feat(pce): allow and deny rules: nested-href plans, list patches that carry untouched entries

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 5: Provisioning

**Files:**
- Create: `crates/pce/src/write/provision.rs`
- Modify: `crates/pce/src/write/mod.rs` (`pub mod provision;`, `pub use provision::{ProvisionRequest, ProvisionSubset, provision_counts};`; `Writer::execute` `dropped` computation ~lines 371-375)
- Test: `crates/pce/tests/policy_writes.rs` (append `mod provision`)

**Interfaces:**
- Consumes: `Pce::pending_changes(CacheMode::Bypass)` (`lib.rs` ~264: one `GET sec_policy/pending`, parsed by `models::parse_pending` into `PendingChange { href, change_type, name, object_type }` for both the list and the dict shape), `checked_nested_href`, `checked_policy_href`, `parent_href` (Task 1), `Writer::bypass`, `ObjectKind::Provision` (Task 1: prefixes `policy:` and `objects:`, not pending).
- Produces:

```rust
pub enum ProvisionSubset { All, Hrefs(Vec<String>) }
pub struct ProvisionRequest { pub description: Option<String>, pub subset: ProvisionSubset }
/// object type → number of hrefs, from the body's `change_subset` (empty lists left out).
pub fn provision_counts(body: &serde_json::Value) -> std::collections::BTreeMap<String, usize>;
impl Writer<'_> {
    pub async fn plan_provision(&self, req: &ProvisionRequest) -> Result<Plan, WriteError>;
}
```

- [ ] **Step 1: Write the failing tests** (append to `crates/pce/tests/policy_writes.rs`)

```rust
mod provision {
    use super::*;
    use pce::write::{ProvisionRequest, ProvisionSubset, provision_counts};
    use std::collections::BTreeMap;
    use wiremock::matchers::body_json;

    fn all() -> ProvisionRequest {
        ProvisionRequest {
            description: None,
            subset: ProvisionSubset::All,
        }
    }

    fn hrefs(h: &[&str]) -> ProvisionRequest {
        ProvisionRequest {
            description: None,
            subset: ProvisionSubset::Hrefs(h.iter().map(|s| s.to_string()).collect()),
        }
    }

    async fn mount_pending(server: &MockServer, body: Value) {
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/sec_policy/pending"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(server)
            .await;
    }

    fn fixture_body() -> Value {
        json!({
            "update_description": "illumio-tui provision",
            "change_subset": {
                "ip_lists": [{"href": "/orgs/5/sec_policy/draft/ip_lists/21"}],
                "rule_sets": [{"href": "/orgs/5/sec_policy/draft/rule_sets/101"}]
            }
        })
    }

    #[tokio::test]
    async fn all_groups_every_pending_href_by_type_from_both_shapes() {
        for fixture in [
            include_str!("../../../fixtures/pending_dict.json"),
            include_str!("../../../fixtures/pending_list.json"),
        ] {
            let server = MockServer::start().await;
            mount_pending(&server, serde_json::from_str(fixture).unwrap()).await;
            let pce = pce_for(&server);
            let guard = guard_with(MemoryAudit::new());
            let plan = pce.writer(&guard).plan_provision(&all()).await.unwrap();
            let req = plan.request().unwrap();
            assert_eq!(req.method, WriteMethod::Post);
            assert_eq!(req.path, "/orgs/5/sec_policy");
            assert_eq!(req.body, Some(fixture_body()));
            assert_eq!(req.summary, "provision 1 ip_list, 1 rule_set");
            assert_eq!(
                provision_counts(req.body.as_ref().unwrap()),
                BTreeMap::from([("ip_lists".to_string(), 1), ("rule_sets".to_string(), 1)])
            );
            assert!(matches!(
                plan,
                Plan::Send {
                    kind: ObjectKind::Provision,
                    ..
                }
            ));
            assert_eq!(request_count(&server).await, 1, "one pending GET");
        }
    }

    #[tokio::test]
    async fn rule_hrefs_map_to_their_ruleset_once() {
        let server = MockServer::start().await;
        mount_pending(
            &server,
            json!([
                {"href": "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1", "change_type": "update"},
                {"href": "/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8", "change_type": "create"},
                {"href": "/orgs/5/sec_policy/draft/rule_sets/100", "change_type": "update", "name": "Shop-Prod"},
                {"href": "/orgs/5/sec_policy/draft/services/31", "change_type": "update", "name": "PostgreSQL"}
            ]),
        )
        .await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let req_all = ProvisionRequest {
            description: Some("change 42".into()),
            subset: ProvisionSubset::All,
        };
        let plan = pce.writer(&guard).plan_provision(&req_all).await.unwrap();
        let body = plan.request().unwrap().body.clone().unwrap();
        assert_eq!(
            body,
            json!({
                "update_description": "change 42",
                "change_subset": {
                    "rule_sets": [{"href": "/orgs/5/sec_policy/draft/rule_sets/100"}],
                    "services": [{"href": "/orgs/5/sec_policy/draft/services/31"}]
                }
            })
        );
        let text = body.to_string();
        assert!(!text.contains("sec_rules") && !text.contains("deny_rules"), "{text}");
        // A requested rule href provisions its ruleset.
        let plan = pce
            .writer(&guard)
            .plan_provision(&hrefs(&[
                "/orgs/5/sec_policy/active/rule_sets/100/sec_rules/1",
            ]))
            .await
            .unwrap();
        assert_eq!(
            plan.request().unwrap().body.as_ref().unwrap()["change_subset"],
            json!({"rule_sets": [{"href": "/orgs/5/sec_policy/draft/rule_sets/100"}]})
        );
    }

    #[tokio::test]
    async fn explicit_hrefs_must_be_pending() {
        let server = MockServer::start().await;
        mount_pending(
            &server,
            serde_json::from_str(include_str!("../../../fixtures/pending_list.json")).unwrap(),
        )
        .await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let w = pce.writer(&guard);
        let plan = w
            .plan_provision(&hrefs(&["/orgs/5/sec_policy/active/ip_lists/21"]))
            .await
            .unwrap();
        assert_eq!(
            plan.request().unwrap().body,
            Some(json!({
                "update_description": "illumio-tui provision",
                "change_subset": {"ip_lists": [{"href": "/orgs/5/sec_policy/draft/ip_lists/21"}]}
            }))
        );
        assert_eq!(plan.request().unwrap().summary, "provision 1 ip_list");
        let m = invalid(
            w.plan_provision(&hrefs(&["/orgs/5/sec_policy/draft/services/31"]))
                .await
                .unwrap_err(),
        );
        assert_eq!(
            m,
            "/orgs/5/sec_policy/draft/services/31 has no pending changes"
        );
    }

    #[tokio::test]
    async fn bad_hrefs_are_refused_before_any_request() {
        let server = MockServer::start().await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let w = pce.writer(&guard);
        for bad in [
            "/orgs/5/labels/1",
            "/orgs/5/rule_sets/100",
            "/orgs/5/sec_policy/draft/rule_sets/../ip_lists/21",
            "/orgs/5/sec_policy/draft/workloads/1",
            "/orgs/7/sec_policy/draft/ip_lists/21",
        ] {
            invalid(w.plan_provision(&hrefs(&[bad])).await.unwrap_err());
        }
        assert_eq!(
            invalid(w.plan_provision(&hrefs(&[])).await.unwrap_err()),
            "nothing to provision: name at least one href, or provision all"
        );
        assert_eq!(request_count(&server).await, 0);
    }

    #[tokio::test]
    async fn nothing_pending_is_a_noop_and_sends_nothing() {
        for empty in [json!([]), json!({"rule_sets": [], "ip_lists": []})] {
            let server = MockServer::start().await;
            mount_pending(&server, empty).await;
            let pce = pce_for(&server);
            let audit = MemoryAudit::new();
            let guard = guard_with(audit.clone());
            let w = pce.writer(&guard);
            let plan = w.plan_provision(&all()).await.unwrap();
            let Plan::Noop {
                existing_href,
                reason,
            } = &plan
            else {
                panic!("expected Noop")
            };
            assert_eq!(existing_href, "/orgs/5/sec_policy/pending");
            assert_eq!(reason, "nothing pending");
            assert!(matches!(
                w.execute(plan).await.unwrap(),
                Outcome::Skipped { .. }
            ));
            assert!(audit.entries().is_empty());
            assert_eq!(request_count(&server).await, 1);
        }
    }

    #[tokio::test]
    async fn every_pending_type_is_grouped_by_its_object_type() {
        let server = MockServer::start().await;
        mount_pending(
            &server,
            json!([
                {"href": "/orgs/5/sec_policy/draft/virtual_services/7", "change_type": "update"},
                {"href": "/orgs/5/sec_policy/draft/ip_lists/21", "change_type": "create"}
            ]),
        )
        .await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let w = pce.writer(&guard);
        let plan = w.plan_provision(&all()).await.unwrap();
        let body = plan.request().unwrap().body.clone().unwrap();
        assert_eq!(
            body["change_subset"],
            json!({
                "ip_lists": [{"href": "/orgs/5/sec_policy/draft/ip_lists/21"}],
                "virtual_services": [{"href": "/orgs/5/sec_policy/draft/virtual_services/7"}]
            }),
            "--all means all: every pending type is listed under its own key"
        );
        let plan = w
            .plan_provision(&hrefs(&["/orgs/5/sec_policy/draft/ip_lists/21"]))
            .await
            .unwrap();
        let body = plan.request().unwrap().body.clone().unwrap();
        assert_eq!(
            body["change_subset"],
            json!({"ip_lists": [{"href": "/orgs/5/sec_policy/draft/ip_lists/21"}]}),
            "explicit hrefs ignore unrelated pending entries"
        );
        let m = invalid(
            w.plan_provision(&hrefs(&["/orgs/5/sec_policy/draft/rule_sets/..//1"]))
                .await
                .unwrap_err(),
        );
        assert!(m.contains("rule_sets"), "{m}");
    }

    #[tokio::test]
    async fn execute_provision_posts_once_and_invalidates_policy_and_objects() {
        let server = MockServer::start().await;
        mount_pending(
            &server,
            serde_json::from_str(include_str!("../../../fixtures/pending_dict.json")).unwrap(),
        )
        .await;
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets"))
            .respond_with(ResponseTemplate::new(200).set_body_json(rule_sets()))
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/sec_policy/draft/ip_lists"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/v2/orgs/5/sec_policy"))
            .and(body_json(fixture_body()))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({
                "href": "/orgs/5/sec_policy/12", "version": 12,
                "commit_message": "illumio-tui provision"
            })))
            .expect(1)
            .mount(&server)
            .await;
        let pce = pce_for(&server);
        pce.rule_sets(PolicyScope::Draft, CacheMode::Use)
            .await
            .unwrap();
        pce.ip_lists(CacheMode::Use).await.unwrap();
        let audit = MemoryAudit::new();
        let guard = guard_with(audit.clone());
        let w = pce.writer(&guard);
        let plan = w.plan_provision(&all()).await.unwrap();
        let Outcome::Written(out) = w.execute(plan).await.unwrap() else {
            panic!("expected Written")
        };
        assert_eq!(out.href, "/orgs/5/sec_policy/12");
        assert!(!out.pending, "a provision is not pending");
        assert_eq!(out.status, 201);
        assert_eq!(out.object["version"], 12);
        assert!(out.dropped_fields.is_empty(), "{:?}", out.dropped_fields);
        let entries = audit.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].method, WriteMethod::Post);
        assert_eq!(entries[0].path, "/orgs/5/sec_policy");
        assert_eq!(entries[0].body, Some(fixture_body()));
        assert_eq!(entries[0].href.as_deref(), Some("/orgs/5/sec_policy/12"));
        assert!(
            !pce.rule_sets(PolicyScope::Draft, CacheMode::Use)
                .await
                .unwrap()
                .hit,
            "policy: invalidated"
        );
        assert!(
            !pce.ip_lists(CacheMode::Use).await.unwrap().hit,
            "objects: invalidated"
        );
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p pce --test policy_writes provision`
Expected: compile error: `unresolved imports pce::write::ProvisionRequest, pce::write::ProvisionSubset, pce::write::provision_counts`.

- [ ] **Step 3: Implement `crates/pce/src/write/provision.rs`**

```rust
//! Provisioning: `POST /orgs/N/sec_policy` with an explicit
//! `change_subset`, built from one fresh read of the pending list. A rule
//! is provisioned through its ruleset, so rule hrefs never appear.
use super::{
    ObjectKind, Plan, WriteError, WriteRequest, Writer, checked_nested_href, checked_policy_href,
    parent_href,
};
use crate::client::WriteMethod;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProvisionSubset {
    /// Every pending change, listed explicitly.
    All,
    /// These hrefs; each must be pending. A rule href means its ruleset.
    Hrefs(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvisionRequest {
    pub description: Option<String>,
    pub subset: ProvisionSubset,
}

/// The object types this version provisions (spec 3.7).
const RULE_CHILDREN: [&str; 2] = ["sec_rules", "deny_rules"];

/// Object type → number of hrefs in the body's `change_subset`.
pub fn provision_counts(body: &Value) -> BTreeMap<String, usize> {
    body.get("change_subset")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| v.as_array().map(|a| (k.clone(), a.len())))
                .filter(|(_, n)| *n > 0)
                .collect()
        })
        .unwrap_or_default()
}

/// `provision 1 ip_list, 3 rule_sets`.
fn summary(counts: &BTreeMap<String, usize>) -> String {
    let parts: Vec<String> = counts
        .iter()
        .map(|(k, n)| {
            if *n == 1 {
                format!("1 {}", k.strip_suffix('s').unwrap_or(k))
            } else {
                format!("{n} {k}")
            }
        })
        .collect();
    format!("provision {}", parts.join(", "))
}

/// `(object type, draft href)` for a pending or requested href. A rule or
/// deny rule maps to its ruleset; anything else is a draft policy object
/// grouped under its own type segment. Parsed by component, checked strictly.
fn provisionable(href: &str, org_id: u64) -> Result<(String, String), WriteError> {
    let trimmed = href.trim().trim_end_matches('/');
    let parts: Vec<&str> = trimmed.split('/').collect();
    if let [_, _, _, _, _, _, _, child, _] = parts.as_slice()
        && RULE_CHILDREN.contains(child)
    {
        let nested = checked_nested_href(trimmed, "rule_sets", child, org_id)?;
        return Ok(("rule_sets".into(), parent_href(&nested)));
    }
    let kind = parts.get(5).copied().unwrap_or_default();
    if kind.is_empty() || !kind.bytes().all(|b| b.is_ascii_lowercase() || b == b'_') {
        return Err(WriteError::invalid(format!(
            "{trimmed} is not a draft policy object href"
        )));
    }
    let checked = checked_policy_href(trimmed, kind, org_id)?;
    Ok((kind.to_string(), checked))
}

impl Writer<'_> {
    pub async fn plan_provision(&self, req: &ProvisionRequest) -> Result<Plan, WriteError> {
        let org = self.pce().client().org_id();
        // Requested hrefs are checked before any request.
        let requested: Option<Vec<(String, String)>> = match &req.subset {
            ProvisionSubset::All => None,
            ProvisionSubset::Hrefs(h) if h.is_empty() => {
                return Err(WriteError::invalid(
                    "nothing to provision: name at least one href, or provision all",
                ));
            }
            ProvisionSubset::Hrefs(h) => Some(
                h.iter()
                    .map(|x| provisionable(x, org))
                    .collect::<Result<_, _>>()?,
            ),
        };
        let pending = self.pce().pending_changes(self.bypass()).await?.value;
        let mut pending_set: BTreeSet<(String, String)> = BTreeSet::new();
        for p in &pending {
            match provisionable(&p.href, org) {
                Ok(entry) => {
                    pending_set.insert(entry);
                }
                // `All` must mean all: an entry this version cannot
                // provision refuses the whole plan.
                Err(e) if requested.is_none() => return Err(e),
                Err(_) => {}
            }
        }
        let chosen: BTreeSet<(String, String)> = match requested {
            None => pending_set,
            Some(list) => {
                if let Some((_, h)) = list.iter().find(|e| !pending_set.contains(*e)) {
                    return Err(WriteError::invalid(format!("{h} has no pending changes")));
                }
                list.into_iter().collect()
            }
        };
        if chosen.is_empty() {
            return Ok(Plan::Noop {
                existing_href: self.pce().client().org_path("sec_policy/pending"),
                reason: "nothing pending".into(),
            });
        }
        let mut subset: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        for (kind, href) in chosen {
            subset.entry(kind).or_default().push(json!({ "href": href }));
        }
        let description = req
            .description
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .unwrap_or("illumio-tui provision");
        let body = json!({
            "update_description": description,
            "change_subset": subset,
        });
        let counts = provision_counts(&body);
        Ok(Plan::Send {
            request: WriteRequest {
                method: WriteMethod::Post,
                path: self.pce().client().org_path("sec_policy"),
                body: Some(body),
                summary: summary(&counts),
            },
            kind: ObjectKind::Provision,
        })
    }
}
```

- [ ] **Step 4: Wire it up and skip `dropped_fields` for a provision**

In `crates/pce/src/write/mod.rs` add `pub mod provision;` and `pub use provision::{ProvisionRequest, ProvisionSubset, provision_counts};`. In `Writer::execute` replace the `dropped` computation (~lines 371-375) with:

```rust
        // A provision answers with the new policy version, which echoes
        // none of the request's keys: nothing was dropped.
        let dropped = if request.method == WriteMethod::Delete || kind == ObjectKind::Provision {
            Vec::new()
        } else {
            dropped_fields(request.body.as_ref(), &object)
        };
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p pce --test policy_writes`
Expected: PASS (7 new tests in `provision`).

- [ ] **Step 6: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p pce
git add crates/pce/src/write/provision.rs crates/pce/src/write/mod.rs crates/pce/tests/policy_writes.rs
git commit -m "feat(pce): provisioning plans from one pending read, rule hrefs folded into their ruleset

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 6: Managed workload labels and the label-group lookup

**Files:**
- Modify: `crates/pce/src/write/workloads.rs` (module doc ~lines 1-5; new `LabelsPatch` after `WorkloadPatch` ~line 245; new planner in `impl Writer<'_>` after `plan_update_workload` ~line 394)
- Modify: `crates/pce/src/write/mod.rs` (re-export: `pub use workloads::{LabelRemoval, LabelsPatch, WorkloadDraft, WorkloadPatch};`)
- Modify: `crates/pce/src/resolve.rs` (`NameIndex` fields ~27-36, `new` ~39-60, `with_labels` ~66-76, new method after `label_group_name` ~121; `mod tests` ~401)
- Test: `crates/pce/tests/policy_writes.rs` (append `mod managed_labels`)

**Interfaces:**
- Consumes: `WorkloadPatch::apply(&Workload, &NameIndex) -> Result<Value, Vec<String>>` (`write/workloads.rs` ~104: with only label fields set, the body holds only `labels`), `LabelRemoval` (~60), `Writer::fetch_workload` (~362), `require_unmanaged` (~257, still used by `plan_update_workload`), `checked_href`, `NameIndex::new`, `tests::index()` in `resolve.rs` (~405, built from the four fixtures).
- Produces:

```rust
// write/workloads.rs
#[derive(Default)]
pub struct LabelsPatch { pub set_labels: Option<Vec<String>>, pub add_labels: Vec<String>, pub remove_labels: Vec<LabelRemoval> }
impl LabelsPatch { pub fn is_empty(&self) -> bool; }
impl Writer<'_> {
    pub async fn plan_relabel_workload(&self, href: &str, patch: &LabelsPatch, idx: &NameIndex) -> Result<Plan, WriteError>;
}
// resolve.rs
impl NameIndex { pub fn label_group_href(&self, name: &str) -> Option<&str>; }
```

- [ ] **Step 1: Write the failing tests**

Append to `mod tests` in `crates/pce/src/resolve.rs`:

```rust
    #[test]
    fn label_group_href_is_case_insensitive_and_survives_with_labels() {
        let idx = index();
        assert_eq!(
            idx.label_group_href("all PROD envs"),
            Some("/orgs/5/sec_policy/draft/label_groups/10")
        );
        assert_eq!(idx.label_group_href("All prod env"), None);
        let relabelled = idx.with_labels(&[]);
        assert_eq!(
            relabelled.label_group_href("All prod envs"),
            Some("/orgs/5/sec_policy/draft/label_groups/10")
        );
    }
```

Append to `crates/pce/tests/policy_writes.rs`:

```rust
mod managed_labels {
    use super::*;
    use pce::NameIndex;
    use pce::models::Label;
    use pce::write::{LabelRemoval, LabelsPatch, WorkloadPatch};

    /// `web01` / `host-1.example`: `managed: true`, a VEN, labels 1, 3, 4.
    fn web01() -> Value {
        let all: Value =
            serde_json::from_str(include_str!("../../../fixtures/workloads.json")).unwrap();
        all[0].clone()
    }

    fn idx() -> NameIndex {
        let labels: Vec<Label> =
            serde_json::from_str(include_str!("../../../fixtures/labels.json")).unwrap();
        NameIndex::new(&labels, &[], &[], &[])
    }

    async fn mount_web01(server: &MockServer) {
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/workloads/aaaa-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(web01()))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn relabel_of_a_managed_workload_sends_only_labels() {
        let server = MockServer::start().await;
        mount_web01(&server).await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let w = pce.writer(&guard);
        let add = LabelsPatch {
            add_labels: vec!["/orgs/5/labels/5".into()],
            ..LabelsPatch::default()
        };
        let plan = w
            .plan_relabel_workload("/orgs/5/workloads/aaaa-1", &add, &idx())
            .await
            .unwrap();
        let req = plan.request().unwrap();
        assert_eq!(req.method, WriteMethod::Put);
        assert_eq!(req.path, "/orgs/5/workloads/aaaa-1");
        assert_eq!(
            req.body,
            Some(json!({"labels": [
                {"href": "/orgs/5/labels/1"}, {"href": "/orgs/5/labels/3"},
                {"href": "/orgs/5/labels/4"}, {"href": "/orgs/5/labels/5"}
            ]}))
        );
        assert_eq!(req.summary, "relabel workload host-1.example");
        assert!(matches!(
            plan,
            Plan::Send {
                kind: ObjectKind::Workload,
                ..
            }
        ));
        let remove = LabelsPatch {
            remove_labels: vec![LabelRemoval {
                key: "role".into(),
                value: None,
            }],
            ..LabelsPatch::default()
        };
        let plan = w
            .plan_relabel_workload("/orgs/5/workloads/aaaa-1", &remove, &idx())
            .await
            .unwrap();
        assert_eq!(
            plan.request().unwrap().body,
            Some(json!({"labels": [{"href": "/orgs/5/labels/3"}, {"href": "/orgs/5/labels/4"}]}))
        );
        assert_eq!(request_count(&server).await, 2, "one GET per plan, nothing sent");
    }

    #[tokio::test]
    async fn managed_non_label_update_is_still_refused() {
        let server = MockServer::start().await;
        mount_web01(&server).await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let patch = WorkloadPatch {
            description: Some("x".into()),
            ..WorkloadPatch::default()
        };
        let m = invalid(
            pce.writer(&guard)
                .plan_update_workload("/orgs/5/workloads/aaaa-1", &patch, &idx())
                .await
                .unwrap_err(),
        );
        assert!(m.contains("VEN-managed"), "{m}");
    }

    #[tokio::test]
    async fn empty_or_conflicting_label_patches_are_refused_before_any_request() {
        let server = MockServer::start().await;
        let pce = pce_for(&server);
        let guard = guard_with(MemoryAudit::new());
        let w = pce.writer(&guard);
        assert_eq!(
            invalid(
                w.plan_relabel_workload("/orgs/5/workloads/aaaa-1", &LabelsPatch::default(), &idx())
                    .await
                    .unwrap_err()
            ),
            "nothing to change"
        );
        let both = LabelsPatch {
            set_labels: Some(vec![]),
            add_labels: vec!["/orgs/5/labels/5".into()],
            remove_labels: vec![],
        };
        assert_eq!(
            invalid(
                w.plan_relabel_workload("/orgs/5/workloads/aaaa-1", &both, &idx())
                    .await
                    .unwrap_err()
            ),
            "set the labels or add and remove them, not both"
        );
        let add = LabelsPatch {
            add_labels: vec!["/orgs/5/labels/5".into()],
            ..LabelsPatch::default()
        };
        invalid(
            w.plan_relabel_workload("/orgs/5/labels/1", &add, &idx())
                .await
                .unwrap_err(),
        );
        assert_eq!(request_count(&server).await, 0);
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p pce`
Expected: compile errors: `no method named 'label_group_href' found for struct 'NameIndex'`, `unresolved import pce::write::LabelsPatch`.

- [ ] **Step 3: Implement `label_group_href`** (`crates/pce/src/resolve.rs`)

Add the field `label_group_by_name: HashMap<String, String>,` to `NameIndex` (lower-cased name → href). In `NameIndex::new`, inside `for g in groups`, before the existing insert:

```rust
            idx.label_group_by_name
                .insert(g.name.to_ascii_lowercase(), g.href.clone());
```

In `with_labels`, add `label_group_by_name: self.label_group_by_name.clone(),` to the struct literal. After `label_group_name` add:

```rust
    /// A label group's href by name, case-insensitively (like `ip_list_href`).
    pub fn label_group_href(&self, name: &str) -> Option<&str> {
        self.label_group_by_name
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }
```

- [ ] **Step 4: Implement `LabelsPatch` and `plan_relabel_workload`** (`crates/pce/src/write/workloads.rs`)

Replace the module doc's sentence "label changes on managed workloads are a later plan" with "only its labels can change, through `plan_relabel_workload`". Insert after `impl WorkloadPatch`:

```rust
/// A label change on any workload, VEN-managed included: the PCE lets the
/// label set change while the VEN owns everything else.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LabelsPatch {
    /// Replace the whole label set (hrefs). Conflicts with add/remove.
    pub set_labels: Option<Vec<String>>,
    pub add_labels: Vec<String>,
    pub remove_labels: Vec<LabelRemoval>,
}

impl LabelsPatch {
    pub fn is_empty(&self) -> bool {
        self.set_labels.is_none() && self.add_labels.is_empty() && self.remove_labels.is_empty()
    }

    fn as_workload_patch(&self) -> WorkloadPatch {
        WorkloadPatch {
            set_labels: self.set_labels.clone(),
            add_labels: self.add_labels.clone(),
            remove_labels: self.remove_labels.clone(),
            ..WorkloadPatch::default()
        }
    }
}
```

Insert in `impl Writer<'_>` after `plan_update_workload`:

```rust
    /// `PUT` with only `labels`. No managed check: this is the one change
    /// allowed on a VEN-managed workload. `plan_update_workload` keeps
    /// refusing managed workloads for every other change.
    pub async fn plan_relabel_workload(
        &self,
        href: &str,
        patch: &LabelsPatch,
        idx: &NameIndex,
    ) -> Result<Plan, WriteError> {
        let href = checked_href(href, "workloads", self.pce().client().org_id())?;
        if patch.is_empty() {
            return Err(WriteError::invalid("nothing to change"));
        }
        if patch.set_labels.is_some()
            && (!patch.add_labels.is_empty() || !patch.remove_labels.is_empty())
        {
            return Err(WriteError::invalid(
                "set the labels or add and remove them, not both",
            ));
        }
        let current = self.fetch_workload(&href).await?;
        let body = patch
            .as_workload_patch()
            .apply(&current, idx)
            .map_err(WriteError::Invalid)?;
        let who = current
            .hostname
            .clone()
            .or(current.name.clone())
            .unwrap_or(href.clone());
        Ok(Plan::Send {
            request: WriteRequest {
                method: WriteMethod::Put,
                path: href,
                body: Some(body),
                summary: format!("relabel workload {who}"),
            },
            kind: ObjectKind::Workload,
        })
    }
```

Change the re-export in `crates/pce/src/write/mod.rs` to `pub use workloads::{LabelRemoval, LabelsPatch, WorkloadDraft, WorkloadPatch};`.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p pce`
Expected: PASS (1 new unit test, 3 new contract tests).

- [ ] **Step 6: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p pce
git add crates/pce/src/write/workloads.rs crates/pce/src/write/mod.rs crates/pce/src/resolve.rs crates/pce/tests/policy_writes.rs
git commit -m "feat(pce): relabel VEN-managed workloads with a labels-only PUT; label group lookup by name

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 7: CLI token grammar and resolution

**Files:**
- Modify: `crates/illumio-tui/src/cli/write.rs` (imports ~lines 5-14; `similar` ~196 and `with_similar` ~208 become `pub(crate)`; `resolve_ip_list` ~291 and `resolve_service` ~306 delegate to the new `ip_list_in`/`service_in`; new code after `resolve_service`; `mod tests` ~321)

**Interfaces:**
- Consumes: `parse_kv`, `is_href`, `label_in`, `pick_one`, `not_pending_delete` (`cli/write.rs` ~177-289), `Context { pce, cache_mode, config.org_id }` (`cli/context.rs` ~14), `Pce::{labels, label_groups, ip_lists, services}` (`lib.rs` ~130-158), `Writer::find_workload` (`write/workloads.rs` ~316, public: exact hostname case-insensitive, then exact name, `max_results` 500, ambiguity refused), `ActorRef::{all, new, from_href, validate}`, `ActorKind`, `ServiceRef::{Port, Href, from_href}` (Task 2), `checked_policy_href` (Task 1), `PortSpec::parse`.
- Produces:

```rust
pub enum ActorToken {
    All,
    Label { spec: String, exclusion: bool },
    LabelGroup { name: String, exclusion: bool },
    IpList { name: String, exclusion: bool },
    Workload { name: String },
    Href { href: String, exclusion: bool },
}
impl ActorToken { pub fn parse(s: &str) -> Result<ActorToken, String>; }
pub enum ServiceToken { Href(String), Port(PortSpec), Name(String) }
impl ServiceToken { pub fn parse(s: &str) -> Result<ServiceToken, String>; }
pub struct Lists<'a> { /* guard, and each object list fetched at most once */ }
impl<'a> Lists<'a> { pub fn new(guard: &'a WriteGuard) -> Self; }
pub fn parse_actors(specs: &[String]) -> Result<Vec<ActorToken>, CliError>;
pub fn parse_services(specs: &[String]) -> Result<Vec<ServiceToken>, CliError>;
pub fn parse_scopes(specs: &[String]) -> Result<Vec<Vec<ActorToken>>, CliError>;
pub async fn resolve_actor(ctx: &Context, token: &ActorToken, lists: &mut Lists<'_>) -> Result<ActorRef, CliError>;
pub async fn resolve_actors(ctx: &Context, tokens: &[ActorToken], lists: &mut Lists<'_>) -> Result<Vec<ActorRef>, CliError>;
pub async fn resolve_scopes(ctx: &Context, scopes: &[Vec<ActorToken>], lists: &mut Lists<'_>) -> Result<Vec<Vec<ActorRef>>, CliError>;
pub async fn resolve_service_ref(ctx: &Context, token: &ServiceToken, lists: &mut Lists<'_>) -> Result<ServiceRef, CliError>;
pub async fn resolve_services(ctx: &Context, tokens: &[ServiceToken], lists: &mut Lists<'_>) -> Result<Vec<ServiceRef>, CliError>;
pub async fn resolve_label_group(ctx: &Context, name: &str, lists: &mut Lists<'_>) -> Result<String, CliError>;
pub async fn resolve_workload(ctx: &Context, guard: &WriteGuard, target: &str) -> Result<String, CliError>;
pub(crate) fn similar<'a>(needle: &str, names: impl Iterator<Item = &'a str>) -> Vec<String>;
pub(crate) fn with_similar(msg: String, similar: &[String]) -> String;
```

The end-to-end tests of resolution (one GET per kind, the "known values" and `similar:` messages) are in Task 8, the first task with a command that resolves tokens.

- [ ] **Step 1: Write the failing tests** (append to `mod tests` in `crates/illumio-tui/src/cli/write.rs`)

```rust
    #[test]
    fn actor_tokens_parse_every_form() {
        let label = |spec: &str, exclusion| ActorToken::Label {
            spec: spec.into(),
            exclusion,
        };
        assert_eq!(ActorToken::parse("app=web").unwrap(), label("app=web", false));
        assert_eq!(
            ActorToken::parse(" label:app = a=b ").unwrap(),
            label("app=a=b", false)
        );
        assert_eq!(ActorToken::parse("!env=prod").unwrap(), label("env=prod", true));
        assert_eq!(
            ActorToken::parse("group:All prod envs").unwrap(),
            ActorToken::LabelGroup {
                name: "All prod envs".into(),
                exclusion: false
            }
        );
        assert_eq!(
            ActorToken::parse("!iplist:Corp").unwrap(),
            ActorToken::IpList {
                name: "Corp".into(),
                exclusion: true
            }
        );
        assert_eq!(
            ActorToken::parse("workload:web-1.lab").unwrap(),
            ActorToken::Workload {
                name: "web-1.lab".into()
            }
        );
        assert_eq!(ActorToken::parse("ALL").unwrap(), ActorToken::All);
        assert_eq!(
            ActorToken::parse("!/orgs/5/labels/1").unwrap(),
            ActorToken::Href {
                href: "/orgs/5/labels/1".into(),
                exclusion: true
            }
        );
    }

    #[test]
    fn actor_tokens_refuse_bad_input() {
        assert_eq!(
            ActorToken::parse("!all").unwrap_err(),
            "all cannot be excluded"
        );
        assert!(
            ActorToken::parse("!workload:x")
                .unwrap_err()
                .contains("only labels, label groups and IP lists can be excluded")
        );
        assert!(ActorToken::parse("group:").unwrap_err().contains("missing name"));
        assert!(ActorToken::parse("web").unwrap_err().contains("is not an actor"));
        assert!(
            ActorToken::parse("app=")
                .unwrap_err()
                .contains("expected key=value")
        );
        assert!(ActorToken::parse("!").unwrap_err().contains("is empty"));
    }

    #[test]
    fn service_tokens_split_ports_names_and_hrefs() {
        use pce::write::PortSpec;
        assert_eq!(
            ServiceToken::parse("/orgs/5/sec_policy/draft/services/31").unwrap(),
            ServiceToken::Href("/orgs/5/sec_policy/draft/services/31".into())
        );
        assert_eq!(
            ServiceToken::parse("443/tcp").unwrap(),
            ServiceToken::Port(PortSpec::parse("443/tcp").unwrap())
        );
        assert_eq!(
            ServiceToken::parse("8000-8100/udp").unwrap(),
            ServiceToken::Port(PortSpec::parse("8000-8100/udp").unwrap())
        );
        assert_eq!(
            ServiceToken::parse("icmp").unwrap(),
            ServiceToken::Port(PortSpec::parse("icmp").unwrap())
        );
        assert_eq!(
            ServiceToken::parse("HTTPS").unwrap(),
            ServiceToken::Name("HTTPS".into())
        );
        assert_eq!(
            ServiceToken::parse("service:tcp").unwrap(),
            ServiceToken::Name("tcp".into())
        );
        assert!(
            ServiceToken::parse("443")
                .unwrap_err()
                .contains("bare number is ambiguous")
        );
        assert!(
            ServiceToken::parse("443/tpc")
                .unwrap_err()
                .contains("unknown protocol")
        );
        assert!(ServiceToken::parse("service: ").is_err());
    }

    #[test]
    fn scopes_take_labels_and_groups_only() {
        let scopes = parse_scopes(&["app=web, group:G".into(), "env=prod".into()]).unwrap();
        assert_eq!(scopes.len(), 2);
        assert_eq!(scopes[0].len(), 2);
        assert_eq!(
            parse_scopes(&["iplist:Corp".into()])
                .unwrap_err()
                .to_string(),
            "scopes take labels and label groups only"
        );
        assert!(
            parse_scopes(&[" , ".into()])
                .unwrap_err()
                .to_string()
                .contains("--scope needs at least one label or label group")
        );
        assert!(parse_actors(&["all".into(), "web".into()]).is_err());
        assert_eq!(
            parse_services(&["443/tcp".into(), "HTTPS".into()])
                .unwrap()
                .len(),
            2
        );
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p illumio-tui --lib cli::write`
Expected: compile errors: `failed to resolve: use of undeclared type 'ActorToken'`, `cannot find function 'parse_scopes' in this scope`.

- [ ] **Step 3: Implement**

Change the imports at the top of `crates/illumio-tui/src/cli/write.rs` to:

```rust
use super::context::Context;
use super::error::CliError;
use super::output::{TableRows, WriteMeta};
use clap::Args;
use pce::models::{IpList, Label, LabelGroup, Service};
use pce::write::{
    ActorKind, ActorRef, JsonlAudit, Outcome, Plan, PortSpec, ServiceRef, WriteContext,
    WriteError, WriteGuard, WriteRefused, WriteRequest, Written, checked_policy_href,
};
use serde::Serialize;
```

Make `fn similar` and `fn with_similar` `pub(crate)`. Replace `resolve_ip_list` and `resolve_service` with:

```rust
fn ip_list_in(lists: &[IpList], name: &str) -> Result<String, CliError> {
    pick_one(
        "IP list",
        name.trim(),
        lists
            .iter()
            .filter(|l| not_pending_delete(l.update_type.as_deref()))
            .map(|l| (l.name.as_str(), l.href.as_str())),
    )
}

fn service_in(services: &[Service], name: &str) -> Result<String, CliError> {
    pick_one(
        "service",
        name.trim(),
        services
            .iter()
            .filter(|s| not_pending_delete(s.update_type.as_deref()))
            .map(|s| (s.name.as_str(), s.href.as_str())),
    )
}

fn label_group_in(groups: &[LabelGroup], name: &str) -> Result<String, CliError> {
    pick_one(
        "label group",
        name.trim(),
        groups
            .iter()
            .filter(|g| not_pending_delete(g.update_type.as_deref()))
            .map(|g| (g.name.as_str(), g.href.as_str())),
    )
}

pub async fn resolve_ip_list(ctx: &Context, name: &str) -> Result<String, CliError> {
    if is_href(name) {
        return Ok(name.trim().to_string());
    }
    ip_list_in(&ctx.pce.ip_lists(ctx.cache_mode).await?.value, name)
}

pub async fn resolve_service(ctx: &Context, name: &str) -> Result<String, CliError> {
    if is_href(name) {
        return Ok(name.trim().to_string());
    }
    service_in(&ctx.pce.services(ctx.cache_mode).await?.value, name)
}
```

Append after `resolve_service`:

```rust
/// An href as given, or an exact hostname/name match through
/// `Writer::find_workload` (one or two reads of at most 500 rows).
pub async fn resolve_workload(
    ctx: &Context,
    guard: &WriteGuard,
    target: &str,
) -> Result<String, CliError> {
    if is_href(target) {
        return Ok(target.trim().to_string());
    }
    Ok(ctx.pce.writer(guard).find_workload(target).await?.href)
}

const ACTOR_GRAMMAR: &str = "use key=value, label:key=value, group:NAME, iplist:NAME, workload:HOST, all, an href, or ! before a label, label group or IP list";

/// One `--provider`, `--consumer` or `--scope` token, parsed without any
/// request (grammar in the README's Writes section).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActorToken {
    All,
    /// `key=value`, already split and re-joined on the first `=`.
    Label { spec: String, exclusion: bool },
    LabelGroup { name: String, exclusion: bool },
    IpList { name: String, exclusion: bool },
    Workload { name: String },
    Href { href: String, exclusion: bool },
}

impl ActorToken {
    pub fn parse(s: &str) -> Result<Self, String> {
        let t = s.trim();
        let (exclusion, rest) = match t.strip_prefix('!') {
            Some(r) => (true, r.trim()),
            None => (false, t),
        };
        if rest.is_empty() {
            return Err(format!("{s:?} is empty; {ACTOR_GRAMMAR}"));
        }
        let named = |prefix: &str| -> Option<Result<String, String>> {
            rest.strip_prefix(prefix).map(|n| {
                let n = n.trim();
                if n.is_empty() {
                    Err(format!("{s:?}: missing name after {prefix}"))
                } else {
                    Ok(n.to_string())
                }
            })
        };
        if rest.eq_ignore_ascii_case("all") {
            return if exclusion {
                Err("all cannot be excluded".into())
            } else {
                Ok(Self::All)
            };
        }
        if is_href(rest) {
            return Ok(Self::Href {
                href: rest.to_string(),
                exclusion,
            });
        }
        if let Some(name) = named("group:") {
            return Ok(Self::LabelGroup {
                name: name?,
                exclusion,
            });
        }
        if let Some(name) = named("iplist:") {
            return Ok(Self::IpList {
                name: name?,
                exclusion,
            });
        }
        if let Some(name) = named("workload:") {
            if exclusion {
                return Err(format!(
                    "{s:?}: only labels, label groups and IP lists can be excluded"
                ));
            }
            return Ok(Self::Workload { name: name? });
        }
        let spec = rest.strip_prefix("label:").unwrap_or(rest);
        if spec.contains('=') {
            let (k, v) = parse_kv(spec).map_err(|e| e.to_string())?;
            return Ok(Self::Label {
                spec: format!("{k}={v}"),
                exclusion,
            });
        }
        Err(format!("{s:?} is not an actor; {ACTOR_GRAMMAR}"))
    }

    /// Scopes take labels and label groups; an href's kind is checked by
    /// the library without a request.
    fn scope_member(&self) -> bool {
        matches!(
            self,
            Self::Label { .. } | Self::LabelGroup { .. } | Self::Href { .. }
        )
    }
}

/// One `--service` token: an href, an inline port, or a service name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceToken {
    Href(String),
    Port(PortSpec),
    Name(String),
}

impl ServiceToken {
    pub fn parse(s: &str) -> Result<Self, String> {
        let t = s.trim();
        if t.is_empty() {
            return Err("empty service".into());
        }
        if is_href(t) {
            return Ok(Self::Href(t.to_string()));
        }
        if let Some(n) = t.strip_prefix("service:") {
            let n = n.trim();
            return if n.is_empty() {
                Err(format!("{s:?}: missing service name"))
            } else {
                Ok(Self::Name(n.to_string()))
            };
        }
        // A slash or a bare number can only mean a port: report the port
        // error, not "no service named".
        if t.contains('/') || t.bytes().all(|b| b.is_ascii_digit()) {
            return PortSpec::parse(t).map(Self::Port);
        }
        Ok(PortSpec::parse(t)
            .map(Self::Port)
            .unwrap_or_else(|_| Self::Name(t.to_string())))
    }
}

pub fn parse_actors(specs: &[String]) -> Result<Vec<ActorToken>, CliError> {
    specs
        .iter()
        .map(|s| ActorToken::parse(s).map_err(CliError::Args))
        .collect()
}

pub fn parse_services(specs: &[String]) -> Result<Vec<ServiceToken>, CliError> {
    specs
        .iter()
        .map(|s| ServiceToken::parse(s).map_err(CliError::Args))
        .collect()
}

/// Every `--scope` value: comma-separated labels and label groups.
pub fn parse_scopes(specs: &[String]) -> Result<Vec<Vec<ActorToken>>, CliError> {
    specs
        .iter()
        .map(|spec| {
            let tokens: Vec<String> = spec
                .split(',')
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(str::to_string)
                .collect();
            if tokens.is_empty() {
                return Err(CliError::Args(
                    "--scope needs at least one label or label group; leave --scope out for a ruleset that applies everywhere"
                        .into(),
                ));
            }
            let parsed = parse_actors(&tokens)?;
            if parsed.iter().any(|t| !t.scope_member()) {
                return Err(CliError::Args(
                    "scopes take labels and label groups only".into(),
                ));
            }
            Ok(parsed)
        })
        .collect()
}

/// The object lists one invocation resolves names against, each fetched
/// at most once and only when a token needs it. Holds the guard so
/// `workload:HOST` can use `Writer::find_workload`.
pub struct Lists<'a> {
    guard: &'a WriteGuard,
    labels: Option<Vec<Label>>,
    label_groups: Option<Vec<LabelGroup>>,
    ip_lists: Option<Vec<IpList>>,
    services: Option<Vec<Service>>,
}

impl<'a> Lists<'a> {
    pub fn new(guard: &'a WriteGuard) -> Self {
        Self {
            guard,
            labels: None,
            label_groups: None,
            ip_lists: None,
            services: None,
        }
    }

    async fn labels(&mut self, ctx: &Context) -> Result<&[Label], CliError> {
        if self.labels.is_none() {
            self.labels = Some(ctx.pce.labels(ctx.cache_mode).await?.value);
        }
        Ok(self.labels.as_deref().unwrap_or_default())
    }

    async fn label_groups(&mut self, ctx: &Context) -> Result<&[LabelGroup], CliError> {
        if self.label_groups.is_none() {
            self.label_groups = Some(ctx.pce.label_groups(ctx.cache_mode).await?.value);
        }
        Ok(self.label_groups.as_deref().unwrap_or_default())
    }

    async fn ip_lists(&mut self, ctx: &Context) -> Result<&[IpList], CliError> {
        if self.ip_lists.is_none() {
            self.ip_lists = Some(ctx.pce.ip_lists(ctx.cache_mode).await?.value);
        }
        Ok(self.ip_lists.as_deref().unwrap_or_default())
    }

    async fn services(&mut self, ctx: &Context) -> Result<&[Service], CliError> {
        if self.services.is_none() {
            self.services = Some(ctx.pce.services(ctx.cache_mode).await?.value);
        }
        Ok(self.services.as_deref().unwrap_or_default())
    }
}

/// A label group's href: an href checked without a request, or an exact
/// name from the one label-groups list.
pub async fn resolve_label_group(
    ctx: &Context,
    name: &str,
    lists: &mut Lists<'_>,
) -> Result<String, CliError> {
    if is_href(name) {
        return Ok(checked_policy_href(name, "label_groups", ctx.config.org_id)?);
    }
    label_group_in(lists.label_groups(ctx).await?, name)
}

fn with_exclusion(mut a: ActorRef, exclusion: bool) -> ActorRef {
    a.exclusion = exclusion;
    a
}

pub async fn resolve_actor(
    ctx: &Context,
    token: &ActorToken,
    lists: &mut Lists<'_>,
) -> Result<ActorRef, CliError> {
    let actor = match token {
        ActorToken::All => ActorRef::all(),
        ActorToken::Href { href, exclusion } => {
            with_exclusion(ActorRef::from_href(href, ctx.config.org_id)?, *exclusion)
        }
        ActorToken::Label { spec, exclusion } => {
            let href = label_in(lists.labels(ctx).await?, spec)?;
            with_exclusion(ActorRef::new(ActorKind::Label, href), *exclusion)
        }
        ActorToken::LabelGroup { name, exclusion } => {
            let href = resolve_label_group(ctx, name, lists).await?;
            with_exclusion(ActorRef::new(ActorKind::LabelGroup, href), *exclusion)
        }
        ActorToken::IpList { name, exclusion } => {
            let href = ip_list_in(lists.ip_lists(ctx).await?, name)?;
            with_exclusion(ActorRef::new(ActorKind::IpList, href), *exclusion)
        }
        ActorToken::Workload { name } => {
            let guard = lists.guard;
            ActorRef::new(
                ActorKind::Workload,
                resolve_workload(ctx, guard, name).await?,
            )
        }
    };
    actor.validate().map_err(CliError::Args)?;
    Ok(actor)
}

pub async fn resolve_actors(
    ctx: &Context,
    tokens: &[ActorToken],
    lists: &mut Lists<'_>,
) -> Result<Vec<ActorRef>, CliError> {
    let mut out = Vec::with_capacity(tokens.len());
    for t in tokens {
        out.push(resolve_actor(ctx, t, lists).await?);
    }
    Ok(out)
}

pub async fn resolve_scopes(
    ctx: &Context,
    scopes: &[Vec<ActorToken>],
    lists: &mut Lists<'_>,
) -> Result<Vec<Vec<ActorRef>>, CliError> {
    let mut out = Vec::with_capacity(scopes.len());
    for scope in scopes {
        out.push(resolve_actors(ctx, scope, lists).await?);
    }
    Ok(out)
}

pub async fn resolve_service_ref(
    ctx: &Context,
    token: &ServiceToken,
    lists: &mut Lists<'_>,
) -> Result<ServiceRef, CliError> {
    match token {
        ServiceToken::Href(h) => Ok(ServiceRef::from_href(h, ctx.config.org_id)?),
        ServiceToken::Port(p) => Ok(ServiceRef::Port(p.clone())),
        ServiceToken::Name(n) => Ok(ServiceRef::Href(service_in(
            lists.services(ctx).await?,
            n,
        )?)),
    }
}

pub async fn resolve_services(
    ctx: &Context,
    tokens: &[ServiceToken],
    lists: &mut Lists<'_>,
) -> Result<Vec<ServiceRef>, CliError> {
    let mut out = Vec::with_capacity(tokens.len());
    for t in tokens {
        out.push(resolve_service_ref(ctx, t, lists).await?);
    }
    Ok(out)
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p illumio-tui --lib cli::write && cargo test -p illumio-tui --test cli_writes`
Expected: PASS: 4 new unit tests, and the existing IP list and service CLI tests still pass through the delegating resolvers.

- [ ] **Step 5: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p illumio-tui
git add crates/illumio-tui/src/cli/write.rs
git commit -m "feat(cli): actor and service token grammar, resolved against lists fetched once

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 8: CLI rulesets, rules and deny rules

**Files:**
- Modify: `crates/illumio-tui/src/cli/policy.rs` (module doc and imports ~lines 1-9; `PolicyCmd` ~11-19; `RulesArgs.ruleset` ~36-37; `run` ~135-141; `rules` ~170-183; new types and handlers appended)
- Test: `crates/illumio-tui/tests/cli_writes.rs` (append `mod policy`), `crates/illumio-tui/tests/cli.rs` (`every_flag_has_help_text` list ~lines 47-64)

**Interfaces:**
- Consumes: `write::{prepare, run_plan, WriteFlags, is_href, Lists, parse_actors, parse_services, parse_scopes, resolve_actors, resolve_services, resolve_scopes, similar, with_similar}` (Plan 3 and Task 7), `Pce::rule_sets(PolicyScope::Draft, ctx.cache_mode)`, `RuleSetDraft`, `RuleSetPatch`, `RuleDraft`, `RulePatch`, `DenyRuleDraft`, `DenyRulePatch`, `ActorRef`, `ServiceRef`, `IfMissing`, `WriteError`, `checked_nested_href` (`pce::write`), `models::ResolveLabelsAs`. The existing `find_ruleset` (~163) stays for the read commands.
- Produces:

```rust
pub enum PolicyCmd { Rulesets(RulesetsCmd), Rules(RulesCmd), DenyRules(DenyRulesAction), Pending }
pub struct RulesetsCmd { pub action: Option<RulesetsAction>, pub list: RulesetsArgs }
pub enum RulesetsAction { Add(RulesetAddArgs), Update(RulesetUpdateArgs), Delete(RulesetDeleteArgs) }
pub struct RulesCmd { pub action: Option<RulesAction>, pub list: RulesArgs }   // RulesArgs.ruleset: Option<String>
pub enum RulesAction { Add(RuleAddArgs), Update(RuleUpdateArgs), Delete(RuleDeleteArgs) }
pub enum DenyRulesAction { Add(DenyRuleAddArgs), Update(DenyRuleUpdateArgs), Delete(DenyRuleDeleteArgs) }
pub struct RuleSides { pub providers: Vec<String>, pub consumers: Vec<String>, pub services: Vec<String> }
pub struct SideEdits { pub add_providers: Vec<String>, pub remove_providers: Vec<String>, pub add_consumers: Vec<String>, pub remove_consumers: Vec<String>, pub add_services: Vec<String>, pub remove_services: Vec<String> }
async fn ruleset_href(ctx: &Context, needle: &str) -> Result<String, CliError>;
fn ruleset_named(sets: &[RuleSet], name: &str) -> Result<String, CliError>;
```

- [ ] **Step 1: Write the failing tests**

In `crates/illumio-tui/tests/cli.rs`, `every_flag_has_help_text`, add after `vec!["policy", "rules", "--help"],`:

```rust
        vec!["policy", "rulesets", "add", "--help"],
        vec!["policy", "rulesets", "update", "--help"],
        vec!["policy", "rulesets", "delete", "--help"],
        vec!["policy", "rules", "add", "--help"],
        vec!["policy", "rules", "update", "--help"],
        vec!["policy", "rules", "delete", "--help"],
        vec!["policy", "deny-rules", "add", "--help"],
        vec!["policy", "deny-rules", "update", "--help"],
        vec!["policy", "deny-rules", "delete", "--help"],
```

Append to `crates/illumio-tui/tests/cli_writes.rs`:

```rust
mod policy {
    use super::*;
    use wiremock::matchers::query_param;

    pub const RS_100: &str = "/orgs/5/sec_policy/draft/rule_sets/100";
    pub const RULE_2: &str = "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/2";
    pub const DENY_8: &str = "/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8";

    pub fn fixture_labels() -> Value {
        serde_json::from_str(include_str!("../../../fixtures/labels.json")).unwrap()
    }
    pub fn fixture_label_groups() -> Value {
        serde_json::from_str(include_str!("../../../fixtures/label_groups.json")).unwrap()
    }
    pub fn fixture_ip_lists() -> Value {
        serde_json::from_str(include_str!("../../../fixtures/ip_lists.json")).unwrap()
    }
    pub fn fixture_services() -> Value {
        serde_json::from_str(include_str!("../../../fixtures/services.json")).unwrap()
    }
    pub fn fixture_rule_sets() -> Value {
        serde_json::from_str(include_str!("../../../fixtures/rule_sets.json")).unwrap()
    }
    pub fn fixture_workloads() -> Value {
        serde_json::from_str(include_str!("../../../fixtures/workloads.json")).unwrap()
    }

    pub async fn mount_get(server: &MockServer, p: &str, body: Value) {
        Mock::given(method("GET"))
            .and(path(format!("/api/v2{p}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(server)
            .await;
    }

    /// Labels, label groups, IP lists, services and the draft rulesets.
    pub async fn mount_lists(server: &MockServer) {
        mount_get(server, "/orgs/5/labels", fixture_labels()).await;
        mount_get(server, "/orgs/5/sec_policy/draft/label_groups", fixture_label_groups()).await;
        mount_get(server, "/orgs/5/sec_policy/draft/ip_lists", fixture_ip_lists()).await;
        mount_get(server, "/orgs/5/sec_policy/draft/services", fixture_services()).await;
        mount_get(server, "/orgs/5/sec_policy/draft/rule_sets", fixture_rule_sets()).await;
    }

    async fn gets(server: &MockServer, p: &str) -> usize {
        count(server, "GET", &format!("/api/v2{p}")).await
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rulesets_add_resolves_scopes_with_one_get_per_kind() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_lists(&server).await;
        let out = stdout_json(
            writer(&server, state.path())
                .args([
                    "policy",
                    "rulesets",
                    "add",
                    "Web-Tier",
                    "--scope",
                    "app=shop,env=prod",
                    "--scope",
                    "group:All prod envs",
                    "--dry-run",
                ])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["method"], "POST");
        assert_eq!(out["data"]["path"], "/orgs/5/sec_policy/draft/rule_sets");
        assert_eq!(
            out["data"]["body"],
            json!({
                "name": "Web-Tier", "enabled": true,
                "scopes": [
                    [{"label": {"href": "/orgs/5/labels/3"}}, {"label": {"href": "/orgs/5/labels/4"}}],
                    [{"label_group": {"href": "/orgs/5/sec_policy/draft/label_groups/10"}}]
                ]
            })
        );
        assert_eq!(gets(&server, "/orgs/5/labels").await, 1);
        assert_eq!(gets(&server, "/orgs/5/sec_policy/draft/label_groups").await, 1);
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
        assert_eq!(out["meta"]["api_calls"], 2);
        assert!(audit_lines(state.path()).is_empty(), "a dry run is not audited");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rulesets_add_unscoped_disabled_and_if_missing() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_lists(&server).await;
        let out = stdout_json(
            writer(&server, state.path())
                .args(["policy", "rulesets", "add", "Global", "--disabled", "--dry-run"])
                .assert()
                .success(),
        );
        assert_eq!(
            out["data"]["body"],
            json!({"name": "Global", "enabled": false, "scopes": [[]]})
        );
        assert!(server.received_requests().await.unwrap().is_empty());
        let out = stdout_json(
            writer(&server, state.path())
                .args(["policy", "rulesets", "add", "shop-prod", "--if-missing", "--dry-run"])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["skipped"], true);
        assert_eq!(out["data"]["href"], RS_100);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rulesets_add_scope_with_an_ip_list_is_refused_before_any_request() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        let a = writer(&server, state.path())
            .args(["policy", "rulesets", "add", "X", "--scope", "iplist:Corp", "--dry-run"])
            .assert()
            .code(2);
        assert_eq!(
            stderr_json(&a)["error"]["message"],
            "scopes take labels and label groups only"
        );
        writer(&server, state.path())
            .args(["policy", "rulesets", "add", "X", "--scope", "--dry-run"])
            .assert()
            .code(2);
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rulesets_update_by_name_and_by_href() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_lists(&server).await;
        mount_get(&server, RS_100, fixture_rule_sets()[0].clone()).await;
        let out = stdout_json(
            writer(&server, state.path())
                .args([
                    "policy",
                    "rulesets",
                    "update",
                    "shop-prod",
                    "--clear-scopes",
                    "--disabled",
                    "--dry-run",
                ])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["method"], "PUT");
        assert_eq!(out["data"]["path"], RS_100);
        assert_eq!(out["data"]["body"], json!({"enabled": false, "scopes": [[]]}));
        let out = stdout_json(
            writer(&server, state.path())
                .args([
                    "policy",
                    "rulesets",
                    "update",
                    RS_100,
                    "--description",
                    "Ringfence v2",
                    "--enabled",
                    "--dry-run",
                ])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["body"], json!({"description": "Ringfence v2"}));
        assert_eq!(
            gets(&server, "/orgs/5/sec_policy/draft/rule_sets").await,
            1,
            "only the name needed the list"
        );
        assert_eq!(gets(&server, RS_100).await, 2);
        writer(&server, state.path())
            .args([
                "policy",
                "rulesets",
                "update",
                RS_100,
                "--scope",
                "app=shop",
                "--clear-scopes",
                "--dry-run",
            ])
            .assert()
            .code(2);
        let a = writer(&server, state.path())
            .args(["policy", "rulesets", "update", RS_100, "--dry-run"])
            .assert()
            .code(2);
        assert!(
            stderr_json(&a)["error"]["message"]
                .as_str()
                .unwrap()
                .starts_with("nothing to change")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rulesets_unknown_and_ambiguous_names() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_lists(&server).await;
        let a = writer(&server, state.path())
            .args(["policy", "rulesets", "delete", "shop", "--dry-run"])
            .assert()
            .code(2);
        assert_eq!(
            stderr_json(&a)["error"]["message"],
            r#"no ruleset named "shop"; similar: Shop-Prod"#
        );
        let dup = MockServer::start().await;
        mount_get(
            &dup,
            "/orgs/5/sec_policy/draft/rule_sets",
            json!([
                {"href": "/orgs/5/sec_policy/draft/rule_sets/200", "name": "Dup"},
                {"href": "/orgs/5/sec_policy/draft/rule_sets/201", "name": "DUP"}
            ]),
        )
        .await;
        let a = writer(&dup, state.path())
            .args(["policy", "rulesets", "delete", "dup", "--dry-run"])
            .assert()
            .code(2);
        assert_eq!(
            stderr_json(&a)["error"]["message"],
            r#""dup" names several rulesets: /orgs/5/sec_policy/draft/rule_sets/200, /orgs/5/sec_policy/draft/rule_sets/201; use the href"#
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rulesets_delete_by_name_with_table_output() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_lists(&server).await;
        let mut t = cmd(&server);
        t.env("ILLUMIO_WRITES", "1").env("XDG_STATE_HOME", state.path());
        t.args([
            "--allow-writes",
            "--table",
            "policy",
            "rulesets",
            "delete",
            "Global-Infra",
            "--dry-run",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("DELETE")
                .and(predicate::str::contains("/orgs/5/sec_policy/draft/rule_sets/101")),
        )
        .stderr(predicate::str::contains("cache none"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rules_add_resolves_the_mixed_grammar_with_one_get_per_kind() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_lists(&server).await;
        let out = stdout_json(
            writer(&server, state.path())
                .args([
                    "policy",
                    "rules",
                    "add",
                    "shop-prod",
                    "--provider",
                    "role=web",
                    "--provider",
                    "group:All prod envs",
                    "--consumer",
                    "iplist:Corp",
                    "--consumer",
                    "!iplist:Any (0.0.0.0/0 and ::/0)",
                    "--consumer",
                    "all",
                    "--service",
                    "PostgreSQL",
                    "--service",
                    "443/tcp",
                    "--service",
                    "8000-8099/tcp",
                    "--service",
                    "/orgs/5/sec_policy/active/services/32",
                    "--description",
                    "web tier",
                    "--unscoped-consumers",
                    "--resolve-providers-as",
                    "workloads",
                    "--dry-run",
                ])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["method"], "POST");
        assert_eq!(out["data"]["path"], format!("{RS_100}/sec_rules"));
        assert_eq!(
            out["data"]["body"],
            json!({
                "enabled": true, "description": "web tier",
                "providers": [
                    {"label": {"href": "/orgs/5/labels/1"}},
                    {"label_group": {"href": "/orgs/5/sec_policy/draft/label_groups/10"}}
                ],
                "consumers": [
                    {"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/21"}},
                    {"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/20"}, "exclusion": true},
                    {"actors": "ams"}
                ],
                "ingress_services": [
                    {"href": "/orgs/5/sec_policy/draft/services/31"},
                    {"port": 443, "proto": 6},
                    {"port": 8000, "to_port": 8099, "proto": 6},
                    {"href": "/orgs/5/sec_policy/draft/services/32"}
                ],
                "unscoped_consumers": true,
                "resolve_labels_as": {"providers": ["workloads"], "consumers": ["workloads"]}
            })
        );
        for p in [
            "/orgs/5/sec_policy/draft/rule_sets",
            "/orgs/5/labels",
            "/orgs/5/sec_policy/draft/label_groups",
            "/orgs/5/sec_policy/draft/ip_lists",
            "/orgs/5/sec_policy/draft/services",
        ] {
            assert_eq!(gets(&server, p).await, 1, "{p}");
        }
        assert_eq!(server.received_requests().await.unwrap().len(), 5);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rules_add_workload_actor_uses_find_workload() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        Mock::given(method("GET"))
            .and(path("/api/v2/orgs/5/workloads"))
            .and(query_param("max_results", "500"))
            .and(query_param("hostname", "HOST-1.example"))
            .respond_with(ResponseTemplate::new(200).set_body_json(fixture_workloads()))
            .expect(1)
            .mount(&server)
            .await;
        let out = stdout_json(
            writer(&server, state.path())
                .args([
                    "policy",
                    "rules",
                    "add",
                    RS_100,
                    "--provider",
                    "workload:HOST-1.example",
                    "--consumer",
                    "all",
                    "--service",
                    "icmp",
                    "--dry-run",
                ])
                .assert()
                .success(),
        );
        assert_eq!(
            out["data"]["body"],
            json!({
                "enabled": true,
                "providers": [{"workload": {"href": "/orgs/5/workloads/aaaa-1"}}],
                "consumers": [{"actors": "ams"}],
                "ingress_services": [{"proto": 1}],
                "unscoped_consumers": false
            })
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rules_add_errors_before_any_request() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        let a = writer(&server, state.path())
            .args([
                "policy", "rules", "add", "shop-prod", "--consumer", "all", "--service", "443/tcp",
                "--dry-run",
            ])
            .assert()
            .code(2);
        assert_eq!(stderr_json(&a)["error"]["kind"], "invalid");
        assert_eq!(
            stderr_json(&a)["error"]["message"],
            "a rule needs at least one provider"
        );
        for (args, msg) in [
            (["--provider", "!all"], "all cannot be excluded"),
            (["--provider", "web"], "is not an actor"),
            (["--provider", "!workload:x"], "can be excluded"),
        ] {
            let a = writer(&server, state.path())
                .args(["policy", "rules", "add", RS_100])
                .args(args)
                .args(["--consumer", "all", "--service", "443/tcp", "--dry-run"])
                .assert()
                .code(2);
            let m = stderr_json(&a)["error"]["message"].as_str().unwrap().to_string();
            assert!(m.contains(msg), "{args:?}: {m}");
        }
        let a = writer(&server, state.path())
            .args([
                "policy", "rules", "add", RS_100, "--provider", "all", "--consumer", "all",
                "--service", "443", "--dry-run",
            ])
            .assert()
            .code(2);
        assert!(
            stderr_json(&a)["error"]["message"]
                .as_str()
                .unwrap()
                .contains("bare number is ambiguous")
        );
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rules_add_unknown_names_list_known_values_and_similar() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_lists(&server).await;
        let a = writer(&server, state.path())
            .args([
                "policy", "rules", "add", RS_100, "--provider", "role=we", "--consumer", "all",
                "--service", "443/tcp", "--dry-run",
            ])
            .assert()
            .code(2);
        assert_eq!(
            stderr_json(&a)["error"]["message"],
            "unknown label role=we; known values for role: db, web; similar: web"
        );
        let a = writer(&server, state.path())
            .args([
                "policy", "rules", "add", RS_100, "--provider", "all", "--consumer", "iplist:corp",
                "--service", "443/tcp", "--dry-run",
            ])
            .assert()
            .code(2);
        assert_eq!(
            stderr_json(&a)["error"]["message"],
            r#"no IP list named "corp"; similar: Corp"#
        );
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method.as_str() == "GET")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rules_add_sends_once_and_reports_the_nested_href() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        let body = json!({
            "enabled": true,
            "providers": [{"actors": "ams"}],
            "consumers": [{"actors": "ams"}],
            "ingress_services": [{"port": 443, "proto": 6}],
            "unscoped_consumers": false
        });
        let mut created = body.clone();
        created["href"] = json!(format!("{RS_100}/sec_rules/9"));
        Mock::given(method("POST"))
            .and(path(format!("/api/v2{RS_100}/sec_rules")))
            .and(body_json(body))
            .respond_with(ResponseTemplate::new(201).set_body_json(created))
            .expect(1)
            .mount(&server)
            .await;
        let a = writer(&server, state.path())
            .args([
                "policy", "rules", "add", RS_100, "--provider", "all", "--consumer", "all",
                "--service", "443/tcp", "--yes",
            ])
            .assert()
            .success();
        assert_no_secret(&a);
        let out = stdout_json(a);
        assert_eq!(out["data"]["href"], format!("{RS_100}/sec_rules/9"));
        assert_eq!(out["data"]["pending"], true);
        assert_eq!(out["meta"]["write"]["method"], "POST");
        assert_eq!(out["meta"]["write"]["status"], 201);
        let lines = audit_lines(state.path());
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0]["path"], format!("{RS_100}/sec_rules"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rules_update_by_href_adds_and_removes() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_get(&server, RULE_2, fixture_rule_sets()[0]["rules"][1].clone()).await;
        let out = stdout_json(
            writer(&server, state.path())
                .args([
                    "policy",
                    "rules",
                    "update",
                    "/orgs/5/sec_policy/active/rule_sets/100/sec_rules/2",
                    "--add-consumer",
                    "all",
                    "--remove-service",
                    "8000-8099/tcp",
                    "--scoped-consumers",
                    "--dry-run",
                ])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["method"], "PUT");
        assert_eq!(out["data"]["path"], RULE_2);
        assert_eq!(
            out["data"]["body"],
            json!({
                "consumers": [
                    {"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/21"}},
                    {"actors": "ams"}
                ],
                "ingress_services": [{"port": 443, "proto": 6}],
                "unscoped_consumers": false
            })
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 1, "the rule GET only");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rules_update_refuses_the_wrong_href_shape_before_any_request() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        for href in [RS_100, DENY_8, "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/../2"] {
            let a = writer(&server, state.path())
                .args(["policy", "rules", "update", href, "--description", "x", "--dry-run"])
                .assert()
                .code(2);
            let e = stderr_json(&a);
            assert_eq!(e["error"]["kind"], "invalid", "{href}");
            assert!(
                e["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("expected /orgs/<org>/sec_policy/draft/rule_sets/<id>/sec_rules/<id>"),
                "{href}: {e}"
            );
        }
        let a = writer(&server, state.path())
            .args(["policy", "rules", "update", RULE_2, "--dry-run"])
            .assert()
            .code(2);
        assert!(
            stderr_json(&a)["error"]["message"]
                .as_str()
                .unwrap()
                .starts_with("nothing to change")
        );
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rules_update_removing_an_absent_consumer_is_exit_2() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_get(&server, RULE_2, fixture_rule_sets()[0]["rules"][1].clone()).await;
        let a = writer(&server, state.path())
            .args(["policy", "rules", "update", RULE_2, "--remove-consumer", "all", "--yes"])
            .assert()
            .code(2);
        assert_eq!(
            stderr_json(&a)["error"]["message"],
            "all is not among the consumers"
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
        assert!(audit_lines(state.path()).is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn deny_rules_add_update_delete_and_rule_delete() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_lists(&server).await;
        let deny: Value =
            serde_json::from_str(include_str!("../../../fixtures/deny_rules.json")).unwrap();
        mount_get(&server, DENY_8, deny[1].clone()).await;
        let out = stdout_json(
            writer(&server, state.path())
                .args([
                    "policy",
                    "deny-rules",
                    "add",
                    RS_100,
                    "--provider",
                    "all",
                    "--consumer",
                    "iplist:Corp",
                    "--service",
                    "22/tcp",
                    "--override",
                    "--dry-run",
                ])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["path"], format!("{RS_100}/deny_rules"));
        assert_eq!(
            out["data"]["body"],
            json!({
                "enabled": true,
                "providers": [{"actors": "ams"}],
                "consumers": [{"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/21"}}],
                "ingress_services": [{"port": 22, "proto": 6}],
                "override": true
            })
        );
        let out = stdout_json(
            writer(&server, state.path())
                .args(["policy", "deny-rules", "update", DENY_8, "--no-override", "--dry-run"])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["body"], json!({"override": false}));
        let out = stdout_json(
            writer(&server, state.path())
                .args(["policy", "deny-rules", "delete", DENY_8, "--dry-run"])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["method"], "DELETE");
        assert_eq!(out["data"]["path"], DENY_8);
        let out = stdout_json(
            writer(&server, state.path())
                .args(["policy", "rules", "delete", RULE_2, "--dry-run"])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["method"], "DELETE");
        assert_eq!(out["data"]["path"], RULE_2);
        writer(&server, state.path())
            .args(["policy", "deny-rules", "delete", RULE_2, "--dry-run"])
            .assert()
            .code(2);
        writer(&server, state.path())
            .args(["policy", "deny-rules", "update", DENY_8, "--override", "--no-override", "--dry-run"])
            .assert()
            .code(2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rules_listing_still_needs_a_ruleset() {
        let server = MockServer::start().await;
        let a = cmd(&server)
            .args(["--json", "policy", "rules"])
            .assert()
            .code(2);
        assert!(
            stderr_json(&a)["error"]["message"]
                .as_str()
                .unwrap()
                .contains("needs a ruleset name or href")
        );
        assert!(server.received_requests().await.unwrap().is_empty());
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p illumio-tui --test cli_writes policy && cargo test -p illumio-tui --test cli every_flag_has_help_text`
Expected: FAIL: the binary rejects `policy rulesets add` (`unexpected argument 'add'`, exit 2 where 0 is expected) and `policy deny-rules` (`unrecognized subcommand 'deny-rules'`); the help test fails on `policy rulesets add --help`.

- [ ] **Step 3: Implement the command types** (`crates/illumio-tui/src/cli/policy.rs`)

Replace the module doc and imports (~lines 1-9) with:

```rust
//! Policy: rulesets (list; add, update, delete), the rules of one ruleset
//! (list; add, update, delete allow rules), deny rules, and pending
//! provisioning changes.
use super::context::{Context, cache_label};
use super::error::CliError;
use super::output::TableRows;
use super::write::{
    self, ActorToken, Lists, ServiceToken, WriteFlags, is_href, parse_actors, parse_scopes,
    parse_services, resolve_actors, resolve_scopes, resolve_services, run_plan,
};
use clap::{Args, Subcommand};
use pce::models::{PendingChange, PolicyScope, ResolveLabelsAs, RuleSet};
use pce::write::{
    ActorRef, DenyRuleDraft, DenyRulePatch, IfMissing, RuleDraft, RulePatch, RuleSetDraft,
    RuleSetPatch, ServiceRef, WriteError, checked_nested_href,
};
use pce::{PceError, RuleSetView, RuleView};
use serde::Serialize;
```

Replace `PolicyCmd` (~lines 11-19) with:

```rust
#[derive(Subcommand, Debug)]
pub enum PolicyCmd {
    /// List rulesets with resolved scopes; add, update or delete one
    Rulesets(RulesetsCmd),
    /// List the rules of one ruleset (name or href); add, update or delete an allow rule
    Rules(RulesCmd),
    /// Add, update or delete a deny rule
    #[command(subcommand)]
    DenyRules(DenyRulesAction),
    /// List changes waiting to be provisioned
    Pending,
}

/// `policy rulesets` lists by default; `add`, `update` and `delete` write.
#[derive(Args, Debug)]
#[command(args_conflicts_with_subcommands = true)]
pub struct RulesetsCmd {
    #[command(subcommand)]
    pub action: Option<RulesetsAction>,
    #[command(flatten)]
    pub list: RulesetsArgs,
}

#[derive(Subcommand, Debug)]
pub enum RulesetsAction {
    /// Create a ruleset (needs writes enabled and --allow-writes)
    Add(RulesetAddArgs),
    /// Change a ruleset's name, description, state or scopes
    Update(RulesetUpdateArgs),
    /// Delete a ruleset and its rules
    Delete(RulesetDeleteArgs),
}

#[derive(Args, Debug)]
pub struct RulesetAddArgs {
    /// Ruleset name
    pub name: String,
    /// Description
    #[arg(long)]
    pub description: Option<String>,
    /// Create the ruleset disabled
    #[arg(long)]
    pub disabled: bool,
    /// One scope: labels or label groups, comma-separated (key=value, group:NAME, an href); repeat for alternatives; none means everywhere
    #[arg(long = "scope")]
    pub scopes: Vec<String>,
    /// Do nothing when a ruleset with this name (case-insensitive) exists
    #[arg(long)]
    pub if_missing: bool,
    #[command(flatten)]
    pub write: WriteFlags,
}

#[derive(Args, Debug)]
pub struct RulesetUpdateArgs {
    /// Ruleset name (case-insensitive) or href
    pub ruleset: String,
    /// New name
    #[arg(long)]
    pub name: Option<String>,
    /// New description
    #[arg(long)]
    pub description: Option<String>,
    /// Enable the ruleset
    #[arg(long, conflicts_with = "disabled")]
    pub enabled: bool,
    /// Disable the ruleset
    #[arg(long)]
    pub disabled: bool,
    /// Replace every scope: one --scope per scope, labels or label groups comma-separated
    #[arg(long = "scope", conflicts_with = "clear_scopes")]
    pub scopes: Vec<String>,
    /// Remove every scope (the ruleset then applies everywhere)
    #[arg(long)]
    pub clear_scopes: bool,
    #[command(flatten)]
    pub write: WriteFlags,
}

#[derive(Args, Debug)]
pub struct RulesetDeleteArgs {
    /// Ruleset name (case-insensitive) or href
    pub ruleset: String,
    #[command(flatten)]
    pub write: WriteFlags,
}

/// `policy rules <ruleset>` lists; `add`, `update` and `delete` write.
#[derive(Args, Debug)]
#[command(args_conflicts_with_subcommands = true)]
pub struct RulesCmd {
    #[command(subcommand)]
    pub action: Option<RulesAction>,
    #[command(flatten)]
    pub list: RulesArgs,
}

#[derive(Subcommand, Debug)]
pub enum RulesAction {
    /// Create an allow rule in a ruleset (needs writes enabled and --allow-writes)
    Add(RuleAddArgs),
    /// Change an allow rule, addressed by href
    Update(RuleUpdateArgs),
    /// Delete an allow rule, addressed by href
    Delete(RuleDeleteArgs),
}

#[derive(Subcommand, Debug)]
pub enum DenyRulesAction {
    /// Create a deny rule in a ruleset (needs writes enabled and --allow-writes)
    Add(DenyRuleAddArgs),
    /// Change a deny rule, addressed by href
    Update(DenyRuleUpdateArgs),
    /// Delete a deny rule, addressed by href
    Delete(DenyRuleDeleteArgs),
}

/// The three sides of a new rule.
#[derive(Args, Debug)]
pub struct RuleSides {
    /// Provider: key=value, label:key=value, group:NAME, iplist:NAME, workload:HOST, all or an href; prefix ! to exclude (repeatable)
    #[arg(long = "provider")]
    pub providers: Vec<String>,
    /// Consumer, same forms as --provider (repeatable)
    #[arg(long = "consumer")]
    pub consumers: Vec<String>,
    /// Service: a name (or service:NAME), P/PROTO, P-Q/PROTO, a protocol name, or an href (repeatable)
    #[arg(long = "service")]
    pub services: Vec<String>,
}

/// List edits shared by `rules update` and `deny-rules update`.
#[derive(Args, Debug)]
pub struct SideEdits {
    /// Provider to add, same forms as add --provider (repeatable)
    #[arg(long = "add-provider")]
    pub add_providers: Vec<String>,
    /// Provider to remove; must match an existing provider (repeatable)
    #[arg(long = "remove-provider")]
    pub remove_providers: Vec<String>,
    /// Consumer to add (repeatable)
    #[arg(long = "add-consumer")]
    pub add_consumers: Vec<String>,
    /// Consumer to remove; must match an existing consumer (repeatable)
    #[arg(long = "remove-consumer")]
    pub remove_consumers: Vec<String>,
    /// Service to add, same forms as add --service (repeatable)
    #[arg(long = "add-service")]
    pub add_services: Vec<String>,
    /// Service to remove; must match an existing service (repeatable)
    #[arg(long = "remove-service")]
    pub remove_services: Vec<String>,
}

#[derive(Args, Debug)]
pub struct RuleAddArgs {
    /// Ruleset name (case-insensitive) or href
    pub ruleset: String,
    #[command(flatten)]
    pub sides: RuleSides,
    /// Description
    #[arg(long)]
    pub description: Option<String>,
    /// Create the rule disabled
    #[arg(long)]
    pub disabled: bool,
    /// Consumers may come from outside the ruleset's scopes
    #[arg(long)]
    pub unscoped_consumers: bool,
    /// Resolve provider labels as workloads or virtual_services (repeatable; the other side defaults to workloads)
    #[arg(long = "resolve-providers-as", value_parser = ["workloads", "virtual_services"])]
    pub resolve_providers_as: Vec<String>,
    /// Resolve consumer labels as workloads or virtual_services (repeatable; the other side defaults to workloads)
    #[arg(long = "resolve-consumers-as", value_parser = ["workloads", "virtual_services"])]
    pub resolve_consumers_as: Vec<String>,
    #[command(flatten)]
    pub write: WriteFlags,
}

#[derive(Args, Debug)]
pub struct RuleUpdateArgs {
    /// Rule href: /orgs/N/sec_policy/draft/rule_sets/ID/sec_rules/ID
    pub rule: String,
    /// New description
    #[arg(long)]
    pub description: Option<String>,
    /// Enable the rule
    #[arg(long, conflicts_with = "disabled")]
    pub enabled: bool,
    /// Disable the rule
    #[arg(long)]
    pub disabled: bool,
    #[command(flatten)]
    pub edits: SideEdits,
    /// Consumers may come from outside the ruleset's scopes
    #[arg(long, conflicts_with = "scoped_consumers")]
    pub unscoped_consumers: bool,
    /// Consumers must come from inside the ruleset's scopes
    #[arg(long)]
    pub scoped_consumers: bool,
    #[command(flatten)]
    pub write: WriteFlags,
}

#[derive(Args, Debug)]
pub struct RuleDeleteArgs {
    /// Rule href: /orgs/N/sec_policy/draft/rule_sets/ID/sec_rules/ID
    pub rule: String,
    #[command(flatten)]
    pub write: WriteFlags,
}

#[derive(Args, Debug)]
pub struct DenyRuleAddArgs {
    /// Ruleset name (case-insensitive) or href
    pub ruleset: String,
    #[command(flatten)]
    pub sides: RuleSides,
    /// Description
    #[arg(long)]
    pub description: Option<String>,
    /// Create the deny rule disabled
    #[arg(long)]
    pub disabled: bool,
    /// Override deny: evaluated before the allow rules
    #[arg(long = "override")]
    pub override_: bool,
    #[command(flatten)]
    pub write: WriteFlags,
}

#[derive(Args, Debug)]
pub struct DenyRuleUpdateArgs {
    /// Deny rule href: /orgs/N/sec_policy/draft/rule_sets/ID/deny_rules/ID
    pub deny_rule: String,
    /// New description
    #[arg(long)]
    pub description: Option<String>,
    /// Enable the deny rule
    #[arg(long, conflicts_with = "disabled")]
    pub enabled: bool,
    /// Disable the deny rule
    #[arg(long)]
    pub disabled: bool,
    #[command(flatten)]
    pub edits: SideEdits,
    /// Make it an override deny
    #[arg(long = "override", conflicts_with = "no_override")]
    pub override_: bool,
    /// Make it a regular deny
    #[arg(long)]
    pub no_override: bool,
    #[command(flatten)]
    pub write: WriteFlags,
}

#[derive(Args, Debug)]
pub struct DenyRuleDeleteArgs {
    /// Deny rule href: /orgs/N/sec_policy/draft/rule_sets/ID/deny_rules/ID
    pub deny_rule: String,
    #[command(flatten)]
    pub write: WriteFlags,
}
```

In `RulesArgs` (~line 36) change the positional to:

```rust
    /// Ruleset name (exact, case-insensitive) or href
    pub ruleset: Option<String>,
```

Replace `run` (~lines 135-141) with:

```rust
pub async fn run(ctx: &Context, cmd: &PolicyCmd) -> Result<(), CliError> {
    match cmd {
        PolicyCmd::Rulesets(c) => match &c.action {
            None => rulesets(ctx, &c.list).await,
            Some(RulesetsAction::Add(a)) => ruleset_add(ctx, a).await,
            Some(RulesetsAction::Update(a)) => ruleset_update(ctx, a).await,
            Some(RulesetsAction::Delete(a)) => ruleset_delete(ctx, a).await,
        },
        PolicyCmd::Rules(c) => match &c.action {
            None => rules(ctx, &c.list).await,
            Some(RulesAction::Add(a)) => rule_add(ctx, a).await,
            Some(RulesAction::Update(a)) => rule_update(ctx, a).await,
            Some(RulesAction::Delete(a)) => rule_delete(ctx, a).await,
        },
        PolicyCmd::DenyRules(action) => match action {
            DenyRulesAction::Add(a) => deny_rule_add(ctx, a).await,
            DenyRulesAction::Update(a) => deny_rule_update(ctx, a).await,
            DenyRulesAction::Delete(a) => deny_rule_delete(ctx, a).await,
        },
        PolicyCmd::Pending => pending(ctx).await,
    }
}
```

In `rules` (~line 170), insert as the first statement and use `needle` where the body used `args.ruleset`:

```rust
    let Some(needle) = args.ruleset.as_deref() else {
        return Err(CliError::Args(
            "policy rules needs a ruleset name or href (or one of add, update, delete)".into(),
        ));
    };
```

so the lookup reads `find_ruleset(&c.value, needle)` and the error reads `format!("ruleset {needle:?} not found; known: {}", names.join(", "))`.

- [ ] **Step 4: Implement the handlers** (append to `crates/illumio-tui/src/cli/policy.rs`)

```rust
fn on_off(on: bool, off: bool) -> Option<bool> {
    if on {
        Some(true)
    } else if off {
        Some(false)
    } else {
        None
    }
}

/// Exactly one live draft ruleset named `name` (case-insensitive); a
/// pending delete does not count.
fn ruleset_named(sets: &[RuleSet], name: &str) -> Result<String, CliError> {
    let live = || {
        sets.iter()
            .filter(|rs| rs.update_type.as_deref() != Some("delete"))
    };
    let hits: Vec<&str> = live()
        .filter(|rs| rs.name.eq_ignore_ascii_case(name))
        .map(|rs| rs.href.as_str())
        .collect();
    match hits.as_slice() {
        [one] => Ok(one.to_string()),
        [] => Err(CliError::Args(write::with_similar(
            format!("no ruleset named {name:?}"),
            &write::similar(name, live().map(|rs| rs.name.as_str())),
        ))),
        many => Err(CliError::Args(format!(
            "{name:?} names several rulesets: {}; use the href",
            many.join(", ")
        ))),
    }
}

/// An href as given (the planner checks it, no request), or a name
/// resolved with one draft ruleset list GET.
async fn ruleset_href(ctx: &Context, needle: &str) -> Result<String, CliError> {
    if is_href(needle) {
        return Ok(needle.trim().to_string());
    }
    let sets = ctx
        .pce
        .rule_sets(PolicyScope::Draft, ctx.cache_mode)
        .await?
        .value;
    ruleset_named(&sets, needle.trim())
}

/// The library's own messages for a rule with an empty side, raised before
/// the ruleset name or any token costs a request.
fn require_sides(sides: &RuleSides) -> Result<(), CliError> {
    let mut errs = Vec::new();
    if sides.providers.is_empty() {
        errs.push("a rule needs at least one provider".to_string());
    }
    if sides.consumers.is_empty() {
        errs.push("a rule needs at least one consumer".to_string());
    }
    if sides.services.is_empty() {
        errs.push("a rule needs at least one service".to_string());
    }
    if errs.is_empty() {
        Ok(())
    } else {
        Err(WriteError::Invalid(errs).into())
    }
}

fn resolve_labels_as(providers: &[String], consumers: &[String]) -> Option<ResolveLabelsAs> {
    if providers.is_empty() && consumers.is_empty() {
        return None;
    }
    let side = |v: &[String]| {
        if v.is_empty() {
            vec!["workloads".to_string()]
        } else {
            v.to_vec()
        }
    };
    Some(ResolveLabelsAs {
        providers: side(providers),
        consumers: side(consumers),
    })
}

struct ParsedSides {
    providers: Vec<ActorToken>,
    consumers: Vec<ActorToken>,
    services: Vec<ServiceToken>,
}

impl RuleSides {
    fn parse(&self) -> Result<ParsedSides, CliError> {
        require_sides(self)?;
        Ok(ParsedSides {
            providers: parse_actors(&self.providers)?,
            consumers: parse_actors(&self.consumers)?,
            services: parse_services(&self.services)?,
        })
    }
}

struct ParsedEdits {
    add_providers: Vec<ActorToken>,
    remove_providers: Vec<ActorToken>,
    add_consumers: Vec<ActorToken>,
    remove_consumers: Vec<ActorToken>,
    add_services: Vec<ServiceToken>,
    remove_services: Vec<ServiceToken>,
}

struct ResolvedEdits {
    add_providers: Vec<ActorRef>,
    remove_providers: Vec<ActorRef>,
    add_consumers: Vec<ActorRef>,
    remove_consumers: Vec<ActorRef>,
    add_services: Vec<ServiceRef>,
    remove_services: Vec<ServiceRef>,
}

impl SideEdits {
    fn is_empty(&self) -> bool {
        self.add_providers.is_empty()
            && self.remove_providers.is_empty()
            && self.add_consumers.is_empty()
            && self.remove_consumers.is_empty()
            && self.add_services.is_empty()
            && self.remove_services.is_empty()
    }

    fn parse(&self) -> Result<ParsedEdits, CliError> {
        Ok(ParsedEdits {
            add_providers: parse_actors(&self.add_providers)?,
            remove_providers: parse_actors(&self.remove_providers)?,
            add_consumers: parse_actors(&self.add_consumers)?,
            remove_consumers: parse_actors(&self.remove_consumers)?,
            add_services: parse_services(&self.add_services)?,
            remove_services: parse_services(&self.remove_services)?,
        })
    }
}

impl ParsedEdits {
    async fn resolve(
        &self,
        ctx: &Context,
        lists: &mut Lists<'_>,
    ) -> Result<ResolvedEdits, CliError> {
        Ok(ResolvedEdits {
            add_providers: resolve_actors(ctx, &self.add_providers, lists).await?,
            remove_providers: resolve_actors(ctx, &self.remove_providers, lists).await?,
            add_consumers: resolve_actors(ctx, &self.add_consumers, lists).await?,
            remove_consumers: resolve_actors(ctx, &self.remove_consumers, lists).await?,
            add_services: resolve_services(ctx, &self.add_services, lists).await?,
            remove_services: resolve_services(ctx, &self.remove_services, lists).await?,
        })
    }
}

const NOTHING_FOR_RULE: &str = "nothing to change; pass --description, --enabled, --disabled, --add-provider, --remove-provider, --add-consumer, --remove-consumer, --add-service, --remove-service";

async fn ruleset_add(ctx: &Context, a: &RulesetAddArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let scopes = parse_scopes(&a.scopes)?;
    let mut lists = Lists::new(&guard);
    let draft = RuleSetDraft {
        name: a.name.clone(),
        description: a.description.clone(),
        enabled: !a.disabled,
        scopes: resolve_scopes(ctx, &scopes, &mut lists).await?,
    };
    let if_missing = if a.if_missing {
        IfMissing::SkipExisting
    } else {
        IfMissing::Create
    };
    let plan = ctx
        .pce
        .writer(&guard)
        .plan_create_rule_set(&draft, if_missing)
        .await?;
    run_plan(ctx, &a.write, &guard, plan).await
}

async fn ruleset_update(ctx: &Context, a: &RulesetUpdateArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let enabled = on_off(a.enabled, a.disabled);
    if a.name.is_none()
        && a.description.is_none()
        && enabled.is_none()
        && a.scopes.is_empty()
        && !a.clear_scopes
    {
        return Err(CliError::Args(
            "nothing to change; pass --name, --description, --enabled, --disabled, --scope or --clear-scopes".into(),
        ));
    }
    let scopes = parse_scopes(&a.scopes)?;
    let href = ruleset_href(ctx, &a.ruleset).await?;
    let mut lists = Lists::new(&guard);
    let set_scopes = if a.clear_scopes {
        Some(Vec::new())
    } else if scopes.is_empty() {
        None
    } else {
        Some(resolve_scopes(ctx, &scopes, &mut lists).await?)
    };
    let patch = RuleSetPatch {
        name: a.name.clone(),
        description: a.description.clone(),
        enabled,
        set_scopes,
    };
    let plan = ctx
        .pce
        .writer(&guard)
        .plan_update_rule_set(&href, &patch)
        .await?;
    run_plan(ctx, &a.write, &guard, plan).await
}

async fn ruleset_delete(ctx: &Context, a: &RulesetDeleteArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let href = ruleset_href(ctx, &a.ruleset).await?;
    let plan = ctx.pce.writer(&guard).plan_delete_rule_set(&href)?;
    run_plan(ctx, &a.write, &guard, plan).await
}

async fn rule_add(ctx: &Context, a: &RuleAddArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let sides = a.sides.parse()?;
    let ruleset = ruleset_href(ctx, &a.ruleset).await?;
    let mut lists = Lists::new(&guard);
    let draft = RuleDraft {
        description: a.description.clone(),
        enabled: !a.disabled,
        providers: resolve_actors(ctx, &sides.providers, &mut lists).await?,
        consumers: resolve_actors(ctx, &sides.consumers, &mut lists).await?,
        services: resolve_services(ctx, &sides.services, &mut lists).await?,
        unscoped_consumers: a.unscoped_consumers,
        resolve_labels_as: resolve_labels_as(&a.resolve_providers_as, &a.resolve_consumers_as),
    };
    let plan = ctx.pce.writer(&guard).plan_create_rule(&ruleset, &draft)?;
    run_plan(ctx, &a.write, &guard, plan).await
}

async fn rule_update(ctx: &Context, a: &RuleUpdateArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let href = checked_nested_href(&a.rule, "rule_sets", "sec_rules", ctx.config.org_id)?;
    let enabled = on_off(a.enabled, a.disabled);
    let unscoped = on_off(a.unscoped_consumers, a.scoped_consumers);
    if a.description.is_none() && enabled.is_none() && unscoped.is_none() && a.edits.is_empty() {
        return Err(CliError::Args(format!(
            "{NOTHING_FOR_RULE}, --unscoped-consumers or --scoped-consumers"
        )));
    }
    let edits = a.edits.parse()?;
    let mut lists = Lists::new(&guard);
    let e = edits.resolve(ctx, &mut lists).await?;
    let patch = RulePatch {
        description: a.description.clone(),
        enabled,
        add_providers: e.add_providers,
        remove_providers: e.remove_providers,
        add_consumers: e.add_consumers,
        remove_consumers: e.remove_consumers,
        add_services: e.add_services,
        remove_services: e.remove_services,
        unscoped_consumers: unscoped,
        resolve_labels_as: None,
    };
    let plan = ctx.pce.writer(&guard).plan_update_rule(&href, &patch).await?;
    run_plan(ctx, &a.write, &guard, plan).await
}

async fn rule_delete(ctx: &Context, a: &RuleDeleteArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let plan = ctx.pce.writer(&guard).plan_delete_rule(&a.rule)?;
    run_plan(ctx, &a.write, &guard, plan).await
}

async fn deny_rule_add(ctx: &Context, a: &DenyRuleAddArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let sides = a.sides.parse()?;
    let ruleset = ruleset_href(ctx, &a.ruleset).await?;
    let mut lists = Lists::new(&guard);
    let draft = DenyRuleDraft {
        description: a.description.clone(),
        enabled: !a.disabled,
        providers: resolve_actors(ctx, &sides.providers, &mut lists).await?,
        consumers: resolve_actors(ctx, &sides.consumers, &mut lists).await?,
        services: resolve_services(ctx, &sides.services, &mut lists).await?,
        override_: a.override_,
    };
    let plan = ctx
        .pce
        .writer(&guard)
        .plan_create_deny_rule(&ruleset, &draft)?;
    run_plan(ctx, &a.write, &guard, plan).await
}

async fn deny_rule_update(ctx: &Context, a: &DenyRuleUpdateArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let href = checked_nested_href(&a.deny_rule, "rule_sets", "deny_rules", ctx.config.org_id)?;
    let enabled = on_off(a.enabled, a.disabled);
    let override_ = on_off(a.override_, a.no_override);
    if a.description.is_none() && enabled.is_none() && override_.is_none() && a.edits.is_empty() {
        return Err(CliError::Args(format!(
            "{NOTHING_FOR_RULE}, --override or --no-override"
        )));
    }
    let edits = a.edits.parse()?;
    let mut lists = Lists::new(&guard);
    let e = edits.resolve(ctx, &mut lists).await?;
    let patch = DenyRulePatch {
        description: a.description.clone(),
        enabled,
        add_providers: e.add_providers,
        remove_providers: e.remove_providers,
        add_consumers: e.add_consumers,
        remove_consumers: e.remove_consumers,
        add_services: e.add_services,
        remove_services: e.remove_services,
        override_,
    };
    let plan = ctx
        .pce
        .writer(&guard)
        .plan_update_deny_rule(&href, &patch)
        .await?;
    run_plan(ctx, &a.write, &guard, plan).await
}

async fn deny_rule_delete(ctx: &Context, a: &DenyRuleDeleteArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let plan = ctx.pce.writer(&guard).plan_delete_deny_rule(&a.deny_rule)?;
    run_plan(ctx, &a.write, &guard, plan).await
}
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p illumio-tui --test cli_writes && cargo test -p illumio-tui --test cli`
Expected: PASS: 16 new tests in `cli_writes::policy`, the extended help test, and the existing `policy rulesets` / `policy rules shop-prod` / `policy rules ghost` read tests unchanged.

- [ ] **Step 6: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p illumio-tui
git add crates/illumio-tui/src/cli/policy.rs crates/illumio-tui/tests/cli_writes.rs crates/illumio-tui/tests/cli.rs
git commit -m "feat(cli): policy rulesets, rules and deny-rules add/update/delete

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 9: CLI provisioning, managed labels and the guard table

**Files:**
- Modify: `crates/illumio-tui/src/cli/write.rs` (`WriteOutput` and its `TableRows` ~lines 67-116; `run_plan` ~118-174; imports)
- Modify: `crates/illumio-tui/src/cli/policy.rs` (`PolicyCmd` gains `Provision`; `run`; new `ProvisionArgs`, `provision`, `provision_targets`; imports)
- Modify: `crates/illumio-tui/src/cli/workloads.rs` (imports ~line 6-10; `WorkloadsAction::Update` doc ~59; `WorkloadUpdateArgs` ~88-118; `workload_href` ~168-177 removed; `workload_update` ~179-255; `workload_delete` ~257)
- Modify: `crates/illumio-tui/src/cli/mod.rs` (`Command::Policy` doc ~line 90)
- Test: `crates/illumio-tui/tests/cli_writes.rs` (`ALL_WRITES` ~134-165; append `mod provision` and `mod managed_labels`), `crates/illumio-tui/tests/cli.rs` (help list)

**Interfaces:**
- Consumes: `ProvisionRequest`, `ProvisionSubset`, `provision_counts`, `LabelsPatch`, `Writer::plan_provision`, `Writer::plan_relabel_workload` (Tasks 5, 6), `ruleset_named` (Task 8), `write::resolve_workload` (Task 7), `Written`, `WriteRequest`, `WriteMeta`, `Context::emit_write` (`cli/context.rs` ~95).
- Produces:

```rust
// cli/write.rs
pub enum WriteOutput {
    DryRun(WriteRequest),
    Written(Written),
    Skipped { skipped: bool, href: String, reason: String },
    ProvisionDryRun { #[serde(flatten)] request: WriteRequest, counts: BTreeMap<String, usize> },
    Provisioned { #[serde(flatten)] written: Written, counts: BTreeMap<String, usize> },
}
pub async fn run_plan(ctx: &Context, flags: &WriteFlags, guard: &WriteGuard, plan: Plan) -> Result<(), CliError>;           // unchanged signature
pub async fn run_provision_plan(ctx: &Context, flags: &WriteFlags, guard: &WriteGuard, plan: Plan) -> Result<(), CliError>;
// cli/policy.rs
pub struct ProvisionArgs { pub all: bool, pub targets: Vec<String>, pub description: Option<String>, pub write: WriteFlags }
// cli/workloads.rs
WorkloadUpdateArgs { .., pub managed_labels: bool }
```

- [ ] **Step 1: Write the failing tests**

In `crates/illumio-tui/tests/cli.rs`, `every_flag_has_help_text`, add `vec!["policy", "provision", "--help"],`.

In `crates/illumio-tui/tests/cli_writes.rs` replace the `ALL_WRITES` declaration (~lines 134-165) with:

```rust
/// Every write subcommand, with arguments that would otherwise be valid.
const ALL_WRITES: [&[&str]; 23] = [
    &["labels", "add", "a=b"],
    &["labels", "update", "/orgs/5/labels/1", "--value", "x"],
    &["labels", "delete", "/orgs/5/labels/1"],
    &["iplists", "add", "X", "--range", "10.0.0.1"],
    &[
        "iplists",
        "update",
        "/orgs/5/sec_policy/draft/ip_lists/1",
        "--add-fqdn",
        "a",
    ],
    &["iplists", "delete", "/orgs/5/sec_policy/draft/ip_lists/1"],
    &["services", "add", "X", "--port", "1/tcp"],
    &[
        "services",
        "update",
        "/orgs/5/sec_policy/draft/services/1",
        "--add-port",
        "1/tcp",
    ],
    &["services", "delete", "/orgs/5/sec_policy/draft/services/1"],
    &["workloads", "add", "X"],
    &[
        "workloads",
        "update",
        "/orgs/5/workloads/u1",
        "--description",
        "d",
    ],
    &["workloads", "delete", "/orgs/5/workloads/u1"],
    &[
        "workloads",
        "update",
        "/orgs/5/workloads/m1",
        "--add-label",
        "app=x",
        "--managed-labels",
    ],
    &["policy", "rulesets", "add", "X"],
    &[
        "policy",
        "rulesets",
        "update",
        "/orgs/5/sec_policy/draft/rule_sets/1",
        "--description",
        "d",
    ],
    &["policy", "rulesets", "delete", "/orgs/5/sec_policy/draft/rule_sets/1"],
    &[
        "policy",
        "rules",
        "add",
        "/orgs/5/sec_policy/draft/rule_sets/1",
        "--provider",
        "all",
        "--consumer",
        "all",
        "--service",
        "443/tcp",
    ],
    &[
        "policy",
        "rules",
        "update",
        "/orgs/5/sec_policy/draft/rule_sets/1/sec_rules/2",
        "--description",
        "d",
    ],
    &[
        "policy",
        "rules",
        "delete",
        "/orgs/5/sec_policy/draft/rule_sets/1/sec_rules/2",
    ],
    &[
        "policy",
        "deny-rules",
        "add",
        "/orgs/5/sec_policy/draft/rule_sets/1",
        "--provider",
        "all",
        "--consumer",
        "all",
        "--service",
        "22/tcp",
    ],
    &[
        "policy",
        "deny-rules",
        "update",
        "/orgs/5/sec_policy/draft/rule_sets/1/deny_rules/3",
        "--override",
    ],
    &[
        "policy",
        "deny-rules",
        "delete",
        "/orgs/5/sec_policy/draft/rule_sets/1/deny_rules/3",
    ],
    &["policy", "provision", "--all"],
];
```

Append to `crates/illumio-tui/tests/cli_writes.rs`:

```rust
mod provision {
    use super::policy::{fixture_rule_sets, mount_get};
    use super::*;

    fn pending_dict() -> Value {
        serde_json::from_str(include_str!("../../../fixtures/pending_dict.json")).unwrap()
    }

    fn all_body() -> Value {
        json!({
            "update_description": "illumio-tui provision",
            "change_subset": {
                "ip_lists": [{"href": "/orgs/5/sec_policy/draft/ip_lists/21"}],
                "rule_sets": [{"href": "/orgs/5/sec_policy/draft/rule_sets/101"}]
            }
        })
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn all_dry_run_lists_every_pending_href_with_counts() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_get(&server, "/orgs/5/sec_policy/pending", pending_dict()).await;
        let out = stdout_json(
            writer(&server, state.path())
                .args(["policy", "provision", "--all", "--dry-run"])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["method"], "POST");
        assert_eq!(out["data"]["path"], "/orgs/5/sec_policy");
        assert_eq!(out["data"]["body"], all_body());
        assert_eq!(out["data"]["counts"], json!({"ip_lists": 1, "rule_sets": 1}));
        assert_eq!(out["data"]["summary"], "provision 1 ip_list, 1 rule_set");
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
        assert!(audit_lines(state.path()).is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn by_ruleset_name_and_href_with_a_description() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_get(&server, "/orgs/5/sec_policy/pending", pending_dict()).await;
        mount_get(&server, "/orgs/5/sec_policy/draft/rule_sets", fixture_rule_sets()).await;
        let out = stdout_json(
            writer(&server, state.path())
                .args([
                    "policy",
                    "provision",
                    "global-infra",
                    "/orgs/5/sec_policy/active/ip_lists/21",
                    "--description",
                    "change 42",
                    "--dry-run",
                ])
                .assert()
                .success(),
        );
        let mut expected = all_body();
        expected["update_description"] = json!("change 42");
        assert_eq!(out["data"]["body"], expected);
        assert_eq!(
            count(&server, "GET", "/api/v2/orgs/5/sec_policy/draft/rule_sets").await,
            1
        );
        assert_eq!(count(&server, "GET", "/api/v2/orgs/5/sec_policy/pending").await, 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_non_pending_href_is_exit_2_and_nothing_pending_is_skipped() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_get(&server, "/orgs/5/sec_policy/pending", pending_dict()).await;
        let a = writer(&server, state.path())
            .args([
                "policy",
                "provision",
                "/orgs/5/sec_policy/draft/services/31",
                "--yes",
            ])
            .assert()
            .code(2);
        assert_eq!(stderr_json(&a)["error"]["kind"], "invalid");
        assert_eq!(
            stderr_json(&a)["error"]["message"],
            "/orgs/5/sec_policy/draft/services/31 has no pending changes"
        );
        let empty = MockServer::start().await;
        mount_get(&empty, "/orgs/5/sec_policy/pending", json!([])).await;
        let out = stdout_json(
            writer(&empty, state.path())
                .args(["policy", "provision", "--all", "--yes"])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["skipped"], true);
        assert_eq!(out["data"]["reason"], "nothing pending");
        assert!(audit_lines(state.path()).is_empty());
        writer(&empty, state.path())
            .args(["policy", "provision", "--dry-run"])
            .assert()
            .code(2);
        writer(&empty, state.path())
            .args(["policy", "provision", "--all", "Global-Infra", "--dry-run"])
            .assert()
            .code(2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sends_once_and_reports_the_version_in_json_and_table() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_get(&server, "/orgs/5/sec_policy/pending", pending_dict()).await;
        Mock::given(method("POST"))
            .and(path("/api/v2/orgs/5/sec_policy"))
            .and(body_json(all_body()))
            .respond_with(
                ResponseTemplate::new(201)
                    .set_body_json(json!({"href": "/orgs/5/sec_policy/12", "version": 12})),
            )
            .expect(2)
            .mount(&server)
            .await;
        let a = writer(&server, state.path())
            .args(["policy", "provision", "--all", "--yes"])
            .assert()
            .success();
        assert_no_secret(&a);
        let out = stdout_json(a);
        assert_eq!(out["data"]["href"], "/orgs/5/sec_policy/12");
        assert_eq!(out["data"]["pending"], false);
        assert_eq!(out["data"]["counts"], json!({"ip_lists": 1, "rule_sets": 1}));
        assert_eq!(out["data"]["dropped_fields"], json!([]));
        assert_eq!(out["meta"]["write"]["method"], "POST");
        assert_eq!(out["meta"]["write"]["path"], "/orgs/5/sec_policy");
        let mut t = cmd(&server);
        t.env("ILLUMIO_WRITES", "1").env("XDG_STATE_HOME", state.path());
        t.args(["--allow-writes", "--table", "policy", "provision", "--all", "--yes"])
            .assert()
            .success()
            .stdout(
                predicate::str::contains("provisioned")
                    .and(predicate::str::contains("/orgs/5/sec_policy/12"))
                    .and(predicate::str::contains("ip_lists=1, rule_sets=1")),
            );
        let lines = audit_lines(state.path());
        assert_eq!(lines.len(), 2, "one audit line per send");
        assert_eq!(lines[0]["body"], all_body());
    }
}

mod managed_labels {
    use super::policy::{fixture_labels, fixture_workloads, mount_get};
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn managed_labels_relabels_a_managed_workload_with_labels_only() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_get(&server, "/orgs/5/labels", fixture_labels()).await;
        mount_get(&server, "/orgs/5/workloads/aaaa-1", fixture_workloads()[0].clone()).await;
        Mock::given(method("PUT"))
            .and(path("/api/v2/orgs/5/workloads/aaaa-1"))
            .and(body_json(json!({"labels": [
                {"href": "/orgs/5/labels/1"}, {"href": "/orgs/5/labels/3"},
                {"href": "/orgs/5/labels/4"}, {"href": "/orgs/5/labels/5"}
            ]})))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        let out = stdout_json(
            writer(&server, state.path())
                .args([
                    "workloads",
                    "update",
                    "/orgs/5/workloads/aaaa-1",
                    "--add-label",
                    "loc=eu",
                    "--managed-labels",
                    "--yes",
                ])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["href"], "/orgs/5/workloads/aaaa-1");
        assert_eq!(out["data"]["pending"], false);
        assert_eq!(audit_lines(state.path()).len(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn managed_labels_by_hostname_dry_run() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_get(&server, "/orgs/5/labels", fixture_labels()).await;
        mount_get(&server, "/orgs/5/workloads", fixture_workloads()).await;
        mount_get(&server, "/orgs/5/workloads/aaaa-1", fixture_workloads()[0].clone()).await;
        let out = stdout_json(
            writer(&server, state.path())
                .args([
                    "workloads",
                    "update",
                    "host-1.example",
                    "--remove-label",
                    "role",
                    "--managed-labels",
                    "--dry-run",
                ])
                .assert()
                .success(),
        );
        assert_eq!(out["data"]["method"], "PUT");
        assert_eq!(
            out["data"]["body"],
            json!({"labels": [{"href": "/orgs/5/labels/3"}, {"href": "/orgs/5/labels/4"}]})
        );
        assert_eq!(out["data"]["summary"], "relabel workload host-1.example");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn managed_labels_refuses_other_changes_before_any_request() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        for extra in [
            ["--description", "x"],
            ["--name", "y"],
            ["--hostname", "h"],
            ["--add-ip", "10.0.0.9"],
            ["--remove-ip", "10.0.0.9"],
        ] {
            let a = writer(&server, state.path())
                .args([
                    "workloads",
                    "update",
                    "/orgs/5/workloads/aaaa-1",
                    "--add-label",
                    "loc=eu",
                    "--managed-labels",
                    "--yes",
                ])
                .args(extra)
                .assert()
                .code(2);
            assert_eq!(
                stderr_json(&a)["error"]["message"],
                "--managed-labels allows label changes only",
                "{extra:?}"
            );
        }
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_managed_workload_without_the_flag_is_still_refused() {
        let server = MockServer::start().await;
        let state = tempfile::tempdir().unwrap();
        mount_get(&server, "/orgs/5/labels", fixture_labels()).await;
        mount_get(&server, "/orgs/5/workloads/aaaa-1", fixture_workloads()[0].clone()).await;
        let a = writer(&server, state.path())
            .args([
                "workloads",
                "update",
                "/orgs/5/workloads/aaaa-1",
                "--add-label",
                "loc=eu",
                "--yes",
            ])
            .assert()
            .code(2);
        assert!(
            stderr_json(&a)["error"]["message"]
                .as_str()
                .unwrap()
                .contains("VEN-managed")
        );
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method.as_str() == "GET")
        );
        assert!(audit_lines(state.path()).is_empty());
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p illumio-tui --test cli_writes`
Expected: FAIL: `every_write_command_is_guarded_and_needs_yes` (the binary rejects `--managed-labels` and `policy provision` with clap exit 2 where exit 6 is expected first), and the `provision` and `managed_labels` modules fail on `unrecognized subcommand 'provision'` / `unexpected argument '--managed-labels'`.

- [ ] **Step 3: Provision output in `crates/illumio-tui/src/cli/write.rs`**

Add `use pce::write::provision_counts;` and `use std::collections::BTreeMap;` to the imports. Replace `WriteOutput`, `compact`, its `TableRows` impl and `run_plan` (~lines 67-174) with:

```rust
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
    /// `policy provision --dry-run`: the request and its object counts.
    ProvisionDryRun {
        #[serde(flatten)]
        request: WriteRequest,
        counts: BTreeMap<String, usize>,
    },
    /// `policy provision --yes`: the new policy version and the counts.
    Provisioned {
        #[serde(flatten)]
        written: Written,
        counts: BTreeMap<String, usize>,
    },
}

fn compact(v: &serde_json::Value) -> String {
    serde_json::to_string(v).unwrap_or_default()
}

fn row(k: &str, v: String) -> Vec<String> {
    vec![k.to_string(), v]
}

fn request_rows(r: &WriteRequest) -> Vec<Vec<String>> {
    vec![
        row("method", r.method.as_str().into()),
        row("path", r.path.clone()),
        row("summary", r.summary.clone()),
        row("body", r.body.as_ref().map(compact).unwrap_or_default()),
    ]
}

fn written_rows(w: &Written) -> Vec<Vec<String>> {
    vec![
        row("href", w.href.clone()),
        row("pending", w.pending.to_string()),
        row("status", w.status.to_string()),
        row("dropped_fields", w.dropped_fields.join(", ")),
        row("warnings", w.warnings.join("; ")),
        row("object", compact(&w.object)),
    ]
}

/// `ip_lists=1, rule_sets=3`.
fn counts_text(counts: &BTreeMap<String, usize>) -> String {
    counts
        .iter()
        .map(|(k, n)| format!("{k}={n}"))
        .collect::<Vec<_>>()
        .join(", ")
}

impl TableRows for WriteOutput {
    fn headers(&self) -> Vec<&'static str> {
        vec!["field", "value"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        match self {
            WriteOutput::DryRun(r) => request_rows(r),
            WriteOutput::Written(w) => written_rows(w),
            WriteOutput::Skipped {
                skipped,
                href,
                reason,
            } => vec![
                row("skipped", skipped.to_string()),
                row("href", href.clone()),
                row("reason", reason.clone()),
            ],
            WriteOutput::ProvisionDryRun { request, counts } => {
                let mut rows = request_rows(request);
                rows.push(row("counts", counts_text(counts)));
                rows
            }
            WriteOutput::Provisioned { written, counts } => {
                let mut rows = vec![
                    row("provisioned", written.href.clone()),
                    row("counts", counts_text(counts)),
                ];
                rows.extend(written_rows(written));
                rows
            }
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
    send(ctx, flags, guard, plan, false).await
}

/// [`run_plan`] for `policy provision`: the dry run and the result carry
/// `counts` (object type to number of hrefs in `change_subset`).
pub async fn run_provision_plan(
    ctx: &Context,
    flags: &WriteFlags,
    guard: &WriteGuard,
    plan: Plan,
) -> Result<(), CliError> {
    send(ctx, flags, guard, plan, true).await
}

async fn send(
    ctx: &Context,
    flags: &WriteFlags,
    guard: &WriteGuard,
    plan: Plan,
    with_counts: bool,
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
    let counts = with_counts
        .then(|| provision_counts(request.body.as_ref().unwrap_or(&serde_json::Value::Null)));
    if flags.dry_run {
        let out = match counts {
            Some(counts) => WriteOutput::ProvisionDryRun { request, counts },
            None => WriteOutput::DryRun(request),
        };
        return ctx.emit_write(&out, None);
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
            let out = match counts {
                Some(counts) => WriteOutput::Provisioned { written: w, counts },
                None => WriteOutput::Written(w),
            };
            ctx.emit_write(&out, Some(meta))
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
```

The existing unit tests `write_output_renders_key_value_rows` and `write_output_renders_written_rows` keep passing unchanged (same rows for the three old variants).

- [ ] **Step 4: `policy provision` in `crates/illumio-tui/src/cli/policy.rs`**

Extend the `pce::write` import with `ProvisionRequest, ProvisionSubset`. Add to `PolicyCmd`, after `Pending`:

```rust
    /// Provision pending changes (needs writes enabled and --allow-writes)
    Provision(ProvisionArgs),
```

Add after `DenyRuleDeleteArgs`:

```rust
#[derive(Args, Debug)]
pub struct ProvisionArgs {
    /// Provision every pending change
    #[arg(long, conflicts_with = "targets", required_unless_present = "targets")]
    pub all: bool,
    /// Ruleset names, or hrefs of pending rulesets, rules, IP lists, services or label groups
    pub targets: Vec<String>,
    /// Description recorded with the new policy version
    #[arg(long)]
    pub description: Option<String>,
    #[command(flatten)]
    pub write: WriteFlags,
}
```

Add the arm `PolicyCmd::Provision(a) => provision(ctx, a).await,` to `run`, and append:

```rust
/// Hrefs as given (the planner checks them); names resolved as rulesets
/// with one draft list GET, made only when a name is given.
async fn provision_targets(ctx: &Context, targets: &[String]) -> Result<Vec<String>, CliError> {
    let sets = if targets.iter().all(|t| is_href(t)) {
        Vec::new()
    } else {
        ctx.pce
            .rule_sets(PolicyScope::Draft, ctx.cache_mode)
            .await?
            .value
    };
    targets
        .iter()
        .map(|t| {
            if is_href(t) {
                Ok(t.trim().to_string())
            } else {
                ruleset_named(&sets, t.trim())
            }
        })
        .collect()
}

async fn provision(ctx: &Context, a: &ProvisionArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    let subset = if a.all {
        ProvisionSubset::All
    } else {
        ProvisionSubset::Hrefs(provision_targets(ctx, &a.targets).await?)
    };
    let req = ProvisionRequest {
        description: a.description.clone(),
        subset,
    };
    let plan = ctx.pce.writer(&guard).plan_provision(&req).await?;
    write::run_provision_plan(ctx, &a.write, &guard, plan).await
}
```

In `crates/illumio-tui/src/cli/mod.rs` change the doc line of `Command::Policy` to `/// Rulesets, rules, deny rules, pending changes, provisioning`.

- [ ] **Step 5: `--managed-labels` in `crates/illumio-tui/src/cli/workloads.rs`**

Change the write import to `use super::write::{self, WriteFlags, label_in, parse_kv, resolve_labels, resolve_workload, run_plan};` (`is_href` is no longer used here) and the library import to `use pce::write::{IfMissing, InterfaceSpec, LabelRemoval, LabelsPatch, WorkloadDraft, WorkloadPatch};`. Change the doc of `WorkloadsAction::Update` to `/// Change an unmanaged workload's name, hostname, interfaces or labels; with --managed-labels, a VEN-managed workload's labels`. Add as the last field before `write` in `WorkloadUpdateArgs`:

```rust
    /// Change labels on a VEN-managed workload (label flags only; the PUT carries only labels)
    #[arg(long)]
    pub managed_labels: bool,
```

Delete `workload_href` (~lines 168-177). In `workload_delete` replace `workload_href(ctx, &guard, &a.target)` with `resolve_workload(ctx, &guard, &a.target)`. Replace `workload_update` with:

```rust
async fn workload_update(ctx: &Context, a: &WorkloadUpdateArgs) -> Result<(), CliError> {
    let guard = write::prepare(ctx, &a.write)?;
    if a.managed_labels
        && (a.name.is_some()
            || a.hostname.is_some()
            || a.description.is_some()
            || !a.add_ips.is_empty()
            || !a.remove_ips.is_empty())
    {
        return Err(CliError::Args(
            "--managed-labels allows label changes only".into(),
        ));
    }
    let remove_labels = a
        .remove_labels
        .iter()
        .map(|s| match s.split_once('=') {
            Some(_) => parse_kv(s).map(|(k, v)| LabelRemoval {
                key: k,
                value: Some(v),
            }),
            None if s.trim().is_empty() => Err(CliError::Args("--remove-label needs a key".into())),
            None => Ok(LabelRemoval {
                key: s.trim().to_string(),
                value: None,
            }),
        })
        .collect::<Result<Vec<_>, _>>()?;
    // Nothing to change: refuse before any lookup or label resolution.
    let nothing = WorkloadPatch {
        name: a.name.clone(),
        hostname: a.hostname.clone(),
        description: a.description.clone(),
        add_addresses: a.add_ips.clone(),
        remove_addresses: a.remove_ips.clone(),
        set_labels: a.set_labels.as_ref().map(|_| Vec::new()),
        add_labels: a.add_labels.clone(),
        remove_labels: remove_labels.clone(),
    }
    .is_empty();
    if nothing {
        return Err(CliError::Args(
            "nothing to change; pass --name, --hostname, --description, --add-ip, --remove-ip, --set-labels, --add-label or --remove-label".into(),
        ));
    }
    // One labels list (one GET), only when labels change: it resolves the
    // specs and supplies keys for the workload's current labels.
    let touches_labels =
        a.set_labels.is_some() || !a.add_labels.is_empty() || !a.remove_labels.is_empty();
    let labels = if touches_labels {
        ctx.pce.labels(ctx.cache_mode).await?.value
    } else {
        Vec::new()
    };
    let resolve = |specs: &[String]| -> Result<Vec<String>, CliError> {
        specs.iter().map(|s| label_in(&labels, s)).collect()
    };
    // `--set-labels ""` clears every label: clap yields `Some(vec![""])`.
    let set_labels = match &a.set_labels {
        Some(specs) => {
            let specs: Vec<String> = specs
                .iter()
                .filter(|s| !s.trim().is_empty())
                .cloned()
                .collect();
            Some(resolve(&specs)?)
        }
        None => None,
    };
    let add_labels = resolve(&a.add_labels)?;
    let href = resolve_workload(ctx, &guard, &a.target).await?;
    let idx = pce::NameIndex::new(&labels, &[], &[], &[]);
    let w = ctx.pce.writer(&guard);
    let plan = if a.managed_labels {
        let patch = LabelsPatch {
            set_labels,
            add_labels,
            remove_labels,
        };
        w.plan_relabel_workload(&href, &patch, &idx).await?
    } else {
        let patch = WorkloadPatch {
            name: a.name.clone(),
            hostname: a.hostname.clone(),
            description: a.description.clone(),
            add_addresses: a.add_ips.clone(),
            remove_addresses: a.remove_ips.clone(),
            set_labels,
            add_labels,
            remove_labels,
        };
        w.plan_update_workload(&href, &patch, &idx).await?
    };
    run_plan(ctx, &a.write, &guard, plan).await
}
```

The body above `let add_labels` is the Plan 3 function unchanged except for the new first check.

- [ ] **Step 6: Run the tests**

Run: `cargo test -p illumio-tui`
Expected: PASS: the guard table with 23 commands (each exit 6 without `--allow-writes`, exit 2 without `--yes`, no request, no audit line), 4 `provision` tests, 4 `managed_labels` tests, the help test, and every Plan 3 workload test (by hostname, delete) through `resolve_workload`.

- [ ] **Step 7: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p illumio-tui
git add crates/illumio-tui/src/cli crates/illumio-tui/tests/cli_writes.rs crates/illumio-tui/tests/cli.rs
git commit -m "feat(cli): policy provision with counts, workloads update --managed-labels, guard table for every write

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 10: Docs, tracking, opt-in live test, dry-run smoke, final gate

**Files:**
- Modify: `README.md` ("Writes" section ~lines 133-183: the last sentence "Rulesets, rules and provisioning are the next stage." is replaced, and a "Policy writes" subsection is inserted before "### Editing in the TUI")
- Modify: `docs/architecture.md` ("Writes" section ~lines 93-117)
- Modify: `docs/index.html` (section 1 list, after the CLI writes bullet ~line 94)
- Modify: `tasks/todo.md` (append "Plan 5" checklist and review)
- Modify: `crates/pce/tests/live_writes.rs` (shared setup helper; a new opt-in test)

**Interfaces:**
- Consumes: everything above; `WriteGuard::new`, `MemoryAudit`, `RuleSetDraft`, `RuleDraft`, `RulePatch`, `ActorRef`, `ServiceRef`, `PortSpec`, `ProvisionRequest`, `ProvisionSubset`, `IfMissing`, `Outcome`.
- Produces: docs; `crates/pce/tests/live_writes.rs::live_ruleset_and_rule_lifecycle` (opt-in, **not run** by this plan).

- [ ] **Step 1: Extend the opt-in live test** (`crates/pce/tests/live_writes.rs`)

This test is NOT run by this plan. `PCE_LIVE_WRITES=1` is never set, and the test never runs against demo100. Without the variable it prints a skip line and passes.

Replace the file with:

```rust
//! Opt-in, destructive on purpose and therefore doubly gated: `PCE_LIVE_WRITES=1`
//! plus the usual PCE_HOST, PCE_ORG_ID, API_KEY, API_SECRET (optionally
//! PCE_PORT, PCE_TLS_VERIFY). `live_label_create_and_delete` creates one label
//! `illumio-tui-smoke-<ts>` and deletes it again (4 API calls);
//! `live_ruleset_and_rule_lifecycle` creates a disabled, unscoped draft
//! ruleset with one rule, updates the rule, plans (never sends) a provision,
//! and deletes both again (about 10 API calls; nothing reaches active
//! policy). Run them only against a PCE you designate as non-production.
//! Never in CI.
use pce::client::{ClientConfig, PceClient};
use pce::write::{
    ActorRef, IfMissing, LabelDraft, MemoryAudit, Outcome, PortSpec, ProvisionRequest,
    ProvisionSubset, RuleDraft, RulePatch, RuleSetDraft, ServiceRef, WriteContext, WriteGuard,
};
use pce::{Credentials, Pce};
use std::sync::Arc;

/// The PCE, an in-memory audit and a guard from the environment, or `None`
/// (with a skip line) unless `PCE_LIVE_WRITES=1`.
fn live() -> Option<(Pce, Arc<MemoryAudit>, WriteGuard)> {
    if std::env::var("PCE_LIVE_WRITES").as_deref() != Ok("1") {
        eprintln!("PCE_LIVE_WRITES != 1, skipping");
        return None;
    }
    let host = std::env::var("PCE_HOST").expect("PCE_HOST");
    let org: u64 = std::env::var("PCE_ORG_ID")
        .expect("PCE_ORG_ID")
        .parse()
        .unwrap();
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
        WriteContext {
            profile: "live".into(),
            host,
            org_id: org,
            source: "test".into(),
        },
    )
    .unwrap();
    Some((pce, audit, guard))
}

#[tokio::test]
async fn live_label_create_and_delete() {
    let Some((pce, audit, guard)) = live() else {
        return;
    };
    let w = pce.writer(&guard);

    let value = format!("illumio-tui-smoke-{}", jiff::Timestamp::now().as_second());
    let draft = LabelDraft::new("illumio-tui-smoke", &value);
    let plan = w
        .plan_create_label(&draft, IfMissing::SkipExisting)
        .await
        .unwrap();
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

#[tokio::test]
async fn live_ruleset_and_rule_lifecycle() {
    let Some((pce, audit, guard)) = live() else {
        return;
    };
    let w = pce.writer(&guard);
    let name = format!("illumio-tui-smoke-{}", jiff::Timestamp::now().as_second());
    let draft = RuleSetDraft {
        name,
        description: Some("illumio-tui live smoke; safe to delete".into()),
        enabled: false,
        scopes: vec![],
    };
    let plan = w
        .plan_create_rule_set(&draft, IfMissing::SkipExisting)
        .await
        .unwrap();
    let Outcome::Written(rs) = w.execute(plan).await.unwrap() else {
        panic!("smoke ruleset already existed; clean up the PCE")
    };
    assert!(rs.pending);

    let rule = RuleDraft {
        description: None,
        enabled: true,
        providers: vec![ActorRef::all()],
        consumers: vec![ActorRef::all()],
        services: vec![ServiceRef::Port(PortSpec::parse("443/tcp").unwrap())],
        unscoped_consumers: false,
        resolve_labels_as: None,
    };
    let Outcome::Written(r) = w
        .execute(w.plan_create_rule(&rs.href, &rule).unwrap())
        .await
        .unwrap()
    else {
        panic!("rule create produced no result")
    };
    assert!(
        r.href.starts_with(&format!("{}/sec_rules/", rs.href)),
        "{}",
        r.href
    );
    let patch = RulePatch {
        description: Some("updated by the smoke".into()),
        ..RulePatch::default()
    };
    w.execute(w.plan_update_rule(&r.href, &patch).await.unwrap())
        .await
        .unwrap();

    // Provisioning is planned, never sent: the smoke must not change
    // active policy.
    let plan = w
        .plan_provision(&ProvisionRequest {
            description: None,
            subset: ProvisionSubset::Hrefs(vec![rs.href.clone()]),
        })
        .await
        .unwrap();
    let body = plan.request().unwrap().body.clone().unwrap();
    assert!(body.to_string().contains(&rs.href), "{body}");

    w.execute(w.plan_delete_rule(&r.href).unwrap())
        .await
        .unwrap();
    w.execute(w.plan_delete_rule_set(&rs.href).unwrap())
        .await
        .unwrap();
    assert_eq!(audit.entries().len(), 5, "create, create, update, delete, delete");
}
```

Run (without the variable): `cargo test -p pce --test live_writes`
Expected: PASS, both tests print `PCE_LIVE_WRITES != 1, skipping`. **Do not run it with `PCE_LIVE_WRITES=1`, and never against demo100.**

- [ ] **Step 2: README** (`README.md`)

Replace the sentence `Rulesets, rules and provisioning are the next stage.` at the end of the "Writes" section's last paragraph with `Policy writes are below.`, and insert before `### Editing in the TUI`:

````markdown
### Policy writes

Rulesets, allow rules and deny rules are written to draft (`"pending": true`)
and reach the workloads when they are provisioned. Every command takes
`--dry-run` and `--yes` like the object writes; a ruleset is named by its
name (case-insensitive, one list GET) or its href, a rule or deny rule only by
its href (`policy rules <ruleset> --json` lists them).

    illumio-tui --allow-writes policy rulesets add Web-Tier --scope app=web,env=prod --yes
    illumio-tui --allow-writes policy rulesets update Web-Tier --clear-scopes --dry-run
    illumio-tui --allow-writes policy rules add Web-Tier --provider app=web --consumer iplist:Corp --service 443/tcp --yes --json
    illumio-tui --allow-writes policy rules update /orgs/1/sec_policy/draft/rule_sets/7/sec_rules/9 --add-consumer '!iplist:Guest' --dry-run
    illumio-tui --allow-writes policy deny-rules add Web-Tier --provider all --consumer iplist:Any --service 23/tcp --override --yes
    illumio-tui --allow-writes policy provision --all --dry-run
    illumio-tui --allow-writes policy provision Web-Tier --description "web tier v2" --yes
    illumio-tui --allow-writes workloads update web-01 --add-label env=prod --managed-labels --yes

Providers, consumers and scope members (`--provider`, `--consumer`,
`--scope`, `--add-*`/`--remove-*`):

| token | meaning |
|-------|---------|
| `app=web`, `label:app=web` | a label |
| `group:NAME` | a label group |
| `iplist:NAME` | an IP list |
| `workload:HOST` | a workload, by hostname, then by name |
| `all` | every workload |
| `!TOKEN` | exclusion (labels, label groups and IP lists only; quote it in the shell) |
| `/orgs/N/...` | an href of one of these kinds, checked without a request |

Services (`--service`): a service name (`HTTPS`, or `service:NAME` when the
name looks like a port), an inline port (`443/tcp`, `8000-8100/udp`, `icmp`)
or a service href. A `--scope` holds labels and label groups only,
comma-separated; repeat `--scope` for alternatives; no `--scope` means
everywhere. Each object list is read at most once per command, and only
when a token needs it; unknown names are exit 2 with the known values or up
to five similar names, before any write.

`rules update` and `deny-rules update` re-read the rule and send only the
keys that change; a changed provider, consumer or service list is sent whole
with the untouched entries exactly as the PCE returned them, and a
`--remove-*` that matches nothing is exit 2. Virtual service and virtual
server actors are not supported. A ruleset `PUT` never carries its rules.

`policy provision --all` reads the pending list once and provisions every
pending ruleset, IP list, service and label group by explicit href; a
changed rule is provisioned through its ruleset. With names or hrefs instead
of `--all`, each must be pending. `data.counts` gives the number of objects
per type; nothing pending is `skipped` (exit 0). A pending object of another
type (virtual services, firewall settings) makes `--all` refuse: provision
it in the PCE, or name the hrefs.

`workloads update --managed-labels` changes the labels of a VEN-managed
workload with a `PUT` that carries only `labels`; any other flag with it is
exit 2 before any request. Without it, managed workloads stay refused.
````

- [ ] **Step 3: `docs/architecture.md`**

In the "Writes" section, the sentence that says `execute` "invalidates the cache prefix of the kind" followed by the parenthesis naming `objects:` or `workloads` (it spans a line break) becomes "invalidates the cache prefixes of the kind (`objects:`, `workloads`, `policy:`, or both `policy:` and `objects:` after a provision)". Then append this paragraph after the section's last paragraph (before `### TUI editing`):

```markdown
Policy writes (Plan 5) add `write/actors.rs` (`ActorRef`, `ServiceRef`: the
typed sides of a rule and the members of a scope), `write/rule_sets.rs`,
`write/rules.rs` and `write/provision.rs`. Rules and deny rules live under
their ruleset's href (`checked_nested_href`); an update reads the rule once
as JSON and applies the patch to that JSON, so untouched providers,
consumers and services go back exactly as the PCE returned them, unknown
keys included; the policy models carry an `extra` map for the same reason.
`plan_provision` reads `sec_policy/pending` once (bypassing the cache) and
sends an explicit `change_subset`; a pending rule maps to its ruleset. The
CLI parses actor and service tokens (`ActorToken`, `ServiceToken`) before
any request and resolves names against lists it fetches at most once per
command (`Lists`).
```

In "Testing" add: `` `crates/pce/tests/policy_writes.rs` pins every policy body (rulesets, rules, deny rules, provisioning, managed labels) with wiremock. ``

- [ ] **Step 4: `docs/index.html`**

After the list item that starts `<li>Writes labels, IP lists, services and unmanaged workloads from the CLI:` (~line 94) add:

```html
    <li>Writes policy from the CLI: rulesets, allow and deny rules with a compact actor grammar (<code>app=web</code>, <code>group:Prod</code>, <code>iplist:Corp</code>, <code>!</code> to exclude), provisioning of everything pending or a chosen few, and label changes on VEN-managed workloads, all with the same dry run, single send and audit log.</li>
```

No other change to the page.

- [ ] **Step 5: Final gate**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git status --short crates/illumio-tui/src/tui
git diff --stat main -- crates/illumio-tui/src/tui
```

Expected: green. `git diff --stat main -- crates/illumio-tui/src/tui` lists exactly one file, `crates/illumio-tui/src/tui/update.rs`, with the four-variant arm from Task 1. Record the test count from the summary lines for the review.

- [ ] **Step 6: Dry-run smoke against demo100 (reads only)**

Credentials stay in `../illumio-mcp-server/.env`. Pass them with `--env-file`, never copy them. `--yes` is never passed on demo100, and `PCE_LIVE_WRITES` stays unset. Every command below is a dry run, and the scratch state directory proves that nothing was audited.

```bash
cargo build -p illumio-tui
B=target/debug/illumio-tui
export XDG_STATE_HOME="$(mktemp -d)"
export ILLUMIO_WRITES=1
# 1. The first draft ruleset's name (a read; reads were approved on demo100).
NAME="$($B --env-file ../illumio-mcp-server/.env --json policy rulesets | jq -r '.data[0].name')"
echo "$NAME"
# 2. Provision dry run: expected 1 API call (the pending GET), counts per type.
$B --env-file ../illumio-mcp-server/.env --allow-writes --json policy provision --all --dry-run | jq '.data.summary, .data.counts, .meta.api_calls'
# 3. Rule dry run under that ruleset: expected 1 API call (the ruleset list GET).
$B --env-file ../illumio-mcp-server/.env --allow-writes --json policy rules add "$NAME" --provider all --consumer all --service 443/tcp --dry-run | jq '.data.method, .data.path, .meta.api_calls'
# 4. Ruleset dry run: expected 0 API calls.
$B --env-file ../illumio-mcp-server/.env --allow-writes --json policy rulesets add smoke-test --dry-run | jq '.data.method, .data.body, .meta.api_calls'
# 5. No audit line was written.
wc -l < "$XDG_STATE_HOME/illumio-tui/writes.jsonl" 2>/dev/null || echo "no audit file"
```

Expected: step 2 prints `api_calls` 1 and either a summary or exit 0 `skipped` ("nothing pending"). If it exits 2 because demo100 has a pending object of an unsupported type, record the message as a finding. Do not change code to get around it. Step 3 prints `POST` with a path ending in `/sec_rules` and `api_calls` 1. Step 4 prints `POST` and `api_calls` 0. Step 5 prints `0` or `no audit file`. If `jq` is missing, read the JSON by eye. Write the smoke results to the ledger with a quoted heredoc (zsh backticks).

- [ ] **Step 7: Tracking** (`tasks/todo.md`)

Append with a quoted heredoc (`cat >> tasks/todo.md <<'EOF'`):

```markdown

## Plan 5: policy writes (rulesets, rules, deny rules, provisioning, managed labels)
Spec: docs/superpowers/specs/2026-10-11-policy-writes-design.md · Plan: docs/superpowers/plans/2026-10-11-policy-writes.md
- [x] Task 1 Models and kinds
- [x] Task 2 Actors and services
- [x] Task 3 Rulesets
- [x] Task 4 Rules and deny rules
- [x] Task 5 Provisioning
- [x] Task 6 Managed workload labels and the label-group lookup
- [x] Task 7 CLI token grammar and resolution
- [x] Task 8 CLI rulesets, rules and deny rules
- [x] Task 9 CLI provisioning, managed labels and the guard table
- [x] Task 10 Docs, tracking, opt-in live test, dry-run smoke, final gate

## Review (Plan 5)
```

Then append the review bullets with the measured values: what shipped (one line per command group); the final gate command and the test count from Step 5; the TUI diff (one arm in `tui/update.rs`); the smoke's three `api_calls` values and the audit-file result from Step 6; that `live_writes.rs` was extended and not run; the deviations each task reported; and the parked items the final review named.

- [ ] **Step 8: Commit**

```bash
git add README.md docs/architecture.md docs/index.html tasks/todo.md crates/pce/tests/live_writes.rs
git commit -m "docs: policy writes, provisioning and managed labels; opt-in live lifecycle test; Plan 5 tracking

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```
