# App Focus (3 of 3): the Exact Blocked Timeline and Spikes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** In the app overlay, `t` replaces the `approx` blocked timeline with an exact one: the window cut by a bucket plan (24×1h for 1d, 28×6h for 1w, 30×1d for 30d, about 24 equal buckets otherwise), one blocked-only focused query per bucket (per side), confirmed with the count, run one at a time, `Esc` stops, answers cached, a stopped run kept as partial and resumed by `t`. Both timelines name their worst spike (a bucket at least 3× the median of the non-zero buckets; none with fewer than 4 non-zero buckets). The CLI gains `illumio-tui app --timeline` (with `--yes` above the threshold).

**Architecture:** Pure additions to `pce::traffic::timeline`: `bucket_count` / `bucket_plan`, `spike` and `Spike`, `bucket_query` (a bucket's blocked-only body), `blocked_connections`, `exact_timeline`, and a `spike` field on `Timeline`. Each bucket runs through Plan 1's `Pce::focused_traffic` in its own cache slot (`timeline:{i}`), so a resumed or repeated bucket is a cache hit. The TUI keeps the run in the overlay's `AppView::exact` (`ExactTimeline`): `t` asks, `y` emits `Cmd::RunTimelineBucket` for the first missing bucket, each `Msg::TimelineBucketDone` stores its count and emits the next; `Esc` cancels the one in flight; reopening the overlay for the same app and filter keeps the loaded buckets. Every bucket query is counted in the overlay header.

**Tech Stack:** Rust 2024 (rust-version 1.88), tokio, tokio-util (`CancellationToken`, already a dependency), ratatui 0.30, crossterm, serde/serde_json, clap 4, jiff, insta (plain snapshot files), wiremock 0.6, assert_cmd. No new dependency.

**Spec:** `docs/superpowers/specs/2026-10-13-app-focus-design.md` (binding): Part 2's "Exact timeline (`t`)" row, "Rough vs exact timeline", "Spikes", the CLI `--timeline` / `--yes`, and "Exact timeline cancelled part-way". Delivery item 3. Builds on Plan 1 (`2026-10-13-app-focus-1-focus.md`) and Plan 2 (`2026-10-13-app-focus-2-overlay.md`), both merged first.

**Tasks:** 6, in this order: 1 `pce::traffic::timeline` bucket plan, spikes, bucket queries, exact timeline; 2 TUI `ExactTimeline` state; 3 TUI `t`, the run, `Esc`, resume, runtime; 4 TUI rendering (exact, partial, spike) and snapshots; 5 CLI `--timeline` / `--yes`; 6 docs, help, tracking, read-only smoke, final gate.

## Global Constraints

- Never hammer the PCE. Every request must be caused by the user, throttled, cached, counted in the header/meta, and visible. Nothing refreshes unsolicited. The exact timeline always asks first with the number of queries, runs one bucket at a time, stops on `Esc`, never restarts on its own, and counts every query in the overlay header and in `meta.queries`.
- No new dependencies. `git diff main -- '*Cargo.toml' Cargo.lock` must be empty.
- The gate is `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`. Run `cargo fmt --all` first and accept its formatting.
- Snapshots are insta files, updated with `INSTA_UPDATE=always` (`INSTA_UPDATE=always cargo test -p illumio-tui --test tui_snapshots`, then `find crates/illumio-tui/tests/snapshots -name '*.snap.new' -delete`). Read every new or changed `.snap`.
- Live smokes on the demo PCE are read-only. Never pass `--yes`, never press `y`. They run with `--env-file ../illumio-mcp-server/.env`. Never read or print that file.
- Every commit message ends with these two lines:
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx
- Elm-style TUI: `update` is pure and returns `Cmd`s; `Runtime` executes them.
- Plan 1 and 2 interfaces used here, by their exact names: `Pce::focused_traffic(slot, focus, base, idx, mode, cancel, progress)`, `FocusedTraffic::{flows, truncated, hit, query_count}`, `Focus`, `FocusDirection::sides`, `TrafficState::window_bounds`, `queries_text`, `Bucket`, `equal_buckets`, `Timeline`, `TimelineBucket`, `approx_timeline`, `APPROX_BUCKETS`, `AppView`, `Loadable`, `open_app`, `app_overlay_key`, `app_enter`, `rebuild_app_traffic`, `render_timeline`, `sparkline`, `cli::app::{AppArgs, AppReport, run}`, `cli::traffic::{build_query, BuiltQuery, cli_focus, fetch}`.

## Spec deviations and resolutions (read before Task 1)

- **`both` needs two queries per bucket.** "One blocked-only query per bucket" holds per side: for `both` each bucket is the `in` and the `out` query, merged without duplicates (Plan 1's rule). The prompt and `meta.queries` count queries, not buckets: 24 buckets × in and out = 48.
- **What a bucket counts.** The exact timeline counts blocked connections (`num_connections` of the blocked flows the focus keeps) per bucket; the approximate one counts flow records (Plan 2). A bucket whose query hits the cap is marked truncated (its count is a lower bound) and drawn in the warning colour.
- **The CLI threshold.** The spec names "the confirm threshold" without a value. This plan sets `TIMELINE_CONFIRM_QUERIES = 24`: one direction over 1d (24 queries) runs without `--yes`; `both` (48) or a week (28) needs it. The check runs before any request. Open question for Alex.
- **Bucket boundaries.** Buckets are laid back from the window's end (the loaded result's range in the TUI, the query's in the CLI), not aligned to the clock hour.
- **Resume.** The loaded buckets live in the overlay's state and survive closing and reopening the overlay for the same app, direction and traffic filter; `t` then queries only the missing buckets. A bucket queried again (after an error) is a cache hit in its slot.
- **Spike tie.** With two equal worst buckets, the earlier one is named.

## Review Focus

- **`Esc` while a bucket is in flight**: the answer that arrives after it must not be stored or start the next bucket. Pinned in Task 3 (`esc_stops_the_run_and_a_late_answer_is_dropped`).
- **A bucket query that fails**: the run stops, the loaded buckets stay, the status says `t resumes`; nothing retries on its own. Pinned in Task 3 (`a_failed_bucket_stops_the_run_and_keeps_the_rest`).
- **`X` or a new traffic result after an exact run**: the exact counts belong to one app, direction and filter; they must not be shown for another. Pinned in Task 3 (`x_shows_the_approx_timeline_for_the_new_direction`).
- **Short or odd windows** (`90m`, `90d`): the plan falls back to about 24 equal buckets and never yields an empty bucket list for a non-empty window. Pinned in Task 1 (`bucket_plans_follow_the_window`).
- **A flat or sparse timeline**: no spike with fewer than 4 non-zero buckets, none below 3× the median, the even-count median averaged. Pinned in Task 1 (`spikes_need_four_non_zero_buckets_and_three_times_the_median`).

---

### Task 1: `pce::traffic::timeline`: bucket plan, spikes, bucket queries, exact timeline

**Files:**
- Modify: `crates/pce/src/traffic/timeline.rs` (Plan 2's module: `Timeline` gains `spike`; `approx_timeline` sets it; new items)
- Modify: `crates/pce/src/traffic/mod.rs` (re-exports)
- Test: `crates/pce/src/traffic/timeline.rs` (`mod tests`)

**Interfaces:**
- Consumes: Plan 2's `Bucket`, `equal_buckets`, `APPROX_BUCKETS`, `Timeline`, `TimelineBucket`, `approx_timeline`; `format_timestamp`; `TrafficQuery`; `PolicyDecision`.
- Produces:

```rust
pub const TIMELINE_CONFIRM_QUERIES: u32 = 24;
pub const SPIKE_MULTIPLE: f64 = 3.0;
pub const SPIKE_MIN_BUCKETS: usize = 4;
pub fn bucket_count(window: std::time::Duration) -> usize;          // 24 / 28 / 30 / 24
pub fn bucket_plan(start: Timestamp, end: Timestamp) -> Vec<Bucket>;
pub struct Spike { pub index: usize, pub start: Timestamp, pub end: Timestamp, pub multiple: f64 }
impl Spike { pub fn text(&self) -> String; }                         // spike 09:00 (x6 median)
pub fn spike(counts: &[u64]) -> Option<(usize, f64)>;
pub struct Timeline { pub exact: bool, pub partial: bool, pub buckets: Vec<TimelineBucket>, pub spike: Option<Spike> }
impl Timeline { pub fn with_spike(self) -> Self; pub fn loaded(&self) -> usize; }
pub fn bucket_query(base: &TrafficQuery, b: &Bucket) -> TrafficQuery;
pub fn blocked_connections(flows: &[ResolvedFlow]) -> u64;
pub fn exact_timeline(buckets: &[Bucket], counts: &[Option<(u64, bool)>]) -> Timeline;
```

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `crates/pce/src/traffic/timeline.rs` (and `use crate::traffic::resolved::tests::flows;`, `use std::time::Duration;` to its imports):

```rust
    #[test]
    fn bucket_plans_follow_the_window() {
        let end = ts("2026-10-10T08:00:00Z");
        let plan = |secs: i64| {
            bucket_plan(end.checked_sub(SignedDuration::from_secs(secs)).unwrap(), end)
        };
        let day = plan(86_400);
        assert_eq!(day.len(), 24);
        assert_eq!(day[0].end.duration_since(day[0].start), SignedDuration::from_hours(1));
        let week = plan(7 * 86_400);
        assert_eq!(week.len(), 28);
        assert_eq!(week[0].end.duration_since(week[0].start), SignedDuration::from_hours(6));
        let month = plan(30 * 86_400);
        assert_eq!(month.len(), 30);
        assert_eq!(month[0].end.duration_since(month[0].start), SignedDuration::from_hours(24));
        for odd in [5_400, 2 * 3_600, 90 * 86_400] {
            let p = plan(odd);
            assert_eq!(p.len(), APPROX_BUCKETS, "{odd}s: about 24 equal buckets");
            assert_eq!(p.last().unwrap().end, end);
        }
        assert_eq!(bucket_count(Duration::from_secs(86_400)), 24);
        assert_eq!(bucket_count(Duration::from_secs(7 * 86_400)), 28);
        assert_eq!(bucket_count(Duration::from_secs(30 * 86_400)), 30);
        assert_eq!(bucket_count(Duration::from_secs(3_600)), 24);
        assert!(bucket_plan(end, end).is_empty());
    }

    #[test]
    fn spikes_need_four_non_zero_buckets_and_three_times_the_median() {
        assert_eq!(spike(&[1, 1, 2, 1, 1, 1, 12, 2, 1]), Some((6, 12.0)));
        assert_eq!(spike(&[0, 5, 0, 30, 0, 0]), None, "two non-zero buckets: never a spike");
        assert_eq!(spike(&[2, 2, 2, 5]), None, "2.5x the median is not a spike");
        assert_eq!(spike(&[2, 2, 2, 6]), Some((3, 3.0)), "exactly 3x is");
        assert_eq!(
            spike(&[1, 3, 0, 4, 20]),
            Some((4, 20.0 / 3.5)),
            "even count: the median of 1 3 4 20 is 3.5"
        );
        assert_eq!(spike(&[9, 1, 1, 9, 1, 1]), Some((0, 9.0)), "a tie names the earlier");
        assert_eq!(spike(&[]), None);
    }

    #[test]
    fn spike_text_names_the_time_and_the_multiple() {
        let s = Spike {
            index: 6,
            start: ts("2026-10-09T14:00:00Z"),
            end: ts("2026-10-09T15:00:00Z"),
            multiple: 6.2,
        };
        assert_eq!(s.text(), "spike 14:00 (x6 median)");
        let d = Spike {
            index: 3,
            start: ts("2026-10-04T08:00:00Z"),
            end: ts("2026-10-05T08:00:00Z"),
            multiple: 3.0,
        };
        assert_eq!(d.text(), "spike 10-04 (x3 median)");
    }

    #[test]
    fn a_bucket_query_is_the_base_narrowed_to_blocked_and_the_bucket() {
        use crate::traffic::query::TrafficQueryBuilder;
        let base = TrafficQueryBuilder::between(ts("2026-10-09T08:00:00Z"), ts("2026-10-10T08:00:00Z"))
            .max_results(500)
            .build()
            .unwrap();
        let b = Bucket {
            start: ts("2026-10-09T09:00:00Z"),
            end: ts("2026-10-09T10:00:00Z"),
        };
        let q = bucket_query(&base, &b);
        assert_eq!(q.start_date, "2026-10-09T09:00:00Z");
        assert_eq!(q.end_date, "2026-10-09T10:00:00Z");
        assert_eq!(q.policy_decisions, vec![PolicyDecision::Blocked]);
        assert_eq!(q.max_results, 500);
        assert_eq!(q.sources, base.sources);
    }

    #[test]
    fn the_exact_timeline_counts_blocked_connections_and_marks_what_is_missing() {
        assert_eq!(blocked_connections(&flows()), 3, "only the NAS flow is blocked");
        let buckets = equal_buckets(ts("2026-10-10T00:00:00Z"), ts("2026-10-10T06:00:00Z"), 6);
        let full = exact_timeline(
            &buckets,
            &[Some((1, false)), Some((1, false)), Some((2, false)), Some((1, false)), Some((9, true)), Some((1, false))],
        );
        assert!(full.exact && !full.partial);
        assert_eq!(full.loaded(), 6);
        assert!(full.buckets[4].truncated);
        let s = full.spike.as_ref().expect("9 is 9x the median of 1");
        assert_eq!((s.index, s.start), (4, ts("2026-10-10T04:00:00Z")));
        let part = exact_timeline(&buckets, &[Some((1, false)), None, None, None, None, None]);
        assert!(part.partial);
        assert_eq!(part.loaded(), 1);
        assert_eq!(part.buckets[1].count, None);
        assert!(part.spike.is_none());
    }

    #[test]
    fn the_approx_timeline_names_its_spike_too() {
        let buckets = equal_buckets(ts("2026-10-10T07:00:00Z"), ts("2026-10-10T08:00:00Z"), 4);
        let shop = Focus::new(parse_app("shop/prod"), vec!["app".into(), "env".into()]);
        let t = approx_timeline(&rich_flows(), &shop, &buckets);
        assert!(t.spike.is_none(), "one non-zero bucket");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p pce --lib traffic::timeline`
Expected: FAIL to compile with "cannot find function `bucket_plan` in this scope".

- [ ] **Step 3: Write the implementation**

In `crates/pce/src/traffic/timeline.rs`:

Extend the imports:

```rust
use super::query::format_timestamp;
use crate::models::{PolicyDecision, TrafficQuery};
use std::time::Duration;
```

(merging `PolicyDecision` into the existing `crate::models` import).

Add after `APPROX_BUCKETS`:

```rust
/// Above this many bucket queries `illumio-tui app --timeline` needs `--yes`.
pub const TIMELINE_CONFIRM_QUERIES: u32 = 24;
/// A spike is a bucket at least this many times the median of the non-zero ones.
pub const SPIKE_MULTIPLE: f64 = 3.0;
/// With fewer non-zero buckets than this, no spike is named.
pub const SPIKE_MIN_BUCKETS: usize = 4;

/// The exact timeline's bucket count for a window: 24 (of 1h) for 1d, 28
/// (of 6h) for 1w, 30 (of 1d) for 30d, else about 24 equal buckets.
pub fn bucket_count(window: Duration) -> usize {
    match window.as_secs() {
        86_400 => 24,
        604_800 => 28,
        2_592_000 => 30,
        _ => APPROX_BUCKETS,
    }
}

/// `start` to `end` cut by `bucket_count`, oldest first, the last bucket
/// ending at `end`.
pub fn bucket_plan(start: Timestamp, end: Timestamp) -> Vec<Bucket> {
    if end <= start {
        return Vec::new();
    }
    let window = end.duration_since(start).unsigned_abs();
    equal_buckets(start, end, bucket_count(window))
}
```

Add after `Bucket`:

```rust
/// The worst bucket, if it is a spike.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Spike {
    pub index: usize,
    pub start: Timestamp,
    pub end: Timestamp,
    /// The bucket over the median of the non-zero buckets.
    pub multiple: f64,
}

impl Spike {
    /// `spike 09:00 (x6 median)`; a bucket of a day or more by its date
    /// (`spike 10-04 (x3 median)`). UTC, like every timestamp here.
    pub fn text(&self) -> String {
        let day = self.end.duration_since(self.start) >= SignedDuration::from_hours(24);
        let when = if day {
            self.start.strftime("%m-%d").to_string()
        } else {
            self.start.strftime("%H:%M").to_string()
        };
        format!("spike {when} (x{:.0} median)", self.multiple)
    }
}

/// The worst bucket when it is at least `SPIKE_MULTIPLE` times the median
/// of the non-zero buckets: `(index, multiple)`. None with fewer than
/// `SPIKE_MIN_BUCKETS` non-zero buckets; the earlier of equal maxima.
pub fn spike(counts: &[u64]) -> Option<(usize, f64)> {
    let mut nz: Vec<u64> = counts.iter().copied().filter(|&c| c > 0).collect();
    if nz.len() < SPIKE_MIN_BUCKETS {
        return None;
    }
    nz.sort_unstable();
    let mid = nz.len() / 2;
    let median = if nz.len() % 2 == 0 {
        (nz[mid - 1] + nz[mid]) as f64 / 2.0
    } else {
        nz[mid] as f64
    };
    let (index, &max) = counts
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(&a.0)))?;
    let multiple = max as f64 / median;
    (multiple >= SPIKE_MULTIPLE).then_some((index, multiple))
}
```

Change `Timeline` to:

```rust
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Timeline {
    /// `false`: counted from the loaded flows' last-detected times.
    pub exact: bool,
    /// Some buckets are not loaded (an exact timeline stopped part-way).
    pub partial: bool,
    pub buckets: Vec<TimelineBucket>,
    /// The worst bucket when it is a spike (over the loaded buckets).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spike: Option<Spike>,
}
```

(`Eq` is dropped: `Spike` holds an `f64`. Plan 2's code only compares `Timeline`s with `PartialEq`, if at all.)

Add to `impl Timeline`:

```rust
    /// `spike` from the loaded counts.
    pub fn with_spike(mut self) -> Self {
        let counts: Vec<u64> = self.counts();
        self.spike = spike(&counts).map(|(i, multiple)| Spike {
            index: i,
            start: self.buckets[i].start,
            end: self.buckets[i].end,
            multiple,
        });
        self
    }

    /// The buckets with a count.
    pub fn loaded(&self) -> usize {
        self.buckets.iter().filter(|b| b.count.is_some()).count()
    }
```

In `approx_timeline`, add `spike: None,` to the returned `Timeline { ... }` and call `.with_spike()` on it:

```rust
    Timeline {
        exact: false,
        partial: false,
        buckets: /* unchanged */,
        spike: None,
    }
    .with_spike()
```

Add after `approx_timeline`:

```rust
/// One bucket's query: `base` for the bucket's range, blocked flows only.
/// The focus's labels are put in by `Pce::focused_traffic`.
pub fn bucket_query(base: &TrafficQuery, b: &Bucket) -> TrafficQuery {
    let mut q = base.clone();
    q.start_date = format_timestamp(b.start);
    q.end_date = format_timestamp(b.end);
    q.policy_decisions = vec![PolicyDecision::Blocked];
    q
}

/// The connections of the blocked flows: one exact bucket's count.
pub fn blocked_connections(flows: &[ResolvedFlow]) -> u64 {
    flows
        .iter()
        .filter(|f| f.policy_decision == PolicyDecision::Blocked)
        .map(|f| f.num_connections)
        .sum()
}

/// The exact timeline from per-bucket `(connections, truncated)`, `None`
/// for a bucket not loaded yet (then `partial`); the spike over the loaded
/// ones.
pub fn exact_timeline(buckets: &[Bucket], counts: &[Option<(u64, bool)>]) -> Timeline {
    let rows: Vec<TimelineBucket> = buckets
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let c = counts.get(i).copied().flatten();
            TimelineBucket {
                start: b.start,
                end: b.end,
                count: c.map(|(n, _)| n),
                truncated: c.is_some_and(|(_, t)| t),
            }
        })
        .collect();
    Timeline {
        exact: true,
        partial: rows.iter().any(|b| b.count.is_none()),
        buckets: rows,
        spike: None,
    }
    .with_spike()
}
```

In `crates/pce/src/traffic/mod.rs`, replace the Plan 2 `pub use timeline::{...};` with:

```rust
pub use timeline::{
    APPROX_BUCKETS, Bucket, SPIKE_MIN_BUCKETS, SPIKE_MULTIPLE, Spike, TIMELINE_CONFIRM_QUERIES,
    Timeline, TimelineBucket, approx_timeline, blocked_connections, bucket_count, bucket_plan,
    bucket_query, equal_buckets, exact_timeline, spike,
};
```

`SignedDuration::from_hours` exists in jiff 0.2; if a jiff version lacks it, use `SignedDuration::from_secs(3_600 * n)`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p pce --lib traffic::timeline`
Expected: PASS: Plan 2's 3 tests and these 6.

Run: `cargo test --workspace`
Expected: PASS (Plan 2's TUI and CLI code builds `Timeline` only through `approx_timeline`; the JSON of an approximate timeline without a spike is unchanged).

- [ ] **Step 5: Gate and commit**

```bash
cargo fmt --all && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add crates/pce/src/traffic/timeline.rs crates/pce/src/traffic/mod.rs
git commit -m "feat(pce): timeline bucket plan, spikes, blocked-only bucket queries, exact timeline

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

---

### Task 2: TUI `ExactTimeline` state

**Files:**
- Modify: `crates/illumio-tui/src/tui/app_view.rs` (new `ExactTimeline`; `AppView` gains `exact`; `AppView::new`)
- Test: `crates/illumio-tui/src/tui/app_view.rs` (`mod tests`)

**Interfaces:**
- Consumes: Task 1's `bucket_plan`, `exact_timeline`, `Timeline`; Plan 1's `Focus`; `TrafficFilter`.
- Produces:

```rust
// tui/app_view.rs
pub struct ExactTimeline {
    pub buckets: Vec<Bucket>,
    pub counts: Vec<Option<(u64, bool)>>,
    pub focus: Focus,
    pub filter: TrafficFilter,
    pub running: Option<usize>,
    pub cancel: Option<CancellationToken>,
}
impl ExactTimeline {
    pub fn new(buckets: Vec<Bucket>, focus: Focus, filter: TrafficFilter) -> Self;
    pub fn next_missing(&self) -> Option<usize>;
    pub fn remaining(&self) -> usize;
    pub fn loaded(&self) -> usize;
    pub fn is_for(&self, focus: &Focus, filter: &TrafficFilter) -> bool;
    pub fn stop(&mut self) -> bool;                   // cancels the bucket in flight
    pub fn timeline(&self) -> Timeline;
}
pub struct AppView { /* ... */ pub exact: Option<ExactTimeline> }
```

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `crates/illumio-tui/src/tui/app_view.rs`:

```rust
    #[test]
    fn an_exact_run_walks_its_buckets_and_stopping_keeps_what_it_loaded() {
        let m = model_with_window("24h");
        let (s, e) = m.traffic.window_bounds().unwrap();
        let focus = shop(FocusDirection::In);
        let mut x = ExactTimeline::new(
            pce::traffic::bucket_plan(s, e),
            focus.clone(),
            m.traffic.filter.clone(),
        );
        assert_eq!((x.buckets.len(), x.remaining(), x.loaded()), (24, 24, 0));
        assert_eq!(x.next_missing(), Some(0));
        x.counts[0] = Some((3, false));
        x.counts[1] = Some((0, false));
        assert_eq!(x.next_missing(), Some(2));
        assert_eq!((x.remaining(), x.loaded()), (22, 2));
        let token = tokio_util::sync::CancellationToken::new();
        x.running = Some(2);
        x.cancel = Some(token.clone());
        assert!(x.stop());
        assert!(token.is_cancelled() && x.running.is_none());
        assert!(!x.stop(), "nothing in flight");
        let t = x.timeline();
        assert!(t.exact && t.partial);
        assert_eq!(t.loaded(), 2);
        assert!(x.is_for(&focus, &m.traffic.filter));
        assert!(!x.is_for(&shop(FocusDirection::Out), &m.traffic.filter), "another direction");
        let mut wider = m.traffic.filter.clone();
        wider.window = "7d".into();
        assert!(!x.is_for(&focus, &wider), "another filter");
    }

    #[test]
    fn a_new_view_has_no_exact_timeline() {
        let v = AppView::new(shop(FocusDirection::Both), "24h".into(), 1);
        assert!(v.exact.is_none());
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p illumio-tui --lib tui::app_view`
Expected: FAIL to compile with "cannot find type `ExactTimeline`".

- [ ] **Step 3: Write the implementation**

In `crates/illumio-tui/src/tui/app_view.rs`, extend the imports:

```rust
use crate::tui::filter::TrafficFilter;
use pce::traffic::{Bucket, exact_timeline};
use tokio_util::sync::CancellationToken;
```

(merge `Bucket` and `exact_timeline` into the existing `pce::traffic::{...}` import). Add above `pub struct AppView`:

```rust
/// `t`: the exact blocked timeline of one app, direction and traffic
/// filter, one bucket query (per side) at a time. Kept when stopped, so
/// `t` resumes with the missing buckets only.
#[derive(Debug, Clone)]
pub struct ExactTimeline {
    pub buckets: Vec<Bucket>,
    /// Per bucket: blocked connections and whether its query hit the cap;
    /// `None` until loaded.
    pub counts: Vec<Option<(u64, bool)>>,
    pub focus: Focus,
    /// The filter the bucket queries are built from (`bucket_query`
    /// replaces its range and decisions).
    pub filter: TrafficFilter,
    /// The bucket whose query is in flight.
    pub running: Option<usize>,
    pub cancel: Option<CancellationToken>,
}

impl ExactTimeline {
    pub fn new(buckets: Vec<Bucket>, focus: Focus, filter: TrafficFilter) -> Self {
        let n = buckets.len();
        Self {
            buckets,
            counts: vec![None; n],
            focus,
            filter,
            running: None,
            cancel: None,
        }
    }

    /// The oldest bucket without a count.
    pub fn next_missing(&self) -> Option<usize> {
        self.counts.iter().position(Option::is_none)
    }

    pub fn remaining(&self) -> usize {
        self.counts.iter().filter(|c| c.is_none()).count()
    }

    pub fn loaded(&self) -> usize {
        self.counts.len() - self.remaining()
    }

    /// The same app, keys, direction and traffic filter.
    pub fn is_for(&self, focus: &Focus, filter: &TrafficFilter) -> bool {
        &self.focus == focus && &self.filter == filter
    }

    /// `Esc`: cancels the bucket in flight (its answer is then dropped);
    /// whether one was.
    pub fn stop(&mut self) -> bool {
        self.cancel.take().map(|c| c.cancel());
        self.running.take().is_some()
    }

    pub fn timeline(&self) -> Timeline {
        exact_timeline(&self.buckets, &self.counts)
    }
}
```

Add the field to `AppView` after `cached`:

```rust
    /// `t`: the exact timeline, once asked for.
    pub exact: Option<ExactTimeline>,
```

and `exact: None,` to `AppView::new`.

If clippy flags `self.cancel.take().map(|c| c.cancel());` (`option_map_unit_fn`), write `if let Some(c) = self.cancel.take() { c.cancel(); }`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p illumio-tui --lib tui::app_view`
Expected: PASS.

- [ ] **Step 5: Gate and commit**

```bash
cargo fmt --all && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add crates/illumio-tui/src/tui/app_view.rs
git commit -m "feat(tui): exact timeline state: buckets, counts, the bucket in flight, resume

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

---

### Task 3: `t`, the run, `Esc`, resume and the runtime

**Files:**
- Modify: `crates/illumio-tui/src/tui/msg.rs` (`TimelineBucketOutcome`; `Msg::TimelineBucketDone`; `Cmd::RunTimelineBucket` and its `PartialEq` arm)
- Modify: `crates/illumio-tui/src/tui/model.rs` (`PendingAction::AppTimeline`)
- Modify: `crates/illumio-tui/src/tui/update.rs` (`open_app` keeps a matching exact run; `app_overlay_key`: `t`, `Esc` while running; `app_enter` stops a run first; `Overlay::Confirm` `y` / `n` arms; new `Msg::TimelineBucketDone` arm; new fns `timeline_key`, `next_timeline_bucket`, `stop_timeline`; new test module `timeline_tests`)
- Modify: `crates/illumio-tui/src/tui/app.rs` (`execute`: `Cmd::RunTimelineBucket`; `apply` task-result list)
- Test: `crates/illumio-tui/src/tui/update.rs` (`mod timeline_tests`), `crates/illumio-tui/tests/tui_runtime.rs`

**Interfaces:**
- Consumes: Task 1's `bucket_plan`, `bucket_query`, `blocked_connections`; Task 2's `ExactTimeline`; Plan 2's `open_app`, `app_overlay_key`, `app_enter`, `AppView::{requests, cached, seq, label_hrefs, focus}`; Plan 1's `Pce::focused_traffic`, `queries_text`; `TrafficFilter::to_query_between`.
- Produces:

```rust
// tui/msg.rs
pub struct TimelineBucketOutcome { pub connections: u64, pub truncated: bool, pub hit: bool }
pub enum Msg { /* ... */ TimelineBucketDone { seq: u64, index: usize, result: Result<TimelineBucketOutcome, String> } }
pub enum Cmd { /* ... */ RunTimelineBucket { seq: u64, index: usize, filter: TrafficFilter, focus: Focus,
    start: jiff::Timestamp, end: jiff::Timestamp, cancel: CancellationToken } }
// tui/model.rs
pub enum PendingAction { /* ... */ AppTimeline }
// tui/update.rs
fn timeline_key(model: &mut Model) -> Vec<Cmd>;
fn next_timeline_bucket(model: &mut Model) -> Vec<Cmd>;
fn stop_timeline(model: &mut Model) -> bool;
```

- [ ] **Step 1: Write the failing tests**

Append to the end of `crates/illumio-tui/src/tui/update.rs`:

```rust
#[cfg(test)]
mod timeline_tests {
    use super::*;
    use crate::tui::msg::TimelineBucketOutcome;
    use crate::tui::model::{Model, Overlay};
    use crate::tui::test_support::{model_with_window, with_policy, with_workloads};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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

    /// The rich flows in a known 24h window, workloads and rulesets
    /// loaded, the overlay open on shop/prod at `direction`: 0 requests.
    fn open(direction: &str) -> Model {
        let mut m = with_policy(with_workloads(model_with_window("24h")));
        if direction != "both" {
            quiet(&mut m, Msg::Command(Command::Focus(Some("shop/prod".into()))));
            quiet(&mut m, key('X')); // in
            if direction == "out" {
                quiet(&mut m, key('X'));
            }
        }
        quiet(&mut m, Msg::Command(Command::App(Some("shop/prod".into()))));
        assert_eq!(m.app_view.as_ref().unwrap().requests, 0);
        m
    }

    fn bucket(cmds: &[Cmd]) -> usize {
        match cmds {
            [Cmd::RunTimelineBucket { index, .. }] => *index,
            other => panic!("{other:?}"),
        }
    }

    fn answer(m: &mut Model, index: usize, connections: u64) -> Vec<Cmd> {
        let seq = m.app_view.as_ref().unwrap().seq;
        update(
            m,
            Msg::TimelineBucketDone {
                seq,
                index,
                result: Ok(TimelineBucketOutcome {
                    connections,
                    truncated: false,
                    hit: false,
                }),
            },
        )
    }

    fn exact(m: &Model) -> &crate::tui::app_view::ExactTimeline {
        m.app_view.as_ref().unwrap().exact.as_ref().expect("an exact run")
    }

    #[test]
    fn t_asks_with_the_query_count_and_y_runs_one_bucket_at_a_time() {
        let mut m = open("both");
        quiet(&mut m, key('t'));
        let Overlay::Confirm { reason, .. } = &m.overlay else {
            panic!("{:?}", m.overlay)
        };
        assert_eq!(
            reason,
            "Exact timeline of shop/prod: 48 queries (24 buckets × in and out), one at a time; Esc stops. Run? (y/n)"
        );
        let cmds = update(&mut m, key('y'));
        assert_eq!(m.overlay, Overlay::App);
        let [Cmd::RunTimelineBucket { index, start, end, focus, filter, .. }] = cmds.as_slice() else {
            panic!("{cmds:?}")
        };
        assert_eq!(*index, 0);
        let (s, _) = m.traffic.window_bounds().unwrap();
        assert_eq!(*start, s);
        assert_eq!(end.duration_since(*start), jiff::SignedDuration::from_hours(1));
        assert_eq!(focus.app_text(), "shop/prod");
        assert_eq!(filter, &m.traffic.filter);
        assert_eq!(m.app_view.as_ref().unwrap().requests, 2, "one bucket, two sides");
        assert_eq!(bucket(&answer(&mut m, 0, 4)), 1, "the next bucket follows the answer");
        assert_eq!(exact(&m).counts[0], Some((4, false)));
        assert_eq!(m.app_view.as_ref().unwrap().requests, 4);
    }

    #[test]
    fn the_last_answer_ends_the_run_and_t_then_sends_nothing() {
        let mut m = open("in");
        quiet(&mut m, key('t'));
        let mut cmds = update(&mut m, key('y'));
        for i in 0..24 {
            assert_eq!(bucket(&cmds), i);
            cmds = answer(&mut m, i, if i == 6 { 12 } else { 1 });
        }
        assert!(cmds.is_empty(), "24 of 24: done");
        assert_eq!(m.app_view.as_ref().unwrap().requests, 24, "one side: one query per bucket");
        let t = exact(&m).timeline();
        assert!(!t.partial);
        assert_eq!(t.spike.map(|s| (s.index, s.multiple)), Some((6, 12.0)));
        assert!(m.status.text.starts_with("exact timeline loaded: 24 buckets"), "{}", m.status.text);
        quiet(&mut m, key('t'));
        assert_eq!(m.overlay, Overlay::App, "nothing left to ask about");
    }

    #[test]
    fn esc_stops_the_run_and_a_late_answer_is_dropped() {
        let mut m = open("in");
        quiet(&mut m, key('t'));
        let _ = update(&mut m, key('y'));
        let _ = answer(&mut m, 0, 2);
        quiet(&mut m, kc(KeyCode::Esc));
        assert_eq!(m.overlay, Overlay::App, "the first Esc stops the run, not the overlay");
        assert!(exact(&m).running.is_none());
        assert_eq!(m.status.text, "exact timeline stopped: 1 of 24 buckets (partial); t resumes");
        let seq = m.app_view.as_ref().unwrap().seq;
        quiet(&mut m, Msg::TimelineBucketDone {
            seq,
            index: 1,
            result: Ok(TimelineBucketOutcome { connections: 9, truncated: false, hit: false }),
        });
        assert_eq!(exact(&m).counts[1], None, "the cancelled bucket's answer is dropped");
        quiet(&mut m, kc(KeyCode::Esc));
        assert_eq!(m.overlay, Overlay::None, "the second Esc closes");
    }

    #[test]
    fn t_resumes_with_the_missing_buckets_even_after_reopening() {
        let mut m = open("in");
        quiet(&mut m, key('t'));
        let _ = update(&mut m, key('y'));
        let _ = answer(&mut m, 0, 2);
        let _ = answer(&mut m, 1, 3);
        quiet(&mut m, kc(KeyCode::Esc)); // stop
        quiet(&mut m, kc(KeyCode::Esc)); // close
        quiet(&mut m, Msg::Command(Command::App(Some("shop/prod".into()))));
        assert_eq!(exact(&m).loaded(), 2, "the loaded buckets survive reopening");
        quiet(&mut m, key('t'));
        let Overlay::Confirm { reason, .. } = &m.overlay else {
            panic!("{:?}", m.overlay)
        };
        assert_eq!(
            reason,
            "Exact timeline of shop/prod: 22 queries (22 buckets), one at a time; Esc stops; 2 loaded buckets are kept. Run? (y/n)"
        );
        assert_eq!(bucket(&update(&mut m, key('y'))), 2);
    }

    #[test]
    fn n_sends_nothing_and_returns_to_the_overlay() {
        let mut m = open("both");
        quiet(&mut m, key('t'));
        quiet(&mut m, key('n'));
        assert_eq!(m.overlay, Overlay::App);
        assert_eq!(m.app_view.as_ref().unwrap().requests, 0);
    }

    #[test]
    fn a_failed_bucket_stops_the_run_and_keeps_the_rest() {
        let mut m = open("in");
        quiet(&mut m, key('t'));
        let _ = update(&mut m, key('y'));
        let _ = answer(&mut m, 0, 2);
        let seq = m.app_view.as_ref().unwrap().seq;
        quiet(&mut m, Msg::TimelineBucketDone { seq, index: 1, result: Err("HTTP 503".into()) });
        assert!(exact(&m).running.is_none());
        assert_eq!(exact(&m).loaded(), 1);
        assert_eq!(m.status.text, "exact timeline stopped: HTTP 503; 1 of 24 buckets kept; t resumes");
    }

    #[test]
    fn x_shows_the_approx_timeline_for_the_new_direction() {
        let mut m = open("in");
        quiet(&mut m, key('t'));
        let _ = update(&mut m, key('y'));
        let _ = answer(&mut m, 0, 2);
        quiet(&mut m, key('X'));
        let v = m.app_view.as_ref().unwrap();
        assert!(v.exact.as_ref().is_none_or(|e| e.running.is_none()), "X stops a run");
        assert!(!v.exact.as_ref().is_some_and(|e| e.focus == v.focus), "the exact counts were for in");
    }

    #[test]
    fn enter_stops_a_run_before_leaving() {
        let mut m = open("in");
        quiet(&mut m, key('t'));
        let _ = update(&mut m, key('y'));
        quiet(&mut m, kc(KeyCode::Enter)); // the first inbound peer: to Flows
        assert_eq!(m.overlay, Overlay::None);
        assert!(m.app_view.as_ref().unwrap().exact.as_ref().unwrap().running.is_none());
    }

    #[test]
    fn a_group_without_labels_has_no_exact_timeline() {
        let mut m = with_policy(with_workloads(model_with_window("24h")));
        // A flow from the Corp IP list: its source group `ipl:Corp/-` has no labels.
        m.traffic
            .flows
            .select_where(|r| r.flow.src.ip_lists.first().is_some());
        let _ = update(&mut m, key('A'));
        quiet(&mut m, key('t'));
        assert!(m.status.text.contains("is not a label group"), "{}", m.status.text);
        assert_eq!(m.overlay, Overlay::App);
    }

    #[test]
    fn the_overlay_counts_every_bucket_query() {
        let mut m = open("both");
        let before = m.app_view.as_ref().unwrap().requests;
        let mut sent = 0;
        quiet(&mut m, key('t'));
        let mut cmds = update(&mut m, key('y'));
        for i in 0..5 {
            sent += cmds
                .iter()
                .filter(|c| matches!(c, Cmd::RunTimelineBucket { .. }))
                .count() as u32
                * 2;
            cmds = answer(&mut m, i, 1);
        }
        assert_eq!(m.app_view.as_ref().unwrap().requests - before, sent + 2, "the sixth bucket is in flight");
    }
}
```

Append to `crates/illumio-tui/tests/tui_runtime.rs`:

```rust
#[tokio::test]
async fn an_exact_timeline_bucket_runs_a_blocked_only_focused_query_in_its_slot() {
    use illumio_tui::tui::app_view::{AppView, ExactTimeline};
    use illumio_tui::tui::model::Overlay;
    use illumio_tui::tui::test_support::model_with_window;
    use pce::traffic::{Focus, FocusDirection, bucket_plan, format_timestamp};
    use tokio_util::sync::CancellationToken;
    use wiremock::matchers::body_partial_json;
    let mut m = model_with_window("24h");
    let (start, end) = m.traffic.window_bounds().unwrap();
    let plan = bucket_plan(start, end);
    let b = plan[0];
    let filter = m.traffic.filter.clone();
    let focus = Focus::new(vec!["shop".into(), "prod".into()], vec!["app".into(), "env".into()])
        .with_direction(FocusDirection::In);
    let mut view = AppView::new(focus.clone(), "24h".into(), 1);
    let mut x = ExactTimeline::new(plan, focus.clone(), filter.clone());
    x.running = Some(0);
    view.exact = Some(x);
    view.label_hrefs = Some(vec!["/orgs/5/labels/3".into(), "/orgs/5/labels/4".into()]);
    m.app_view = Some(view);
    m.app_seq = 1;
    m.overlay = Overlay::App;

    let server = MockServer::start().await;
    mount_objects(&server).await;
    let href = "/orgs/5/traffic_flows/async_queries/q-b0";
    // Bucket 0 only: blocked-only, the app on the destination side, the bucket's range.
    Mock::given(method("POST"))
        .and(path("/api/v2/orgs/5/traffic_flows/async_queries"))
        .and(body_partial_json(json!({
            "start_date": format_timestamp(b.start),
            "end_date": format_timestamp(b.end),
            "policy_decisions": ["blocked"],
            "destinations": {"include": [[{"label": {"href": "/orgs/5/labels/3"}}, {"label": {"href": "/orgs/5/labels/4"}}]]}
        })))
        .respond_with(ResponseTemplate::new(202).set_body_json(json!({"href": href, "status": "queued"})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v2{href}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "completed", "result": {"href": format!("{href}/download")}})))
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
    let (mut rt, _tx) = Runtime::new(pce_for(&server), m);
    rt.execute(Cmd::RunTimelineBucket {
        seq: 1,
        index: 0,
        filter,
        focus,
        start: b.start,
        end: b.end,
        cancel: CancellationToken::new(),
    });
    let model = run_loop(
        &mut rt,
        Box::pin(press(vec![(800, KeyCode::Esc), (50, KeyCode::Esc), (50, ch('q'))])),
        &mut |_| {},
    )
    .await;
    let x = model.app_view.as_ref().unwrap().exact.as_ref().unwrap();
    assert_eq!(x.counts[0], Some((3, false)), "the NAS flow's 3 blocked connections");
    assert_eq!(model.busy, 0);
}
```

The mock answers bucket 0's body only. Its answer holds the 4-flow fixture; `Focus::keeps` (in) keeps 3 flows and only the NAS flow is blocked, so the bucket counts 3. The runtime then starts bucket 1, whose body (another `start_date`) no mock matches: wiremock answers 404, the run stops with an error status, and `expect(1)` holds. The two `Esc` stop a run still in flight (if any) and close the overlay; `q` then quits.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p illumio-tui --lib timeline_tests`
Expected: FAIL to compile with "no variant named `TimelineBucketDone` found for enum `Msg`".

- [ ] **Step 3: Write the implementation**

`crates/illumio-tui/src/tui/msg.rs`: after `AppWorkloadsPayload`:

```rust
/// One exact-timeline bucket's answer: the blocked connections the focus
/// keeps, whether a query hit the cap, whether every query was a cache hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimelineBucketOutcome {
    pub connections: u64,
    pub truncated: bool,
    pub hit: bool,
}
```

in `Msg`, after `AppWorkloadsLoaded { ... },`:

```rust
    /// The answer to `Cmd::RunTimelineBucket` for overlay `seq`.
    TimelineBucketDone {
        seq: u64,
        index: usize,
        result: Result<TimelineBucketOutcome, String>,
    },
```

in `Cmd`, after `LoadAppWorkloads { ... },`:

```rust
    /// `t` in the app overlay: one bucket of the exact timeline, `start` to
    /// `end`, blocked only, the app's labels on its side(s), in cache slot
    /// `timeline:{index}`.
    RunTimelineBucket {
        seq: u64,
        index: usize,
        filter: TrafficFilter,
        focus: Focus,
        start: jiff::Timestamp,
        end: jiff::Timestamp,
        cancel: CancellationToken,
    },
```

and its `PartialEq` arm:

```rust
            (
                Cmd::RunTimelineBucket {
                    seq: a,
                    index: ia,
                    filter: fa,
                    focus: xa,
                    start: sa,
                    end: ea,
                    ..
                },
                Cmd::RunTimelineBucket {
                    seq: b,
                    index: ib,
                    filter: fb,
                    focus: xb,
                    start: sb,
                    end: eb,
                    ..
                },
            ) => a == b && ia == ib && fa == fb && xa == xb && sa == sb && ea == eb,
```

`crates/illumio-tui/src/tui/model.rs`: add to `PendingAction` after `ReloadApp,`:

```rust
    /// `t` in the app overlay: the exact timeline's missing buckets.
    AppTimeline,
```

`crates/illumio-tui/src/tui/update.rs`:

Imports: `use crate::tui::app_view::ExactTimeline;` (merge with the Plan 2 import), `TimelineBucketOutcome` in the `crate::tui::msg::{...}` list, `use tokio_util::sync::CancellationToken;`.

In `open_app` (Plan 2), right after `model.app_seq += 1; let seq = model.app_seq;`, add:

```rust
    // An exact run of the same app, direction and filter is kept (stopped):
    // `t` resumes it.
    let kept = model
        .app_view
        .take()
        .and_then(|v| v.exact)
        .filter(|e| e.is_for(&focus, &model.traffic.filter))
        .map(|mut e| {
            e.stop();
            e
        });
```

and right after `let mut v = AppView::new(...);` add `v.exact = kept;`.

In `app_overlay_key`, replace the arm `KeyCode::Esc | KeyCode::Char('q') => model.overlay = Overlay::None,` with:

```rust
        KeyCode::Esc | KeyCode::Char('q') => {
            if stop_timeline(model) {
                return Vec::new();
            }
            model.overlay = Overlay::None;
        }
```

add before the `KeyCode::Enter` arm:

```rust
        KeyCode::Char('t') => return timeline_key(model),
```

and in the `KeyCode::Char('X')` arm, stop a run before the direction changes:

```rust
        KeyCode::Char('X') => {
            if let Some(e) = v.exact.as_mut() {
                e.stop();
            }
            v.focus.direction = v.focus.direction.next();
            rebuild_app_traffic(model);
        }
```

At the top of `app_enter` add:

```rust
    stop_timeline(model);
```

In `on_key`'s `Overlay::Confirm` `y` arm add:

```rust
                    PendingAction::AppTimeline => {
                        model.overlay = Overlay::App;
                        next_timeline_bucket(model)
                    }
```

and extend the Plan 2 cancel-branch check to:

```rust
                if matches!(action, PendingAction::ReloadApp | PendingAction::AppTimeline) {
                    model.overlay = Overlay::App;
                }
```

After the `Msg::AppWorkloadsLoaded` arm add:

```rust
        Msg::TimelineBucketDone { seq, index, result } => {
            let Some(v) = model.app_view.as_mut().filter(|v| v.seq == seq) else {
                return Vec::new(); // an earlier overlay's answer
            };
            let sides = u32::try_from(v.focus.direction.sides().len()).unwrap_or(2);
            let Some(e) = v.exact.as_mut().filter(|e| e.running == Some(index)) else {
                return Vec::new(); // stopped: dropped
            };
            e.running = None;
            e.cancel = None;
            match result {
                Ok(TimelineBucketOutcome {
                    connections,
                    truncated,
                    hit,
                }) => {
                    e.counts[index] = Some((connections, truncated));
                    if hit {
                        v.cached += sides;
                    }
                    next_timeline_bucket(model)
                }
                Err(err) => {
                    let (loaded, total) = (e.loaded(), e.buckets.len());
                    model.status = StatusLine::error(format!(
                        "exact timeline stopped: {err}; {loaded} of {total} buckets kept; t resumes"
                    ));
                    Vec::new()
                }
            }
        }
```

Add after `refresh_app_policy`:

```rust
/// `t` in the overlay: asks to load the exact timeline's missing buckets,
/// naming the query count (buckets × sides). A new bucket plan when there
/// is none for this app, direction and filter. Sends nothing itself.
fn timeline_key(model: &mut Model) -> Vec<Cmd> {
    let filter = model.traffic.filter.clone();
    let bounds = model.traffic.window_bounds();
    let Some(v) = model.app_view.as_mut() else {
        return Vec::new();
    };
    let app = v.focus.app_text();
    if v.label_hrefs.is_none() {
        model.status = StatusLine::info(format!("no exact timeline: {app} is not a label group"));
        return Vec::new();
    }
    if v.exact.as_ref().is_some_and(|e| e.running.is_some()) {
        model.status = StatusLine::info("exact timeline running · Esc stops");
        return Vec::new();
    }
    if !v.exact.as_ref().is_some_and(|e| e.is_for(&v.focus, &filter)) {
        let Some((start, end)) = bounds else {
            model.status = StatusLine::info("no time range in this result: r runs the query first");
            return Vec::new();
        };
        v.exact = Some(ExactTimeline::new(
            pce::traffic::bucket_plan(start, end),
            v.focus.clone(),
            filter,
        ));
    }
    let Some(e) = v.exact.as_ref() else {
        return Vec::new();
    };
    let remaining = e.remaining();
    if remaining == 0 {
        model.status = StatusLine::info(format!("exact timeline loaded: {} buckets", e.buckets.len()));
        return Vec::new();
    }
    let sides = v.focus.direction.sides().len();
    let queries = u32::try_from(remaining * sides).unwrap_or(u32::MAX);
    let per = if sides == 2 { " × in and out" } else { "" };
    let kept = match e.loaded() {
        0 => String::new(),
        n => format!("; {n} loaded bucket{} kept", if n == 1 { " is" } else { "s are" }),
    };
    model.overlay = Overlay::Confirm {
        reason: format!(
            "Exact timeline of {app}: {} ({remaining} bucket{}{per}), one at a time; Esc stops{kept}. Run? (y/n)",
            queries_text(queries),
            if remaining == 1 { "" } else { "s" }
        ),
        action: PendingAction::AppTimeline,
    };
    Vec::new()
}

/// The next missing bucket's query (one per side, counted), or the end of
/// the run.
fn next_timeline_bucket(model: &mut Model) -> Vec<Cmd> {
    let Some(v) = model.app_view.as_mut() else {
        return Vec::new();
    };
    let seq = v.seq;
    let sides = u32::try_from(v.focus.direction.sides().len()).unwrap_or(2);
    let Some(e) = v.exact.as_mut() else {
        return Vec::new();
    };
    let total = e.buckets.len();
    let Some(index) = e.next_missing() else {
        model.status = StatusLine::info(format!("exact timeline loaded: {total} buckets"));
        return Vec::new();
    };
    let cancel = CancellationToken::new();
    e.running = Some(index);
    e.cancel = Some(cancel.clone());
    let b = e.buckets[index];
    let cmd = Cmd::RunTimelineBucket {
        seq,
        index,
        filter: e.filter.clone(),
        focus: e.focus.clone(),
        start: b.start,
        end: b.end,
        cancel,
    };
    v.requests += sides;
    model.status = StatusLine::info(format!(
        "exact timeline: bucket {} of {total} · Esc stops",
        index + 1
    ));
    vec![cmd]
}

/// Stops an exact run in flight (its answer is dropped); whether one was.
fn stop_timeline(model: &mut Model) -> bool {
    let Some(e) = model.app_view.as_mut().and_then(|v| v.exact.as_mut()) else {
        return false;
    };
    if !e.stop() {
        return false;
    }
    model.status = StatusLine::info(format!(
        "exact timeline stopped: {} of {} buckets (partial); t resumes",
        e.loaded(),
        e.buckets.len()
    ));
    true
}
```

`crates/illumio-tui/src/tui/app.rs`: in `execute`, after the `Cmd::LoadAppWorkloads` arm:

```rust
            Cmd::RunTimelineBucket {
                seq,
                index,
                filter,
                focus,
                start,
                end,
                cancel,
            } => {
                self.model.busy += 1;
                let (pce, tx) = self.task_ctx();
                tokio::spawn(async move {
                    let result = async {
                        let q = filter
                            .to_query_between(start, end, &pce, CacheMode::Use)
                            .await?;
                        let base = pce::traffic::bucket_query(&q, &pce::traffic::Bucket { start, end });
                        let idx = pce.name_index(CacheMode::Use).await?;
                        let ft = pce
                            .focused_traffic(
                                &format!("timeline:{index}"),
                                &focus,
                                &base,
                                &idx,
                                CacheMode::Use,
                                &cancel,
                                None,
                            )
                            .await?;
                        Ok::<_, pce::PceError>(crate::tui::msg::TimelineBucketOutcome {
                            connections: pce::traffic::blocked_connections(&ft.flows),
                            truncated: ft.truncated,
                            hit: ft.hit,
                        })
                    }
                    .await
                    .map_err(|e| e.to_string());
                    let _ = tx.send(Msg::TimelineBucketDone { seq, index, result });
                    let _ = tx.send(Msg::Stats(pce.stats()));
                });
            }
```

and add `| Msg::TimelineBucketDone { .. }` to `apply`'s `task_result` list.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p illumio-tui --lib timeline_tests app_tests`
Expected: PASS (Plan 2's `app_tests` too: `Esc` without a run still closes the overlay).

Run: `cargo test -p illumio-tui --test tui_runtime an_exact_timeline_bucket`
Expected: PASS.

- [ ] **Step 5: Gate and commit**

```bash
cargo fmt --all && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add crates/illumio-tui/src/tui/msg.rs crates/illumio-tui/src/tui/model.rs crates/illumio-tui/src/tui/update.rs crates/illumio-tui/src/tui/app.rs crates/illumio-tui/tests/tui_runtime.rs
git commit -m "feat(tui): t loads the exact blocked timeline, bucket by bucket; Esc stops; t resumes

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

---

### Task 4: Drawing the exact timeline, partial buckets and the spike

**Files:**
- Modify: `crates/illumio-tui/src/tui/ui/widgets/app_overlay.rs` (`render_timeline`; new `exact_sparkline`)
- Test: `crates/illumio-tui/tests/tui_snapshots.rs`, `crates/illumio-tui/src/tui/ui/widgets/app_overlay.rs` (`mod tests`)
- Snapshots (new): `app_overlay_exact_80x24`, `app_overlay_exact_partial_80x24`; (re-recorded) Plan 2's `app_overlay_*` ones whose timeline line now ends with `t: exact`

**Interfaces:**
- Consumes: Task 1's `Timeline::{spike, loaded}`, `Spike::text`; Task 2's `ExactTimeline::{timeline, running, loaded}`; Plan 2's `render_timeline`, `sparkline`.
- Produces:

```rust
pub fn exact_sparkline(buckets: &[TimelineBucket], width: usize) -> Vec<Span<'static>>;
```

- [ ] **Step 1: Write the failing tests**

Add to `crates/illumio-tui/src/tui/ui/widgets/app_overlay.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::ui::theme::Theme;
    use pce::traffic::TimelineBucket;

    fn b(count: Option<u64>, truncated: bool) -> TimelineBucket {
        TimelineBucket {
            start: "2026-10-10T00:00:00Z".parse().unwrap(),
            end: "2026-10-10T01:00:00Z".parse().unwrap(),
            count,
            truncated,
        }
    }

    #[test]
    fn exact_sparkline_dots_the_missing_buckets_and_warns_on_truncated_ones() {
        let t = Theme::from_name("dark");
        let spans = exact_sparkline(&[b(Some(1), false), b(None, false), b(Some(8), true)], 10, &t);
        let text: String = spans.iter().map(|s| s.content.to_string()).collect();
        assert_eq!(text, "▁·█");
        assert_eq!(spans[2].style.fg, Some(t.warn), "a capped bucket is a lower bound");
        assert_eq!(spans[1].style, t.dim_style());
    }
}
```

(`exact_sparkline` takes the theme too: `pub fn exact_sparkline(buckets: &[TimelineBucket], width: usize, t: &Theme) -> Vec<Span<'static>>`.)

Append to `crates/illumio-tui/tests/tui_snapshots.rs`:

```rust
/// `app_overlay_model` with an exact run of `loaded` buckets (of 24), one
/// spike at bucket 6.
fn app_overlay_exact(loaded: usize) -> illumio_tui::tui::model::Model {
    use illumio_tui::tui::app_view::ExactTimeline;
    let mut m = app_overlay_model();
    let (start, end) = m.traffic.window_bounds().unwrap();
    let v = m.app_view.as_mut().unwrap();
    let mut x = ExactTimeline::new(
        pce::traffic::bucket_plan(start, end),
        v.focus.clone(),
        m.traffic.filter.clone(),
    );
    for i in 0..loaded {
        x.counts[i] = Some((if i == 6 { 12 } else { 1 }, false));
    }
    v.exact = Some(x);
    v.requests = 2 * loaded as u32;
    m
}

#[test]
fn app_overlay_exact_80x24() {
    let m = app_overlay_exact(24);
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("spike 14:00 (x12 median)"), "bucket 6 starts 14:00 UTC: {out}");
    assert!(out.contains("exact"), "{out}");
    assert!(!out.contains("approx"), "the exact timeline replaces the approximate one: {out}");
    assert!(out.contains("48 req"), "{out}");
    insta::assert_snapshot!("app_overlay_exact_80x24", out);
}

#[test]
fn app_overlay_exact_partial_80x24() {
    let m = app_overlay_exact(9);
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("exact · partial 9/24 · t resumes"), "{out}");
    insta::assert_snapshot!("app_overlay_exact_partial_80x24", out);
}
```

and add to Plan 2's `app_overlay_80x24` assertions list the string `"t: exact"`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p illumio-tui --test tui_snapshots app_overlay_exact`
Expected: FAIL: no `spike` text and no `exact` panel meta.

- [ ] **Step 3: Write the implementation**

In `crates/illumio-tui/src/tui/ui/widgets/app_overlay.rs`, import `pce::traffic::TimelineBucket`, and add after `sparkline`:

```rust
/// The exact timeline's cells: loaded buckets scaled to the largest (in the
/// blocked colour, a capped one in the warning colour), a missing one `·`.
pub fn exact_sparkline(buckets: &[TimelineBucket], width: usize, t: &Theme) -> Vec<Span<'static>> {
    let max = buckets
        .iter()
        .filter_map(|b| b.count)
        .max()
        .unwrap_or(0)
        .max(1);
    let skip = buckets.len().saturating_sub(width);
    buckets
        .iter()
        .skip(skip)
        .map(|b| match b.count {
            None => Span::styled("·", t.dim_style()),
            Some(v) => {
                let i = ((v * 7) / max) as usize;
                let c = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'][i.min(7)];
                let colour = if b.truncated { t.warn } else { t.bad };
                Span::styled(c.to_string(), Style::new().fg(colour))
            }
        })
        .collect()
}
```

Replace `render_timeline` with:

```rust
/// The blocked timeline: the exact one when it was loaded for this view's
/// app and direction (complete, partial or running), else the approximate
/// one from the loaded flows, labelled `approx` with `t: exact`. Either
/// names its spike.
fn render_timeline(v: &AppView, t: &Theme, f: &mut Frame, area: Rect) {
    let exact = v.exact.as_ref().filter(|e| e.focus == v.focus);
    let meta = match exact {
        Some(e) if e.running.is_some() => {
            format!("exact · {}/{} · Esc stops", e.loaded(), e.buckets.len())
        }
        Some(e) if e.remaining() > 0 => {
            format!("exact · partial {}/{} · t resumes", e.loaded(), e.buckets.len())
        }
        Some(e) => format!("exact · {} buckets", e.buckets.len()),
        None => String::new(),
    };
    let block = panel("Blocked over time", &meta, t, false);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let width = (inner.width as usize).saturating_sub(34);
    if let Some(e) = exact {
        let tl = e.timeline();
        let mut spans = exact_sparkline(&tl.buckets, width, t);
        if let Some(s) = &tl.spike {
            spans.push(Span::styled(format!("  {}", s.text()), Style::new().fg(t.bad)));
        } else if tl.loaded() > 0 && tl.is_quiet() {
            spans.push(Span::styled("  nothing blocked", t.dim_style()));
        }
        f.render_widget(Paragraph::new(Line::from(spans)), inner);
        return;
    }
    let Some(tl) = &v.timeline else {
        f.render_widget(note("no time range in this result", t), inner);
        return;
    };
    if tl.is_quiet() {
        f.render_widget(note("no blocked flows in the window · approx · t: exact", t), inner);
        return;
    }
    let mut spans = vec![Span::styled(sparkline(&tl.counts(), width), Style::new().fg(t.bad))];
    if let Some(s) = &tl.spike {
        spans.push(Span::styled(format!("  {}", s.text()), Style::new().fg(t.bad)));
    }
    spans.push(Span::styled("  approx · t: exact", t.dim_style()));
    f.render_widget(Paragraph::new(Line::from(spans)), inner);
}
```

The 34 cells kept free fit `  spike 14:00 (x12 median)` (26) plus `  approx · t: exact` would not both fit at 80 columns with 24 cells of sparkline; the sparkline is cut to the newest buckets that fit (as `sparkline` and `exact_sparkline` already do).

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p illumio-tui --lib app_overlay`
Expected: PASS.

Run: `INSTA_UPDATE=always cargo test -p illumio-tui --test tui_snapshots && find crates/illumio-tui/tests/snapshots -name '*.snap.new' -delete`
Expected: two new snapshots; Plan 2's `app_overlay_*` snapshots change only in the timeline line (`approx · t: exact`, the sparkline possibly shorter). Read every diff.

- [ ] **Step 5: Gate and commit**

```bash
cargo fmt --all && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add crates/illumio-tui/src/tui/ui/widgets/app_overlay.rs crates/illumio-tui/tests/tui_snapshots.rs crates/illumio-tui/tests/snapshots/
git commit -m "feat(tui): draw the exact timeline, its missing buckets and the worst spike

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

---

### Task 5: CLI `illumio-tui app --timeline` / `--yes`

**Files:**
- Modify: `crates/illumio-tui/src/cli/app.rs` (`AppArgs::timeline`; `run`: the offline threshold check, the bucket loop, `meta.queries`)
- Test: `crates/illumio-tui/tests/cli.rs`

**Interfaces:**
- Consumes: Task 1's `bucket_count`, `bucket_plan`, `bucket_query`, `blocked_connections`, `exact_timeline`, `TIMELINE_CONFIRM_QUERIES`; Plan 1's `build_query`, `BuiltQuery`, `Pce::focused_traffic`, `Meta.queries`; Plan 2's `AppArgs`, `AppReport`, `run`.
- Produces: `AppArgs { /* ... */ pub timeline: bool }`.

- [ ] **Step 1: Write the failing tests**

Append to `crates/illumio-tui/tests/cli.rs`:

```rust
#[tokio::test(flavor = "multi_thread")]
async fn app_timeline_runs_one_blocked_only_query_per_bucket() {
    let server = MockServer::start().await;
    // 1 focused query (in) for the peers, then 24 buckets of 1h/24.
    mount_app(&server, 1 + 24).await;
    let out = stdout_json(
        cmd(&server)
            .args(["app", "shop/prod", "--window", "1h", "--direction", "in", "--timeline"])
            .assert()
            .success(),
    );
    let t = &out["data"]["timeline"];
    assert_eq!(t["exact"], true);
    assert_eq!(t["partial"], false);
    let buckets = t["buckets"].as_array().unwrap();
    assert_eq!(buckets.len(), 24);
    assert!(
        buckets.iter().all(|b| b["count"] == 3),
        "every bucket's answer holds the NAS flow's 3 blocked connections"
    );
    assert!(t.get("spike").is_none(), "a flat timeline has no spike");
    assert_eq!(out["meta"]["queries"], 25);
    let posts: Vec<serde_json::Value> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method.as_str() == "POST")
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect();
    assert_eq!(posts.len(), 25);
    assert!(
        posts[1..].iter().all(|b| b["policy_decisions"] == json!(["blocked"])),
        "bucket queries are blocked-only"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn app_timeline_above_the_threshold_needs_yes_before_any_request() {
    let server = MockServer::start().await;
    mount_traffic_quiet(&server).await;
    for args in [
        vec!["app", "shop/prod", "--window", "1h", "--timeline"],
        vec!["app", "shop/prod", "--window", "7d", "--direction", "in", "--timeline"],
    ] {
        let err = cmd(&server).args(&args).assert().code(2);
        let err = String::from_utf8_lossy(&err.get_output().stderr).to_string();
        assert!(err.contains("--timeline sends") && err.contains("--yes"), "{args:?}: {err}");
    }
}
```

`received_requests()` needs wiremock's request recording, on by default. `mount_traffic_quiet` is the Plan 1 helper whose mocks all `expect(0)`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p illumio-tui --test cli app_timeline`
Expected: FAIL: clap rejects `--timeline`.

- [ ] **Step 3: Write the implementation**

In `crates/illumio-tui/src/cli/app.rs`:

Imports: add `use super::traffic::{BuiltQuery, build_query};` (merge), `use tokio_util::sync::CancellationToken;`, and extend `pce::traffic::{...}` with `TIMELINE_CONFIRM_QUERIES, Bucket, blocked_connections, bucket_count, bucket_plan, bucket_query, exact_timeline, parse_window`.

Add to `AppArgs` before `yes`:

```rust
    /// The exact blocked timeline: one blocked-only query per bucket and side (24x1h for
    /// 1d, 28x6h for 1w, 30x1d for 30d, else about 24); above 24 queries needs --yes
    #[arg(long)]
    pub timeline: bool,
```

and change the `yes` doc comment to `/// Confirm an expensive query (window > 7d, max_results > 10000, --timeline above 24 queries)`.

In `run`, right after the `--csv` check, add the offline threshold check:

```rust
    let sides: u32 = match args.direction {
        Some(DirectionArg::In) | Some(DirectionArg::Out) => 1,
        _ => 2,
    };
    if args.timeline {
        let window = parse_window(
            args.window
                .as_deref()
                .unwrap_or(&ctx.config.traffic_default_window),
        )?;
        let queries = u32::try_from(bucket_count(window)).unwrap_or(u32::MAX) * sides;
        if queries > TIMELINE_CONFIRM_QUERIES && !args.yes {
            return Err(CliError::Args(format!(
                "--timeline sends {queries} queries (> {TIMELINE_CONFIRM_QUERIES}): re-run with --yes to confirm"
            )));
        }
    }
```

Replace the `timeline: approx_timeline(&fetched.flows, &focus, &buckets),` field of `AppReport` with `timeline,` and compute `timeline` before building the report (after `let buckets = ...;`):

```rust
    let mut bucket_queries = 0u32;
    let mut bucket_truncated = false;
    let timeline = if args.timeline {
        let BuiltQuery { query: base, .. } = build_query(ctx, &filter).await?;
        let plan = bounds
            .map(|(s, e)| bucket_plan(s, e))
            .unwrap_or_default();
        let mut counts = Vec::with_capacity(plan.len());
        for (i, b) in plan.iter().enumerate() {
            let ft = ctx
                .pce
                .focused_traffic(
                    &format!("timeline:{i}"),
                    &focus,
                    &bucket_query(&base, b),
                    &idx,
                    ctx.cache_mode,
                    &CancellationToken::new(),
                    None,
                )
                .await?;
            bucket_queries += ft.query_count();
            bucket_truncated |= ft.truncated;
            counts.push(Some((blocked_connections(&ft.flows), ft.truncated)));
        }
        exact_timeline(&plan, &counts)
    } else {
        approx_timeline(&fetched.flows, &focus, &buckets)
    };
```

and after `let mut meta = fetched.meta(ctx);` add:

```rust
    meta.queries = Some(meta.queries.unwrap_or(0) + bucket_queries);
    meta.truncated |= bucket_truncated;
```

`build_query` for an `app` filter has no `--src` / `--dst`, so it sends nothing (the window, cap and cost guard are checked offline); `bucket_query` then replaces its range and decisions. `meta` is computed after the bucket loop, so `api_calls` and `bytes` include it. The unused `Bucket` import, if clippy flags it, is removed.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p illumio-tui --test cli app_`
Expected: PASS: Plan 2's three `app_` tests and these two.

- [ ] **Step 5: Gate and commit**

```bash
cargo fmt --all && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add crates/illumio-tui/src/cli/app.rs crates/illumio-tui/tests/cli.rs
git commit -m "feat(cli): app --timeline, one blocked-only query per bucket; --yes above 24 queries

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

---

### Task 6: Docs, in-TUI help, tracking, read-only smoke, final gate

**Files:**
- Modify: `README.md` (the app overlay paragraph from Plan 2; the `illumio-tui app` paragraph; CLI examples)
- Modify: `docs/index.html` (the `A` paragraph from Plan 2)
- Modify: `crates/illumio-tui/src/tui/ui/widgets/overlay.rs` (`HELP_LINES`)
- Modify: `tasks/todo.md`
- Snapshots (re-recorded): the help overlay ones
- Create (scratchpad only): `$SCRATCH/timeline_smoke.sh`, `SCRATCH=/private/tmp/claude-501/-Volumes-MacMini-4TB-alex-extern-git-illumio-tui/f0fc1b9b-52cf-486d-a697-e4a11c8ea13a/scratchpad`

**Interfaces:** documentation only.

- [ ] **Step 1: In-TUI help**

In `HELP_LINES`, after Plan 2's `("App      w r Esc", ...)` entry add:

```rust
    (
        "App      t",
        "exact blocked timeline: 1 query per bucket (asks)",
    ),
    ("", "one at a time; Esc stops, t resumes; spike: x3 median"),
```

Re-record the help snapshots with `INSTA_UPDATE=always cargo test -p illumio-tui --test tui_snapshots` and delete strays; read the diffs.

- [ ] **Step 2: README**

In the app overlay paragraph (Plan 2), replace

```text
  inbound and outbound peers by service and decision, blocked traffic over time (`approx`:
  flow records by last-detected time), and the rulesets whose scope covers it or whose rules
```

with

```text
  inbound and outbound peers by service and decision, blocked traffic over time (`approx`:
  flow records by last-detected time; `t` replaces it with the exact timeline, one
  blocked-only query per bucket and side, 24×1h for 1d, 28×6h for 1w, 30×1d for 30d, about
  24 buckets otherwise, after asking with the count; one at a time, `Esc` stops and keeps
  what loaded, `t` resumes; both name the worst spike, a bucket at least 3× the median of
  the non-zero ones), and the rulesets whose scope covers it or whose rules
```

In the `illumio-tui app` paragraph (Plan 2), replace

```text
bucket of the window, by last-detected time). It costs the focused traffic queries (one per
```

with

```text
bucket of the window, by last-detected time; `--timeline`: `exact: true`, blocked connections
from one blocked-only query per bucket and side, with `spike` when one bucket is at least 3× the
median; above 24 queries it needs `--yes`). It costs the focused traffic queries (one per
```

In the CLI examples add:

```text
    illumio-tui app shop/prod --window 24h --direction in --timeline --json
```

- [ ] **Step 3: docs/index.html**

In Plan 2's `<p><code>A</code> opens one screen for that app: ...</p>`, replace `and blocked traffic over time.` with:

```html
and blocked traffic over time, with its worst spike named; <code>t</code> loads the exact timeline, one blocked-only query per bucket, after asking how many.
```

- [ ] **Step 4: Tracking**

Append to `tasks/todo.md`:

```markdown
## App focus 3: exact timeline (plan docs/superpowers/plans/2026-10-13-app-focus-3-timeline.md)
- [ ] Task 1 pce timeline: bucket plan, spikes, bucket queries, exact timeline
- [ ] Task 2 TUI ExactTimeline state
- [ ] Task 3 t, the run, Esc, resume, runtime
- [ ] Task 4 drawing exact, partial, spike
- [ ] Task 5 CLI --timeline / --yes
- [ ] Task 6 docs, help, smoke, gate
```

- [ ] **Step 5: Final gate and dependency check**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git diff main -- '*Cargo.toml' Cargo.lock
```

Expected: the gate passes; the diff is empty.

- [ ] **Step 6: Read-only smoke on demo100**

Never `--yes`, never `y`; the env file is passed by path, never read. Part A sends no traffic query at all. Part B costs 1 + 24 Explorer queries (cap 50, one at a time) and runs **only if Alex approves it in the session**; otherwise skip it and say so in the review.

Create `$SCRATCH/timeline_smoke.sh`:

```bash
#!/usr/bin/env bash
# Read-only smoke of `app --timeline` against the test PCE.
set -euo pipefail
BIN="$1"; ENVF="$2"; APP="$3"; PART="${4:-A}"
run() { "$BIN" --env-file "$ENVF" "$@"; }
# A: the threshold refuses without --yes, before any request (2 sides x 24 buckets).
set +e
run app "$APP" --window 1h --timeline --json >/dev/null 2>"$SCRATCH_DIR/err.txt"
code=$?
set -e
test "$code" = 2 && grep -q -- "--yes" "$SCRATCH_DIR/err.txt" && echo "A: 48 queries refused without --yes"
[ "$PART" = "B" ] || exit 0
# B (approved only): one direction, 24 buckets, cap 50.
run app "$APP" --window 1h --direction in --max-results 50 --timeline --json | python3 -I -c '
import json, sys
d = json.load(sys.stdin)
t, meta = d["data"]["timeline"], d["meta"]
assert t["exact"] is True and len(t["buckets"]) == 24, t
assert meta["queries"] == 25, meta
print("B: exact timeline,", sum(b["count"] or 0 for b in t["buckets"]), "blocked connections,",
      "spike" if "spike" in t else "no spike", "·", meta["api_calls"], "api calls")'
```

Pick `APP` as Plan 1's smoke did (the first label app of `traffic groups --window 1h --max-results 200 --by in --json`), then:

```bash
cargo build -p illumio-tui
mkdir -p "$SCRATCH/timeline-smoke"
SCRATCH_DIR="$SCRATCH/timeline-smoke" bash "$SCRATCH/timeline_smoke.sh" target/debug/illumio-tui ../illumio-mcp-server/.env "$APP" A
# Only after Alex says yes:
# SCRATCH_DIR="$SCRATCH/timeline-smoke" bash "$SCRATCH/timeline_smoke.sh" target/debug/illumio-tui ../illumio-mcp-server/.env "$APP" B
```

Expected: `A: 48 queries refused without --yes`; with approval, one `B:` line. On a failure stop and debug; never loop against the PCE.

- [ ] **Step 7: Review section**

Append `## Review (App focus 3)` to `tasks/todo.md`: what shipped (one line each), the test count, the empty dependency diff, the smoke parts run (and whether B was approved), the statement that nothing was written, and every deviation a task reported.

- [ ] **Step 8: Commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add README.md docs/index.html crates/illumio-tui/src/tui/ui/widgets/overlay.rs crates/illumio-tui/tests/snapshots/ tasks/todo.md
git commit -m "docs: the exact blocked timeline (t), spikes and app --timeline

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

---

## Self-review

**1. Spec coverage**

| Spec | Requirement | Task (test) |
| --- | --- | --- |
| Exact timeline (`t`) | one blocked-only query per bucket; 24×1h / 28×6h / 30×1d / about 24 | 1 (`bucket_plans_follow_the_window`, `a_bucket_query_is_the_base_narrowed_...`); 3 runtime test (`policy_decisions: ["blocked"]`) |
| | confirmed with the count | 3 (`t_asks_with_the_query_count_...`, `n_sends_nothing_...`) |
| | run one at a time | 3 (`t_asks_...`: one `Cmd` per answer; `the_last_answer_ends_the_run_...`) |
| | `Esc` cancels | 3 (`esc_stops_the_run_and_a_late_answer_is_dropped`, `enter_stops_a_run_before_leaving`) |
| | cached | 3 runtime test (slot `timeline:{i}` through `focused_traffic`); Plan 1's slot tests |
| Rough vs exact | approx labelled; exact replaces it once loaded | 4 (`app_overlay_exact_80x24`: no `approx`; Plan 2 snapshots: `approx · t: exact`) |
| Spikes | ≥3× the median of non-zero buckets; worst named with time and multiple; <4 non-zero: none | 1 (`spikes_need_four_...`, `spike_text_...`, `the_exact_timeline_counts_...`); 4 (`spike 14:00 (x12 median)`) |
| Cancelled part-way | loaded buckets kept and marked partial; `t` resumes from the cache | 2 (`an_exact_run_walks_...`); 3 (`t_resumes_with_the_missing_buckets_even_after_reopening`); 4 (`app_overlay_exact_partial_80x24`) |
| Requests | every request counted and visible | 3 (`the_overlay_counts_every_bucket_query`); 5 (`meta.queries` 25) |
| CLI | `--timeline` adds the bucket queries; above the threshold needs `--yes` | 5 (`app_timeline_runs_one_blocked_only_query_per_bucket`, `app_timeline_above_the_threshold_needs_yes_before_any_request`) |
| Testing | bucket plans; spike detection; `--timeline` refuses without `--yes` | 1; 5 |
| Delivery | docs task | 6 |

**2. Placeholder scan.** No `TBD`, "similar to", or test-less steps. Two steps name a fallback with its exact replacement (a jiff constructor, a clippy rewrite); none leaves a choice open.

**3. Type consistency.** `bucket_count`, `bucket_plan`, `Spike::text`, `spike`, `Timeline::{spike, with_spike, loaded}`, `bucket_query`, `blocked_connections`, `exact_timeline`, `TIMELINE_CONFIRM_QUERIES` (Task 1) are what Tasks 2-5 use. `ExactTimeline::{new, next_missing, remaining, loaded, is_for, stop, timeline}` and `AppView::exact` (Task 2) are what Tasks 3-4 use. `Cmd::RunTimelineBucket`, `Msg::TimelineBucketDone`, `TimelineBucketOutcome`, `PendingAction::AppTimeline` (Task 3) match between `msg.rs`, `update.rs` and `app.rs`. Plan 1 and 2 names are used verbatim (see Global Constraints); `Timeline` loses `Eq` (it now holds an `f64`), which no earlier code relies on.

**4. Review Focus.** Each line names its test: Task 3 (`Esc` with a late answer, a failed bucket, `X` after a run), Task 1 (odd windows, sparse timelines).
