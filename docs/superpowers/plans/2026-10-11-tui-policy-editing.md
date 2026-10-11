# TUI Policy Editing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bring the Plan 5 policy writes into the TUI: create, edit and delete rulesets, allow rules and deny rules on the Policy screen; mark pending changes and provision them (or everything pending) from the pending overlay after the exact `change_subset` body is shown; diff a pending ruleset against its active version; turn a blocked flow on Traffic into a pre-filled draft allow rule. The guard, the confirm-before-send overlay, the single send, the audit log and the no-unsolicited-request rule of Plans 4 and 5 are unchanged.

**Architecture:** The token grammar (`ActorToken`, `ServiceToken`, scopes and the pure href checkers) moves from `cli/write.rs` into a crate-level `tokens.rs` that the CLI and the TUI share. `tui/edit.rs` gains the policy intents, renders a rule's actors and services back to tokens through the loaded `NameIndex`, resolves typed tokens through the same index with zero requests, and builds `RuleSetDraft`/`RuleSetPatch`/`RuleDraft`/`RulePatch`/`DenyRuleDraft`/`DenyRulePatch`/`ProvisionRequest` text-first (a pre-filled token that is still there keeps the PCE's entry). `update.rs` binds `n`/`N`/`e`/`x` on Policy, `Space`/`P` in the pending overlay and `w` on Traffic, all behind `writes_refused` plus the active-scope refusal. `app.rs::plan_intent` maps every new `WriteIntent` to its Plan 5 planner (a ruleset typed by name costs one cached draft ruleset read first), and `load_diff` gains the ruleset diff computed by `tui/diff.rs::diff_rule_set`. After `Msg::WriteDone` the draft rulesets reload once with `Bypass` (after a provision, the pending list too), and the write's report stays in the status line.

**Tech Stack:** Rust 2024 (rust-version 1.88), tokio, ratatui, crossterm, serde_json, insta (snapshots), wiremock 0.6 (runtime tests); the existing `pce` crate (`pce::write` policy planners and specs, `pce::NameIndex`, `pce::traffic::ResolvedFlow`).

**Spec:** `docs/superpowers/specs/2026-10-11-tui-policy-editing-design.md` (binding). Plan 4 spec `docs/superpowers/specs/2026-10-11-tui-editing-design.md` and Plan 5 spec `docs/superpowers/specs/2026-10-11-policy-writes-design.md` for the scaffolding and the planners.

**Tasks:** 8. The suggested separate "help, hints, scope refusals, in-flight" task is split up: every key's refusals (writes off, in flight, active scope, no selection) are tested in the task that binds the key; the Policy hint changes with the rules keys (Task 3) and the Traffic hint with `w` (Task 6); the help lines and the cross-screen writes-off runtime test are Task 7, once every key exists, so the help snapshots change exactly once.

## Global Constraints

- edition 2024, rust-version 1.88; no new dependencies.
- Every PCE request is user-caused (no timers, no refresh on its own).
- POST/PUT/DELETE are sent exactly once and never retried.
- The TUI never bypasses `WriteGuard` and never builds a request path or body itself (planners do). The one Plan 4 exception stays: `named` in `app.rs` rewrites a delete's `summary`, which is neither sent nor audited.
- Draft is the only policy write target.
- Token resolution in TUI forms goes through the loaded `NameIndex` with zero requests, except one draft ruleset list read when a ruleset is given by name at submit.
- `workload:` tokens are accepted only as hrefs in the TUI.
- The CLI and its envelope are untouched (`crates/illumio-tui/src/cli/**` changes only to import the moved token module).
- Existing runtime tests keep passing.
- Both themes snapshot for every new overlay (a dark text snapshot and a light styled snapshot).
- The `pce` crate does not change, except the one method named in the spec refinements below (`NameIndex::service_href`).
- `PCE_LIVE_WRITES=1` is never set. No write is sent to demo100.
- The headless smoke never presses `y` on a confirm overlay.
- The TUI's `ObjectKind` (Objects tabs, `tui/model.rs`) and `pce::write::ObjectKind` collide: the latter is always written fully qualified, never imported or aliased.
- Commit trailer on every commit, exactly (after a blank line): `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`.
- Gate before every commit: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p illumio-tui` (Task 1 also runs `cargo test -p pce`; Task 8 runs `cargo test --workspace`). Run `cargo fmt --all` first and accept its import ordering.
- Edition 2024 clippy may rewrite nested `if`/`if let` into let-chains: accept and list it as a deviation in the task report. If clippy reports `large_enum_variant` on `EditIntent`, `WriteIntent`, `Overlay`, `Cmd` or `Msg`, box the new payload (for example `EditIntent::RuleFromFlow(Box<ResolvedFlow>)`) and list it as a deviation.
- Tests: unit tests in `#[cfg(test)] mod tests`; snapshot tests in `crates/illumio-tui/tests/tui_snapshots.rs`; runtime tests in `crates/illumio-tui/tests/tui_runtime.rs` (wiremock, `MemoryAudit`).
- Snapshots: new ones are created with `INSTA_UPDATE=always cargo test -p illumio-tui --test tui_snapshots`, each new or changed `.snap` is read before committing, and `git status --short crates/illumio-tui/tests/snapshots` must list exactly the files the task names.
- Spec refinements this plan makes (reviewers: these are intended):
  - `NameIndex::service_href(name) -> Option<&str>` is added to `crates/pce/src/resolve.rs` (one method, one unit test): spec 3 asks for "a name lookup over the index's services" and the index exposes none (`service(href)` and `service_name(href)` only). It matches like the CLI's `service_in`: exact name, pending deletions excluded, `None` when ambiguous.
  - `tokens.rs` also carries the helpers the moved types need: `is_href`, `split_kv` (the pure half of the CLI's `parse_kv`), `similar`, `with_similar`, `with_exclusion`, and `parse_scope` (one scope; `parse_scope_tokens` maps it over several). The CLI keeps `parse_kv`, `parse_scopes` (with its `--scope` messages), `actor_from_href_token` and `service_from_href_token` as thin private or public wrappers, and re-exports `ActorToken`, `ServiceToken`, `is_href`, `similar`, `with_similar` so `cli/policy.rs`, `cli/objects.rs` and `cli/workloads.rs` keep their imports.
  - `EditIntent::DeleteRule { href }` and `EditIntent::DeleteDenyRule { href }` instead of `(RuleView)`: `EditIntent` derives `PartialEq` (it lives in `Form`) and `RuleView` does not; the delete needs only the href, and the key handler picks the variant from the `RuleView`'s `RuleKind`.
  - `edit::build_intent` takes `org_id: u64` as a third argument: the href checkers need the profile's org. Three existing call sites change (`update.rs` form submit, an `update.rs` test, the `build` helper and two direct calls in the `edit.rs` tests).
  - The rule form's field is labelled `Unscoped` (spec: "Unscoped consumers"): the form's label column is 14 cells wide.
  - Flow pre-fill order is labels, then `workload:<href>`, then `iplist:<name>` per IP list, then `all` (spec 6 lists IP lists before the workload href): an unlabeled workload is narrower than the IP lists of its address, and a real PCE lists `Any (0.0.0.0/0 and ::/0)` for almost every address.
  - "form.error naming similar rulesets after the one read" (spec 7): the read runs in the planner task after the form closed, so a submitted `w` form is kept in `TrafficState.rule_form` until its plan answers; a planner error reopens it with the error in `form.error` (and in the status line), fields as typed.
  - A write's report survives the reload it causes: `PolicyState.quiet_reload` makes the next `RuleSetsLoaded` keep the status line; after a provision `PolicyState.provisioned` holds the report and the pending reload's status becomes `<report>; N pending changes left`. `Msg::WriteDone` carries no body, so `PolicyState.provision_counts` captures the counts text from the plan's body when `y` is pressed.
  - After a ruleset, rule or deny-rule write the rulesets reload only when the Policy scope is draft (an active view is not changed by a draft write); after a provision they reload in whichever scope is shown.
  - `Msg::PendingLoaded` opens the pending overlay only when no other overlay is open (the post-provision reload must not replace an error overlay).
  - `Msg::Planned` with a `Plan::Noop` for `ObjectKind::Provision` is an info status (`nothing pending`), not a failure.
  - The pending overlay's key line (`j/k select · Space mark · Enter diff · P provision · Esc`) is the last row inside its box; the panel meta shows `sel/n` and `· N marked`.
  - `Enter` on a pending `sec_rules` or `deny_rules` entry says `rule changes show in their ruleset's diff`.
  - The ruleset diff's rule summaries use the form's token grammar (`iplist:Corp`, `all`, `PostgreSQL`), as spec 5's example does, rather than `RuleView` names (`ipl:Corp`, `All workloads`); ` (disabled)` is appended for a disabled rule; enabled reads `yes`/`no`.
  - `w` on a flow whose decision is `unknown` says `this flow has no policy decision; nothing to add`.
  - Form titles name rulesets and rules by href id (`New allow rule in ruleset 100`, `Edit deny rule 8 in ruleset 100`): the intents carry hrefs.
  - A pre-filled token whose name would not survive the comma split (a name containing `,` or `|`) is shown as the object's href; a service whose name parses as a port (`tcp`) is shown as `service:tcp`; a retyped token naming an entry already shown (`label:role=web` for `role=web`) is neither added nor removed.

## Review Focus

1. `e` on allow rule `sec_rules/1` (services `PostgreSQL`), `, 443/tcp` appended to Services, `Enter`, `y`. Expected: one GET of the rule at plan time, one PUT whose body is exactly `{"ingress_services": [{"href": ".../services/31"}, {"port": 443, "proto": 6}]}`, one draft rulesets GET to reload, status `updated <href> (pending)`. Test: Task 3 `edit_rule_puts_only_the_changed_services`.
2. A rule with a virtual-service provider and an `any` port: the form shows the href and `?`; editing only Description plans a description-only patch; changing Providers or Services is refused with `this rule has entries the form cannot edit; use the CLI` and no `Cmd`. Test: Task 3 `inexpressible_entries_refuse_a_changed_field_only`.
3. Pending overlay with three entries, `Space` on the first and third, `P`, `y`. Expected: the plan's one fresh pending GET, one POST whose `change_subset` is exactly those two hrefs, then one rulesets GET and one pending GET; status `provisioned /orgs/5/sec_policy/7: rule_sets=1, services=1; 3 pending changes left`. Test: Task 4 `provision_marked_posts_exactly_the_marked_hrefs`.
4. `w` on the potentially blocked fixture flow (Corp → host-1, 8443/tcp), ruleset typed as `Shop-Prod`. Expected: providers `app=shop, env=prod, role=web`, consumers `iplist:Corp`, Unscoped `y`; one draft rulesets GET to resolve the name, one POST to `.../rule_sets/100/sec_rules`, one reload; an unknown name reopens the form with `no ruleset named "Shop"; similar: Shop-Prod` and sends nothing. Tests: Task 6 `w_creates_a_rule_in_the_named_ruleset`, Task 6 `an_unknown_ruleset_name_reopens_the_w_form_with_the_error`.
5. Writes off: `n`/`e`/`x` on both Policy panes, `N` on the rules pane, `P` in the pending overlay, `w` on Traffic. Expected: the reason in the status line as a warning, no form, no write request, the request counter only moved by the rulesets and pending loads; in the active scope the Policy keys say `switch to draft to edit policy`. Tests: Task 7 `writes_disabled_policy_and_traffic_keys_send_nothing`, Task 2 `policy_keys_refuse_in_the_active_scope_and_without_a_selection`.

---

## File structure

| File | Responsibility |
| --- | --- |
| `crates/illumio-tui/src/tokens.rs` (new) | `ActorToken`, `ServiceToken`, `parse_scope`, `parse_scope_tokens`, `actor_from_href_token`, `service_from_href_token`, `is_href`, `split_kv`, `similar`, `with_similar`, `with_exclusion` |
| `crates/illumio-tui/src/lib.rs` | `pub mod tokens;` |
| `crates/illumio-tui/src/cli/write.rs` | imports the moved grammar; thin wrappers keep the CLI's error type and messages |
| `crates/pce/src/resolve.rs` | `NameIndex::service_href` |
| `crates/illumio-tui/src/tui/edit.rs` | token resolution and rendering over the index; ruleset, rule, deny-rule, provision and flow intents; forms; text-first adapters; `RulesetTarget`; `ruleset_href_named` |
| `crates/illumio-tui/src/tui/diff.rs` | `diff_rule_set`, `rule_set_diff_for` |
| `crates/illumio-tui/src/tui/model.rs` | `PolicyState.{pending_marked, quiet_reload, provision_counts, provisioned}`, `selected_rule_set`, `find_rule`, `find_deny_rule`; `TrafficState.rule_form` |
| `crates/illumio-tui/src/tui/msg.rs` | doc of `Cmd::LoadDiff` (no new variant: the new `WriteIntent`s flow through `Cmd::PlanWrite`) |
| `crates/illumio-tui/src/tui/update.rs` | Policy `n`/`N`/`e`/`x`, pending `Space`/`P`, Traffic `w`, reloads, provision report, reopened `w` form |
| `crates/illumio-tui/src/tui/app.rs` | `plan_intent` arms (ruleset name read), ruleset diff task |
| `crates/illumio-tui/src/tui/ui/widgets/overlay.rs` | pending marks and key line, ruleset diff title, help lines, hints |
| `crates/illumio-tui/src/tui/test_support.rs` | `provision_plan`, `rule_set_diff_fixture` |
| `crates/illumio-tui/tests/tui_snapshots.rs`, `tests/tui_runtime.rs` | snapshots and wiremock runtime tests |
| `README.md`, `docs/architecture.md`, `docs/index.html`, `tasks/todo.md` | docs and tracking |

---

### Task 1: Shared token module and TUI token resolution

**Files:**
- Create: `crates/illumio-tui/src/tokens.rs`
- Modify: `crates/illumio-tui/src/lib.rs` (add `pub mod tokens;` after `pub mod config;`, line 4)
- Modify: `crates/illumio-tui/src/cli/write.rs` (imports lines 5-14; `parse_kv` ~line 254, `is_href` ~268, `similar` ~272, `with_similar` ~286; `ACTOR_GRAMMAR` through the end of `parse_scopes` ~lines 433-619; `with_exclusion`, `actor_from_href_token`, `service_from_href_token` ~lines 689-747; tests `actor_tokens_parse_every_form` ~969, `actor_tokens_refuse_bad_input` ~1017, `service_tokens_split_ports_names_and_hrefs` ~1046, `prefixed_hrefs_resolve_through_the_strict_checkers_without_lists` ~1113, moved)
- Modify: `crates/pce/src/resolve.rs` (`NameIndex::service_href` after `service_name` ~line 163; test at the end of `mod tests` ~line 673)
- Modify: `crates/illumio-tui/src/tui/edit.rs` (imports lines 5-11; resolvers and renderers after `resolve_labels` ~line 440; tests)

**Interfaces:**
- Consumes: `ActorRef::{new, all, from_href, from_actor, validate, same_actor}`, `ActorKind`, `ServiceRef::{from_href, from_ingress}`, `PortSpec::{parse, display}` (`crates/pce/src/write/actors.rs`, `specs.rs`), `checked_href`, `checked_policy_href`, `WriteError` (`write/mod.rs` ~lines 150, 206, 288), `NameIndex::{label, label_href, values_for_key, label_group_href, label_group_name, ip_list_href, ip_list_name, ip_list_names, service_name}` (`resolve.rs` ~lines 96-163), `Label::{display, is_deleted}` (`models/labels.rs` ~lines 20-24), `Actor`, `IngressService` (`models/policy.rs` ~lines 33, 60), `fixture_index` (`tui/test_support.rs` ~line 13), `CliError::{Args, Write}` (`cli/error.rs` ~line 8).
- Produces:

```rust
// crates/illumio-tui/src/tokens.rs
pub enum ActorToken { All, Label { spec: String, exclusion: bool }, LabelGroup { name: String, exclusion: bool },
                      IpList { name: String, exclusion: bool }, Workload { name: String }, Href { href: String, exclusion: bool } }
impl ActorToken { pub fn parse(s: &str) -> Result<Self, String>; pub fn scope_member(&self) -> bool; }
pub enum ServiceToken { Href(String), Port(PortSpec), Name(String) }
impl ServiceToken { pub fn parse(s: &str) -> Result<Self, String>; }
pub fn is_href(s: &str) -> bool;
pub fn split_kv(spec: &str) -> Result<(String, String), String>;
pub fn similar<'a>(needle: &str, names: impl Iterator<Item = &'a str>) -> Vec<String>;
pub fn with_similar(msg: String, similar: &[String]) -> String;
pub fn with_exclusion(a: ActorRef, exclusion: bool) -> ActorRef;
pub fn parse_scope(spec: &str) -> Result<Vec<ActorToken>, String>;
pub fn parse_scope_tokens(specs: &[String]) -> Result<Vec<Vec<ActorToken>>, String>;
pub fn actor_from_href_token(token: &ActorToken, org_id: u64) -> Option<Result<ActorRef, WriteError>>;
pub fn service_from_href_token(token: &ServiceToken, org_id: u64) -> Option<Result<ServiceRef, WriteError>>;

// crates/pce/src/resolve.rs
impl NameIndex { pub fn service_href(&self, name: &str) -> Option<&str>; }

// crates/illumio-tui/src/tui/edit.rs
pub fn resolve_actor_tokens(tokens: &[ActorToken], idx: &NameIndex, org_id: u64) -> Result<Vec<ActorRef>, String>;
pub fn resolve_service_tokens(tokens: &[ServiceToken], idx: &NameIndex, org_id: u64) -> Result<Vec<ServiceRef>, String>;
pub fn actor_tokens(actors: &[Actor], idx: &NameIndex) -> Vec<String>;
pub fn service_tokens(services: &[IngressService], idx: &NameIndex) -> Vec<String>;
```

- [ ] **Step 1: Write the failing test** (create `crates/illumio-tui/src/tokens.rs` with only this, and add `pub mod tokens;` to `crates/illumio-tui/src/lib.rs` after `pub mod config;`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_parse_one_per_spec_and_keep_empty_ones_empty() {
        let scopes = parse_scope_tokens(&["app=web, group:G".into(), " , ".into()]).unwrap();
        assert_eq!(scopes.len(), 2);
        assert_eq!(
            scopes[0],
            vec![
                ActorToken::Label {
                    spec: "app=web".into(),
                    exclusion: false
                },
                ActorToken::LabelGroup {
                    name: "G".into(),
                    exclusion: false
                },
            ]
        );
        assert!(scopes[1].is_empty(), "the caller decides what an empty scope means");
        assert_eq!(
            parse_scope("iplist:Corp").unwrap_err(),
            "scopes take labels and label groups only"
        );
        assert!(parse_scope("app=").unwrap_err().contains("expected key=value"));
        assert_eq!(split_kv("app=a=b").unwrap(), ("app".into(), "a=b".into()));
        assert!(is_href(" /orgs/5/labels/1"));
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p illumio-tui --lib tokens::`
Expected: FAIL, compile errors `cannot find function parse_scope_tokens in this scope` (and `ActorToken`, `parse_scope`, `split_kv`, `is_href`).

- [ ] **Step 3: Implement the module** (replace the whole of `crates/illumio-tui/src/tokens.rs`; `ActorToken`, `ServiceToken`, `similar`, `with_similar` and the four moved tests are verbatim from `cli/write.rs`, only `parse_kv(spec).map_err(|e| e.to_string())?` became `split_kv(spec)?` and the href helpers return `WriteError`)

```rust
//! The token grammar the CLI and the TUI share: rule actors (`app=web`,
//! `label:key=value`, `group:NAME`, `iplist:NAME`, `workload:HOST`, `all`,
//! hrefs, `!` before a label, label group or IP list), rule services
//! (`443/tcp`, `icmp`, `HTTPS`, `service:NAME`, hrefs) and ruleset scopes.
//! Parsing makes no request, and neither does checking an href (the
//! library's strict checkers). Resolving a name is the caller's job: the
//! CLI reads each object list at most once, the TUI looks names up in the
//! loaded `NameIndex`.
use pce::write::{
    ActorKind, ActorRef, PortSpec, ServiceRef, WriteError, checked_href, checked_policy_href,
};

const ACTOR_GRAMMAR: &str = "use key=value, label:key=value, group:NAME, iplist:NAME, workload:HOST, all, an href, or ! before a label, label group or IP list";

pub fn is_href(s: &str) -> bool {
    s.trim_start().starts_with("/orgs/")
}

/// `key=value`, split on the first `=`; both sides trimmed and non-empty.
pub fn split_kv(spec: &str) -> Result<(String, String), String> {
    let (k, v) = spec
        .split_once('=')
        .ok_or_else(|| format!("expected key=value, got {spec:?}"))?;
    let (k, v) = (k.trim(), v.trim());
    if k.is_empty() || v.is_empty() {
        return Err(format!(
            "expected key=value with both sides non-empty, got {spec:?}"
        ));
    }
    Ok((k.to_string(), v.to_string()))
}

/// Up to five names that contain `needle` case-insensitively, for an
/// error that found no exact match.
pub fn similar<'a>(needle: &str, names: impl Iterator<Item = &'a str>) -> Vec<String> {
    let needle = needle.to_lowercase();
    let mut out: Vec<String> = names
        .filter(|n| n.to_lowercase().contains(&needle))
        .map(str::to_string)
        .collect();
    out.sort();
    out.dedup();
    out.truncate(5);
    out
}

pub fn with_similar(msg: String, similar: &[String]) -> String {
    if similar.is_empty() {
        msg
    } else {
        format!("{msg}; similar: {}", similar.join(", "))
    }
}

/// One provider, consumer or scope token, parsed without any request
/// (grammar in the README's Writes section).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActorToken {
    All,
    /// `key=value`, already split and re-joined on the first `=`.
    Label {
        spec: String,
        exclusion: bool,
    },
    LabelGroup {
        name: String,
        exclusion: bool,
    },
    IpList {
        name: String,
        exclusion: bool,
    },
    Workload {
        name: String,
    },
    Href {
        href: String,
        exclusion: bool,
    },
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
                Err(format!("{s:?}: all cannot be excluded"))
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
        if spec != rest && is_href(spec) {
            // `label:<href>` is checked as a label href at resolution.
            return Ok(Self::Label {
                spec: spec.to_string(),
                exclusion,
            });
        }
        if spec.contains('=') {
            let (k, v) = split_kv(spec)?;
            return Ok(Self::Label {
                spec: format!("{k}={v}"),
                exclusion,
            });
        }
        Err(format!("{s:?} is not an actor; {ACTOR_GRAMMAR}"))
    }

    /// Scopes take labels and label groups; an href's kind is checked by
    /// the library without a request.
    pub fn scope_member(&self) -> bool {
        matches!(
            self,
            Self::Label { .. } | Self::LabelGroup { .. } | Self::Href { .. }
        )
    }
}

/// One service token: an href, an inline port, or a service name.
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

/// One scope: comma-separated labels and label groups. A spec with no
/// token is an empty scope; what that means is the caller's decision (the
/// CLI refuses it, the TUI refuses it between `|`).
pub fn parse_scope(spec: &str) -> Result<Vec<ActorToken>, String> {
    let parsed = spec
        .split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(ActorToken::parse)
        .collect::<Result<Vec<_>, _>>()?;
    if parsed.iter().any(|t| !t.scope_member()) {
        return Err("scopes take labels and label groups only".into());
    }
    Ok(parsed)
}

/// [`parse_scope`] for each spec, in order.
pub fn parse_scope_tokens(specs: &[String]) -> Result<Vec<Vec<ActorToken>>, String> {
    specs.iter().map(String::as_str).map(parse_scope).collect()
}

pub fn with_exclusion(mut a: ActorRef, exclusion: bool) -> ActorRef {
    a.exclusion = exclusion;
    a
}

/// Tokens that carry an href are checked by the library's strict checkers
/// for the kind their prefix names, with no list consulted. `None` for
/// tokens that name an object (or `all`).
pub fn actor_from_href_token(
    token: &ActorToken,
    org_id: u64,
) -> Option<Result<ActorRef, WriteError>> {
    let (kind, href, segment, policy, exclusion) = match token {
        ActorToken::Href { href, exclusion } => {
            return Some(
                ActorRef::from_href(href, org_id).map(|a| with_exclusion(a, *exclusion)),
            );
        }
        ActorToken::Label { spec, exclusion } if is_href(spec) => {
            (ActorKind::Label, spec, "labels", false, *exclusion)
        }
        ActorToken::LabelGroup { name, exclusion } if is_href(name) => (
            ActorKind::LabelGroup,
            name,
            "label_groups",
            true,
            *exclusion,
        ),
        ActorToken::IpList { name, exclusion } if is_href(name) => {
            (ActorKind::IpList, name, "ip_lists", true, *exclusion)
        }
        ActorToken::Workload { name } if is_href(name) => {
            (ActorKind::Workload, name, "workloads", false, false)
        }
        _ => return None,
    };
    let checked = if policy {
        checked_policy_href(href, segment, org_id)
    } else {
        checked_href(href, segment, org_id)
    };
    Some(checked.map(|h| with_exclusion(ActorRef::new(kind, h), exclusion)))
}

pub fn service_from_href_token(
    token: &ServiceToken,
    org_id: u64,
) -> Option<Result<ServiceRef, WriteError>> {
    match token {
        ServiceToken::Href(h) => Some(ServiceRef::from_href(h, org_id)),
        ServiceToken::Name(n) if is_href(n) => Some(ServiceRef::from_href(n, org_id)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_parse_one_per_spec_and_keep_empty_ones_empty() {
        let scopes = parse_scope_tokens(&["app=web, group:G".into(), " , ".into()]).unwrap();
        assert_eq!(scopes.len(), 2);
        assert_eq!(
            scopes[0],
            vec![
                ActorToken::Label {
                    spec: "app=web".into(),
                    exclusion: false
                },
                ActorToken::LabelGroup {
                    name: "G".into(),
                    exclusion: false
                },
            ]
        );
        assert!(scopes[1].is_empty(), "the caller decides what an empty scope means");
        assert_eq!(
            parse_scope("iplist:Corp").unwrap_err(),
            "scopes take labels and label groups only"
        );
        assert!(parse_scope("app=").unwrap_err().contains("expected key=value"));
        assert_eq!(split_kv("app=a=b").unwrap(), ("app".into(), "a=b".into()));
        assert!(is_href(" /orgs/5/labels/1"));
    }

    #[test]
    fn actor_tokens_parse_every_form() {
        let label = |spec: &str, exclusion| ActorToken::Label {
            spec: spec.into(),
            exclusion,
        };
        assert_eq!(
            ActorToken::parse("app=web").unwrap(),
            label("app=web", false)
        );
        assert_eq!(
            ActorToken::parse(" label:app = a=b ").unwrap(),
            label("app=a=b", false)
        );
        assert_eq!(
            ActorToken::parse("!env=prod").unwrap(),
            label("env=prod", true)
        );
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
            r#""!all": all cannot be excluded"#
        );
        assert!(
            ActorToken::parse("!workload:x")
                .unwrap_err()
                .contains("only labels, label groups and IP lists can be excluded")
        );
        assert!(
            ActorToken::parse("group:")
                .unwrap_err()
                .contains("missing name")
        );
        assert!(
            ActorToken::parse("web")
                .unwrap_err()
                .contains("is not an actor")
        );
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
    fn prefixed_hrefs_resolve_through_the_strict_checkers_without_lists() {
        // The href paths are pure (no Context, no Lists, no request), so
        // these tests cannot fetch anything.
        let org = 5;
        let actor = |s: &str| {
            let tok = ActorToken::parse(s).unwrap();
            actor_from_href_token(&tok, org).expect("href token")
        };
        let a = actor("label:/orgs/5/labels/1").unwrap();
        assert_eq!(
            (a.kind, a.href.as_deref()),
            (ActorKind::Label, Some("/orgs/5/labels/1"))
        );
        assert!(actor("!label:/orgs/5/labels/1").unwrap().exclusion);
        let a = actor("group:/orgs/5/sec_policy/active/label_groups/2").unwrap();
        assert_eq!(a.kind, ActorKind::LabelGroup);
        assert_eq!(
            a.href.as_deref(),
            Some("/orgs/5/sec_policy/draft/label_groups/2")
        );
        let a = actor("!iplist:/orgs/5/sec_policy/draft/ip_lists/3").unwrap();
        assert_eq!(a.kind, ActorKind::IpList);
        assert!(a.exclusion);
        let a = actor("workload:/orgs/5/workloads/abc").unwrap();
        assert_eq!(a.kind, ActorKind::Workload);
        assert_eq!(a.href.as_deref(), Some("/orgs/5/workloads/abc"));
        assert!(
            actor("iplist:/orgs/5/labels/1")
                .unwrap_err()
                .to_string()
                .contains("ip_lists")
        );
        assert!(
            actor("group:/orgs/5/labels/1")
                .unwrap_err()
                .to_string()
                .contains("label_groups")
        );
        assert!(
            actor("workload:/orgs/5/labels/1")
                .unwrap_err()
                .to_string()
                .contains("workloads")
        );
        assert!(actor("label:/orgs/9/labels/1").is_err(), "wrong org");
        assert!(actor_from_href_token(&ActorToken::parse("app=web").unwrap(), org).is_none());
        assert!(actor_from_href_token(&ActorToken::parse("iplist:Corp").unwrap(), org).is_none());

        let svc = |s: &str| service_from_href_token(&ServiceToken::parse(s).unwrap(), org);
        assert_eq!(
            svc("service:/orgs/5/sec_policy/active/services/31")
                .unwrap()
                .unwrap(),
            ServiceRef::Href("/orgs/5/sec_policy/draft/services/31".into())
        );
        assert!(svc("service:/orgs/5/labels/1").unwrap().is_err());
        assert!(svc("HTTPS").is_none());
        assert!(svc("443/tcp").is_none());
    }
}
```

- [ ] **Step 4: Point the CLI at the module** (`crates/illumio-tui/src/cli/write.rs`)

Replace the import block (lines 5-14) with:

```rust
use super::context::Context;
use super::error::CliError;
use super::output::{TableRows, WriteMeta};
use crate::tokens::with_exclusion;
pub use crate::tokens::{ActorToken, ServiceToken, is_href};
pub(crate) use crate::tokens::{similar, with_similar};
use clap::Args;
use pce::models::{IpList, Label, LabelGroup, Service};
use pce::write::provision_counts;
use pce::write::{
    ActorKind, ActorRef, JsonlAudit, Outcome, Plan, ServiceRef, WriteContext, WriteError,
    WriteGuard, WriteRefused, WriteRequest, Written, checked_href, checked_policy_href,
};
```

Replace `pub fn parse_kv` (its body) with the wrapper, and delete `pub fn is_href`, `pub(crate) fn similar` and `pub(crate) fn with_similar` (they are re-exported above):

```rust
/// `key=value`, split on the first `=`; both sides trimmed and non-empty.
pub fn parse_kv(spec: &str) -> Result<(String, String), CliError> {
    crate::tokens::split_kv(spec).map_err(CliError::Args)
}
```

Delete everything from `const ACTOR_GRAMMAR` through the end of `impl ServiceToken` (the two enums and their impls now live in `tokens.rs`), keep `parse_actors` and `parse_services` as they are, and replace `parse_scopes` with:

```rust
/// Every `--scope` value: comma-separated labels and label groups.
pub fn parse_scopes(specs: &[String]) -> Result<Vec<Vec<ActorToken>>, CliError> {
    specs
        .iter()
        .map(|spec| {
            let scope = crate::tokens::parse_scope(spec).map_err(CliError::Args)?;
            if scope.is_empty() {
                return Err(CliError::Args(
                    "--scope needs at least one label or label group; leave --scope out for a ruleset that applies everywhere"
                        .into(),
                ));
            }
            Ok(scope)
        })
        .collect()
}
```

Replace `fn with_exclusion`, `fn actor_from_href_token` and `fn service_from_href_token` with:

```rust
/// [`crate::tokens::actor_from_href_token`] with the CLI's error type.
fn actor_from_href_token(token: &ActorToken, org_id: u64) -> Option<Result<ActorRef, CliError>> {
    crate::tokens::actor_from_href_token(token, org_id).map(|r| r.map_err(CliError::from))
}

/// [`crate::tokens::service_from_href_token`] with the CLI's error type.
fn service_from_href_token(
    token: &ServiceToken,
    org_id: u64,
) -> Option<Result<ServiceRef, CliError>> {
    crate::tokens::service_from_href_token(token, org_id).map(|r| r.map_err(CliError::from))
}
```

In `mod tests` of `cli/write.rs`, delete the four moved tests (`actor_tokens_parse_every_form`, `actor_tokens_refuse_bad_input`, `service_tokens_split_ports_names_and_hrefs`, `prefixed_hrefs_resolve_through_the_strict_checkers_without_lists`); `parse_kv_splits_on_the_first_equals`, `zero_matches_list_similar_names` and `scopes_take_labels_and_groups_only` stay unchanged.

- [ ] **Step 5: Run the move**

Run: `cargo fmt --all && cargo test -p illumio-tui --lib && cargo test -p illumio-tui --test cli --test cli_writes`
Expected: PASS: the five `tokens::tests`, every `cli::write::tests` test that stayed, and every CLI integration test unchanged.

- [ ] **Step 6: Gate and commit the move**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p illumio-tui
git add crates/illumio-tui/src/tokens.rs crates/illumio-tui/src/lib.rs crates/illumio-tui/src/cli/write.rs
git commit -m "refactor: move the actor, service and scope token grammar into a shared module

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

- [ ] **Step 7: Write the failing tests**

Append to `mod tests` in `crates/pce/src/resolve.rs`:

```rust
    #[test]
    fn service_href_finds_one_live_service_by_exact_name() {
        let idx = index();
        assert_eq!(
            idx.service_href("Web"),
            Some("/orgs/5/sec_policy/draft/services/32")
        );
        assert_eq!(idx.service_href("web"), None, "exact, like the CLI");
        assert_eq!(idx.service_href("Nope"), None);
        let services: Vec<Service> = serde_json::from_value(serde_json::json!([
            {"href": "/orgs/5/sec_policy/draft/services/41", "name": "Dup"},
            {"href": "/orgs/5/sec_policy/draft/services/42", "name": "Dup"},
            {"href": "/orgs/5/sec_policy/draft/services/43", "name": "Gone", "update_type": "delete"}
        ]))
        .unwrap();
        let idx = NameIndex::new(&[], &[], &[], &services);
        assert_eq!(idx.service_href("Dup"), None, "ambiguous: the href is needed");
        assert_eq!(idx.service_href("Gone"), None, "pending deletion");
    }
```

Append to `mod tests` in `crates/illumio-tui/src/tui/edit.rs`:

```rust
    fn rule_sets() -> Vec<pce::models::RuleSet> {
        serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap()
    }

    fn actor(v: serde_json::Value) -> Actor {
        serde_json::from_value(v).unwrap()
    }

    fn resolve_actors(tokens: &[String]) -> Result<Vec<ActorRef>, String> {
        let parsed = tokens
            .iter()
            .map(|t| ActorToken::parse(t))
            .collect::<Result<Vec<_>, _>>()?;
        resolve_actor_tokens(&parsed, &fixture_index(), 5)
    }

    #[test]
    fn actor_tokens_render_every_kind_and_resolve_back() {
        let sets = rule_sets();
        let actors = vec![
            sets[0].rules[0].providers[0].clone(), // label 2
            sets[0].rules[1].consumers[0].clone(), // IP list 21
            sets[1].rules[0].consumers[0].clone(), // label group 10
            sets[1].rules[0].providers[0].clone(), // ams
            actor(json!({"label": {"href": "/orgs/5/labels/1"}, "exclusion": true})),
            actor(json!({"workload": {"href": "/orgs/5/workloads/aaaa-1"}})),
            actor(json!({"label": {"href": "/orgs/5/labels/99"}})),
        ];
        let tokens = actor_tokens(&actors, &fixture_index());
        assert_eq!(
            tokens,
            vec![
                "role=db",
                "iplist:Corp",
                "group:All prod envs",
                "all",
                "!role=web",
                "workload:/orgs/5/workloads/aaaa-1",
                "/orgs/5/labels/99",
            ]
        );
        let want: Vec<ActorRef> = actors
            .iter()
            .map(|a| ActorRef::from_actor(a).unwrap())
            .collect();
        assert_eq!(
            resolve_actors(&tokens).unwrap(),
            want,
            "every token resolves back to its actor"
        );
    }

    #[test]
    fn service_tokens_render_names_and_ports_and_resolve_back() {
        let sets = rule_sets();
        let mut services = sets[0].rules[0].ingress_services.clone(); // PostgreSQL
        services.extend(sets[0].rules[1].ingress_services.clone()); // 443/tcp, 8000-8099/tcp
        services.extend(sets[1].rules[0].ingress_services.clone()); // All Services
        services.push(serde_json::from_value(json!({"proto": 1})).unwrap());
        let tokens = service_tokens(&services, &fixture_index());
        assert_eq!(
            tokens,
            vec!["PostgreSQL", "443/tcp", "8000-8099/tcp", "All Services", "icmp"]
        );
        let parsed: Vec<ServiceToken> = tokens
            .iter()
            .map(|t| ServiceToken::parse(t).unwrap())
            .collect();
        let want: Vec<ServiceRef> = services.iter().map(ServiceRef::from_ingress).collect();
        assert_eq!(
            resolve_service_tokens(&parsed, &fixture_index(), 5).unwrap(),
            want
        );
        // A service named like a protocol keeps its name behind `service:`.
        let tcp: Service = serde_json::from_value(json!({
            "href": "/orgs/5/sec_policy/draft/services/40", "name": "tcp", "service_ports": []
        }))
        .unwrap();
        let idx = NameIndex::new(&[], &[], &[], &[tcp]);
        let svc = vec![IngressService::Ref(pce::models::Ref::new(
            "/orgs/5/sec_policy/draft/services/40",
        ))];
        assert_eq!(service_tokens(&svc, &idx), vec!["service:tcp"]);
        assert_eq!(
            resolve_service_tokens(&[ServiceToken::parse("service:tcp").unwrap()], &idx, 5)
                .unwrap(),
            vec![ServiceRef::Href("/orgs/5/sec_policy/draft/services/40".into())]
        );
    }

    #[test]
    fn inexpressible_entries_render_as_href_or_question_mark() {
        let idx = fixture_index();
        let actors = vec![
            actor(json!({"virtual_service": {"href": "/orgs/5/sec_policy/draft/virtual_services/9"}})),
            Actor::default(),
        ];
        assert_eq!(
            actor_tokens(&actors, &idx),
            vec!["/orgs/5/sec_policy/draft/virtual_services/9", "?"]
        );
        let services: Vec<IngressService> = serde_json::from_value(json!([
            {"proto": -1},
            {"proto": 47},
            {"href": "/orgs/5/sec_policy/draft/services/77"}
        ]))
        .unwrap();
        assert_eq!(
            service_tokens(&services, &idx),
            vec!["?", "?", "/orgs/5/sec_policy/draft/services/77"],
            "`any` and protocol 47 cannot be typed back; an unknown service keeps its href"
        );
    }

    #[test]
    fn resolution_errors_name_what_is_known_and_workload_names_stay_in_the_cli() {
        let err = |t: &str| resolve_actors(&[t.to_string()]).unwrap_err();
        assert_eq!(
            err("app=nope"),
            "unknown label app=nope; known values for app: shop"
        );
        assert_eq!(
            err("tier=1"),
            "unknown label tier=1; no labels have key tier"
        );
        assert_eq!(err("iplist:Cor"), r#"no IP list named "Cor"; similar: Corp"#);
        assert_eq!(err("group:Nope"), r#"no label group named "Nope""#);
        assert_eq!(
            err("workload:web-1.lab"),
            "workload names resolve in the CLI; use workload:/orgs/… here"
        );
        assert!(resolve_actors(&["/orgs/9/labels/1".into()]).is_err(), "wrong org");
        assert_eq!(
            resolve_actors(&["workload:/orgs/5/workloads/aaaa-3".into()]).unwrap(),
            vec![ActorRef::new(ActorKind::Workload, "/orgs/5/workloads/aaaa-3")]
        );
        let svc = |t: &str| {
            resolve_service_tokens(&[ServiceToken::parse(t).unwrap()], &fixture_index(), 5)
        };
        assert_eq!(svc("HTTPS").unwrap_err(), r#"no service named "HTTPS""#);
        assert_eq!(
            svc("443/tcp").unwrap(),
            vec![ServiceRef::Port(PortSpec::parse("443/tcp").unwrap())]
        );
    }
```

Add `use serde_json::json;` next to `use super::*;` at the top of that `mod tests`.

- [ ] **Step 8: Run to verify failure**

Run: `cargo test -p pce --lib resolve::tests::service_href; cargo test -p illumio-tui --lib tui::edit::`
Expected: FAIL, `no method named service_href found for struct NameIndex`, and in `edit.rs` `cannot find function actor_tokens` (and `resolve_actor_tokens`, `resolve_service_tokens`, `service_tokens`).

- [ ] **Step 9: Implement**

In `crates/pce/src/resolve.rs`, directly after `pub fn service_name`:

```rust
    /// The one live service named exactly `name` (like the CLI's lookup:
    /// case-sensitive, pending deletions excluded). `None` when there is no
    /// such service, or several.
    pub fn service_href(&self, name: &str) -> Option<&str> {
        let mut hits = self
            .services
            .values()
            .filter(|s| s.name == name && s.update_type.as_deref() != Some("delete"));
        let first = hits.next()?;
        hits.next().is_none().then_some(first.href.as_str())
    }
```

In `crates/illumio-tui/src/tui/edit.rs`, replace the import block (lines 5-11) with:

```rust
use crate::tokens::{
    ActorToken, ServiceToken, actor_from_href_token, service_from_href_token, similar, split_kv,
    with_exclusion, with_similar,
};
use crate::tui::form::{Field, Form};
use pce::NameIndex;
use pce::models::{Actor, Fqdn, IngressService, Interface, IpList, Label, Service, Workload};
use pce::write::{
    ActorKind, ActorRef, InterfaceSpec, IpListDraft, IpListPatch, IpRangeSpec, LabelDraft,
    PortSpec, ServiceDraft, ServicePatch, ServiceRef, WorkloadDraft, WorkloadPatch, WriteRequest,
};
```

and insert after `fn resolve_labels`:

```rust
const WORKLOAD_NAMES: &str = "workload names resolve in the CLI; use workload:/orgs/… here";

/// Provider, consumer or scope tokens to actors through the loaded index.
/// No request is made: names are looked up in `idx`, hrefs are checked by
/// the library's strict checkers, and `workload:` takes an href only.
pub fn resolve_actor_tokens(
    tokens: &[ActorToken],
    idx: &NameIndex,
    org_id: u64,
) -> Result<Vec<ActorRef>, String> {
    tokens
        .iter()
        .map(|t| resolve_actor_token(t, idx, org_id))
        .collect()
}

fn resolve_actor_token(token: &ActorToken, idx: &NameIndex, org_id: u64) -> Result<ActorRef, String> {
    let actor = match actor_from_href_token(token, org_id) {
        Some(checked) => checked.map_err(|e| e.to_string())?,
        None => match token {
            ActorToken::All => ActorRef::all(),
            ActorToken::Label { spec, exclusion } => {
                let (k, v) = split_kv(spec)?;
                let href = idx
                    .label_href(&k, &v)
                    .ok_or_else(|| unknown_label(idx, &k, &v))?;
                with_exclusion(ActorRef::new(ActorKind::Label, href), *exclusion)
            }
            ActorToken::LabelGroup { name, exclusion } => {
                let href = idx
                    .label_group_href(name)
                    .ok_or_else(|| format!("no label group named {name:?}"))?;
                with_exclusion(ActorRef::new(ActorKind::LabelGroup, href), *exclusion)
            }
            ActorToken::IpList { name, exclusion } => {
                let href = idx.ip_list_href(name).ok_or_else(|| {
                    let names = idx.ip_list_names();
                    with_similar(
                        format!("no IP list named {name:?}"),
                        &similar(name, names.iter().map(String::as_str)),
                    )
                })?;
                with_exclusion(ActorRef::new(ActorKind::IpList, href), *exclusion)
            }
            ActorToken::Workload { .. } => return Err(WORKLOAD_NAMES.into()),
            ActorToken::Href { href, exclusion } => with_exclusion(
                ActorRef::from_href(href, org_id).map_err(|e| e.to_string())?,
                *exclusion,
            ),
        },
    };
    actor.validate()?;
    Ok(actor)
}

fn unknown_label(idx: &NameIndex, k: &str, v: &str) -> String {
    let known = idx.values_for_key(k);
    if known.is_empty() {
        format!("unknown label {k}={v}; no labels have key {k}")
    } else {
        format!(
            "unknown label {k}={v}; known values for {k}: {}",
            known.join(", ")
        )
    }
}

/// Service tokens to rule services through the loaded index: a port as
/// typed, a name by exact match, an href through the strict checker.
pub fn resolve_service_tokens(
    tokens: &[ServiceToken],
    idx: &NameIndex,
    org_id: u64,
) -> Result<Vec<ServiceRef>, String> {
    tokens
        .iter()
        .map(|t| {
            if let Some(checked) = service_from_href_token(t, org_id) {
                return checked.map_err(|e| e.to_string());
            }
            match t {
                ServiceToken::Port(p) => Ok(ServiceRef::Port(p.clone())),
                ServiceToken::Name(n) => idx
                    .service_href(n)
                    .map(|h| ServiceRef::Href(h.to_string()))
                    .ok_or_else(|| format!("no service named {n:?}")),
                ServiceToken::Href(h) => {
                    ServiceRef::from_href(h, org_id).map_err(|e| e.to_string())
                }
            }
        })
        .collect()
}

/// A rule's or a scope's actors as form tokens: `app=web`, `group:Name`,
/// `iplist:Corp`, `all`, `!…`, `workload:/orgs/…`. An object the index
/// does not know, or whose name would not survive the comma split, shows
/// its href (which resolves back as it is). An actor the grammar cannot
/// express shows its href (virtual services and servers) or `?`.
pub fn actor_tokens(actors: &[Actor], idx: &NameIndex) -> Vec<String> {
    actors.iter().map(|a| actor_token(a, idx)).collect()
}

fn actor_token(a: &Actor, idx: &NameIndex) -> String {
    let Ok(r) = ActorRef::from_actor(a) else {
        return a
            .virtual_service
            .as_ref()
            .or(a.virtual_server.as_ref())
            .map_or_else(|| "?".to_string(), |v| v.href.clone());
    };
    let Some(href) = r.href else {
        return "all".into();
    };
    let named = match r.kind {
        ActorKind::Label => idx
            .label(&href)
            .filter(|l| !l.is_deleted())
            .map(Label::display),
        ActorKind::LabelGroup => idx.label_group_name(&href).map(|n| format!("group:{n}")),
        ActorKind::IpList => idx.ip_list_name(&href).map(|n| format!("iplist:{n}")),
        ActorKind::Workload => Some(format!("workload:{href}")),
        ActorKind::All => None,
    };
    let body = named.filter(|t| !t.contains([',', '|'])).unwrap_or(href);
    if r.exclusion { format!("!{body}") } else { body }
}

/// A rule's services as form tokens: the service's name (`service:NAME`
/// when the name would read as a port), `443/tcp`, `8000-8099/tcp`,
/// `icmp`; an unknown service as its href; an inline port the grammar
/// cannot type back (`any`, a bare protocol number) as `?`.
pub fn service_tokens(services: &[IngressService], idx: &NameIndex) -> Vec<String> {
    services.iter().map(|s| service_token(s, idx)).collect()
}

fn service_token(s: &IngressService, idx: &NameIndex) -> String {
    match ServiceRef::from_ingress(s) {
        ServiceRef::Href(href) => match idx.service_name(&href) {
            Some(n) if !n.contains([',', '|']) => {
                if ServiceToken::parse(n) == Ok(ServiceToken::Name(n.to_string())) {
                    n.to_string()
                } else {
                    format!("service:{n}")
                }
            }
            _ => href,
        },
        ServiceRef::Port(p) => port_token(&p).unwrap_or_else(|| "?".into()),
    }
}

/// `p` as typed (`443/tcp`, `icmp`) when the parser reads it back as `p`.
fn port_token(p: &PortSpec) -> Option<String> {
    let t = p.display();
    PortSpec::parse(&t).is_ok_and(|q| q == *p).then_some(t)
}
```

- [ ] **Step 10: Run the tests**

Run: `cargo test -p pce --lib resolve:: && cargo test -p illumio-tui --lib tui::edit::`
Expected: PASS (`service_href_finds_one_live_service_by_exact_name`, the four new `edit` tests and every existing `edit` test).

- [ ] **Step 11: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p pce && cargo test -p illumio-tui
git add crates/pce/src/resolve.rs crates/illumio-tui/src/tui/edit.rs
git commit -m "feat(tui): resolve and render policy tokens through the loaded name index

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 2: Ruleset editing

**Files:**
- Modify: `crates/illumio-tui/src/tui/edit.rs` (`EditIntent` ~line 15, `WriteIntent` ~line 33 and `kind()` ~line 49; `start` ~line 82; new field builders and helpers after `workload_fields` ~line 239; `build_intent` signature ~line 444 and new arms; the tests' `build` helper ~line 641 and the two `build_intent(&f, None)` calls ~lines 801, 808; imports)
- Modify: `crates/illumio-tui/src/tui/model.rs` (`PolicyState` fields ~line 923, `PolicyState::new` ~line 949, new `selected_rule_set` after `selected_href` ~line 1030)
- Modify: `crates/illumio-tui/src/tui/update.rs` (imports lines 6-9; `Msg::RuleSetsLoaded` ~lines 263-306; the form submit ~line 825; `policy_key` ~line 1237; `reload_after_write` ~line 1376; new `policy_edit_key` and `ruleset_intent` after `workloads_edit_key` ~line 1796; the test in `a_created_label_joins_the_name_index_when_the_labels_reload` ~line 5442; tests)
- Modify: `crates/illumio-tui/src/tui/app.rs` (`plan_intent` ~line 482)
- Test: `crates/illumio-tui/tests/tui_snapshots.rs`, `crates/illumio-tui/tests/tui_runtime.rs` (append)

**Interfaces:**
- Consumes: `RuleSetDraft::{validate}`, `RuleSetPatch::{validate, is_empty}` (`pce/src/write/rule_sets.rs` ~lines 13-112), `Writer::{plan_create_rule_set(&RuleSetDraft, IfMissing), plan_update_rule_set(&str, &RuleSetPatch), plan_delete_rule_set(&str)}` (~lines 139-235), `parse_scope_tokens` (Task 1), `resolve_actor_tokens`, `actor_tokens` (Task 1), `Form::value` (`tui/form.rs` ~line 64), `writes_refused`, `begin_edit`, `load_rule_sets` (`update.rs` ~lines 1687, 1700, 1210), `named` (`app.rs` ~line 521), `written`, `writes_on`, `key`, `kc` (update tests ~lines 4996, 4727, 1874, 2634), `key_session`, `typed`, `ch`, `writes_seen`, `writes_model`, `tui_guard`, `mount_objects` (`tests/tui_runtime.rs`).
- Produces:

```rust
// tui/edit.rs
pub enum EditIntent { /* Plan 4 variants */ CreateRuleSet, UpdateRuleSet(RuleSet), DeleteRuleSet(RuleSet) }
pub enum WriteIntent { /* Plan 4 variants */ CreateRuleSet(RuleSetDraft), UpdateRuleSet { href: String, patch: RuleSetPatch }, DeleteRuleSet { href: String, name: String } }
pub fn build_intent(form: &Form, index: Option<&NameIndex>, org_id: u64) -> Result<Option<WriteIntent>, String>;
// tui/model.rs
pub struct PolicyState { /* … */ pub quiet_reload: bool }
impl PolicyState { pub fn selected_rule_set(&self) -> Option<&RuleSet>; }
// tui/update.rs (private)
fn policy_edit_key(model: &mut Model, c: char) -> Vec<Cmd>;
fn ruleset_intent(p: &PolicyState, c: char) -> Option<EditIntent>;
// tests/tui_runtime.rs
async fn mount_rule_sets(server: &MockServer, n: u64);
```

- [ ] **Step 1: Write the failing tests**

In `crates/illumio-tui/src/tui/edit.rs` tests, change the `build` helper and the two direct calls to pass the org (they stay otherwise unchanged):

```rust
    fn build(f: &Form) -> Result<Option<WriteIntent>, String> {
        build_intent(f, Some(&*fixture_index()), 5)
    }
```

```rust
        let err = build_intent(&f, None, 5).unwrap_err();
```

```rust
            build_intent(&f, None, 5).unwrap().is_some(),
```

and append:

```rust
    #[test]
    fn rule_set_form_prefills_scopes_as_tokens() {
        let sets = rule_sets();
        let f = form(EditIntent::UpdateRuleSet(sets[0].clone()));
        assert_eq!(f.title, "Edit ruleset Shop-Prod");
        assert_eq!(
            f.fields.iter().map(|x| x.label).collect::<Vec<_>>(),
            vec!["Name", "Description", "Enabled", "Scopes"]
        );
        assert_eq!(f.value("Description"), "Ringfence");
        assert_eq!(f.value("Enabled"), "y");
        assert_eq!(f.value("Scopes"), "app=shop, env=prod");
        assert_eq!(build(&f).unwrap(), None, "a pre-filled form changes nothing");
        let g = form(EditIntent::UpdateRuleSet(sets[1].clone()));
        assert_eq!(g.value("Enabled"), "n");
        assert_eq!(g.value("Scopes"), "", "[[]] applies everywhere");
        assert_eq!(build(&g).unwrap(), None);
        let n = form(EditIntent::CreateRuleSet);
        assert_eq!(n.title, "New ruleset");
        assert_eq!((n.value("Enabled"), n.value("Scopes")), ("y", ""));
    }

    #[test]
    fn rule_set_create_and_edit_build_drafts_and_patches() {
        let mut f = form(EditIntent::CreateRuleSet);
        set(&mut f, "Name", "Lab");
        set(&mut f, "Scopes", "role=web, env=prod | group:All prod envs");
        assert_eq!(
            build(&f).unwrap(),
            Some(WriteIntent::CreateRuleSet(RuleSetDraft {
                name: "Lab".into(),
                description: None,
                enabled: true,
                scopes: vec![
                    vec![
                        ActorRef::new(ActorKind::Label, "/orgs/5/labels/1"),
                        ActorRef::new(ActorKind::Label, "/orgs/5/labels/4"),
                    ],
                    vec![ActorRef::new(
                        ActorKind::LabelGroup,
                        "/orgs/5/sec_policy/draft/label_groups/10"
                    )],
                ],
            }))
        );
        set(&mut f, "Scopes", "iplist:Corp");
        assert_eq!(
            build(&f).unwrap_err(),
            "Scopes: scopes take labels and label groups only"
        );
        set(&mut f, "Scopes", "role=web |");
        let err = build(&f).unwrap_err();
        assert!(err.contains("an empty scope"), "{err}");
        set(&mut f, "Scopes", "");
        set(&mut f, "Enabled", "maybe");
        assert_eq!(build(&f).unwrap_err(), r#"Enabled: "maybe" is not y or n"#);
        // An edit sends what changed; the scopes only when their text changed.
        let mut e = form(EditIntent::UpdateRuleSet(rule_sets()[0].clone()));
        set(&mut e, "Description", "Ringfence shop");
        set(&mut e, "Enabled", "N");
        assert_eq!(
            build(&e).unwrap(),
            Some(WriteIntent::UpdateRuleSet {
                href: "/orgs/5/sec_policy/draft/rule_sets/100".into(),
                patch: RuleSetPatch {
                    description: Some("Ringfence shop".into()),
                    enabled: Some(false),
                    ..RuleSetPatch::default()
                },
            })
        );
        set(&mut e, "Scopes", "");
        let Some(WriteIntent::UpdateRuleSet { patch, .. }) = build(&e).unwrap() else {
            panic!("not an update")
        };
        assert_eq!(
            patch.set_scopes,
            Some(vec![]),
            "an emptied Scopes field means everywhere"
        );
        let Start::Plan(w) = start(EditIntent::DeleteRuleSet(rule_sets()[0].clone()), None) else {
            panic!("a delete opens no form")
        };
        assert_eq!(
            w,
            WriteIntent::DeleteRuleSet {
                href: "/orgs/5/sec_policy/draft/rule_sets/100".into(),
                name: "Shop-Prod".into()
            }
        );
        assert_eq!(w.kind(), pce::write::ObjectKind::RuleSet);
    }
```

In `crates/illumio-tui/src/tui/update.rs` tests, add the org to the `build_intent` call in `a_created_label_joins_the_name_index_when_the_labels_reload`:

```rust
                Some(idx.as_ref()),
                5
            )
            .is_ok_and(|i| i.is_some())
```

and append:

```rust
    #[test]
    fn ruleset_keys_open_forms_and_x_plans_the_delete() {
        use crate::tui::edit::WriteIntent;
        use crate::tui::test_support::model_with_policy;
        let mut m = writes_on(model_with_policy());
        m.screen = Screen::Policy;
        assert!(update(&mut m, key('n')).is_empty());
        assert!(
            matches!(&m.overlay, Overlay::Form(f) if f.title == "New ruleset"),
            "{:?}",
            m.overlay
        );
        update(&mut m, kc(KeyCode::Esc));
        assert!(update(&mut m, key('e')).is_empty());
        assert!(
            matches!(&m.overlay, Overlay::Form(f) if f.title == "Edit ruleset Shop-Prod"),
            "{:?}",
            m.overlay
        );
        update(&mut m, kc(KeyCode::Esc));
        assert_eq!(
            update(&mut m, key('x')),
            vec![Cmd::PlanWrite(WriteIntent::DeleteRuleSet {
                href: "/orgs/5/sec_policy/draft/rule_sets/100".into(),
                name: "Shop-Prod".into(),
            })]
        );
        assert!(m.writes.in_flight);
        assert!(update(&mut m, key('e')).is_empty(), "one write at a time");
        assert_eq!(m.status.text, "a write is already in progress");
    }

    #[test]
    fn policy_keys_refuse_in_the_active_scope_and_without_a_selection() {
        use crate::tui::test_support::model_with_policy;
        let mut m = model_with_policy();
        m.screen = Screen::Policy;
        for c in ['n', 'e', 'x'] {
            assert!(update(&mut m, key(c)).is_empty(), "{c}");
            assert_eq!(m.status.kind, StatusKind::Warn);
            assert_eq!(m.status.text, "started without --allow-writes");
            assert_eq!(m.overlay, Overlay::None);
        }
        let mut m = writes_on(model_with_policy());
        m.screen = Screen::Policy;
        m.policy.scope = PolicyScope::Active;
        for c in ['n', 'e', 'x'] {
            assert!(update(&mut m, key(c)).is_empty(), "{c}");
            assert_eq!(m.status.kind, StatusKind::Warn);
            assert_eq!(m.status.text, "switch to draft to edit policy");
            assert_eq!(m.overlay, Overlay::None);
        }
        let mut m = writes_on(model_fixture());
        m.screen = Screen::Policy;
        for c in ['e', 'x'] {
            assert!(update(&mut m, key(c)).is_empty(), "{c}");
            assert_eq!(m.status.text, "no row selected");
            assert_eq!(m.overlay, Overlay::None);
        }
    }

    #[test]
    fn a_policy_write_reloads_the_draft_rulesets_and_keeps_the_report() {
        use crate::tui::test_support::model_with_policy;
        use pce::write::Outcome;
        let mut m = model_with_policy();
        m.screen = Screen::Policy;
        let cmds = update(
            &mut m,
            Msg::WriteDone {
                kind: pce::write::ObjectKind::RuleSet,
                method: Some(pce::WriteMethod::Post),
                result: Ok(Outcome::Written(written(
                    "/orgs/5/sec_policy/draft/rule_sets/102",
                    true,
                ))),
            },
        );
        assert_eq!(
            cmds,
            vec![Cmd::LoadRuleSets {
                scope: PolicyScope::Draft,
                mode: pce::CacheMode::Bypass
            }]
        );
        let report = "created /orgs/5/sec_policy/draft/rule_sets/102 (pending)";
        assert_eq!(m.status.text, report);
        let sets = m.policy.raw.clone();
        update(
            &mut m,
            Msg::RuleSetsLoaded {
                scope: PolicyScope::Draft,
                result: Ok((sets.clone(), 0)),
            },
        );
        assert_eq!(m.status.text, report, "the reload keeps the write's report");
        update(
            &mut m,
            Msg::RuleSetsLoaded {
                scope: PolicyScope::Draft,
                result: Ok((sets, 0)),
            },
        );
        assert_eq!(m.status.text, "2 rulesets (draft)", "only that one reload is quiet");
        m.policy.scope = PolicyScope::Active;
        let cmds = update(
            &mut m,
            Msg::WriteDone {
                kind: pce::write::ObjectKind::Rule,
                method: Some(pce::WriteMethod::Delete),
                result: Ok(Outcome::Written(written(
                    "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1",
                    true,
                ))),
            },
        );
        assert!(cmds.is_empty(), "a draft write does not change the active view");
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p illumio-tui --lib`
Expected: FAIL, compile errors `no variant named CreateRuleSet found for enum EditIntent` and `this function takes 2 arguments but 3 arguments were supplied` (`build_intent`).

- [ ] **Step 3: Implement**

`crates/illumio-tui/src/tui/edit.rs`. Imports (replace the block from Task 1):

```rust
use crate::tokens::{
    ActorToken, ServiceToken, actor_from_href_token, parse_scope_tokens, service_from_href_token,
    similar, split_kv, with_exclusion, with_similar,
};
use crate::tui::form::{Field, Form};
use pce::NameIndex;
use pce::models::{
    Actor, Fqdn, IngressService, Interface, IpList, Label, RuleSet, Service, Workload,
};
use pce::write::{
    ActorKind, ActorRef, InterfaceSpec, IpListDraft, IpListPatch, IpRangeSpec, LabelDraft,
    PortSpec, RuleSetDraft, RuleSetPatch, ServiceDraft, ServicePatch, ServiceRef, WorkloadDraft,
    WorkloadPatch, WriteRequest,
};
```

Add to `EditIntent` (after `DeleteWorkload(Workload),`):

```rust
    CreateRuleSet,
    UpdateRuleSet(RuleSet),
    DeleteRuleSet(RuleSet),
```

Add to `WriteIntent` (after `DeleteWorkload { href: String, name: String },`):

```rust
    CreateRuleSet(RuleSetDraft),
    UpdateRuleSet { href: String, patch: RuleSetPatch },
    DeleteRuleSet { href: String, name: String },
```

Add to the `match` in `WriteIntent::kind` (after the workload arm):

```rust
            WriteIntent::CreateRuleSet(_)
            | WriteIntent::UpdateRuleSet { .. }
            | WriteIntent::DeleteRuleSet { .. } => pce::write::ObjectKind::RuleSet,
```

In `start`, add as the first statement (before `let form = …`):

```rust
    // Policy forms render actors through the index; without one they show hrefs.
    let empty = NameIndex::default();
    let idx = index.unwrap_or(&empty);
```

and add these arms to its `match` (after the `DeleteWorkload` arm):

```rust
        EditIntent::CreateRuleSet => form(
            "New ruleset".into(),
            rule_set_fields(None, idx),
            EditIntent::CreateRuleSet,
        ),
        EditIntent::UpdateRuleSet(rs) => {
            let title = format!("Edit ruleset {}", rs.name);
            let fields = rule_set_fields(Some(&rs), idx);
            form(title, fields, EditIntent::UpdateRuleSet(rs))
        }
        EditIntent::DeleteRuleSet(rs) => Start::Plan(WriteIntent::DeleteRuleSet {
            name: rs.name,
            href: rs.href,
        }),
```

Insert after `fn workload_fields`:

```rust
const SCOPES_HINT: &str = "app=web, env=prod | app=db; empty = everywhere";
const YES_NO_HINT: &str = "y or n";
const NO_INDEX: &str = "the name index is not loaded; press r on Objects";
const CANNOT_EDIT_RULESET: &str = "this ruleset has entries the form cannot edit; use the CLI";

fn yn(on: bool) -> &'static str {
    if on { "y" } else { "n" }
}

/// `y`/`yes` or `n`/`no`, any case.
fn yes_no(form: &Form, label: &str) -> Result<bool, String> {
    match form.value(label).trim().to_ascii_lowercase().as_str() {
        "y" | "yes" => Ok(true),
        "n" | "no" => Ok(false),
        other => Err(format!("{label}: {other:?} is not y or n")),
    }
}

fn need_index(index: Option<&NameIndex>) -> Result<&NameIndex, String> {
    index.ok_or_else(|| NO_INDEX.to_string())
}

fn rule_set_fields(rs: Option<&RuleSet>, idx: &NameIndex) -> Vec<Field> {
    vec![
        Field::new(
            "Name",
            rs.map(|r| r.name.clone()).unwrap_or_default(),
            "Shop-Prod",
            true,
        ),
        Field::new(
            "Description",
            rs.and_then(|r| r.description.clone()).unwrap_or_default(),
            "optional",
            false,
        ),
        Field::new(
            "Enabled",
            yn(rs.is_none_or(|r| r.enabled)),
            YES_NO_HINT,
            true,
        ),
        Field::new(
            "Scopes",
            rs.map(|r| scopes_text(&r.scopes, idx)).unwrap_or_default(),
            SCOPES_HINT,
            false,
        ),
    ]
}

/// Scopes as the Scopes field shows them, `app=web, env=prod | group:G`;
/// empty for a ruleset that applies everywhere (`[[]]` or `[]`).
fn scopes_text(scopes: &[Vec<Actor>], idx: &NameIndex) -> String {
    scopes
        .iter()
        .filter(|s| !s.is_empty())
        .map(|s| actor_tokens(s, idx).join(", "))
        .collect::<Vec<_>>()
        .join(" | ")
}

/// The Scopes text as tokens per scope, for the "did it change" test.
fn scope_items(text: &str) -> Vec<Vec<String>> {
    text.split('|')
        .map(items)
        .filter(|s| !s.is_empty())
        .collect()
}

/// The Scopes field to scopes: empty means everywhere; each `|`-separated
/// part is one scope of labels and label groups, resolved through the index.
fn resolve_scopes(text: &str, idx: &NameIndex, org_id: u64) -> Result<Vec<Vec<ActorRef>>, String> {
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    let specs: Vec<String> = text.split('|').map(str::to_string).collect();
    let scopes = parse_scope_tokens(&specs).map_err(|e| format!("Scopes: {e}"))?;
    if scopes.iter().any(Vec::is_empty) {
        return Err(
            "Scopes: an empty scope between |; leave Scopes empty for a ruleset that applies everywhere"
                .into(),
        );
    }
    scopes
        .iter()
        .map(|s| resolve_actor_tokens(s, idx, org_id).map_err(|e| format!("Scopes: {e}")))
        .collect()
}
```

Change the signature of `build_intent` and document the new argument:

```rust
/// The submitted form as a write: `Ok(None)` when an edit changes nothing,
/// `Err` (shown in the form) when a field does not parse or validate.
/// `org_id` is the profile's org, for the href checkers.
pub fn build_intent(
    form: &Form,
    index: Option<&NameIndex>,
    org_id: u64,
) -> Result<Option<WriteIntent>, String> {
```

Add these arms to its `match` (before the delete arm), and add `| EditIntent::DeleteRuleSet(_)` to the delete arm:

```rust
        EditIntent::CreateRuleSet => {
            let idx = need_index(index)?;
            let d = RuleSetDraft {
                name: v("Name"),
                description: non_empty(v("Description")),
                enabled: yes_no(form, "Enabled")?,
                scopes: resolve_scopes(&v("Scopes"), idx, org_id)?,
            };
            d.validate().map_err(invalid)?;
            Ok(Some(WriteIntent::CreateRuleSet(d)))
        }
        EditIntent::UpdateRuleSet(rs) => {
            let idx = need_index(index)?;
            let enabled = yes_no(form, "Enabled")?;
            let scopes = v("Scopes");
            // The field shows every scope, so a changed field replaces them all.
            let set_scopes = if scope_items(&scopes) == scope_items(&scopes_text(&rs.scopes, idx)) {
                None
            } else if rs
                .scopes
                .iter()
                .flatten()
                .any(|a| ActorRef::from_actor(a).is_err())
            {
                return Err(format!("Scopes: {CANNOT_EDIT_RULESET}"));
            } else {
                Some(resolve_scopes(&scopes, idx, org_id)?)
            };
            let patch = RuleSetPatch {
                name: changed(&rs.name, v("Name")),
                description: changed(rs.description.as_deref().unwrap_or(""), v("Description")),
                enabled: (enabled != rs.enabled).then_some(enabled),
                set_scopes,
            };
            patch.validate().map_err(invalid)?;
            if patch.is_empty() {
                Ok(None)
            } else {
                Ok(Some(WriteIntent::UpdateRuleSet {
                    href: rs.href.clone(),
                    patch,
                }))
            }
        }
```

The delete arm becomes:

```rust
        // Deletes never open a form (`start`); a form never carries one.
        EditIntent::DeleteLabel(_)
        | EditIntent::DeleteIpList(_)
        | EditIntent::DeleteService(_)
        | EditIntent::DeleteWorkload(_)
        | EditIntent::DeleteRuleSet(_) => Err("a delete has no form".into()),
```

`crates/illumio-tui/src/tui/model.rs`. Add to `PolicyState` (after `pub diff_loading: bool,`):

```rust
    /// The next rulesets result was asked for by a write: it keeps the
    /// write's report in the status line instead of `N rulesets`.
    pub quiet_reload: bool,
```

initialise it in `PolicyState::new` (after `diff_loading: false,`):

```rust
            quiet_reload: false,
```

and add after `pub fn selected_href`:

```rust
    /// The selected ruleset as loaded, for an edit form.
    pub fn selected_rule_set(&self) -> Option<&RuleSet> {
        let href = self.selected_href()?;
        self.raw.iter().find(|r| r.href == href)
    }
```

`crates/illumio-tui/src/tui/update.rs`. Imports:

```rust
use crate::tui::model::{
    Connection, EventsFilterEdit, Model, ObjectKind, Overlay, PendingAction, PolicyPane,
    PolicyState, QueryState, Screen, StatusLine, TrafficData, TrafficView,
};
```

In `Msg::RuleSetsLoaded`, take the flag right after `model.policy.loading = false;`:

```rust
            model.policy.loading = false;
            // A reload a write asked for keeps the write's report.
            let quiet = std::mem::take(&mut model.policy.quiet_reload);
```

and wrap the status line it sets:

```rust
                    if !quiet {
                        model.status = StatusLine::info(format!(
                            "{} rulesets ({})",
                            model.policy.rulesets.len(),
                            model.policy.scope.segment()
                        ));
                    }
```

The form submit passes the org:

```rust
                match edit::build_intent(
                    &form,
                    model.index().map(|i| i.as_ref()),
                    model.profile.org_id,
                ) {
```

In `policy_key`, add to the `match k.code` (after the `KeyCode::Char('a')` arm):

```rust
            KeyCode::Char(c @ ('n' | 'e' | 'x')) if model.policy.pane == PolicyPane::RuleSets => {
                return policy_edit_key(model, c);
            }
```

Replace the policy arm of `reload_after_write` (the `RuleSet | Rule | DenyRule | Provision => Vec::new()` arm and its comment) with:

```rust
        // A draft write changes the draft view only; the reload keeps the
        // write's report in the status line.
        pce::write::ObjectKind::RuleSet
        | pce::write::ObjectKind::Rule
        | pce::write::ObjectKind::DenyRule => {
            if model.policy.scope != PolicyScope::Draft {
                return Vec::new();
            }
            model.policy.quiet_reload = true;
            load_rule_sets(model, CacheMode::Bypass)
        }
        // Not started from the TUI yet.
        pce::write::ObjectKind::Provision => Vec::new(),
```

Insert after `fn workloads_edit_key`:

```rust
/// `n` / `e` / `x` on Policy (and `N` on the rules pane): draft only.
fn policy_edit_key(model: &mut Model, c: char) -> Vec<Cmd> {
    if writes_refused(model) {
        return Vec::new();
    }
    if model.policy.scope == PolicyScope::Active {
        model.status = StatusLine::warn("switch to draft to edit policy");
        return Vec::new();
    }
    let intent = match model.policy.pane {
        PolicyPane::RuleSets => ruleset_intent(&model.policy, c),
        PolicyPane::Rules => None,
    };
    let Some(intent) = intent else {
        model.status = StatusLine::warn("no row selected");
        return Vec::new();
    };
    begin_edit(model, intent)
}

/// `n` a new ruleset; `e` / `x` the selected one.
fn ruleset_intent(p: &PolicyState, c: char) -> Option<EditIntent> {
    if c == 'n' {
        return Some(EditIntent::CreateRuleSet);
    }
    let rs = p.selected_rule_set()?.clone();
    Some(if c == 'e' {
        EditIntent::UpdateRuleSet(rs)
    } else {
        EditIntent::DeleteRuleSet(rs)
    })
}
```

`crates/illumio-tui/src/tui/app.rs`, `plan_intent`: add after the `DeleteWorkload` arm:

```rust
        WriteIntent::CreateRuleSet(d) => w.plan_create_rule_set(&d, IfMissing::Create).await?,
        WriteIntent::UpdateRuleSet { href, patch } => {
            w.plan_update_rule_set(&href, &patch).await?
        }
        WriteIntent::DeleteRuleSet { href, name } => {
            named(w.plan_delete_rule_set(&href)?, "rule_set", &name)
        }
```

- [ ] **Step 4: Run the unit tests**

Run: `cargo fmt --all && cargo test -p illumio-tui --lib`
Expected: PASS (the two new `edit` tests, the three new `update` tests, every existing test).

- [ ] **Step 5: Write the snapshot and runtime tests**

Append to `crates/illumio-tui/tests/tui_snapshots.rs`:

```rust
#[test]
fn rule_set_form_80x24() {
    use illumio_tui::tui::edit::{EditIntent, form_for};
    use illumio_tui::tui::model::{Overlay, Screen};
    use illumio_tui::tui::test_support::fixture_index;
    let mut m = model_with_policy();
    m.screen = Screen::Policy;
    let shop = m.policy.raw[0].clone();
    let idx = fixture_index();
    m.overlay = Overlay::Form(
        form_for(EditIntent::UpdateRuleSet(shop), Some(idx.as_ref())).expect("a form"),
    );
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("Edit ruleset Shop-Prod"), "{out}");
    assert!(out.contains("app=shop, env=prod"), "{out}");
    assert!(out.contains("Enabled*"), "{out}");
    insta::assert_snapshot!("rule_set_form_80x24", out);
    m.set_theme("light");
    insta::assert_snapshot!("rule_set_form_light_styled_80x24", render_styled(&m, 80, 24));
}
```

Append to `crates/illumio-tui/tests/tui_runtime.rs`:

```rust
/// The draft rulesets, expected `n` times (the Policy screen plus reloads);
/// the first ruleset has inline deny rules, so no fallback is asked for.
async fn mount_rule_sets(server: &MockServer, n: u64) {
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/rule_sets.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .expect(n)
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets/100/deny_rules"))
        .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
        .expect(0)
        .mount(server)
        .await;
}

#[tokio::test]
async fn create_ruleset_posts_once_and_reloads_the_rulesets_once() {
    let server = MockServer::start().await;
    mount_objects(&server).await;
    mount_rule_sets(&server, 2).await;
    let body = json!({
        "name": "Lab",
        "enabled": true,
        "scopes": [[{"label": {"href": "/orgs/5/labels/1"}}]]
    });
    let mut created = body.clone();
    created["href"] = json!("/orgs/5/sec_policy/draft/rule_sets/102");
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets"))
        .and(body_json(body))
        .respond_with(ResponseTemplate::new(201).set_body_json(created))
        .expect(1)
        .mount(&server)
        .await;
    let audit = MemoryAudit::new();
    let mut keys = vec![(500, ch('2')), (300, ch('n'))];
    keys.extend(typed("Lab"));
    keys.extend([(20, KeyCode::Tab), (20, KeyCode::Tab), (20, KeyCode::Tab)]);
    keys.extend(typed("role=web"));
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
        "created /orgs/5/sec_policy/draft/rule_sets/102 (pending)"
    );
    assert_eq!(
        model.stats.calls, 8,
        "health + four lists + rulesets + POST + one rulesets reload"
    );
    assert_eq!(writes_seen(&server).await, vec!["POST"]);
    let entries = audit.entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].source, "tui");
    assert_eq!(model.busy, 0);
}
```

- [ ] **Step 6: Create and read the snapshots, run everything**

Run: `INSTA_UPDATE=always cargo test -p illumio-tui --test tui_snapshots rule_set_form && git status --short crates/illumio-tui/tests/snapshots`
Expected: exactly two new files, `crates/illumio-tui/tests/snapshots/tui_snapshots__rule_set_form_80x24.snap` and `crates/illumio-tui/tests/snapshots/tui_snapshots__rule_set_form_light_styled_80x24.snap`. Read both: the form box titled `Edit ruleset Shop-Prod`, four fields (`Name*`, `Description`, `Enabled*`, `Scopes`), `y` and `app=shop, env=prod` pre-filled.

Run: `cargo test -p illumio-tui`
Expected: PASS, including `create_ruleset_posts_once_and_reloads_the_rulesets_once`.

- [ ] **Step 7: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p illumio-tui
git add crates/illumio-tui/src/tui/edit.rs crates/illumio-tui/src/tui/model.rs crates/illumio-tui/src/tui/update.rs crates/illumio-tui/src/tui/app.rs crates/illumio-tui/tests/tui_snapshots.rs crates/illumio-tui/tests/tui_runtime.rs crates/illumio-tui/tests/snapshots/tui_snapshots__rule_set_form_80x24.snap crates/illumio-tui/tests/snapshots/tui_snapshots__rule_set_form_light_styled_80x24.snap
git commit -m "feat(tui): create, edit and delete draft rulesets on the Policy screen

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 3: Rule and deny-rule editing

**Files:**
- Modify: `crates/illumio-tui/src/tui/edit.rs` (imports; `RulesetTarget` before `EditIntent`; `EditIntent`, `WriteIntent`, `kind()`; `start`; rule field builders after `resolve_scopes` (Task 2); adapters after `port_token` (Task 1); `ruleset_href_named`; `build_intent` arms; tests)
- Modify: `crates/illumio-tui/src/tui/model.rs` (imports lines 7-10; `find_rule`, `find_deny_rule` after `selected_rule_set` (Task 2); test)
- Modify: `crates/illumio-tui/src/tui/update.rs` (imports line 16; `policy_key` binding from Task 2; `policy_edit_key` from Task 2; new `rule_intent` after `ruleset_intent`; tests)
- Modify: `crates/illumio-tui/src/tui/app.rs` (imports lines 7 and 17; `plan_intent`)
- Modify: `crates/illumio-tui/src/tui/ui/widgets/overlay.rs` (`screen_hint` ~line 598)
- Test: `crates/illumio-tui/tests/tui_snapshots.rs`, `crates/illumio-tui/tests/tui_runtime.rs` (append)

**Interfaces:**
- Consumes: `RuleDraft`, `RulePatch`, `DenyRuleDraft`, `DenyRulePatch` (with `validate`/`is_empty`, `Default` on the patches; `pce/src/write/rules.rs` ~lines 17-63), `Writer::{plan_create_rule(&str, &RuleDraft), plan_update_rule(&str, &RulePatch), plan_delete_rule(&str), plan_create_deny_rule(&str, &DenyRuleDraft), plan_update_deny_rule(&str, &DenyRulePatch), plan_delete_deny_rule(&str)}` (`rules.rs` ~lines 508-588), `Pce::rule_sets(PolicyScope, CacheMode)` (`pce/src/lib.rs` ~line 239), `ActorRef::same_actor`, `ServiceRef::same_service` (`actors.rs` ~lines 180, 237), `RuleKind`, `RuleView` (`resolve.rs` ~lines 285-300), Task 1 resolvers and renderers, Task 2 `yn`, `yes_no`, `need_index`, `YES_NO_HINT`, `policy_edit_key`, `mount_rule_sets`.
- Produces:

```rust
// tui/edit.rs
pub enum RulesetTarget { Href(String), Name(String) }
pub enum EditIntent { /* … */ CreateRule { ruleset_href: String }, CreateDenyRule { ruleset_href: String },
                      UpdateRule(Rule, String), UpdateDenyRule(DenyRule, String),
                      DeleteRule { href: String }, DeleteDenyRule { href: String } }
pub enum WriteIntent { /* … */ CreateRule { ruleset: RulesetTarget, draft: RuleDraft }, UpdateRule { href: String, patch: RulePatch },
                       DeleteRule { href: String }, CreateDenyRule { ruleset_href: String, draft: DenyRuleDraft },
                       UpdateDenyRule { href: String, patch: DenyRulePatch }, DeleteDenyRule { href: String } }
pub fn ruleset_href_named(sets: &[RuleSet], name: &str) -> Result<String, String>;
// tui/model.rs
impl PolicyState {
    pub fn find_rule(&self, href: &str) -> Option<(Rule, String)>;       // (rule, ruleset href)
    pub fn find_deny_rule(&self, href: &str) -> Option<(DenyRule, String)>;
}
// tui/update.rs (private)
fn rule_intent(p: &PolicyState, c: char) -> Option<EditIntent>;
```

- [ ] **Step 1: Write the failing tests**

Append to the tests in `crates/illumio-tui/src/tui/edit.rs`:

```rust
    const SHOP: &str = "/orgs/5/sec_policy/draft/rule_sets/100";

    #[test]
    fn rule_forms_create_allow_and_deny_rules_from_tokens() {
        let mut f = form(EditIntent::CreateRule {
            ruleset_href: SHOP.into(),
        });
        assert_eq!(f.title, "New allow rule in ruleset 100");
        assert_eq!(
            f.fields.iter().map(|x| x.label).collect::<Vec<_>>(),
            vec!["Providers", "Consumers", "Services", "Description", "Enabled", "Unscoped"]
        );
        assert_eq!((f.value("Enabled"), f.value("Unscoped")), ("y", "n"));
        set(&mut f, "Providers", "role=db");
        set(&mut f, "Consumers", "role=web, !env=prod");
        set(&mut f, "Services", "PostgreSQL, 443/tcp");
        assert_eq!(
            build(&f).unwrap(),
            Some(WriteIntent::CreateRule {
                ruleset: RulesetTarget::Href(SHOP.into()),
                draft: RuleDraft {
                    description: None,
                    enabled: true,
                    providers: vec![ActorRef::new(ActorKind::Label, "/orgs/5/labels/2")],
                    consumers: vec![
                        ActorRef::new(ActorKind::Label, "/orgs/5/labels/1"),
                        ActorRef::new(ActorKind::Label, "/orgs/5/labels/4").excluded(),
                    ],
                    services: vec![
                        ServiceRef::Href("/orgs/5/sec_policy/draft/services/31".into()),
                        ServiceRef::Port(PortSpec::parse("443/tcp").unwrap()),
                    ],
                    unscoped_consumers: false,
                    resolve_labels_as: None,
                },
            })
        );
        set(&mut f, "Consumers", "workload:web-1");
        assert_eq!(
            build(&f).unwrap_err(),
            "Consumers: workload names resolve in the CLI; use workload:/orgs/… here"
        );
        let mut d = form(EditIntent::CreateDenyRule {
            ruleset_href: SHOP.into(),
        });
        assert_eq!(d.title, "New deny rule in ruleset 100");
        assert_eq!(
            d.fields.iter().map(|x| x.label).collect::<Vec<_>>(),
            vec!["Providers", "Consumers", "Services", "Description", "Enabled", "Override"]
        );
        set(&mut d, "Providers", "all");
        set(&mut d, "Consumers", "iplist:Corp");
        set(&mut d, "Services", "23/tcp");
        set(&mut d, "Override", "y");
        assert_eq!(
            build(&d).unwrap(),
            Some(WriteIntent::CreateDenyRule {
                ruleset_href: SHOP.into(),
                draft: DenyRuleDraft {
                    description: None,
                    enabled: true,
                    providers: vec![ActorRef::all()],
                    consumers: vec![ActorRef::new(
                        ActorKind::IpList,
                        "/orgs/5/sec_policy/draft/ip_lists/21"
                    )],
                    services: vec![ServiceRef::Port(PortSpec::parse("23/tcp").unwrap())],
                    override_: true,
                },
            })
        );
    }

    #[test]
    fn rule_edit_patches_only_what_changed() {
        let corp = rule_sets()[0].rules[1].clone(); // role=web ← Corp, 443/tcp + 8000-8099/tcp
        let mut f = form(EditIntent::UpdateRule(corp, SHOP.into()));
        assert_eq!(f.title, "Edit allow rule 2 in ruleset 100");
        assert_eq!(f.value("Providers"), "role=web");
        assert_eq!(f.value("Consumers"), "iplist:Corp");
        assert_eq!(f.value("Services"), "443/tcp, 8000-8099/tcp");
        assert_eq!(f.value("Description"), "corp to web");
        assert_eq!(f.value("Unscoped"), "y");
        assert_eq!(build(&f).unwrap(), None, "a pre-filled form changes nothing");
        set(&mut f, "Consumers", "iplist:Corp, role=db");
        set(&mut f, "Services", "443/tcp");
        set(&mut f, "Unscoped", "n");
        assert_eq!(
            build(&f).unwrap(),
            Some(WriteIntent::UpdateRule {
                href: "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/2".into(),
                patch: RulePatch {
                    add_consumers: vec![ActorRef::new(ActorKind::Label, "/orgs/5/labels/2")],
                    remove_services: vec![ServiceRef::Port(
                        PortSpec::parse("8000-8099/tcp").unwrap()
                    )],
                    unscoped_consumers: Some(false),
                    ..RulePatch::default()
                },
            })
        );
        // Another spelling of a shown entry is neither added nor removed.
        let mut g = form(EditIntent::UpdateRule(
            rule_sets()[0].rules[0].clone(),
            SHOP.into(),
        ));
        assert_eq!(g.value("Consumers"), "role=web");
        set(&mut g, "Consumers", "label:role=web");
        assert_eq!(build(&g).unwrap(), None);
        let Start::Plan(w) = start(
            EditIntent::DeleteRule {
                href: "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1".into(),
            },
            None,
        ) else {
            panic!("a delete opens no form")
        };
        assert_eq!(
            w,
            WriteIntent::DeleteRule {
                href: "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1".into()
            }
        );
        assert_eq!(w.kind(), pce::write::ObjectKind::Rule);
    }

    #[test]
    fn inexpressible_entries_refuse_a_changed_field_only() {
        let mut rule = rule_sets()[0].rules[0].clone(); // web to db
        rule.providers.push(actor(
            json!({"virtual_service": {"href": "/orgs/5/sec_policy/draft/virtual_services/9"}}),
        ));
        rule.ingress_services
            .push(serde_json::from_value(json!({"proto": -1})).unwrap());
        let mut f = form(EditIntent::UpdateRule(rule, SHOP.into()));
        assert_eq!(
            f.value("Providers"),
            "role=db, /orgs/5/sec_policy/draft/virtual_services/9"
        );
        assert_eq!(f.value("Services"), "PostgreSQL, ?");
        assert_eq!(build(&f).unwrap(), None, "shown as it is, nothing changes");
        set(&mut f, "Description", "web to db, reviewed");
        let Some(WriteIntent::UpdateRule { patch, .. }) = build(&f).unwrap() else {
            panic!("not an update")
        };
        assert_eq!(
            patch,
            RulePatch {
                description: Some("web to db, reviewed".into()),
                ..RulePatch::default()
            },
            "the untouched lists are carried by the library as the PCE returned them"
        );
        set(&mut f, "Providers", "role=db");
        assert_eq!(
            build(&f).unwrap_err(),
            "Providers: this rule has entries the form cannot edit; use the CLI"
        );
        set(&mut f, "Providers", "role=db, /orgs/5/sec_policy/draft/virtual_services/9");
        set(&mut f, "Services", "PostgreSQL, ?, 443/tcp");
        assert_eq!(
            build(&f).unwrap_err(),
            "Services: this rule has entries the form cannot edit; use the CLI"
        );
    }

    #[test]
    fn deny_rule_edit_flips_override_and_state() {
        let deny = rule_sets()[0].deny_rules[1].clone(); // override, role=db ← all, 22/tcp
        let mut f = form(EditIntent::UpdateDenyRule(deny, SHOP.into()));
        assert_eq!(f.title, "Edit deny rule 8 in ruleset 100");
        assert_eq!(
            (
                f.value("Providers"),
                f.value("Consumers"),
                f.value("Services"),
                f.value("Override")
            ),
            ("role=db", "all", "22/tcp", "y")
        );
        assert_eq!(build(&f).unwrap(), None);
        set(&mut f, "Override", "n");
        set(&mut f, "Enabled", "n");
        assert_eq!(
            build(&f).unwrap(),
            Some(WriteIntent::UpdateDenyRule {
                href: "/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8".into(),
                patch: DenyRulePatch {
                    enabled: Some(false),
                    override_: Some(false),
                    ..DenyRulePatch::default()
                },
            })
        );
        let Start::Plan(w) = start(
            EditIntent::DeleteDenyRule {
                href: "/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8".into(),
            },
            None,
        ) else {
            panic!("a delete opens no form")
        };
        assert_eq!(w.kind(), pce::write::ObjectKind::DenyRule);
    }

    #[test]
    fn ruleset_names_resolve_to_one_live_draft_ruleset() {
        let mut sets = rule_sets();
        assert_eq!(ruleset_href_named(&sets, " shop-prod ").unwrap(), SHOP);
        assert_eq!(
            ruleset_href_named(&sets, "Shop").unwrap_err(),
            r#"no ruleset named "Shop"; similar: Shop-Prod"#
        );
        sets[0].update_type = Some("delete".into());
        assert_eq!(
            ruleset_href_named(&sets, "Shop-Prod").unwrap_err(),
            r#"no ruleset named "Shop-Prod""#,
            "a pending deletion does not count"
        );
        let mut twin = sets[1].clone();
        twin.href = "/orgs/5/sec_policy/draft/rule_sets/102".into();
        sets.push(twin);
        assert_eq!(
            ruleset_href_named(&sets, "Global-Infra").unwrap_err(),
            r#""Global-Infra" names several rulesets: /orgs/5/sec_policy/draft/rule_sets/101, /orgs/5/sec_policy/draft/rule_sets/102; use the href"#
        );
    }
```

Append to the tests in `crates/illumio-tui/src/tui/model.rs`:

```rust
    #[test]
    fn rules_are_found_by_href_inline_or_in_the_deny_fallback() {
        let mut p = PolicyState::new(60);
        p.raw = serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap();
        let (r, rs) = p
            .find_rule("/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/2")
            .unwrap();
        assert_eq!(r.description.as_deref(), Some("corp to web"));
        assert_eq!(rs, "/orgs/5/sec_policy/draft/rule_sets/100");
        let (d, _) = p
            .find_deny_rule("/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8")
            .unwrap();
        assert!(d.override_);
        assert!(
            p.find_rule("/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8")
                .is_none()
        );
        let fetched: DenyRule = serde_json::from_value(serde_json::json!({
            "href": "/orgs/5/sec_policy/draft/rule_sets/101/deny_rules/9",
            "providers": [{"actors": "ams"}], "consumers": [{"actors": "ams"}],
            "ingress_services": [{"port": 23, "proto": 6}]
        }))
        .unwrap();
        p.fallback_deny
            .insert("/orgs/5/sec_policy/draft/rule_sets/101".into(), vec![fetched]);
        let (_, rs) = p
            .find_deny_rule("/orgs/5/sec_policy/draft/rule_sets/101/deny_rules/9")
            .unwrap();
        assert_eq!(rs, "/orgs/5/sec_policy/draft/rule_sets/101");
    }
```

Append to the tests in `crates/illumio-tui/src/tui/update.rs`:

```rust
    #[test]
    fn rule_keys_open_prefilled_forms_and_x_plans_by_kind() {
        use crate::tui::edit::WriteIntent;
        use crate::tui::test_support::model_with_policy;
        let mut m = writes_on(model_with_policy());
        m.screen = Screen::Policy;
        update(&mut m, kc(KeyCode::Tab));
        assert_eq!(m.policy.pane, PolicyPane::Rules);
        update(&mut m, key('n'));
        assert!(
            matches!(&m.overlay, Overlay::Form(f) if f.title == "New allow rule in ruleset 100"),
            "{:?}",
            m.overlay
        );
        update(&mut m, kc(KeyCode::Esc));
        update(&mut m, key('N'));
        assert!(
            matches!(&m.overlay, Overlay::Form(f) if f.title == "New deny rule in ruleset 100"),
            "{:?}",
            m.overlay
        );
        update(&mut m, kc(KeyCode::Esc));
        // Row 0 is the override deny rule 8 (evaluation order).
        update(&mut m, key('e'));
        match &m.overlay {
            Overlay::Form(f) => {
                assert_eq!(f.title, "Edit deny rule 8 in ruleset 100");
                assert_eq!(f.value("Override"), "y");
            }
            other => panic!("{other:?}"),
        }
        update(&mut m, kc(KeyCode::Esc));
        assert_eq!(
            update(&mut m, key('x')),
            vec![Cmd::PlanWrite(WriteIntent::DeleteDenyRule {
                href: "/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8".into()
            })]
        );
        m.writes.in_flight = false;
        update(&mut m, key('j')); // allow rule 1
        update(&mut m, key('e'));
        match &m.overlay {
            Overlay::Form(f) => {
                assert_eq!(f.title, "Edit allow rule 1 in ruleset 100");
                assert_eq!(f.value("Services"), "PostgreSQL");
            }
            other => panic!("{other:?}"),
        }
        update(&mut m, kc(KeyCode::Esc));
        assert_eq!(
            update(&mut m, key('x')),
            vec![Cmd::PlanWrite(WriteIntent::DeleteRule {
                href: "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1".into()
            })]
        );
    }

    #[test]
    fn rules_pane_keys_go_through_the_same_refusals() {
        use crate::tui::test_support::model_with_policy;
        let mut m = model_with_policy();
        m.screen = Screen::Policy;
        update(&mut m, kc(KeyCode::Tab));
        for c in ['n', 'N', 'e', 'x'] {
            assert!(update(&mut m, key(c)).is_empty(), "{c}");
            assert_eq!(m.status.text, "started without --allow-writes");
            assert_eq!(m.overlay, Overlay::None);
        }
        let mut m = writes_on(model_with_policy());
        m.screen = Screen::Policy;
        m.policy.scope = PolicyScope::Active;
        update(&mut m, kc(KeyCode::Tab));
        for c in ['n', 'N', 'e', 'x'] {
            assert!(update(&mut m, key(c)).is_empty(), "{c}");
            assert_eq!(m.status.text, "switch to draft to edit policy");
            assert_eq!(m.overlay, Overlay::None);
        }
        // `N` means nothing on the rulesets pane.
        let mut m = writes_on(model_with_policy());
        m.screen = Screen::Policy;
        m.status = StatusLine::info("ready");
        assert!(update(&mut m, key('N')).is_empty());
        assert_eq!(m.overlay, Overlay::None);
        assert_eq!(m.status.text, "ready");
        // No rule rows: e and x have nothing to act on.
        update(&mut m, kc(KeyCode::Tab));
        m.policy.rules.set_rows(Vec::new());
        for c in ['e', 'x'] {
            assert!(update(&mut m, key(c)).is_empty(), "{c}");
            assert_eq!(m.status.text, "no row selected");
        }
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p illumio-tui --lib`
Expected: FAIL, compile errors `no variant named CreateRule found for enum EditIntent`, `cannot find type RulesetTarget`, `no method named find_rule found for struct PolicyState`.

- [ ] **Step 3: Implement**

`crates/illumio-tui/src/tui/edit.rs`. Imports (replace the block from Task 2):

```rust
use crate::tokens::{
    ActorToken, ServiceToken, actor_from_href_token, parse_scope_tokens, service_from_href_token,
    similar, split_kv, with_exclusion, with_similar,
};
use crate::tui::form::{Field, Form};
use pce::NameIndex;
use pce::models::{
    Actor, DenyRule, Fqdn, IngressService, Interface, IpList, Label, Rule, RuleSet, Service,
    Workload,
};
use pce::write::{
    ActorKind, ActorRef, DenyRuleDraft, DenyRulePatch, InterfaceSpec, IpListDraft, IpListPatch,
    IpRangeSpec, LabelDraft, PortSpec, RuleDraft, RulePatch, RuleSetDraft, RuleSetPatch,
    ServiceDraft, ServicePatch, ServiceRef, WorkloadDraft, WorkloadPatch, WriteRequest,
};
```

Insert before `pub enum EditIntent`:

```rust
/// Where a new allow rule goes: the selected ruleset (Policy) or the name
/// or href typed into a `w` form. A name costs one draft ruleset read at
/// plan time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RulesetTarget {
    Href(String),
    Name(String),
}
```

Add to `EditIntent` (after `DeleteRuleSet(RuleSet),`):

```rust
    CreateRule { ruleset_href: String },
    CreateDenyRule { ruleset_href: String },
    /// The rule as loaded and its ruleset's href.
    UpdateRule(Rule, String),
    UpdateDenyRule(DenyRule, String),
    DeleteRule { href: String },
    DeleteDenyRule { href: String },
```

Add to `WriteIntent` (after `DeleteRuleSet { href: String, name: String },`):

```rust
    CreateRule { ruleset: RulesetTarget, draft: RuleDraft },
    UpdateRule { href: String, patch: RulePatch },
    DeleteRule { href: String },
    CreateDenyRule { ruleset_href: String, draft: DenyRuleDraft },
    UpdateDenyRule { href: String, patch: DenyRulePatch },
    DeleteDenyRule { href: String },
```

Add to `WriteIntent::kind` (after the ruleset arm):

```rust
            WriteIntent::CreateRule { .. }
            | WriteIntent::UpdateRule { .. }
            | WriteIntent::DeleteRule { .. } => pce::write::ObjectKind::Rule,
            WriteIntent::CreateDenyRule { .. }
            | WriteIntent::UpdateDenyRule { .. }
            | WriteIntent::DeleteDenyRule { .. } => pce::write::ObjectKind::DenyRule,
```

Add to `start` (after the `DeleteRuleSet` arm):

```rust
        EditIntent::CreateRule { ruleset_href } => form(
            format!("New allow rule in ruleset {}", href_id(&ruleset_href)),
            rule_fields(None, idx),
            EditIntent::CreateRule { ruleset_href },
        ),
        EditIntent::CreateDenyRule { ruleset_href } => form(
            format!("New deny rule in ruleset {}", href_id(&ruleset_href)),
            deny_rule_fields(None, idx),
            EditIntent::CreateDenyRule { ruleset_href },
        ),
        EditIntent::UpdateRule(r, rs) => {
            let title = format!(
                "Edit allow rule {} in ruleset {}",
                href_id(&r.href),
                href_id(&rs)
            );
            let fields = rule_fields(Some(&r), idx);
            form(title, fields, EditIntent::UpdateRule(r, rs))
        }
        EditIntent::UpdateDenyRule(d, rs) => {
            let title = format!(
                "Edit deny rule {} in ruleset {}",
                href_id(&d.href),
                href_id(&rs)
            );
            let fields = deny_rule_fields(Some(&d), idx);
            form(title, fields, EditIntent::UpdateDenyRule(d, rs))
        }
        EditIntent::DeleteRule { href } => Start::Plan(WriteIntent::DeleteRule { href }),
        EditIntent::DeleteDenyRule { href } => Start::Plan(WriteIntent::DeleteDenyRule { href }),
```

Insert after `fn resolve_scopes` (Task 2):

```rust
const ACTORS_HINT: &str = "app=web, group:Name, iplist:Corp, all, !env=dev";
const SERVICES_HINT: &str = "443/tcp, 8000-8100/udp, icmp, HTTPS";
const UNSCOPED_HINT: &str = "y: consumers outside the ruleset's scope";
const OVERRIDE_HINT: &str = "y: evaluated before allow rules";
const CANNOT_EDIT_RULE: &str = "this rule has entries the form cannot edit; use the CLI";

/// The last segment of an href, for form titles.
fn href_id(href: &str) -> &str {
    href.rsplit('/').next().unwrap_or(href)
}

/// Providers, Consumers and Services, pre-filled from a rule's lists.
fn side_fields(
    sides: Option<(&[Actor], &[Actor], &[IngressService])>,
    idx: &NameIndex,
) -> Vec<Field> {
    let (p, c, s) = sides.map_or_else(
        || (String::new(), String::new(), String::new()),
        |(p, c, s)| {
            (
                actor_tokens(p, idx).join(", "),
                actor_tokens(c, idx).join(", "),
                service_tokens(s, idx).join(", "),
            )
        },
    );
    vec![
        Field::new("Providers", p, ACTORS_HINT, true),
        Field::new("Consumers", c, ACTORS_HINT, true),
        Field::new("Services", s, SERVICES_HINT, true),
    ]
}

fn rule_fields(r: Option<&Rule>, idx: &NameIndex) -> Vec<Field> {
    let mut fields = side_fields(
        r.map(|r| (&r.providers[..], &r.consumers[..], &r.ingress_services[..])),
        idx,
    );
    fields.extend([
        Field::new(
            "Description",
            r.and_then(|r| r.description.clone()).unwrap_or_default(),
            "optional",
            false,
        ),
        Field::new("Enabled", yn(r.is_none_or(|r| r.enabled)), YES_NO_HINT, true),
        Field::new(
            "Unscoped",
            yn(r.is_some_and(|r| r.unscoped_consumers.unwrap_or(false))),
            UNSCOPED_HINT,
            true,
        ),
    ]);
    fields
}

fn deny_rule_fields(d: Option<&DenyRule>, idx: &NameIndex) -> Vec<Field> {
    let mut fields = side_fields(
        d.map(|d| (&d.providers[..], &d.consumers[..], &d.ingress_services[..])),
        idx,
    );
    fields.extend([
        Field::new(
            "Description",
            d.and_then(|d| d.description.clone()).unwrap_or_default(),
            "optional",
            false,
        ),
        Field::new("Enabled", yn(d.is_none_or(|d| d.enabled)), YES_NO_HINT, true),
        Field::new(
            "Override",
            yn(d.is_some_and(|d| d.override_)),
            OVERRIDE_HINT,
            true,
        ),
    ]);
    fields
}

/// A Providers or Consumers field to actors.
fn actor_field(
    form: &Form,
    label: &str,
    idx: &NameIndex,
    org_id: u64,
) -> Result<Vec<ActorRef>, String> {
    let tokens = items(form.value(label))
        .iter()
        .map(|t| ActorToken::parse(t.as_str()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("{label}: {e}"))?;
    resolve_actor_tokens(&tokens, idx, org_id).map_err(|e| format!("{label}: {e}"))
}

/// The Services field to rule services.
fn service_field(form: &Form, idx: &NameIndex, org_id: u64) -> Result<Vec<ServiceRef>, String> {
    let tokens = items(form.value("Services"))
        .iter()
        .map(|t| ServiceToken::parse(t.as_str()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Services: {e}"))?;
    resolve_service_tokens(&tokens, idx, org_id).map_err(|e| format!("Services: {e}"))
}

/// The new allow rule a Policy `n` form (or a `w` form) describes.
fn rule_draft(form: &Form, idx: &NameIndex, org_id: u64) -> Result<RuleDraft, String> {
    let d = RuleDraft {
        description: non_empty(form.value("Description").trim().to_string()),
        enabled: yes_no(form, "Enabled")?,
        providers: actor_field(form, "Providers", idx, org_id)?,
        consumers: actor_field(form, "Consumers", idx, org_id)?,
        services: service_field(form, idx, org_id)?,
        unscoped_consumers: yes_no(form, "Unscoped")?,
        resolve_labels_as: None,
    };
    d.validate().map_err(|e| e.join("; "))?;
    Ok(d)
}

fn deny_rule_draft(form: &Form, idx: &NameIndex, org_id: u64) -> Result<DenyRuleDraft, String> {
    let d = DenyRuleDraft {
        description: non_empty(form.value("Description").trim().to_string()),
        enabled: yes_no(form, "Enabled")?,
        providers: actor_field(form, "Providers", idx, org_id)?,
        consumers: actor_field(form, "Consumers", idx, org_id)?,
        services: service_field(form, idx, org_id)?,
        override_: yes_no(form, "Override")?,
    };
    d.validate().map_err(|e| e.join("; "))?;
    Ok(d)
}

fn actor_editable(a: &Actor) -> bool {
    ActorRef::from_actor(a).is_ok()
}

fn service_editable(s: &IngressService) -> bool {
    match ServiceRef::from_ingress(s) {
        ServiceRef::Href(_) => true,
        ServiceRef::Port(p) => port_token(&p).is_some(),
    }
}

/// What an edited Providers or Consumers field adds and removes, matched
/// against the shown tokens first: a token still there keeps the PCE's
/// entry, a token gone removes exactly that entry, a new token is resolved
/// through the index. A token retyped in another spelling of a shown
/// entry is neither added nor removed. A field holding an entry the
/// grammar cannot express may not change.
fn actor_changes(
    label: &str,
    current: &[Actor],
    text: &str,
    idx: &NameIndex,
    org_id: u64,
) -> Result<(Vec<ActorRef>, Vec<ActorRef>), String> {
    let shown = actor_tokens(current, idx);
    let typed = items(text);
    if typed == shown {
        return Ok((Vec::new(), Vec::new()));
    }
    if !current.iter().all(actor_editable) {
        return Err(format!("{label}: {CANNOT_EDIT_RULE}"));
    }
    let mut remove = Vec::new();
    for (a, t) in current.iter().zip(&shown) {
        if !typed.contains(t) {
            remove.push(ActorRef::from_actor(a).map_err(|e| format!("{label}: {e}"))?);
        }
    }
    let new = typed
        .iter()
        .filter(|t| !shown.contains(t))
        .map(|t| ActorToken::parse(t.as_str()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("{label}: {e}"))?;
    let mut add = resolve_actor_tokens(&new, idx, org_id).map_err(|e| format!("{label}: {e}"))?;
    add.retain(|a| match remove.iter().position(|r| r.same_actor(a)) {
        Some(i) => {
            remove.remove(i);
            false
        }
        None => true,
    });
    Ok((add, remove))
}

/// [`actor_changes`] for the Services field.
fn service_changes(
    current: &[IngressService],
    text: &str,
    idx: &NameIndex,
    org_id: u64,
) -> Result<(Vec<ServiceRef>, Vec<ServiceRef>), String> {
    let shown = service_tokens(current, idx);
    let typed = items(text);
    if typed == shown {
        return Ok((Vec::new(), Vec::new()));
    }
    if !current.iter().all(service_editable) {
        return Err(format!("Services: {CANNOT_EDIT_RULE}"));
    }
    let mut remove: Vec<ServiceRef> = current
        .iter()
        .zip(&shown)
        .filter(|(_, t)| !typed.contains(t))
        .map(|(s, _)| ServiceRef::from_ingress(s))
        .collect();
    let new = typed
        .iter()
        .filter(|t| !shown.contains(t))
        .map(|t| ServiceToken::parse(t.as_str()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Services: {e}"))?;
    let mut add =
        resolve_service_tokens(&new, idx, org_id).map_err(|e| format!("Services: {e}"))?;
    add.retain(|a| match remove.iter().position(|r| r.same_service(a)) {
        Some(i) => {
            remove.remove(i);
            false
        }
        None => true,
    });
    Ok((add, remove))
}

/// The one draft ruleset named `name` (any case; a pending deletion does
/// not count), for a rule typed into a named ruleset.
pub fn ruleset_href_named(sets: &[RuleSet], name: &str) -> Result<String, String> {
    let name = name.trim();
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
        [] => Err(with_similar(
            format!("no ruleset named {name:?}"),
            &similar(name, live().map(|rs| rs.name.as_str())),
        )),
        many => Err(format!(
            "{name:?} names several rulesets: {}; use the href",
            many.join(", ")
        )),
    }
}
```

Add to `build_intent` (before the delete arm):

```rust
        EditIntent::CreateRule { ruleset_href } => {
            let draft = rule_draft(form, need_index(index)?, org_id)?;
            Ok(Some(WriteIntent::CreateRule {
                ruleset: RulesetTarget::Href(ruleset_href.clone()),
                draft,
            }))
        }
        EditIntent::CreateDenyRule { ruleset_href } => {
            let draft = deny_rule_draft(form, need_index(index)?, org_id)?;
            Ok(Some(WriteIntent::CreateDenyRule {
                ruleset_href: ruleset_href.clone(),
                draft,
            }))
        }
        EditIntent::UpdateRule(r, _) => {
            let idx = need_index(index)?;
            let (add_providers, remove_providers) =
                actor_changes("Providers", &r.providers, &v("Providers"), idx, org_id)?;
            let (add_consumers, remove_consumers) =
                actor_changes("Consumers", &r.consumers, &v("Consumers"), idx, org_id)?;
            let (add_services, remove_services) =
                service_changes(&r.ingress_services, &v("Services"), idx, org_id)?;
            let enabled = yes_no(form, "Enabled")?;
            let unscoped = yes_no(form, "Unscoped")?;
            let patch = RulePatch {
                description: changed(r.description.as_deref().unwrap_or(""), v("Description")),
                enabled: (enabled != r.enabled).then_some(enabled),
                add_providers,
                remove_providers,
                add_consumers,
                remove_consumers,
                add_services,
                remove_services,
                unscoped_consumers: (unscoped != r.unscoped_consumers.unwrap_or(false))
                    .then_some(unscoped),
                resolve_labels_as: None,
            };
            if patch.is_empty() {
                Ok(None)
            } else {
                Ok(Some(WriteIntent::UpdateRule {
                    href: r.href.clone(),
                    patch,
                }))
            }
        }
        EditIntent::UpdateDenyRule(d, _) => {
            let idx = need_index(index)?;
            let (add_providers, remove_providers) =
                actor_changes("Providers", &d.providers, &v("Providers"), idx, org_id)?;
            let (add_consumers, remove_consumers) =
                actor_changes("Consumers", &d.consumers, &v("Consumers"), idx, org_id)?;
            let (add_services, remove_services) =
                service_changes(&d.ingress_services, &v("Services"), idx, org_id)?;
            let enabled = yes_no(form, "Enabled")?;
            let override_ = yes_no(form, "Override")?;
            let patch = DenyRulePatch {
                description: changed(d.description.as_deref().unwrap_or(""), v("Description")),
                enabled: (enabled != d.enabled).then_some(enabled),
                add_providers,
                remove_providers,
                add_consumers,
                remove_consumers,
                add_services,
                remove_services,
                override_: (override_ != d.override_).then_some(override_),
            };
            if patch.is_empty() {
                Ok(None)
            } else {
                Ok(Some(WriteIntent::UpdateDenyRule {
                    href: d.href.clone(),
                    patch,
                }))
            }
        }
```

The delete arm becomes:

```rust
        // Deletes never open a form (`start`); a form never carries one.
        EditIntent::DeleteLabel(_)
        | EditIntent::DeleteIpList(_)
        | EditIntent::DeleteService(_)
        | EditIntent::DeleteWorkload(_)
        | EditIntent::DeleteRuleSet(_)
        | EditIntent::DeleteRule { .. }
        | EditIntent::DeleteDenyRule { .. } => Err("a delete has no form".into()),
```

`crates/illumio-tui/src/tui/model.rs`. Imports:

```rust
use pce::models::{
    DenyRule, Event, EventFilter, IpList, Label, LabelGroup, PendingChange, PolicyScope, Rule,
    RuleSet, Service, VenHealth, Workload,
};
```

Add after `pub fn selected_rule_set`:

```rust
    /// An allow rule by href, with its ruleset's href.
    pub fn find_rule(&self, href: &str) -> Option<(Rule, String)> {
        self.raw.iter().find_map(|rs| {
            rs.rules
                .iter()
                .find(|r| r.href == href)
                .map(|r| (r.clone(), rs.href.clone()))
        })
    }

    /// A deny rule by href (inline, or fetched from the fallback endpoint),
    /// with its ruleset's href.
    pub fn find_deny_rule(&self, href: &str) -> Option<(DenyRule, String)> {
        self.raw
            .iter()
            .find_map(|rs| {
                rs.deny_rules
                    .iter()
                    .find(|d| d.href == href)
                    .map(|d| (d.clone(), rs.href.clone()))
            })
            .or_else(|| {
                self.fallback_deny.iter().find_map(|(rs, list)| {
                    list.iter()
                        .find(|d| d.href == href)
                        .map(|d| (d.clone(), rs.clone()))
                })
            })
    }
```

`crates/illumio-tui/src/tui/update.rs`. Imports: change `use pce::{CacheMode, WriteMethod};` to:

```rust
use pce::{CacheMode, RuleKind, WriteMethod};
```

In `policy_key`, replace the Task 2 arm with:

```rust
            KeyCode::Char(c @ ('n' | 'e' | 'x')) => return policy_edit_key(model, c),
            // `N` (a deny rule) is bound on the rules pane only.
            KeyCode::Char('N') if model.policy.pane == PolicyPane::Rules => {
                return policy_edit_key(model, 'N');
            }
```

In `policy_edit_key`, the rules pane now has intents:

```rust
    let intent = match model.policy.pane {
        PolicyPane::RuleSets => ruleset_intent(&model.policy, c),
        PolicyPane::Rules => rule_intent(&model.policy, c),
    };
```

Insert after `fn ruleset_intent`:

```rust
/// `n` / `N` a new allow / deny rule in the selected ruleset; `e` / `x` the
/// selected rule, allow or deny by its kind, found by href in what was loaded.
fn rule_intent(p: &PolicyState, c: char) -> Option<EditIntent> {
    let ruleset_href = p.selected_href()?;
    match c {
        'n' => return Some(EditIntent::CreateRule { ruleset_href }),
        'N' => return Some(EditIntent::CreateDenyRule { ruleset_href }),
        _ => {}
    }
    let view = p.rules.selected_item()?;
    let href = view.href.clone();
    match (c, view.kind == RuleKind::Allow) {
        ('x', true) => Some(EditIntent::DeleteRule { href }),
        ('x', false) => Some(EditIntent::DeleteDenyRule { href }),
        (_, true) => p
            .find_rule(&href)
            .map(|(r, rs)| EditIntent::UpdateRule(r, rs)),
        (_, false) => p
            .find_deny_rule(&href)
            .map(|(d, rs)| EditIntent::UpdateDenyRule(d, rs)),
    }
}
```

`crates/illumio-tui/src/tui/app.rs`. Imports:

```rust
use crate::tui::edit::{self, RulesetTarget, WriteIntent};
```

```rust
use pce::models::{PendingChange, PolicyScope, WorkloadFilter};
```

In `plan_intent`, add after the `DeleteRuleSet` arm, and extend its doc comment with "a ruleset typed by name costs one draft ruleset read (through the cache) before the rule is planned":

```rust
        WriteIntent::CreateRule { ruleset, draft } => {
            let href = match ruleset {
                RulesetTarget::Href(h) => h,
                // The only read a form causes: the draft rulesets, through the cache.
                RulesetTarget::Name(n) => {
                    let sets = pce
                        .rule_sets(PolicyScope::Draft, CacheMode::Use)
                        .await?
                        .value;
                    edit::ruleset_href_named(&sets, &n).map_err(WriteError::invalid)?
                }
            };
            w.plan_create_rule(&href, &draft)?
        }
        WriteIntent::UpdateRule { href, patch } => w.plan_update_rule(&href, &patch).await?,
        WriteIntent::DeleteRule { href } => w.plan_delete_rule(&href)?,
        WriteIntent::CreateDenyRule {
            ruleset_href,
            draft,
        } => w.plan_create_deny_rule(&ruleset_href, &draft)?,
        WriteIntent::UpdateDenyRule { href, patch } => {
            w.plan_update_deny_rule(&href, &patch).await?
        }
        WriteIntent::DeleteDenyRule { href } => w.plan_delete_deny_rule(&href)?,
```

`crates/illumio-tui/src/tui/ui/widgets/overlay.rs`: replace `screen_hint` with:

```rust
/// Key hints for the status line. With writes on, Objects, Workloads and
/// Policy lead with the edit keys and drop their least used hints so the
/// status text keeps room at 80 columns.
pub fn screen_hint(screen: Screen, writes: bool) -> &'static str {
    match (screen, writes) {
        (Screen::Objects, true) => "n:new e:edit x:del  Tab type  Enter detail",
        (Screen::Workloads, true) => "n:new e:edit x:del  t/T traffic  Enter detail",
        (Screen::Policy, true) => "n/N:new e:edit x:del  d scope  p pending",
        (Screen::Traffic, _) => "f filter  r run  a auto  v view  s sort  / search",
        (Screen::Policy, _) => "d scope  p pending  Enter detail  Tab pane  a auto",
        (Screen::Objects, false) => "Tab type  Enter detail  r refresh  a auto  / search",
        (Screen::Workloads, false) => "t/T traffic  h/H health  Enter detail  r refresh",
        (Screen::Events, _) => "f filter  r refresh  a auto  S severity  Enter detail",
    }
}
```

- [ ] **Step 4: Run the unit tests**

Run: `cargo fmt --all && cargo test -p illumio-tui --lib`
Expected: PASS (five new `edit` tests, one `model` test, two `update` tests, all existing ones).

- [ ] **Step 5: Write the snapshot and runtime tests**

Append to `crates/illumio-tui/tests/tui_snapshots.rs`:

```rust
#[test]
fn rule_and_deny_rule_forms_80x24() {
    use illumio_tui::tui::edit::{EditIntent, form_for};
    use illumio_tui::tui::model::{Overlay, Screen};
    use illumio_tui::tui::test_support::fixture_index;
    let idx = fixture_index();
    let mut m = model_with_policy();
    m.screen = Screen::Policy;
    let shop = m.policy.raw[0].clone();
    m.overlay = Overlay::Form(
        form_for(
            EditIntent::UpdateRule(shop.rules[1].clone(), shop.href.clone()),
            Some(idx.as_ref()),
        )
        .expect("a form"),
    );
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("Edit allow rule 2 in ruleset 100"), "{out}");
    assert!(out.contains("iplist:Corp"), "{out}");
    assert!(out.contains("443/tcp, 8000-8099/tcp"), "{out}");
    assert!(out.contains("Unscoped*"), "{out}");
    insta::assert_snapshot!("form_rule_80x24", out);
    m.set_theme("light");
    insta::assert_snapshot!("form_rule_light_styled_80x24", render_styled(&m, 80, 24));
    let mut d = model_with_policy();
    d.screen = Screen::Policy;
    d.overlay = Overlay::Form(
        form_for(
            EditIntent::UpdateDenyRule(shop.deny_rules[1].clone(), shop.href.clone()),
            Some(idx.as_ref()),
        )
        .expect("a form"),
    );
    let out = render_to_string(&d, 80, 24);
    assert!(out.contains("Edit deny rule 8 in ruleset 100"), "{out}");
    assert!(out.contains("Override*"), "{out}");
    insta::assert_snapshot!("form_deny_rule_80x24", out);
    d.set_theme("light");
    insta::assert_snapshot!(
        "form_deny_rule_light_styled_80x24",
        render_styled(&d, 80, 24)
    );
}
```

Append to `crates/illumio-tui/tests/tui_runtime.rs`:

```rust
#[tokio::test]
async fn create_rule_from_policy_posts_once_and_reloads_once() {
    let server = MockServer::start().await;
    mount_objects(&server).await;
    mount_rule_sets(&server, 2).await;
    let body = json!({
        "enabled": true,
        "providers": [{"label": {"href": "/orgs/5/labels/2"}}],
        "consumers": [{"label": {"href": "/orgs/5/labels/1"}}],
        "ingress_services": [{"port": 443, "proto": 6}],
        "unscoped_consumers": false,
        "resolve_labels_as": {"providers": ["workloads"], "consumers": ["workloads"]}
    });
    let mut created = body.clone();
    created["href"] = json!("/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/9");
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets/100/sec_rules"))
        .and(body_json(body))
        .respond_with(ResponseTemplate::new(201).set_body_json(created))
        .expect(1)
        .mount(&server)
        .await;
    let audit = MemoryAudit::new();
    let mut keys = vec![(500, ch('2')), (300, KeyCode::Tab), (50, ch('n'))];
    keys.extend(typed("role=db"));
    keys.push((20, KeyCode::Tab));
    keys.extend(typed("role=web"));
    keys.push((20, KeyCode::Tab));
    keys.extend(typed("443/tcp"));
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
        "created /orgs/5/sec_policy/draft/rule_sets/100/sec_rules/9 (pending)"
    );
    assert_eq!(
        model.stats.calls, 8,
        "health + four lists + rulesets + POST + one rulesets reload"
    );
    assert_eq!(writes_seen(&server).await, vec!["POST"]);
    assert_eq!(audit.entries().len(), 1);
    assert_eq!(model.busy, 0);
}

#[tokio::test]
async fn edit_rule_puts_only_the_changed_services() {
    let server = MockServer::start().await;
    mount_objects(&server).await;
    mount_rule_sets(&server, 2).await;
    let rule = json!({
        "href": "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1", "enabled": true,
        "description": "web to db",
        "providers": [{"label": {"href": "/orgs/5/labels/2"}}],
        "consumers": [{"label": {"href": "/orgs/5/labels/1"}}],
        "ingress_services": [{"href": "/orgs/5/sec_policy/draft/services/31"}],
        "resolve_labels_as": {"providers": ["workloads"], "consumers": ["workloads"]},
        "unscoped_consumers": false, "sec_connect": false, "stateless": false,
        "machine_auth": false, "network_type": "brn", "update_type": null
    });
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(rule.clone()))
        .expect(1)
        .mount(&server)
        .await;
    let mut updated = rule;
    updated["ingress_services"] = json!([
        {"href": "/orgs/5/sec_policy/draft/services/31"},
        {"port": 443, "proto": 6}
    ]);
    Mock::given(method("PUT"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1"))
        .and(body_json(json!({"ingress_services": [
            {"href": "/orgs/5/sec_policy/draft/services/31"},
            {"port": 443, "proto": 6}
        ]})))
        .respond_with(ResponseTemplate::new(200).set_body_json(updated))
        .expect(1)
        .mount(&server)
        .await;
    let audit = MemoryAudit::new();
    let mut keys = vec![
        (500, ch('2')),
        (300, KeyCode::Tab),
        (50, ch('j')), // allow rule 1 (the override deny rule sorts first)
        (50, ch('e')),
        (20, KeyCode::Tab),
        (20, KeyCode::Tab),
    ];
    keys.extend(typed(", 443/tcp"));
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
        "updated /orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1 (pending)"
    );
    assert_eq!(
        model.stats.calls, 9,
        "health + four lists + rulesets + the plan's rule GET + PUT + one rulesets reload"
    );
    assert_eq!(writes_seen(&server).await, vec!["PUT"]);
    assert_eq!(audit.entries().len(), 1);
    assert_eq!(model.busy, 0);
}

#[tokio::test]
async fn delete_deny_rule_sends_one_delete() {
    let server = MockServer::start().await;
    mount_objects(&server).await;
    mount_rule_sets(&server, 2).await;
    Mock::given(method("DELETE"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let audit = MemoryAudit::new();
    let keys = vec![
        (500, ch('2')),
        (300, KeyCode::Tab),
        (50, ch('x')), // row 0: the override deny rule 8
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
    assert_eq!(
        model.status.text,
        "deleted /orgs/5/sec_policy/draft/rule_sets/100/deny_rules/8 (pending)"
    );
    assert_eq!(
        model.stats.calls, 8,
        "health + four lists + rulesets + DELETE + one rulesets reload"
    );
    assert_eq!(writes_seen(&server).await, vec!["DELETE"]);
    assert_eq!(audit.entries()[0].method, pce::WriteMethod::Delete);
    assert_eq!(model.busy, 0);
}
```

- [ ] **Step 6: Create and read the snapshots, run everything**

Run: `INSTA_UPDATE=always cargo test -p illumio-tui --test tui_snapshots rule_and_deny_rule_forms && git status --short crates/illumio-tui/tests/snapshots`
Expected: exactly four new files: `tui_snapshots__form_rule_80x24.snap`, `tui_snapshots__form_rule_light_styled_80x24.snap`, `tui_snapshots__form_deny_rule_80x24.snap`, `tui_snapshots__form_deny_rule_light_styled_80x24.snap`. Read them: six fields each, `Unscoped*` (allow) and `Override*` (deny) last, tokens pre-filled (`role=web`, `iplist:Corp`, `443/tcp, 8000-8099/tcp`; `role=db`, `all`, `22/tcp`). The status hint is the writes-off Policy hint (the snapshot models have writes off).

Run: `cargo test -p illumio-tui`
Expected: PASS, including the three new runtime tests.

- [ ] **Step 7: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p illumio-tui
git add crates/illumio-tui/src/tui/edit.rs crates/illumio-tui/src/tui/model.rs crates/illumio-tui/src/tui/update.rs crates/illumio-tui/src/tui/app.rs crates/illumio-tui/src/tui/ui/widgets/overlay.rs crates/illumio-tui/tests/tui_snapshots.rs crates/illumio-tui/tests/tui_runtime.rs crates/illumio-tui/tests/snapshots/tui_snapshots__form_rule_80x24.snap crates/illumio-tui/tests/snapshots/tui_snapshots__form_rule_light_styled_80x24.snap crates/illumio-tui/tests/snapshots/tui_snapshots__form_deny_rule_80x24.snap crates/illumio-tui/tests/snapshots/tui_snapshots__form_deny_rule_light_styled_80x24.snap
git commit -m "feat(tui): create, edit and delete draft allow and deny rules on the Policy screen

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 4: Pending marks and provisioning

**Files:**
- Modify: `crates/illumio-tui/src/tui/model.rs` (imports line 14; `PolicyState` fields and `new`)
- Modify: `crates/illumio-tui/src/tui/edit.rs` (imports; `WriteIntent::Provision`, `kind()`)
- Modify: `crates/illumio-tui/src/tui/update.rs` (imports lines 3, 15; `Msg::PendingLoaded` ~line 429; `Msg::WriteDone` ~line 454; `Msg::Planned` ~line 511; the `ConfirmWrite` `y` arm ~line 857; the `Overlay::Pending` arm ~line 890; `reload_after_write`; new `provision_key` and `counts_text` after `pending_diff`; tests)
- Modify: `crates/illumio-tui/src/tui/app.rs` (`plan_intent`)
- Modify: `crates/illumio-tui/src/tui/ui/widgets/overlay.rs` (`render_pending` ~line 509)
- Modify: `crates/illumio-tui/src/tui/test_support.rs` (new `provision_plan` after `ip_list_delete_plan`)
- Test: `crates/illumio-tui/tests/tui_snapshots.rs`, `crates/illumio-tui/tests/tui_runtime.rs` (append); changed snapshot `tui_snapshots__policy_pending_80x24.snap`

**Interfaces:**
- Consumes: `ProvisionRequest { description, subset }`, `ProvisionSubset::{All, Hrefs}`, `provision_counts(&Value) -> BTreeMap<String, usize>` (`pce/src/write/provision.rs` ~lines 13-43), `Writer::plan_provision(&ProvisionRequest)` (~line 121: one pending GET past the cache; `Plan::Noop { reason: "nothing pending" }` when empty), `ObjectKind::Provision` (prefixes `policy:` and `objects:`), Task 2 `quiet_reload`, `load_rule_sets`, `writes_refused`, `written` (update tests), `mount_rule_sets` (Task 2).
- Produces:

```rust
// tui/model.rs
pub struct PolicyState { /* … */ pub pending_marked: BTreeSet<String>, pub provision_counts: Option<String>, pub provisioned: Option<String> }
// tui/edit.rs
pub enum WriteIntent { /* … */ Provision(ProvisionRequest) }
// tui/update.rs (private)
fn provision_key(model: &mut Model) -> Vec<Cmd>;
fn counts_text(body: Option<&serde_json::Value>) -> String;   // "ip_lists=1, rule_sets=1"
// tui/test_support.rs
pub fn provision_plan() -> pce::write::Plan;
```

- [ ] **Step 1: Write the failing tests**

Add to `crates/illumio-tui/src/tui/test_support.rs` after `ip_list_delete_plan` (test support, compiled always):

```rust
/// The plan `P` shows for the two fixture pending entries (a ruleset and an
/// IP list), as `Writer::plan_provision` builds it.
pub fn provision_plan() -> pce::write::Plan {
    pce::write::Plan::Send {
        request: pce::write::WriteRequest {
            method: pce::WriteMethod::Post,
            path: "/orgs/5/sec_policy".into(),
            body: Some(serde_json::json!({
                "update_description": "illumio-tui provision",
                "change_subset": {
                    "ip_lists": [{"href": "/orgs/5/sec_policy/draft/ip_lists/21"}],
                    "rule_sets": [{"href": "/orgs/5/sec_policy/draft/rule_sets/101"}]
                }
            })),
            summary: "provision 1 ip_list, 1 rule_set".into(),
        },
        kind: pce::write::ObjectKind::Provision,
    }
}
```

Append to the tests in `crates/illumio-tui/src/tui/update.rs`:

```rust
    fn pending_fixture() -> Vec<pce::models::PendingChange> {
        pce::models::parse_pending(
            serde_json::from_str(include_str!("../../../../fixtures/pending_list.json")).unwrap(),
        )
    }

    #[test]
    fn space_marks_entries_and_p_plans_the_marked_or_all() {
        use crate::tui::edit::WriteIntent;
        use crate::tui::test_support::model_with_policy;
        let provision = |subset| {
            vec![Cmd::PlanWrite(WriteIntent::Provision(ProvisionRequest {
                description: Some("illumio-tui provision".into()),
                subset,
            }))]
        };
        let mut m = writes_on(model_with_policy());
        m.screen = Screen::Policy;
        update(&mut m, Msg::PendingLoaded(Ok(pending_fixture())));
        assert_eq!(m.overlay, Overlay::Pending);
        update(&mut m, key(' '));
        assert!(
            m.policy
                .pending_marked
                .contains("/orgs/5/sec_policy/draft/rule_sets/101")
        );
        update(&mut m, key(' '));
        assert!(m.policy.pending_marked.is_empty(), "Space toggles");
        update(&mut m, key('j'));
        update(&mut m, key(' '));
        assert_eq!(
            update(&mut m, key('P')),
            provision(ProvisionSubset::Hrefs(vec![
                "/orgs/5/sec_policy/draft/ip_lists/21".into()
            ]))
        );
        assert_eq!(
            m.overlay,
            Overlay::None,
            "the confirm overlay opens on the plan's answer"
        );
        assert!(m.writes.in_flight);
        m.writes.in_flight = false;
        update(&mut m, Msg::PendingLoaded(Ok(pending_fixture())));
        assert!(m.policy.pending_marked.is_empty(), "a reload clears the marks");
        assert_eq!(
            update(&mut m, key('P')),
            provision(ProvisionSubset::All),
            "no marks: everything pending"
        );
        m.writes.in_flight = false;
        update(&mut m, Msg::PendingLoaded(Ok(vec![])));
        assert!(update(&mut m, key('P')).is_empty());
        assert_eq!(m.status.text, "nothing pending");
        let mut off = model_with_policy();
        off.screen = Screen::Policy;
        update(&mut off, Msg::PendingLoaded(Ok(pending_fixture())));
        assert!(update(&mut off, key('P')).is_empty());
        assert_eq!(off.status.text, "started without --allow-writes");
        assert_eq!(off.overlay, Overlay::Pending);
    }

    #[test]
    fn provision_confirm_reports_the_counts_and_reloads_pending_and_rulesets() {
        use crate::tui::test_support::{model_with_policy, provision_plan};
        use pce::write::Outcome;
        let mut m = writes_on(model_with_policy());
        m.screen = Screen::Policy;
        update(
            &mut m,
            Msg::Planned(Ok((provision_plan(), pce::write::ObjectKind::Provision))),
        );
        assert_eq!(
            update(&mut m, key('y')),
            vec![Cmd::ExecuteWrite {
                plan: provision_plan(),
                kind: pce::write::ObjectKind::Provision
            }]
        );
        assert_eq!(
            m.policy.provision_counts.as_deref(),
            Some("ip_lists=1, rule_sets=1")
        );
        let cmds = update(
            &mut m,
            Msg::WriteDone {
                kind: pce::write::ObjectKind::Provision,
                method: Some(pce::WriteMethod::Post),
                result: Ok(Outcome::Written(written("/orgs/5/sec_policy/7", false))),
            },
        );
        assert_eq!(
            cmds,
            vec![
                Cmd::LoadRuleSets {
                    scope: PolicyScope::Draft,
                    mode: pce::CacheMode::Bypass
                },
                Cmd::LoadPending(pce::CacheMode::Bypass),
            ]
        );
        let report = "provisioned /orgs/5/sec_policy/7: ip_lists=1, rule_sets=1";
        assert_eq!(m.status.text, report);
        assert!(m.policy.provision_counts.is_none());
        let sets = m.policy.raw.clone();
        update(
            &mut m,
            Msg::RuleSetsLoaded {
                scope: PolicyScope::Draft,
                result: Ok((sets, 0)),
            },
        );
        assert_eq!(m.status.text, report, "the rulesets reload keeps the report");
        update(&mut m, Msg::PendingLoaded(Ok(vec![])));
        assert_eq!(m.status.text, format!("{report}; 0 pending changes left"));
        assert_eq!(m.overlay, Overlay::Pending);
    }

    #[test]
    fn nothing_to_provision_is_an_info_and_a_pending_reload_never_hides_an_error() {
        let mut m = writes_on(model_fixture());
        update(
            &mut m,
            Msg::Planned(Ok((
                pce::write::Plan::Noop {
                    existing_href: "/orgs/5/sec_policy/pending".into(),
                    reason: "nothing pending".into(),
                },
                pce::write::ObjectKind::Provision,
            ))),
        );
        assert_eq!(m.status.kind, StatusKind::Info);
        assert_eq!(m.status.text, "nothing pending");
        assert_eq!(m.overlay, Overlay::None);
        m.overlay = Overlay::Error("write failed: 406".into());
        update(&mut m, Msg::PendingLoaded(Ok(vec![])));
        assert_eq!(
            m.overlay,
            Overlay::Error("write failed: 406".into()),
            "the reload does not replace the error"
        );
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p illumio-tui --lib`
Expected: FAIL, compile errors `no variant named Provision found for enum WriteIntent`, `no field pending_marked on type PolicyState`, `cannot find struct ProvisionRequest`.

- [ ] **Step 3: Implement**

`crates/illumio-tui/src/tui/model.rs`: change `use std::collections::{HashMap, HashSet};` to:

```rust
use std::collections::{BTreeSet, HashMap, HashSet};
```

add to `PolicyState` (after `pub quiet_reload: bool,`):

```rust
    /// Pending entries marked with `Space` for `P` (hrefs); cleared when the
    /// list is loaded again.
    pub pending_marked: BTreeSet<String>,
    /// The counts of the provision just sent (`ip_lists=1, rule_sets=2`),
    /// taken from its body at `y`: the PCE's answer does not repeat them.
    pub provision_counts: Option<String>,
    /// The report of the provision just done; the pending reload it caused
    /// shows it in front of the new count.
    pub provisioned: Option<String>,
```

and initialise them in `PolicyState::new` (after `quiet_reload: false,`):

```rust
            pending_marked: BTreeSet::new(),
            provision_counts: None,
            provisioned: None,
```

`crates/illumio-tui/src/tui/edit.rs`: add `ProvisionRequest` to the `pce::write` import:

```rust
use pce::write::{
    ActorKind, ActorRef, DenyRuleDraft, DenyRulePatch, InterfaceSpec, IpListDraft, IpListPatch,
    IpRangeSpec, LabelDraft, PortSpec, ProvisionRequest, RuleDraft, RulePatch, RuleSetDraft,
    RuleSetPatch, ServiceDraft, ServicePatch, ServiceRef, WorkloadDraft, WorkloadPatch,
    WriteRequest,
};
```

add to `WriteIntent` (after `DeleteDenyRule { href: String },`):

```rust
    /// `P` in the pending overlay: the marked entries, or everything.
    Provision(ProvisionRequest),
```

and to `WriteIntent::kind` (after the deny-rule arm):

```rust
            WriteIntent::Provision(_) => pce::write::ObjectKind::Provision,
```

`crates/illumio-tui/src/tui/update.rs`. Imports:

```rust
use crate::tui::edit::{self, EditIntent, Start, WriteIntent};
```

```rust
use pce::write::{
    Outcome, Plan, ProvisionRequest, ProvisionSubset, WriteError, provision_counts,
};
```

Replace the `Msg::PendingLoaded` arm with:

```rust
        Msg::PendingLoaded(result) => {
            model.policy.pending_loading = false;
            match result {
                Ok(p) => {
                    let n = p.len();
                    // After a provision the reload keeps its report in front.
                    model.status = StatusLine::info(match model.policy.provisioned.take() {
                        Some(done) => format!("{done}; {n} pending changes left"),
                        None => format!("{n} pending changes"),
                    });
                    model.policy.pending_selected = 0;
                    model.policy.pending_marked.clear();
                    model.policy.pending = Some(p);
                    // Never over another overlay (an error a provision left open).
                    if matches!(model.overlay, Overlay::None | Overlay::Pending) {
                        model.overlay = Overlay::Pending;
                    }
                }
                Err(e) => {
                    model.policy.provisioned = None;
                    fail(model, "pending", &e, false);
                }
            }
            Vec::new()
        }
```

Replace the `Msg::WriteDone` arm with:

```rust
        Msg::WriteDone {
            kind,
            method,
            result,
        } => {
            model.writes.in_flight = false;
            // The runtime dropped `policy:pending`; the next `p` fetches again.
            model.policy.pending = None;
            model.policy.pending_marked.clear();
            let counts = model.policy.provision_counts.take();
            match result {
                Ok(Outcome::Written(w)) => {
                    let mut text = if kind == pce::write::ObjectKind::Provision {
                        format!("provisioned {}: {}", w.href, counts.unwrap_or_default())
                    } else {
                        let verb = match method {
                            Some(WriteMethod::Post) => "created",
                            Some(WriteMethod::Put) => "updated",
                            Some(WriteMethod::Delete) => "deleted",
                            None => "wrote",
                        };
                        let mut t = format!("{verb} {}", w.href);
                        if w.pending {
                            t.push_str(" (pending)");
                        }
                        t
                    };
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
                    if kind == pce::write::ObjectKind::Provision {
                        model.policy.provisioned = Some(model.status.text.clone());
                    }
                    reload_after_write(model, kind)
                }
                Ok(Outcome::Skipped { reason, .. }) => {
                    model.status = StatusLine::info(format!("skipped: {reason}"));
                    Vec::new()
                }
                // The write may have applied: say so once (the error's own
                // text carries the advice) and reload to show the real state.
                // A failed audit names its log in the error text.
                Err(e) if is_unanswered(&e) => {
                    let audited = !matches!(e, WriteError::AuditFailed { .. });
                    model.status = StatusLine::error(if audited {
                        "write not answered; reloading"
                    } else {
                        "write not answered and not audited; reloading"
                    });
                    model.overlay =
                        Overlay::Error(format!("the request was sent but not answered: {e}"));
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

In `Msg::Planned`, insert before the `// Cannot happen with IfMissing::Create; reported, never sent.` arm:

```rust
                // Provisioning with nothing pending is an answer, not a failure.
                Ok((Plan::Noop { reason, .. }, pce::write::ObjectKind::Provision)) => {
                    model.status = StatusLine::info(reason);
                }
```

In the `Overlay::ConfirmWrite` arm, replace the `y` arm with:

```rust
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    if kind == pce::write::ObjectKind::Provision {
                        // The answer is the new policy version; the counts
                        // come from what is sent.
                        model.policy.provision_counts =
                            Some(counts_text(plan.request().and_then(|r| r.body.as_ref())));
                    }
                    model.overlay = Overlay::None;
                    model.status = StatusLine::info("sending …");
                    model.writes.in_flight = true;
                    vec![Cmd::ExecuteWrite { plan, kind }]
                }
```

Replace the `Overlay::Pending` arm with:

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
                KeyCode::Char(' ') => {
                    let p = &mut model.policy;
                    if let Some(href) = p
                        .pending
                        .as_ref()
                        .and_then(|l| l.get(p.pending_selected))
                        .map(|c| c.href.clone())
                        && !p.pending_marked.remove(&href)
                    {
                        p.pending_marked.insert(href);
                    }
                }
                KeyCode::Char('P') => return provision_key(model),
                KeyCode::Enter => return pending_diff(model),
                _ => {}
            }
            Vec::new()
        }
```

Replace the `Provision` arm of `reload_after_write` (Task 2's `// Not started from the TUI yet.`) with:

```rust
        // Provisioning changes both scopes and the pending list.
        pce::write::ObjectKind::Provision => {
            model.policy.quiet_reload = true;
            let mut cmds = load_rule_sets(model, CacheMode::Bypass);
            model.policy.pending_loading = true;
            cmds.push(Cmd::LoadPending(CacheMode::Bypass));
            cmds
        }
```

Insert after `fn pending_diff`:

```rust
const PROVISION_DESCRIPTION: &str = "illumio-tui provision";

/// `P` in the pending overlay: provision the marked entries, or everything
/// pending when none is marked. The plan reads the pending list once more,
/// past the cache, and the confirm overlay shows the exact body.
fn provision_key(model: &mut Model) -> Vec<Cmd> {
    if writes_refused(model) {
        return Vec::new();
    }
    if model.policy.pending.as_ref().is_none_or(Vec::is_empty) {
        model.status = StatusLine::info("nothing pending");
        return Vec::new();
    }
    let marked = &model.policy.pending_marked;
    let subset = if marked.is_empty() {
        ProvisionSubset::All
    } else {
        ProvisionSubset::Hrefs(marked.iter().cloned().collect())
    };
    model.overlay = Overlay::None;
    model.status = StatusLine::info("planning…");
    model.writes.in_flight = true;
    vec![Cmd::PlanWrite(WriteIntent::Provision(ProvisionRequest {
        description: Some(PROVISION_DESCRIPTION.into()),
        subset,
    }))]
}

/// `ip_lists=1, rule_sets=2`: what a provision body lists, per type, as the
/// CLI prints it.
fn counts_text(body: Option<&serde_json::Value>) -> String {
    body.map(provision_counts)
        .unwrap_or_default()
        .iter()
        .map(|(k, n)| format!("{k}={n}"))
        .collect::<Vec<_>>()
        .join(", ")
}
```

`crates/illumio-tui/src/tui/app.rs`, `plan_intent`: add after the `DeleteDenyRule` arm:

```rust
        // One pending GET past the cache, then the explicit change subset.
        WriteIntent::Provision(req) => w.plan_provision(&req).await?,
```

`crates/illumio-tui/src/tui/ui/widgets/overlay.rs`: replace `render_pending` with:

```rust
/// The pending overlay's key line, the last row inside its box.
const PENDING_KEYS: &str = "j/k select · Space mark · Enter diff · P provision · Esc";

pub fn render_pending(model: &Model, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let r = centered(area, 70, 16);
    f.render_widget(Clear, r);
    let pending = model.policy.pending.as_deref().unwrap_or(&[]);
    let marked = &model.policy.pending_marked;
    // Rows inside the borders, less the key line.
    let rows = (r.height.saturating_sub(2) as usize).saturating_sub(1);
    let sel = model
        .policy
        .pending_selected
        .min(pending.len().saturating_sub(1));
    // Keep the selection in view.
    let first = sel.saturating_sub(rows.saturating_sub(1));
    let mut lines: Vec<Line> = if pending.is_empty() {
        vec![Line::from(Span::styled("none", t.dim_style()))]
    } else {
        pending
            .iter()
            .enumerate()
            .skip(first)
            .take(rows)
            .map(|(i, c)| {
                Line::from(vec![
                    Span::styled(
                        if i == sel { "▸ " } else { "  " },
                        Style::new().fg(t.accent),
                    ),
                    Span::styled(
                        if marked.contains(&c.href) { "[x] " } else { "[ ] " },
                        Style::new().fg(t.accent),
                    ),
                    Span::styled(format!("{:<16}", c.object_type), t.header_style()),
                    Span::styled(format!("{:<8}", c.change_type), Style::new().fg(t.warn)),
                    Span::styled(c.name.clone().unwrap_or_else(|| c.href.clone()), t.base()),
                ])
            })
            .collect()
    };
    lines.resize(rows, Line::from(""));
    lines.push(Line::from(Span::styled(PENDING_KEYS, t.dim_style())));
    let meta = match (pending.len(), marked.len()) {
        (0, _) => "0".to_string(),
        (n, 0) => format!("{}/{n}", sel + 1),
        (n, m) => format!("{}/{n} · {m} marked", sel + 1),
    };
    f.render_widget(
        Paragraph::new(lines)
            .block(panel("Pending changes", &meta, t, true))
            .style(t.base()),
        r,
    );
}
```

- [ ] **Step 4: Run the unit tests**

Run: `cargo fmt --all && cargo test -p illumio-tui --lib`
Expected: PASS (the three new tests and every existing one; `write_done_reports_reloads_the_kind_once_and_forgets_pending` still passes unchanged).

- [ ] **Step 5: Write the snapshot and runtime tests**

Append to `crates/illumio-tui/tests/tui_snapshots.rs`:

```rust
#[test]
fn pending_marks_and_provision_confirm_80x24() {
    use illumio_tui::tui::model::{Overlay, Screen, WritesState};
    use illumio_tui::tui::test_support::provision_plan;
    let mut m = model_with_policy();
    m.screen = Screen::Policy;
    m.policy.pending = Some(pce::models::parse_pending(
        serde_json::from_str(include_str!("../../../fixtures/pending_list.json")).unwrap(),
    ));
    m.policy.pending_selected = 1;
    m.policy
        .pending_marked
        .insert("/orgs/5/sec_policy/draft/ip_lists/21".into());
    m.overlay = Overlay::Pending;
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("[x] ip_lists"), "{out}");
    assert!(out.contains("[ ] rule_sets"), "{out}");
    assert!(out.contains("2/2 · 1 marked"), "{out}");
    assert!(
        out.contains("j/k select · Space mark · Enter diff · P provision · Esc"),
        "{out}"
    );
    insta::assert_snapshot!("policy_pending_marks_80x24", out);
    m.set_theme("light");
    insta::assert_snapshot!(
        "policy_pending_marks_light_styled_80x24",
        render_styled(&m, 80, 24)
    );
    // What an admin sees after `P`: writes on, so the header carries the marker.
    let mut c = model_with_policy();
    c.writes = WritesState::on();
    c.screen = Screen::Policy;
    c.overlay = Overlay::ConfirmWrite {
        plan: provision_plan(),
        kind: pce::write::ObjectKind::Provision,
        scroll: 0,
    };
    let out = render_to_string(&c, 80, 24);
    assert!(out.contains("provision 1 ip_list, 1 rule_set"), "{out}");
    assert!(out.contains("POST /orgs/5/sec_policy"), "{out}");
    assert!(out.contains("\"change_subset\""), "{out}");
    insta::assert_snapshot!("confirm_provision_80x24", out);
    c.set_theme("light");
    insta::assert_snapshot!(
        "confirm_provision_light_styled_80x24",
        render_styled(&c, 80, 24)
    );
}
```

Append to `crates/illumio-tui/tests/tui_runtime.rs`:

```rust
/// Three pending entries (served the same on every GET, expected
/// `pending_gets` times) and the provision POST with exactly `body`.
async fn mount_provision(server: &MockServer, pending_gets: u64, body: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/pending"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"href": "/orgs/5/sec_policy/draft/rule_sets/101", "change_type": "create", "name": "Global-Infra"},
            {"href": "/orgs/5/sec_policy/draft/ip_lists/21", "change_type": "create", "name": "Corp"},
            {"href": "/orgs/5/sec_policy/draft/services/32", "change_type": "update", "name": "Web"}
        ])))
        .expect(pending_gets)
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/sec_policy"))
        .and(body_json(body))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "href": "/orgs/5/sec_policy/7",
            "commit_message": "illumio-tui provision"
        })))
        .expect(1)
        .mount(server)
        .await;
}

#[tokio::test]
async fn provision_marked_posts_exactly_the_marked_hrefs() {
    use illumio_tui::tui::model::Overlay;
    let server = MockServer::start().await;
    mount_objects(&server).await;
    mount_rule_sets(&server, 2).await; // the screen + the reload
    // Opening the list, the plan's fresh read, the reload.
    mount_provision(
        &server,
        3,
        json!({
            "update_description": "illumio-tui provision",
            "change_subset": {
                "rule_sets": [{"href": "/orgs/5/sec_policy/draft/rule_sets/101"}],
                "services": [{"href": "/orgs/5/sec_policy/draft/services/32"}]
            }
        }),
    )
    .await;
    let audit = MemoryAudit::new();
    let keys = vec![
        (500, ch('2')),
        (300, ch('p')),
        (300, ch(' ')), // Global-Infra
        (50, ch('j')),
        (50, ch('j')),
        (50, ch(' ')), // Web
        (50, ch('P')),
        (300, ch('y')),
        (500, KeyCode::F(12)),
    ];
    let model = key_session(
        &server,
        writes_model(),
        Some(tui_guard(audit.clone())),
        keys,
    )
    .await;
    assert_eq!(
        model.status.text,
        "provisioned /orgs/5/sec_policy/7: rule_sets=1, services=1; 3 pending changes left"
    );
    assert_eq!(model.overlay, Overlay::Pending, "the list reloads into its overlay");
    assert_eq!(
        model.stats.calls, 11,
        "health + four lists + rulesets + pending + the plan's pending read + POST + rulesets + pending"
    );
    assert_eq!(writes_seen(&server).await, vec!["POST"]);
    let entries = audit.entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, "/orgs/5/sec_policy");
    assert_eq!(model.busy, 0);
}

#[tokio::test]
async fn provision_all_lists_every_pending_entry() {
    let server = MockServer::start().await;
    mount_objects(&server).await;
    mount_rule_sets(&server, 2).await;
    mount_provision(
        &server,
        3,
        json!({
            "update_description": "illumio-tui provision",
            "change_subset": {
                "ip_lists": [{"href": "/orgs/5/sec_policy/draft/ip_lists/21"}],
                "rule_sets": [{"href": "/orgs/5/sec_policy/draft/rule_sets/101"}],
                "services": [{"href": "/orgs/5/sec_policy/draft/services/32"}]
            }
        }),
    )
    .await;
    let audit = MemoryAudit::new();
    let keys = vec![
        (500, ch('2')),
        (300, ch('p')),
        (300, ch('P')),
        (300, ch('y')),
        (500, KeyCode::F(12)),
    ];
    let model = key_session(
        &server,
        writes_model(),
        Some(tui_guard(audit.clone())),
        keys,
    )
    .await;
    assert_eq!(
        model.status.text,
        "provisioned /orgs/5/sec_policy/7: ip_lists=1, rule_sets=1, services=1; 3 pending changes left"
    );
    assert_eq!(model.stats.calls, 11);
    assert_eq!(writes_seen(&server).await, vec!["POST"]);
    assert_eq!(audit.entries().len(), 1);
    assert_eq!(model.busy, 0);
}
```

- [ ] **Step 6: Create and read the snapshots, run everything**

Run: `INSTA_UPDATE=always cargo test -p illumio-tui --test tui_snapshots pending && git status --short crates/illumio-tui/tests/snapshots`
Expected: four new files, `tui_snapshots__policy_pending_marks_80x24.snap`, `tui_snapshots__policy_pending_marks_light_styled_80x24.snap`, `tui_snapshots__confirm_provision_80x24.snap`, `tui_snapshots__confirm_provision_light_styled_80x24.snap`, and one intentionally changed file, `tui_snapshots__policy_pending_80x24.snap` (each row now has `[ ] `, the meta reads `1/2`, the key line is the box's last row). Read all five. The provision confirm shows `✎ writes` in the header and the writes-on Policy hint `n/N:new e:edit x:del  d scope  p pending`.

Run: `cargo test -p illumio-tui`
Expected: PASS, including `provision_marked_posts_exactly_the_marked_hrefs` and `provision_all_lists_every_pending_entry`.

- [ ] **Step 7: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p illumio-tui
git add crates/illumio-tui/src/tui/model.rs crates/illumio-tui/src/tui/edit.rs crates/illumio-tui/src/tui/update.rs crates/illumio-tui/src/tui/app.rs crates/illumio-tui/src/tui/ui/widgets/overlay.rs crates/illumio-tui/src/tui/test_support.rs crates/illumio-tui/tests/tui_snapshots.rs crates/illumio-tui/tests/tui_runtime.rs crates/illumio-tui/tests/snapshots/tui_snapshots__policy_pending_marks_80x24.snap crates/illumio-tui/tests/snapshots/tui_snapshots__policy_pending_marks_light_styled_80x24.snap crates/illumio-tui/tests/snapshots/tui_snapshots__confirm_provision_80x24.snap crates/illumio-tui/tests/snapshots/tui_snapshots__confirm_provision_light_styled_80x24.snap crates/illumio-tui/tests/snapshots/tui_snapshots__policy_pending_80x24.snap
git commit -m "feat(tui): mark pending changes and provision them, or everything, from the pending overlay

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 5: Ruleset pending diff

**Files:**
- Modify: `crates/illumio-tui/src/tui/diff.rs` (module doc line 1; imports lines 5-6; `Diffable` impl for `RuleSet` after the `Service` impl ~line 193; `diff_for` signature ~line 195; new functions after `service_diff_for` ~line 243; tests)
- Modify: `crates/illumio-tui/src/tui/update.rs` (`pending_diff` ~line 1396 and its doc; the test `pending_list_selects_with_j_k_and_enter_asks_for_one_diff` ~line 5212; new test)
- Modify: `crates/illumio-tui/src/tui/app.rs` (`Cmd::LoadDiff` arm ~line 165; `load_diff` ~line 531)
- Modify: `crates/illumio-tui/src/tui/msg.rs` (doc comment of `Cmd::LoadDiff` ~line 145)
- Modify: `crates/illumio-tui/src/tui/ui/widgets/overlay.rs` (`render_diff` kind names ~line 577)
- Modify: `crates/illumio-tui/src/tui/test_support.rs` (new `rule_set_diff_fixture`)
- Test: `crates/illumio-tui/tests/tui_snapshots.rs`, `crates/illumio-tui/tests/tui_runtime.rs` (append)

**Interfaces:**
- Consumes: `field`, `non_empty`, `draft_href`, `Diffable`, `diff_for` (`diff.rs` ~lines 55-230), `actor_tokens`, `service_tokens` (Task 1), `rule_kind_name` (`model.rs` ~line 790), `RuleSetView::from_rule_set(..).scopes` (`resolve.rs` ~line 379; `app=shop & env=prod`, `All`), `RuleKind`, `Pce::rule_sets(PolicyScope, CacheMode)`, `Model::index` (`model.rs` ~line 1843), `mount_rule_sets` (Task 2).
- Produces:

```rust
// tui/diff.rs
pub fn diff_rule_set(active: Option<&RuleSet>, draft: Option<&RuleSet>, idx: &NameIndex) -> Vec<DiffLine>;
pub fn rule_set_diff_for(change: &PendingChange, draft: &[RuleSet], active: &[RuleSet], idx: &NameIndex) -> ObjectDiff;
// tui/app.rs (private)
async fn load_diff(pce: &Pce, change: &PendingChange, kind: pce::write::ObjectKind, index: Option<&NameIndex>) -> pce::Result<ObjectDiff>;
// tui/test_support.rs
pub fn rule_set_diff_fixture() -> (pce::models::RuleSet, pce::models::RuleSet); // (active, draft)
```

- [ ] **Step 1: Write the failing tests**

Add to `crates/illumio-tui/src/tui/test_support.rs` (after `provision_plan`):

```rust
/// Shop-Prod as active, and a draft of it with a new description, rule 2
/// without its 8000-8099/tcp service and a new rule 4: the ruleset diff the
/// tests and the snapshot draw.
pub fn rule_set_diff_fixture() -> (pce::models::RuleSet, pce::models::RuleSet) {
    let sets: Vec<pce::models::RuleSet> =
        serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json"))
            .expect("rule sets fixture");
    let active = sets[0].clone();
    let mut draft = active.clone();
    draft.description = Some("Ringfence shop".into());
    draft.rules[1].ingress_services.truncate(1);
    draft.rules.push(
        serde_json::from_value(serde_json::json!({
            "href": "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/4", "enabled": true,
            "providers": [{"label": {"href": "/orgs/5/labels/1"}}],
            "consumers": [{"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/20"}}],
            "ingress_services": [{"port": 22, "proto": 6}]
        }))
        .expect("rule 4"),
    );
    (active, draft)
}
```

Append to the tests in `crates/illumio-tui/src/tui/diff.rs`:

```rust
    #[test]
    fn a_ruleset_diff_lists_fields_and_one_line_per_rule() {
        use crate::tui::test_support::{fixture_index, rule_set_diff_fixture};
        let (active, draft) = rule_set_diff_fixture();
        assert_eq!(
            diff_rule_set(Some(&active), Some(&draft), &fixture_index()),
            vec![
                line(Sign::Same, "name: Shop-Prod"),
                line(Sign::Removed, "description: Ringfence"),
                line(Sign::Added, "description: Ringfence shop"),
                line(Sign::Same, "enabled: yes"),
                line(Sign::Same, "scopes: app=shop & env=prod"),
                line(Sign::Same, "allow role=db ← role=web PostgreSQL"),
                line(Sign::Removed, "allow role=web ← iplist:Corp 443/tcp, 8000-8099/tcp"),
                line(Sign::Added, "allow role=web ← iplist:Corp 443/tcp"),
                line(Sign::Added, "allow role=web ← iplist:Any (0.0.0.0/0 and ::/0) 22/tcp"),
                line(Sign::Same, "deny all ← iplist:Any (0.0.0.0/0 and ::/0) 23/tcp"),
                line(Sign::Same, "override_deny role=db ← all 22/tcp"),
            ]
        );
    }

    #[test]
    fn a_created_ruleset_is_all_added() {
        use crate::tui::test_support::fixture_index;
        let sets: Vec<RuleSet> =
            serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap();
        let change = PendingChange {
            href: "/orgs/5/sec_policy/draft/rule_sets/101".into(),
            change_type: "create".into(),
            name: Some("Global-Infra".into()),
            object_type: "rule_sets".into(),
        };
        let d = rule_set_diff_for(&change, &sets, &[], &fixture_index());
        assert_eq!(d.name, "Global-Infra");
        assert_eq!(d.change, DiffKind::Created);
        assert_eq!(
            d.lines,
            vec![
                line(Sign::Added, "name: Global-Infra"),
                line(Sign::Added, "enabled: no"),
                line(Sign::Added, "scopes: All"),
                line(Sign::Added, "allow all ← group:All prod envs All Services"),
            ]
        );
    }
```

In the tests of `crates/illumio-tui/src/tui/update.rs`, in `pending_list_selects_with_j_k_and_enter_asks_for_one_diff` replace

```rust
        assert!(update(&mut m, enter()).is_empty(), "entry 0 is a ruleset");
        assert_eq!(m.status.text, "rule diffs arrive with Plan 5");
```

with

```rust
        assert_eq!(
            update(&mut m, enter()),
            vec![Cmd::LoadDiff {
                change: pending[0].clone(),
                kind: pce::write::ObjectKind::RuleSet
            }],
            "entry 0 is a ruleset"
        );
        m.policy.diff_loading = false;
```

and append:

```rust
    #[test]
    fn a_pending_rule_entry_points_at_its_ruleset() {
        use crate::tui::test_support::model_with_policy;
        let mut m = model_with_policy();
        m.screen = Screen::Policy;
        update(
            &mut m,
            Msg::PendingLoaded(Ok(vec![pce::models::PendingChange {
                href: "/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/1".into(),
                change_type: "update".into(),
                name: None,
                object_type: "sec_rules".into(),
            }])),
        );
        assert!(update(&mut m, enter()).is_empty());
        assert_eq!(m.status.text, "rule changes show in their ruleset's diff");
        assert!(!m.policy.diff_loading);
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p illumio-tui --lib`
Expected: FAIL, compile errors `cannot find function diff_rule_set` and `cannot find function rule_set_diff_for` (and, once those exist, the `update` assertions until `pending_diff` changes).

- [ ] **Step 3: Implement**

`crates/illumio-tui/src/tui/diff.rs`. Change the first doc line to `//! What a pending IP list, service or ruleset changes against its active version.` and the imports to:

```rust
use crate::tui::edit::{actor_tokens, service_tokens};
use crate::tui::model::rule_kind_name;
use pce::models::{Actor, IngressService, IpList, PendingChange, RuleSet, Service};
use pce::write::IpRangeSpec;
use pce::{NameIndex, RuleKind, RuleSetView};
```

Add after `impl Diffable for Service`:

```rust
impl Diffable for RuleSet {
    fn href(&self) -> &str {
        &self.href
    }
    fn name(&self) -> &str {
        &self.name
    }
}
```

Change the last parameter of `diff_for` so it takes a closure (the two existing callers pass `fn` items and stay as they are):

```rust
fn diff_for<T: Diffable>(
    change: &PendingChange,
    draft: &[T],
    active: &[T],
    diff: impl Fn(Option<&T>, Option<&T>) -> Vec<DiffLine>,
) -> ObjectDiff {
```

Add after `pub fn service_diff_for`:

```rust
fn yes_no(on: bool) -> &'static str {
    if on { "yes" } else { "no" }
}

/// `allow role=web ← iplist:Corp 443/tcp`: kind, providers ← consumers,
/// services, in the form's token grammar; ` (disabled)` when it is off.
/// Unknown objects show their href, compared in draft form.
fn rule_summary(
    kind: RuleKind,
    providers: &[Actor],
    consumers: &[Actor],
    services: &[IngressService],
    enabled: bool,
    idx: &NameIndex,
) -> String {
    let mut s = format!(
        "{} {} ← {} {}",
        rule_kind_name(kind),
        actor_tokens(providers, idx).join(", "),
        actor_tokens(consumers, idx).join(", "),
        service_tokens(services, idx).join(", ")
    );
    if !enabled {
        s.push_str(" (disabled)");
    }
    s.replace("/sec_policy/active/", "/sec_policy/draft/")
}

/// Every allow and deny rule of `rs` as (draft href, summary), allow rules first.
fn rule_lines(rs: &RuleSet, idx: &NameIndex) -> Vec<(String, String)> {
    let allow = rs.rules.iter().map(|r| {
        (
            draft_href(&r.href),
            rule_summary(
                RuleKind::Allow,
                &r.providers,
                &r.consumers,
                &r.ingress_services,
                r.enabled,
                idx,
            ),
        )
    });
    let deny = rs.deny_rules.iter().map(|d| {
        let kind = if d.override_ {
            RuleKind::OverrideDeny
        } else {
            RuleKind::Deny
        };
        (
            draft_href(&d.href),
            rule_summary(
                kind,
                &d.providers,
                &d.consumers,
                &d.ingress_services,
                d.enabled,
                idx,
            ),
        )
    });
    allow.chain(deny).collect()
}

/// Draft rules in order (same, changed as `-` old `+` new, or added), then
/// rules only the active side has.
fn rules(out: &mut Vec<DiffLine>, active: &[(String, String)], draft: &[(String, String)]) {
    let line = |sign, text: &str| DiffLine {
        sign,
        text: text.to_string(),
    };
    for (href, d) in draft {
        match active.iter().find(|(h, _)| h == href) {
            Some((_, a)) if a == d => out.push(line(Sign::Same, d)),
            Some((_, a)) => {
                out.push(line(Sign::Removed, a));
                out.push(line(Sign::Added, d));
            }
            None => out.push(line(Sign::Added, d)),
        }
    }
    for (href, a) in active {
        if !draft.iter().any(|(h, _)| h == href) {
            out.push(line(Sign::Removed, a));
        }
    }
}

/// A ruleset's name, description, state and scopes (`-`/`+` when they
/// differ), then one line per rule, matched by href.
pub fn diff_rule_set(
    active: Option<&RuleSet>,
    draft: Option<&RuleSet>,
    idx: &NameIndex,
) -> Vec<DiffLine> {
    let mut out = Vec::new();
    field(
        &mut out,
        "name",
        active.map(|r| r.name.as_str()),
        draft.map(|r| r.name.as_str()),
    );
    field(
        &mut out,
        "description",
        active.and_then(|r| non_empty(&r.description)),
        draft.and_then(|r| non_empty(&r.description)),
    );
    field(
        &mut out,
        "enabled",
        active.map(|r| yes_no(r.enabled)),
        draft.map(|r| yes_no(r.enabled)),
    );
    let scopes = |r: &RuleSet| RuleSetView::from_rule_set(r, idx).scopes.join(" | ");
    let (a, d) = (active.map(scopes), draft.map(scopes));
    field(&mut out, "scopes", a.as_deref(), d.as_deref());
    rules(
        &mut out,
        &active.map(|r| rule_lines(r, idx)).unwrap_or_default(),
        &draft.map(|r| rule_lines(r, idx)).unwrap_or_default(),
    );
    out
}

pub fn rule_set_diff_for(
    change: &PendingChange,
    draft: &[RuleSet],
    active: &[RuleSet],
    idx: &NameIndex,
) -> ObjectDiff {
    diff_for(change, draft, active, |a, d| diff_rule_set(a, d, idx))
}
```

`crates/illumio-tui/src/tui/update.rs`: in `pending_diff`, change the doc comment to "`Enter` on the pending overlay: one diff request for an IP list, a service or a ruleset; anything else says why there is none." and replace the `"rule_sets" | "sec_rules" | "deny_rules"` arm with:

```rust
        "rule_sets" => pce::write::ObjectKind::RuleSet,
        // A rule is provisioned, and diffed, through its ruleset.
        "sec_rules" | "deny_rules" => {
            model.status = StatusLine::info("rule changes show in their ruleset's diff");
            return Vec::new();
        }
```

`crates/illumio-tui/src/tui/app.rs`: replace the `Cmd::LoadDiff` arm with:

```rust
            Cmd::LoadDiff { change, kind } => {
                self.model.busy += 1;
                let (pce, tx) = self.task_ctx();
                // A ruleset diff names labels and lists through the loaded index.
                let index = self.model.index().cloned();
                tokio::spawn(async move {
                    let result = load_diff(&pce, &change, kind, index.as_deref())
                        .await
                        .map_err(|e| e.to_string());
                    let _ = tx.send(Msg::DiffLoaded(result));
                    let _ = tx.send(Msg::Stats(pce.stats()));
                });
            }
```

and replace `load_diff` with:

```rust
/// The draft and active list of one kind through the cache, compared
/// locally. The draft list is usually cached already (the name index, the
/// Policy screen), so the common cost is one GET for the active list, and
/// none when both are fresh.
async fn load_diff(
    pce: &Pce,
    change: &PendingChange,
    kind: pce::write::ObjectKind,
    index: Option<&NameIndex>,
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
        pce::write::ObjectKind::RuleSet => {
            let draft = pce.rule_sets(PolicyScope::Draft, CacheMode::Use).await?.value;
            let active = pce.rule_sets(PolicyScope::Active, CacheMode::Use).await?.value;
            let empty = NameIndex::default();
            Ok(diff::rule_set_diff_for(
                change,
                &draft,
                &active,
                index.unwrap_or(&empty),
            ))
        }
        other => Err(pce::PceError::Invalid(format!(
            "no diff for {}",
            other.as_str()
        ))),
    }
}
```

`crates/illumio-tui/src/tui/msg.rs`: the doc comment of `Cmd::LoadDiff` becomes `/// The draft and active list of one kind (cached) for a pending IP list, service or ruleset.`

`crates/illumio-tui/src/tui/ui/widgets/overlay.rs`, `render_diff`: add the ruleset name to the kind match:

```rust
    let kind = match href.object_type() {
        "ip_lists" => "IP list",
        "services" => "service",
        "rule_sets" => "ruleset",
        other => other,
    };
```

- [ ] **Step 4: Run the unit tests**

Run: `cargo fmt --all && cargo test -p illumio-tui --lib`
Expected: PASS (two new `diff` tests, the changed and the new `update` test, everything else).

- [ ] **Step 5: Write the snapshot and runtime tests**

Append to `crates/illumio-tui/tests/tui_snapshots.rs`:

```rust
#[test]
fn pending_rule_set_diff_80x24() {
    use illumio_tui::tui::diff::{DiffKind, ObjectDiff, diff_rule_set};
    use illumio_tui::tui::model::{Overlay, Screen};
    use illumio_tui::tui::test_support::{fixture_index, rule_set_diff_fixture};
    let (active, draft) = rule_set_diff_fixture();
    let mut m = model_with_policy();
    m.screen = Screen::Policy;
    m.overlay = Overlay::Diff {
        diff: ObjectDiff {
            name: "Shop-Prod".into(),
            href: draft.href.clone(),
            change: DiffKind::Updated,
            lines: diff_rule_set(Some(&active), Some(&draft), &fixture_index()),
        },
        scroll: 0,
    };
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("ruleset Shop-Prod · updated"), "{out}");
    assert!(out.contains("+ allow role=web ← iplist:Corp 443/tcp"), "{out}");
    assert!(out.contains("- description: Ringfence"), "{out}");
    insta::assert_snapshot!("pending_rule_set_diff_80x24", out);
    m.set_theme("light");
    insta::assert_snapshot!(
        "pending_rule_set_diff_light_styled_80x24",
        render_styled(&m, 80, 24)
    );
}
```

Append to `crates/illumio-tui/tests/tui_runtime.rs`:

```rust
#[tokio::test]
async fn ruleset_diff_fetches_the_active_rulesets_once() {
    use illumio_tui::tui::diff::{DiffKind, Sign};
    use illumio_tui::tui::model::Overlay;
    let server = MockServer::start().await;
    mount_objects(&server).await;
    mount_rule_sets(&server, 1).await; // the screen; the diff reads it from the cache
    for (p, body) in [
        (
            "/api/v2/orgs/5/sec_policy/pending",
            include_str!("../../../fixtures/pending_list.json"),
        ),
        ("/api/v2/orgs/5/sec_policy/active/rule_sets", "[]"),
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
    let keys = vec![
        (500, ch('2')),
        (300, ch('p')),
        (300, KeyCode::Enter), // Global-Infra, a pending create
        (300, KeyCode::Esc),
        (50, KeyCode::Enter),
        (300, KeyCode::F(12)),
    ];
    let model = key_session(&server, model(), None, keys).await;
    match &model.overlay {
        Overlay::Diff { diff, scroll } => {
            assert_eq!(diff.name, "Global-Infra");
            assert_eq!(diff.change, DiffKind::Created);
            assert!(diff.lines.iter().all(|l| l.sign == Sign::Added), "{diff:?}");
            assert!(
                diff.lines
                    .iter()
                    .any(|l| l.text == "allow all ← group:All prod envs All Services"),
                "{diff:?}"
            );
            assert_eq!(*scroll, 0);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        model.stats.calls, 8,
        "health + four lists + rulesets + pending + one active rulesets GET; the second Enter is free"
    );
    assert_eq!(model.busy, 0);
}
```

- [ ] **Step 6: Create and read the snapshots, run everything**

Run: `INSTA_UPDATE=always cargo test -p illumio-tui --test tui_snapshots pending_rule_set_diff && git status --short crates/illumio-tui/tests/snapshots`
Expected: exactly two new files, `tui_snapshots__pending_rule_set_diff_80x24.snap` and `tui_snapshots__pending_rule_set_diff_light_styled_80x24.snap`. Read them: title `ruleset Shop-Prod · updated`, eleven lines in the order of `a_ruleset_diff_lists_fields_and_one_line_per_rule`, added green, removed red, unchanged dimmed.

Run: `cargo test -p illumio-tui`
Expected: PASS, including `ruleset_diff_fetches_the_active_rulesets_once`; `pending_diff_fetches_the_active_list_once` still passes.

- [ ] **Step 7: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p illumio-tui
git add crates/illumio-tui/src/tui/diff.rs crates/illumio-tui/src/tui/update.rs crates/illumio-tui/src/tui/app.rs crates/illumio-tui/src/tui/msg.rs crates/illumio-tui/src/tui/ui/widgets/overlay.rs crates/illumio-tui/src/tui/test_support.rs crates/illumio-tui/tests/tui_snapshots.rs crates/illumio-tui/tests/tui_runtime.rs crates/illumio-tui/tests/snapshots/tui_snapshots__pending_rule_set_diff_80x24.snap crates/illumio-tui/tests/snapshots/tui_snapshots__pending_rule_set_diff_light_styled_80x24.snap
git commit -m "feat(tui): diff a pending ruleset against its active version

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 6: Blocked flow to rule (`w`)

**Files:**
- Modify: `crates/illumio-tui/src/tui/edit.rs` (imports; `EditIntent::RuleFromFlow`; `start` and `build_intent` arms; `endpoint_tokens`, `flow_rule_fields`, `ruleset_target` after `ruleset_href_named` (Task 3); tests)
- Modify: `crates/illumio-tui/src/tui/model.rs` (`TrafficState` ~line 671 and `TrafficState::new` ~line 688)
- Modify: `crates/illumio-tui/src/tui/update.rs` (imports line 13; form submit ~line 824; `Msg::Planned`; `traffic_key` ~line 1050; new `flow_rule_key` after `rule_intent` (Task 3); tests)
- Modify: `crates/illumio-tui/src/tui/ui/widgets/overlay.rs` (`screen_hint`)
- Test: `crates/illumio-tui/tests/tui_snapshots.rs`, `crates/illumio-tui/tests/tui_runtime.rs` (append); changed snapshots `tui_snapshots__header_writes_dark_styled_80x24.snap`, `tui_snapshots__header_writes_light_styled_80x24.snap`

**Interfaces:**
- Consumes: `ResolvedFlow { src, dst, port, proto, policy_decision, … }`, `ResolvedFlow::service_text` (`8443/tcp`, `icmp`), `ResolvedEndpoint { ip, fqdn, hostname, name, workload_href, labels: Vec<ResolvedLabel { href, key, value }>, ip_lists: Vec<String> }`, `ResolvedEndpoint::display_name` (`pce/src/traffic/resolved.rs` ~lines 6-98; labels sorted by key, an unknown label has key `?` and its href as value), `PolicyDecision::{Allowed, PotentiallyBlocked, Blocked, Unknown}` (`models/common.rs` ~line 32), `TrafficView::{Flows, Aggregate, Blocked}`, `TrafficState.flows` (`model.rs` ~lines 402, 671), `checked_policy_href`, `is_href` (Task 1), `rule_draft`, `RulesetTarget`, `ACTORS_HINT`, `SERVICES_HINT`, `UNSCOPED_HINT` (Task 3), `fixture_flows`, `model_with_flows`, `label_post_plan` (`test_support.rs` ~lines 55, 63, 191). Fixture flows (`fixtures/traffic_flows.json`, table order unsorted): 0 allowed host-1 → host-2 5432/tcp; 1 potentially blocked `10.1.4.9` (IP list Corp, no workload) → host-1.example (`app=shop, env=prod, role=web`) 8443/tcp; 2 blocked unmanaged-nas (workload `aaaa-3`, no labels) → host-2.example (`app=shop, env=prod, role=db`) icmp; 3 allowed.
- Produces:

```rust
// tui/edit.rs
pub enum EditIntent { /* … */ RuleFromFlow(ResolvedFlow) }
// tui/model.rs
pub struct TrafficState { /* … */ pub rule_form: Option<Form> }
// tui/update.rs (private)
fn flow_rule_key(model: &mut Model) -> Vec<Cmd>;
```

- [ ] **Step 1: Write the failing tests**

Append to the tests in `crates/illumio-tui/src/tui/edit.rs`:

```rust
    #[test]
    fn flow_prefill_covers_every_endpoint_shape() {
        let flows = crate::tui::test_support::fixture_flows();
        let f = form(EditIntent::RuleFromFlow(flows[1].clone())); // Corp -> host-1, 8443/tcp
        assert_eq!(f.title, "Allow rule from flow");
        assert_eq!(
            f.fields.iter().map(|x| x.label).collect::<Vec<_>>(),
            vec![
                "Ruleset",
                "Providers",
                "Consumers",
                "Services",
                "Description",
                "Enabled",
                "Unscoped"
            ]
        );
        assert_eq!(f.value("Ruleset"), "");
        assert_eq!(f.value("Providers"), "app=shop, env=prod, role=web");
        assert_eq!(f.value("Consumers"), "iplist:Corp");
        assert_eq!(f.value("Services"), "8443/tcp");
        assert_eq!(
            f.value("Description"),
            "allow 10.1.4.9 to host-1.example on 8443/tcp"
        );
        assert_eq!((f.value("Enabled"), f.value("Unscoped")), ("y", "y"));
        let g = form(EditIntent::RuleFromFlow(flows[2].clone())); // unmanaged-nas -> host-2, icmp
        assert_eq!(g.value("Providers"), "app=shop, env=prod, role=db");
        assert_eq!(g.value("Consumers"), "workload:/orgs/5/workloads/aaaa-3");
        assert_eq!(g.value("Services"), "icmp");
        assert_eq!(
            g.value("Description"),
            "allow unmanaged-nas to host-2.example on icmp"
        );
        let h = form(EditIntent::RuleFromFlow(flows[0].clone())); // a labelled source
        assert_eq!(h.value("Consumers"), "app=shop, env=prod, role=web");
        assert_eq!(h.value("Unscoped"), "n");
        let mut odd = flows[1].clone();
        odd.src = pce::traffic::ResolvedEndpoint {
            ip: Some("10.9.9.9".into()),
            ..pce::traffic::ResolvedEndpoint::default()
        };
        odd.dst.labels.push(pce::ResolvedLabel {
            href: "/orgs/5/labels/99".into(),
            key: "?".into(),
            value: "/orgs/5/labels/99".into(),
        });
        let o = form(EditIntent::RuleFromFlow(odd.clone()));
        assert_eq!(o.value("Consumers"), "all", "nothing known about the source");
        assert_eq!(
            o.value("Providers"),
            "app=shop, env=prod, role=web, /orgs/5/labels/99",
            "an unknown label keeps its href"
        );
        odd.src.workload_href = Some("/orgs/5/workloads/aaaa-9".into());
        odd.src.ip_lists = vec!["Any (0.0.0.0/0 and ::/0)".into()];
        let w = form(EditIntent::RuleFromFlow(odd));
        assert_eq!(
            w.value("Consumers"),
            "workload:/orgs/5/workloads/aaaa-9",
            "a workload is narrower than the IP lists of its address"
        );
    }

    #[test]
    fn a_flow_rule_targets_a_ruleset_by_name_or_checked_href() {
        let flows = crate::tui::test_support::fixture_flows();
        let mut f = form(EditIntent::RuleFromFlow(flows[1].clone()));
        set(&mut f, "Ruleset", " Shop-Prod ");
        let want = RuleDraft {
            description: Some("allow 10.1.4.9 to host-1.example on 8443/tcp".into()),
            enabled: true,
            providers: vec![
                ActorRef::new(ActorKind::Label, "/orgs/5/labels/3"),
                ActorRef::new(ActorKind::Label, "/orgs/5/labels/4"),
                ActorRef::new(ActorKind::Label, "/orgs/5/labels/1"),
            ],
            consumers: vec![ActorRef::new(
                ActorKind::IpList,
                "/orgs/5/sec_policy/draft/ip_lists/21",
            )],
            services: vec![ServiceRef::Port(PortSpec::parse("8443/tcp").unwrap())],
            unscoped_consumers: true,
            resolve_labels_as: None,
        };
        assert_eq!(
            build(&f).unwrap(),
            Some(WriteIntent::CreateRule {
                ruleset: RulesetTarget::Name("Shop-Prod".into()),
                draft: want.clone(),
            })
        );
        set(&mut f, "Ruleset", SHOP);
        assert_eq!(
            build(&f).unwrap(),
            Some(WriteIntent::CreateRule {
                ruleset: RulesetTarget::Href(SHOP.into()),
                draft: want,
            })
        );
        set(&mut f, "Ruleset", "/orgs/9/sec_policy/draft/rule_sets/100");
        let err = build(&f).unwrap_err();
        assert!(err.starts_with("Ruleset: "), "{err}");
    }
```

Append to the tests in `crates/illumio-tui/src/tui/update.rs`:

```rust
    #[test]
    fn w_opens_the_flow_form_only_for_a_blocked_flow() {
        use crate::tui::test_support::model_with_flows;
        let mut m = model_with_flows();
        assert!(update(&mut m, key('w')).is_empty());
        assert_eq!(m.status.text, "started without --allow-writes");
        let mut m = writes_on(model_with_flows());
        assert!(update(&mut m, key('w')).is_empty(), "row 0 is allowed");
        assert_eq!(m.status.kind, StatusKind::Warn);
        assert_eq!(m.status.text, "this flow is allowed; nothing to add");
        assert_eq!(m.overlay, Overlay::None);
        update(&mut m, key('j'));
        assert!(update(&mut m, key('w')).is_empty());
        assert!(
            matches!(&m.overlay, Overlay::Form(f) if f.title == "Allow rule from flow"),
            "{:?}",
            m.overlay
        );
        update(&mut m, kc(KeyCode::Esc));
        for view in [TrafficView::Aggregate, TrafficView::Blocked] {
            m.traffic.view = view;
            assert!(update(&mut m, key('w')).is_empty());
            assert_eq!(
                m.status.text,
                "select a flow in the Flows view (Enter on a pair)"
            );
            assert_eq!(m.overlay, Overlay::None);
        }
    }

    #[test]
    fn an_unknown_ruleset_name_reopens_the_w_form_with_the_error() {
        use crate::tui::edit::{RulesetTarget, WriteIntent};
        use crate::tui::test_support::{label_post_plan, model_with_flows};
        let mut m = writes_on(model_with_flows());
        update(&mut m, key('j'));
        update(&mut m, key('w'));
        for c in "Shop".chars() {
            update(&mut m, key(c));
        }
        let cmds = update(&mut m, enter());
        assert!(
            matches!(
                cmds.as_slice(),
                [Cmd::PlanWrite(WriteIntent::CreateRule { ruleset: RulesetTarget::Name(n), .. })]
                    if n == "Shop"
            ),
            "{cmds:?}"
        );
        assert!(m.traffic.rule_form.is_some());
        let e = r#"no ruleset named "Shop"; similar: Shop-Prod"#;
        assert!(update(&mut m, Msg::Planned(Err(e.into()))).is_empty());
        match &m.overlay {
            Overlay::Form(f) => {
                assert_eq!(f.error.as_deref(), Some(e));
                assert_eq!(f.value("Ruleset"), "Shop");
                assert_eq!(f.value("Consumers"), "iplist:Corp", "the fields as typed");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(m.status.text, format!("write: {e}"));
        assert!(m.traffic.rule_form.is_none());
        // Fixed and submitted again: a plan that arrives leaves nothing behind.
        for c in "-Prod".chars() {
            update(&mut m, key(c));
        }
        assert_eq!(update(&mut m, enter()).len(), 1);
        update(
            &mut m,
            Msg::Planned(Ok((label_post_plan(), pce::write::ObjectKind::Rule))),
        );
        assert!(m.traffic.rule_form.is_none());
        assert!(matches!(m.overlay, Overlay::ConfirmWrite { .. }), "{:?}", m.overlay);
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p illumio-tui --lib`
Expected: FAIL, compile errors `no variant named RuleFromFlow found for enum EditIntent` and `no field rule_form on type TrafficState`.

- [ ] **Step 3: Implement**

`crates/illumio-tui/src/tui/edit.rs`. Imports (replace the block from Task 4):

```rust
use crate::tokens::{
    ActorToken, ServiceToken, actor_from_href_token, is_href, parse_scope_tokens,
    service_from_href_token, similar, split_kv, with_exclusion, with_similar,
};
use crate::tui::form::{Field, Form};
use pce::NameIndex;
use pce::models::{
    Actor, DenyRule, Fqdn, IngressService, Interface, IpList, Label, Rule, RuleSet, Service,
    Workload,
};
use pce::traffic::{ResolvedEndpoint, ResolvedFlow};
use pce::write::{
    ActorKind, ActorRef, DenyRuleDraft, DenyRulePatch, InterfaceSpec, IpListDraft, IpListPatch,
    IpRangeSpec, LabelDraft, PortSpec, ProvisionRequest, RuleDraft, RulePatch, RuleSetDraft,
    RuleSetPatch, ServiceDraft, ServicePatch, ServiceRef, WorkloadDraft, WorkloadPatch,
    WriteRequest, checked_policy_href,
};
```

Add to `EditIntent` (after `DeleteDenyRule { href: String },`):

```rust
    /// `w` on a blocked flow: a draft allow rule pre-filled from it.
    RuleFromFlow(ResolvedFlow),
```

Add to `start` (after the `DeleteDenyRule` arm):

```rust
        EditIntent::RuleFromFlow(f) => {
            let fields = flow_rule_fields(&f);
            form("Allow rule from flow".into(), fields, EditIntent::RuleFromFlow(f))
        }
```

Insert after `pub fn ruleset_href_named`:

```rust
const RULESET_HINT: &str = "ruleset name or href";

/// An endpoint as actor tokens, the narrowest that is known: its labels,
/// else its workload, else its IP lists, else `all`.
fn endpoint_tokens(e: &ResolvedEndpoint) -> Vec<String> {
    if !e.labels.is_empty() {
        return e
            .labels
            .iter()
            .map(|l| {
                if l.key == "?" || l.value.contains([',', '|']) {
                    l.href.clone()
                } else {
                    format!("{}={}", l.key, l.value)
                }
            })
            .collect();
    }
    if let Some(h) = &e.workload_href {
        return vec![format!("workload:{h}")];
    }
    if !e.ip_lists.is_empty() {
        return e.ip_lists.iter().map(|n| format!("iplist:{n}")).collect();
    }
    vec!["all".into()]
}

/// The `w` form: the ruleset to type, then an allow rule from the
/// destination (providers) to the source (consumers) on the flow's port.
fn flow_rule_fields(f: &ResolvedFlow) -> Vec<Field> {
    vec![
        Field::new("Ruleset", "", RULESET_HINT, true),
        Field::new(
            "Providers",
            endpoint_tokens(&f.dst).join(", "),
            ACTORS_HINT,
            true,
        ),
        Field::new(
            "Consumers",
            endpoint_tokens(&f.src).join(", "),
            ACTORS_HINT,
            true,
        ),
        Field::new("Services", f.service_text(), SERVICES_HINT, true),
        Field::new(
            "Description",
            format!(
                "allow {} to {} on {}",
                f.src.display_name(),
                f.dst.display_name(),
                f.service_text()
            ),
            "optional",
            false,
        ),
        Field::new("Enabled", "y", YES_NO_HINT, true),
        Field::new("Unscoped", yn(f.src.labels.is_empty()), UNSCOPED_HINT, true),
    ]
}

/// The Ruleset field of a `w` form: an href, checked without a request, or
/// a name, resolved at plan time with one draft ruleset read.
fn ruleset_target(text: &str, org_id: u64) -> Result<RulesetTarget, String> {
    if is_href(text) {
        return checked_policy_href(text, "rule_sets", org_id)
            .map(RulesetTarget::Href)
            .map_err(|e| format!("Ruleset: {e}"));
    }
    Ok(RulesetTarget::Name(text.to_string()))
}
```

Add to `build_intent` (before the delete arm):

```rust
        EditIntent::RuleFromFlow(_) => {
            let ruleset = ruleset_target(&v("Ruleset"), org_id)?;
            let draft = rule_draft(form, need_index(index)?, org_id)?;
            Ok(Some(WriteIntent::CreateRule { ruleset, draft }))
        }
```

`crates/illumio-tui/src/tui/model.rs`: add to `TrafficState` (after `pub auto: AutoRefresh,`):

```rust
    /// A submitted `w` form while its plan is on the way: a planner error
    /// (an unknown ruleset name) brings it back with the error.
    pub rule_form: Option<Form>,
```

and to `TrafficState::new` (after `auto: AutoRefresh::new(auto_min_seconds),`):

```rust
            rule_form: None,
```

`crates/illumio-tui/src/tui/update.rs`. Imports: change `use pce::models::{EventFilter, PolicyScope};` to

```rust
use pce::models::{EventFilter, PolicyDecision, PolicyScope};
```

In the `Overlay::Form` arm, `FormEvent::Submit`, the `Ok(Some(intent))` arm keeps a `w` form:

```rust
                    Ok(Some(intent)) => {
                        if matches!(form.intent, EditIntent::RuleFromFlow(_)) {
                            model.traffic.rule_form = Some(form);
                        }
                        model.overlay = Overlay::None;
                        model.status = StatusLine::info("planning…");
                        model.writes.in_flight = true;
                        vec![Cmd::PlanWrite(intent)]
                    }
```

In `Msg::Planned`, take the kept form right after `model.writes.in_flight = false;`:

```rust
            model.writes.in_flight = false;
            let flow_form = model.traffic.rule_form.take();
```

and replace its last arm, `Err(e) => fail(model, "write", &e, false),`, with:

```rust
                Err(e) => {
                    // A `w` form comes back with the planner's answer (an
                    // unknown ruleset name lists similar ones), fields as typed.
                    if let Some(mut f) = flow_form
                        && model.overlay == Overlay::None
                    {
                        f.error = Some(e.clone());
                        model.overlay = Overlay::Form(f);
                    }
                    fail(model, "write", &e, false);
                }
```

In `traffic_key`, add to the `match k.code` (after the `KeyCode::Char('g')` arm):

```rust
        KeyCode::Char('w') => return flow_rule_key(model),
```

Insert after `fn rule_intent`:

```rust
/// `w` on Traffic: a draft allow rule from the selected blocked or
/// potentially blocked flow (Flows view or a drilled pair). The aggregated
/// views carry names, not label hrefs, so they refuse.
fn flow_rule_key(model: &mut Model) -> Vec<Cmd> {
    if writes_refused(model) {
        return Vec::new();
    }
    if model.traffic.view != TrafficView::Flows {
        model.status = StatusLine::warn("select a flow in the Flows view (Enter on a pair)");
        return Vec::new();
    }
    let Some(flow) = model.traffic.flows.selected_item().cloned() else {
        model.status = StatusLine::warn("no row selected");
        return Vec::new();
    };
    match flow.policy_decision {
        PolicyDecision::Blocked | PolicyDecision::PotentiallyBlocked => {
            begin_edit(model, EditIntent::RuleFromFlow(flow))
        }
        PolicyDecision::Allowed => {
            model.status = StatusLine::warn("this flow is allowed; nothing to add");
            Vec::new()
        }
        PolicyDecision::Unknown => {
            model.status = StatusLine::warn("this flow has no policy decision; nothing to add");
            Vec::new()
        }
    }
}
```

`crates/illumio-tui/src/tui/ui/widgets/overlay.rs`: in `screen_hint`, add before `(Screen::Traffic, _)`:

```rust
        (Screen::Traffic, true) => "w:rule  f filter  r run  v view  / search",
```

and change its doc comment's first sentence to "Key hints for the status line. With writes on, Objects, Workloads, Policy and Traffic lead with the edit keys and drop their least used hints so the status text keeps room at 80 columns."

- [ ] **Step 4: Run the unit tests**

Run: `cargo fmt --all && cargo test -p illumio-tui --lib`
Expected: PASS (two new `edit` tests, two new `update` tests, everything else).

- [ ] **Step 5: Write the snapshot and runtime tests**

Append to `crates/illumio-tui/tests/tui_snapshots.rs`:

```rust
#[test]
fn rule_from_flow_form_80x24() {
    use illumio_tui::tui::edit::{EditIntent, form_for};
    use illumio_tui::tui::model::Overlay;
    use illumio_tui::tui::test_support::{fixture_flows, fixture_index};
    let mut m = model_with_flows();
    let idx = fixture_index();
    m.overlay = Overlay::Form(
        form_for(
            EditIntent::RuleFromFlow(fixture_flows()[1].clone()),
            Some(idx.as_ref()),
        )
        .expect("a form"),
    );
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("Allow rule from flow"), "{out}");
    assert!(out.contains("Ruleset*"), "{out}");
    assert!(out.contains("app=shop, env=prod, role=web"), "{out}");
    assert!(out.contains("iplist:Corp"), "{out}");
    assert!(out.contains("8443/tcp"), "{out}");
    insta::assert_snapshot!("form_rule_from_flow_80x24", out);
    m.set_theme("light");
    insta::assert_snapshot!(
        "form_rule_from_flow_light_styled_80x24",
        render_styled(&m, 80, 24)
    );
}
```

Append to `crates/illumio-tui/tests/tui_runtime.rs`:

```rust
#[tokio::test]
async fn w_creates_a_rule_in_the_named_ruleset() {
    use illumio_tui::tui::test_support::model_with_flows;
    let server = MockServer::start().await;
    mount_rule_sets(&server, 2).await; // the name lookup + the reload
    let body = json!({
        "enabled": true,
        "providers": [
            {"label": {"href": "/orgs/5/labels/3"}},
            {"label": {"href": "/orgs/5/labels/4"}},
            {"label": {"href": "/orgs/5/labels/1"}}
        ],
        "consumers": [{"ip_list": {"href": "/orgs/5/sec_policy/draft/ip_lists/21"}}],
        "ingress_services": [{"port": 8443, "proto": 6}],
        "unscoped_consumers": true,
        "description": "allow 10.1.4.9 to host-1.example on 8443/tcp",
        "resolve_labels_as": {"providers": ["workloads"], "consumers": ["workloads"]}
    });
    let mut created = body.clone();
    created["href"] = json!("/orgs/5/sec_policy/draft/rule_sets/100/sec_rules/9");
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets/100/sec_rules"))
        .and(body_json(body))
        .respond_with(ResponseTemplate::new(201).set_body_json(created))
        .expect(1)
        .mount(&server)
        .await;
    let audit = MemoryAudit::new();
    // Connected with the fixture index and the fixture flows already shown.
    let mut m = model_with_flows();
    m.writes = WritesState::on();
    let (mut rt, _tx) = Runtime::with_writer(pce_for(&server), m, tui_guard(audit.clone()));
    let mut keys = vec![(100, ch('j')), (50, ch('w'))]; // the potentially blocked flow
    keys.extend(typed("Shop-Prod"));
    keys.extend([(50, KeyCode::Enter), (300, ch('y')), (400, KeyCode::F(12))]);
    let model = run_loop(&mut rt, Box::pin(press(keys)), &mut |_| {}).await;
    assert_eq!(
        model.status.text,
        "created /orgs/5/sec_policy/draft/rule_sets/100/sec_rules/9 (pending)"
    );
    assert_eq!(
        model.stats.calls, 3,
        "one rulesets GET for the name, the POST, one rulesets reload"
    );
    assert_eq!(writes_seen(&server).await, vec!["POST"]);
    assert_eq!(audit.entries().len(), 1);
    assert_eq!(model.busy, 0);
}

#[tokio::test]
async fn w_on_an_allowed_flow_sends_nothing() {
    use illumio_tui::tui::test_support::model_with_flows;
    let server = MockServer::start().await;
    let mut m = model_with_flows();
    m.writes = WritesState::on();
    let (mut rt, _tx) =
        Runtime::with_writer(pce_for(&server), m, tui_guard(MemoryAudit::new()));
    let keys = vec![(100, ch('w')), (200, KeyCode::F(12))]; // row 0 is allowed
    let model = run_loop(&mut rt, Box::pin(press(keys)), &mut |_| {}).await;
    assert_eq!(model.status.text, "this flow is allowed; nothing to add");
    assert!(
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty()
    );
}
```

- [ ] **Step 6: Create and read the snapshots, run everything**

Run: `INSTA_UPDATE=always cargo test -p illumio-tui --test tui_snapshots && git status --short crates/illumio-tui/tests/snapshots`
Expected: two new files, `tui_snapshots__form_rule_from_flow_80x24.snap` and `tui_snapshots__form_rule_from_flow_light_styled_80x24.snap`, and two intentionally changed files, `tui_snapshots__header_writes_dark_styled_80x24.snap` and `tui_snapshots__header_writes_light_styled_80x24.snap` (Traffic with writes on now hints `w:rule  f filter  r run  v view  / search`). Read all four; nothing else may change.

Run: `cargo test -p illumio-tui`
Expected: PASS, including `w_creates_a_rule_in_the_named_ruleset` and `w_on_an_allowed_flow_sends_nothing`.

- [ ] **Step 7: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p illumio-tui
git add crates/illumio-tui/src/tui/edit.rs crates/illumio-tui/src/tui/model.rs crates/illumio-tui/src/tui/update.rs crates/illumio-tui/src/tui/ui/widgets/overlay.rs crates/illumio-tui/tests/tui_snapshots.rs crates/illumio-tui/tests/tui_runtime.rs crates/illumio-tui/tests/snapshots/tui_snapshots__form_rule_from_flow_80x24.snap crates/illumio-tui/tests/snapshots/tui_snapshots__form_rule_from_flow_light_styled_80x24.snap crates/illumio-tui/tests/snapshots/tui_snapshots__header_writes_dark_styled_80x24.snap crates/illumio-tui/tests/snapshots/tui_snapshots__header_writes_light_styled_80x24.snap
git commit -m "feat(tui): turn a blocked flow into a draft allow rule with w

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 7: Help lines and writes-off coverage across screens

**Files:**
- Modify: `crates/illumio-tui/src/tui/ui/widgets/overlay.rs` (`HELP_LINES` ~lines 17-67)
- Modify: `crates/illumio-tui/src/tui/update.rs` (test `help_scrolls_to_its_last_line_and_reopens_at_the_top` ~line 4872; new test)
- Test: `crates/illumio-tui/tests/tui_runtime.rs` (append); changed snapshots `tui_snapshots__help_overlay_80x24.snap`, `tui_snapshots__help_overlay_end_80x24.snap`, `tui_snapshots__help_overlay_end_light_styled_80x24.snap`

**Interfaces:**
- Consumes: `help_last_scroll(height)` (`overlay.rs` ~line 71: rows inside the box at 24 lines are 22), the keys bound in Tasks 2-6, `writes_refused` (`update.rs` ~line 1687), `pending_fixture` (Task 4), `mount_rule_sets` (Task 2), `key_session`, `writes_seen`, `mount_objects`, `model` (`tests/tui_runtime.rs`).
- Produces: `HELP_LINES` with 28 entries (scrollable at 80x24, last scroll 6). No new functions.

- [ ] **Step 1: Write the failing tests**

In `crates/illumio-tui/src/tui/update.rs`, in `help_scrolls_to_its_last_line_and_reopens_at_the_top`, replace

```rust
        assert_eq!(HELP_LINES.len(), 25);
        update(&mut m, key('?'));
        let last = help_last_scroll(24);
        assert_eq!(last, 3, "25 lines, 22 rows inside the box");
```

with

```rust
        assert_eq!(HELP_LINES.len(), 28);
        update(&mut m, key('?'));
        let last = help_last_scroll(24);
        assert_eq!(last, 6, "28 lines, 22 rows inside the box");
```

and append to the tests:

```rust
    #[test]
    fn policy_and_traffic_edit_keys_wait_for_a_write_in_flight() {
        use crate::tui::ui::widgets::overlay::HELP_LINES;
        use crate::tui::test_support::{model_with_flows, model_with_policy};
        let busy = "a write is already in progress";
        let mut m = writes_on(model_with_policy());
        m.screen = Screen::Policy;
        m.writes.in_flight = true;
        for c in ['n', 'e', 'x'] {
            assert!(update(&mut m, key(c)).is_empty(), "{c}");
            assert_eq!(m.status.text, busy);
            assert_eq!(m.overlay, Overlay::None);
        }
        update(&mut m, kc(KeyCode::Tab));
        for c in ['n', 'N', 'e', 'x'] {
            assert!(update(&mut m, key(c)).is_empty(), "{c}");
            assert_eq!(m.status.text, busy);
            assert_eq!(m.overlay, Overlay::None);
        }
        m.policy.pending = Some(pending_fixture());
        m.overlay = Overlay::Pending;
        assert!(update(&mut m, key('P')).is_empty());
        assert_eq!(m.status.text, busy);
        assert_eq!(m.overlay, Overlay::Pending);
        let mut t = writes_on(model_with_flows());
        t.writes.in_flight = true;
        update(&mut t, key('j'));
        assert!(update(&mut t, key('w')).is_empty());
        assert_eq!(t.status.text, busy);
        assert_eq!(t.overlay, Overlay::None);
        // Every new key is in the help.
        let keys: Vec<&str> = HELP_LINES.iter().map(|(k, _)| *k).collect();
        for k in ["Traffic  w", "Policy   N", "Pending  j/k Space Enter", "Pending  P"] {
            assert!(keys.contains(&k), "{k}");
        }
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p illumio-tui --lib -- help_scrolls policy_and_traffic_edit_keys`
Expected: FAIL: `assertion left == right failed` (25 against 28) in `help_scrolls_to_its_last_line_and_reopens_at_the_top`, and the missing `Traffic  w` key in `policy_and_traffic_edit_keys_wait_for_a_write_in_flight`.

- [ ] **Step 3: Implement** (replace `HELP_LINES` in `crates/illumio-tui/src/tui/ui/widgets/overlay.rs`; every description fits the 54 columns the 80-column box leaves)

```rust
pub const HELP_LINES: &[(&str, &str)] = &[
    (
        "1 2 3 4 5",
        "Traffic · Policy · Objects · Workloads · Events",
    ),
    (":", "command prompt: :traffic :policy :objects :workloads"),
    ("", ":events  :filter <expr>  :window <dur>"),
    ("", ":profile <name>  :draft  :active  :help  :q"),
    ("?", "this help"),
    ("q / Ctrl-C", "quit"),
    ("↑ ↓ PgUp PgDn Home End", "move in the current table"),
    ("s", "cycle sort column and direction"),
    ("/", "search in the current table (Enter keep, Esc clear)"),
    ("r", "refresh the current screen (bypasses the cache)"),
    ("a", "toggle auto-refresh for the current screen"),
    ("Traffic  v", "cycle Flows · App-to-app · Blocked"),
    (
        "Traffic  f",
        "edit filter: window src dst service decisions",
    ),
    ("Traffic  g", "change the aggregate label keys"),
    (
        "Traffic  Enter",
        "detail (flows) · drill into a pair (aggregate)",
    ),
    ("Traffic  w", "draft allow rule from a blocked flow (writes)"),
    ("Policy   d", "toggle draft / active"),
    ("Policy   p", "pending changes"),
    (
        "Policy   Tab / Enter",
        "pane · rule detail · on a ruleset: load deny rules",
    ),
    ("Policy   N", "new deny rule in the selected ruleset (rules pane)"),
    ("Objects  Tab / Enter", "next object type · detail"),
    (
        "Workloads t/T h H Enter",
        "Traffic as src/dst · unhealthy · health sort · detail",
    ),
    (
        "Events   f / Enter / S",
        "filter · event as JSON · sort by severity",
    ),
    (
        "n / e / x  (writes)",
        "new / edit / delete on Objects, Workloads, Policy",
    ),
    (
        "Pending  j/k Space Enter",
        "select · mark · diff of a pending entry",
    ),
    ("Pending  P", "provision the marked entries, or all (writes)"),
    ("y / n", "send / cancel a shown request"),
    ("Esc", "close overlay / cancel"),
];
```

- [ ] **Step 4: Run the unit tests**

Run: `cargo fmt --all && cargo test -p illumio-tui --lib`
Expected: PASS.

- [ ] **Step 5: Write the runtime test** (append to `crates/illumio-tui/tests/tui_runtime.rs`)

```rust
#[tokio::test]
async fn writes_disabled_policy_and_traffic_keys_send_nothing() {
    use illumio_tui::tui::model::{Overlay, StatusKind};
    let server = MockServer::start().await;
    mount_objects(&server).await;
    mount_rule_sets(&server, 1).await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/pending"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/pending_list.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let keys = vec![
        (500, ch('2')),
        (300, ch('n')),
        (50, ch('e')),
        (50, ch('x')),
        (50, KeyCode::Tab),
        (50, ch('n')),
        (50, ch('N')),
        (50, ch('e')),
        (50, ch('x')),
        (50, ch('p')),
        (300, ch('P')),
        (50, KeyCode::Esc),
        (50, ch('1')),
        (50, ch('w')),
        (200, KeyCode::F(12)),
    ];
    let model = key_session(&server, model(), None, keys).await;
    assert_eq!(
        model.stats.calls, 7,
        "health + four lists + rulesets + pending; no editing key costs a request"
    );
    assert!(writes_seen(&server).await.is_empty());
    assert_eq!(model.status.text, "writes are not enabled");
    assert_eq!(model.status.kind, StatusKind::Warn);
    assert_eq!(model.overlay, Overlay::None);
}
```

- [ ] **Step 6: Update and read the help snapshots, run everything**

Run: `INSTA_UPDATE=always cargo test -p illumio-tui --test tui_snapshots help_overlay && git status --short crates/illumio-tui/tests/snapshots`
Expected: exactly three intentionally changed files: `tui_snapshots__help_overlay_80x24.snap` (now `1-22 of 28`, `Traffic  w` and `Policy   N` on the first page), `tui_snapshots__help_overlay_end_80x24.snap` and `tui_snapshots__help_overlay_end_light_styled_80x24.snap` (`7-28 of 28`, the `Pending` lines, `y / n` and `Esc` last). Read all three; no description is clipped.

Run: `cargo test -p illumio-tui`
Expected: PASS, including `writes_disabled_policy_and_traffic_keys_send_nothing`.

- [ ] **Step 7: Gate and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p illumio-tui
git add crates/illumio-tui/src/tui/ui/widgets/overlay.rs crates/illumio-tui/src/tui/update.rs crates/illumio-tui/tests/tui_runtime.rs crates/illumio-tui/tests/snapshots/tui_snapshots__help_overlay_80x24.snap crates/illumio-tui/tests/snapshots/tui_snapshots__help_overlay_end_80x24.snap crates/illumio-tui/tests/snapshots/tui_snapshots__help_overlay_end_light_styled_80x24.snap
git commit -m "feat(tui): help lines for policy editing; writes-off coverage on Policy and Traffic

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 8: Docs, tracking, final gate and request-free smoke

**Files:**
- Modify: `README.md` (`## TUI` bullets for Traffic ~line 85 and Policy ~line 90; new `### Policy editing in the TUI` subsection after `### Editing in the TUI`, before `## PCE protection` ~line 284)
- Modify: `docs/architecture.md` (the TUI module table's `diff` row ~line 52; the token sentence in `## Writes` ~lines 128-130; new `### TUI policy editing` before `## Testing` ~line 162)
- Modify: `docs/index.html` (the writes `<ul>`, one `<li>` after line 96)
- Modify: `tasks/todo.md` (append the Plan 6 checklist and review)
- `tasks/lessons.md`: no change unless the execution produced a correction (then append a `## Plan 6 (TUI policy editing)` section).
- Create (scratchpad only, never committed): `$SCRATCH/plan6_smoke.py`, where `$SCRATCH` is the executing session's scratchpad directory

**Interfaces:**
- Consumes: the built binary `target/debug/illumio-tui`; `../illumio-mcp-server/.env` (credentials of the read-only test PCE demo100, read by `--env-file`, never copied, printed or committed); the scratch venv with `pyte` (Plan 4 smoke).
- Produces: documentation only; no code changes.

- [ ] **Step 1: README**

In `## TUI`, replace the Traffic and Policy bullets with:

```markdown
- **Traffic**: `f` edit the filter, `r` run (bypasses the cache), `v` cycle Flows / App-to-app /
  Blocked, `g` change the group keys, `s` sort, `/` search, `Enter` detail (flows) or drill into
  a pair (app-to-app), `a` auto-refresh. A filter that would be expensive asks for
  confirmation first. With writes on, `w` turns a blocked flow into a draft allow rule.
- **Policy**: `d` toggle draft / active, `p` pending changes, `Tab` switch panes, `Enter` rule
  detail, or on a ruleset load its deny rules. Rules are listed in evaluation order: override
  deny, allow, deny. With writes on, `n` / `e` / `x` create, edit and delete rulesets and rules
  in draft (`N` a deny rule), and the pending overlay marks with `Space` and provisions with `P`.
```

Insert before `## PCE protection`:

```markdown
### Policy editing in the TUI

On the Policy screen, in the draft scope, the same three keys edit policy:
on the rulesets pane `n` opens a new ruleset (Name, Description, Enabled
`y`/`n`, Scopes), `e` edits the selected one and `x` deletes it; on the
rules pane `n` adds an allow rule and `N` a deny rule to the selected
ruleset, `e` edits the selected rule (allow or deny) and `x` deletes it. In
the active scope the keys say `switch to draft to edit policy`.

Rule fields use the CLI's token grammar, comma-separated: Providers and
Consumers take `app=web`, `group:Name`, `iplist:Corp`, `all`,
`workload:/orgs/…` (an href; workload names resolve in the CLI only) and
`!` before a label, label group or IP list; Services take `443/tcp`,
`8000-8100/udp`, `icmp` or a service name. Scopes are comma-separated
labels or label groups, alternatives separated by `|`
(`app=web, env=prod | app=web, env=staging`); an empty field means the
ruleset applies everywhere. Names resolve through the loaded index, with no
request. An edit form shows the rule's entries as tokens and sends only
what changed; an entry the grammar cannot express (a virtual service, an
`any` port) shows as its href or `?`, and changing that field is refused
with `use the CLI`.

In the pending overlay (`p`), `Space` marks entries and `P` provisions the
marked ones, or everything pending when none is marked. The confirm overlay
shows the exact `POST /orgs/N/sec_policy` body with its `change_subset`;
`y` sends it once, the status line says `provisioned <version>: <counts>`,
and the pending list and the rulesets reload. `Enter` on a pending ruleset
shows its changes against active: name, description, state, scopes and one
line per rule (`+ allow app=web ← iplist:Corp 443/tcp`), for one GET of the
active rulesets, cached for five minutes.

On Traffic, `w` on a blocked or potentially blocked flow (Flows view or a
drilled pair) opens an allow-rule form pre-filled from the flow: the
destination's labels as providers, the source's as consumers (an unlabeled
workload as `workload:<href>`, else its IP lists, else `all`), `port/proto`
as the service, and Unscoped `y` when the source has no labels. Type the
ruleset's name or href; a name costs one draft ruleset read at submit, and
an unknown name brings the form back with similar names.
```

- [ ] **Step 2: Architecture** (`docs/architecture.md`)

In the TUI module table, the `diff` row becomes:

```markdown
| `diff` | draft against active for a pending IP list, service or ruleset |
```

In `## Writes`, replace the sentence "CLI parses actor and service tokens (`ActorToken`, `ServiceToken`) before any request and resolves names against lists it fetches at most once per command (`Lists`)." with:

```markdown
The token grammar (`ActorToken`, `ServiceToken`, scopes and the strict href
helpers) lives in `crates/illumio-tui/src/tokens.rs`, shared by the CLI and
the TUI. The CLI parses tokens before any request and resolves names against
lists it fetches at most once per command (`Lists`).
```

Insert before `## Testing`:

```markdown
### TUI policy editing

Policy writes in the TUI reuse the editing path above. `n`/`e`/`x` on the
Policy panes (and `N` on the rules pane) build `EditIntent`s from the
selected ruleset or rule, found by href in `PolicyState.raw` or the
deny-rule fallback; they refuse in the active scope. `tui/edit.rs` renders a
rule's actors and services back to tokens through the `NameIndex`
(`actor_tokens`, `service_tokens`) and resolves typed tokens through it
(`resolve_actor_tokens`, `resolve_service_tokens`) without a request. An
edit compares the field's tokens with the shown ones first, so untouched
entries stay exactly as the PCE returned them, and a field holding an entry
the grammar cannot express may not change.

`app::plan_intent` maps the intents to the policy planners. A `w` form names
its ruleset: `RulesetTarget::Name` costs one draft ruleset read through the
cache before `plan_create_rule`; when the plan fails, the form comes back
with the error (`TrafficState.rule_form`). `P` in the pending overlay plans a
`ProvisionRequest` (the marked hrefs, or all), which reads the pending list
once more past the cache. After `WriteDone` a policy write reloads the draft
rulesets once with `Bypass` (`PolicyState.quiet_reload` keeps the write's
report in the status line); a provision also reloads the pending list and
reports `provisioned <version>: <counts>`, the counts taken from the body
when `y` was pressed.

`Enter` on a pending ruleset reads the draft rulesets (cached) and the active
ones (one GET, five minutes); `diff::diff_rule_set` lists name, description,
state, scopes and one line per rule, matched by href.
```

- [ ] **Step 3: Website** (`docs/index.html`, after the `<li>` that starts "Edits the same objects in the TUI", line 96)

```html
    <li>Edits policy in the TUI too: rulesets, allow and deny rules on the Policy screen with the same token grammar, marked or complete provisioning from the pending list after the exact <code>change_subset</code> is shown, a ruleset diff against active, and <code>w</code> to turn a blocked flow into a draft allow rule.</li>
```

- [ ] **Step 4: Tracking** (append to `tasks/todo.md`)

```markdown
## Plan 6: TUI policy editing (rulesets, rules, deny rules, provisioning, ruleset diff, blocked flow to rule)
Spec: docs/superpowers/specs/2026-10-11-tui-policy-editing-design.md · Plan: docs/superpowers/plans/2026-10-11-tui-policy-editing.md
- [x] Task 1 Shared token module and TUI token resolution
- [x] Task 2 Ruleset editing
- [x] Task 3 Rule and deny-rule editing
- [x] Task 4 Pending marks and provisioning
- [x] Task 5 Ruleset pending diff
- [x] Task 6 Blocked flow to rule (w)
- [x] Task 7 Help lines and writes-off coverage across screens
- [x] Task 8 Docs, tracking, final gate and request-free smoke
```

Tasks 1-7 have landed when this task runs; Task 8 is ticked in the same commit as this file (Step 8).

- [ ] **Step 5: Final gate**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: PASS; note the test count for the review.

Run: `git diff --stat 0e4ebad HEAD -- crates/illumio-tui/src/cli crates/illumio-tui/tests/cli.rs crates/illumio-tui/tests/cli_writes.rs`
Expected: exactly one file, `crates/illumio-tui/src/cli/write.rs` (imports, wrappers, the moved items and tests removed). Read `git diff 0e4ebad HEAD -- crates/illumio-tui/src/cli/write.rs` once more: no message, exit code or envelope changed.

Run: `git diff --stat 0e4ebad HEAD -- crates/pce`
Expected: exactly one file, `crates/pce/src/resolve.rs` (`service_href` and its test).

- [ ] **Step 6: Request-free smoke against demo100**

No write may be sent to demo100. `PCE_LIVE_WRITES` stays unset, the script asserts before starting the binary that no step contains `y` or `Y`, and every run uses a scratch `XDG_STATE_HOME`, so the real audit log is never touched. The only POST is the Traffic query of step C10 (an Explorer query, not a write; it is never audited).

Set up (reuse the Plan 4 venv if it is still in this session's scratchpad):

```bash
cargo build -p illumio-tui
python3 -m venv "$SCRATCH/venv" && "$SCRATCH/venv/bin/pip" install pyte
mkdir -p "$SCRATCH/state"
```

Create `$SCRATCH/plan6_smoke.py`:

```python
#!/usr/bin/env python3
"""Plan 6 smoke against the read-only test PCE. Sends no write: no step
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


def text(display):
    return "\n".join(display)


def audit_lines():
    if not os.path.exists(AUDIT):
        return 0
    with open(AUDIT) as f:
        return sum(1 for _ in f)


REFUSED = "writes are not enabled"

# A: flag given, profile does not allow writes. Every editing key refuses
# and costs nothing.
a = run(
    ["--allow-writes"],
    {},
    [
        (2.0, "2"),        # 1 Policy: the rulesets load
        (1.0, "nex"),      # 2 rulesets pane
        (1.0, "\tnNex"),   # 3 rules pane
        (2.0, "p"),        # 4 pending list (one GET)
        (1.0, "P"),        # 5 provision
        (1.0, "\x1b1w"),   # 6 Traffic, w
    ],
)
for i in (2, 3, 5, 6):
    assert REFUSED in status(a[i][1]), (i, status(a[i][1]))
assert a[1][0] == a[2][0] == a[3][0], ("an editing key made a request", a[1][0], a[2][0], a[3][0])
assert a[4][0] == a[5][0] == a[6][0], ("P or w made a request", a[4][0], a[5][0], a[6][0])
assert "✎ writes" not in a[6][1][0]
print("A ok: n e x N P w refused; calls", [c for c, _ in a])

# C: writes on. A rule form opened and cancelled; a provision planned and its
# confirm cancelled with n; a w form opened on a blocked flow, if the window
# has one, and cancelled. Never y.
c = run(
    ["--allow-writes"],
    {"ILLUMIO_WRITES": "1"},
    [
        (2.0, "2"),            # 1 Policy
        (1.0, "\tn"),          # 2 rules pane: new allow rule form
        (0.5, "\x1b"),         # 3 cancelled
        (2.0, "p"),            # 4 pending overlay
        (0.5, " "),            # 5 mark the first entry
        (3.0, "P"),            # 6 plan: one pending GET, then the confirm overlay
        (0.5, "n"),            # 7 cancel the confirm
        (0.5, "\x1b"),         # 8 close the pending overlay, if open
        (0.5, "1"),            # 9 Traffic
        (30.0, "r"),           # 10 run the default query (an Explorer POST, not a write)
        (1.0, "/blocked\r"),   # 11 Flows view narrowed to blocked and potentially blocked rows
        (1.0, "w"),            # 12 the rule form for the first such flow
        (0.5, "\x1b"),         # 13 cancelled
    ],
)
assert "✎ writes" in c[1][1][0], c[1][1][0]
assert "New allow rule in ruleset" in text(c[2][1]), "the rule form did not open"
assert "cancelled, nothing sent" in status(c[3][1]), status(c[3][1])
assert c[1][0] == c[3][0], ("the form made a request", c[1][0], c[3][0])
print("C pending overlay:\n" + text(c[5][1]))
if "Send this request?" in text(c[6][1]):
    confirm = text(c[6][1])
    assert "POST /orgs/" in confirm and "change_subset" in confirm, confirm
    assert "cancelled, nothing sent" in status(c[7][1]), status(c[7][1])
    print("C provision confirm shown and cancelled; calls", c[5][0], "->", c[7][0])
else:
    print("C P did not open a confirm:", status(c[6][1]))
if "Allow rule from flow" in text(c[12][1]):
    assert "cancelled, nothing sent" in status(c[13][1]), status(c[13][1])
    assert c[11][0] == c[13][0], ("the w form made a request", c[11][0], c[13][0])
    print("C w form:\n" + text(c[12][1]))
else:
    print("C no blocked flow in the window; w said:", status(c[12][1]))
print("C calls per step", [n for n, _ in c])
assert audit_lines() == 0, "an audit line exists: something was sent"
print("audit log lines:", audit_lines())
```

Run it:

```bash
"$SCRATCH/venv/bin/python" -I "$SCRATCH/plan6_smoke.py" target/debug/illumio-tui ../illumio-mcp-server/.env "$SCRATCH/state"
```

Expected: `A ok`, the C assertions pass, the provision confirm shown and cancelled (or the status line when demo100 has nothing pending), the `w` form printed (or the recorded reason when the window has no blocked flow), and `audit log lines: 0`. If an assertion fails, stop and debug (systematic-debugging); do not retry against the PCE in a loop.

- [ ] **Step 7: Review section** (append to `tasks/todo.md` under the Plan 6 checklist)

Write `## Review (Plan 6)` with: what shipped (one line per user-facing feature), the final gate's test count from Step 5, the two `git diff --stat` results from Step 5, the smoke's request counts for A and C and whether the provision confirm and the `w` form were exercised on demo100 (or why not), the statement that no write was sent to any PCE (the TUI write path is proven by the wiremock runtime tests in `tests/tui_runtime.rs` only), and every deviation reported by a task.

- [ ] **Step 8: Commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add README.md docs/architecture.md docs/index.html tasks/todo.md
git commit -m "docs: TUI policy editing, provisioning and blocked flow to rule; Plan 6 tracking and review

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```
