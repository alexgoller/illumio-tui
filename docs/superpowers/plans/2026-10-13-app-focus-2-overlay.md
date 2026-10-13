# App Focus (2 of 3): the App Overlay (`A` / `:app`) and `illumio-tui app` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `A` on any row whose app is unambiguous (or `:app <app>`) opens one full-screen, btop-style overlay for that app: inbound and outbound peers (peer group × service × decision), the rough `approx` blocked-over-time timeline, the app's workloads with enforcement modes and VEN health, and the policy that covers it. Opening sends at most two counted requests (workloads, and rulesets if not cached). `illumio-tui app <app>` prints the same as one envelope (without `--timeline`, which is Plan 3).

**Architecture:** Pure profile builders in the `pce` crate: `pce::traffic::timeline` (buckets and the approximate timeline from last-detected times) and `pce::app` (peers via `group_by_service` per decision, the workload summary, ruleset scope coverage). The TUI keeps Elm style: an `AppView` (new `tui/app_view.rs`) lives in `Model::app_view`, `Overlay::App` marks it open, `update` builds it from the loaded traffic (0 requests), from the Workloads and Policy screens' lists when they are loaded (0 requests), and otherwise from `Cmd::LoadAppWorkloads` (one request filtered by the app's labels) and the existing `Cmd::LoadRuleSets`; every request the overlay emits is counted in its header. The CLI reuses Plan 1's `cli_focus` and `fetch` and adds a multi-table output (`Context::emit_report`).

**Tech Stack:** Rust 2024 (rust-version 1.88), tokio, ratatui 0.30, crossterm, serde/serde_json, clap 4, jiff, insta (plain snapshot files), wiremock 0.6, assert_cmd. No new dependency.

**Spec:** `docs/superpowers/specs/2026-10-13-app-focus-design.md` (binding), Part 2, delivery item 2. Builds on Plan 1 (`docs/superpowers/plans/2026-10-13-app-focus-1-focus.md`), which must be merged first. Plan 3 (`2026-10-13-app-focus-3-timeline.md`) builds on the timeline types and the `AppView` this plan produces.

**Tasks:** 7, in this order: 1 `pce::traffic::timeline` (buckets, approximate timeline); 2 `pce::app` (peers, workloads, policy); 3 TUI `AppView` state and the model hooks; 4 TUI keys, commands and loads (`A`, `:app`, `Tab`, `j`/`k`, `X`, `Esc`, `Enter`, `w`, `r`); 5 TUI rendering and snapshots; 6 CLI `illumio-tui app`; 7 docs, help, tracking, read-only smoke, final gate.

## Global Constraints

- Never hammer the PCE. Every request must be caused by the user, throttled, cached, counted in the header/meta, and visible. Nothing refreshes unsolicited. Opening the overlay sends at most two requests (workloads, and rulesets if not cached), both counted in its header; `X`, `Tab`, cursor keys and `Esc` send nothing; `r` asks first.
- No new dependencies. `git diff main -- '*Cargo.toml' Cargo.lock` must be empty.
- The gate is `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`. Run `cargo fmt --all` first and accept its formatting.
- Snapshots are insta files, updated with `INSTA_UPDATE=always` (`INSTA_UPDATE=always cargo test -p illumio-tui --test tui_snapshots`, then `find crates/illumio-tui/tests/snapshots -name '*.snap.new' -delete`). Read every new or changed `.snap` before committing.
- Live smokes on the demo PCE are read-only. Never pass `--yes`, never press `y`. They run with `--env-file ../illumio-mcp-server/.env`. Never read or print that file.
- Every commit message ends with these two lines:
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx
- Elm-style TUI: `update` is pure and returns `Cmd`s; `Runtime` executes them. Pure aggregation lives in `pce`.
- Plan 1 interfaces used here, by their exact names: `pce::traffic::{Focus, FocusDirection, GroupKey, parse_app, app_labels, apps_in, closest}`, `Focus::{new, with_direction, keeps, app_text, label_matches, is_label_group}`, `FocusDirection::{header_text, next, sides}`, `TrafficState::{focus, focus_result, unfocused, loaded_flows, row_apps, set_focus, loaded_apps}`, `FocusResult::covers`, `AppPicker`, `AppPickerPurpose`, `AppPickerOutcome`, `open_app_picker`, `cli::traffic::{DirectionArg, CliFocus, cli_focus, fetch, Fetched}`, `Meta::queries`, `write_table`/`write_table_csv` taking `?Sized`.

## Spec deviations and resolutions (read before Task 1)

- **Workloads without a request.** When the Workloads screen already holds every workload (`loaded_at` set, not truncated), the overlay filters that list by the app's labels: 0 requests. Otherwise one `GET /workloads?labels=[[...]]` (cached). The spec's table says "1 (paged), cached"; the reuse only lowers it.
- **VEN "active / stale".** Active is `VenHealth::Healthy | Warning`, stale is `Offline | Error`; `Unmanaged` workloads have no VEN and are counted apart (`· N unmanaged`). Enforcement words: `full` → `enforced`, `selective`, `visibility_only` → `visibility`, `idle`, anything else `other`.
- **Scope coverage.** A ruleset covers the app when one of its scopes holds only actors that name one of the app's labels (a label, or a label group containing one) and no exclusion names one; an empty scope (`[[]]`) covers every app and is listed as `all workloads`. A ruleset that does not cover the app but has rules whose consumers or providers name its labels is listed with that rule count. Label-group `sub_groups` are not expanded (as the impact code today).
- **Peers by decision.** One row per peer group, service and decision: `group_by_service` run on each decision's flows. A flow from the app to itself is a peer row on both sides.
- **Excluded side.** With direction `in`, the Outbound panel says `X shows outbound` instead of computing rows (and the other way round).
- **`:app` finds apps without flows.** `:app <app>` accepts an app of the loaded result or any app whose labels exist in the name index (so "App with no flows in the window" can be opened); otherwise the status line names the closest loaded app. `:focus` stays limited to the loaded apps.
- **`w` leaves the overlay.** The rule form replaces the overlay (one overlay at a time); `A` reopens it with no request (everything is cached in the view's sources).
- **CLI tables.** `--table` prints every section under a title line; `--csv` needs `--section workloads|inbound|outbound|policy|timeline` (exit 2 before any request otherwise), since one CSV holds one table. JSON is always the whole report. Open question for Alex.

## Review Focus

- **A truncated base result**: peers and the approximate timeline built from a truncated, unfocused result miss flows. Expected: the Inbound/Outbound titles say `partial`, never silently complete. Pinned in Task 3 (`the_view_is_partial_when_the_traffic_is_truncated_and_not_focused`).
- **A late answer after the overlay was closed and reopened** (another app): a workloads answer for the old view must not fill the new one. Pinned in Task 4 (`a_stale_workloads_answer_is_dropped`).
- **The Policy screen loading when `A` is pressed**: a load already in flight must not be doubled; the overlay waits for it. Pinned in Task 4 (`a_rulesets_load_in_flight_is_not_doubled`).
- **A workloads request that fails**: that panel shows the error and `r retries`; the other panels still render. Pinned in Task 5 (`app_overlay_workloads_error_80x24`).
- **An app that is not a label group** (`unmanaged-nas/-`): peers and timeline still work; workloads and policy say why they are unavailable, and nothing is requested for them. Pinned in Task 4 (`a_name_keyed_app_requests_nothing_for_workloads_or_policy`).

---

### Task 1: `pce::traffic::timeline`: buckets and the approximate timeline

**Files:**
- Create: `crates/pce/src/traffic/timeline.rs`
- Modify: `crates/pce/src/traffic/mod.rs` (module list, re-exports)
- Test: `crates/pce/src/traffic/timeline.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: Plan 1's `Focus::keeps`; `ResolvedFlow::last_detected`; `PolicyDecision`.
- Produces (Plan 3 extends these):

```rust
// pce::traffic::timeline, re-exported from pce::traffic
pub const APPROX_BUCKETS: usize = 24;
pub struct Bucket { pub start: Timestamp, pub end: Timestamp }               // Copy, Serialize
pub fn equal_buckets(start: Timestamp, end: Timestamp, n: usize) -> Vec<Bucket>;
pub struct TimelineBucket { pub start: Timestamp, pub end: Timestamp, pub count: Option<u64>, pub truncated: bool }
pub struct Timeline { pub exact: bool, pub partial: bool, pub buckets: Vec<TimelineBucket> }
impl Timeline {
    pub fn counts(&self) -> Vec<u64>;        // None as 0
    pub fn is_quiet(&self) -> bool;          // no count above 0
}
pub fn approx_timeline(flows: &[ResolvedFlow], focus: &Focus, buckets: &[Bucket]) -> Timeline;
```

- [ ] **Step 1: Write the failing tests**

Create `crates/pce/src/traffic/timeline.rs` with the test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::traffic::focus::{FocusDirection, parse_app};
    use crate::traffic::resolved::tests::rich_flows;

    fn ts(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    #[test]
    fn equal_buckets_cut_the_range_and_end_exactly_at_its_end() {
        let b = equal_buckets(ts("2026-10-09T08:00:00Z"), ts("2026-10-10T08:00:00Z"), 24);
        assert_eq!(b.len(), 24);
        assert_eq!(b[0].start, ts("2026-10-09T08:00:00Z"));
        assert_eq!(b[0].end, ts("2026-10-09T09:00:00Z"));
        assert_eq!(b[23].end, ts("2026-10-10T08:00:00Z"));
        assert!(b.windows(2).all(|w| w[0].end == w[1].start), "no gap, no overlap");
        let odd = equal_buckets(ts("2026-10-10T00:00:00Z"), ts("2026-10-10T00:00:10Z"), 3);
        assert_eq!(odd.last().unwrap().end, ts("2026-10-10T00:00:10Z"));
        assert!(equal_buckets(ts("2026-10-10T00:00:00Z"), ts("2026-10-10T00:00:00Z"), 4).is_empty());
        assert!(equal_buckets(ts("2026-10-10T00:00:00Z"), ts("2026-10-10T01:00:00Z"), 0).is_empty());
    }

    #[test]
    fn approx_counts_blocked_flow_records_by_last_detected() {
        let shop = Focus::new(parse_app("shop/prod"), vec!["app".into(), "env".into()]);
        let buckets = equal_buckets(ts("2026-10-10T07:00:00Z"), ts("2026-10-10T08:00:00Z"), 4);
        let t = approx_timeline(&rich_flows(), &shop, &buckets);
        assert!(!t.exact && !t.partial);
        assert_eq!(
            t.counts(),
            vec![2, 0, 0, 0],
            "flows 15 (07:12) and 21 (07:01) are the blocked ones into shop/prod"
        );
        assert_eq!(t.buckets[0].count, Some(2));
        assert!(!t.is_quiet());
        let out = approx_timeline(
            &rich_flows(),
            &shop.clone().with_direction(FocusDirection::Out),
            &buckets,
        );
        assert!(out.is_quiet(), "shop/prod sends nothing blocked");
        let early = equal_buckets(ts("2026-10-01T00:00:00Z"), ts("2026-10-02T00:00:00Z"), 24);
        assert!(approx_timeline(&rich_flows(), &shop, &early).is_quiet(), "outside every bucket");
    }

    #[test]
    fn a_flow_on_the_last_instant_counts_in_the_last_bucket() {
        let crm = Focus::new(parse_app("crm/prod"), vec!["app".into(), "env".into()]);
        // flow 6, crm/dev -> crm/prod, blocked, last seen 07:20:00
        let buckets = equal_buckets(ts("2026-10-10T07:00:00Z"), ts("2026-10-10T07:20:00Z"), 2);
        assert_eq!(approx_timeline(&rich_flows(), &crm, &buckets).counts(), vec![0, 1]);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Add `pub mod timeline;` to `crates/pce/src/traffic/mod.rs` after `pub mod steps;`.

Run: `cargo test -p pce --lib traffic::timeline`
Expected: FAIL to compile with "cannot find function `equal_buckets` in this scope".

- [ ] **Step 3: Write the implementation**

Above the tests in `crates/pce/src/traffic/timeline.rs`:

```rust
//! Blocked traffic over time: a window cut into buckets, and the
//! approximate timeline the loaded flows give. The approximate one counts
//! flow records by their last-detected time, not connections over time;
//! Plan 3's exact timeline fills the same shape from one blocked-only
//! query per bucket.
use super::focus::Focus;
use super::resolved::ResolvedFlow;
use crate::models::PolicyDecision;
use jiff::{SignedDuration, Timestamp};
use serde::Serialize;

/// The approximate timeline's bucket count.
pub const APPROX_BUCKETS: usize = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Bucket {
    pub start: Timestamp,
    pub end: Timestamp,
}

/// `start` to `end` in `n` equal buckets, oldest first, the last one ending
/// exactly at `end`; none for an empty range or `n == 0`.
pub fn equal_buckets(start: Timestamp, end: Timestamp, n: usize) -> Vec<Bucket> {
    if end <= start || n == 0 {
        return Vec::new();
    }
    let total = end.duration_since(start).as_nanos();
    let at = |i: usize| -> Timestamp {
        if i >= n {
            return end;
        }
        let off = i64::try_from(total * i as i128 / n as i128).unwrap_or(i64::MAX);
        start
            .checked_add(SignedDuration::from_nanos(off))
            .unwrap_or(end)
    };
    (0..n)
        .map(|i| Bucket {
            start: at(i),
            end: at(i + 1),
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TimelineBucket {
    pub start: Timestamp,
    pub end: Timestamp,
    /// Approximate: blocked flow records. Exact: blocked connections.
    /// `None` until the bucket is loaded (exact only).
    pub count: Option<u64>,
    /// The bucket's query hit the cap: `count` is a lower bound.
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Timeline {
    /// `false`: counted from the loaded flows' last-detected times.
    pub exact: bool,
    /// Some buckets are not loaded (an exact timeline stopped part-way).
    pub partial: bool,
    pub buckets: Vec<TimelineBucket>,
}

impl Timeline {
    /// Every bucket's count, oldest first; an unloaded one as 0.
    pub fn counts(&self) -> Vec<u64> {
        self.buckets.iter().map(|b| b.count.unwrap_or(0)).collect()
    }

    /// No bucket counts anything.
    pub fn is_quiet(&self) -> bool {
        self.buckets.iter().all(|b| b.count.unwrap_or(0) == 0)
    }
}

/// The approximate timeline: the blocked flows `focus` keeps, counted as
/// flow records in the bucket their `last_detected` falls into (a bucket
/// holds its start, not its end; the last one holds its end too). A flow
/// without a parseable `last_detected`, or outside every bucket, is not
/// counted.
pub fn approx_timeline(flows: &[ResolvedFlow], focus: &Focus, buckets: &[Bucket]) -> Timeline {
    let mut counts = vec![0u64; buckets.len()];
    let last = buckets.len().checked_sub(1);
    for f in flows
        .iter()
        .filter(|f| f.policy_decision == PolicyDecision::Blocked && focus.keeps(f))
    {
        let Some(t) = f
            .last_detected
            .as_deref()
            .and_then(|s| s.parse::<Timestamp>().ok())
        else {
            continue;
        };
        let i = buckets
            .iter()
            .position(|b| b.start <= t && t < b.end)
            .or_else(|| last.filter(|&l| buckets[l].end == t));
        if let Some(i) = i {
            counts[i] += 1;
        }
    }
    Timeline {
        exact: false,
        partial: false,
        buckets: buckets
            .iter()
            .zip(counts)
            .map(|(b, c)| TimelineBucket {
                start: b.start,
                end: b.end,
                count: Some(c),
                truncated: false,
            })
            .collect(),
    }
}
```

In `crates/pce/src/traffic/mod.rs` add after the `pub use steps::...` line:

```rust
pub use timeline::{
    APPROX_BUCKETS, Bucket, Timeline, TimelineBucket, approx_timeline, equal_buckets,
};
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p pce --lib traffic::timeline`
Expected: PASS, 3 tests.

- [ ] **Step 5: Gate and commit**

```bash
cargo fmt --all && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add crates/pce/src/traffic/timeline.rs crates/pce/src/traffic/mod.rs
git commit -m "feat(pce): timeline buckets and the approximate blocked timeline

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

---

### Task 2: `pce::app`: peers, workloads and policy of one app

**Files:**
- Create: `crates/pce/src/app.rs`
- Modify: `crates/pce/src/lib.rs` (add `pub mod app;` after `pub mod auth;`)
- Test: `crates/pce/src/app.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: Plan 1's `Focus`, `GroupKey`; `group_by_service`, `group_key`, `Direction`; `Workload::{ven_health, display_name, labels, enforcement_mode}`, `VenHealth`, `WorkloadFilter`, `RuleSet`, `Actor`, `ActorMatch`, `NameIndex::{label, label_group}`.
- Produces:

```rust
// pce::app
pub struct AppPeer { pub peer: GroupKey, pub service: String, pub decision: PolicyDecision,
    pub flows: u64, pub connections: u64, pub bytes: u64 }
impl AppPeer { pub fn peer_text(&self) -> String; }
pub fn app_peers(flows: &[ResolvedFlow], focus: &Focus, side: Direction) -> Vec<AppPeer>;
pub struct EnforcementCounts { pub enforced: usize, pub selective: usize, pub visibility: usize, pub idle: usize, pub other: usize }
impl EnforcementCounts { pub fn count(&mut self, mode: &str); pub fn text(&self) -> String; }
pub fn enforcement_word(mode: &str) -> &'static str;
pub struct AppWorkload { pub href: String, pub name: String, pub role: Option<String>, pub enforcement: String, pub ven: VenHealth }
pub struct WorkloadSummary { pub workloads: usize, pub enforcement: EnforcementCounts,
    pub ven_active: usize, pub ven_stale: usize, pub unmanaged: usize }
impl WorkloadSummary { pub fn ven_text(&self) -> String; }
pub fn label_hrefs(matches: &[ActorMatch]) -> Vec<String>;
pub fn app_workload_filter(hrefs: &[String]) -> WorkloadFilter;
pub fn workload_in_app(w: &Workload, hrefs: &[String]) -> bool;
pub fn app_workloads(workloads: &[Workload], idx: &NameIndex, tier_key: &str, now: Timestamp)
    -> (Vec<AppWorkload>, WorkloadSummary);
pub enum PolicyReason { Scope, AllWorkloads, Rules }       // Ord, Serialize snake_case
pub struct AppPolicyRow { pub href: String, pub name: String, pub enabled: bool, pub reason: PolicyReason, pub rules: usize }
pub fn scope_covers(scope: &[Actor], hrefs: &[String], idx: &NameIndex) -> bool;
pub fn app_policy(rule_sets: &[RuleSet], hrefs: &[String], idx: &NameIndex) -> Vec<AppPolicyRow>;
```

- [ ] **Step 1: Write the failing tests**

Create `crates/pce/src/app.rs` with the test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::traffic::resolved::tests::{flows, index};
    use crate::traffic::{FocusDirection, parse_app};

    fn shop() -> Focus {
        Focus::new(parse_app("shop/prod"), vec!["app".into(), "env".into()])
    }

    fn shop_hrefs() -> Vec<String> {
        vec!["/orgs/5/labels/3".into(), "/orgs/5/labels/4".into()]
    }

    fn rows(p: &[AppPeer]) -> Vec<(String, String, PolicyDecision, u64)> {
        p.iter()
            .map(|r| (r.peer_text(), r.service.clone(), r.decision, r.connections))
            .collect()
    }

    #[test]
    fn peers_are_one_row_per_group_service_and_decision() {
        let f = flows();
        assert_eq!(
            rows(&app_peers(&f, &shop(), Direction::In)),
            vec![
                ("shop/prod".into(), "5432/tcp".into(), PolicyDecision::Allowed, 310),
                ("ipl:Corp/-".into(), "8443/tcp".into(), PolicyDecision::PotentiallyBlocked, 12),
                ("unmanaged-nas/-".into(), "icmp".into(), PolicyDecision::Blocked, 3),
            ]
        );
        assert_eq!(
            rows(&app_peers(&f, &shop(), Direction::Out)),
            vec![
                ("shop/prod".into(), "5432/tcp".into(), PolicyDecision::Allowed, 310),
                ("fqdn-1.example/-".into(), "443/tcp".into(), PolicyDecision::Allowed, 5),
            ],
            "the flow to itself is a peer on both sides"
        );
        let mut split = f[0].clone();
        split.policy_decision = PolicyDecision::Blocked;
        let both = app_peers(&[f[0].clone(), split], &shop(), Direction::In);
        assert_eq!(both.len(), 2, "same peer and service, two decisions: two rows");
    }

    #[test]
    fn workloads_count_enforcement_and_ven_health() {
        let all: Vec<Workload> =
            serde_json::from_str(include_str!("../../../fixtures/workloads.json")).unwrap();
        let mine: Vec<Workload> = all
            .iter()
            .filter(|w| workload_in_app(w, &shop_hrefs()))
            .cloned()
            .collect();
        assert_eq!(mine.len(), 2, "aaaa-1 and aaaa-2 carry app=shop env=prod");
        let now: Timestamp = "2026-10-10T08:00:00Z".parse().unwrap();
        let (rows, s) = app_workloads(&mine, &index(), "role", now);
        let view: Vec<(String, Option<String>, String, VenHealth)> = rows
            .iter()
            .map(|w| (w.name.clone(), w.role.clone(), w.enforcement.clone(), w.ven))
            .collect();
        assert_eq!(
            view,
            vec![
                ("host-1.example".into(), Some("web".into()), "full".into(), VenHealth::Healthy),
                ("host-2.example".into(), Some("db".into()), "visibility_only".into(), VenHealth::Offline),
            ]
        );
        assert_eq!(s.workloads, 2);
        assert_eq!(s.enforcement.text(), "enforced 1 · visibility 1");
        assert_eq!(s.ven_text(), "1 active · 1 stale");
        let (_, all_s) = app_workloads(&all, &index(), "role", now);
        assert_eq!(all_s.unmanaged, 1, "aaaa-3 has no VEN");
        assert_eq!(all_s.ven_text(), "2 active · 1 stale · 1 unmanaged");
        assert_eq!(enforcement_word("visibility_only"), "visibility");
        assert_eq!(EnforcementCounts::default().text(), "none");
    }

    #[test]
    fn the_workload_filter_is_one_and_group_of_the_app_labels() {
        let f = app_workload_filter(&shop_hrefs());
        assert_eq!(f.labels, vec![shop_hrefs()]);
        assert!(
            f.to_query()
                .contains(&("labels", r#"[["/orgs/5/labels/3","/orgs/5/labels/4"]]"#.to_string()))
        );
        let m = shop().label_matches(&index()).unwrap();
        assert_eq!(label_hrefs(&m), shop_hrefs());
    }

    #[test]
    fn policy_lists_covering_scopes_first_then_global_ones_then_named_rules() {
        let sets: Vec<RuleSet> =
            serde_json::from_str(include_str!("../../../fixtures/rule_sets.json")).unwrap();
        let idx = index();
        let rows: Vec<(String, PolicyReason, usize)> = app_policy(&sets, &shop_hrefs(), &idx)
            .into_iter()
            .map(|r| (r.name, r.reason, r.rules))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("Shop-Prod".into(), PolicyReason::Scope, 4),
                ("Global-Infra".into(), PolicyReason::AllWorkloads, 1),
            ],
            "Shop-Prod: 2 allow + 2 deny rules; Global-Infra's scope is all workloads"
        );
        let narrow: RuleSet = serde_json::from_value(serde_json::json!({
            "href": "/orgs/5/sec_policy/draft/rule_sets/200",
            "name": "Shop-Dev",
            "enabled": true,
            "scopes": [[{"label": {"href": "/orgs/5/labels/3"}}, {"label": {"href": "/orgs/5/labels/104"}}]],
            "rules": [
                {"href": "/r/1", "enabled": true, "consumers": [{"label": {"href": "/orgs/5/labels/4"}}], "providers": [{"actors": "ams"}], "ingress_services": []},
                {"href": "/r/2", "enabled": true, "consumers": [{"actors": "ams"}], "providers": [{"actors": "ams"}], "ingress_services": []}
            ]
        }))
        .unwrap();
        let rows = app_policy(&[narrow], &shop_hrefs(), &idx);
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0].reason, rows[0].rules), (PolicyReason::Rules, 1), "env=dev scope: only the rule naming env=prod");
        assert!(app_policy(&[], &shop_hrefs(), &idx).is_empty());
    }

    #[test]
    fn a_scope_covers_only_when_every_actor_names_the_app_and_no_exclusion_does() {
        let idx = index();
        let label = |h: &str| Actor {
            actors: None,
            label: Some(Ref::new(h)),
            label_group: None,
            workload: None,
            virtual_service: None,
            virtual_server: None,
            ip_list: None,
            exclusion: None,
            extra: Default::default(),
        };
        let h = shop_hrefs();
        assert!(scope_covers(&[label("/orgs/5/labels/3")], &h, &idx), "app=shop: wider than the app");
        assert!(!scope_covers(&[label("/orgs/5/labels/3"), label("/orgs/5/labels/5")], &h, &idx), "loc=eu is not the app's");
        let mut ex = label("/orgs/5/labels/4");
        ex.exclusion = Some(true);
        assert!(!scope_covers(&[label("/orgs/5/labels/3"), ex], &h, &idx), "env=prod excluded");
        assert!(scope_covers(&[], &h, &idx), "an empty scope is every workload");
    }

    #[test]
    fn direction_does_not_change_the_peers_of_a_side() {
        let f = flows();
        let inbound = shop().with_direction(FocusDirection::In);
        assert_eq!(app_peers(&f, &inbound, Direction::In), app_peers(&f, &shop(), Direction::In));
    }
}
```

The `Actor` literal must list every field of `pce::models::Actor` (`crates/pce/src/models/policy.rs:33-56`); if the struct gains a `Default` derive later, prefer `..Default::default()`.

- [ ] **Step 2: Run the tests to verify they fail**

Add `pub mod app;` to `crates/pce/src/lib.rs` after `pub mod auth;`.

Run: `cargo test -p pce --lib app::tests`
Expected: FAIL to compile with "cannot find function `app_peers` in this scope".

- [ ] **Step 3: Write the implementation**

Above the tests in `crates/pce/src/app.rs`:

```rust
//! One app's profile, for the app overlay and `illumio-tui app`: its peers
//! by service and decision, its workloads' enforcement and VEN health, and
//! the policy that covers it. Pure; no requests.
use crate::models::{
    Actor, ActorMatch, PolicyDecision, Ref, RuleSet, VenHealth, Workload, WorkloadFilter,
};
use crate::resolve::NameIndex;
use crate::traffic::{Direction, Focus, GroupKey, ResolvedFlow, group_by_service, group_key};
use jiff::Timestamp;
use serde::Serialize;

/// One peer group, service and decision on one side of the app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AppPeer {
    pub peer: GroupKey,
    /// `443/tcp`, or `icmp`.
    pub service: String,
    pub decision: PolicyDecision,
    pub flows: u64,
    pub connections: u64,
    pub bytes: u64,
}

impl AppPeer {
    /// `web/prod`.
    pub fn peer_text(&self) -> String {
        self.peer.join("/")
    }
}

/// The app's peers on `side` (`In`: the sources of the flows into it,
/// `Out`: the destinations of its flows out), one row per peer group,
/// service and decision: `group_by_service` on each decision's flows. By
/// connections, then peer, service and decision. A flow from the app to
/// itself is a peer row on both sides.
pub fn app_peers(flows: &[ResolvedFlow], focus: &Focus, side: Direction) -> Vec<AppPeer> {
    let on_side: Vec<&ResolvedFlow> = flows
        .iter()
        .filter(|f| {
            let ep = match side {
                Direction::In => &f.dst,
                Direction::Out => &f.src,
            };
            group_key(ep, &focus.keys) == focus.app
        })
        .collect();
    let mut out = Vec::new();
    for d in [
        PolicyDecision::Blocked,
        PolicyDecision::PotentiallyBlocked,
        PolicyDecision::Allowed,
        PolicyDecision::Unknown,
    ] {
        let part: Vec<ResolvedFlow> = on_side
            .iter()
            .filter(|f| f.policy_decision == d)
            .map(|f| (*f).clone())
            .collect();
        for g in group_by_service(&part, &focus.keys) {
            out.push(AppPeer {
                peer: match side {
                    Direction::In => g.src,
                    Direction::Out => g.dst,
                },
                service: g.service.unwrap_or_default(),
                decision: d,
                flows: g.flows,
                connections: g.connections,
                bytes: g.bytes,
            });
        }
    }
    out.sort_by(|a, b| {
        b.connections
            .cmp(&a.connections)
            .then_with(|| a.peer.cmp(&b.peer))
            .then_with(|| a.service.cmp(&b.service))
            .then_with(|| a.decision.as_str().cmp(b.decision.as_str()))
    });
    out
}

/// `full` → `enforced`, `visibility_only` → `visibility`; the panel's words.
pub fn enforcement_word(mode: &str) -> &'static str {
    match mode {
        "full" => "enforced",
        "selective" => "selective",
        "visibility_only" => "visibility",
        "idle" => "idle",
        _ => "other",
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct EnforcementCounts {
    pub enforced: usize,
    pub selective: usize,
    pub visibility: usize,
    pub idle: usize,
    pub other: usize,
}

impl EnforcementCounts {
    pub fn count(&mut self, mode: &str) {
        match enforcement_word(mode) {
            "enforced" => self.enforced += 1,
            "selective" => self.selective += 1,
            "visibility" => self.visibility += 1,
            "idle" => self.idle += 1,
            _ => self.other += 1,
        }
    }

    /// `enforced 9 · selective 2 · visibility 3`: the modes present, in
    /// that order; `none` without workloads.
    pub fn text(&self) -> String {
        let parts: Vec<String> = [
            ("enforced", self.enforced),
            ("selective", self.selective),
            ("visibility", self.visibility),
            ("idle", self.idle),
            ("other", self.other),
        ]
        .into_iter()
        .filter(|(_, n)| *n > 0)
        .map(|(w, n)| format!("{w} {n}"))
        .collect();
        if parts.is_empty() {
            "none".into()
        } else {
            parts.join(" · ")
        }
    }
}

/// One workload of the app: name, role (the tier key's label), enforcement
/// mode as the PCE names it, VEN health.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AppWorkload {
    pub href: String,
    pub name: String,
    pub role: Option<String>,
    pub enforcement: String,
    pub ven: VenHealth,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct WorkloadSummary {
    pub workloads: usize,
    pub enforcement: EnforcementCounts,
    /// VENs healthy or with a warning.
    pub ven_active: usize,
    /// VENs offline or in error.
    pub ven_stale: usize,
    /// Workloads without a VEN.
    pub unmanaged: usize,
}

impl WorkloadSummary {
    /// `13 active · 1 stale`, with `· 2 unmanaged` when some have no VEN.
    pub fn ven_text(&self) -> String {
        let mut s = format!("{} active · {} stale", self.ven_active, self.ven_stale);
        if self.unmanaged > 0 {
            s.push_str(&format!(" · {} unmanaged", self.unmanaged));
        }
        s
    }
}

/// The label hrefs of `Focus::label_matches`.
pub fn label_hrefs(matches: &[ActorMatch]) -> Vec<String> {
    matches
        .iter()
        .filter_map(|m| match m {
            ActorMatch::Label { label } => Some(label.href.clone()),
            _ => None,
        })
        .collect()
}

/// `GET /workloads?labels=[[...]]`: one AND group of the app's labels.
pub fn app_workload_filter(hrefs: &[String]) -> WorkloadFilter {
    WorkloadFilter {
        labels: vec![hrefs.to_vec()],
        ..WorkloadFilter::default()
    }
}

/// The workload carries every one of the app's labels.
pub fn workload_in_app(w: &Workload, hrefs: &[String]) -> bool {
    hrefs
        .iter()
        .all(|h| w.labels.iter().any(|l| &l.href == h))
}

/// One row per workload (by name) and the summary: enforcement modes, VEN
/// health at `now`, the role from the `tier_key` label.
pub fn app_workloads(
    workloads: &[Workload],
    idx: &NameIndex,
    tier_key: &str,
    now: Timestamp,
) -> (Vec<AppWorkload>, WorkloadSummary) {
    let mut s = WorkloadSummary::default();
    let mut rows: Vec<AppWorkload> = workloads
        .iter()
        .map(|w| {
            let ven = w.ven_health(now);
            let enforcement = w.enforcement_mode.clone().unwrap_or_default();
            s.workloads += 1;
            s.enforcement.count(&enforcement);
            match ven {
                VenHealth::Healthy | VenHealth::Warning => s.ven_active += 1,
                VenHealth::Offline | VenHealth::Error => s.ven_stale += 1,
                VenHealth::Unmanaged => s.unmanaged += 1,
            }
            AppWorkload {
                href: w.href.clone(),
                name: w.display_name(),
                role: w.labels.iter().find_map(|l| {
                    idx.label(&l.href)
                        .filter(|x| x.key == tier_key)
                        .map(|x| x.value.clone())
                }),
                enforcement,
                ven,
            }
        })
        .collect();
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    (rows, s)
}

/// Why a ruleset is listed: a scope covers the app, its scope is every
/// workload, or only some of its rules name the app's labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyReason {
    Scope,
    AllWorkloads,
    Rules,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AppPolicyRow {
    pub href: String,
    pub name: String,
    pub enabled: bool,
    pub reason: PolicyReason,
    /// Every rule (allow and deny) of a covering ruleset; the rules naming
    /// the app otherwise.
    pub rules: usize,
}

fn names_app(a: &Actor, hrefs: &[String], idx: &NameIndex) -> bool {
    if let Some(l) = &a.label {
        return hrefs.contains(&l.href);
    }
    if let Some(g) = &a.label_group {
        return idx
            .label_group(&g.href)
            .is_some_and(|g| g.labels.iter().any(|l: &Ref| hrefs.contains(&l.href)));
    }
    false
}

/// One scope (an AND of label and label-group actors) covers the app:
/// every actor names one of its labels and no exclusion does. An empty
/// scope is every workload.
pub fn scope_covers(scope: &[Actor], hrefs: &[String], idx: &NameIndex) -> bool {
    scope.iter().all(|a| {
        let n = names_app(a, hrefs, idx);
        if a.exclusion == Some(true) { !n } else { n }
    })
}

/// The rulesets for the app: those a scope covers (by name), then those
/// scoped to every workload, then those with rules whose consumers or
/// providers name its labels.
pub fn app_policy(rule_sets: &[RuleSet], hrefs: &[String], idx: &NameIndex) -> Vec<AppPolicyRow> {
    let names = |consumers: &[Actor], providers: &[Actor]| {
        consumers
            .iter()
            .chain(providers)
            .any(|a| a.exclusion != Some(true) && names_app(a, hrefs, idx))
    };
    let mut rows: Vec<AppPolicyRow> = rule_sets
        .iter()
        .filter_map(|rs| {
            let all = rs.rules.len() + rs.deny_rules.len();
            let named = rs
                .rules
                .iter()
                .filter(|r| names(&r.consumers, &r.providers))
                .count()
                + rs
                    .deny_rules
                    .iter()
                    .filter(|r| names(&r.consumers, &r.providers))
                    .count();
            let covering: Vec<&Vec<Actor>> = rs
                .scopes
                .iter()
                .filter(|s| scope_covers(s, hrefs, idx))
                .collect();
            let reason = if covering
                .iter()
                .any(|s| s.iter().any(|a| a.exclusion != Some(true)))
            {
                PolicyReason::Scope
            } else if rs.scopes.is_empty() || !covering.is_empty() {
                PolicyReason::AllWorkloads
            } else if named > 0 {
                PolicyReason::Rules
            } else {
                return None;
            };
            Some(AppPolicyRow {
                href: rs.href.clone(),
                name: rs.name.clone(),
                enabled: rs.enabled,
                reason,
                rules: if reason == PolicyReason::Rules { named } else { all },
            })
        })
        .collect();
    rows.sort_by(|a, b| a.reason.cmp(&b.reason).then_with(|| a.name.cmp(&b.name)));
    rows
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p pce --lib app::tests`
Expected: PASS, 6 tests.

- [ ] **Step 5: Gate and commit**

```bash
cargo fmt --all && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add crates/pce/src/app.rs crates/pce/src/lib.rs
git commit -m "feat(pce): app profile: peers by service and decision, workloads, policy coverage

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

---

### Task 3: TUI `AppView` state and the model hooks

**Files:**
- Create: `crates/illumio-tui/src/tui/app_view.rs`
- Modify: `crates/illumio-tui/src/tui/mod.rs` (add `pub mod app_view;`)
- Modify: `crates/illumio-tui/src/tui/model.rs` (`Overlay::App` and its `PartialEq` arm; `Model` fields `app_view`, `app_seq` ~3849 and `new_connecting` ~3880-3940; `reset_data` ~4047-4071; `TrafficState::{flows_for, partial_for, window_bounds}`)
- Modify: `crates/illumio-tui/src/tui/test_support.rs` (new `with_policy`; `model_with_policy` uses it)
- Test: `crates/illumio-tui/src/tui/app_view.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: Tasks 1-2; Plan 1's `Focus`, `FocusResult::covers`, `TrafficState::{focus_result, unfocused, loaded_flows}`, `TrafficData::{window_start, slices}`.
- Produces (Tasks 4-5 and Plan 3 use these):

```rust
// tui/app_view.rs
pub enum AppPanel { Inbound, Outbound, Policy, Workloads }
impl AppPanel { pub const ALL: [AppPanel; 4]; pub fn index(self) -> usize; pub fn next(self) -> Self; pub fn prev(self) -> Self; pub fn title(self) -> &'static str; }
pub enum Loadable<T> { Loading, Loaded(T), Failed(String), Unavailable(String) }
pub struct AppView {
    pub focus: Focus, pub window: String, pub seq: u64, pub panel: AppPanel, pub cursor: [usize; 4],
    pub inbound: Vec<AppPeer>, pub outbound: Vec<AppPeer>, pub timeline: Option<Timeline>, pub partial: bool,
    pub label_hrefs: Option<Vec<String>>,
    pub workloads: Loadable<(Vec<AppWorkload>, WorkloadSummary)>, pub workloads_truncated: bool,
    pub policy: Loadable<Vec<AppPolicyRow>>, pub policy_sent: bool,
    pub requests: u32, pub cached: u32,
}
impl AppView {
    pub fn new(focus: Focus, window: String, seq: u64) -> Self;
    pub fn rebuild_traffic(&mut self, flows: &[ResolvedFlow], bounds: Option<(Timestamp, Timestamp)>, partial: bool);
    pub fn panel_len(&self, panel: AppPanel) -> usize;
    pub fn move_cursor(&mut self, delta: isize);
    pub fn selected_peer(&self) -> Option<(Direction, &AppPeer)>;
    pub fn selected_workload(&self) -> Option<&AppWorkload>;
    pub fn selected_policy(&self) -> Option<&AppPolicyRow>;
    pub fn header_title(&self) -> String;   // shop/prod · in+out · last 24h
    pub fn header_meta(&self) -> String;    // 2 req · cached
}
// tui/model.rs
pub enum Overlay { /* ... */ App }
pub struct Model { /* ... */ pub app_view: Option<AppView>, pub app_seq: u64 }
impl TrafficState {
    pub fn flows_for(&self, focus: &Focus) -> &[ResolvedFlow];
    pub fn partial_for(&self, focus: &Focus) -> bool;
    pub fn window_bounds(&self) -> Option<(jiff::Timestamp, jiff::Timestamp)>;
}
// tui/test_support.rs
pub fn with_policy(m: Model) -> Model;
```

- [ ] **Step 1: Write the failing tests**

Create `crates/illumio-tui/src/tui/app_view.rs` with its test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::test_support::{model_with_flows, model_with_window};
    use pce::traffic::{FocusDirection, parse_app};

    fn shop(d: FocusDirection) -> Focus {
        Focus::new(parse_app("shop/prod"), vec!["app".into(), "env".into()]).with_direction(d)
    }

    #[test]
    fn the_traffic_panels_follow_the_direction() {
        let m = model_with_flows();
        let mut v = AppView::new(shop(FocusDirection::Both), "24h".into(), 1);
        v.rebuild_traffic(m.traffic.loaded_flows(), None, false);
        assert_eq!((v.inbound.len(), v.outbound.len()), (3, 2));
        assert!(v.timeline.is_none(), "no range: no timeline");
        v.focus.direction = FocusDirection::In;
        v.rebuild_traffic(m.traffic.loaded_flows(), None, false);
        assert_eq!((v.inbound.len(), v.outbound.len()), (3, 0));
        assert_eq!(v.header_title(), "shop/prod · in · last 24h");
        assert_eq!(v.header_meta(), "0 req · cached");
    }

    #[test]
    fn the_approx_timeline_spans_the_loaded_window() {
        let m = model_with_window("24h");
        let mut v = AppView::new(shop(FocusDirection::Both), "24h".into(), 1);
        let focus = v.focus.clone();
        v.rebuild_traffic(
            m.traffic.flows_for(&focus),
            m.traffic.window_bounds(),
            m.traffic.partial_for(&focus),
        );
        let t = v.timeline.as_ref().expect("the window is known");
        assert_eq!(t.buckets.len(), pce::traffic::APPROX_BUCKETS);
        assert_eq!(t.counts().iter().sum::<u64>(), 2, "flows 15 and 21");
        assert_eq!(*t.counts().last().unwrap(), 2, "both in the last hour");
    }

    #[test]
    fn tab_cycles_the_panels_and_the_cursor_stays_in_its_panel() {
        let m = model_with_flows();
        let mut v = AppView::new(shop(FocusDirection::Both), "24h".into(), 1);
        v.rebuild_traffic(m.traffic.loaded_flows(), None, false);
        assert_eq!(v.panel, AppPanel::Inbound);
        v.move_cursor(5);
        assert_eq!(v.cursor[0], 2, "clamped to the 3 inbound rows");
        assert_eq!(v.selected_peer().map(|(d, p)| (d, p.peer_text())), Some((Direction::In, "unmanaged-nas/-".into())));
        v.move_cursor(-1);
        assert_eq!(v.cursor[0], 1);
        let mut seen = Vec::new();
        for _ in 0..4 {
            v.panel = v.panel.next();
            seen.push(v.panel);
        }
        assert_eq!(seen, vec![AppPanel::Outbound, AppPanel::Policy, AppPanel::Workloads, AppPanel::Inbound]);
        assert_eq!(AppPanel::Inbound.prev(), AppPanel::Workloads);
        v.panel = AppPanel::Policy;
        v.move_cursor(1);
        assert_eq!(v.cursor[2], 0, "nothing loaded: the cursor stays");
        assert!(v.selected_policy().is_none());
    }

    #[test]
    fn the_view_is_partial_when_the_traffic_is_truncated_and_not_focused() {
        let mut m = model_with_flows();
        m.traffic.data.as_mut().unwrap().truncated = true;
        let f = shop(FocusDirection::Both);
        assert!(m.traffic.partial_for(&f));
        let data = crate::tui::model::TrafficData {
            flows: m.traffic.loaded_flows()[..3].to_vec(),
            truncated: false,
            matches_count: Some(3),
            elapsed: std::time::Duration::ZERO,
            cache_hit: false,
            fetched_at: std::time::Instant::now(),
            filter: m.traffic.filter.clone(),
            slices: None,
            backfill: None,
        };
        m.traffic.apply_focused(f.clone(), 2, data);
        assert!(!m.traffic.partial_for(&f), "a focused result covers it");
        assert_eq!(m.traffic.flows_for(&f).len(), 3);
        let other = Focus::new(parse_app("unmanaged-nas/-"), vec!["app".into(), "env".into()]);
        assert_eq!(m.traffic.flows_for(&other).len(), 4, "another app reads the unfocused result");
        assert!(m.traffic.partial_for(&other));
    }

    #[test]
    fn the_header_counts_requests_and_says_cached_when_every_answer_was() {
        let mut v = AppView::new(shop(FocusDirection::Both), "1h".into(), 1);
        v.requests = 2;
        assert_eq!(v.header_meta(), "2 req");
        v.cached = 2;
        assert_eq!(v.header_meta(), "2 req · cached");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Add `pub mod app_view;` to `crates/illumio-tui/src/tui/mod.rs` (after `pub mod app_picker;`).

Run: `cargo test -p illumio-tui --lib tui::app_view`
Expected: FAIL to compile with "cannot find type `AppView` in this scope".

- [ ] **Step 3: Write the implementation**

Above the tests in `crates/illumio-tui/src/tui/app_view.rs`:

```rust
//! The app overlay's state (`A` / `:app`): one app's peers, blocked
//! timeline, workloads and policy. Built from what is loaded plus at most
//! two requests, each counted here. Pure: `update` fills it,
//! `ui::widgets::app_overlay` draws it.
use jiff::Timestamp;
use pce::app::{AppPeer, AppPolicyRow, AppWorkload, WorkloadSummary, app_peers};
use pce::traffic::{
    APPROX_BUCKETS, Direction, Focus, FocusDirection, ResolvedFlow, Timeline, approx_timeline,
    equal_buckets,
};

/// The panels `Tab` moves between, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppPanel {
    Inbound,
    Outbound,
    Policy,
    Workloads,
}

impl AppPanel {
    pub const ALL: [AppPanel; 4] = [
        AppPanel::Inbound,
        AppPanel::Outbound,
        AppPanel::Policy,
        AppPanel::Workloads,
    ];

    pub fn index(self) -> usize {
        AppPanel::ALL.iter().position(|p| *p == self).unwrap_or(0)
    }

    /// `Tab`.
    pub fn next(self) -> Self {
        AppPanel::ALL[(self.index() + 1) % AppPanel::ALL.len()]
    }

    /// `BackTab`.
    pub fn prev(self) -> Self {
        AppPanel::ALL[(self.index() + AppPanel::ALL.len() - 1) % AppPanel::ALL.len()]
    }

    pub fn title(self) -> &'static str {
        match self {
            AppPanel::Inbound => "Inbound",
            AppPanel::Outbound => "Outbound",
            AppPanel::Policy => "Policy",
            AppPanel::Workloads => "Workloads",
        }
    }
}

/// A panel fed by a request.
#[derive(Debug, Clone, PartialEq)]
pub enum Loadable<T> {
    Loading,
    Loaded(T),
    /// The request failed; `r` retries.
    Failed(String),
    /// Nothing can be loaded (why).
    Unavailable(String),
}

#[derive(Debug, Clone)]
pub struct AppView {
    pub focus: Focus,
    /// The traffic window, as the filter names it (`24h`).
    pub window: String,
    /// Answers carrying another `seq` belong to an earlier overlay.
    pub seq: u64,
    pub panel: AppPanel,
    /// One cursor per panel, by `AppPanel::index`.
    pub cursor: [usize; 4],
    pub inbound: Vec<AppPeer>,
    pub outbound: Vec<AppPeer>,
    /// `None` without a known time range.
    pub timeline: Option<Timeline>,
    /// The traffic these panels come from was truncated and is not a
    /// focused result for this app.
    pub partial: bool,
    /// The app's labels; `None` for a group that is not a label group.
    pub label_hrefs: Option<Vec<String>>,
    pub workloads: Loadable<(Vec<AppWorkload>, WorkloadSummary)>,
    pub workloads_truncated: bool,
    pub policy: Loadable<Vec<AppPolicyRow>>,
    /// The rulesets were requested by this overlay (an answer counts).
    pub policy_sent: bool,
    /// Requests this overlay sent.
    pub requests: u32,
    /// Of those, the ones answered from the cache.
    pub cached: u32,
}

impl AppView {
    pub fn new(focus: Focus, window: String, seq: u64) -> Self {
        Self {
            focus,
            window,
            seq,
            panel: AppPanel::Inbound,
            cursor: [0; 4],
            inbound: Vec::new(),
            outbound: Vec::new(),
            timeline: None,
            partial: false,
            label_hrefs: None,
            workloads: Loadable::Loading,
            workloads_truncated: false,
            policy: Loadable::Loading,
            policy_sent: false,
            requests: 0,
            cached: 0,
        }
    }

    /// Peers and the approximate timeline from `flows` for the direction
    /// in force; a side the direction leaves out has no rows. No request.
    pub fn rebuild_traffic(
        &mut self,
        flows: &[ResolvedFlow],
        bounds: Option<(Timestamp, Timestamp)>,
        partial: bool,
    ) {
        let d = self.focus.direction;
        self.inbound = if d == FocusDirection::Out {
            Vec::new()
        } else {
            app_peers(flows, &self.focus, Direction::In)
        };
        self.outbound = if d == FocusDirection::In {
            Vec::new()
        } else {
            app_peers(flows, &self.focus, Direction::Out)
        };
        self.timeline = bounds.map(|(start, end)| {
            approx_timeline(flows, &self.focus, &equal_buckets(start, end, APPROX_BUCKETS))
        });
        self.partial = partial;
        for p in AppPanel::ALL {
            let n = self.panel_len(p);
            self.cursor[p.index()] = self.cursor[p.index()].min(n.saturating_sub(1));
        }
    }

    pub fn panel_len(&self, panel: AppPanel) -> usize {
        match panel {
            AppPanel::Inbound => self.inbound.len(),
            AppPanel::Outbound => self.outbound.len(),
            AppPanel::Policy => match &self.policy {
                Loadable::Loaded(rows) => rows.len(),
                _ => 0,
            },
            AppPanel::Workloads => match &self.workloads {
                Loadable::Loaded((rows, _)) => rows.len(),
                _ => 0,
            },
        }
    }

    /// `j` / `k`: within the current panel, clamped.
    pub fn move_cursor(&mut self, delta: isize) {
        let n = self.panel_len(self.panel);
        let c = &mut self.cursor[self.panel.index()];
        *c = c.saturating_add_signed(delta).min(n.saturating_sub(1));
    }

    pub fn selected_peer(&self) -> Option<(Direction, &AppPeer)> {
        let i = self.cursor[self.panel.index()];
        match self.panel {
            AppPanel::Inbound => self.inbound.get(i).map(|p| (Direction::In, p)),
            AppPanel::Outbound => self.outbound.get(i).map(|p| (Direction::Out, p)),
            _ => None,
        }
    }

    pub fn selected_workload(&self) -> Option<&AppWorkload> {
        match (&self.workloads, self.panel) {
            (Loadable::Loaded((rows, _)), AppPanel::Workloads) => {
                rows.get(self.cursor[AppPanel::Workloads.index()])
            }
            _ => None,
        }
    }

    pub fn selected_policy(&self) -> Option<&AppPolicyRow> {
        match (&self.policy, self.panel) {
            (Loadable::Loaded(rows), AppPanel::Policy) => {
                rows.get(self.cursor[AppPanel::Policy.index()])
            }
            _ => None,
        }
    }

    /// `shop/prod · in+out · last 24h`.
    pub fn header_title(&self) -> String {
        format!(
            "{} · {} · last {}",
            self.focus.app_text(),
            self.focus.direction.header_text(),
            self.window
        )
    }

    /// `2 req`, `· cached` once every request it sent came from the cache
    /// (or it sent none).
    pub fn header_meta(&self) -> String {
        let mut s = format!("{} req", self.requests);
        if self.requests == 0 || self.cached >= self.requests {
            s.push_str(" · cached");
        }
        s
    }
}
```

In `crates/illumio-tui/src/tui/model.rs`:

- add `use crate::tui::app_view::AppView;` to the imports;
- add to `pub enum Overlay` after `AppPicker(AppPicker),`:

```rust
    /// `A` / `:app`: the app overlay; its state is `Model::app_view`.
    App,
```

- in `impl PartialEq for Overlay` change `| (Overlay::Pending, Overlay::Pending) => true,` to

```rust
            | (Overlay::Pending, Overlay::Pending)
            | (Overlay::App, Overlay::App) => true,
```

- add to `pub struct Model` after `pub events: EventsState,`:

```rust
    /// The app overlay's state while it is open (and after, until the next
    /// `A`: reopening rebuilds it).
    pub app_view: Option<AppView>,
    /// The last app overlay's sequence number: answers of older ones are dropped.
    pub app_seq: u64,
```

- in `Model::new_connecting`'s `Self { ... }` after `events,` add `app_view: None, app_seq: 0,`;
- in `reset_data`, after `self.events = EventsState::new(min);` add `self.app_view = None;`;
- add to `impl TrafficState` (after `loaded_apps` from Plan 1):

```rust
    /// The loaded flows an app overlay for `focus` reads: a focused result
    /// that covers it, else the unfocused result.
    pub fn flows_for(&self, focus: &Focus) -> &[ResolvedFlow] {
        match (&self.focus_result, &self.unfocused) {
            (Some(r), _) if r.covers(focus) => self.loaded_flows(),
            (_, Some(u)) => &u.flows,
            _ => self.loaded_flows(),
        }
    }

    /// Whether `flows_for(focus)` may miss some of its flows: truncated,
    /// and not a focused result covering it.
    pub fn partial_for(&self, focus: &Focus) -> bool {
        match (&self.focus_result, &self.unfocused) {
            (Some(r), _) if r.covers(focus) => false,
            (_, Some(u)) => u.truncated,
            _ => self.data.as_ref().is_some_and(|d| d.truncated),
        }
    }

    /// The loaded result's window, `(start, end)`: its range's end and the
    /// filter's window back from it. `None` before a result with bounds.
    pub fn window_bounds(&self) -> Option<(jiff::Timestamp, jiff::Timestamp)> {
        let d = self.unfocused.as_ref().or(self.data.as_ref())?;
        Some((d.window_start()?, d.slices.as_ref()?.end))
    }
```

In `crates/illumio-tui/src/tui/test_support.rs` replace `model_with_policy` with:

```rust
/// `m` with the fixture rulesets loaded (draft scope, loaded 5 s ago).
pub fn with_policy(mut m: Model) -> Model {
    let raw: Vec<pce::models::RuleSet> =
        serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json"))
            .expect("rule sets fixture");
    let idx = fixture_index();
    m.policy.set_rule_sets(raw, &idx, None);
    // The first ruleset has inline deny rules; the second is left to the fallback.
    m.policy.loaded_at = Some(Instant::now() - Duration::from_secs(5));
    m
}

/// A connected model with the fixture rulesets loaded (draft scope, loaded 5 s ago).
pub fn model_with_policy() -> Model {
    with_policy(model_fixture())
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p illumio-tui --lib tui::app_view`
Expected: PASS, 5 tests. Then `cargo test -p illumio-tui` (the `ui/mod.rs` overlay `match` must also compile: add the arm `Overlay::App => {}` there for now; Task 5 replaces it with the renderer; the `on_key` overlay `match` in `update.rs` gets `Overlay::App => Vec::new(),` for now; Task 4 replaces it).

- [ ] **Step 5: Gate and commit**

```bash
cargo fmt --all && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add crates/illumio-tui/src/tui/app_view.rs crates/illumio-tui/src/tui/mod.rs crates/illumio-tui/src/tui/model.rs crates/illumio-tui/src/tui/test_support.rs crates/illumio-tui/src/tui/update.rs crates/illumio-tui/src/tui/ui/mod.rs
git commit -m "feat(tui): app overlay state: peers, approximate timeline, panels, counted requests

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

---

### Task 4: `A`, `:app`, the overlay's keys and its two loads

**Files:**
- Modify: `crates/illumio-tui/src/tui/app_picker.rs` (`AppPickerPurpose::Overlay`)
- Modify: `crates/illumio-tui/src/tui/commands.rs` (`Command::App`, `parse`, tests)
- Modify: `crates/illumio-tui/src/tui/msg.rs` (`AppWorkloadsPayload`; `Msg::AppWorkloadsLoaded`; `Cmd::LoadAppWorkloads` and its `PartialEq` arm)
- Modify: `crates/illumio-tui/src/tui/model.rs` (`PendingAction::ReloadApp`)
- Modify: `crates/illumio-tui/src/tui/update.rs` (`Msg::RuleSetsLoaded` ~327-381 hooks; new `Msg::AppWorkloadsLoaded` arm; `on_key`: `Overlay::App` arm, the picker's `Pick` arm, the `Confirm` `y` and `n` arms; `traffic_key` `A`; `on_command`; new fns `app_open_key`, `open_app_by_name`, `open_app`, `rebuild_app_traffic`, `app_overlay_key`, `app_enter`, `app_rule_key`, `app_reload`, `refresh_app_policy`; new test module `app_tests`)
- Modify: `crates/illumio-tui/src/tui/app.rs` (`execute`: `Cmd::LoadAppWorkloads`; `apply` task-result list)
- Test: `crates/illumio-tui/src/tui/update.rs` (`mod app_tests`), `crates/illumio-tui/tests/tui_runtime.rs`

**Interfaces:**
- Consumes: Task 3's `AppView`, `Loadable`, `AppPanel`, `Model::{app_view, app_seq}`, `TrafficState::{flows_for, partial_for, window_bounds}`; Task 2's `app_workload_filter`, `workload_in_app`, `app_workloads`, `app_policy`, `label_hrefs`; Plan 1's `open_app_picker`, `focus_by_name`, `TrafficState::{row_apps, set_focus, loaded_apps}`; existing `load_rule_sets`, `switch`, `begin_edit`, `writes_refused`, `deny_fallback`.
- Produces:

```rust
// tui/app_picker.rs
pub enum AppPickerPurpose { Focus, Overlay }
// tui/commands.rs
pub enum Command { /* ... */ App(Option<String>) }             // `:app [app]`
// tui/msg.rs
pub struct AppWorkloadsPayload { pub workloads: Vec<Workload>, pub truncated: bool, pub hit: bool }
pub enum Msg { /* ... */ AppWorkloadsLoaded { seq: u64, result: Result<AppWorkloadsPayload, String> } }
pub enum Cmd { /* ... */ LoadAppWorkloads { seq: u64, filter: WorkloadFilter, mode: CacheMode } }
// tui/model.rs
pub enum PendingAction { /* ... */ ReloadApp }
// tui/update.rs (private; Plan 3 adds `t` next to them)
fn open_app(model: &mut Model, focus: Focus) -> Vec<Cmd>;
fn open_app_by_name(model: &mut Model, name: &str) -> Vec<Cmd>;
fn rebuild_app_traffic(model: &mut Model);
fn app_overlay_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd>;
fn app_reload(model: &mut Model) -> Vec<Cmd>;
```

- [ ] **Step 1: Write the failing tests**

In `crates/illumio-tui/src/tui/commands.rs` add to `parses_every_documented_command`:

```rust
        assert_eq!(parse("app").unwrap(), Command::App(None));
        assert_eq!(parse("app shop/prod").unwrap(), Command::App(Some("shop/prod".into())));
```

Append to the end of `crates/illumio-tui/src/tui/update.rs`:

```rust
#[cfg(test)]
mod app_tests {
    use super::*;
    use crate::tui::app_view::{AppPanel, Loadable};
    use crate::tui::msg::AppWorkloadsPayload;
    use crate::tui::model::{Model, Overlay, TrafficView, WritesState};
    use crate::tui::test_support::{model_with_flows, with_policy, with_workloads};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use pce::models::Workload;
    use pce::traffic::FocusDirection;

    fn key(c: char) -> Msg {
        Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
    }

    fn kc(code: KeyCode) -> Msg {
        Msg::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[track_caller]
    fn quiet(m: &mut Model, msg: Msg) {
        assert_eq!(update(m, msg), Vec::<Cmd>::new());
    }

    fn requests(cmds: &[Cmd]) -> u32 {
        cmds.iter()
            .filter(|c| matches!(c, Cmd::LoadAppWorkloads { .. } | Cmd::LoadRuleSets { .. }))
            .count() as u32
    }

    fn view(m: &Model) -> &crate::tui::app_view::AppView {
        m.app_view.as_ref().expect("the overlay's state")
    }

    fn shop_labels() -> Vec<String> {
        vec!["/orgs/5/labels/3".into(), "/orgs/5/labels/4".into()]
    }

    fn fixture_workloads() -> Vec<Workload> {
        serde_json::from_str(include_str!("../../../../fixtures/workloads.json")).unwrap()
    }

    /// Flows, workloads and rulesets all loaded: the overlay costs nothing.
    fn everything() -> Model {
        with_policy(with_workloads(model_with_flows()))
    }

    #[test]
    fn a_opens_the_overlay_for_the_row_app_with_at_most_two_requests() {
        let mut m = model_with_flows();
        m.traffic
            .flows
            .select_where(|r| r.flow.dst.display_name() == "fqdn-1.example");
        let cmds = update(&mut m, key('A'));
        assert_eq!(
            cmds,
            vec![
                Cmd::LoadAppWorkloads {
                    seq: 1,
                    filter: pce::app::app_workload_filter(&shop_labels()),
                    mode: CacheMode::Use,
                },
                Cmd::LoadRuleSets {
                    scope: PolicyScope::Draft,
                    mode: CacheMode::Use,
                },
            ]
        );
        assert_eq!(m.overlay, Overlay::App);
        let v = view(&m);
        assert_eq!(v.requests, 2);
        assert_eq!(v.focus.app_text(), "shop/prod");
        assert_eq!(v.focus.direction, FocusDirection::Both, "no focus: both");
        assert_eq!((v.inbound.len(), v.outbound.len()), (3, 2));
        assert_eq!(v.header_title(), "shop/prod · in+out · last 24h");
        assert!(matches!(v.workloads, Loadable::Loading) && matches!(v.policy, Loadable::Loading));
    }

    #[test]
    fn loaded_screens_make_the_overlay_free() {
        let mut m = everything();
        quiet(&mut m, key('A'));
        let v = view(&m);
        assert_eq!(v.requests, 0);
        assert_eq!(v.header_meta(), "0 req · cached");
        let Loadable::Loaded((rows, s)) = &v.workloads else {
            panic!("{:?}", v.workloads)
        };
        assert_eq!(rows.iter().map(|w| w.name.as_str()).collect::<Vec<_>>(), vec!["host-1.example", "host-2.example"]);
        assert_eq!(s.enforcement.text(), "enforced 1 · visibility 1");
        assert_eq!(s.ven_text(), "1 active · 1 stale");
        let Loadable::Loaded(p) = &v.policy else {
            panic!("{:?}", v.policy)
        };
        assert_eq!(p.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), vec!["Shop-Prod", "Global-Infra"]);
    }

    #[test]
    fn the_overlay_takes_the_focus_direction_and_x_cycles_it_without_requests() {
        let mut m = everything();
        quiet(&mut m, Msg::Command(Command::Focus(Some("shop/prod".into()))));
        quiet(&mut m, key('X')); // in
        quiet(&mut m, key('A'));
        assert_eq!(view(&m).focus.direction, FocusDirection::In);
        assert_eq!(view(&m).outbound.len(), 0);
        quiet(&mut m, key('X'));
        assert_eq!(view(&m).focus.direction, FocusDirection::Out);
        assert_eq!((view(&m).inbound.len(), view(&m).outbound.len()), (0, 2));
        assert_eq!(
            m.traffic.focus.as_ref().unwrap().direction,
            FocusDirection::In,
            "the overlay's X leaves the traffic focus alone"
        );
    }

    #[test]
    fn esc_closes_and_tab_and_j_k_move_without_requests() {
        let mut m = everything();
        quiet(&mut m, key('A'));
        quiet(&mut m, key('j'));
        assert_eq!(view(&m).cursor[0], 1);
        quiet(&mut m, kc(KeyCode::Tab));
        assert_eq!(view(&m).panel, AppPanel::Outbound);
        quiet(&mut m, kc(KeyCode::BackTab));
        assert_eq!(view(&m).panel, AppPanel::Inbound);
        quiet(&mut m, kc(KeyCode::Esc));
        assert_eq!(m.overlay, Overlay::None);
        assert_eq!(m.screen, Screen::Traffic);
        assert_eq!(m.traffic.view, TrafficView::Flows, "back to the view it was opened from");
    }

    #[test]
    fn enter_on_a_peer_focuses_that_pair_in_flows() {
        let mut m = everything();
        quiet(&mut m, key('A'));
        quiet(&mut m, kc(KeyCode::Enter)); // inbound row 0: shop/prod -> shop/prod 5432/tcp
        assert_eq!(m.overlay, Overlay::None);
        assert_eq!(m.traffic.view, TrafficView::Flows);
        assert_eq!(m.traffic.focus.as_ref().map(|f| f.app_text()).as_deref(), Some("shop/prod"));
        assert_eq!(
            m.traffic.drill,
            Some(Drill::Service(
                vec!["shop".into(), "prod".into()],
                vec!["shop".into(), "prod".into()],
                "5432/tcp".into(),
                Decisions::All
            ))
        );
        assert_eq!(m.traffic.flows.len(), 1);
    }

    #[test]
    fn enter_on_a_workload_or_a_ruleset_opens_it_on_its_screen() {
        let mut m = everything();
        quiet(&mut m, key('A'));
        for _ in 0..3 {
            quiet(&mut m, kc(KeyCode::Tab));
        }
        assert_eq!(view(&m).panel, AppPanel::Workloads);
        quiet(&mut m, key('j'));
        let _ = update(&mut m, kc(KeyCode::Enter));
        assert_eq!(m.screen, Screen::Workloads);
        assert!(m.workloads.show_detail);
        assert_eq!(
            m.workloads.table.selected_item().map(|r| r.workload.href.as_str()),
            Some("/orgs/5/workloads/aaaa-2")
        );
        m.screen = Screen::Traffic;
        quiet(&mut m, key('A'));
        quiet(&mut m, kc(KeyCode::Tab));
        quiet(&mut m, kc(KeyCode::Tab));
        let _ = update(&mut m, kc(KeyCode::Enter));
        assert_eq!(m.screen, Screen::Policy);
        assert_eq!(
            m.policy.rulesets.selected_item().map(|r| r.name.as_str()),
            Some("Shop-Prod")
        );
    }

    #[test]
    fn w_drafts_an_allow_rule_from_a_blocked_peer_only() {
        let mut m = everything();
        m.writes = WritesState::on();
        quiet(&mut m, key('A'));
        quiet(&mut m, kc(KeyCode::Enter)); // leave via a peer...
        quiet(&mut m, key('A')); // ...and reopen: nothing sent
        quiet(&mut m, key('w'));
        assert_eq!(m.status.text, "this row is allowed; nothing to add");
        quiet(&mut m, key('j'));
        quiet(&mut m, key('j')); // unmanaged-nas/- icmp, blocked
        let _ = update(&mut m, key('w'));
        assert!(matches!(m.overlay, Overlay::Form(_)), "{:?}", m.overlay);
    }

    #[test]
    fn r_asks_then_reloads_both_panels_past_the_cache() {
        let mut m = everything();
        quiet(&mut m, key('A'));
        quiet(&mut m, key('r'));
        let Overlay::Confirm { reason, .. } = &m.overlay else {
            panic!("{:?}", m.overlay)
        };
        assert_eq!(reason, "Reload the workloads and the rulesets of shop/prod: 2 requests? (y/n)");
        quiet(&mut m, key('n'));
        assert_eq!(m.overlay, Overlay::App, "no: back to the overlay, nothing sent");
        quiet(&mut m, key('r'));
        let cmds = update(&mut m, key('y'));
        assert_eq!(requests(&cmds), 2);
        assert!(cmds.iter().all(|c| matches!(
            c,
            Cmd::LoadAppWorkloads { mode: CacheMode::Bypass, .. } | Cmd::LoadRuleSets { mode: CacheMode::Bypass, .. }
        )));
        assert_eq!(m.overlay, Overlay::App);
        assert_eq!(view(&m).requests, 2);
    }

    #[test]
    fn the_overlay_never_sends_a_request_it_did_not_count() {
        let mut m = model_with_flows();
        let mut sent = 0;
        for msg in [
            key('A'),
            kc(KeyCode::Tab),
            key('j'),
            key('X'),
            key('X'),
            key('X'),
            kc(KeyCode::BackTab),
            key('k'),
            key('r'),
            key('y'),
            key('X'),
        ] {
            sent += requests(&update(&mut m, msg));
        }
        assert_eq!(
            sent, 3,
            "open: workloads and rulesets; r: the workloads again (the rulesets are still loading)"
        );
        assert_eq!(view(&m).requests, sent);
    }

    #[test]
    fn answers_fill_the_panels_and_stale_ones_are_dropped() {
        let mut m = model_with_flows();
        let _ = update(&mut m, key('A'));
        let seq = view(&m).seq;
        quiet(
            &mut m,
            Msg::AppWorkloadsLoaded {
                seq: seq + 7,
                result: Ok(AppWorkloadsPayload { workloads: fixture_workloads(), truncated: false, hit: true }),
            },
        );
        assert!(matches!(view(&m).workloads, Loadable::Loading), "a stale answer is dropped");
        quiet(
            &mut m,
            Msg::AppWorkloadsLoaded {
                seq,
                result: Ok(AppWorkloadsPayload { workloads: fixture_workloads(), truncated: false, hit: true }),
            },
        );
        assert!(matches!(&view(&m).workloads, Loadable::Loaded((rows, _)) if rows.len() == 2), "filtered to the app");
        assert_eq!(view(&m).cached, 1);
        let rs: Vec<pce::models::RuleSet> =
            serde_json::from_str(include_str!("../../../../fixtures/rule_sets.json")).unwrap();
        let _ = update(&mut m, Msg::RuleSetsLoaded { scope: PolicyScope::Draft, result: Ok((rs, 3)) });
        assert!(matches!(&view(&m).policy, Loadable::Loaded(rows) if rows.len() == 2));
        assert_eq!(view(&m).header_meta(), "2 req · cached");
    }

    #[test]
    fn a_stale_workloads_answer_is_dropped() {
        let mut m = model_with_flows();
        let _ = update(&mut m, key('A'));
        let old = view(&m).seq;
        quiet(&mut m, kc(KeyCode::Esc));
        let _ = update(&mut m, Msg::Command(Command::App(Some("shop/prod".into()))));
        assert_ne!(view(&m).seq, old);
        quiet(
            &mut m,
            Msg::AppWorkloadsLoaded {
                seq: old,
                result: Err("late".into()),
            },
        );
        assert!(matches!(view(&m).workloads, Loadable::Loading));
    }

    #[test]
    fn a_failed_workloads_request_shows_the_error_and_r_retries() {
        let mut m = model_with_flows();
        let _ = update(&mut m, key('A'));
        let seq = view(&m).seq;
        quiet(&mut m, Msg::AppWorkloadsLoaded { seq, result: Err("HTTP 500".into()) });
        assert_eq!(view(&m).workloads, Loadable::Failed("HTTP 500".into()));
        assert_eq!(view(&m).inbound.len(), 3, "the other panels stay");
    }

    #[test]
    fn a_rulesets_load_in_flight_is_not_doubled() {
        let mut m = with_workloads(model_with_flows());
        m.policy.loading = true;
        let cmds = update(&mut m, key('A'));
        assert_eq!(requests(&cmds), 0, "workloads are loaded; the rulesets are on their way");
        assert!(matches!(view(&m).policy, Loadable::Loading));
        assert!(!view(&m).policy_sent);
    }

    #[test]
    fn a_name_keyed_app_requests_nothing_for_workloads_or_policy() {
        let mut m = model_with_flows();
        // Flows row 2: unmanaged-nas -> shop/prod; A takes the source.
        m.traffic
            .flows
            .select_where(|r| r.flow.src.display_name() == "unmanaged-nas");
        let cmds = update(&mut m, key('A'));
        assert_eq!(requests(&cmds), 0);
        let v = view(&m);
        assert_eq!(v.focus.app_text(), "unmanaged-nas/-");
        assert_eq!(v.outbound.len(), 1, "its one blocked flow");
        assert!(matches!(&v.workloads, Loadable::Unavailable(why) if why.contains("not a label group")));
        assert!(matches!(&v.policy, Loadable::Unavailable(_)));
    }

    #[test]
    fn app_by_name_reaches_apps_without_flows_and_names_the_closest_otherwise() {
        let mut m = everything();
        m.traffic.data.as_mut().unwrap().flows.clear();
        m.traffic.rebuild_views();
        quiet(&mut m, Msg::Command(Command::App(Some("shop/prod".into()))));
        assert_eq!(m.overlay, Overlay::App, "its labels exist");
        assert!(view(&m).inbound.is_empty() && view(&m).outbound.is_empty());
        quiet(&mut m, kc(KeyCode::Esc));
        quiet(&mut m, Msg::Command(Command::App(Some("shop/prd".into()))));
        assert_eq!(m.overlay, Overlay::None);
        assert!(m.status.text.starts_with("unknown app \"shop/prd\""), "{}", m.status.text);
        quiet(&mut m, Msg::Command(Command::App(None)));
        assert!(matches!(&m.overlay, Overlay::AppPicker(p) if p.purpose == crate::tui::app_picker::AppPickerPurpose::Overlay));
    }
}
```

Append to `crates/illumio-tui/tests/tui_runtime.rs`:

```rust
#[tokio::test]
async fn opening_the_app_overlay_reads_workloads_and_rulesets_once() {
    use illumio_tui::tui::app_view::Loadable;
    use illumio_tui::tui::commands::Command;
    use illumio_tui::tui::msg::Msg;
    use illumio_tui::tui::test_support::model_with_flows;
    use wiremock::matchers::query_param;
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .and(query_param("labels", r#"[["/orgs/5/labels/3","/orgs/5/labels/4"]]"#))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/workloads.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/rule_sets.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;
    // The Policy screen's own deny-rule fallback may ask for the selected ruleset's.
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;
    let pce = pce_for(&server);
    let (mut rt, tx) = Runtime::new(pce, model_with_flows());
    tx.send(Msg::Command(Command::App(Some("shop/prod".into()))))
        .unwrap();
    let model = run_loop(&mut rt, Box::pin(press(vec![(600, ch('q'))])), &mut |_| {}).await;
    let v = model.app_view.as_ref().expect("the overlay");
    assert_eq!(v.requests, 2);
    assert!(matches!(&v.workloads, Loadable::Loaded((rows, _)) if rows.len() == 2));
    assert!(matches!(&v.policy, Loadable::Loaded(rows) if rows.len() == 2));
    assert_eq!(model.busy, 0);
}
```

`q` inside the overlay closes it (it does not quit); the stream ends after it, which ends the loop.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p illumio-tui --lib app_tests commands`
Expected: FAIL to compile with "no variant named `App` found for enum `Command`" and "no variant named `LoadAppWorkloads` found for enum `Cmd`".

- [ ] **Step 3: Write the implementation**

`crates/illumio-tui/src/tui/app_picker.rs`: add to `AppPickerPurpose`:

```rust
    /// `A` / `:app`: open the app overlay for it.
    Overlay,
```

`crates/illumio-tui/src/tui/commands.rs`: add after `Focus(Option<String>),`:

```rust
    /// `:app <app>` opens the app overlay; `:app` alone opens the picker.
    App(Option<String>),
```

and in `parse` next to `"focus"`:

```rust
        "app" => Ok(Command::App((!arg.is_empty()).then(|| arg.to_string()))),
```

`crates/illumio-tui/src/tui/msg.rs`: add `WorkloadFilter` to the `pce::models::{...}` import; after `FocusedOutcome`:

```rust
/// The app overlay's workloads: the PCE's answer to the label filter, and
/// whether it came from the cache.
#[derive(Debug)]
pub struct AppWorkloadsPayload {
    pub workloads: Vec<Workload>,
    pub truncated: bool,
    pub hit: bool,
}
```

in `Msg`, after `WorkloadsLoaded(...)`:

```rust
    /// The app overlay's workloads (`Cmd::LoadAppWorkloads`) for overlay `seq`.
    AppWorkloadsLoaded {
        seq: u64,
        result: Result<AppWorkloadsPayload, String>,
    },
```

in `Cmd`, after `LoadWorkloads(CacheMode),`:

```rust
    /// The app overlay's workloads: one read filtered by the app's labels.
    LoadAppWorkloads {
        seq: u64,
        filter: WorkloadFilter,
        mode: CacheMode,
    },
```

and its `PartialEq` arm after `(Cmd::LoadWorkloads(a), Cmd::LoadWorkloads(b)) => a == b,`:

```rust
            (
                Cmd::LoadAppWorkloads {
                    seq: a,
                    filter: fa,
                    mode: ma,
                },
                Cmd::LoadAppWorkloads {
                    seq: b,
                    filter: fb,
                    mode: mb,
                },
            ) => a == b && fa == fb && ma == mb,
```

`crates/illumio-tui/src/tui/model.rs`: add to `PendingAction` after `RunFocused(...)`:

```rust
    /// `r` in the app overlay: the workloads and the rulesets again, past the cache.
    ReloadApp,
```

`crates/illumio-tui/src/tui/app.rs`: in `execute`, after the `Cmd::LoadWorkloads(mode) => { ... }` arm:

```rust
            Cmd::LoadAppWorkloads { seq, filter, mode } => {
                self.model.busy += 1;
                let (pce, tx) = self.task_ctx();
                tokio::spawn(async move {
                    let result = pce
                        .workloads(&filter, mode)
                        .await
                        .map(|c| crate::tui::msg::AppWorkloadsPayload {
                            workloads: c.value,
                            truncated: c.truncated,
                            hit: c.hit,
                        })
                        .map_err(|e| e.to_string());
                    let _ = tx.send(Msg::AppWorkloadsLoaded { seq, result });
                    let _ = tx.send(Msg::Stats(pce.stats()));
                });
            }
```

and add `| Msg::AppWorkloadsLoaded { .. }` to `apply`'s `task_result` list.

`crates/illumio-tui/src/tui/update.rs`:

Imports: `use crate::tui::app_view::{AppPanel, AppView, Loadable};`, `AppWorkloadsPayload` in the `crate::tui::msg::{...}` list, `use pce::traffic::{Direction, FocusDirection};` (merge into the existing `pce::traffic` import), `use pce::models::Workload;` (merge into `pce::models::{...}`).

In `Msg::RuleSetsLoaded`'s `Ok((raw, age)) =>` branch, right before its final `deny_fallback(model, mode)` expression, add `refresh_app_policy(model, Ok(age));`; in its `Err(e) =>` branch, after `fail(model, "rulesets", &e, tripped);`, add `refresh_app_policy(model, Err(&e));`.

After the `Msg::WorkloadsLoaded(result) => { ... }` arm add:

```rust
        Msg::AppWorkloadsLoaded { seq, result } => {
            let now = model.workloads.now.unwrap_or_else(jiff::Timestamp::now);
            let tier = model.limits.tier_key.clone();
            let idx = model.index().cloned();
            let Some(v) = model.app_view.as_mut().filter(|v| v.seq == seq) else {
                return Vec::new(); // an earlier overlay's answer
            };
            match (result, idx, v.label_hrefs.clone()) {
                (Ok(p), Some(idx), Some(hrefs)) => {
                    if p.hit {
                        v.cached += 1;
                    }
                    let mine: Vec<Workload> = p
                        .workloads
                        .into_iter()
                        .filter(|w| pce::app::workload_in_app(w, &hrefs))
                        .collect();
                    v.workloads = Loadable::Loaded(pce::app::app_workloads(&mine, &idx, &tier, now));
                    v.workloads_truncated = p.truncated;
                }
                (Err(e), _, _) => v.workloads = Loadable::Failed(e),
                _ => v.workloads = Loadable::Unavailable("the name index is not loaded".into()),
            }
            Vec::new()
        }
```

In `on_key`, replace the Task 3 placeholder `Overlay::App => Vec::new(),` with `Overlay::App => app_overlay_key(model, k),`; in the `Overlay::AppPicker` arm replace the `Pick` branch with:

```rust
            AppPickerOutcome::Pick { app, purpose } => {
                model.overlay = Overlay::None;
                match purpose {
                    AppPickerPurpose::Focus => {
                        focus_by_name(model, &app);
                        Vec::new()
                    }
                    AppPickerPurpose::Overlay => open_app_by_name(model, &app),
                }
            }
```

In the `Overlay::Confirm` `y` arm add `PendingAction::ReloadApp => app_reload(model),`; in its `n` / `N` / `Esc` arm, right after `model.status = StatusLine::info("cancelled");`, add:

```rust
                if action == PendingAction::ReloadApp {
                    model.overlay = Overlay::App;
                }
```

In `traffic_key`'s `match k.code`, after the `KeyCode::Char('F') => return focus_key(model),` arm:

```rust
        KeyCode::Char('A') => return app_open_key(model),
```

In `on_command`, after the `Command::Focus(None) => { ... }` arm:

```rust
        Command::App(Some(name)) => open_app_by_name(model, &name),
        Command::App(None) => {
            open_app_picker(model, AppPickerPurpose::Overlay, "");
            Vec::new()
        }
```

Add these functions after `start_focused`:

```rust
/// `A`: the app overlay for the row's app (the first of `row_apps`, the
/// order `F` takes), at the focus direction (or `both`); on a row without
/// an app, the picker.
fn app_open_key(model: &mut Model) -> Vec<Cmd> {
    let Some(app) = model.traffic.row_apps().into_iter().next() else {
        open_app_picker(model, AppPickerPurpose::Overlay, "");
        return Vec::new();
    };
    let direction = model
        .traffic
        .focus
        .as_ref()
        .map_or(FocusDirection::Both, |f| f.direction);
    let focus = Focus::new(app, model.traffic.keys.clone()).with_direction(direction);
    open_app(model, focus)
}

/// `:app <app>` and the picker: an app of the loaded result, or one whose
/// labels the name index knows (it may have no flows in the window); else
/// the closest loaded app in the status line, nothing sent.
fn open_app_by_name(model: &mut Model, name: &str) -> Vec<Cmd> {
    let keys = model.traffic.keys.clone();
    let apps = model.traffic.loaded_apps();
    let app = pce::traffic::parse_app(name);
    let labelled = model
        .index()
        .is_some_and(|idx| pce::traffic::app_labels(&app, &keys, idx).is_ok());
    if !apps.iter().any(|a| a == name) && !labelled {
        let near = pce::traffic::closest(name, &apps, 3);
        model.status = StatusLine::warn(if near.is_empty() {
            format!("unknown app {name:?} in the loaded result or the labels; nothing sent")
        } else {
            format!("unknown app {name:?}; closest: {}; nothing sent", near.join(", "))
        });
        return Vec::new();
    }
    let direction = model
        .traffic
        .focus
        .as_ref()
        .map_or(FocusDirection::Both, |f| f.direction);
    open_app(model, Focus::new(app, keys).with_direction(direction))
}

/// Opens the overlay for `focus`: the traffic panels from the loaded result
/// (no request); the workloads from the Workloads screen's list when it
/// holds every workload, else one request with the app's labels; the
/// rulesets from the Policy screen when loaded, else one request (none
/// while one is already on its way). Every request is counted.
fn open_app(model: &mut Model, focus: Focus) -> Vec<Cmd> {
    model.app_seq += 1;
    let seq = model.app_seq;
    let mut v = AppView::new(focus.clone(), model.traffic.filter.window.clone(), seq);
    let t = &model.traffic;
    v.rebuild_traffic(t.flows_for(&focus), t.window_bounds(), t.partial_for(&focus));
    let idx = model.index().cloned();
    v.label_hrefs = idx
        .as_ref()
        .and_then(|idx| focus.label_matches(idx).ok())
        .map(|m| pce::app::label_hrefs(&m));
    let mut cmds = Vec::new();
    let why = if idx.is_none() {
        "the name index is not loaded yet".to_string()
    } else {
        format!("{} is not a label group: no workloads or rulesets to match", focus.app_text())
    };
    match (&v.label_hrefs, &idx) {
        (Some(hrefs), Some(idx)) if model.workloads.loaded_at.is_some() && !model.workloads.truncated => {
            let mine: Vec<Workload> = model
                .workloads
                .raw
                .iter()
                .filter(|w| pce::app::workload_in_app(w, hrefs))
                .cloned()
                .collect();
            let now = model.workloads.now.unwrap_or_else(jiff::Timestamp::now);
            v.workloads = Loadable::Loaded(pce::app::app_workloads(&mine, idx, &model.limits.tier_key, now));
        }
        (Some(hrefs), Some(_)) => cmds.push(Cmd::LoadAppWorkloads {
            seq,
            filter: pce::app::app_workload_filter(hrefs),
            mode: CacheMode::Use,
        }),
        _ => v.workloads = Loadable::Unavailable(why.clone()),
    }
    match (&v.label_hrefs, &idx) {
        (Some(hrefs), Some(idx)) if model.policy.loaded_at.is_some() => {
            v.policy = Loadable::Loaded(pce::app::app_policy(&model.policy.raw, hrefs, idx));
        }
        (Some(_), Some(_)) if model.policy.loading => {}
        (Some(_), Some(_)) => {
            v.policy_sent = true;
            cmds.extend(load_rule_sets(model, CacheMode::Use));
        }
        _ => v.policy = Loadable::Unavailable(why),
    }
    v.requests += u32::try_from(cmds.len()).unwrap_or(u32::MAX);
    let n = v.requests;
    model.status = StatusLine::info(format!(
        "{} · {n} request{}",
        v.header_title(),
        if n == 1 { "" } else { "s" }
    ));
    model.app_view = Some(v);
    model.overlay = Overlay::App;
    cmds
}

/// `X` in the overlay: its peers and timeline again for the new direction,
/// from the loaded flows. No request.
fn rebuild_app_traffic(model: &mut Model) {
    let t = &model.traffic;
    if let Some(v) = model.app_view.as_mut() {
        let focus = v.focus.clone();
        v.rebuild_traffic(t.flows_for(&focus), t.window_bounds(), t.partial_for(&focus));
    }
}

/// The overlay's keys: `Esc` / `q` close, `Tab` / `BackTab` panels, `j` /
/// `k` rows, `X` direction, `Enter` opens, `w` drafts a rule, `r` asks to
/// reload. Only `r` (after `y`) and `Enter` (on another screen) can send.
fn app_overlay_key(model: &mut Model, k: KeyEvent) -> Vec<Cmd> {
    let Some(v) = model.app_view.as_mut() else {
        model.overlay = Overlay::None;
        return Vec::new();
    };
    match k.code {
        KeyCode::Esc | KeyCode::Char('q') => model.overlay = Overlay::None,
        KeyCode::Tab => v.panel = v.panel.next(),
        KeyCode::BackTab => v.panel = v.panel.prev(),
        KeyCode::Down | KeyCode::Char('j') => v.move_cursor(1),
        KeyCode::Up | KeyCode::Char('k') => v.move_cursor(-1),
        KeyCode::Char('X') => {
            v.focus.direction = v.focus.direction.next();
            rebuild_app_traffic(model);
        }
        KeyCode::Enter => return app_enter(model),
        KeyCode::Char('w') => return app_rule_key(model),
        KeyCode::Char('r') => {
            if v.label_hrefs.is_none() {
                model.status = StatusLine::info(format!(
                    "nothing to reload: {} is not a label group",
                    v.focus.app_text()
                ));
            } else {
                model.overlay = Overlay::Confirm {
                    reason: format!(
                        "Reload the workloads and the rulesets of {}: 2 requests? (y/n)",
                        v.focus.app_text()
                    ),
                    action: PendingAction::ReloadApp,
                };
            }
        }
        _ => {}
    }
    Vec::new()
}

/// `y` on the reload prompt: both panels past the cache, counted; back to
/// the overlay.
fn app_reload(model: &mut Model) -> Vec<Cmd> {
    model.overlay = Overlay::App;
    let Some(v) = model.app_view.as_mut() else {
        return Vec::new();
    };
    let Some(hrefs) = v.label_hrefs.clone() else {
        return Vec::new();
    };
    let seq = v.seq;
    v.workloads = Loadable::Loading;
    v.policy = Loadable::Loading;
    v.policy_sent = true;
    v.cached = 0;
    let mut cmds = vec![Cmd::LoadAppWorkloads {
        seq,
        filter: pce::app::app_workload_filter(&hrefs),
        mode: CacheMode::Bypass,
    }];
    if !model.policy.loading {
        cmds.extend(load_rule_sets(model, CacheMode::Bypass));
    }
    if let Some(v) = model.app_view.as_mut() {
        v.requests += u32::try_from(cmds.len()).unwrap_or(u32::MAX);
    }
    cmds
}

/// `Enter`: a peer closes the overlay and focuses that pair in Flows; a
/// workload opens its detail on the Workloads screen; a ruleset opens it on
/// the Policy screen (those screens load what they lack, as always).
fn app_enter(model: &mut Model) -> Vec<Cmd> {
    let Some(v) = model.app_view.as_ref() else {
        return Vec::new();
    };
    match v.panel {
        AppPanel::Inbound | AppPanel::Outbound => {
            let Some((side, p)) = v.selected_peer() else {
                return Vec::new();
            };
            let (src, dst) = match side {
                Direction::In => (p.peer.clone(), v.focus.app.clone()),
                Direction::Out => (v.focus.app.clone(), p.peer.clone()),
            };
            let service = p.service.clone();
            let focus = v.focus.clone();
            model.overlay = Overlay::None;
            model.screen = Screen::Traffic;
            let t = &mut model.traffic;
            t.set_focus(focus);
            t.drill = Some(Drill::Service(src, dst, service, Decisions::All));
            t.view = TrafficView::Flows;
            t.rebuild_views();
            model.status = focus_status(&model.traffic);
            Vec::new()
        }
        AppPanel::Workloads => {
            let Some(href) = v.selected_workload().map(|w| w.href.clone()) else {
                return Vec::new();
            };
            model.overlay = Overlay::None;
            let cmds = switch(model, Screen::Workloads);
            let w = &mut model.workloads;
            w.table.select_where(|r| r.workload.href == href);
            w.show_detail = w.table.selected_item().is_some_and(|r| r.workload.href == href);
            if !w.show_detail {
                model.status = StatusLine::info("the Workloads screen is loading: Enter there opens it");
            }
            cmds
        }
        AppPanel::Policy => {
            let Some(href) = v.selected_policy().map(|r| r.href.clone()) else {
                return Vec::new();
            };
            model.overlay = Overlay::None;
            let mut cmds = switch(model, Screen::Policy);
            model.policy.pane = PolicyPane::RuleSets;
            model.policy.rulesets.select_where(|r| r.href == href);
            if let Some(idx) = model.index().cloned() {
                model
                    .policy
                    .rebuild_rules(&idx, model.workloads.impact_source());
                cmds.extend(deny_fallback(model, CacheMode::Use));
            }
            cmds
        }
    }
}

/// `w` on a blocked (or potentially blocked) peer row: the draft allow rule
/// from one of the flows behind it, as `w` on a traffic row (writes armed).
fn app_rule_key(model: &mut Model) -> Vec<Cmd> {
    if writes_refused(model) {
        return Vec::new();
    }
    let Some(v) = model.app_view.as_ref() else {
        return Vec::new();
    };
    let Some((side, p)) = v.selected_peer() else {
        model.status = StatusLine::warn("select a peer row (Inbound or Outbound)");
        return Vec::new();
    };
    if !matches!(p.decision, PolicyDecision::Blocked | PolicyDecision::PotentiallyBlocked) {
        model.status = StatusLine::warn("this row is allowed; nothing to add");
        return Vec::new();
    }
    let (focus, peer, service, decision) = (v.focus.clone(), p.peer.clone(), p.service.clone(), p.decision);
    let flow = model
        .traffic
        .flows_for(&focus)
        .iter()
        .find(|f| {
            let (app_ep, peer_ep) = match side {
                Direction::In => (&f.dst, &f.src),
                Direction::Out => (&f.src, &f.dst),
            };
            f.policy_decision == decision
                && f.service_text() == service
                && pce::traffic::group_key(app_ep, &focus.keys) == focus.app
                && pce::traffic::group_key(peer_ep, &focus.keys) == peer
        })
        .cloned();
    let Some(flow) = flow else {
        model.status = StatusLine::warn("no loaded flow behind this row");
        return Vec::new();
    };
    begin_edit(model, EditIntent::RuleFromFlow(Box::new(flow)))
}

/// The overlay's policy panel from the rulesets that just arrived, or their
/// error. `age` above 0 means the answer came from the cache.
fn refresh_app_policy(model: &mut Model, outcome: Result<u64, &str>) {
    let idx = model.index().cloned();
    let Some(v) = model.app_view.as_mut() else {
        return;
    };
    if !matches!(v.policy, Loadable::Loading) {
        return;
    }
    match (outcome, idx, v.label_hrefs.clone()) {
        (Ok(age), Some(idx), Some(hrefs)) => {
            if v.policy_sent && age > 0 {
                v.cached += 1;
            }
            v.policy = Loadable::Loaded(pce::app::app_policy(&model.policy.raw, &hrefs, &idx));
        }
        (Err(e), _, _) => v.policy = Loadable::Failed(e.to_string()),
        _ => v.policy = Loadable::Unavailable("the name index is not loaded".into()),
    }
}
```

`refresh_app_policy` reads `model.policy.raw` while `model.app_view` is borrowed mutably: both are fields of `model` accessed directly, which the borrow checker accepts. If it does not (an intermediate method borrow), clone `model.policy.raw` into a local before `as_mut()`.

In `answers_fill_the_panels_and_stale_ones_are_dropped` the rulesets answer carries `age` 3: a cache hit, so the header says `2 req · cached`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p illumio-tui --lib app_tests commands focus_tests`
Expected: PASS.

Run: `cargo test -p illumio-tui --test tui_runtime opening_the_app_overlay`
Expected: PASS (each mock is hit exactly once).

- [ ] **Step 5: Gate and commit**

```bash
cargo fmt --all && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add crates/illumio-tui/src/tui/app_picker.rs crates/illumio-tui/src/tui/commands.rs crates/illumio-tui/src/tui/msg.rs crates/illumio-tui/src/tui/model.rs crates/illumio-tui/src/tui/update.rs crates/illumio-tui/src/tui/app.rs crates/illumio-tui/tests/tui_runtime.rs
git commit -m "feat(tui): A and :app open the app overlay; at most two counted requests; Tab, X, Enter, w, r

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

---

### Task 5: Rendering the overlay, and its snapshots

**Files:**
- Create: `crates/illumio-tui/src/tui/ui/widgets/app_overlay.rs`
- Modify: `crates/illumio-tui/src/tui/ui/widgets/mod.rs` (add `pub mod app_overlay;`)
- Modify: `crates/illumio-tui/src/tui/ui/mod.rs` (replace the Task 3 `Overlay::App => {}` arm)
- Test: `crates/illumio-tui/tests/tui_snapshots.rs`
- Snapshots (new): `app_overlay_80x24`, `app_overlay_120x40`, `app_overlay_light_styled_80x24`, `app_overlay_loading_80x24`, `app_overlay_no_flows_80x24`, `app_overlay_workloads_error_80x24`

**Interfaces:**
- Consumes: Task 3's `AppView`, `AppPanel`, `Loadable`; Task 2's `AppPeer`, `AppPolicyRow`, `PolicyReason`, `AppWorkload`, `enforcement_word`; `compact_count`; `Theme::{decision_style, ok, warn, bad, accent, dim_style, base, header_style}`; `panel`; `layout::chrome`.
- Produces (Plan 3 changes `render_timeline` and the `t` hint):

```rust
// tui/ui/widgets/app_overlay.rs
pub fn render_app(model: &Model, f: &mut Frame, area: Rect);
pub fn sparkline(counts: &[u64], width: usize) -> String;
fn render_timeline(v: &AppView, t: &Theme, f: &mut Frame, area: Rect);
```

- [ ] **Step 1: Write the failing tests**

Append to `crates/illumio-tui/tests/tui_snapshots.rs` (add `model_with_window`, `with_policy`, `with_workloads` to the `test_support` import list at the top):

```rust
fn open_app(m: &mut illumio_tui::tui::model::Model, app: &str) {
    use illumio_tui::tui::commands::Command;
    use illumio_tui::tui::msg::Msg;
    illumio_tui::tui::update(m, Msg::Command(Command::App(Some(app.into()))));
}

/// The rich flows in a known 24h window, workloads and rulesets loaded:
/// every panel filled, nothing sent.
fn app_overlay_model() -> illumio_tui::tui::model::Model {
    let mut m = with_policy(with_workloads(model_with_window("24h")));
    open_app(&mut m, "shop/prod");
    m
}

#[test]
fn app_overlay_80x24() {
    let m = app_overlay_model();
    let out = render_to_string(&m, 80, 24);
    for s in [
        "shop/prod · in+out · last 24h",
        "0 req · cached",
        "Workloads 2",
        "enforced 1 · visibility 1",
        "VENs  1 active · 1 stale",
        "Inbound",
        "Outbound",
        "Blocked over time",
        "approx",
        "RS Shop-Prod",
        "host-1.example",
    ] {
        assert!(out.contains(s), "missing {s:?}:\n{out}");
    }
    insta::assert_snapshot!("app_overlay_80x24", out);
}

#[test]
fn app_overlay_120x40() {
    let m = app_overlay_model();
    insta::assert_snapshot!("app_overlay_120x40", render_to_string(&m, 120, 40));
}

#[test]
fn app_overlay_light_styled_80x24() {
    let mut m = app_overlay_model();
    m.set_theme("light");
    insta::assert_snapshot!("app_overlay_light_styled_80x24", render_styled(&m, 80, 24));
}

#[test]
fn app_overlay_loading_80x24() {
    let mut m = model_with_flows();
    open_app(&mut m, "shop/prod");
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("2 req") && out.contains("loading"), "{out}");
    assert!(out.contains("no time range"), "model_with_flows has no range: {out}");
    insta::assert_snapshot!("app_overlay_loading_80x24", out);
}

#[test]
fn app_overlay_no_flows_80x24() {
    let mut m = with_policy(with_workloads(model_with_window("24h")));
    m.traffic.data.as_mut().unwrap().flows.clear();
    m.traffic.rebuild_views();
    open_app(&mut m, "shop/prod");
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("no flows in the window"), "{out}");
    assert!(out.contains("host-1.example"), "workloads still load: {out}");
    insta::assert_snapshot!("app_overlay_no_flows_80x24", out);
}

#[test]
fn app_overlay_workloads_error_80x24() {
    use illumio_tui::tui::msg::Msg;
    let mut m = with_policy(model_with_window("24h"));
    open_app(&mut m, "shop/prod");
    let seq = m.app_view.as_ref().unwrap().seq;
    illumio_tui::tui::update(&mut m, Msg::AppWorkloadsLoaded { seq, result: Err("HTTP 500".into()) });
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("HTTP 500 · r retries"), "{out}");
    assert!(out.contains("RS Shop-Prod"), "the policy panel still renders: {out}");
    insta::assert_snapshot!("app_overlay_workloads_error_80x24", out);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p illumio-tui --test tui_snapshots app_overlay`
Expected: FAIL: the assertions find none of the overlay text (Task 3's arm draws nothing).

- [ ] **Step 3: Write the implementation**

Create `crates/illumio-tui/src/tui/ui/widgets/app_overlay.rs`:

```rust
//! The app overlay (`A` / `:app`): one app on one screen, btop style. A
//! summary of its workloads and VENs, inbound and outbound peers side by
//! side, blocked traffic over time, then its policy and its workloads.
use crate::tui::app_view::{AppPanel, AppView, Loadable};
use crate::tui::model::Model;
use crate::tui::ui::theme::Theme;
use crate::tui::ui::widgets::panel::panel;
use pce::app::{AppPeer, PolicyReason, enforcement_word};
use pce::models::{PolicyDecision, VenHealth};
use pce::traffic::compact_count;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

/// `▁▂▃▅█`: `counts` scaled to the largest, one cell each, at most `width`
/// (the newest buckets kept when they do not fit).
pub fn sparkline(counts: &[u64], width: usize) -> String {
    let max = counts.iter().copied().max().unwrap_or(0).max(1);
    let skip = counts.len().saturating_sub(width);
    counts
        .iter()
        .skip(skip)
        .map(|&v| {
            let i = ((v * 7) / max) as usize;
            ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'][i.min(7)]
        })
        .collect()
}

fn fit(s: &str, w: usize) -> String {
    let n = s.chars().count();
    if n <= w {
        format!("{s:<w$}")
    } else if w == 0 {
        String::new()
    } else {
        format!("{}…", s.chars().take(w - 1).collect::<String>())
    }
}

fn mark(d: PolicyDecision) -> &'static str {
    match d {
        PolicyDecision::Allowed => "✓",
        PolicyDecision::PotentiallyBlocked => "~",
        PolicyDecision::Blocked => "✗",
        PolicyDecision::Unknown => "?",
    }
}

/// The rows of a panel, the cursor's row reversed when the panel has the
/// focus, scrolled so the cursor stays visible.
fn rows_paragraph<'a>(
    lines: Vec<Line<'a>>,
    cursor: usize,
    focused: bool,
    height: usize,
) -> Paragraph<'a> {
    let first = cursor.saturating_sub(height.saturating_sub(1));
    let lines: Vec<Line> = lines
        .into_iter()
        .enumerate()
        .skip(first)
        .take(height)
        .map(|(i, l)| {
            if focused && i == cursor {
                l.patch_style(Style::new().add_modifier(Modifier::REVERSED))
            } else {
                l
            }
        })
        .collect();
    Paragraph::new(lines)
}

fn note<'a>(text: impl Into<String>, t: &Theme) -> Paragraph<'a> {
    Paragraph::new(Line::from(Span::styled(text.into(), t.dim_style())))
}

pub fn render_app(model: &Model, f: &mut Frame, area: Rect) {
    let Some(v) = &model.app_view else {
        return;
    };
    let t = &model.theme;
    let body = crate::tui::ui::layout::chrome(area).map_or(area, |c| c.body);
    f.render_widget(Clear, body);
    let title = v.header_title();
    let meta = v.header_meta();
    let outer = panel(&title, &meta, t, true);
    let inner = outer.inner(body);
    f.render_widget(outer, body);
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(4),
        Constraint::Length(3),
        Constraint::Min(4),
    ])
    .split(inner);
    render_summary(v, t, f, rows[0]);
    let peers = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[1]);
    render_peers(v, AppPanel::Inbound, t, f, peers[0]);
    render_peers(v, AppPanel::Outbound, t, f, peers[1]);
    render_timeline(v, t, f, rows[2]);
    let lower = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[3]);
    render_policy(v, t, f, lower[0]);
    render_workloads(v, t, f, lower[1]);
}

fn render_summary(v: &AppView, t: &Theme, f: &mut Frame, area: Rect) {
    let lines = match &v.workloads {
        Loadable::Loaded((_, s)) => vec![
            Line::from(vec![
                Span::styled(format!("Workloads {}", s.workloads), t.header_style()),
                Span::raw("   "),
                Span::styled(s.enforcement.text(), t.base()),
                Span::styled(
                    if v.workloads_truncated { "  (truncated)" } else { "" },
                    Style::new().fg(t.warn),
                ),
            ]),
            Line::from(vec![
                Span::styled("VENs  ", t.header_style()),
                Span::styled(s.ven_text(), t.base()),
            ]),
        ],
        Loadable::Loading => vec![Line::from(Span::styled("Workloads  loading…", t.dim_style()))],
        Loadable::Failed(e) => vec![Line::from(Span::styled(
            format!("Workloads  {e} · r retries"),
            Style::new().fg(t.bad),
        ))],
        Loadable::Unavailable(why) => vec![Line::from(Span::styled(
            format!("Workloads  {why}"),
            t.dim_style(),
        ))],
    };
    f.render_widget(Paragraph::new(lines), area);
}

fn peer_line<'a>(p: &AppPeer, width: usize, t: &Theme) -> Line<'a> {
    let conns = compact_count(p.connections);
    let svc_w = 9;
    let peer_w = width.saturating_sub(svc_w + conns.len().max(5) + 4);
    Line::from(Span::styled(
        format!(
            "{} {} {} {:>5}",
            fit(&p.peer_text(), peer_w),
            fit(&p.service, svc_w),
            mark(p.decision),
            conns
        ),
        t.decision_style(p.decision),
    ))
}

fn render_peers(v: &AppView, side: AppPanel, t: &Theme, f: &mut Frame, area: Rect) {
    let focused = v.panel == side;
    let title = if v.partial {
        format!("{} · partial", side.title())
    } else {
        side.title().to_string()
    };
    let block = panel(&title, "", t, focused);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let (rows, shown) = match side {
        AppPanel::Inbound => (&v.inbound, v.focus.direction != pce::traffic::FocusDirection::Out),
        _ => (&v.outbound, v.focus.direction != pce::traffic::FocusDirection::In),
    };
    if !shown {
        let other = if side == AppPanel::Inbound { "inbound" } else { "outbound" };
        f.render_widget(note(format!("X shows {other}"), t), inner);
        return;
    }
    if rows.is_empty() {
        f.render_widget(note("no flows in the window", t), inner);
        return;
    }
    let w = inner.width as usize;
    let lines: Vec<Line> = rows.iter().map(|p| peer_line(p, w, t)).collect();
    f.render_widget(
        rows_paragraph(lines, v.cursor[side.index()], focused, inner.height as usize),
        inner,
    );
}

/// The blocked timeline: the approximate one from the loaded flows,
/// labelled `approx`.
fn render_timeline(v: &AppView, t: &Theme, f: &mut Frame, area: Rect) {
    let block = panel("Blocked over time", "", t, false);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let Some(tl) = &v.timeline else {
        f.render_widget(note("no time range in this result", t), inner);
        return;
    };
    if tl.is_quiet() {
        f.render_widget(note("no blocked flows in the window · approx", t), inner);
        return;
    }
    let spark = sparkline(&tl.counts(), (inner.width as usize).saturating_sub(10));
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(spark, Style::new().fg(t.bad)),
            Span::styled("  approx", t.dim_style()),
        ])),
        inner,
    );
}

fn render_policy(v: &AppView, t: &Theme, f: &mut Frame, area: Rect) {
    let focused = v.panel == AppPanel::Policy;
    let block = panel(AppPanel::Policy.title(), "", t, focused);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let rows = match &v.policy {
        Loadable::Loaded(rows) => rows,
        Loadable::Loading => return f.render_widget(note("loading…", t), inner),
        Loadable::Failed(e) => {
            return f.render_widget(
                Paragraph::new(Span::styled(format!("{e} · r retries"), Style::new().fg(t.bad))),
                inner,
            );
        }
        Loadable::Unavailable(why) => return f.render_widget(note(why.clone(), t), inner),
    };
    if rows.is_empty() {
        f.render_widget(note("no ruleset covers this app", t), inner);
        return;
    }
    let w = inner.width as usize;
    let lines: Vec<Line> = rows
        .iter()
        .map(|r| {
            let why = match r.reason {
                PolicyReason::Scope => "",
                PolicyReason::AllWorkloads => " · all",
                PolicyReason::Rules => " · names it",
            };
            let tail = format!("{} rules{why}", r.rules);
            let name_w = w.saturating_sub(tail.len() + 5);
            let style = if r.enabled { t.base() } else { t.dim_style() };
            Line::from(Span::styled(format!("RS {}  {tail}", fit(&r.name, name_w)), style))
        })
        .collect();
    f.render_widget(
        rows_paragraph(lines, v.cursor[AppPanel::Policy.index()], focused, inner.height as usize),
        inner,
    );
}

fn render_workloads(v: &AppView, t: &Theme, f: &mut Frame, area: Rect) {
    let focused = v.panel == AppPanel::Workloads;
    let block = panel(AppPanel::Workloads.title(), "", t, focused);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let rows = match &v.workloads {
        Loadable::Loaded((rows, _)) => rows,
        Loadable::Loading => return f.render_widget(note("loading…", t), inner),
        Loadable::Failed(e) => {
            return f.render_widget(
                Paragraph::new(Span::styled(format!("{e} · r retries"), Style::new().fg(t.bad))),
                inner,
            );
        }
        Loadable::Unavailable(why) => return f.render_widget(note(why.clone(), t), inner),
    };
    if rows.is_empty() {
        f.render_widget(note("no workload carries these labels", t), inner);
        return;
    }
    let w = inner.width as usize;
    // name, role (7), enforcement (10), the dot: four separators.
    let name_w = w.saturating_sub(7 + 10 + 1 + 3);
    let lines: Vec<Line> = rows
        .iter()
        .map(|wl| {
            let dot = match wl.ven {
                VenHealth::Healthy => Style::new().fg(t.ok),
                VenHealth::Warning => Style::new().fg(t.warn),
                VenHealth::Offline | VenHealth::Error => Style::new().fg(t.bad),
                VenHealth::Unmanaged => t.dim_style(),
            };
            Line::from(vec![
                Span::styled(
                    format!(
                        "{} {} {} ",
                        fit(&wl.name, name_w),
                        fit(wl.role.as_deref().unwrap_or("-"), 7),
                        fit(enforcement_word(&wl.enforcement), 10)
                    ),
                    t.base(),
                ),
                Span::styled("●", dot),
            ])
        })
        .collect();
    f.render_widget(
        rows_paragraph(lines, v.cursor[AppPanel::Workloads.index()], focused, inner.height as usize),
        inner,
    );
}
```

`crates/illumio-tui/src/tui/ui/widgets/mod.rs`: add `pub mod app_overlay;`. `crates/illumio-tui/src/tui/ui/mod.rs`: replace `Overlay::App => {}` with

```rust
        Overlay::App => widgets::app_overlay::render_app(model, f, area),
```

If `Line::patch_style` is not available on this ratatui version, build the reversed line by mapping its spans' styles (`Span::patch_style`), which ratatui 0.30 has.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `INSTA_UPDATE=always cargo test -p illumio-tui --test tui_snapshots app_overlay && find crates/illumio-tui/tests/snapshots -name '*.snap.new' -delete`
Expected: PASS, six new snapshot files. Read each one: header title and `req` meta on the outer border; summary lines; Inbound and Outbound side by side with `✓` / `~` / `✗` marks; the timeline sparkline and `approx`; `RS Shop-Prod  4 rules` and `RS Global-Infra  1 rules · all`; the workloads with `●`. Nothing in the 80-column ones may run past the border.

Run: `cargo test -p illumio-tui`
Expected: PASS; no other snapshot changed.

- [ ] **Step 5: Gate and commit**

```bash
cargo fmt --all && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add crates/illumio-tui/src/tui/ui/widgets/app_overlay.rs crates/illumio-tui/src/tui/ui/widgets/mod.rs crates/illumio-tui/src/tui/ui/mod.rs crates/illumio-tui/tests/tui_snapshots.rs crates/illumio-tui/tests/snapshots/
git commit -m "feat(tui): draw the app overlay: workloads, VENs, peers, approx timeline, policy

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

---

### Task 6: CLI `illumio-tui app <app>`

**Files:**
- Create: `crates/illumio-tui/src/cli/app.rs`
- Modify: `crates/illumio-tui/src/cli/mod.rs` (module list; `Command::App`; `run` dispatch)
- Modify: `crates/illumio-tui/src/cli/traffic.rs` (`Fetched`, its fields, `Fetched::meta` and `fetch` become `pub(crate)`)
- Modify: `crates/illumio-tui/src/cli/context.rs` (new `emit_report`)
- Test: `crates/illumio-tui/tests/cli.rs` (`every_flag_has_help_text` gains `app --help`; three new tests)

**Interfaces:**
- Consumes: Plan 1's `DirectionArg`, `FilterArgs` (`app`, `direction`), `ByArg`, `cli_focus`, `fetch`, `Fetched` (`flows`, `window`, `truncated`, `cache`, `queries`), `Meta.queries`; Tasks 1-2 (`app_peers`, `app_workloads`, `app_workload_filter`, `workload_in_app`, `label_hrefs`, `app_policy`, `approx_timeline`, `equal_buckets`, `APPROX_BUCKETS`); `Pce::{workloads, rule_sets, name_index}`; `write_table`, `write_table_csv`, `write_json`, `meta_line`.
- Produces (Plan 3 adds `--timeline` here):

```rust
// cli/app.rs
pub enum SectionArg { Workloads, Inbound, Outbound, Policy, Timeline }
pub struct AppArgs { pub app: String, pub direction: Option<DirectionArg>, pub window: Option<String>,
    pub keys: Vec<String>, pub max_results: Option<u32>, pub active: bool, pub section: Option<SectionArg>, pub yes: bool }
pub struct AppReport { pub app: String, pub direction: FocusDirection, pub workloads: WorkloadsSection,
    pub inbound: Vec<AppPeer>, pub outbound: Vec<AppPeer>, pub policy: Vec<AppPolicyRow>, pub timeline: Timeline }
pub struct WorkloadsSection { pub summary: WorkloadSummary, pub rows: Vec<AppWorkload> }
pub async fn run(ctx: &Context, args: &AppArgs) -> Result<(), CliError>;
// cli/context.rs
pub fn emit_report<T: Serialize>(&self, data: &T, tables: &[(String, &dyn TableRows)], meta: &Meta) -> Result<(), CliError>;
```

- [ ] **Step 1: Write the failing tests**

In `crates/illumio-tui/tests/cli.rs`, add `vec!["app", "--help"],` to the list in `every_flag_has_help_text`, add `query_param` to the `wiremock::matchers` import, and append:

```rust
async fn mount_app(server: &MockServer, posts: u64) {
    mount_traffic(server, posts).await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/workloads"))
        .and(query_param("labels", r#"[["/orgs/5/labels/3","/orgs/5/labels/4"]]"#))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/workloads.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/orgs/5/sec_policy/draft/rule_sets"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(include_str!("../../../fixtures/rule_sets.json"))
                .insert_header("Content-Type", "application/json"),
        )
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn app_prints_one_envelope_with_every_section() {
    let server = MockServer::start().await;
    mount_app(&server, 2).await;
    let out = stdout_json(
        cmd(&server)
            .args(["app", "shop/prod", "--window", "1h"])
            .assert()
            .success(),
    );
    let d = &out["data"];
    assert_eq!(d["app"], "shop/prod");
    assert_eq!(d["direction"], "both");
    assert_eq!(d["workloads"]["summary"]["workloads"], 2);
    assert_eq!(d["workloads"]["summary"]["enforcement"]["enforced"], 1);
    assert_eq!(d["workloads"]["summary"]["enforcement"]["visibility"], 1);
    let rows = d["workloads"]["rows"].as_array().unwrap();
    assert_eq!(rows[0]["name"], "host-1.example");
    assert_eq!(rows[0]["role"], "web");
    assert_eq!(d["inbound"].as_array().unwrap().len(), 3);
    assert_eq!(d["outbound"].as_array().unwrap().len(), 2);
    assert_eq!(d["inbound"][2]["decision"], "blocked");
    let policy: Vec<(&str, &str)> = d["policy"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r["name"].as_str().unwrap(), r["reason"].as_str().unwrap()))
        .collect();
    assert_eq!(policy, vec![("Shop-Prod", "scope"), ("Global-Infra", "all_workloads")]);
    assert_eq!(d["timeline"]["exact"], false);
    assert_eq!(d["timeline"]["buckets"].as_array().unwrap().len(), 24);
    assert_eq!(out["meta"]["queries"], 2);
    assert_eq!(
        out["meta"]["api_calls"],
        4 + 3 + 3 + 1 + 1,
        "4 lists, POST + poll + download per side, workloads, rulesets"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn app_tables_and_csv_print_per_section() {
    let server = MockServer::start().await;
    mount_app(&server, 2).await;
    let table = cmd(&server)
        .args(["app", "shop/prod", "--window", "1h", "--direction", "in", "--table"])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&table.get_output().stdout).to_string();
    for s in [
        "workloads · 2 · enforced 1 · visibility 1 · VENs",
        "inbound · 3",
        "outbound · 0",
        "policy · 2",
        "timeline · approx",
    ] {
        assert!(text.contains(s), "missing {s:?}:\n{text}");
    }
    let csv = cmd(&server)
        .args(["app", "shop/prod", "--window", "1h", "--direction", "out", "--csv", "--section", "outbound"])
        .assert()
        .success();
    let text = csv_stdout(&csv);
    assert!(text.starts_with("peer,service,decision,flows,conns,bytes\r\n"), "{text}");
    assert_eq!(text.lines().count(), 3, "header and two peers: {text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn app_mistakes_exit_2_before_any_request() {
    let server = MockServer::start().await;
    mount_traffic_quiet(&server).await;
    let err = cmd(&server)
        .args(["app", "shop/prod", "--csv"])
        .assert()
        .code(2);
    assert!(String::from_utf8_lossy(&err.get_output().stderr).contains("--csv prints one table: add --section"));
    cmd(&server)
        .args(["app", "shop/prod", "--window", "30d"])
        .assert()
        .code(2);
    let server = MockServer::start().await;
    mount_app(&server, 0).await;
    let err = cmd(&server).args(["app", "shp/prod"]).assert().code(2);
    assert!(String::from_utf8_lossy(&err.get_output().stderr).contains("closest: shop"));
}
```

(`mount_traffic_quiet` mounts every list with `expect(0)`: the `--csv` check and the 30-day cost guard run offline. `mount_app(&server, 0)` lets the labels be read but expects no traffic POST for the unknown app.)

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p illumio-tui --test cli app_`
Expected: FAIL: clap reports "unrecognized subcommand 'app'".

- [ ] **Step 3: Write the implementation**

`crates/illumio-tui/src/cli/traffic.rs`: change `struct Fetched` to `pub(crate) struct Fetched` and each of its fields to `pub(crate)`; `fn meta(&self, ctx: &Context) -> Meta` to `pub(crate) fn meta`; `async fn fetch(` to `pub(crate) async fn fetch(`.

`crates/illumio-tui/src/cli/context.rs`: add `use std::io::Write;` and, after `emit_meta`:

```rust
    /// A report of several tables. JSON: `data` whole. `--table`: each
    /// table under its title line. `--csv`: the tables' full rows (the
    /// caller passes exactly one).
    pub fn emit_report<T: Serialize>(
        &self,
        data: &T,
        tables: &[(String, &dyn TableRows)],
        meta: &Meta,
    ) -> Result<(), CliError> {
        let mut out = std::io::stdout().lock();
        match self.format {
            OutputFormat::Json => write_json(&mut out, data, meta)?,
            OutputFormat::Table => {
                for (i, (title, table)) in tables.iter().enumerate() {
                    if i > 0 {
                        writeln!(out)?;
                    }
                    writeln!(out, "{title}")?;
                    write_table(&mut out, *table)?;
                }
                eprintln!("{}", meta_line(meta));
            }
            OutputFormat::Csv => {
                for (_, table) in tables {
                    write_table_csv(&mut out, *table)?;
                }
                eprintln!("{}", meta_line(meta));
            }
        }
        Ok(())
    }
```

Create `crates/illumio-tui/src/cli/app.rs`:

```rust
//! `illumio-tui app <app>`: one app's profile in one envelope: its
//! workloads (enforcement and VEN health), its inbound and outbound peers
//! by service and decision, the policy that covers it, and blocked traffic
//! over time (approximate: from the flows' last-detected times).
use super::context::{Context, cache_label};
use super::error::CliError;
use super::output::{OutputFormat, TableRows};
use super::traffic::{ByArg, DirectionArg, FilterArgs, cli_focus, fetch};
use clap::{Args, ValueEnum};
use jiff::Timestamp;
use pce::app::{
    AppPeer, AppPolicyRow, AppWorkload, PolicyReason, WorkloadSummary, app_peers, app_policy,
    app_workload_filter, app_workloads, enforcement_word, label_hrefs, workload_in_app,
};
use pce::models::{PolicyScope, Workload};
use pce::traffic::{
    APPROX_BUCKETS, Direction, FocusDirection, Timeline, approx_timeline, equal_buckets,
};
use serde::Serialize;

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionArg {
    /// The workloads, enforcement and VEN health
    Workloads,
    /// The inbound peers
    Inbound,
    /// The outbound peers
    Outbound,
    /// The rulesets covering the app
    Policy,
    /// Blocked traffic over time
    Timeline,
}

#[derive(Args, Debug, Clone, Default)]
pub struct AppArgs {
    /// The app: one value per label key (--keys, else the profile's scope keys), e.g. shop/prod
    pub app: String,
    /// in (the app is the destination), out (the source) or both (default; two queries)
    #[arg(long, value_enum)]
    pub direction: Option<DirectionArg>,
    /// Relative window ending now, e.g. 1h, 24h, 7d (default from profile, 24h)
    #[arg(long)]
    pub window: Option<String>,
    /// Label keys the app is a group of, e.g. app,env (default: the profile's scope keys)
    #[arg(long, value_delimiter = ',')]
    pub keys: Vec<String>,
    /// Row cap per traffic query, up to 200000 (default from profile; > 10000 needs --yes)
    #[arg(long)]
    pub max_results: Option<u32>,
    /// Match the active policy instead of the draft
    #[arg(long)]
    pub active: bool,
    /// With --table or --csv: only this section (--csv needs one)
    #[arg(long, value_enum)]
    pub section: Option<SectionArg>,
    /// Confirm an expensive query (window > 7d or max_results > 10000)
    #[arg(long)]
    pub yes: bool,
}

#[derive(Debug, Serialize)]
pub struct WorkloadsSection {
    pub summary: WorkloadSummary,
    pub rows: Vec<AppWorkload>,
}

#[derive(Debug, Serialize)]
pub struct AppReport {
    pub app: String,
    pub direction: FocusDirection,
    pub workloads: WorkloadsSection,
    pub inbound: Vec<AppPeer>,
    pub outbound: Vec<AppPeer>,
    pub policy: Vec<AppPolicyRow>,
    pub timeline: Timeline,
}

impl TableRows for WorkloadsSection {
    fn headers(&self) -> Vec<&'static str> {
        vec!["name", "role", "enforcement", "ven"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.rows
            .iter()
            .map(|w| {
                vec![
                    w.name.clone(),
                    w.role.clone().unwrap_or_default(),
                    enforcement_word(&w.enforcement).to_string(),
                    w.ven.as_str().to_string(),
                ]
            })
            .collect()
    }
    fn full_headers(&self) -> Vec<&'static str> {
        vec!["href", "name", "role", "enforcement_mode", "ven"]
    }
    fn full_rows(&self) -> Vec<Vec<String>> {
        self.rows
            .iter()
            .map(|w| {
                vec![
                    w.href.clone(),
                    w.name.clone(),
                    w.role.clone().unwrap_or_default(),
                    w.enforcement.clone(),
                    w.ven.as_str().to_string(),
                ]
            })
            .collect()
    }
}

struct Peers<'a>(&'a [AppPeer]);

impl TableRows for Peers<'_> {
    fn headers(&self) -> Vec<&'static str> {
        vec!["peer", "service", "decision", "flows", "conns", "bytes"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.0
            .iter()
            .map(|p| {
                vec![
                    p.peer_text(),
                    p.service.clone(),
                    p.decision.as_str().to_string(),
                    p.flows.to_string(),
                    p.connections.to_string(),
                    p.bytes.to_string(),
                ]
            })
            .collect()
    }
}

struct Policy<'a>(&'a [AppPolicyRow]);

impl TableRows for Policy<'_> {
    fn headers(&self) -> Vec<&'static str> {
        vec!["ruleset", "enabled", "reason", "rules"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.0
            .iter()
            .map(|r| {
                vec![
                    r.name.clone(),
                    r.enabled.to_string(),
                    match r.reason {
                        PolicyReason::Scope => "scope",
                        PolicyReason::AllWorkloads => "all_workloads",
                        PolicyReason::Rules => "rules",
                    }
                    .to_string(),
                    r.rules.to_string(),
                ]
            })
            .collect()
    }
}

struct Buckets<'a>(&'a Timeline);

impl TableRows for Buckets<'_> {
    fn headers(&self) -> Vec<&'static str> {
        vec!["start", "end", "count", "truncated"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.0
            .buckets
            .iter()
            .map(|b| {
                vec![
                    pce::traffic::format_timestamp(b.start),
                    pce::traffic::format_timestamp(b.end),
                    b.count.map(|c| c.to_string()).unwrap_or_default(),
                    b.truncated.to_string(),
                ]
            })
            .collect()
    }
}

pub async fn run(ctx: &Context, args: &AppArgs) -> Result<(), CliError> {
    if ctx.format == OutputFormat::Csv && args.section.is_none() {
        return Err(CliError::Args(
            "--csv prints one table: add --section workloads|inbound|outbound|policy|timeline"
                .into(),
        ));
    }
    let keys = if args.keys.is_empty() {
        ctx.config.scope_keys.clone()
    } else {
        args.keys.clone()
    };
    let filter = FilterArgs {
        window: args.window.clone(),
        max_results: args.max_results,
        yes: args.yes,
        app: Some(args.app.clone()),
        direction: args.direction,
        ..FilterArgs::default()
    };
    let focus = cli_focus(&filter, &keys, ByArg::Pairs)?
        .ok_or_else(|| CliError::Other("--app is set".into()))?;
    let Some(fetched) = fetch(ctx, &filter, Some(&focus)).await? else {
        return Ok(()); // a dry run; `app` has none, kept for completeness
    };
    let focus = focus.focus;
    let idx = ctx.pce.name_index(ctx.cache_mode).await?;
    let hrefs = label_hrefs(&focus.label_matches(&idx).map_err(CliError::Args)?);
    let wl = ctx
        .pce
        .workloads(&app_workload_filter(&hrefs), ctx.cache_mode)
        .await?;
    let scope = if args.active {
        PolicyScope::Active
    } else {
        PolicyScope::Draft
    };
    let rs = ctx.pce.rule_sets(scope, ctx.cache_mode).await?;
    let mine: Vec<Workload> = wl
        .value
        .iter()
        .filter(|w| workload_in_app(w, &hrefs))
        .cloned()
        .collect();
    let (rows, summary) = app_workloads(&mine, &idx, &ctx.config.tier_key, Timestamp::now());
    let bounds = fetched.window.as_ref().and_then(|w| {
        Some((
            w.start.parse::<Timestamp>().ok()?,
            w.end.parse::<Timestamp>().ok()?,
        ))
    });
    let buckets = bounds
        .map(|(s, e)| equal_buckets(s, e, APPROX_BUCKETS))
        .unwrap_or_default();
    let d = focus.direction;
    let report = AppReport {
        app: focus.app_text(),
        direction: d,
        workloads: WorkloadsSection { summary, rows },
        inbound: if d == FocusDirection::Out {
            Vec::new()
        } else {
            app_peers(&fetched.flows, &focus, Direction::In)
        },
        outbound: if d == FocusDirection::In {
            Vec::new()
        } else {
            app_peers(&fetched.flows, &focus, Direction::Out)
        },
        policy: app_policy(&rs.value, &hrefs, &idx),
        timeline: approx_timeline(&fetched.flows, &focus, &buckets),
    };
    let mut meta = fetched.meta(ctx);
    meta.truncated |= wl.truncated || rs.truncated;
    let all_hit = fetched.cache == "hit" && wl.hit && rs.hit;
    meta.cache = cache_label(all_hit, ctx.cache_mode);
    let s = &report.workloads.summary;
    let (inbound, outbound) = (Peers(&report.inbound), Peers(&report.outbound));
    let (policy, buckets) = (Policy(&report.policy), Buckets(&report.timeline));
    let tables: Vec<(SectionArg, String, &dyn TableRows)> = vec![
        (
            SectionArg::Workloads,
            format!(
                "workloads · {} · {} · VENs {}",
                s.workloads,
                s.enforcement.text(),
                s.ven_text()
            ),
            &report.workloads,
        ),
        (
            SectionArg::Inbound,
            format!("inbound · {}", report.inbound.len()),
            &inbound,
        ),
        (
            SectionArg::Outbound,
            format!("outbound · {}", report.outbound.len()),
            &outbound,
        ),
        (
            SectionArg::Policy,
            format!("policy · {}", report.policy.len()),
            &policy,
        ),
        (
            SectionArg::Timeline,
            format!(
                "timeline · {} · blocked {}",
                if report.timeline.exact { "exact" } else { "approx" },
                if report.timeline.exact { "connections" } else { "flow records" }
            ),
            &buckets,
        ),
    ];
    let chosen: Vec<(String, &dyn TableRows)> = tables
        .into_iter()
        .filter(|(sec, _, _)| args.section.is_none_or(|want| want == *sec))
        .map(|(_, title, table)| (title, table))
        .collect();
    ctx.emit_report(&report, &chosen, &meta)
}
```

`crates/illumio-tui/src/cli/mod.rs`: add `pub mod app;` to the module list; add to `pub enum Command` after `Workloads(...)`:

```rust
    /// One app: workloads and VENs, inbound and outbound peers, policy, blocked traffic over time
    App(app::AppArgs),
```

and to `run`'s dispatch: `Command::App(a) => app::run(&ctx, &a).await,`.

`fetched.cache` is a `&'static str` from `cache_label`; comparing it with `"hit"` is the simplest way to fold the three sources into one label.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p illumio-tui --test cli app_ every_flag_has_help_text`
Expected: PASS.

- [ ] **Step 5: Gate and commit**

```bash
cargo fmt --all && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add crates/illumio-tui/src/cli/app.rs crates/illumio-tui/src/cli/mod.rs crates/illumio-tui/src/cli/traffic.rs crates/illumio-tui/src/cli/context.rs crates/illumio-tui/tests/cli.rs
git commit -m "feat(cli): illumio-tui app: workloads, peers, policy and approx timeline in one envelope

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

---

### Task 7: Docs, in-TUI help, tracking, read-only smoke, final gate

**Files:**
- Modify: `README.md` (after the focus paragraph Plan 1 added; CLI examples; a new `app` paragraph after the `traffic groups` / `mesh` paragraph ~280-300)
- Modify: `docs/index.html` (views section ~168; CLI section ~207)
- Modify: `crates/illumio-tui/src/tui/ui/widgets/overlay.rs` (`HELP_LINES`)
- Modify: `tasks/todo.md`
- Snapshots (re-recorded): the help overlay ones
- Create (scratchpad only): `$SCRATCH/app_smoke.sh`, `SCRATCH=/private/tmp/claude-501/-Volumes-MacMini-4TB-alex-extern-git-illumio-tui/f0fc1b9b-52cf-486d-a697-e4a11c8ea13a/scratchpad`

**Interfaces:** documentation only.

- [ ] **Step 1: In-TUI help**

In `HELP_LINES`, replace

```rust
    ("", ":focus <app>  (alone: a picker of the loaded apps)"),
```

with

```rust
    ("", ":focus <app>  (alone: a picker of the loaded apps)"),
    ("", ":app <app>  the app overlay (alone: the picker)"),
```

and after the `("", "Esc clears; truncated: r re-queries the app (1-2 queries)"),` line add:

```rust
    (
        "Traffic  A",
        "app overlay: peers, blocked over time, policy, workloads",
    ),
    (
        "App      Tab j/k X Enter",
        "panel · row · direction · open (peer: its flows)",
    ),
    (
        "App      w r Esc",
        "draft allow rule · reload panels (asks) · back",
    ),
```

Run: `INSTA_UPDATE=always cargo test -p illumio-tui --test tui_snapshots && find crates/illumio-tui/tests/snapshots -name '*.snap.new' -delete`; read the help snapshot diffs.

- [ ] **Step 2: README**

After the paragraph that ends `...\`Esc\` then puts the unfocused result back\n  without a request.` (Plan 1), add:

```text
  `A` (or `:app shop/prod`) opens the app overlay for the row's app, at the focus direction
  (`X` cycles it there too): its workloads with their enforcement modes and VEN health
  (`Workloads 14   enforced 9 · selective 2 · visibility 3`, `VENs 13 active · 1 stale`), its
  inbound and outbound peers by service and decision, blocked traffic over time (`approx`:
  flow records by last-detected time), and the rulesets whose scope covers it or whose rules
  name its labels (draft or active, as on the Policy screen). The traffic panels come from
  the loaded result; the workloads and the rulesets come from their screens when those are
  loaded, else one request each (cached), counted in the overlay's header. `Tab` moves
  between panels; `Enter` on a peer focuses that pair in Flows, on a workload opens its
  detail, on a ruleset opens it on the Policy screen; `w` drafts an allow rule from a blocked
  peer (writes on); `r` reloads the panels after asking; `Esc` returns.
```

In the CLI examples, after the `traffic groups ... --app shop/prod --direction in` line (Plan 1) add:

```text
    illumio-tui app shop/prod --window 24h --json
    illumio-tui app shop/prod --direction in --csv --section inbound > inbound.csv
```

After the paragraph ending `... \`--backfill\` does not combine with \`--app\`.` (Plan 1) add:

```text
`illumio-tui app <app> [--direction in|out|both] [--window W] [--keys K] [--active]` prints one
envelope for one app: `workloads` (`summary` with the enforcement breakdown and the VEN counts,
`rows` with name, role, enforcement mode and VEN health), `inbound` and `outbound` (one row per
peer group, service and decision), `policy` (rulesets by `reason`: `scope`, `all_workloads` or
`rules`, with their rule counts) and `timeline` (`exact: false`: blocked flow records per
bucket of the window, by last-detected time). It costs the focused traffic queries (one per
side), one workloads read filtered by the app's labels and one rulesets read, all cached;
`meta` counts them. `--table` prints each section under a title line; `--csv` prints one,
chosen with `--section workloads|inbound|outbound|policy|timeline`.
```

- [ ] **Step 3: docs/index.html**

After the `<p><code>F</code> focuses every view on one app ...</p>` paragraph (Plan 1) add:

```html
      <p><code>A</code> opens one screen for that app: inbound and outbound peers, its workloads with enforcement modes and VEN health, the rulesets that cover it, and blocked traffic over time. It reuses what is loaded and sends at most two requests, counted in its header.</p>
```

In the CLI section, after the paragraph ending with Plan 1's `--app shop/prod --direction in` sentence, add:

```html
      <p><code>illumio-tui app shop/prod</code> prints the same profile as one JSON envelope for agents: workloads and VENs, peers, policy and the blocked timeline; <code>--table</code> and <code>--csv --section</code> print it per section.</p>
```

- [ ] **Step 4: Tracking**

Append to `tasks/todo.md`:

```markdown
## App focus 2: overlay (plan docs/superpowers/plans/2026-10-13-app-focus-2-overlay.md)
- [ ] Task 1 pce::traffic::timeline
- [ ] Task 2 pce::app
- [ ] Task 3 TUI AppView and model hooks
- [ ] Task 4 A, :app, keys and loads
- [ ] Task 5 rendering and snapshots
- [ ] Task 6 CLI app
- [ ] Task 7 docs, help, smoke, gate
```

- [ ] **Step 5: Final gate and dependency check**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git diff main -- '*Cargo.toml' Cargo.lock
```

Expected: the gate passes; the diff is empty.

- [ ] **Step 6: Read-only smoke on demo100**

Nothing writes, nothing passes `--yes` or `--timeline`, the env file is passed by path and never read. Create `$SCRATCH/app_smoke.sh`:

```bash
#!/usr/bin/env bash
# Read-only smoke of `illumio-tui app` against the test PCE (no --timeline).
set -euo pipefail
BIN="$1"; ENVF="$2"
run() { "$BIN" --env-file "$ENVF" "$@"; }
APP=$(run traffic groups --window 1h --max-results 200 --by in --json | python3 -I -c '
import json, sys
d = json.load(sys.stdin)
apps = ["/".join(s["app"]) for s in d["data"]["sections"]
        if "-" not in s["app"] and not s["app"][0].startswith("ipl:")]
print(apps[0] if apps else "")')
test -n "$APP" || { echo "no label app in the last hour"; exit 1; }
echo "app: $APP"
run app "$APP" --window 1h --max-results 200 --json | python3 -I -c '
import json, sys
d = json.load(sys.stdin)
data, meta = d["data"], d["meta"]
for k in ["workloads", "inbound", "outbound", "policy", "timeline"]:
    assert k in data, k
assert data["timeline"]["exact"] is False
assert meta["queries"] == 2, meta
s = data["workloads"]["summary"]
print("workloads", s["workloads"], "· inbound", len(data["inbound"]), "· outbound", len(data["outbound"]),
      "· rulesets", len(data["policy"]), "·", meta["api_calls"], "api calls")'
run app "$APP" --window 1h --max-results 200 --table >/dev/null
echo "table: ok"
```

```bash
cargo build -p illumio-tui
bash "$SCRATCH/app_smoke.sh" target/debug/illumio-tui ../illumio-mcp-server/.env
```

Expected: the app, one summary line with the counts and `api_calls`, `table: ok`. On a failure stop and debug (superpowers:systematic-debugging); do not loop against the PCE.

- [ ] **Step 7: Review section**

Append `## Review (App focus 2)` to `tasks/todo.md`: what shipped (one line each: pce timeline basics; pce::app; the overlay state, keys and loads; its rendering; `illumio-tui app`), the test count, the empty dependency diff, the smoke's counts and `api_calls`, the statement that nothing was written, and every deviation a task reported.

- [ ] **Step 8: Commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add README.md docs/index.html crates/illumio-tui/src/tui/ui/widgets/overlay.rs crates/illumio-tui/tests/snapshots/ tasks/todo.md
git commit -m "docs: the app overlay (A, :app) and illumio-tui app

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

---

## Self-review

**1. Spec coverage**

| Spec (Part 2) | Requirement | Task (test) |
| --- | --- | --- |
| Opening | `A` on a row with an app (rule of `F`); focus direction or `both`; `X` cycles; `Esc` returns; `:app <app>` | 4 (`a_opens_the_overlay_for_the_row_app_...`, `the_overlay_takes_the_focus_direction_...`, `esc_closes_...`, `app_by_name_...`) |
| Panels | Inbound / Outbound: peer group × service × decision via `group_by_service`, 0 requests | 2 (`peers_are_one_row_per_group_service_and_decision`); 3; 4 |
| | rough timeline, `approx`, flow records by last-detected | 1 (`approx_counts_blocked_flow_records_by_last_detected`); 3 (`the_approx_timeline_spans_the_loaded_window`); 5 (`approx` in `app_overlay_80x24`) |
| | workloads, enforcement, VENs via `WorkloadFilter.labels`, 1 cached request | 2 (`workloads_count_enforcement_and_ven_health`, `the_workload_filter_is_one_and_group_...`); 4 (`a_opens_...`, runtime test) |
| | policy: covering scopes plus rules naming the labels; draft/active as the Policy screen; 0 if cached else 1 | 2 (`policy_lists_covering_scopes_first_...`, `a_scope_covers_only_when_...`); 4 (`loaded_screens_make_the_overlay_free`, `a_rulesets_load_in_flight_is_not_doubled`) |
| Requests | at most two on open, counted in the header | 4 (`a_opens_...`, `the_overlay_never_sends_a_request_it_did_not_count`); 3 (`the_header_counts_requests_...`) |
| | `r` reloads after confirmation | 4 (`r_asks_then_reloads_both_panels_past_the_cache`) |
| Navigation | `Tab`; `Enter` on peer → focus that pair in Flows; on workload → Workloads detail; on ruleset → Policy; `w` on a blocked peer (writes) | 4 (`esc_closes_and_tab_...`, `enter_on_a_peer_...`, `enter_on_a_workload_or_a_ruleset_...`, `w_drafts_an_allow_rule_...`) |
| Workloads panel | one row per workload: name, role, enforcement, VEN; header counts | 2; 5 (`app_overlay_80x24`) |
| CLI | `illumio-tui app <app> [--direction]`; envelope sections; `--table` / `--csv` per section; `meta` requests, bytes, cache | 6 (`app_prints_one_envelope_with_every_section`, `app_tables_and_csv_print_per_section`, `app_mistakes_exit_2_...`) |
| Errors | unknown app in `:app`: closest match, nothing sent | 4 (`app_by_name_...`) |
| | app with no flows: "no flows in the window", workloads and policy still load | 5 (`app_overlay_no_flows_80x24`) |
| | workloads request fails: error in its panel, others render, `r` retries | 4 (`a_failed_workloads_request_...`); 5 (`app_overlay_workloads_error_80x24`) |
| Testing | overlay snapshots; the overlay never sends an uncounted request; `app` envelope shape; smoke `app <app>` without `--timeline` | 5; 4; 6; 7 Step 6 |
| Delivery | docs task | 7 |

The exact timeline (`t`), spikes and `--timeline` / `--yes` for the timeline are Plan 3.

**2. Placeholder scan.** No `TBD`, "similar to", or test-less steps. Two steps name a compiler-dependent fallback (`Line::patch_style`, the `refresh_app_policy` field borrow) and give the exact alternative.

**3. Type consistency.** `Bucket`, `equal_buckets`, `Timeline`, `TimelineBucket`, `approx_timeline`, `APPROX_BUCKETS` (Task 1) are what Tasks 3, 5 and 6 use and what Plan 3 extends. `AppPeer`, `app_peers`, `AppWorkload`, `WorkloadSummary::ven_text`, `EnforcementCounts::text`, `enforcement_word`, `app_workload_filter`, `workload_in_app`, `label_hrefs`, `app_workloads`, `PolicyReason`, `AppPolicyRow`, `app_policy` (Task 2) are used under these names in Tasks 3-6. `AppView`, `AppPanel`, `Loadable`, `Model::{app_view, app_seq}`, `TrafficState::{flows_for, partial_for, window_bounds}`, `with_policy` (Task 3) match Tasks 4-5 and Plan 3. `Cmd::LoadAppWorkloads`, `Msg::AppWorkloadsLoaded`, `AppWorkloadsPayload`, `PendingAction::ReloadApp`, `Command::App`, `AppPickerPurpose::Overlay`, `open_app`, `app_overlay_key`, `rebuild_app_traffic` (Task 4) are what Plan 3 extends. Plan 1 names are used verbatim (see Global Constraints).

**4. Review Focus.** Each of the five lines names its test: Task 3 (truncated base), Task 4 (stale answer, load in flight, name-keyed app), Task 5 (failed workloads).
