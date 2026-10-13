# App Focus (1 of 3): the Focus Predicate, `F` / `X` and `--app` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** One app at a time: `F` (or `:focus <app>`) narrows every traffic view (Flows, App-to-app, Mesh, Blocked, Treemap) to one app's inbound, outbound or both directions of traffic, `X` cycles the direction, `Esc` clears it, the Result header shows `focus shop/prod · in`, changing `g` clears it, and on a truncated result `r` re-queries the app with its labels in the query (two queries for `both`, merged without duplicates). The CLI gains `--app` / `--direction` on `traffic query|groups|mesh`.

**Architecture:** A pure predicate in `pce::traffic::focus` (`Focus::keeps`, next to `group` / `group_by_service`), plus the focused query bodies (`Focus::queries`, `focus_query`) and the merge (`union_flows`). The facade gains cache slots (`Pce::traffic_slot`) so the unfocused result and the two focused ones never evict each other, and `Pce::focused_traffic`, which runs one query per side, resolves, merges and narrows. The TUI applies the focus where the views take their flows today (`TrafficState::view_flows`, under `Decisions::kept`), so `d`, `o`, search and sort work on top of it unchanged; a focused re-query's answer is held in `data` with the unfocused result kept aside (`unfocused`) and put back, without a request, when the focus is cleared. The CLI parses `--app` into the same `Focus` and runs `Pce::focused_traffic`.

**Tech Stack:** Rust 2024 (rust-version 1.88), tokio, ratatui 0.30, crossterm, serde/serde_json, clap 4, jiff, insta (plain snapshot files), wiremock 0.6, assert_cmd. No new dependency.

**Spec:** `docs/superpowers/specs/2026-10-13-app-focus-design.md` (binding). This plan is delivery item 1 ("Focus"). Plans 2 (`2026-10-13-app-focus-2-overlay.md`) and 3 (`2026-10-13-app-focus-3-timeline.md`) build on the interfaces this plan produces; the names below are the ones they use.

**Tasks:** 7, in this order: 1 `pce::traffic::focus`; 2 cache slots and `Pce::focused_traffic`; 3 TUI focus state and `view_flows`; 4 TUI keys `F` / `X` / `Esc`, `g` clears, the app picker, `:focus`, the header badge; 5 TUI truncated-result re-query (`r`); 6 CLI `--app` / `--direction`; 7 docs, help, tracking, read-only smoke, final gate.

## Global Constraints

- Never hammer the PCE. Every request must be caused by the user, throttled, cached, counted in the header/meta, and visible. Nothing refreshes unsolicited. Setting, moving, cycling or clearing the focus sends nothing; every TUI test of those keys asserts `update(...)` returned an empty `Vec<Cmd>`.
- No new dependencies. `git diff main -- '*Cargo.toml' Cargo.lock` must be empty.
- The gate is `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`. Run `cargo fmt --all` first and accept its formatting; the code blocks below may be re-wrapped by `rustfmt`.
- Snapshots are insta files, updated with `INSTA_UPDATE=always` (`INSTA_UPDATE=always cargo test -p illumio-tui --test tui_snapshots`, then `find crates/illumio-tui/tests/snapshots -name '*.snap.new' -delete`). `cargo insta` is not installed. Read every re-recorded `.snap` diff before committing it.
- Live smokes on the demo PCE are read-only. Never pass `--yes`, never press `y`. They run with `--env-file ../illumio-mcp-server/.env`. Never read or print that file.
- Every commit message ends with these two lines:
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx
- Elm-style TUI: `update` is pure and returns `Cmd`s; `Runtime` (`tui/app.rs`) executes them. Pure aggregation lives in `pce::traffic`.
- Edition 2024 clippy may ask for let-chains or iterator rewrites: accept them and list them as deviations in the task report.

## Spec deviations and resolutions (read before Task 1)

- **`Direction` is taken.** `pce::traffic::Direction` already exists (`In` / `Out`, used by every section). The spec's `Focus { direction: Direction }` is `Focus { direction: FocusDirection }` with `In`, `Out`, `Both`; `FocusDirection::sides()` maps it onto the existing `Direction`.
- **`GroupKey` does not exist.** It is introduced as `pub type GroupKey = Vec<String>;` in `pce::traffic::focus`, the type `group_key` already returns.
- **`X` is taken in App-to-app** (`By::prev`). With a focus set, `X` cycles the focus direction on every view, App-to-app included; without a focus, App-to-app keeps `X` = previous state. Open question for Alex in the report.
- **One traffic result per cache.** `Pce::traffic` invalidates every `traffic:` entry on each miss, so the unfocused result and two focused ones would evict each other. Task 2 adds cache slots: the key becomes `traffic:{slot}:{body}` and a miss invalidates only its own slot. `Pce::traffic` is slot `main`; focused queries use `focus:in` / `focus:out`; Plan 3's buckets use `timeline:{i}:in|out`.
- **`Esc` order.** A running backfill's `Esc` (stop after this slice) still comes first, since it is about requests in flight; then `Esc` clears the focus; then today's drill and treemap `Esc`.
- **Backfill and window keys on a focused result.** `+`, `-`, `]`, `[` and `B` work on the unfocused result: if a focused re-query is on screen it is put back first (no request) and the focus keeps narrowing it. The CLI refuses `--backfill` with a focused `--app` (exit 2) instead of sending unfocused slices. Open question for Alex.
- **CLI compatibility of `--app`.** `--app` moves from `GroupsArgs` into the shared `FilterArgs`. A name-keyed group (`unmanaged-nas/-`, `ipl:Corp/-`) has no labels: with `--by in|out|both` it keeps today's behaviour (unfocused query, sections narrowed to it); without `--by` it is exit 2. A label group that does not exist (`pos/PCI`) is now exit 2 with the closest matches (today: exit 0, empty sections, `no app matches` on stderr), as the spec's "Unknown app" requires.
- **Dry run with `--app`.** `data` is an array of request bodies, `in` first (one element for `in` or `out`).

## Review Focus

- **A label value containing `/`** (`app=a/b`): `:focus` and `--app` split on `/`, so such an app cannot be named. Expected: an "unknown app" message with the closest matches, never a wrong focus. Pinned in Task 1 (`parse_app_splits_on_slash_and_an_extra_slot_is_an_error`).
- **The focus on a non-workload group** (`ipl:Corp/-`, an FQDN, an unlabelled host): views narrow locally, but `r` must refuse to query labels that do not exist. Pinned in Task 5 (`a_group_without_labels_is_not_re_queried`).
- **Self-traffic in `both`**: a flow from the app to itself is returned by both focused queries and must count once in every view and in `meta`. Pinned in Task 1 (`union_drops_the_rows_both_queries_returned`) and Task 2 (`focused_traffic_runs_one_query_per_side_in_its_own_slot`).
- **The focus surviving a new result**: after `f`, `]`, `+` or an auto-refresh tick, the focus stays and narrows the new data; a stale focused result must never be shown as if it were the new one. Pinned in Task 5 (`window_and_cap_keys_work_on_the_unfocused_result`).
- **A truncated focused result**: the focused query itself can hit the cap; the header must keep showing `truncated` and `r` must refresh it past the cache rather than run the same cached query. Pinned in Task 5 (`a_truncated_result_offers_r_and_r_sends_one_query_per_side`).

---

### Task 1: `pce::traffic::focus`: the predicate, the direction, the focused queries

**Files:**
- Create: `crates/pce/src/traffic/focus.rs`
- Modify: `crates/pce/src/traffic/mod.rs` (module list and re-exports, whole file 36 lines)
- Modify: `crates/pce/src/traffic/merge.rs:10-36` (`FlowKey` and `flow_key` become `pub(crate)`)
- Test: `crates/pce/src/traffic/focus.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `group_key`, `keys_or_default` (`aggregate.rs`); `Direction` (`sections.rs`); `ResolvedFlow` (`resolved.rs`); `ActorMatch`, `Ref`, `TrafficQuery` (`models`); `NameIndex::{label_href, values_for_key}` (`resolve.rs`); test fixtures `resolved::tests::{flows, rich_flows, index}`.
- Produces (Plans 2 and 3 use these names):

```rust
// pce::traffic::focus, re-exported from pce::traffic
pub type GroupKey = Vec<String>;
pub enum FocusDirection { In, Out, Both }            // Default: Both; Serialize lowercase
impl FocusDirection {
    pub fn as_str(self) -> &'static str;              // in | out | both
    pub fn header_text(self) -> &'static str;         // in | out | in+out
    pub fn next(self) -> Self;                        // in -> out -> both -> in
    pub fn parse(s: &str) -> Option<Self>;
    pub fn sides(self) -> &'static [Direction];       // Both: [In, Out]
}
pub struct Focus { pub app: GroupKey, pub keys: Vec<String>, pub direction: FocusDirection }
impl Focus {
    pub fn new(app: GroupKey, keys: Vec<String>) -> Self;          // direction Both, keys defaulted
    pub fn with_direction(self, direction: FocusDirection) -> Self;
    pub fn keeps(&self, f: &ResolvedFlow) -> bool;
    pub fn app_text(&self) -> String;                               // shop/prod
    pub fn badge(&self) -> String;                                  // focus shop/prod · in
    pub fn same_app(&self, other: &Focus) -> bool;
    pub fn is_label_group(&self) -> bool;
    pub fn label_matches(&self, idx: &NameIndex) -> Result<Vec<ActorMatch>, String>;
    pub fn queries(&self, base: &TrafficQuery, idx: &NameIndex)
        -> Result<Vec<(Direction, TrafficQuery)>, String>;
}
pub fn parse_app(text: &str) -> GroupKey;
pub fn app_labels(app: &[String], keys: &[String], idx: &NameIndex) -> Result<Vec<ActorMatch>, String>;
pub fn focus_query(base: &TrafficQuery, side: Direction, labels: &[ActorMatch]) -> TrafficQuery;
pub fn focus_flows<'a>(focus: Option<&Focus>, flows: &'a [ResolvedFlow]) -> Cow<'a, [ResolvedFlow]>;
pub fn union_flows(first: Vec<ResolvedFlow>, second: Vec<ResolvedFlow>) -> Vec<ResolvedFlow>;
pub fn apps_in(flows: &[ResolvedFlow], keys: &[String]) -> Vec<String>;
pub fn closest(name: &str, candidates: &[String], n: usize) -> Vec<String>;
pub struct FocusedTraffic { pub flows: Vec<ResolvedFlow>, pub truncated: bool,
    pub matches_count: Option<u64>, pub elapsed: Duration, pub hit: bool, pub queries: Vec<TrafficQuery> }
impl FocusedTraffic { pub fn query_count(&self) -> u32; }
// merge.rs
pub(crate) type FlowKey = (...);  pub(crate) fn flow_key(f: &ResolvedFlow) -> FlowKey;
```

- [ ] **Step 1: Write the failing tests**

Create `crates/pce/src/traffic/focus.rs` with only the test module for now (the module body comes in Step 3; the file must exist for `mod focus;` to compile once Step 3 adds it):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::traffic::query::TrafficQueryBuilder;
    use crate::traffic::resolved::tests::{flows, index, rich_flows};
    use serde_json::json;
    use FocusDirection::{Both, In, Out};

    fn k() -> Vec<String> {
        vec!["app".into(), "env".into()]
    }

    fn focus(s: &str, d: FocusDirection) -> Focus {
        Focus::new(parse_app(s), k()).with_direction(d)
    }

    #[test]
    fn keeps_follows_the_direction_and_counts_self_traffic_once() {
        let f = flows();
        let kept = |d| f.iter().filter(|x| focus("shop/prod", d).keeps(x)).count();
        assert_eq!(kept(In), 3, "flow 0 (to itself), the IP list, the NAS");
        assert_eq!(kept(Out), 2, "flow 0 (to itself) and the FQDN");
        assert_eq!(kept(Both), 4, "the union: flow 0 once");
        let rich = rich_flows();
        let n = |d| rich.iter().filter(|x| focus("crm/prod", d).keeps(x)).count();
        assert_eq!((n(In), n(Out), n(Both)), (7, 4, 9));
    }

    #[test]
    fn a_missing_label_key_is_not_the_app() {
        let f = flows();
        let three: Vec<String> = vec!["app".into(), "env".into(), "loc".into()];
        let eu = Focus::new(parse_app("shop/prod/eu"), three.clone());
        assert_eq!(
            f.iter().filter(|x| eu.keeps(x)).count(),
            0,
            "no workload in the flows carries loc: shop/prod/- is not shop/prod/eu"
        );
        let dashed = Focus::new(parse_app("shop/prod/-"), three);
        assert_eq!(f.iter().filter(|x| dashed.keeps(x)).count(), 4);
        assert!(!dashed.is_label_group(), "a - slot has no label to query by");
        assert!(focus("shop/prod", Both).is_label_group());
        assert!(!focus("ipl:Corp/-", Both).is_label_group());
    }

    #[test]
    fn x_cycles_in_out_both_and_a_new_focus_starts_at_both() {
        assert_eq!(Focus::new(parse_app("shop/prod"), k()).direction, Both);
        let mut d = Both;
        let mut seen = Vec::new();
        for _ in 0..3 {
            d = d.next();
            seen.push(d);
        }
        assert_eq!(seen, vec![In, Out, Both]);
        assert_eq!(FocusDirection::parse(" IN "), Some(In));
        assert_eq!(FocusDirection::parse("sideways"), None);
        assert_eq!(Both.sides(), &[Direction::In, Direction::Out]);
        assert_eq!(Out.sides(), &[Direction::Out]);
        assert_eq!(focus("shop/prod", In).badge(), "focus shop/prod · in");
        assert_eq!(Both.header_text(), "in+out");
        assert_eq!(Focus::new(parse_app("shop/prod"), Vec::new()).keys, k(), "no keys: app,env");
        assert!(focus("shop/prod", In).same_app(&focus("shop/prod", Out)));
    }

    #[test]
    fn parse_app_splits_on_slash_and_an_extra_slot_is_an_error() {
        assert_eq!(parse_app(" shop / prod "), vec!["shop", "prod"]);
        let e = app_labels(&parse_app("a/b/prod"), &k(), &index()).unwrap_err();
        assert!(
            e.contains("has 3 values for the 2 keys app,env: write it as <app>/<env>"),
            "{e}"
        );
    }

    #[test]
    fn queries_put_the_app_labels_on_its_side_and_both_needs_two() {
        let idx = index();
        let base = TrafficQueryBuilder::between(
            "2026-10-09T00:00:00Z".parse().unwrap(),
            "2026-10-10T00:00:00Z".parse().unwrap(),
        )
        .source(vec![ActorMatch::IpAddress {
            ip_address: "10.0.0.1".into(),
        }])
        .max_results(500)
        .build()
        .unwrap();
        let labels = json!([
            {"label": {"href": "/orgs/5/labels/3"}},
            {"label": {"href": "/orgs/5/labels/4"}}
        ]);
        let qs = focus("shop/prod", Both).queries(&base, &idx).unwrap();
        assert_eq!(qs.len(), 2);
        assert_eq!(qs[0].0, Direction::In);
        assert_eq!(qs[1].0, Direction::Out);
        let v0 = serde_json::to_value(&qs[0].1).unwrap();
        assert_eq!(v0["destinations"]["include"], json!([labels.clone()]));
        assert_eq!(
            v0["sources"]["include"],
            json!([[{"ip_address": "10.0.0.1"}]]),
            "the user's source filter stays"
        );
        let v1 = serde_json::to_value(&qs[1].1).unwrap();
        assert_eq!(
            v1["sources"]["include"],
            json!([[
                {"ip_address": "10.0.0.1"},
                {"label": {"href": "/orgs/5/labels/3"}},
                {"label": {"href": "/orgs/5/labels/4"}}
            ]]),
            "ANDed into the user's group"
        );
        assert_eq!(v1["destinations"]["include"], json!([[]]));
        for v in [&v0, &v1] {
            assert_eq!(v["start_date"], "2026-10-09T00:00:00Z");
            assert_eq!(v["end_date"], "2026-10-10T00:00:00Z");
            assert_eq!(v["max_results"], 500);
            assert_eq!(
                v["policy_decisions"],
                json!(["allowed", "potentially_blocked", "blocked"])
            );
        }
        assert_eq!(focus("shop/prod", In).queries(&base, &idx).unwrap().len(), 1);
        assert_eq!(focus("shop/prod", Out).queries(&base, &idx).unwrap()[0].0, Direction::Out);
    }

    #[test]
    fn an_unknown_app_names_the_closest_labels() {
        let idx = index();
        let e = focus("shp/prod", Both).label_matches(&idx).unwrap_err();
        assert_eq!(e, "unknown app \"shp/prod\": no app=shp label (closest: shop)");
        let e = focus("shop", Both).label_matches(&idx).unwrap_err();
        assert!(
            e.contains("has 1 value for the 2 keys app,env: write it as <app>/<env>"),
            "{e}"
        );
        let e = focus("unmanaged-nas/-", Both).label_matches(&idx).unwrap_err();
        assert!(
            e.contains("no app=unmanaged-nas label")
                && e.contains("no env label: a - slot cannot be queried"),
            "{e}"
        );
    }

    #[test]
    fn union_drops_the_rows_both_queries_returned() {
        let f = flows();
        let side = |d| -> Vec<ResolvedFlow> {
            f.iter()
                .filter(|x| focus("shop/prod", d).keeps(x))
                .cloned()
                .collect()
        };
        let inbound = side(In);
        let both = union_flows(inbound.clone(), side(Out));
        assert_eq!(both.len(), 4, "flow 0 is in both and counts once");
        assert_eq!(both.iter().map(|x| x.num_connections).sum::<u64>(), 330);
        assert_eq!(&both[..3], &inbound[..], "the first query's rows keep their order");
    }

    #[test]
    fn apps_in_lists_the_workload_groups_and_closest_ranks_them() {
        assert_eq!(apps_in(&flows(), &k()), vec!["shop/prod"]);
        let rich = apps_in(&rich_flows(), &k());
        assert_eq!(
            rich,
            vec![
                "crm/dev", "crm/prod", "erp/dev", "erp/prod", "hr/dev", "hr/prod", "shop/dev",
                "shop/prod"
            ]
        );
        let c = |n: &str| closest(n, &rich, 3);
        assert_eq!(c("crm"), vec!["crm/dev", "crm/prod"]);
        assert_eq!(c("shop/prd"), vec!["shop/prod"]);
        assert_eq!(c("crm/prd"), vec!["crm/prod"]);
        assert!(c("zzzzzz").is_empty());
        assert!(c("").is_empty());
    }

    #[test]
    fn focus_flows_borrows_without_a_focus() {
        let f = flows();
        assert!(matches!(focus_flows(None, &f), Cow::Borrowed(_)));
        assert_eq!(focus_flows(Some(&focus("shop/prod", In)), &f).len(), 3);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Add `pub mod focus;` to `crates/pce/src/traffic/mod.rs` (after `pub mod filter;`) so the file is compiled.

Run: `cargo test -p pce --lib traffic::focus`
Expected: FAIL to compile with "cannot find type `Focus` in this scope" (and `FocusDirection`, `parse_app`, ...).

- [ ] **Step 3: Write the implementation**

In `crates/pce/src/traffic/merge.rs` replace

```rust
type FlowKey = (
```

with

```rust
pub(crate) type FlowKey = (
```

and

```rust
fn flow_key(f: &ResolvedFlow) -> FlowKey {
```

with

```rust
pub(crate) fn flow_key(f: &ResolvedFlow) -> FlowKey {
```

Put this above the test module in `crates/pce/src/traffic/focus.rs`:

```rust
//! App focus: one app (a group under the `g` keys) and the direction of its
//! traffic. A pure predicate over the loaded flows, the focused Explorer
//! queries that spend the cap on the app (one per side, because Explorer
//! ANDs sources and destinations), and their merge. No requests here;
//! `Pce::focused_traffic` runs the queries.
use super::aggregate::{group_key, keys_or_default};
use super::merge::flow_key;
use super::resolved::ResolvedFlow;
use super::sections::Direction;
use crate::models::{ActorMatch, Ref, TrafficQuery};
use crate::resolve::NameIndex;
use serde::Serialize;
use std::borrow::Cow;
use std::collections::{BTreeSet, HashSet};
use std::time::Duration;

/// The values of one group under the keys in force, in key order:
/// `["shop", "prod"]`. What `group_key` returns.
pub type GroupKey = Vec<String>;

/// Which of the app's flows the focus keeps: `In` the flows whose
/// destination is the app, `Out` whose source is, `Both` either.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FocusDirection {
    In,
    Out,
    #[default]
    Both,
}

impl FocusDirection {
    pub fn as_str(self) -> &'static str {
        match self {
            FocusDirection::In => "in",
            FocusDirection::Out => "out",
            FocusDirection::Both => "both",
        }
    }

    /// `in+out` for both: the app overlay's header.
    pub fn header_text(self) -> &'static str {
        match self {
            FocusDirection::In => "in",
            FocusDirection::Out => "out",
            FocusDirection::Both => "in+out",
        }
    }

    /// `X`: in → out → both → in.
    pub fn next(self) -> Self {
        match self {
            FocusDirection::In => FocusDirection::Out,
            FocusDirection::Out => FocusDirection::Both,
            FocusDirection::Both => FocusDirection::In,
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "in" => Some(FocusDirection::In),
            "out" => Some(FocusDirection::Out),
            "both" => Some(FocusDirection::Both),
            _ => None,
        }
    }

    /// The sides a focused query puts the app on, in the order they run.
    pub fn sides(self) -> &'static [Direction] {
        match self {
            FocusDirection::In => &[Direction::In],
            FocusDirection::Out => &[Direction::Out],
            FocusDirection::Both => &[Direction::In, Direction::Out],
        }
    }
}

/// An app plus a direction: what every traffic view is narrowed to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Focus {
    pub app: GroupKey,
    /// The keys `app` is a group of (never empty: `app,env` by default).
    pub keys: Vec<String>,
    pub direction: FocusDirection,
}

impl Focus {
    /// A new focus starts at `both`.
    pub fn new(app: GroupKey, keys: Vec<String>) -> Self {
        Self {
            app,
            keys: keys_or_default(&keys),
            direction: FocusDirection::Both,
        }
    }

    pub fn with_direction(mut self, direction: FocusDirection) -> Self {
        self.direction = direction;
        self
    }

    /// Whether `f` is the app's traffic in this direction. A flow from the
    /// app to itself is kept by `in`, by `out` and (once) by `both`. A side
    /// missing one of the keys is not in the app (its slot is `-`), as in
    /// `group`.
    pub fn keeps(&self, f: &ResolvedFlow) -> bool {
        let dst = || group_key(&f.dst, &self.keys) == self.app;
        let src = || group_key(&f.src, &self.keys) == self.app;
        match self.direction {
            FocusDirection::In => dst(),
            FocusDirection::Out => src(),
            FocusDirection::Both => dst() || src(),
        }
    }

    /// `shop/prod`.
    pub fn app_text(&self) -> String {
        self.app.join("/")
    }

    /// `focus shop/prod · in`: the traffic views' header badge.
    pub fn badge(&self) -> String {
        format!("focus {} · {}", self.app_text(), self.direction.as_str())
    }

    /// The same app under the same keys, whatever the direction.
    pub fn same_app(&self, other: &Focus) -> bool {
        self.app == other.app && self.keys == other.keys
    }

    /// One real label value per key: no `-` slot, no `ipl:` / host name
    /// group. Only such an app can be put into a query.
    pub fn is_label_group(&self) -> bool {
        self.app.len() == self.keys.len()
            && self
                .app
                .iter()
                .all(|v| !v.is_empty() && v != "-" && !v.starts_with("ipl:"))
    }

    /// The app's labels as one AND group, in key order.
    pub fn label_matches(&self, idx: &NameIndex) -> Result<Vec<ActorMatch>, String> {
        app_labels(&self.app, &self.keys, idx)
    }

    /// One query per side of `direction` (`in` first): `base` with the app's
    /// labels on that side (`focus_query`). `Err` names the labels the PCE
    /// does not have, before any request.
    pub fn queries(
        &self,
        base: &TrafficQuery,
        idx: &NameIndex,
    ) -> Result<Vec<(Direction, TrafficQuery)>, String> {
        let labels = self.label_matches(idx)?;
        Ok(self
            .direction
            .sides()
            .iter()
            .map(|&side| (side, focus_query(base, side, &labels)))
            .collect())
    }
}

/// `shop/prod` → `["shop", "prod"]`, each part trimmed. A label value that
/// itself holds `/` cannot be written this way.
pub fn parse_app(text: &str) -> GroupKey {
    text.split('/').map(|s| s.trim().to_string()).collect()
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The label of each of `app`'s values under `keys`, as actor matches. An
/// unknown value is named with up to three of the key's values closest to
/// it; a `-` slot (a missing label) cannot be queried.
pub fn app_labels(
    app: &[String],
    keys: &[String],
    idx: &NameIndex,
) -> Result<Vec<ActorMatch>, String> {
    let text = app.join("/");
    if app.len() != keys.len() {
        let shape: Vec<String> = keys.iter().map(|k| format!("<{k}>")).collect();
        return Err(format!(
            "app {text:?} has {} for the {} {}: write it as {}",
            plural(app.len(), "value", "values"),
            plural(keys.len(), "key", "keys"),
            keys.join(","),
            shape.join("/")
        ));
    }
    let mut labels = Vec::new();
    let mut misses = Vec::new();
    for (key, value) in keys.iter().zip(app) {
        if value == "-" {
            misses.push(format!("no {key} label: a - slot cannot be queried"));
            continue;
        }
        match idx.label_href(key, value) {
            Some(href) => labels.push(ActorMatch::Label {
                label: Ref::new(href),
            }),
            None => {
                let near = closest(value, &idx.values_for_key(key), 3);
                misses.push(if near.is_empty() {
                    format!("no {key}={value} label")
                } else {
                    format!("no {key}={value} label (closest: {})", near.join(", "))
                });
            }
        }
    }
    if misses.is_empty() {
        Ok(labels)
    } else {
        Err(format!("unknown app {text:?}: {}", misses.join("; ")))
    }
}

/// `base` with `labels` ANDed into every include group of `side` (`In`:
/// destinations, `Out`: sources); an empty include becomes `labels` alone.
/// Window, the other side, services, decisions, excludes and the cap stay.
pub fn focus_query(base: &TrafficQuery, side: Direction, labels: &[ActorMatch]) -> TrafficQuery {
    let mut q = base.clone();
    let filter = match side {
        Direction::In => &mut q.destinations,
        Direction::Out => &mut q.sources,
    };
    filter.include.retain(|g| !g.is_empty());
    if filter.include.is_empty() {
        filter.include.push(labels.to_vec());
    } else {
        for group in &mut filter.include {
            for l in labels {
                if !group.contains(l) {
                    group.push(l.clone());
                }
            }
        }
    }
    q
}

/// The flows `focus` keeps, in order; all of them, borrowed, without a focus.
pub fn focus_flows<'a>(focus: Option<&Focus>, flows: &'a [ResolvedFlow]) -> Cow<'a, [ResolvedFlow]> {
    match focus {
        None => Cow::Borrowed(flows),
        Some(f) => Cow::Owned(flows.iter().filter(|x| f.keeps(x)).cloned().collect()),
    }
}

/// Two focused queries' flows as one result: `second`'s rows whose flow
/// (both sides, service, process, user, decision, direction) `first`
/// already holds are dropped, so a flow from the app to itself, which the
/// `in` and the `out` query both return, counts once. Order: `first`, then
/// the rest of `second`.
pub fn union_flows(mut first: Vec<ResolvedFlow>, second: Vec<ResolvedFlow>) -> Vec<ResolvedFlow> {
    let seen: HashSet<_> = first.iter().map(flow_key).collect();
    first.extend(second.into_iter().filter(|f| !seen.contains(&flow_key(f))));
    first
}

/// Every labelled workload group in `flows` under `keys` (`shop/prod`),
/// either side, sorted: what `:focus` and the app picker complete over.
pub fn apps_in(flows: &[ResolvedFlow], keys: &[String]) -> Vec<String> {
    let keys = keys_or_default(keys);
    let mut apps = BTreeSet::new();
    for f in flows {
        for ep in [&f.src, &f.dst] {
            if ep.is_workload() && !ep.labels.is_empty() {
                apps.insert(group_key(ep, &keys).join("/"));
            }
        }
    }
    apps.into_iter().collect()
}

fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = Vec::with_capacity(b.len() + 1);
        cur.push(i + 1);
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != *cb);
            cur.push((prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// Up to `n` of `candidates` closest to `name`, case-insensitively: those
/// containing it first, then by edit distance (at most a quarter of the
/// name's length, at least 2); ties alphabetical. Empty for an empty name.
pub fn closest(name: &str, candidates: &[String], n: usize) -> Vec<String> {
    let needle = name.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let limit = (needle.chars().count() / 4).max(2);
    let mut scored: Vec<(usize, &String)> = candidates
        .iter()
        .filter_map(|c| {
            let lc = c.to_lowercase();
            let score = if lc.contains(&needle) {
                0
            } else {
                edit_distance(&lc, &needle)
            };
            (score <= limit).then_some((score, c))
        })
        .collect();
    scored.sort();
    scored.into_iter().take(n).map(|(_, c)| c.clone()).collect()
}

/// What `Pce::focused_traffic` returns: the merged, narrowed flows of one
/// query per side.
#[derive(Debug, Clone)]
pub struct FocusedTraffic {
    pub flows: Vec<ResolvedFlow>,
    /// Any query hit the cap (or the PCE matched more than it returned).
    pub truncated: bool,
    /// The queries' `matches_count` summed (self-traffic counted per
    /// query); `None` when one did not report it.
    pub matches_count: Option<u64>,
    pub elapsed: Duration,
    /// Every query was answered from the cache.
    pub hit: bool,
    /// The bodies sent (or reused), `in` first.
    pub queries: Vec<TrafficQuery>,
}

impl FocusedTraffic {
    pub fn query_count(&self) -> u32 {
        u32::try_from(self.queries.len()).unwrap_or(u32::MAX)
    }
}
```

Replace `crates/pce/src/traffic/mod.rs`'s module list and re-exports so it reads (the `pub mod focus;` from Step 2 stays, the new `pub use` is added):

```rust
pub mod aggregate;
pub mod coverage;
pub mod filter;
pub mod focus;
pub mod merge;
pub mod mesh;
pub mod poll;
pub mod query;
pub mod resolved;
pub mod sections;
pub mod steps;
pub mod treemap;
```

and after `pub use filter::{FilterExpr, resolve_actor};` add

```rust
pub use focus::{
    Focus, FocusDirection, FocusedTraffic, GroupKey, app_labels, apps_in, closest, focus_flows,
    focus_query, parse_app, union_flows,
};
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p pce --lib traffic::focus`
Expected: PASS, 9 tests. Then `cargo test -p pce` (the merge tests still pass with the visibility change).

- [ ] **Step 5: Gate and commit**

```bash
cargo fmt --all && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add crates/pce/src/traffic/focus.rs crates/pce/src/traffic/mod.rs crates/pce/src/traffic/merge.rs
git commit -m "feat(pce): app focus predicate, direction and focused query bodies

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

---

### Task 2: Traffic cache slots and `Pce::focused_traffic`

**Files:**
- Modify: `crates/pce/src/lib.rs:327-366` (`Pce::traffic` becomes slot `main` of the new `Pce::traffic_slot`; `Pce::focused_traffic` added after it)
- Test: `crates/pce/tests/facade.rs` (two tests appended after `only_the_latest_traffic_result_is_cached`, ~389-477)

**Interfaces:**
- Consumes: Task 1's `Focus::{queries, keeps}`, `union_flows`, `FocusedTraffic`; `traffic::resolve_flows`; `NameIndex`.
- Produces:

```rust
impl Pce {
    /// unchanged signature; now `traffic_slot("main", ...)`
    pub async fn traffic(&self, query: &models::TrafficQuery, mode: CacheMode,
        cancel: &tokio_util::sync::CancellationToken,
        progress: Option<&tokio::sync::mpsc::UnboundedSender<traffic::QueryProgress>>)
        -> Result<Cached<traffic::TrafficResult>>;
    pub async fn traffic_slot(&self, slot: &str, query: &models::TrafficQuery, mode: CacheMode,
        cancel: &tokio_util::sync::CancellationToken,
        progress: Option<&tokio::sync::mpsc::UnboundedSender<traffic::QueryProgress>>)
        -> Result<Cached<traffic::TrafficResult>>;
    /// slots `{slot}:in` / `{slot}:out`; the TUI and CLI pass "focus", Plan 3 "timeline:{i}"
    pub async fn focused_traffic(&self, slot: &str, focus: &traffic::Focus,
        base: &models::TrafficQuery, idx: &NameIndex, mode: CacheMode,
        cancel: &tokio_util::sync::CancellationToken,
        progress: Option<&tokio::sync::mpsc::UnboundedSender<traffic::QueryProgress>>)
        -> Result<traffic::FocusedTraffic>;
}
```

- [ ] **Step 1: Write the failing tests**

Append to `crates/pce/tests/facade.rs`:

```rust
fn fast_poll(p: &mut Pce) {
    p.set_poll_config(pce::traffic::poll::PollConfig {
        initial: Duration::from_millis(1),
        factor: 1.5,
        cap: Duration::from_millis(5),
        grace: Duration::from_millis(100),
    });
}

/// A POST matching `body` answered with query `name`, its status and its
/// download (`flows`); exactly `expect` POSTs.
async fn mount_query(server: &MockServer, name: &str, body: serde_json::Value, flows: &str, expect: u64) {
    use wiremock::matchers::body_partial_json;
    let href = format!("/orgs/5/traffic_flows/async_queries/{name}");
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/traffic_flows/async_queries"))
        .and(body_partial_json(body))
        .respond_with(
            ResponseTemplate::new(202).set_body_json(json!({"href": href, "status": "queued"})),
        )
        .expect(expect)
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{href}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"status": "completed", "result": format!("{href}/download")}),
        ))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{href}/download")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(flows.to_string())
                .insert_header("Content-Type", "application/json"),
        )
        .mount(server)
        .await;
}

fn fixture_index() -> pce::NameIndex {
    let labels: Vec<pce::models::Label> =
        serde_json::from_str(include_str!("../../../fixtures/labels.json")).unwrap();
    pce::NameIndex::new(&labels, &[], &[], &[])
}

fn shop_labels() -> serde_json::Value {
    json!([{"label": {"href": "/orgs/5/labels/3"}}, {"label": {"href": "/orgs/5/labels/4"}}])
}

#[tokio::test]
async fn focused_traffic_runs_one_query_per_side_in_its_own_slot() {
    use pce::traffic::{Focus, FocusDirection, TrafficQueryBuilder};
    use tokio_util::sync::CancellationToken;
    let server = MockServer::start().await;
    let flows = include_str!("../../../fixtures/traffic_flows.json");
    let labels = shop_labels();
    mount_query(&server, "q-in", json!({"destinations": {"include": [labels.clone()]}}), flows, 1).await;
    mount_query(&server, "q-out", json!({"sources": {"include": [labels]}}), flows, 1).await;
    let mut p = pce_for(&server);
    fast_poll(&mut p);
    let idx = fixture_index();
    let base = TrafficQueryBuilder::between(
        "2026-10-09T00:00:00Z".parse().unwrap(),
        "2026-10-10T00:00:00Z".parse().unwrap(),
    )
    .max_results(500)
    .build()
    .unwrap();
    let both = Focus::new(vec!["shop".into(), "prod".into()], vec!["app".into(), "env".into()]);
    let cancel = CancellationToken::new();
    let r = p
        .focused_traffic("focus", &both, &base, &idx, CacheMode::Use, &cancel, None)
        .await
        .unwrap();
    assert_eq!(r.query_count(), 2);
    assert_eq!(r.flows.len(), 4, "flow 0 came back from both queries and counts once");
    assert!(!r.hit);
    let again = p
        .focused_traffic("focus", &both, &base, &idx, CacheMode::Use, &cancel, None)
        .await
        .unwrap();
    assert!(again.hit, "both slots answer from the cache");
    let inbound = both.clone().with_direction(FocusDirection::In);
    let r = p
        .focused_traffic("focus", &inbound, &base, &idx, CacheMode::Use, &cancel, None)
        .await
        .unwrap();
    assert!(r.hit, "the in query is the one `both` ran first");
    assert_eq!(r.flows.len(), 3, "Focus::keeps narrows the in query's answer");
    let nope = Focus::new(vec!["nope".into(), "prod".into()], vec!["app".into(), "env".into()]);
    let e = p
        .focused_traffic("focus", &nope, &base, &idx, CacheMode::Use, &cancel, None)
        .await
        .unwrap_err();
    assert!(
        matches!(&e, pce::PceError::Invalid(m) if m.starts_with("unknown app \"nope/prod\"")),
        "{e:?}"
    );
}

#[tokio::test]
async fn traffic_slots_do_not_evict_each_other() {
    use pce::traffic::{Focus, TrafficQueryBuilder};
    use tokio_util::sync::CancellationToken;
    let server = MockServer::start().await;
    let flows = include_str!("../../../fixtures/traffic_flows.json");
    let labels = shop_labels();
    mount_query(&server, "q-main", json!({"max_results": 501}), "[]", 1).await;
    mount_query(&server, "q-in", json!({"destinations": {"include": [labels.clone()]}}), flows, 1).await;
    mount_query(&server, "q-out", json!({"sources": {"include": [labels]}}), flows, 1).await;
    let mut p = pce_for(&server);
    fast_poll(&mut p);
    let idx = fixture_index();
    let bounds = || {
        TrafficQueryBuilder::between(
            "2026-10-09T00:00:00Z".parse().unwrap(),
            "2026-10-10T00:00:00Z".parse().unwrap(),
        )
    };
    let main = bounds().max_results(501).build().unwrap();
    let base = bounds().max_results(500).build().unwrap();
    let focus = Focus::new(vec!["shop".into(), "prod".into()], vec!["app".into(), "env".into()]);
    let cancel = CancellationToken::new();
    assert!(!p.traffic(&main, CacheMode::Use, &cancel, None).await.unwrap().hit);
    let f = p
        .focused_traffic("focus", &focus, &base, &idx, CacheMode::Use, &cancel, None)
        .await
        .unwrap();
    assert!(!f.hit);
    assert!(
        p.traffic(&main, CacheMode::Use, &cancel, None).await.unwrap().hit,
        "the focused queries left the main slot alone"
    );
    assert!(
        p.focused_traffic("focus", &focus, &base, &idx, CacheMode::Use, &cancel, None)
            .await
            .unwrap()
            .hit,
        "and the main query left the focused slots alone"
    );
}
```

`body_partial_json` compares arrays index by index, so `[[l3, l4]]` does not match a side whose include is `[[]]`; `{"max_results": 501}` keeps the main query apart from the focused ones (500).

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p pce --test facade focused_traffic traffic_slots`
Expected: FAIL to compile with "no method named `focused_traffic` found for struct `Pce`".

- [ ] **Step 3: Write the implementation**

In `crates/pce/src/lib.rs` replace the whole `pub async fn traffic(` method (its doc comment through its closing brace, ~327-366) with:

```rust
    /// Runs (or reuses) one Explorer query in the `main` slot: the
    /// unfocused traffic result (see `traffic_slot`).
    pub async fn traffic(
        &self,
        query: &models::TrafficQuery,
        mode: CacheMode,
        cancel: &tokio_util::sync::CancellationToken,
        progress: Option<&tokio::sync::mpsc::UnboundedSender<traffic::QueryProgress>>,
    ) -> Result<Cached<traffic::TrafficResult>> {
        self.traffic_slot("main", query, mode, cancel, progress)
            .await
    }

    /// Runs (or reuses) one Explorer query in cache slot `slot`. The cache
    /// key is the slot plus the full query body, so the same filter never
    /// runs twice until invalidated. Each slot keeps exactly one result set
    /// (a long TTL stands in for "until the filter changes or the user
    /// refreshes"): the unfocused result (`main`), the focused ones
    /// (`focus:in`, `focus:out`) and a timeline's buckets do not evict
    /// each other.
    pub async fn traffic_slot(
        &self,
        slot: &str,
        query: &models::TrafficQuery,
        mode: CacheMode,
        cancel: &tokio_util::sync::CancellationToken,
        progress: Option<&tokio::sync::mpsc::UnboundedSender<traffic::QueryProgress>>,
    ) -> Result<Cached<traffic::TrafficResult>> {
        let prefix = format!("traffic:{slot}:");
        let key = format!(
            "{prefix}{}",
            serde_json::to_string(query).map_err(|e| PceError::Invalid(e.to_string()))?
        );
        if mode == CacheMode::Use
            && let Some((value, age)) = self.cache.get::<traffic::TrafficResult>(&key)
        {
            let truncated = value.truncated;
            return Ok(Cached {
                value,
                age,
                hit: true,
                truncated,
            });
        }
        let result =
            traffic::run_traffic_query(&self.client, query, &self.poll, cancel, progress).await?;
        self.cache.invalidate_prefix(&prefix);
        self.cache
            .insert(&key, result.clone(), Duration::from_secs(24 * 3600));
        let truncated = result.truncated;
        Ok(Cached {
            value: result,
            age: Duration::ZERO,
            hit: false,
            truncated,
        })
    }

    /// `focus`'s queries over `base` (`Focus::queries`: one per side, `in`
    /// first), each run (or reused) in its own slot `{slot}:in` /
    /// `{slot}:out`, one at a time, resolved with `idx`, merged without
    /// duplicates (`union_flows`) and narrowed by `Focus::keeps`. An app the
    /// labels do not name is `PceError::Invalid` before any request.
    #[allow(clippy::too_many_arguments)]
    pub async fn focused_traffic(
        &self,
        slot: &str,
        focus: &traffic::Focus,
        base: &models::TrafficQuery,
        idx: &NameIndex,
        mode: CacheMode,
        cancel: &tokio_util::sync::CancellationToken,
        progress: Option<&tokio::sync::mpsc::UnboundedSender<traffic::QueryProgress>>,
    ) -> Result<traffic::FocusedTraffic> {
        let queries = focus.queries(base, idx).map_err(PceError::Invalid)?;
        let mut merged: Option<Vec<traffic::ResolvedFlow>> = None;
        let mut out = traffic::FocusedTraffic {
            flows: Vec::new(),
            truncated: false,
            matches_count: Some(0),
            elapsed: Duration::ZERO,
            hit: true,
            queries: Vec::new(),
        };
        for (side, q) in queries {
            let r = self
                .traffic_slot(&format!("{slot}:{}", side.as_str()), &q, mode, cancel, progress)
                .await?;
            let flows = traffic::resolve_flows(&r.value.flows, idx);
            merged = Some(match merged.take() {
                None => flows,
                Some(first) => traffic::union_flows(first, flows),
            });
            out.truncated |= r.truncated;
            out.matches_count = out.matches_count.zip(r.value.matches_count).map(|(a, b)| a + b);
            out.elapsed += r.value.elapsed;
            out.hit &= r.hit;
            out.queries.push(q);
        }
        out.flows = merged
            .unwrap_or_default()
            .into_iter()
            .filter(|f| focus.keeps(f))
            .collect();
        Ok(out)
    }
```

If clippy does not flag `too_many_arguments` (its limit is 7; this method has 8 parameters including `&self`), keep the `#[allow]`; if `cargo clippy` reports it as unneeded, remove it.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p pce --test facade`
Expected: PASS, including the unchanged `only_the_latest_traffic_result_is_cached` and `traffic_is_cached_by_query_and_filters_resolve`.

- [ ] **Step 5: Gate and commit**

```bash
cargo fmt --all && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add crates/pce/src/lib.rs crates/pce/tests/facade.rs
git commit -m "feat(pce): traffic cache slots and focused_traffic, one query per side

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

---

### Task 3: TUI focus state; every view takes `view_flows`

**Files:**
- Modify: `crates/illumio-tui/src/tui/model.rs` (imports 1-28; `TrafficState` fields ~1660-1708 and `new` ~1843-1870; `has_bytes` ~1897; `rebuild_views` ~1921-1939; `rebuild_pairs` ~1942-1954; `apps_rows` ~1984-2003; `blocked_rows` ~2006-2018; new `FocusResult`, `queries_text` and the focus methods; new test module at the end of the file)
- Test: `crates/illumio-tui/src/tui/model.rs` (`#[cfg(test)] mod focus_tests`)

**Interfaces:**
- Consumes: Task 1's `Focus`, `FocusDirection`, `GroupKey`, `focus_flows`, `apps_in`; existing `TrafficState::{loaded_flows, section_table, pairs_table, keys, view}`, `TreemapState::{current, tile, tiles, groups, zoom}`, `Tile::is_other`, `SectionRow::{peer, app}`, `group_key`.
- Produces (Tasks 4-5 and Plans 2-3 use these):

```rust
// tui/model.rs
pub struct FocusResult { pub focus: Focus, pub queries: u32 }
impl FocusResult { pub fn covers(&self, f: &Focus) -> bool; }
pub fn queries_text(n: u32) -> String;                        // "1 query" / "2 queries"
pub struct TrafficState { /* ... */
    pub focus: Option<Focus>,
    pub focus_result: Option<FocusResult>,
    pub unfocused: Option<TrafficData>,
}
impl TrafficState {
    pub fn view_flows(&self) -> Cow<'_, [ResolvedFlow]>;
    pub fn set_focus(&mut self, focus: Focus);
    pub fn drop_focus(&mut self) -> bool;                     // no rebuild
    pub fn clear_focus(&mut self) -> bool;                    // drop_focus + rebuild_views
    pub fn restore_unfocused(&mut self) -> bool;              // no rebuild
    pub fn back_to_unfocused(&mut self) -> bool;              // restore + rebuild when a focused result is shown
    pub fn cycle_focus_direction(&mut self) -> Option<FocusDirection>;
    pub fn apply_focused(&mut self, focus: Focus, queries: u32, data: TrafficData);
    pub fn focus_partial(&self) -> bool;
    pub fn focus_requery(&self) -> Option<Focus>;
    pub fn focus_meta(&self) -> Option<String>;
    pub fn row_apps(&self) -> Vec<GroupKey>;
    pub fn next_focus_app(&self) -> Option<GroupKey>;
    pub fn loaded_apps(&self) -> Vec<String>;
}
```

- [ ] **Step 1: Write the failing tests**

Append to the end of `crates/illumio-tui/src/tui/model.rs`:

```rust
#[cfg(test)]
mod focus_tests {
    use super::*;
    use crate::tui::test_support::{model_with_flows, model_with_rich_flows};
    use pce::traffic::{Focus, FocusDirection};

    fn app(s: &str) -> Vec<String> {
        s.split('/').map(str::to_string).collect()
    }

    fn keys() -> Vec<String> {
        vec!["app".into(), "env".into()]
    }

    fn data_of(t: &TrafficState, n: usize) -> TrafficData {
        TrafficData {
            flows: t.loaded_flows()[..n].to_vec(),
            truncated: false,
            matches_count: Some(n as u64),
            elapsed: Duration::from_millis(900),
            cache_hit: false,
            fetched_at: Instant::now(),
            filter: t.filter.clone(),
            slices: None,
            backfill: None,
        }
    }

    #[test]
    fn the_focus_narrows_every_view_and_clearing_brings_them_back() {
        let mut m = model_with_rich_flows();
        let t = &mut m.traffic;
        assert_eq!(t.flows.len(), 30);
        let crm = app("crm/prod");
        t.set_focus(Focus::new(crm.clone(), keys()).with_direction(FocusDirection::In));
        assert_eq!(t.flows.len(), 7);
        assert!(t.flows.rows().iter().all(|r| r.dst_group == "crm/prod"));
        assert!(!t.groups.is_empty() && t.groups.rows().iter().all(|g| g.dst == crm));
        assert!(!t.mesh.is_empty() && t.mesh.rows().iter().all(|r| r.dst == crm));
        assert!(t.treemap.groups.iter().all(|g| g.dst == crm));
        assert!(
            !t.blocked.is_empty() && t.blocked.rows().iter().all(|r| r.app == crm),
            "flow 6 is blocked into crm/prod"
        );
        assert_eq!(t.cycle_focus_direction(), Some(FocusDirection::Out));
        assert_eq!(t.flows.len(), 4);
        assert_eq!(t.cycle_focus_direction(), Some(FocusDirection::Both));
        assert_eq!(t.flows.len(), 9, "flows 3 and 4 are crm/prod to itself: once");
        assert!(t.clear_focus());
        assert_eq!(t.flows.len(), 30);
        assert!(!t.clear_focus(), "nothing left to clear");
        assert_eq!(t.cycle_focus_direction(), None);
    }

    #[test]
    fn the_decision_filter_and_by_service_work_on_top_of_the_focus() {
        let mut m = model_with_rich_flows();
        let t = &mut m.traffic;
        t.set_focus(Focus::new(app("crm/prod"), keys()));
        t.apps_state.decisions = Decisions::BlockedOnly;
        t.apps_state.by_service = true;
        t.rebuild_pairs();
        let rows: Vec<String> = t
            .services
            .rows()
            .iter()
            .map(|g| format!("{}>{} {}", g.src.join("/"), g.dst.join("/"), g.service.clone().unwrap_or_default()))
            .collect();
        assert_eq!(rows, vec!["crm/dev>crm/prod 3306/tcp"], "the one blocked flow of crm/prod");
    }

    #[test]
    fn f_takes_the_source_then_the_destination_and_in_sections_the_destination_first() {
        let mut m = model_with_flows();
        let t = &mut m.traffic;
        t.flows
            .select_where(|r| r.flow.dst.display_name() == "fqdn-1.example");
        assert_eq!(t.row_apps(), vec![app("shop/prod"), app("fqdn-1.example/-")]);
        assert_eq!(t.next_focus_app(), Some(app("shop/prod")));
        t.set_focus(Focus::new(app("shop/prod"), keys()));
        t.flows
            .select_where(|r| r.flow.dst.display_name() == "fqdn-1.example");
        assert_eq!(
            t.next_focus_app(),
            Some(app("fqdn-1.example/-")),
            "a second F switches sides"
        );
        t.flows.select_where(|r| r.flow.num_connections == 310);
        assert_eq!(t.row_apps(), vec![app("shop/prod")], "a flow to itself has one app");
        t.view = TrafficView::Blocked;
        t.blocked.select_where(|r| !r.is_header());
        assert_eq!(t.row_apps()[0], app("shop/prod"), "Blocked is in: destination first");
        t.blocked.select_where(|r| r.is_header());
        assert_eq!(t.row_apps(), vec![app("shop/prod")], "a header is its app");
        t.view = TrafficView::Mesh;
        t.mesh.home();
        assert_eq!(t.row_apps().len(), 1 + usize::from(t.mesh.selected_item().is_some_and(|r| r.src != r.dst)));
        t.flows.set_rows(Vec::new());
        t.view = TrafficView::Flows;
        assert!(t.row_apps().is_empty());
        assert_eq!(t.next_focus_app(), None);
        assert_eq!(t.loaded_apps(), vec!["shop/prod"]);
    }

    #[test]
    fn a_focus_on_a_truncated_result_is_partial_until_its_focused_query_is_loaded() {
        let mut m = model_with_flows();
        let t = &mut m.traffic;
        t.data.as_mut().unwrap().truncated = true;
        let f = Focus::new(app("shop/prod"), keys());
        t.set_focus(f.clone());
        assert!(t.focus_partial());
        assert_eq!(t.focus_requery(), Some(f.clone()));
        assert_eq!(
            t.focus_meta().as_deref(),
            Some("focus is partial: r re-queries shop/prod · both")
        );
        let focused = data_of(t, 2);
        t.apply_focused(f.clone(), 2, focused);
        assert!(!t.focus_partial());
        assert_eq!(t.focus_meta().as_deref(), Some("focus shop/prod · both · 2 queries"));
        assert_eq!(t.loaded_flows().len(), 2);
        assert_eq!(t.focus_requery(), Some(f.clone()), "r refreshes the focused result");
        assert_eq!(t.cycle_focus_direction(), Some(FocusDirection::In));
        assert!(!t.focus_partial(), "a both result covers in");
        assert!(t.clear_focus());
        assert_eq!(t.loaded_flows().len(), 4, "the unfocused result is back");
        assert!(t.data.as_ref().unwrap().truncated);
        assert!(t.unfocused.is_none() && t.focus_result.is_none());
    }

    #[test]
    fn a_focused_result_of_another_app_goes_back_first_and_window_keys_go_back_too() {
        let mut m = model_with_flows();
        let t = &mut m.traffic;
        t.data.as_mut().unwrap().truncated = true;
        let f = Focus::new(app("shop/prod"), keys()).with_direction(FocusDirection::In);
        let focused = data_of(t, 1);
        t.apply_focused(f, 1, focused);
        assert_eq!(t.loaded_flows().len(), 1);
        t.set_focus(Focus::new(app("unmanaged-nas/-"), keys()));
        assert!(t.unfocused.is_none(), "another app: the unfocused result is back first");
        assert_eq!(t.loaded_flows().len(), 4);
        assert!(t.focus_partial(), "still truncated: partial");
        assert!(!t.back_to_unfocused(), "nothing focused to go back from");
        let f = Focus::new(app("shop/prod"), keys());
        let focused = data_of(t, 2);
        t.apply_focused(f, 2, focused);
        assert!(t.back_to_unfocused());
        assert_eq!(t.loaded_flows().len(), 4);
        assert!(t.focus.is_some(), "the focus stays");
        assert!(t.drop_focus(), "drop_focus reports the focus it dropped");
        assert!(t.focus.is_none() && t.unfocused.is_none());
    }

    #[test]
    fn queries_text_counts() {
        assert_eq!(queries_text(1), "1 query");
        assert_eq!(queries_text(2), "2 queries");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p illumio-tui --lib tui::model::focus_tests`
Expected: FAIL to compile with "no method named `set_focus` found for struct `TrafficState`".

- [ ] **Step 3: Write the implementation**

In the imports at the top of `crates/illumio-tui/src/tui/model.rs`, replace

```rust
use pce::traffic::{
    AppSection, Decisions, Direction, FlowGroup, MeshColumns, MeshKey, MeshRow, PeerRow,
    ResolvedFlow, Totals, app_header, app_sections, app_sections_both, compact_count,
    group_by_labels, group_by_service, group_key, group_text, in_app, in_peer, keys_or_default,
    keys_seen, mesh, mesh_key,
};
```

with

```rust
use pce::traffic::{
    AppSection, Decisions, Direction, FlowGroup, Focus, FocusDirection, GroupKey, MeshColumns,
    MeshKey, MeshRow, PeerRow, ResolvedFlow, Totals, app_header, app_sections, app_sections_both,
    apps_in, compact_count, focus_flows, group_by_labels, group_by_service, group_key, group_text,
    in_app, in_peer, keys_or_default, keys_seen, mesh, mesh_key,
};
use std::borrow::Cow;
```

Above `pub struct TrafficState {` add:

```rust
/// What a focused re-query (`r` on a partial focus) loaded into `TrafficData`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusResult {
    pub focus: Focus,
    /// The Explorer queries it took: 2 for `both`.
    pub queries: u32,
}

impl FocusResult {
    /// Whether this result holds every flow of `f`: the same app under the
    /// same keys, queried for `f`'s direction or for both.
    pub fn covers(&self, f: &Focus) -> bool {
        self.focus.same_app(f)
            && (self.focus.direction == FocusDirection::Both
                || self.focus.direction == f.direction)
    }
}

/// `1 query`, `2 queries`.
pub fn queries_text(n: u32) -> String {
    format!("{n} quer{}", if n == 1 { "y" } else { "ies" })
}
```

In `pub struct TrafficState`, after the `backfill` field add:

```rust
    /// `F` / `:focus`: every view shows only this app's traffic
    /// (`Focus::keeps`) in its direction (`X`). Cleared by `Esc`, `g` and
    /// `:profile`; a new result keeps it.
    pub focus: Option<Focus>,
    /// `data` came from `r` on a partial focus: the focus it was queried
    /// for and the number of queries.
    pub focus_result: Option<FocusResult>,
    /// The unfocused result `focus_result` replaced; put back, without a
    /// request, when the focus is cleared or moves to another app.
    pub unfocused: Option<TrafficData>,
```

In `TrafficState::new`, after `backfill: None,` add:

```rust
            focus: None,
            focus_result: None,
            unfocused: None,
```

Replace `has_bytes`:

```rust
    /// Whether any loaded flow the focus keeps carries a byte count.
    pub fn has_bytes(&self) -> bool {
        self.view_flows().iter().any(ResolvedFlow::has_bytes)
    }
```

Replace the body of `rebuild_views` (keep its doc comment) with:

```rust
    pub fn rebuild_views(&mut self) {
        self.keys = self.resolve_keys();
        let (shown, tree, mesh_rows) = {
            let keys = &self.keys;
            let flows = self.view_flows();
            let shown: Vec<FlowRow> = flows
                .iter()
                .filter(|f| self.drill.as_ref().is_none_or(|d| d.matches(f, keys)))
                .map(|f| FlowRow::new(f.clone(), keys))
                .collect();
            (
                shown,
                group_by_labels(&flows, keys),
                mesh(&flows, keys, self.mesh_cols),
            )
        };
        self.flows.set_rows(shown);
        self.treemap.groups = tree;
        self.mesh.set_rows(mesh_rows);
        self.rebuild_pairs();
        self.rebuild_sections();
        self.relayout_treemap();
    }
```

Replace the body of `rebuild_pairs` (keep its doc comment, and change "from the loaded flows the decision filter keeps" to "from the flows the focus and the decision filter keep") with:

```rust
    pub fn rebuild_pairs(&mut self) {
        let (groups, services) = {
            let st = &self.apps_state;
            let flows = self.view_flows();
            let kept = st.decisions.kept(&flows);
            let groups = group_by_labels(&kept, &self.keys);
            let services = if st.by_service {
                group_by_service(&kept, &self.keys)
            } else {
                Vec::new()
            };
            (groups, services)
        };
        self.groups.set_rows(groups);
        self.services.set_rows(services);
    }
```

In `apps_rows` replace `let flows = self.loaded_flows();` with `let flows = self.view_flows();` and the three `app_sections(flows, ...)` / `app_sections_both(flows, ...)` arguments with `&flows`. In `blocked_rows` replace `self.loaded_flows(),` with `&self.view_flows(),`.

Add these methods to `impl TrafficState` (after `loaded_flows`):

```rust
    /// The loaded flows the focus keeps (all of them, borrowed, without
    /// one): what every view aggregates, under the drill and `d`.
    pub fn view_flows(&self) -> Cow<'_, [ResolvedFlow]> {
        focus_flows(self.focus.as_ref(), self.loaded_flows())
    }

    /// `F` / `:focus`: narrows every view to `focus`. A focused result of
    /// another app is put back first: it holds only that app's flows.
    /// No request.
    pub fn set_focus(&mut self, focus: Focus) {
        if self
            .focus_result
            .as_ref()
            .is_some_and(|r| !r.focus.same_app(&focus))
        {
            self.restore_unfocused();
        }
        self.focus = Some(focus);
        self.rebuild_views();
    }

    /// The focus gone and the unfocused result back, without a rebuild;
    /// whether there was a focus.
    pub fn drop_focus(&mut self) -> bool {
        self.restore_unfocused();
        self.focus.take().is_some()
    }

    /// `Esc`: `drop_focus`, then every view again. No request.
    pub fn clear_focus(&mut self) -> bool {
        let had = self.drop_focus();
        self.rebuild_views();
        had
    }

    /// The unfocused result back in `data` if a focused one replaced it
    /// (no rebuild); whether one was put back.
    pub fn restore_unfocused(&mut self) -> bool {
        self.focus_result = None;
        match self.unfocused.take() {
            Some(d) => {
                self.data = Some(d);
                true
            }
            None => false,
        }
    }

    /// `+`, `-`, `]`, `[` and `B` work on the unfocused result: a focused
    /// one on screen goes back first and every view is rebuilt (the focus
    /// stays and narrows it again). Whether one went back.
    pub fn back_to_unfocused(&mut self) -> bool {
        if self.focus_result.is_none() {
            return false;
        }
        self.restore_unfocused();
        self.rebuild_views();
        true
    }

    /// `X`: the next direction, every view rebuilt; `None` without a focus.
    pub fn cycle_focus_direction(&mut self) -> Option<FocusDirection> {
        let f = self.focus.as_mut()?;
        f.direction = f.direction.next();
        let d = f.direction;
        self.rebuild_views();
        Some(d)
    }

    /// A focused re-query's answer: the unfocused result kept aside (once),
    /// `data` the focused flows, the drill and the zoom dropped (a new
    /// result), every view rebuilt.
    pub fn apply_focused(&mut self, focus: Focus, queries: u32, data: TrafficData) {
        if self.unfocused.is_none() {
            self.unfocused = self.data.take();
        }
        self.data = Some(data);
        self.focus_result = Some(FocusResult {
            focus: focus.clone(),
            queries,
        });
        self.focus = Some(focus);
        self.drill = None;
        self.treemap.zoom = None;
        self.rebuild_views();
    }

    /// The focus may be missing flows: the result it narrows hit the cap
    /// and no focused query covering it is loaded.
    pub fn focus_partial(&self) -> bool {
        let Some(f) = &self.focus else {
            return false;
        };
        match &self.focus_result {
            Some(r) => !r.covers(f),
            None => self.data.as_ref().is_some_and(|d| d.truncated),
        }
    }

    /// What `r` re-queries: the focus while it is partial, or while the
    /// result on screen is its focused query (then `r` refreshes it).
    pub fn focus_requery(&self) -> Option<Focus> {
        let f = self.focus.as_ref()?;
        let shown = self.focus_result.as_ref().is_some_and(|r| r.covers(f));
        (shown || self.focus_partial()).then(|| f.clone())
    }

    /// The Result header's focus part: `focus shop/prod · in`, with `· 2
    /// queries` when a focused query loaded the data, or `focus is partial:
    /// r re-queries shop/prod · in`.
    pub fn focus_meta(&self) -> Option<String> {
        let f = self.focus.as_ref()?;
        if self.focus_partial() {
            return Some(format!(
                "focus is partial: r re-queries {} · {}",
                f.app_text(),
                f.direction.as_str()
            ));
        }
        Some(match &self.focus_result {
            Some(r) if r.covers(f) => format!("{} · {}", f.badge(), queries_text(r.queries)),
            _ => f.badge(),
        })
    }

    /// The apps of the row under the cursor, in the order `F` takes them: a
    /// section header its app; a peer row of `in` sections (App-to-app `in`,
    /// Blocked) the destination, then the source; any other pair, flow,
    /// Mesh row or treemap tile the source, then the destination. A flow
    /// from an app to itself has one. Empty without a row (or on an `other
    /// (N)` tile).
    pub fn row_apps(&self) -> Vec<GroupKey> {
        let keys = &self.keys;
        let mut apps: Vec<GroupKey> = match self.view {
            TrafficView::Flows => self
                .flows
                .selected_item()
                .map(|r| vec![group_key(&r.flow.src, keys), group_key(&r.flow.dst, keys)])
                .unwrap_or_default(),
            TrafficView::Aggregate if self.apps_state.by == By::Pairs => self
                .pairs_table()
                .selected_item()
                .map(|g| vec![g.src.clone(), g.dst.clone()])
                .unwrap_or_default(),
            TrafficView::Aggregate | TrafficView::Blocked => self.section_apps(),
            TrafficView::Mesh => self
                .mesh
                .selected_item()
                .map(|r| vec![r.src.clone(), r.dst.clone()])
                .unwrap_or_default(),
            TrafficView::Treemap => self.treemap_apps(),
        };
        apps.dedup();
        apps
    }

    fn section_apps(&self) -> Vec<GroupKey> {
        let Some(r) = self.section_table().and_then(|t| t.selected_item()) else {
            return Vec::new();
        };
        let Some((direction, row)) = r.peer() else {
            return vec![r.app.clone()];
        };
        let (src, dst) = match direction {
            Direction::In => (row.peer.clone(), r.app.clone()),
            Direction::Out => (r.app.clone(), row.peer.clone()),
        };
        let in_sections = self.view == TrafficView::Blocked || self.apps_state.by == By::In;
        if in_sections {
            vec![dst, src]
        } else {
            vec![src, dst]
        }
    }

    fn treemap_apps(&self) -> Vec<GroupKey> {
        let tm = &self.treemap;
        let Some(target) = tm.current() else {
            return Vec::new();
        };
        let tile = tm.tile(target);
        if tile.is_other() {
            return Vec::new();
        }
        let src_of = |text: &str| {
            tm.groups
                .iter()
                .find(|g| g.src.join("/") == text)
                .map(|g| g.src.clone())
        };
        let dst_of = |text: &str| {
            tm.groups
                .iter()
                .find(|g| g.dst.join("/") == text)
                .map(|g| g.dst.clone())
        };
        match (&tm.zoom, target.inner) {
            (None, None) => src_of(&tile.label).into_iter().collect(),
            (Some(z), _) => [Some(z.clone()), dst_of(&tile.label)]
                .into_iter()
                .flatten()
                .collect(),
            (None, Some(_)) => [src_of(&tm.tiles[target.outer].label), dst_of(&tile.label)]
                .into_iter()
                .flatten()
                .collect(),
        }
    }

    /// The app `F` focuses on the selected row: its first app, or its
    /// second when the focus is already on the first (a second `F` switches
    /// sides). `None` without an app under the cursor.
    pub fn next_focus_app(&self) -> Option<GroupKey> {
        let apps = self.row_apps();
        let first = apps.first()?;
        let on_first = self
            .focus
            .as_ref()
            .is_some_and(|f| &f.app == first && f.keys == self.keys);
        match apps.get(1) {
            Some(second) if on_first => Some(second.clone()),
            _ => Some(first.clone()),
        }
    }

    /// The apps the picker and `:focus` complete over: every labelled
    /// workload group of the unfocused result under the keys in force.
    pub fn loaded_apps(&self) -> Vec<String> {
        let flows = match &self.unfocused {
            Some(d) => d.flows.as_slice(),
            None => self.loaded_flows(),
        };
        apps_in(flows, &self.keys)
    }
```

The `Mesh` assertion in `f_takes_the_source_then_the_destination_and_in_sections_the_destination_first` is written so it holds whichever row sorts first; if clippy or the reviewer prefers a fixed row, select it with `t.mesh.select_where(|r| r.dst != r.src)` and assert `len() == 2`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p illumio-tui --lib tui::model`
Expected: PASS: the 6 new tests and every existing model test (no focus means `view_flows` borrows the loaded flows: nothing else changes).

Run: `cargo test -p illumio-tui`
Expected: PASS; no snapshot changes (nothing renders the focus yet).

- [ ] **Step 5: Gate and commit**

```bash
cargo fmt --all && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add crates/illumio-tui/src/tui/model.rs
git commit -m "feat(tui): focus state; every traffic view aggregates the flows the focus keeps

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

---

### Task 4: `F`, `X`, `Esc`, `g`, the app picker, `:focus` and the header badge

**Files:**
- Create: `crates/illumio-tui/src/tui/app_picker.rs`
- Modify: `crates/illumio-tui/src/tui/mod.rs` (add `pub mod app_picker;`)
- Modify: `crates/illumio-tui/src/tui/model.rs` (`Overlay` enum ~174-226 and its `PartialEq` ~228-298: `AppPicker`)
- Modify: `crates/illumio-tui/src/tui/commands.rs` (`Command::Focus`, `parse`, tests)
- Modify: `crates/illumio-tui/src/tui/update.rs` (imports 1-22; `on_key` overlay arm before `Overlay::None => on_screen_key(model, k),` ~1258; `traffic_key` ~1362-1579; `apply_group_keys` ~1656-1673; `on_command` ~3621-3703; new fns `focus_key`, `open_app_picker`, `focus_on`, `focus_status`, `focus_by_name`; new test module `focus_tests` at the end)
- Modify: `crates/illumio-tui/src/tui/ui/mod.rs:40-68` (dispatch `Overlay::AppPicker`)
- Modify: `crates/illumio-tui/src/tui/ui/widgets/overlay.rs` (new `render_app_picker` after `render_label_key_picker` ~228)
- Modify: `crates/illumio-tui/src/tui/ui/screens/traffic.rs` (`query_meta` ~85-113, `render_stats` ~115-171)
- Test: `crates/illumio-tui/src/tui/app_picker.rs`, `crates/illumio-tui/src/tui/update.rs` (`mod focus_tests`), `crates/illumio-tui/tests/tui_snapshots.rs`
- Snapshots (new): `traffic_focus_badge_80x24`, `traffic_app_picker_80x24`

**Interfaces:**
- Consumes: Task 3's `TrafficState` focus methods; Task 1's `Focus`, `closest`, `GroupKey`; existing `StatusLine`, `centered`, `panel`.
- Produces:

```rust
// tui/app_picker.rs
pub enum AppPickerPurpose { Focus }                      // Plan 2 adds `Overlay`
pub struct AppPicker { pub input: String, pub apps: Vec<String>, pub selected: usize, pub purpose: AppPickerPurpose }
pub enum AppPickerOutcome { Open(AppPicker), Close, Pick { app: String, purpose: AppPickerPurpose } }
impl AppPicker {
    pub fn new(apps: Vec<String>, purpose: AppPickerPurpose) -> Self;
    pub fn with_input(self, input: &str) -> Self;
    pub fn matches(&self) -> Vec<&str>;
    pub fn key(self, k: KeyEvent) -> AppPickerOutcome;
}
// tui/model.rs
pub enum Overlay { /* ... */ AppPicker(AppPicker) }
// tui/commands.rs
pub enum Command { /* ... */ Focus(Option<String>) }    // `:focus [app]`
// tui/update.rs (private; Plan 2 calls them from the same file)
fn focus_key(model: &mut Model) -> Vec<Cmd>;
fn open_app_picker(model: &mut Model, purpose: AppPickerPurpose, input: &str);
fn focus_on(model: &mut Model, app: GroupKey);
fn focus_status(t: &TrafficState) -> StatusLine;
fn focus_by_name(model: &mut Model, name: &str);
// tui/ui/widgets/overlay.rs
pub fn render_app_picker(model: &Model, p: &AppPicker, f: &mut Frame, area: Rect);
```

- [ ] **Step 1: Write the failing tests**

Create `crates/illumio-tui/src/tui/app_picker.rs` with its tests (the code above them comes in Step 3):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn k(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn picker() -> AppPicker {
        AppPicker::new(
            vec!["crm/dev".into(), "crm/prod".into(), "shop/prod".into()],
            AppPickerPurpose::Focus,
        )
    }

    fn open(o: AppPickerOutcome) -> AppPicker {
        match o {
            AppPickerOutcome::Open(p) => p,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn typing_narrows_and_enter_picks_the_selected_match() {
        let p = open(picker().key(k(KeyCode::Char('C'))));
        assert_eq!(p.matches(), vec!["crm/dev", "crm/prod"], "case-insensitive");
        let p = open(p.key(k(KeyCode::Down)));
        assert_eq!(
            p.key(k(KeyCode::Enter)),
            AppPickerOutcome::Pick {
                app: "crm/prod".into(),
                purpose: AppPickerPurpose::Focus
            }
        );
    }

    #[test]
    fn tab_completes_up_wraps_backspace_widens_and_esc_closes() {
        let p = open(picker().with_input("prod").key(k(KeyCode::Up)));
        assert_eq!(p.selected, 1, "Up from the first wraps to the last");
        let p = open(p.key(k(KeyCode::Tab)));
        assert_eq!(p.input, "shop/prod");
        let p = open(p.key(k(KeyCode::Backspace)));
        assert_eq!(p.input, "shop/pro");
        assert_eq!(picker().key(k(KeyCode::Esc)), AppPickerOutcome::Close);
    }

    #[test]
    fn enter_without_a_match_picks_the_text_and_empty_text_stays_open() {
        let p = picker().with_input("nope");
        assert!(p.matches().is_empty());
        assert_eq!(
            p.key(k(KeyCode::Enter)),
            AppPickerOutcome::Pick {
                app: "nope".into(),
                purpose: AppPickerPurpose::Focus
            }
        );
        let p = AppPicker::new(Vec::new(), AppPickerPurpose::Focus);
        assert!(matches!(p.key(k(KeyCode::Enter)), AppPickerOutcome::Open(_)));
    }
}
```

In `crates/illumio-tui/src/tui/commands.rs`, add to `parses_every_documented_command`:

```rust
        assert_eq!(parse("focus").unwrap(), Command::Focus(None));
        assert_eq!(
            parse("focus shop/prod").unwrap(),
            Command::Focus(Some("shop/prod".into()))
        );
```

Append to the end of `crates/illumio-tui/src/tui/update.rs`:

```rust
#[cfg(test)]
mod focus_tests {
    use super::*;
    use crate::tui::model::{Model, Overlay, TrafficView};
    use crate::tui::test_support::{model_fixture, model_with_flows, model_with_rich_flows};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
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

    fn focus_cmd(name: &str) -> Msg {
        Msg::Command(Command::Focus(Some(name.into())))
    }

    fn focused(m: &Model) -> Option<(String, FocusDirection)> {
        m.traffic
            .focus
            .as_ref()
            .map(|f| (f.app_text(), f.direction))
    }

    #[test]
    fn x_and_esc_cycle_and_clear_the_focus_on_every_view_without_requests() {
        for view in [
            TrafficView::Flows,
            TrafficView::Aggregate,
            TrafficView::Mesh,
            TrafficView::Blocked,
            TrafficView::Treemap,
        ] {
            let mut m = model_with_rich_flows();
            m.traffic.view = view;
            quiet(&mut m, focus_cmd("crm/prod"));
            assert_eq!(
                focused(&m),
                Some(("crm/prod".into(), FocusDirection::Both)),
                "{view:?}"
            );
            quiet(&mut m, key('X'));
            assert_eq!(focused(&m).unwrap().1, FocusDirection::In, "{view:?}");
            let t = &m.traffic;
            let crm = |k: &Vec<String>| k.join("/") == "crm/prod";
            match view {
                TrafficView::Flows => {
                    assert_eq!(t.flows.len(), 7);
                    assert!(t.flows.rows().iter().all(|r| r.dst_group == "crm/prod"));
                }
                TrafficView::Aggregate => {
                    assert!(!t.groups.is_empty() && t.groups.rows().iter().all(|g| crm(&g.dst)))
                }
                TrafficView::Mesh => {
                    assert!(!t.mesh.is_empty() && t.mesh.rows().iter().all(|r| crm(&r.dst)))
                }
                TrafficView::Blocked => {
                    assert!(!t.blocked.is_empty() && t.blocked.rows().iter().all(|r| crm(&r.app)))
                }
                TrafficView::Treemap => assert!(
                    !t.treemap.tiles.is_empty() && t.treemap.groups.iter().all(|g| crm(&g.dst))
                ),
            }
            quiet(&mut m, key('X'));
            assert_eq!(focused(&m).unwrap().1, FocusDirection::Out);
            quiet(&mut m, key('X'));
            assert_eq!(focused(&m).unwrap().1, FocusDirection::Both);
            quiet(&mut m, kc(KeyCode::Esc));
            assert_eq!(focused(&m), None, "{view:?}");
            assert_eq!(m.traffic.flows.len(), 30, "{view:?}: every flow again");
            assert_eq!(m.status.text, "focus cleared");
        }
    }

    #[test]
    fn without_a_focus_x_and_esc_keep_their_old_meaning() {
        let mut m = model_with_rich_flows();
        quiet(&mut m, key('v'));
        assert_eq!(m.traffic.view, TrafficView::Aggregate);
        quiet(&mut m, key('X'));
        assert_eq!(m.traffic.apps_state.by, By::Both, "X steps back from pairs");
        m.traffic.view = TrafficView::Flows;
        m.traffic.drill = Some(Drill::Pair(
            vec!["crm".into(), "prod".into()],
            vec!["crm".into(), "prod".into()],
            Decisions::All,
        ));
        m.traffic.rebuild_views();
        quiet(&mut m, focus_cmd("crm/prod"));
        quiet(&mut m, kc(KeyCode::Esc));
        assert!(m.traffic.drill.is_some(), "the first Esc clears the focus only");
        quiet(&mut m, kc(KeyCode::Esc));
        assert!(m.traffic.drill.is_none(), "then Esc clears the drill as before");
    }

    #[test]
    fn f_on_a_row_takes_its_source_then_its_destination() {
        let mut m = model_with_flows();
        m.traffic
            .flows
            .select_where(|r| r.flow.dst.display_name() == "fqdn-1.example");
        quiet(&mut m, key('F'));
        assert_eq!(focused(&m), Some(("shop/prod".into(), FocusDirection::Both)));
        assert_eq!(m.status.text, "focus shop/prod · both · 4 flows");
        m.traffic
            .flows
            .select_where(|r| r.flow.dst.display_name() == "fqdn-1.example");
        quiet(&mut m, key('F'));
        assert_eq!(focused(&m).unwrap().0, "fqdn-1.example/-");
        quiet(&mut m, kc(KeyCode::Esc));
        m.traffic.view = TrafficView::Blocked;
        m.traffic.blocked.select_where(|r| !r.is_header());
        quiet(&mut m, key('F'));
        assert_eq!(focused(&m).unwrap().0, "shop/prod", "Blocked: the destination first");
    }

    #[test]
    fn f_without_a_row_opens_the_picker_or_clears_and_the_picker_focuses() {
        let mut m = model_with_flows();
        m.traffic.flows.set_rows(Vec::new());
        quiet(&mut m, key('F'));
        let Overlay::AppPicker(p) = &m.overlay else {
            panic!("{:?}", m.overlay)
        };
        assert_eq!(p.apps, vec!["shop/prod"]);
        quiet(&mut m, key('s'));
        quiet(&mut m, key('q'));
        quiet(&mut m, kc(KeyCode::Backspace));
        assert!(!m.quit, "q is typed into the picker");
        quiet(&mut m, kc(KeyCode::Enter));
        assert_eq!(m.overlay, Overlay::None);
        assert_eq!(focused(&m).unwrap().0, "shop/prod");
        m.traffic.flows.set_rows(Vec::new());
        quiet(&mut m, key('F'));
        assert_eq!(focused(&m), None, "F on nothing with a focus set clears it");
        quiet(&mut m, Msg::Command(Command::Focus(None)));
        assert!(matches!(m.overlay, Overlay::AppPicker(_)));
        quiet(&mut m, kc(KeyCode::Esc));
        assert_eq!(m.overlay, Overlay::None);
        assert_eq!(focused(&m), None);
    }

    #[test]
    fn focus_names_an_unknown_app_and_its_closest_match_and_sends_nothing() {
        let mut m = model_with_rich_flows();
        quiet(&mut m, focus_cmd("crm/prd"));
        assert_eq!(focused(&m), None);
        assert_eq!(
            m.status.text,
            "unknown app \"crm/prd\"; closest: crm/prod; nothing sent"
        );
        let mut m = model_fixture();
        quiet(&mut m, focus_cmd("crm/prod"));
        assert_eq!(m.status.text, "no result to focus yet: r runs the query");
        quiet(&mut m, key('F'));
        assert_eq!(m.status.text, "no result to focus yet: r runs the query");
    }

    #[test]
    fn focus_from_another_screen_shows_the_traffic_screen() {
        let mut m = model_with_rich_flows();
        m.screen = Screen::Workloads;
        quiet(&mut m, focus_cmd("crm/prod"));
        assert_eq!(m.screen, Screen::Traffic);
    }

    #[test]
    fn g_clears_the_focus_and_says_so() {
        let mut m = model_with_rich_flows();
        quiet(&mut m, focus_cmd("crm/prod"));
        apply_group_keys(&mut m, vec!["app".into()], false);
        assert_eq!(focused(&m), None);
        assert_eq!(
            m.status.text,
            "grouped by app · focus cleared: the app is not defined under the new keys"
        );
        assert_eq!(m.traffic.flows.len(), 30);
        apply_group_keys(&mut m, vec!["app".into(), "env".into()], false);
        assert_eq!(m.status.text, "grouped by app,env", "no focus, nothing to say");
    }
}
```

Append to `crates/illumio-tui/tests/tui_snapshots.rs`:

```rust
#[test]
fn traffic_focus_badge_80x24() {
    use illumio_tui::tui::commands::Command;
    use illumio_tui::tui::msg::Msg;
    let mut m = model_with_flows();
    illumio_tui::tui::update(&mut m, Msg::Command(Command::Focus(Some("shop/prod".into()))));
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("focus shop/prod · both"), "{out}");
    assert!(out.contains("4 of 4 flows"), "{out}");
    insta::assert_snapshot!("traffic_focus_badge_80x24", out);
}

#[test]
fn traffic_app_picker_80x24() {
    use illumio_tui::tui::commands::Command;
    use illumio_tui::tui::msg::Msg;
    let mut m = model_with_flows();
    illumio_tui::tui::update(&mut m, Msg::Command(Command::Focus(None)));
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("Focus on an app") && out.contains("shop/prod"), "{out}");
    insta::assert_snapshot!("traffic_app_picker_80x24", out);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Add `pub mod app_picker;` to `crates/illumio-tui/src/tui/mod.rs` (after `pub mod app;`).

Run: `cargo test -p illumio-tui --lib app_picker focus_tests commands`
Expected: FAIL to compile with "cannot find type `AppPicker`" and "no variant named `Focus` found for enum `Command`".

- [ ] **Step 3: Write the implementation**

Put above the tests in `crates/illumio-tui/src/tui/app_picker.rs`:

```rust
//! The app picker: `F` on a row without an app, or `:focus` alone. Pure:
//! a text line narrows the apps of the loaded result; `Enter` picks.
use crossterm::event::{KeyCode, KeyEvent};

/// What the picked app is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppPickerPurpose {
    /// `F` / `:focus`: narrow every traffic view to it.
    Focus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPicker {
    pub input: String,
    /// Every app of the loaded result (`TrafficState::loaded_apps`).
    pub apps: Vec<String>,
    /// Index into `matches()`.
    pub selected: usize,
    pub purpose: AppPickerPurpose,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppPickerOutcome {
    Open(AppPicker),
    Close,
    /// The selected match, or the typed text when nothing matches (the
    /// caller reports an unknown app).
    Pick {
        app: String,
        purpose: AppPickerPurpose,
    },
}

impl AppPicker {
    pub fn new(apps: Vec<String>, purpose: AppPickerPurpose) -> Self {
        Self {
            input: String::new(),
            apps,
            selected: 0,
            purpose,
        }
    }

    pub fn with_input(mut self, input: &str) -> Self {
        self.input = input.to_string();
        self.selected = 0;
        self
    }

    /// The apps containing the typed text, case-insensitively, in order.
    pub fn matches(&self) -> Vec<&str> {
        let needle = self.input.trim().to_lowercase();
        self.apps
            .iter()
            .filter(|a| a.to_lowercase().contains(&needle))
            .map(String::as_str)
            .collect()
    }

    fn selected_match(&self) -> Option<String> {
        self.matches().get(self.selected).map(|s| s.to_string())
    }

    pub fn key(mut self, k: KeyEvent) -> AppPickerOutcome {
        match k.code {
            KeyCode::Esc => AppPickerOutcome::Close,
            KeyCode::Enter => {
                let app = self
                    .selected_match()
                    .unwrap_or_else(|| self.input.trim().to_string());
                if app.is_empty() {
                    return AppPickerOutcome::Open(self);
                }
                AppPickerOutcome::Pick {
                    app,
                    purpose: self.purpose,
                }
            }
            KeyCode::Down | KeyCode::Up => {
                let n = self.matches().len();
                if n > 0 {
                    self.selected = if k.code == KeyCode::Down {
                        (self.selected + 1) % n
                    } else {
                        (self.selected + n - 1) % n
                    };
                }
                AppPickerOutcome::Open(self)
            }
            KeyCode::Tab => {
                if let Some(m) = self.selected_match() {
                    self.input = m;
                    self.selected = 0;
                }
                AppPickerOutcome::Open(self)
            }
            KeyCode::Backspace => {
                self.input.pop();
                self.selected = 0;
                AppPickerOutcome::Open(self)
            }
            KeyCode::Char(c) => {
                self.input.push(c);
                self.selected = 0;
                AppPickerOutcome::Open(self)
            }
            _ => AppPickerOutcome::Open(self),
        }
    }
}
```

In `crates/illumio-tui/src/tui/model.rs`: add `use crate::tui::app_picker::AppPicker;` to the imports; add to `pub enum Overlay` after `GroupKeys(KeyPicker),`:

```rust
    /// `F` without an app under the cursor, `:focus` alone: the apps of the
    /// loaded result.
    AppPicker(AppPicker),
```

and in `impl PartialEq for Overlay`, after the `(Overlay::FilterEditor(a), Overlay::FilterEditor(b)) => a == b,` arm:

```rust
            (Overlay::AppPicker(a), Overlay::AppPicker(b)) => a == b,
```

In `crates/illumio-tui/src/tui/commands.rs` add to `pub enum Command` after `Window(String),`:

```rust
    /// `:focus <app>` narrows every traffic view to one app; `:focus` alone
    /// opens the picker.
    Focus(Option<String>),
```

and in `parse`, before `"window" =>`:

```rust
        "focus" => Ok(Command::Focus((!arg.is_empty()).then(|| arg.to_string()))),
```

In `crates/illumio-tui/src/tui/update.rs`:

Imports: add `use crate::tui::app_picker::{AppPicker, AppPickerOutcome, AppPickerPurpose};`, add `TrafficState` to the `crate::tui::model::{...}` list, and add `use pce::traffic::{Focus, GroupKey};` (merge with the existing `use pce::traffic::{Decisions, QueryProgress};` into `use pce::traffic::{Decisions, Focus, GroupKey, QueryProgress};`).

In `on_key`, before `Overlay::None => on_screen_key(model, k),`:

```rust
        Overlay::AppPicker(picker) => match picker.key(k) {
            AppPickerOutcome::Open(p) => {
                model.overlay = Overlay::AppPicker(p);
                Vec::new()
            }
            AppPickerOutcome::Close => {
                model.overlay = Overlay::None;
                Vec::new()
            }
            AppPickerOutcome::Pick { app, purpose } => {
                model.overlay = Overlay::None;
                match purpose {
                    AppPickerPurpose::Focus => focus_by_name(model, &app),
                }
                Vec::new()
            }
        },
```

In `traffic_key`, right after the backfill `Esc` block (`if k.code == KeyCode::Esc && let Some(bf) = ... { ... return Vec::new(); }`) add:

```rust
    // With a focus set, Esc clears it first; today's Esc applies after.
    if k.code == KeyCode::Esc && model.traffic.focus.is_some() {
        model.traffic.clear_focus();
        model.status = StatusLine::info("focus cleared");
        return Vec::new();
    }
```

and in its `match k.code {`, right after the `KeyCode::Char('f') => { ... }` arm:

```rust
        KeyCode::Char('F') => return focus_key(model),
        KeyCode::Char('X') if model.traffic.focus.is_some() => {
            model.traffic.cycle_focus_direction();
            model.status = focus_status(&model.traffic);
        }
```

(These arms come before the Blocked `x | X | d` and the App-to-app `x | X` arms, so a focus takes `X` everywhere.)

Replace `apply_group_keys`:

```rust
/// Enter in the picker: the keys (the profile's scope keys when none),
/// every view rebuilt, the drill-down dropped, the focus cleared (its app
/// is not defined under other keys). No request.
fn apply_group_keys(model: &mut Model, keys: Vec<String>, all: bool) {
    let t = &mut model.traffic;
    t.all_keys = all;
    if !all {
        t.group_keys = if keys.is_empty() {
            pce::traffic::keys_or_default(&model.limits.scope_keys)
        } else {
            keys
        };
    }
    let cleared = t.drop_focus();
    t.drill = None;
    t.treemap.zoom = None;
    t.rebuild_views();
    let mut text = format!("grouped by {}", t.effective_keys().join(","));
    if cleared {
        text.push_str(" · focus cleared: the app is not defined under the new keys");
    }
    model.status = StatusLine::info(text);
}
```

Add after `apply_group_keys`:

```rust
/// `F`: focus on the selected row's app (a second `F` takes its other
/// side); with no app under the cursor, clear the focus, or open the picker
/// when there is none. Never requests.
fn focus_key(model: &mut Model) -> Vec<Cmd> {
    if model.traffic.data.is_none() {
        model.status = StatusLine::info("no result to focus yet: r runs the query");
        return Vec::new();
    }
    match model.traffic.next_focus_app() {
        Some(app) => focus_on(model, app),
        None if model.traffic.focus.is_some() => {
            model.traffic.clear_focus();
            model.status = StatusLine::info("focus cleared");
        }
        None => open_app_picker(model, AppPickerPurpose::Focus, ""),
    }
    Vec::new()
}

/// The picker over the apps of the loaded result.
fn open_app_picker(model: &mut Model, purpose: AppPickerPurpose, input: &str) {
    model.overlay =
        Overlay::AppPicker(AppPicker::new(model.traffic.loaded_apps(), purpose).with_input(input));
}

/// A new focus on `app` under the keys in force, at `both`. No request.
fn focus_on(model: &mut Model, app: GroupKey) {
    let focus = Focus::new(app, model.traffic.keys.clone());
    model.traffic.set_focus(focus);
    model.status = focus_status(&model.traffic);
}

/// `focus shop/prod · in · 7 flows`; on a partial focus also what `r`
/// would send (asked once, here).
fn focus_status(t: &TrafficState) -> StatusLine {
    let Some(f) = &t.focus else {
        return StatusLine::info("focus cleared");
    };
    let n = t.view_flows().len();
    if t.focus_partial() {
        let q = u32::try_from(f.direction.sides().len()).unwrap_or(2);
        StatusLine::warn(format!(
            "{} · {n} flows · the result is truncated: r re-queries {} ({})",
            f.badge(),
            f.app_text(),
            crate::tui::model::queries_text(q)
        ))
    } else {
        StatusLine::info(format!("{} · {n} flows", f.badge()))
    }
}

/// `:focus <app>` and the picker: an app of the loaded result by its text
/// (`shop/prod`), else the closest ones in the status line. Shows the
/// Traffic screen. Never requests.
fn focus_by_name(model: &mut Model, name: &str) {
    if model.traffic.data.is_none() {
        model.status = StatusLine::info("no result to focus yet: r runs the query");
        return;
    }
    let apps = model.traffic.loaded_apps();
    if apps.iter().any(|a| a == name) {
        model.screen = Screen::Traffic;
        focus_on(model, pce::traffic::parse_app(name));
        return;
    }
    let near = pce::traffic::closest(name, &apps, 3);
    model.status = StatusLine::warn(if near.is_empty() {
        format!("unknown app {name:?} in the loaded result; nothing sent")
    } else {
        format!("unknown app {name:?}; closest: {}; nothing sent", near.join(", "))
    });
}
```

In `on_command`, before `Command::Profile(name) =>`:

```rust
        Command::Focus(Some(name)) => {
            focus_by_name(model, &name);
            Vec::new()
        }
        Command::Focus(None) => {
            if model.traffic.data.is_none() {
                model.status = StatusLine::info("no result to focus yet: r runs the query");
            } else {
                model.screen = Screen::Traffic;
                open_app_picker(model, AppPickerPurpose::Focus, "");
            }
            Vec::new()
        }
```

In `crates/illumio-tui/src/tui/ui/mod.rs` add after the `Overlay::GroupKeys(picker) => ...` arm:

```rust
        Overlay::AppPicker(p) => widgets::overlay::render_app_picker(model, p, f, area),
```

In `crates/illumio-tui/src/tui/ui/widgets/overlay.rs` add `use crate::tui::app_picker::AppPicker;` and, after `render_label_key_picker`:

```rust
/// `F` without an app, `:focus` alone: the typed text and the apps it keeps.
pub fn render_app_picker(model: &Model, p: &AppPicker, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let matches = p.matches();
    let shown = matches.len().clamp(1, 10) as u16;
    let r = centered(area, area.width.min(60), shown + 4);
    f.render_widget(Clear, r);
    let block = panel("Focus on an app", "Enter · Tab completes · Esc", t, true);
    let mut lines = vec![
        Line::from(vec![
            Span::styled("app: ", t.header_style()),
            Span::styled(format!("{}▏", p.input), t.base()),
        ]),
        Line::from(""),
    ];
    if matches.is_empty() {
        lines.push(Line::from(Span::styled(
            "no app of the loaded result matches",
            t.dim_style(),
        )));
    }
    let first = p.selected.saturating_sub(9);
    for (i, app) in matches.iter().enumerate().skip(first).take(10) {
        let style = if i == p.selected {
            t.base().add_modifier(Modifier::REVERSED)
        } else {
            t.base()
        };
        lines.push(Line::from(Span::styled(format!("  {app}"), style)));
    }
    f.render_widget(Paragraph::new(lines).block(block).style(t.base()), r);
}
```

In `crates/illumio-tui/src/tui/ui/screens/traffic.rs` rename the existing `fn query_meta(model: &Model) -> String` to `fn result_meta(model: &Model) -> String` and add:

```rust
/// The Result header: the focus badge (`focus shop/prod · in`, or `focus is
/// partial: r re-queries shop/prod · in`) in front of the query's meta.
fn query_meta(model: &Model) -> String {
    let meta = result_meta(model);
    match model.traffic.focus_meta() {
        Some(focus) => format!("{focus} · {meta}"),
        None => meta,
    }
}
```

In `render_stats`, replace the decision count loop and the first span so they count the flows the focus keeps:

```rust
    let shown = model.traffic.view_flows();
    let (mut a, mut p, mut b) = (0u64, 0u64, 0u64);
    for fl in shown.iter() {
        match fl.policy_decision {
            PolicyDecision::Allowed => a += 1,
            PolicyDecision::PotentiallyBlocked => p += 1,
            PolicyDecision::Blocked => b += 1,
            PolicyDecision::Unknown => {}
        }
    }
    let count = if model.traffic.focus.is_some() {
        format!("{} of {} flows ", shown.len(), d.flows.len())
    } else {
        format!("{} flows ", d.flows.len())
    };
    let mut left = vec![
        Span::styled(count, t.header_style()),
```

and replace `let data = bars::sparkline_buckets(&d.flows, 12);` with `let data = bars::sparkline_buckets(&shown, 12);`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p illumio-tui --lib app_picker focus_tests commands`
Expected: PASS.

Run: `INSTA_UPDATE=always cargo test -p illumio-tui --test tui_snapshots && find crates/illumio-tui/tests/snapshots -name '*.snap.new' -delete`
Expected: PASS; two new snapshot files (`traffic_focus_badge_80x24.snap`, `traffic_app_picker_80x24.snap`). `git status` must show no other changed snapshot: without a focus the Result header and counts are unchanged. Read both new files: the badge sits in the Result panel's top border, the stats line reads `4 of 4 flows`, the picker box lists `shop/prod`.

- [ ] **Step 5: Gate and commit**

```bash
cargo fmt --all && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add crates/illumio-tui/src/tui/app_picker.rs crates/illumio-tui/src/tui/mod.rs crates/illumio-tui/src/tui/model.rs crates/illumio-tui/src/tui/commands.rs crates/illumio-tui/src/tui/update.rs crates/illumio-tui/src/tui/ui/mod.rs crates/illumio-tui/src/tui/ui/widgets/overlay.rs crates/illumio-tui/src/tui/ui/screens/traffic.rs crates/illumio-tui/tests/tui_snapshots.rs crates/illumio-tui/tests/snapshots/
git commit -m "feat(tui): F, X and Esc focus every traffic view on one app; :focus, the picker, the badge

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

---

### Task 5: `r` on a partial focus: the focused re-query

**Files:**
- Modify: `crates/illumio-tui/src/tui/msg.rs` (new `FocusedOutcome` after `QueryOutcome` ~18-28; `Msg::FocusedDone` after `Msg::SliceDone` ~73-76; `Cmd::RunFocused` after `Cmd::RunSlice` ~134-140; its `PartialEq` arm ~214-229)
- Modify: `crates/illumio-tui/src/tui/model.rs` (`PendingAction::RunFocused`, ~298-311)
- Modify: `crates/illumio-tui/src/tui/update.rs` (`Msg::RunNow` ~213-237; `Msg::QueryDone` ~254-316; new `Msg::FocusedDone` arm; `Overlay::Confirm` `y` arm ~1087; `traffic_key` before `match k.code`; new fns `requery_focus`, `start_focused`; tests in `focus_tests`)
- Modify: `crates/illumio-tui/src/tui/app.rs` (imports 4-26; new `run_focused` after `run_traffic` ~145-206; `execute` arm; `apply` task-result list ~503-521)
- Test: `crates/illumio-tui/src/tui/update.rs` (`mod focus_tests`), `crates/illumio-tui/tests/tui_runtime.rs`, `crates/illumio-tui/tests/tui_snapshots.rs`
- Snapshot (new): `traffic_focus_partial_120x30`

**Interfaces:**
- Consumes: Task 2's `Pce::focused_traffic`, `FocusedTraffic::query_count`; Task 3's `focus_requery`, `apply_focused`, `back_to_unfocused`, `queries_text`; Task 4's `focus_tests` helpers.
- Produces:

```rust
// tui/msg.rs
pub struct FocusedOutcome { pub outcome: QueryOutcome, pub focus: Focus, pub queries: u32 }
pub enum Msg { /* ... */ FocusedDone { seq: u64, result: Result<FocusedOutcome, String> } }
pub enum Cmd { /* ... */ RunFocused { seq: u64, filter: TrafficFilter, focus: Focus, mode: CacheMode, cancel: CancellationToken } }
// tui/model.rs
pub enum PendingAction { /* ... */ RunFocused(TrafficFilter, Focus, CacheMode) }
// tui/update.rs
fn requery_focus(model: &mut Model, focus: Focus) -> Vec<Cmd>;
fn start_focused(model: &mut Model, filter: TrafficFilter, focus: Focus, mode: CacheMode) -> Vec<Cmd>;
// tui/app.rs
fn run_focused(&mut self, seq: u64, filter: TrafficFilter, focus: Focus, mode: CacheMode, cancel: CancellationToken);
```

- [ ] **Step 1: Write the failing tests**

Add to `mod focus_tests` in `crates/illumio-tui/src/tui/update.rs` (and `use crate::tui::msg::FocusedOutcome;`, `use std::time::Duration;` to its imports):

```rust
    fn focused_data(m: &Model, n: usize) -> TrafficData {
        TrafficData {
            flows: m.traffic.loaded_flows()[..n].to_vec(),
            truncated: false,
            matches_count: Some(n as u64),
            elapsed: Duration::from_millis(900),
            cache_hit: false,
            fetched_at: Instant::now(),
            filter: m.traffic.filter.clone(),
            slices: None,
            backfill: None,
        }
    }

    #[test]
    fn a_truncated_result_offers_r_and_r_sends_one_query_per_side() {
        let mut m = model_with_flows();
        m.traffic.data.as_mut().unwrap().truncated = true;
        quiet(&mut m, focus_cmd("shop/prod"));
        assert_eq!(
            m.status.text,
            "focus shop/prod · both · 4 flows · the result is truncated: r re-queries shop/prod (2 queries)"
        );
        let cmds = update(&mut m, key('r'));
        let [Cmd::RunFocused { seq, filter, focus, mode, .. }] = cmds.as_slice() else {
            panic!("{cmds:?}")
        };
        assert_eq!(*seq, m.traffic.seq);
        assert_eq!(filter, &m.traffic.filter);
        assert_eq!(focus.app_text(), "shop/prod");
        assert_eq!(*mode, CacheMode::Use);
        assert_eq!(m.status.text, "focused query running: shop/prod · 2 queries");
        let flows = crate::tui::test_support::fixture_flows();
        let outcome = QueryOutcome {
            flows: flows[..3].to_vec(),
            truncated: false,
            matches_count: Some(3),
            elapsed: Duration::from_millis(900),
            hit: false,
            filter: m.traffic.filter.clone(),
            bounds: None,
        };
        let seq = m.traffic.seq;
        quiet(
            &mut m,
            Msg::FocusedDone {
                seq,
                result: Ok(FocusedOutcome {
                    outcome,
                    focus: focus.clone(),
                    queries: 2,
                }),
            },
        );
        assert!(!m.traffic.is_running());
        assert_eq!(m.traffic.loaded_flows().len(), 3);
        assert_eq!(
            m.traffic.focus_meta().as_deref(),
            Some("focus shop/prod · both · 2 queries")
        );
        assert_eq!(m.status.text, "3 flows for shop/prod in 0.9s · 2 queries");
        let cmds = update(&mut m, key('r'));
        assert!(
            matches!(cmds.as_slice(), [Cmd::RunFocused { mode: CacheMode::Bypass, .. }]),
            "r on the focused result refreshes it past the cache: {cmds:?}"
        );
        m.traffic.cancel_query();
        quiet(&mut m, kc(KeyCode::Esc));
        assert_eq!(focused(&m), None);
        assert_eq!(m.traffic.loaded_flows().len(), 4, "the unfocused result is back");
        assert!(m.traffic.data.as_ref().unwrap().truncated);
    }

    #[test]
    fn a_stale_focused_answer_is_dropped() {
        let mut m = model_with_flows();
        m.traffic.data.as_mut().unwrap().truncated = true;
        quiet(&mut m, focus_cmd("shop/prod"));
        let _ = update(&mut m, key('r'));
        let stale = m.traffic.seq;
        m.traffic.cancel_query();
        let outcome = QueryOutcome {
            flows: Vec::new(),
            truncated: false,
            matches_count: Some(0),
            elapsed: Duration::ZERO,
            hit: false,
            filter: m.traffic.filter.clone(),
            bounds: None,
        };
        let focus = m.traffic.focus.clone().unwrap();
        quiet(
            &mut m,
            Msg::FocusedDone {
                seq: stale,
                result: Ok(FocusedOutcome { outcome, focus, queries: 2 }),
            },
        );
        assert!(m.traffic.focus_result.is_none());
        assert_eq!(m.traffic.loaded_flows().len(), 4);
    }

    #[test]
    fn an_expensive_focused_query_asks_first_and_names_the_count() {
        let mut m = model_with_flows();
        m.traffic.data.as_mut().unwrap().truncated = true;
        m.traffic.filter.max_results = 20_000;
        quiet(&mut m, focus_cmd("shop/prod"));
        quiet(&mut m, key('X'));
        quiet(&mut m, key('r'));
        let Overlay::Confirm { reason, .. } = &m.overlay else {
            panic!("{:?}", m.overlay)
        };
        assert_eq!(
            reason,
            "cap 20000 exceeds 10000. Run 1 query for shop/prod anyway? (y/n)"
        );
        let cmds = update(&mut m, key('y'));
        assert!(
            matches!(cmds.as_slice(), [Cmd::RunFocused { focus, .. }] if focus.direction == FocusDirection::In),
            "{cmds:?}"
        );
    }

    #[test]
    fn a_group_without_labels_is_not_re_queried() {
        let mut m = model_with_flows();
        m.traffic.data.as_mut().unwrap().truncated = true;
        m.traffic.view = TrafficView::Blocked;
        let nas = |r: &crate::tui::model::SectionRow| {
            r.peer().is_some_and(|(_, p)| p.peer_text() == "unmanaged-nas/-")
        };
        m.traffic.blocked.select_where(nas);
        quiet(&mut m, key('F'));
        m.traffic.blocked.select_where(nas);
        quiet(&mut m, key('F'));
        assert_eq!(focused(&m).unwrap().0, "unmanaged-nas/-");
        quiet(&mut m, key('r'));
        assert_eq!(
            m.status.text,
            "unmanaged-nas/- is not a label group: it cannot be re-queried by its labels"
        );
    }

    #[test]
    fn r_without_a_partial_focus_reruns_the_unfocused_query() {
        let mut m = model_with_flows();
        quiet(&mut m, focus_cmd("shop/prod"));
        let cmds = update(&mut m, key('r'));
        assert!(matches!(cmds.as_slice(), [Cmd::RunQuery { mode: CacheMode::Bypass, .. }]), "{cmds:?}");
    }

    #[test]
    fn window_and_cap_keys_work_on_the_unfocused_result() {
        let mut m = model_with_flows();
        m.traffic.data.as_mut().unwrap().truncated = true;
        quiet(&mut m, focus_cmd("shop/prod"));
        let f = m.traffic.focus.clone().unwrap();
        let data = focused_data(&m, 2);
        m.traffic.apply_focused(f, 2, data);
        let _ = update(&mut m, key(']'));
        assert!(m.traffic.unfocused.is_none() && m.traffic.focus_result.is_none());
        assert_eq!(m.traffic.loaded_flows().len(), 4);
        assert!(m.traffic.focus.is_some(), "the focus stays and narrows the unfocused result");
        // A new unfocused result also drops a focused one kept aside.
        let f = m.traffic.focus.clone().unwrap();
        let data = focused_data(&m, 1);
        m.traffic.apply_focused(f, 2, data);
        let seq = m.traffic.seq;
        let outcome = QueryOutcome {
            flows: crate::tui::test_support::fixture_flows(),
            truncated: true,
            matches_count: Some(40),
            elapsed: Duration::from_millis(100),
            hit: false,
            filter: m.traffic.filter.clone(),
            bounds: None,
        };
        let _ = update(&mut m, Msg::QueryDone { seq, result: Ok(outcome) });
        assert!(m.traffic.unfocused.is_none() && m.traffic.focus_result.is_none());
        assert!(m.traffic.focus_partial(), "the new result is truncated: partial again");
    }
```

Append to `crates/illumio-tui/tests/tui_runtime.rs`:

```rust
#[tokio::test]
async fn r_on_a_partial_focus_sends_one_query_per_side() {
    use illumio_tui::tui::commands::Command;
    use illumio_tui::tui::msg::Msg;
    use illumio_tui::tui::test_support::model_with_flows;
    use wiremock::matchers::body_partial_json;
    let server = MockServer::start().await;
    mount_objects(&server).await;
    let labels = json!([{"label": {"href": "/orgs/5/labels/3"}}, {"label": {"href": "/orgs/5/labels/4"}}]);
    for (side, body) in [
        ("in", json!({"destinations": {"include": [labels.clone()]}})),
        ("out", json!({"sources": {"include": [labels.clone()]}})),
    ] {
        let href = format!("/orgs/5/traffic_flows/async_queries/q-{side}");
        Mock::given(method("POST"))
            .and(path("/api/v2/orgs/5/traffic_flows/async_queries"))
            .and(body_partial_json(body))
            .respond_with(
                ResponseTemplate::new(202).set_body_json(json!({"href": href, "status": "queued"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/api/v2{href}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "completed", "result": {"href": format!("{href}/download")}, "matches_count": 4})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/api/v2{href}/download")))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(include_str!("../../../fixtures/traffic_flows.json"))
                    .insert_header("Content-Type", "application/json"),
            )
            .mount(&server)
            .await;
    }
    let pce = pce_for(&server);
    let mut m = model_with_flows();
    m.traffic.data.as_mut().unwrap().truncated = true;
    let (mut rt, tx) = Runtime::new(pce, m);
    tx.send(Msg::Command(Command::Focus(Some("shop/prod".into()))))
        .unwrap();
    let model = run_loop(
        &mut rt,
        Box::pin(press(vec![(100, ch('r')), (1500, ch('q'))])),
        &mut |_| {},
    )
    .await;
    let r = model.traffic.focus_result.as_ref().expect("a focused result");
    assert_eq!(r.queries, 2);
    assert!(model.traffic.unfocused.is_some(), "the unfocused result is kept aside");
    assert_eq!(model.traffic.loaded_flows().len(), 4, "flow 0 once");
    assert_eq!(model.busy, 0);
    assert_eq!(
        model.stats.calls,
        4 + 3 + 3,
        "4 lists for the filter, then POST + poll + download per side"
    );
}
```

Append to `crates/illumio-tui/tests/tui_snapshots.rs`:

```rust
#[test]
fn traffic_focus_partial_120x30() {
    use illumio_tui::tui::commands::Command;
    use illumio_tui::tui::msg::Msg;
    let mut m = model_with_flows();
    m.traffic.data.as_mut().unwrap().truncated = true;
    illumio_tui::tui::update(&mut m, Msg::Command(Command::Focus(Some("shop/prod".into()))));
    let out = render_to_string(&m, 120, 30);
    assert!(out.contains("focus is partial: r re-queries shop/prod · both"), "{out}");
    insta::assert_snapshot!("traffic_focus_partial_120x30", out);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p illumio-tui --lib focus_tests`
Expected: FAIL to compile with "no variant named `RunFocused` found for enum `Cmd`" and "no variant named `FocusedDone` found for enum `Msg`".

- [ ] **Step 3: Write the implementation**

In `crates/illumio-tui/src/tui/msg.rs`: add `use pce::traffic::Focus;` (merge into `use pce::traffic::{Focus, QueryProgress, ResolvedFlow};`); after `pub struct QueryOutcome { ... }` add:

```rust
/// A focused re-query's answer: the merged flows (as one query's outcome),
/// the focus they were queried for and the number of Explorer queries.
#[derive(Debug)]
pub struct FocusedOutcome {
    pub outcome: QueryOutcome,
    pub focus: Focus,
    pub queries: u32,
}
```

after `Msg::SliceDone { ... },`:

```rust
    /// The answer to `Cmd::RunFocused`: one query per side, merged.
    FocusedDone {
        seq: u64,
        result: Result<FocusedOutcome, String>,
    },
```

after `Cmd::RunSlice { ... },`:

```rust
    /// `r` on a partial focus: `filter` with the app's labels on its side,
    /// one query per side (`Pce::focused_traffic`), answered with
    /// `Msg::FocusedDone`.
    RunFocused {
        seq: u64,
        filter: TrafficFilter,
        focus: Focus,
        mode: CacheMode,
        cancel: CancellationToken,
    },
```

and in `impl PartialEq for Cmd`, after the `Cmd::RunSlice` arm:

```rust
            (
                Cmd::RunFocused {
                    seq: a,
                    filter: fa,
                    focus: xa,
                    mode: ma,
                    ..
                },
                Cmd::RunFocused {
                    seq: b,
                    filter: fb,
                    focus: xb,
                    mode: mb,
                    ..
                },
            ) => a == b && fa == fb && xa == xb && ma == mb,
```

In `crates/illumio-tui/src/tui/model.rs`, add to `pub enum PendingAction` after `Backfill(pce::traffic::BackfillPlan),`:

```rust
    /// `r` on a partial focus past a cost threshold: one query per side.
    RunFocused(TrafficFilter, pce::traffic::Focus, pce::CacheMode),
```

In `crates/illumio-tui/src/tui/update.rs`:

Import `FocusedOutcome` (`use crate::tui::msg::{Cmd, FocusedOutcome, Msg, ObjectsPayload, QueryOutcome};`) and `queries_text` (add to the `crate::tui::model::{...}` list).

In `Msg::RunNow`, right after the `is_running` early return:

```rust
            if let Some(focus) = model.traffic.focus_requery() {
                return requery_focus(model, focus);
            }
```

In `Msg::QueryDone`'s `Ok(o) =>` branch, right after the `let QueryOutcome { ... } = o;` destructuring, add:

```rust
                    // A new unfocused result: a focused one kept aside is stale.
                    model.traffic.unfocused = None;
                    model.traffic.focus_result = None;
```

and replace its truncated status

```rust
                    model.status = if truncated {
                        StatusLine::warn(format!(
                            "{n} flows (truncated: the PCE matched {}; raise the cap or narrow the filter)",
                            matches_count
                                .map(|m| m.to_string())
                                .unwrap_or_else(|| "more".into())
                        ))
```

with

```rust
                    let focus_hint = model
                        .traffic
                        .focus
                        .as_ref()
                        .map(|f| format!(" · focus is partial: r re-queries {}", f.app_text()))
                        .unwrap_or_default();
                    model.status = if truncated {
                        StatusLine::warn(format!(
                            "{n} flows (truncated: the PCE matched {}; raise the cap or narrow the filter){focus_hint}",
                            matches_count
                                .map(|m| m.to_string())
                                .unwrap_or_else(|| "more".into())
                        ))
```

After the whole `Msg::QueryDone { seq, result } => { ... }` arm add:

```rust
        Msg::FocusedDone { seq, result } => {
            if model.traffic.seq != seq {
                return Vec::new(); // a cancelled or superseded query
            }
            match result {
                Ok(FocusedOutcome {
                    outcome,
                    focus,
                    queries,
                }) => {
                    let QueryOutcome {
                        flows,
                        truncated,
                        matches_count,
                        elapsed,
                        hit,
                        filter,
                        bounds,
                    } = outcome;
                    let data = TrafficData {
                        flows,
                        truncated,
                        matches_count,
                        elapsed,
                        cache_hit: hit,
                        fetched_at: Instant::now(),
                        filter,
                        slices: bounds.map(|(start, end)| Slices::new(start, end)),
                        backfill: None,
                    };
                    let app = focus.app_text();
                    model.traffic.query = QueryState::Idle;
                    model.traffic.show_detail = false;
                    model.traffic.apply_focused(focus, queries, data);
                    let n = model.traffic.view_flows().len();
                    let q = queries_text(queries);
                    model.status = if truncated {
                        StatusLine::warn(format!(
                            "{n} flows for {app} · {q} (truncated: raise the cap or narrow the filter)"
                        ))
                    } else {
                        StatusLine::info(format!(
                            "{n} flows for {app} in {:.1}s · {q}{}",
                            elapsed.as_secs_f64(),
                            if hit { " (cached)" } else { "" }
                        ))
                    };
                }
                Err(e) => {
                    model.traffic.query = QueryState::Failed(e.clone());
                    fail(model, "focused query", &e, false);
                }
            }
            Vec::new()
        }
```

In `on_key`'s `Overlay::Confirm` `y` arm, add after `PendingAction::Backfill(plan) => start_backfill(model, plan, false),`:

```rust
                    PendingAction::RunFocused(f, focus, mode) => start_focused(model, f, focus, mode),
```

In `traffic_key`, right before `match k.code {`:

```rust
    // The window, cap and backfill keys work on the unfocused result.
    if matches!(k.code, KeyCode::Char('+' | '-' | ']' | '[' | 'B')) {
        model.traffic.back_to_unfocused();
    }
```

Add after `focus_by_name`:

```rust
/// `r` on a partial focus (or on its focused result, which it refreshes past
/// the cache): one query per side, through the cost guard, whose prompt
/// names the count. An app without labels (`ipl:Corp/-`) is refused.
fn requery_focus(model: &mut Model, focus: Focus) -> Vec<Cmd> {
    if !focus.is_label_group() {
        model.status = StatusLine::error(format!(
            "{} is not a label group: it cannot be re-queried by its labels",
            focus.app_text()
        ));
        return Vec::new();
    }
    let f = model.traffic.filter.clone();
    let refresh = model
        .traffic
        .focus_result
        .as_ref()
        .is_some_and(|r| r.covers(&focus));
    let mode = if refresh {
        CacheMode::Bypass
    } else {
        CacheMode::Use
    };
    if let Some(reason) = f.needs_confirmation() {
        let n = u32::try_from(focus.direction.sides().len()).unwrap_or(2);
        model.overlay = Overlay::Confirm {
            reason: format!(
                "{reason}. Run {} for {} anyway? (y/n)",
                queries_text(n),
                focus.app_text()
            ),
            action: PendingAction::RunFocused(f, focus, mode),
        };
        return Vec::new();
    }
    start_focused(model, f, focus, mode)
}

fn start_focused(
    model: &mut Model,
    filter: TrafficFilter,
    focus: Focus,
    mode: CacheMode,
) -> Vec<Cmd> {
    let (seq, cancel) = model.traffic.start_query(Instant::now());
    let n = u32::try_from(focus.direction.sides().len()).unwrap_or(2);
    model.status = StatusLine::info(format!(
        "focused query running: {} · {}",
        focus.app_text(),
        queries_text(n)
    ));
    model.traffic.auto.rearm();
    vec![Cmd::RunFocused {
        seq,
        filter,
        focus,
        mode,
        cancel,
    }]
}
```

In `crates/illumio-tui/src/tui/app.rs`: import `FocusedOutcome` (`use crate::tui::msg::{Cmd, FocusedOutcome, Msg, ObjectsPayload, QueryOutcome};`) and `use pce::traffic::Focus;`. After `run_traffic` add:

```rust
    /// `r` on a partial focus: the filter's query with the app's labels on
    /// its side, one query per side (`Pce::focused_traffic`, cache slots
    /// `focus:in` / `focus:out`), answered with `Msg::FocusedDone`.
    fn run_focused(
        &mut self,
        seq: u64,
        filter: TrafficFilter,
        focus: Focus,
        mode: CacheMode,
        cancel: CancellationToken,
    ) {
        self.model.busy += 1;
        let (pce, tx) = self.task_ctx();
        tokio::spawn(async move {
            let (ptx, mut prx) = tokio::sync::mpsc::unbounded_channel();
            let forward_tx = tx.clone();
            let forward = tokio::spawn(async move {
                while let Some(p) = prx.recv().await {
                    let _ = forward_tx.send(Msg::QueryProgress { seq, progress: p });
                }
            });
            let result = async {
                let query = filter.to_query(&pce, CacheMode::Use).await?;
                let idx = pce.name_index(CacheMode::Use).await?;
                let ft = pce
                    .focused_traffic("focus", &focus, &query, &idx, mode, &cancel, Some(&ptx))
                    .await?;
                let bounds = query
                    .start_date
                    .parse()
                    .ok()
                    .zip(query.end_date.parse().ok());
                let queries = ft.query_count();
                Ok::<_, pce::PceError>(FocusedOutcome {
                    outcome: QueryOutcome {
                        flows: ft.flows,
                        truncated: ft.truncated,
                        matches_count: ft.matches_count,
                        elapsed: ft.elapsed,
                        hit: ft.hit,
                        filter,
                        bounds,
                    },
                    focus,
                    queries,
                })
            }
            .await
            .map_err(|e| e.to_string());
            drop(ptx);
            let _ = forward.await;
            let _ = tx.send(Msg::FocusedDone { seq, result });
            let _ = tx.send(Msg::Stats(pce.stats()));
        });
    }
```

In `execute`, after the `Cmd::RunSlice { ... } => ...` arm:

```rust
            Cmd::RunFocused {
                seq,
                filter,
                focus,
                mode,
                cancel,
            } => self.run_focused(seq, filter, focus, mode, cancel),
```

In `apply`'s `task_result` list add `| Msg::FocusedDone { .. }` after `| Msg::SliceDone { .. }`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p illumio-tui --lib focus_tests`
Expected: PASS.

Run: `cargo test -p illumio-tui --test tui_runtime r_on_a_partial_focus`
Expected: PASS (both POST mocks are hit exactly once; wiremock verifies on drop).

Run: `INSTA_UPDATE=always cargo test -p illumio-tui --test tui_snapshots && find crates/illumio-tui/tests/snapshots -name '*.snap.new' -delete`
Expected: one new file, `traffic_focus_partial_120x30.snap`; read it: the Result header begins `focus is partial: r re-queries shop/prod · both`.

- [ ] **Step 5: Gate and commit**

```bash
cargo fmt --all && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add crates/illumio-tui/src/tui/msg.rs crates/illumio-tui/src/tui/model.rs crates/illumio-tui/src/tui/update.rs crates/illumio-tui/src/tui/app.rs crates/illumio-tui/tests/tui_runtime.rs crates/illumio-tui/tests/tui_snapshots.rs crates/illumio-tui/tests/snapshots/
git commit -m "feat(tui): r re-queries a partial focus, one query per side, merged; Esc puts the result back

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

---

### Task 6: CLI `--app` / `--direction` on `traffic query|groups|mesh`

**Files:**
- Modify: `crates/illumio-tui/src/cli/traffic.rs` (imports 1-24; `GroupsArgs` ~97-115 loses `app`; `FilterArgs` ~137-186 gains `app`, `direction`; new `DirectionArg`, `CliFocus`, `cli_focus`, `DryRuns`; `app_miss` ~673; `sections_for` ~682-708; `Fetched` ~731-747; `fetch` ~867-911; `query`, `groups`, `mesh_cmd` ~921-976)
- Modify: `crates/illumio-tui/src/cli/output.rs` (`Meta` ~47-68 gains `queries`; `meta_line` ~249-279; `write_table` / `write_table_csv` take `?Sized`; test literal ~353)
- Modify: `crates/illumio-tui/src/cli/context.rs:60-74` (`queries: None`)
- Modify: `crates/illumio-tui/src/cli/profile.rs:77-88` (`queries: None`)
- Test: `crates/illumio-tui/tests/cli.rs` (rewrite `traffic_groups_by_in_out_both_with_show_and_app` ~1988-2054; three new tests after it)

**Interfaces:**
- Consumes: Task 1's `Focus`, `FocusDirection`, `parse_app`; Task 2's `Pce::focused_traffic`; existing `build_query`, `BuiltQuery`, `cache_label`, `coverage`.
- Produces (Plan 2's `illumio-tui app` reuses these; it makes `fetch` and `Fetched` `pub(crate)`):

```rust
// cli/traffic.rs
pub enum DirectionArg { In, Out, Both }                       // clap ValueEnum
impl From<DirectionArg> for FocusDirection;
pub struct FilterArgs { /* ... */ pub app: Option<String>, pub direction: Option<DirectionArg> }
pub struct CliFocus { pub focus: Focus, pub query: bool }
pub fn cli_focus(args: &FilterArgs, keys: &[String], by: ByArg) -> Result<Option<CliFocus>, CliError>;
pub struct DryRuns(pub Vec<TrafficQuery>);
struct Fetched { /* ... */ queries: Option<u32> }
async fn fetch(ctx: &Context, args: &FilterArgs, focus: Option<&CliFocus>) -> Result<Option<Fetched>, CliError>;
// cli/output.rs
pub struct Meta { /* ... */ pub queries: Option<u32> }
pub fn write_table<T: TableRows + ?Sized>(out: &mut impl Write, data: &T) -> std::io::Result<()>;
pub fn write_table_csv<T: TableRows + ?Sized>(out: &mut impl Write, data: &T) -> std::io::Result<()>;
```

- [ ] **Step 1: Write the failing tests**

In `crates/illumio-tui/tests/cli.rs` replace the whole `traffic_groups_by_in_out_both_with_show_and_app` test with:

```rust
#[tokio::test(flavor = "multi_thread")]
async fn traffic_groups_by_in_out_both_with_show_and_app() {
    let server = MockServer::start().await;
    // --by in, --by out (name-keyed app: unfocused), --by both, --by in
    // --app shop/prod (focused: 1), --app shop/prod (pairs, focused: 2).
    mount_traffic(&server, 6).await;
    let out = stdout_json(
        cmd(&server)
            .args(["traffic", "groups", "--window", "1h", "--by", "in"])
            .assert()
            .success(),
    );
    let sections = out["data"]["sections"].as_array().unwrap();
    assert_eq!(sections[0]["app"], json!(["shop", "prod"]));
    assert_eq!(sections[0]["totals"]["connections"], 325);
    assert_eq!(sections[0]["rows"].as_array().unwrap().len(), 3);
    let out = stdout_json(
        cmd(&server)
            .args([
                "traffic", "groups", "--window", "1h", "--by", "out", "--show", "blocked-only",
                "--app", "unmanaged-nas/-",
            ])
            .assert()
            .success(),
    );
    let sections = out["data"]["sections"].as_array().unwrap();
    assert_eq!(sections.len(), 1, "a name-keyed app keeps narrowing the sections");
    assert_eq!(sections[0]["direction"], "out");
    assert_eq!(sections[0]["totals"]["blocked"], 1);
    assert!(out["meta"].get("queries").is_none(), "not a focused query");
    let table = cmd(&server)
        .args([
            "traffic", "groups", "--window", "1h", "--by", "both", "--table",
        ])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&table.get_output().stdout).to_string();
    assert!(
        text.contains("\nshop/prod  ← 325 conns · 1 blocked · 1 pot.blocked  → 315 conns\n"),
        "{text}"
    );
    assert!(
        text.contains("\n  ← ipl:Corp/-") && text.contains("\n  → "),
        "{text}"
    );
    let out = stdout_json(
        cmd(&server)
            .args([
                "traffic", "groups", "--window", "1h", "--by", "in", "--app", "shop/prod",
            ])
            .assert()
            .success(),
    );
    let sections = out["data"]["sections"].as_array().unwrap();
    assert_eq!(sections.len(), 1);
    assert_eq!(sections[0]["totals"]["connections"], 325, "the same section, now from a focused query");
    assert_eq!(out["meta"]["queries"], 1, "--by in focuses the query on in");
    let miss = cmd(&server)
        .args([
            "traffic", "groups", "--window", "1h", "--by", "in", "--app", "pos/PCI",
        ])
        .assert()
        .code(2);
    let err = String::from_utf8_lossy(&miss.get_output().stderr);
    assert!(err.contains(r#"unknown app \"pos/PCI\""#), "{err}");
    let out = stdout_json(
        cmd(&server)
            .args(["traffic", "groups", "--window", "1h", "--app", "shop/prod"])
            .assert()
            .success(),
    );
    let pairs = out["data"].as_array().expect("--app without --by gives the pairs");
    assert!(
        pairs
            .iter()
            .all(|p| p["src"] == json!(["shop", "prod"]) || p["dst"] == json!(["shop", "prod"])),
        "{pairs:?}"
    );
    assert_eq!(out["meta"]["queries"], 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn traffic_app_dry_run_prints_one_body_per_side() {
    let server = MockServer::start().await;
    mount_traffic(&server, 0).await;
    let labels = json!([{"label": {"href": "/orgs/5/labels/3"}}, {"label": {"href": "/orgs/5/labels/4"}}]);
    let out = stdout_json(
        cmd(&server)
            .args(["traffic", "query", "--window", "1h", "--app", "shop/prod", "--dry-run"])
            .assert()
            .success(),
    );
    let bodies = out["data"].as_array().expect("one body per side");
    assert_eq!(bodies.len(), 2);
    assert_eq!(bodies[0]["destinations"]["include"], json!([labels.clone()]));
    assert_eq!(bodies[0]["sources"]["include"], json!([[]]));
    assert_eq!(bodies[1]["sources"]["include"], json!([labels.clone()]));
    assert_eq!(bodies[1]["destinations"]["include"], json!([[]]));
    assert_eq!(out["meta"]["cache"], "none");
    for (sub, dir, side) in [("groups", "in", "destinations"), ("mesh", "out", "sources")] {
        let out = stdout_json(
            cmd(&server)
                .args([
                    "traffic", sub, "--window", "1h", "--app", "shop/prod", "--direction", dir,
                    "--dry-run",
                ])
                .assert()
                .success(),
        );
        let bodies = out["data"].as_array().unwrap();
        assert_eq!(bodies.len(), 1, "{sub} --direction {dir}");
        assert_eq!(bodies[0][side]["include"], json!([labels.clone()]));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn traffic_app_spends_the_cap_on_the_app_and_counts_its_queries() {
    let server = MockServer::start().await;
    mount_traffic(&server, 4).await; // groups --direction in: 1; query (both): 2; query out: 1
    let out = stdout_json(
        cmd(&server)
            .args([
                "traffic", "groups", "--window", "1h", "--app", "shop/prod", "--direction", "in",
            ])
            .assert()
            .success(),
    );
    let pairs = out["data"].as_array().unwrap();
    assert_eq!(pairs.len(), 3, "only the flows into shop/prod: {pairs:?}");
    assert!(pairs.iter().all(|p| p["dst"] == json!(["shop", "prod"])));
    assert_eq!(out["meta"]["queries"], 1);
    let out = stdout_json(
        cmd(&server)
            .args(["traffic", "query", "--window", "1h", "--app", "shop/prod"])
            .assert()
            .success(),
    );
    assert_eq!(
        out["data"].as_array().unwrap().len(),
        4,
        "both queries returned flow 0: it counts once"
    );
    assert_eq!(out["meta"]["queries"], 2);
    let table = cmd(&server)
        .args(["traffic", "query", "--window", "1h", "--app", "shop/prod", "--direction", "out", "--table"])
        .assert()
        .success();
    let err = String::from_utf8_lossy(&table.get_output().stderr);
    assert!(err.contains("· 1 query"), "the meta line counts the query: {err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn traffic_app_mistakes_exit_2_before_any_traffic_request() {
    let server = MockServer::start().await;
    mount_traffic(&server, 0).await;
    let err = cmd(&server)
        .args(["traffic", "groups", "--window", "1h", "--app", "shp/prod"])
        .assert()
        .code(2);
    let err = String::from_utf8_lossy(&err.get_output().stderr).to_string();
    assert!(
        err.contains(r#"unknown app \"shp/prod\": no app=shp label (closest: shop)"#),
        "{err}"
    );
    let err = cmd(&server)
        .args([
            "traffic", "groups", "--window", "1h", "--by", "in", "--app", "shop/prod",
            "--direction", "in",
        ])
        .assert()
        .code(2);
    assert!(String::from_utf8_lossy(&err.get_output().stderr).contains("--direction conflicts with --by"));
    cmd(&server)
        .args(["traffic", "query", "--window", "1h", "--direction", "in"])
        .assert()
        .code(2);
    let err = cmd(&server)
        .args(["traffic", "query", "--window", "1h", "--app", "shop/prod", "--backfill"])
        .assert()
        .code(2);
    assert!(String::from_utf8_lossy(&err.get_output().stderr).contains("--backfill does not combine with --app"));
    let err = cmd(&server)
        .args(["traffic", "query", "--window", "1h", "--app", "unmanaged-nas/-"])
        .assert()
        .code(2);
    assert!(
        String::from_utf8_lossy(&err.get_output().stderr).contains("a - slot cannot be queried"),
        "a name-keyed app without --by cannot focus a query"
    );
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p illumio-tui --test cli traffic_app traffic_groups_by_in_out_both`
Expected: FAIL: clap rejects `--direction` ("unexpected argument '--direction'"), and `traffic query --app` is unknown.

- [ ] **Step 3: Write the implementation**

In `crates/illumio-tui/src/cli/output.rs`: add to `pub struct Meta` after `slices`:

```rust
    /// Traffic with `--app`: the Explorer queries the result took (one per
    /// side: 2 for `both`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queries: Option<u32>,
```

in `meta_line`, after the `slices` block:

```rust
    if let Some(n) = meta.queries {
        s.push_str(&format!(" · {n} quer{}", if n == 1 { "y" } else { "ies" }));
    }
```

change `pub fn write_table<T: TableRows>(` to `pub fn write_table<T: TableRows + ?Sized>(` and `pub fn write_table_csv<T: TableRows>(` to `pub fn write_table_csv<T: TableRows + ?Sized>(`, and add `queries: None,` after `slices: None,` in the `meta_line_shows_the_write` test literal. Add `queries: None,` after `slices: None,` in `crates/illumio-tui/src/cli/context.rs` (`Context::meta`) and `crates/illumio-tui/src/cli/profile.rs` (`emit`).

In `crates/illumio-tui/src/cli/traffic.rs`:

Imports: extend the `pce::traffic::{...}` list with `Focus, FocusDirection, parse_app`.

In `GroupsArgs`, delete the `app` field and its doc comment:

```rust
    /// Sections only: keep this app's sections, e.g. pos/PCI
    #[arg(long)]
    pub app: Option<String>,
```

In `FilterArgs`, after `max_slices`:

```rust
    /// Focus on one app, a group of the label keys (--keys, else the profile's scope keys),
    /// e.g. shop/prod: its labels go into the query, so the cap is spent on it
    #[arg(long)]
    pub app: Option<String>,
    /// With --app: in (the app is the destination), out (the source) or both
    /// (default; two queries, merged)
    #[arg(long, value_enum, requires = "app")]
    pub direction: Option<DirectionArg>,
```

After `impl From<ShowArg> for Decisions { ... }` add:

```rust
/// `--direction`: which side of the flows `--app` is on.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectionArg {
    /// The app is the destination
    In,
    /// The app is the source
    Out,
    /// Either (two queries, merged)
    Both,
}

impl From<DirectionArg> for FocusDirection {
    fn from(d: DirectionArg) -> Self {
        match d {
            DirectionArg::In => FocusDirection::In,
            DirectionArg::Out => FocusDirection::Out,
            DirectionArg::Both => FocusDirection::Both,
        }
    }
}

/// `--app` / `--direction` against the keys the command groups by: the
/// focus, and whether its labels go into the query. A name-keyed group
/// (`unmanaged-nas/-`, `ipl:Corp/-`) has no labels to query by: with `--by`
/// sections it keeps narrowing an unfocused query's sections, as before.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliFocus {
    pub focus: Focus,
    pub query: bool,
}

/// The offline checks of `--app`: `--direction` with `--by` sections (the
/// sections already choose a direction), `--backfill` with a focused query
/// (its slices would not be focused). Without `--direction` the direction is
/// `--by`'s (`in`, `out`), else `both`.
pub fn cli_focus(
    args: &FilterArgs,
    keys: &[String],
    by: ByArg,
) -> Result<Option<CliFocus>, CliError> {
    let Some(text) = &args.app else {
        return Ok(None);
    };
    if args.direction.is_some() && by != ByArg::Pairs {
        return Err(CliError::Args(
            "--direction conflicts with --by: the sections already choose a direction".into(),
        ));
    }
    let direction = match (args.direction, by) {
        (Some(d), _) => d.into(),
        (None, ByArg::In) => FocusDirection::In,
        (None, ByArg::Out) => FocusDirection::Out,
        (None, _) => FocusDirection::Both,
    };
    let focus = Focus::new(parse_app(text), keys.to_vec()).with_direction(direction);
    let query = focus.is_label_group() || by == ByArg::Pairs;
    if query && args.backfill {
        return Err(CliError::Args(
            "--backfill does not combine with --app: the slices would not be focused".into(),
        ));
    }
    Ok(Some(CliFocus { focus, query }))
}
```

After `impl TableRows for DryRun { ... }` add:

```rust
/// `--dry-run` with `--app`: one request body per side, `in` first.
#[derive(Debug, Serialize)]
#[serde(transparent)]
pub struct DryRuns(pub Vec<TrafficQuery>);

impl TableRows for DryRuns {
    fn headers(&self) -> Vec<&'static str> {
        vec!["request body"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.0
            .iter()
            .flat_map(|q| {
                DryRun(q.clone())
                    .rows()
                    .into_iter()
                    .chain(std::iter::once(vec![String::new()]))
            })
            .collect()
    }
}
```

Replace `app_miss` and `sections_for`:

```rust
/// `no app matches "pos/PCI"` when `--app` kept no section.
pub fn app_miss(args: &GroupsArgs, s: &Sections) -> Option<String> {
    args.filter
        .app
        .as_ref()
        .filter(|_| s.sections.is_empty())
        .map(|a| format!("no app matches {a:?}"))
}

/// `--by`, `--show` and `--app` over the loaded flows: the sections (only
/// `--app`'s with it), or `None` for `pairs` (see `pairs_for`).
pub fn sections_for(
    flows: &[ResolvedFlow],
    keys: &[String],
    args: &GroupsArgs,
) -> Result<Option<Sections>, CliError> {
    let decisions = Decisions::from(args.show);
    let mut sections = match args.by {
        ByArg::Pairs => return Ok(None),
        ByArg::In => app_sections(flows, keys, Direction::In, decisions),
        ByArg::Out => app_sections(flows, keys, Direction::Out, decisions),
        ByArg::Both => app_sections_both(flows, keys, decisions),
    };
    if let Some(app) = &args.filter.app {
        sections.retain(|s| &s.app_text() == app);
    }
    Ok(Some(Sections {
        sections,
        both: args.by == ByArg::Both,
    }))
}
```

In `struct Fetched` add after `slices`:

```rust
    /// `--app`: the Explorer queries the result took.
    queries: Option<u32>,
```

and in `Fetched::meta` add `m.queries = self.queries;`.

Replace `fetch`:

```rust
/// Builds and runs the query (cost guard, cache, one name index): with a
/// focused `--app`, one query per side through `Pce::focused_traffic`,
/// after the app's labels are checked (an unknown one is exit 2 before any
/// traffic request). A dry run prints the request body (with `--app`, the
/// bodies) and returns `None`.
async fn fetch(
    ctx: &Context,
    args: &FilterArgs,
    focus: Option<&CliFocus>,
) -> Result<Option<Fetched>, CliError> {
    let BuiltQuery { query: q, index } = build_query(ctx, args).await?;
    let window = Some(Window {
        start: q.start_date.clone(),
        end: q.end_date.clone(),
    });
    let focus = focus.filter(|f| f.query).map(|f| &f.focus);
    if args.dry_run && focus.is_none() {
        ctx.emit(&DryRun(q), "none", false, window)?;
        return Ok(None);
    }
    let idx = match index {
        Some(idx) => idx,
        None => ctx.pce.name_index(ctx.cache_mode).await?,
    };
    if let Some(focus) = focus {
        let queries = focus.queries(&q, &idx).map_err(CliError::Args)?;
        if args.dry_run {
            let bodies = DryRuns(queries.into_iter().map(|(_, q)| q).collect());
            ctx.emit(&bodies, "none", false, window)?;
            return Ok(None);
        }
        let ft = ctx
            .pce
            .focused_traffic(
                "focus",
                focus,
                &q,
                &idx,
                ctx.cache_mode,
                &CancellationToken::new(),
                None,
            )
            .await?;
        let covered = if ft.truncated {
            coverage(&ft.flows, q.window()).covered
        } else {
            q.window()
        };
        return Ok(Some(Fetched {
            queries: Some(ft.query_count()),
            cache: cache_label(ft.hit, ctx.cache_mode),
            truncated: ft.truncated,
            window,
            covered,
            slices: None,
            flows: ft.flows,
        }));
    }
    let result = ctx
        .pce
        .traffic(&q, ctx.cache_mode, &CancellationToken::new(), None)
        .await?;
    let mut f = Fetched {
        flows: resolve_flows(&result.value.flows, &idx),
        cache: cache_label(result.hit, ctx.cache_mode),
        truncated: result.truncated,
        window,
        covered: Duration::ZERO,
        slices: None,
        queries: None,
    };
    // A complete result covers its window, whatever its rows' times.
    f.covered = if f.truncated {
        coverage(&f.flows, q.window()).covered
    } else {
        q.window()
    };
    if args.backfill {
        f.slices = Some(0);
        if f.truncated
            && let Some(b) = backfill(ctx, args, &q, &idx, &mut f.flows).await?
        {
            f.slices = Some(b.slices);
            f.covered = b.covered;
            f.truncated = b.truncated;
        }
    }
    Ok(Some(f))
}
```

Replace the first lines of `query`, `groups` and `mesh_cmd`:

```rust
async fn query(ctx: &Context, args: &QueryArgs) -> Result<(), CliError> {
    let keys = if args.group.is_empty() {
        keys_or_profile(ctx, &[])
    } else {
        args.group.clone()
    };
    let focus = cli_focus(&args.filter, &keys, ByArg::Pairs)?;
    let Some(f) = fetch(ctx, &args.filter, focus.as_ref()).await? else {
        return Ok(());
    };
```

```rust
async fn groups(ctx: &Context, args: &GroupsArgs) -> Result<(), CliError> {
    let keys = keys_or_profile(ctx, &args.keys);
    // Bad flag combinations fail before any request.
    sections_for(&[], &[], args)?;
    let focus = cli_focus(&args.filter, &keys, args.by)?;
    let Some(f) = fetch(ctx, &args.filter, focus.as_ref()).await? else {
        return Ok(());
    };
```

(and delete the now-duplicate `let keys = keys_or_profile(ctx, &args.keys);` further down in `groups`),

```rust
async fn mesh_cmd(ctx: &Context, args: &MeshArgs) -> Result<(), CliError> {
    let keys = keys_or_profile(ctx, &args.keys);
    let focus = cli_focus(&args.filter, &keys, ByArg::Pairs)?;
    let Some(f) = fetch(ctx, &args.filter, focus.as_ref()).await? else {
        return Ok(());
    };
```

(and delete its later duplicate `let keys = ...`).

`cargo test -p illumio-tui --test cli every_flag_has_help_text` must still pass: both new flags carry doc comments.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p illumio-tui --test cli traffic`
Expected: PASS, every traffic CLI test, old and new (the dry-run tests without `--app` still print one body object).

- [ ] **Step 5: Gate and commit**

```bash
cargo fmt --all && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add crates/illumio-tui/src/cli/traffic.rs crates/illumio-tui/src/cli/output.rs crates/illumio-tui/src/cli/context.rs crates/illumio-tui/src/cli/profile.rs crates/illumio-tui/tests/cli.rs
git commit -m "feat(cli): --app and --direction focus traffic query, groups and mesh on one app

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

---

### Task 7: Docs, in-TUI help, tracking, read-only smoke, final gate

**Files:**
- Modify: `README.md` (TUI traffic paragraph ~145-172; CLI examples ~236-250; CLI `--app` paragraph ~280-292)
- Modify: `docs/index.html` (views paragraph ~168; CLI paragraph ~207)
- Modify: `crates/illumio-tui/src/tui/ui/widgets/overlay.rs` (`HELP_LINES` ~20-157)
- Modify: `tasks/todo.md` (new section)
- Snapshots (re-recorded): `help_overlay_80x24`, `help_overlay_end_80x24`, `help_overlay_end_light_styled_80x24` (and any other help snapshot `cargo test` reports)
- Create (scratchpad only, never committed): `$SCRATCH/focus_smoke.sh`, where `SCRATCH=/private/tmp/claude-501/-Volumes-MacMini-4TB-alex-extern-git-illumio-tui/f0fc1b9b-52cf-486d-a697-e4a11c8ea13a/scratchpad`

**Interfaces:**
- Consumes: everything above. Produces: documentation only.

- [ ] **Step 1: In-TUI help**

In `HELP_LINES`, replace

```rust
    ("", ":profile <name>  :draft  :active  :help  :q"),
```

with

```rust
    ("", ":profile <name>  :draft  :active  :help  :q"),
    ("", ":focus <app>  (alone: a picker of the loaded apps)"),
```

replace

```rust
    ("Traffic  g", "pick the label keys every view groups by"),
```

with

```rust
    ("Traffic  g", "pick the label keys every view groups by"),
    (
        "Traffic  F / X",
        "focus on the row's app (again: its other side) · in/out/both",
    ),
    ("", "Esc clears; truncated: r re-queries the app (1-2 queries)"),
```

and replace

```rust
    (
        "App-to-app x / X",
        "pairs · in · out · both (sections per app) · back",
    ),
```

with

```rust
    (
        "App-to-app x / X",
        "pairs · in · out · both · back (focused: X is direction)",
    ),
```

Run: `INSTA_UPDATE=always cargo test -p illumio-tui --test tui_snapshots && find crates/illumio-tui/tests/snapshots -name '*.snap.new' -delete`
Expected: only help snapshots change; read each diff: the three new lines and the changed `x / X` line, nothing else.

- [ ] **Step 2: README**

Replace

```text
  destination, `Esc` zooms out. None of this sends a request. With writes on, `w` turns a
  blocked flow (or the first one behind a peer row or a Mesh row) into a draft allow rule.
```

with

```text
  destination, `Esc` zooms out. None of this sends a request. With writes on, `w` turns a
  blocked flow (or the first one behind a peer row or a Mesh row) into a draft allow rule.
  `F` focuses all five views on one app, the row's source (on `in` sections and Blocked, its
  destination; `F` again takes the other side; on a row without an app, or with `:focus`
  alone, a picker of the loaded apps; `:focus shop/prod` names it): only that app's traffic is
  shown, `X` cycles `in` (it is the destination), `out` (the source) and `both`, the Result
  header reads `focus shop/prod · in`, and `d`, `o`, search, sort and drills work on top.
  `Esc` clears it; changing the keys with `g` clears it too. Setting it sends nothing. When the
  result was truncated, the header says `focus is partial: r re-queries shop/prod` and `r`
  runs the query again with the app's labels on its side (two queries for `both`, merged
  without duplicates, through the same cost guard); `Esc` then puts the unfocused result back
  without a request.
```

In the CLI examples block, after `    illumio-tui traffic groups --window 1h --by in --show blocked --app pos/PCI` add:

```text
    illumio-tui traffic groups --window 1h --app shop/prod --direction in
```

Replace

```text
(`--decision` filters the query itself), and `--app` (sections only: without `--by` it is
exit 2) keeps one app's sections (an unknown app prints `no app matches` on stderr; sections are
already per service, so `--services` with `--by` is exit 2).
```

with

```text
(`--decision` filters the query itself); sections are already per service, so `--services`
with `--by` is exit 2. `--app shop/prod` (on `traffic query`, `groups` and `mesh`) focuses the
query on one app, a group of the `--keys` (default: the profile's scope keys): its labels go
into the query as destination includes (`--direction in`), source includes (`out`) or both
(`both`, the default: two queries, merged without duplicates), so the cap is spent on that app;
`meta.queries` counts them and a dry run prints one body per side. With `--by in|out|both` the
direction is the sections' (`--direction` is then exit 2) and only the app's sections are kept;
a name-keyed group such as `unmanaged-nas/-` keeps narrowing the sections of an unfocused query.
An app whose labels do not exist is exit 2 with the closest label values, before any traffic
request (the labels are fetched, cached). `--backfill` does not combine with `--app`.
```

- [ ] **Step 3: docs/index.html**

Replace

```html
      <p>Switching views, grouping, sorting and searching send nothing to the PCE. With writes armed, <code>w</code> turns a blocked flow into a draft allow rule.</p>
```

with

```html
      <p>Switching views, grouping, sorting and searching send nothing to the PCE. With writes armed, <code>w</code> turns a blocked flow into a draft allow rule.</p>
      <p><code>F</code> focuses every view on one app: <code>X</code> cycles its inbound, outbound or both directions, <code>Esc</code> clears it. On a truncated result, <code>r</code> re-queries just that app, with its labels in the query.</p>
```

and in the CLI paragraph that starts `<p><code>--group app,env</code> folds flows into label pairs;`, replace its closing `<code>--keys</code> picks the label keys (default: the profile's scope keys).</p>` with

```html
<code>--keys</code> picks the label keys (default: the profile's scope keys). <code>--app shop/prod --direction in</code> spends the cap on one app: its labels go into the query (two queries for <code>both</code>, merged).</p>
```

- [ ] **Step 4: Tracking**

Append to `tasks/todo.md`:

```markdown
## App focus 1: focus (plan docs/superpowers/plans/2026-10-13-app-focus-1-focus.md)
- [ ] Task 1 pce::traffic::focus
- [ ] Task 2 cache slots and Pce::focused_traffic
- [ ] Task 3 TUI focus state and view_flows
- [ ] Task 4 F / X / Esc / g, picker, :focus, badge
- [ ] Task 5 r re-queries a partial focus
- [ ] Task 6 CLI --app / --direction
- [ ] Task 7 docs, help, smoke, gate
```

Tick each item as its task lands.

- [ ] **Step 5: Final gate and dependency check**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git diff main -- '*Cargo.toml' Cargo.lock
```

Expected: the gate passes; the diff is empty. Record the workspace test count.

- [ ] **Step 6: Read-only smoke on demo100**

Nothing here writes, nothing passes `--yes`, nothing reads or prints the env file. Each command costs at most two Explorer queries at cap 200 over 1h.

Create `$SCRATCH/focus_smoke.sh`:

```bash
#!/usr/bin/env bash
# Read-only smoke of --app / --direction against the test PCE.
set -euo pipefail
BIN="$1"; ENVF="$2"
run() { "$BIN" --env-file "$ENVF" "$@"; }
# One unfocused query to find a real label app (cached for nothing: a new process).
APP=$(run traffic groups --window 1h --max-results 200 --by in --json | python3 -I -c '
import json, sys
d = json.load(sys.stdin)
apps = ["/".join(s["app"]) for s in d["data"]["sections"]
        if "-" not in s["app"] and not s["app"][0].startswith("ipl:")]
print(apps[0] if apps else "")')
test -n "$APP" || { echo "no label app in the last hour"; exit 1; }
echo "app: $APP"
run traffic groups --window 1h --max-results 200 --app "$APP" --direction in --json | python3 -I -c '
import json, sys
d = json.load(sys.stdin)
assert d["meta"]["queries"] == 1, d["meta"]
app = sys.argv[1].split("/")
assert all(p["dst"] == app for p in d["data"]), "every pair ends in the app"
print("groups --direction in:", len(d["data"]), "pairs,", d["meta"]["api_calls"], "api calls")' "$APP"
run traffic query --window 1h --max-results 200 --app "$APP" --dry-run --json | python3 -I -c '
import json, sys
d = json.load(sys.stdin)
assert len(d["data"]) == 2 and d["meta"]["cache"] == "none"
print("dry run: 2 bodies, no traffic request")'
set +e
run traffic groups --window 1h --max-results 200 --app "no-such-app/none" --json >/dev/null 2>"$3/err.txt"
code=$?
set -e
test "$code" = 2 && grep -q "unknown app" "$3/err.txt" && echo "unknown app: exit 2"
```

Run it:

```bash
cargo build -p illumio-tui
mkdir -p "$SCRATCH/focus-smoke"
bash "$SCRATCH/focus_smoke.sh" target/debug/illumio-tui ../illumio-mcp-server/.env "$SCRATCH/focus-smoke"
```

Expected: the app name, `groups --direction in: N pairs, M api calls`, `dry run: 2 bodies, no traffic request`, `unknown app: exit 2`. If an assertion fails, stop and debug (superpowers:systematic-debugging); do not retry against the PCE in a loop.

- [ ] **Step 7: Review section**

Append `## Review (App focus 1)` to `tasks/todo.md`: what shipped (one line each: the pce predicate and focused queries; cache slots; `F` / `X` / `Esc` / `g` / picker / `:focus` / badge; `r` on a partial focus; CLI `--app` / `--direction`), the final test count, the empty dependency diff, the smoke's app, pair count and `api_calls`, the statement that nothing was written to any PCE, and every deviation a task reported.

- [ ] **Step 8: Commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add README.md docs/index.html crates/illumio-tui/src/tui/ui/widgets/overlay.rs crates/illumio-tui/tests/snapshots/ tasks/todo.md
git commit -m "docs: app focus (F, X, :focus, r on a partial focus) and traffic --app/--direction

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

---

## Self-review

**1. Spec coverage**

| Spec | Requirement | Task (test) |
| --- | --- | --- |
| Definitions | app = group under the `g` keys; direction in/out/both; self-traffic once in `both`, in each of `in`/`out` | 1 (`keeps_follows_the_direction_and_counts_self_traffic_once`) |
| Approach | one predicate `Focus::keeps` in `pce::traffic`; applied where `Decisions::kept` runs; `d`/`o`/search/sort on top | 1; 3 (`the_decision_filter_and_by_service_work_on_top_of_the_focus`) |
| Part 1 TUI | `F` on pair/section/flow/mesh/treemap; destination first on `in` sections; second `F` switches | 3 (`f_takes_the_source_then_...`); 4 (`f_on_a_row_takes_its_source_then_its_destination`) |
| | `F` without an app / `:focus <app>`: picker with completion | 4 (`f_without_a_row_opens_the_picker_...`, `app_picker` tests, `traffic_app_picker_80x24`) |
| | `X` in → out → both, new focus at `both` | 1 (`x_cycles_...`); 4 (`x_and_esc_cycle_and_clear_the_focus_on_every_view_without_requests`) |
| | every view narrowed; App-to-app keeps pairs/`o`/sections | 3 (`the_focus_narrows_every_view_...`); 4 (every view) |
| | header badge `focus shop/prod · in` | 4 (`traffic_focus_badge_80x24`) |
| | `Esc` clears; `F` on nothing clears; today's `Esc` after | 4 (`x_and_esc_...`, `without_a_focus_x_and_esc_keep_their_old_meaning`, `f_without_a_row_...`) |
| | `g` clears and says so | 4 (`g_clears_the_focus_and_says_so`) |
| Requests | setting/changing the focus sends nothing | 4 (every key test uses `quiet`) |
| | truncated: header `focus is partial: r re-queries`, status asks once | 3 (`focus_meta`); 4 `focus_status`; 5 (`a_truncated_result_offers_r_...`, `traffic_focus_partial_120x30`) |
| | `in`/`out` one query with labels as dst/src includes; `both` two, merged, dedup | 1 (`queries_put_the_app_labels_...`, `union_drops_...`); 2 (`focused_traffic_runs_one_query_per_side_in_its_own_slot`); 5 runtime test |
| | window, filters, cap, confirmation (count in the prompt), caching kept | 1 (base fields kept); 2 (cache slots); 5 (`an_expensive_focused_query_asks_first_and_names_the_count`) |
| | clearing shows the cached unfocused result, no request | 3 (`a_focus_on_a_truncated_result_...`); 5 (`Esc` in `a_truncated_result_offers_r_...`) |
| CLI | `--app` / `--direction` on query, groups, mesh; default `both`; labels into the query | 6 (`traffic_app_dry_run_prints_one_body_per_side`, `traffic_app_spends_the_cap_...`) |
| | `--by --app` keeps working; `--app` without `--by` gives pairs; `--direction` conflicts with `--by` | 6 (`traffic_groups_by_in_out_both_with_show_and_app`, `traffic_app_mistakes_exit_2_...`) |
| | unknown app exits 2 with closest matches, no traffic request | 1 (`an_unknown_app_names_the_closest_labels`); 6 (`traffic_app_mistakes_exit_2_...` with 0 POSTs) |
| | `meta` counts the requests (2 for `both`) | 6 (`meta.queries`, meta line `· 1 query`) |
| Errors | unknown app in `:focus`: status with the closest match, nothing sent | 4 (`focus_names_an_unknown_app_...`) |
| | missing label key: not in the app | 1 (`a_missing_label_key_is_not_the_app`) |
| Testing | pce unit tests; TUI `F`/`X`/`Esc` on each view; badge; `g`; truncated prompt; CLI bodies, exit 2; demo100 smoke `traffic groups --app <app> --direction in` | 1-6; 7 Step 6 |
| Delivery | docs task: README, index.html, help | 7 |

The overlay (`A`) and the timeline are Plans 2 and 3.

**2. Placeholder scan.** No `TBD`, `TODO`, "similar to Task N" or test-less test steps; every code step carries its code. Two steps name a judgement call explicitly (the clippy `allow` in Task 2, the Mesh assertion in Task 3) and say what to do either way.

**3. Type consistency.** `Focus`, `FocusDirection`, `GroupKey`, `FocusedTraffic::query_count`, `focus_flows`, `union_flows`, `apps_in`, `closest`, `parse_app`, `app_labels` (Task 1) are what Tasks 2-6 call. `Pce::focused_traffic(slot, focus, base, idx, mode, cancel, progress)` (Task 2) is called with `"focus"` by the runtime (Task 5) and the CLI (Task 6). `TrafficState::{focus, focus_result, unfocused, view_flows, set_focus, drop_focus, clear_focus, restore_unfocused, back_to_unfocused, cycle_focus_direction, apply_focused, focus_partial, focus_requery, focus_meta, row_apps, next_focus_app, loaded_apps}` and `FocusResult::covers`, `queries_text` (Task 3) are what Tasks 4-5 use. `AppPicker`, `AppPickerPurpose::Focus`, `AppPickerOutcome` (Task 4) are extended by Plan 2 (`AppPickerPurpose::Overlay`). `Cmd::RunFocused`, `Msg::FocusedDone`, `FocusedOutcome`, `PendingAction::RunFocused` (Task 5) match between `msg.rs`, `update.rs` and `app.rs`. `CliFocus`, `cli_focus`, `fetch(ctx, args, focus)`, `Fetched.queries`, `Meta.queries`, `DirectionArg` (Task 6) are what Plan 2's `app` command reuses.

**4. Review Focus.** The five lines above each name the test that pins them: Task 1 (`/` in a value, self-traffic), Task 2 (self-traffic through the facade), Task 5 (non-label group, the focus surviving a new result, a truncated focused result).
