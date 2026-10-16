# Flow marks (2 of 3): the TUI display Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The Traffic screen and the app overlay show marks: `TrafficState` refreshes `marks.toml` at every rebuild (one `stat`), leaves squelched flows out of every view (`M` shows them), joins the marks to every view's rows, draws a 2-cell gutter (`●` `◆` `≈` `✓` `~`), says what is squelched in each panel's meta, takes accepted flows out of Blocked's counts, marks highlighted treemap tiles, names a flow's mark in its detail, and reports invalid files and marks about to expire.

**Architecture:** Squelch is one more filter layer under `view_flows()` (focus, then marks): `refresh_marks` partitions the focused flows with Plan 1's `partition`, keeps the unsquelched copy (only when something is left out) and the owned `Marked`, and every rebuild function (`rebuild_views`, `rebuild_pairs`, `rebuild_sections`) annotates the rows its own aggregator built and stores its counts in `MarkedView`. Nothing here sends a request: `refresh` is a `stat` of a local file. `TableView::marker` is not a column, so sorting, search and `:export` are unchanged; it is set only while a mark is active, so every existing snapshot stays the same.

**Tech Stack:** Rust 2024 (rust-version 1.88), ratatui 0.30, crossterm, insta (plain snapshot files), tempfile. No new dependency.

**Spec:** `docs/superpowers/specs/2026-10-17-flow-marks-design.md` (binding), delivery item 2. Builds on Plan 1 (`2026-10-17-flow-marks-1-model-cli.md`), which must be merged first. Plan 3 (`2026-10-17-flow-marks-3-create-manage.md`) adds `m` and `:marks`.

**Tasks:** 4, in this order: 1 `TrafficState` applies marks (refresh, partition, counts, `M`, notices); 2 drawing (the gutter, panel metas, Blocked's accepted counts, treemap glyphs, the Flows detail, snapshots); 3 the app overlay and the generator header; 4 help, tracking, final gate.

## Global Constraints

- The gate is `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`. Every task ends with it green. The code in this plan is already formatted: if `cargo fmt --all --check` complains, the transcription is wrong, not the formatter.
- No new dependencies. `git diff main -- '*Cargo.toml' Cargo.lock` stays empty: no `toml_edit`; `toml` 1.x and `serde` (already in the workspace) only. The workspace's `rust-version` is 1.88: no `File::lock` (1.89); the lock is `create_new`.
- Marks never cause a PCE request. They are matched against loaded flows only; the mark form completes from the loaded name index and workloads only. No marks path returns a `Cmd` (TUI) or calls the client (CLI).
- `marks.toml` lives next to the config file in use: `$XDG_CONFIG_HOME/illumio-tui/marks.toml`, else `~/.config/illumio-tui/marks.toml`, or the directory of `--config`. Tests point it at a temp dir (`XDG_CONFIG_HOME` for the CLI, `MarksStore::new(<tempdir>/marks.toml)` in unit tests); no test may read or write the real home. A TUI model has `MarksStore::none()` (no file) until `app::run` gives it the path.
- A broken marks file never crashes the TUI or the CLI: it is a warning and marks are off. An invalid file is never written.
- Marks are not policy: the generator's rules ignore them.
- Tests use fixtures only (`fixtures/`, `fixtures/recorded/`) and wiremock; never contact a PCE, never read `.env` files. The one live smoke (Plan 1, last task) is read-only and run by the integrator.
- Release builds include Windows: Unix permission code (0600, 0700) sits under `#[cfg(unix)]`.
- Edition 2024: clippy wants let-chains for nested `if let`; take its suggestion and list it as a deviation in the task report.
- Every commit message ends with these two lines:
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx
- Plan 1 is merged: `pce::marks`, `illumio_tui::marks` (with `MarksStore::none()`), the CLI.
- TUI tests build their model with `test_support::model_with_marks(<tempdir>, …)`; `model_fixture` and every model before `app::run` hold `MarksStore::none()`, so no existing test reads a marks file.
- Snapshots are plain insta files: the plan gives each new or changed file whole (or the line that changes); `INSTA_UPDATE` is never needed.

## Spec deviations and resolutions (read before Task 1)

- **Squelch is a layer under `view_flows()`**: focus first (`focus_flows()`), then marks; every view, its counts and `has_bytes` read the unsquelched flows, so the visible rows add up to the visible flows. The app overlay and the generator read `flows_for` and apply marks themselves. The Result panel's `N flows` stays the loaded count; its decision counts are those of the visible flows.
- **The store lives in `TrafficState`** as the spec says, `MarksStore::none()` until `app::run` sets the path; `reset_data` (`:profile`) carries the store and `M` over and scopes the marks to the new profile's PCE. The marks are judged at `marks_now` (set at each refresh), so a rebuild of one view (`d`, `o`, a fold) uses the same instant as the others.
- **Notices**: a change into an invalid file gives one warn status (`marks.toml invalid (2 problems): marks off · :marks shows them`, on a valid-again file an info); the first load of a session announces marks expiring within 3 days (`1 mark expires within 3 days · :marks`). `update()` shows a pending notice after the message that rebuilt the views, once.
- **The gutter is drawn only while a mark is active** (`TableView::marker` set by `refresh_marks`), so nothing changes without marks. In plain tables it sits after ratatui's `▸ ` highlight symbol (1 cell plus the column spacing); in sectioned tables after the cursor's two cells. A highlight colours the first column (the group or peer cell); quiet rows are dimmed whole.
- **`~N`** is plain text in the flows cell (`1 ~1`): a cell carries no styled span. Its sort key stays the shown count and `:export` writes the count alone (`Column::full`). The Mesh view has no flows column: its partly squelched rows show the `~` gutter only.
- **With marks active**, the squelch text replaces the `keys …` and `Enter drills` hints in App-to-app's and Blocked's panel meta (Mesh keeps `keys …`), so the title keeps its room at 120 columns.
- **The Flows detail** gets one more line while marks are active (`layout_with(body, 1)`, taken from the table; the treemap's area is the two together, unchanged): `mark       critical m-crit · payments db · never expires · also matches: m-sq1 (squelch)`.
- **Blocked's accepted**: the accept-blocked subset is aggregated with Blocked's own aggregator and taken out of each header's blocked and potentially blocked counts; the header ends `· 2 accepted`; the view meta reads `10 blocked flows · 2 accepted` (still open records, accepted records).
- **Treemap**: highlights only (critical, important, revisit) get a glyph; a source tile shows the strongest of its pairs'. Computed at relayout (`TreemapState::highlights`), never per draw.
- **App overlay**: `M` works there too; a peer panel's title reads `Inbound · 2 peers (5 flows) squelched` (`, shown ~` with `M`). **Generator**: its rows and proposal are those without marks (tested equal); the header title ends ` · 5 squelched (rules include them)`.
- **Help**: this plan adds the `M` line only; `m` and `:marks` come with Plan 3.

## Review Focus

- **A flow both critical and squelched** must surface in every view (gutter `●`, the name cell red) and never count as squelched. Pinned in Task 1 (`squelched_flows_leave_every_view_and_each_view_counts_them`: Corp → shop/prod keeps its critical flow) and Task 2 (`the_panels_say_what_is_squelched_and_the_detail_names_the_mark`).
- **The marks file edited in another editor while the TUI runs**: the next rebuild picks it up, an invalid save turns marks off with one status line (not one per key) and `marks off: marks.toml invalid` in the panel. Pinned in Task 1 (`an_invalid_file_turns_marks_off_with_one_status_line`) and Task 2 (`an_invalid_file_says_marks_off_in_the_panel`).
- **`:profile` to another PCE**: scoped marks of the old PCE go inactive; the file, `M` and the expiry announcement are not lost or repeated. Pinned in Task 1 (`a_profile_switch_keeps_the_file_and_m_and_scopes_to_the_new_pce`).
- **Marks absent** (no file, as in every existing test): no gutter, no meta text, every existing snapshot unchanged. Pinned in Task 2 (`partly_squelched_rows_show_the_hidden_records_beside_the_count`: `model_with_rich_flows` has no marker) and by the unchanged snapshot suite.
- **`M` typed into a table search**: the search takes it. Pinned in Task 1 (`a_search_being_typed_takes_m`).

---

### Task 1: `TrafficState` applies marks: refresh, partition, counts, `M`, notices

**Files:**
- Modify: `crates/illumio-tui/src/tui/app.rs`
- Modify: `crates/illumio-tui/src/tui/mod.rs`
- Modify: `crates/illumio-tui/src/tui/model.rs`
- Modify: `crates/illumio-tui/src/tui/test_support.rs`
- Create: `crates/illumio-tui/src/tui/traffic_marks.rs`
- Modify: `crates/illumio-tui/src/tui/update.rs`
- Test: `crates/illumio-tui/src/tui/update/marks_tests.rs`

**Interfaces:**
- Consumes: Plan 1 (`MarksStore::{new, none, refresh, set, error}`, `marks_path`, `partition`, `annotate_groups`, `annotate_mesh`, `annotate_sections`, `Marked`, `RowMarks`, `Hit`, `PceId`).
- Produces (`illumio_tui::tui::traffic_marks`, `model.rs`):

```rust
pub struct Squelch { pub rows: u64, pub flows: u64 }   // fn meta(self, shown: bool, rows: bool) -> Option<String>
pub struct MarkedView { pub marked: Marked, pub hits: Vec<Option<Hit>>, pub active: usize, pub waiting: usize,
                        pub flows, pairs, services, sections, blocked, mesh, treemap: Squelch, pub blocked_open: u64, pub accepted: u64 }
pub fn sum_marks<'a>(rows: impl IntoIterator<Item = (u64, Option<&'a RowMarks>)>) -> Option<RowMarks>;
impl TrafficState {
    pub fn active_marks(&self) -> Option<ActiveMarks<'_>>;
    pub fn marks_ctx(&self) -> Option<(ActiveMarks<'_>, &MarkedView)>;
    pub(crate) fn refresh_marks(&mut self);            // called first thing in rebuild_views
    pub fn take_marks_notice(&mut self) -> Option<StatusLine>;   // update() shows it once
    pub fn toggle_squelched(&mut self) -> StatusLine;  // `M`
    pub fn marks_meta(&self, view: TrafficView) -> Option<String>;
    pub fn focus_flows(&self) -> &[ResolvedFlow];      // view_flows() before squelch
}
// TrafficState fields: marks: MarksStore, marks_pce: PceId, marks_now: Timestamp, show_squelched: bool, marked: Option<MarkedView>
// FlowRow.mark: Option<Hit>; RowKind::Header { .., marks: Option<Box<RowMarks>>, accepted: u64 }
// test_support: pub const RICH_MARKS: &str; pub fn model_with_marks(dir: &Path, marks: &str) -> Model
```

- [ ] **Step 1: Write the failing tests**

In `crates/illumio-tui/src/tui/mod.rs`, replace:

```rust
pub mod table;
pub mod test_support;
pub mod ui;
pub mod update;
```

with:

```rust
pub mod table;
pub mod test_support;
pub mod traffic_marks;
pub mod ui;
pub mod update;
```

Create `crates/illumio-tui/src/tui/traffic_marks.rs` with its tests only (Step 3 adds the code above them):

```rust
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::tui::model::{By, Model, RowKind};
    use crate::tui::test_support::model_with_marks;
    use pce::marks::Marker;

    pub(crate) use crate::tui::test_support::RICH_MARKS;

    /// The rich model with `marks` in a temp `marks.toml`, rebuilt.
    pub(crate) fn marked_model(marks: &str) -> (Model, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let m = model_with_marks(dir.path(), marks);
        (m, dir)
    }

    #[test]
    fn squelched_flows_leave_every_view_and_each_view_counts_them() {
        let (m, _dir) = marked_model(RICH_MARKS);
        let t = &m.traffic;
        let mv = t.marked.as_ref().unwrap();
        assert_eq!((mv.active, mv.waiting), (6, 0));
        assert_eq!(t.flows.rows().len(), 21, "30 flows, 9 squelched");
        assert_eq!(mv.flows, Squelch { rows: 9, flows: 9 });
        assert_eq!(mv.pairs, Squelch { rows: 4, flows: 9 });
        assert_eq!(mv.treemap.flows, 9);
        assert_eq!(mv.blocked, Squelch { rows: 2, flows: 2 });
        assert_eq!((mv.blocked_open, mv.accepted), (10, 2));
        let shown: u64 = t.groups.rows().iter().map(|g| g.flows).sum();
        assert_eq!(shown + mv.pairs.flows, 30, "visible + squelched = input");
        assert!(
            t.groups
                .rows()
                .iter()
                .all(|g| g.src.join("/") != "ipl:Corp/-" || g.dst.join("/") == "shop/prod"),
            "Corp is left only where its critical flow is"
        );
        let corp = t
            .groups
            .rows()
            .iter()
            .find(|g| g.src.join("/") == "ipl:Corp/-")
            .unwrap();
        assert_eq!(
            corp.marks.as_ref().and_then(RowMarks::marker),
            Some(Marker::Critical)
        );
        assert_eq!(corp.marks.as_ref().unwrap().hidden, 1);
        let hit = t
            .flows
            .rows()
            .iter()
            .find(|r| r.src.label_value("app") == Some("hr"))
            .and_then(|r| r.mark)
            .unwrap();
        assert_eq!(hit.action, Action::Revisit);
        assert_eq!(
            t.marks_meta(TrafficView::Aggregate).as_deref(),
            Some("4 rows (9 flows) squelched · M shows them")
        );
        assert_eq!(
            t.marks_meta(TrafficView::Blocked).as_deref(),
            Some("10 blocked flows · 2 accepted · 2 rows (2 flows) squelched · M shows them")
        );
        assert_eq!(
            t.marks_meta(TrafficView::Treemap).as_deref(),
            Some("9 flows squelched · M shows them")
        );
    }

    #[test]
    fn blocked_headers_take_accepted_records_out_and_say_how_many() {
        let (m, _dir) = marked_model(RICH_MARKS);
        let erp = m
            .traffic
            .blocked
            .rows()
            .iter()
            .find(|r| r.is_header() && r.app.join("/") == "erp/prod")
            .unwrap();
        let line = erp.header_line().unwrap();
        assert!(line.ends_with("· 2 pot.blocked · 2 accepted"), "{line}");
        let RowKind::Header { marks, .. } = &erp.kind else {
            unreachable!()
        };
        assert_eq!(marks.as_ref().unwrap().count(Action::AcceptBlocked), 2);
    }

    #[test]
    fn m_shows_squelched_rows_and_the_counts_stay() {
        let (mut m, _dir) = marked_model(RICH_MARKS);
        let s = m.traffic.toggle_squelched();
        assert_eq!(s.text, "squelched rows shown (~) · M hides them");
        let t = &m.traffic;
        assert_eq!(t.flows.rows().len(), 30);
        let mv = t.marked.as_ref().unwrap();
        assert_eq!(mv.pairs, Squelch { rows: 4, flows: 9 });
        let corp = t
            .flows
            .rows()
            .iter()
            .find(|r| r.src.ip_lists.first().map(String::as_str) == Some("Corp"))
            .unwrap();
        assert_eq!(corp.mark.map(|h| h.action), Some(Action::Squelch));
        assert_eq!(
            t.marks_meta(TrafficView::Flows).as_deref(),
            Some("9 rows (9 flows) squelched, shown ~ · M hides them")
        );
        let shop = t
            .groups
            .rows()
            .iter()
            .find(|g| g.src.join("/") == "shop/prod" && g.dst.join("/") == "shop/prod")
            .unwrap();
        assert_eq!(
            shop.marks.as_ref().and_then(RowMarks::marker),
            Some(Marker::Squelched)
        );
        m.traffic.toggle_squelched();
        assert_eq!(m.traffic.flows.rows().len(), 21);
    }

    #[test]
    fn sections_count_their_own_squelched_rows() {
        let (mut m, _dir) = marked_model(RICH_MARKS);
        m.traffic.apps_state.by = By::In;
        m.traffic.rebuild_sections();
        let mv = m.traffic.marked.as_ref().unwrap();
        assert_eq!(mv.sections.flows, 9);
        let peers: u64 = m
            .traffic
            .apps
            .rows()
            .iter()
            .filter_map(|r| r.peer())
            .map(|(_, p)| p.flows)
            .sum();
        assert_eq!(peers + 9, 30);
        assert!(mv.sections.rows > 0);
    }

    #[test]
    fn an_invalid_file_turns_marks_off_with_one_status_line() {
        let (mut m, _dir) = marked_model("[[mark]]\naction = \"squelch\"\n");
        assert!(m.traffic.marked.is_none());
        assert_eq!(m.traffic.flows.rows().len(), 30, "nothing left out");
        assert_eq!(
            m.traffic.marks_meta(TrafficView::Flows).as_deref(),
            Some("marks off: marks.toml invalid")
        );
        let s = m.traffic.take_marks_notice().unwrap();
        assert_eq!(
            s.text,
            "marks.toml invalid (2 problems): marks off · :marks shows them"
        );
        assert!(m.traffic.take_marks_notice().is_none(), "once");
        m.traffic.rebuild_views();
        assert!(
            m.traffic.take_marks_notice().is_none(),
            "unchanged file: no reparse"
        );
    }

    #[test]
    fn marks_expiring_within_three_days_are_announced_once() {
        let soon = (jiff::Timestamp::now() + jiff::SignedDuration::from_hours(48)).to_string();
        let text = format!(
            "[[mark]]\naction = \"squelch\"\nsrc = \"any\"\ndst = \"any\"\ncomment = \"x\"\nexpires = \"{soon}\"\n"
        );
        let (mut m, _dir) = marked_model(&text);
        assert_eq!(
            m.traffic.take_marks_notice().map(|s| s.text).as_deref(),
            Some("1 mark expires within 3 days · :marks")
        );
        m.traffic.rebuild_views();
        assert!(m.traffic.take_marks_notice().is_none());
    }

    #[test]
    fn draft_decision_marks_wait_for_draft_decisions() {
        let (m, _dir) = marked_model(
            "[[mark]]\naction = \"revisit\"\nsrc = \"any\"\ndst = \"any\"\ndecision = \"blocked\"\ndraft_decision = \"allowed\"\ncomment = \"fixed by the pending policy\"\n",
        );
        let mv = m.traffic.marked.as_ref().unwrap();
        assert_eq!((mv.active, mv.waiting), (0, 1));
        assert_eq!(
            m.traffic.marks_meta(TrafficView::Flows).as_deref(),
            Some("1 mark wait for the draft decision (D)")
        );
    }

    #[test]
    fn a_profile_switch_keeps_the_file_and_m_and_scopes_to_the_new_pce() {
        let (mut m, _dir) = marked_model(RICH_MARKS);
        m.traffic.toggle_squelched();
        m.profile.host = "other.example".into();
        m.reset_data();
        assert!(m.traffic.show_squelched);
        assert_eq!(m.traffic.marks_pce.host, "other.example");
        assert!(m.traffic.marks.set().is_some(), "the same file");
    }
}
```

In `crates/illumio-tui/src/tui/update.rs`, replace:

```rust

mod generator;

/// The status and header note for a result a supercluster region did not
```

with:

```rust

mod generator;
#[cfg(test)]
mod marks_tests;

/// The status and header note for a result a supercluster region did not
```

Create `crates/illumio-tui/src/tui/update/marks_tests.rs`:

```rust
//! The marks' keys on the Traffic screen: `M` toggles squelched rows with
//! a rebuild and never a request; a search being typed takes the key.
use crate::tui::model::TrafficView;
use crate::tui::msg::{Cmd, Msg};
use crate::tui::traffic_marks::tests::{RICH_MARKS, marked_model};
use crate::tui::update;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn key(c: char) -> Msg {
    Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

#[test]
fn m_toggles_squelched_rows_in_every_view_without_a_request() {
    let (mut m, _dir) = marked_model(RICH_MARKS);
    for view in [
        TrafficView::Flows,
        TrafficView::Aggregate,
        TrafficView::Mesh,
        TrafficView::Blocked,
        TrafficView::Treemap,
    ] {
        m.traffic.view = view;
        let before = m.traffic.show_squelched;
        assert_eq!(update(&mut m, key('M')), Vec::<Cmd>::new(), "{view:?}");
        assert_eq!(m.traffic.show_squelched, !before);
    }
    assert!(m.status.text.contains("M "), "{}", m.status.text);
}

#[test]
fn a_search_being_typed_takes_m() {
    let (mut m, _dir) = marked_model(RICH_MARKS);
    assert!(update(&mut m, key('/')).is_empty());
    assert!(update(&mut m, key('M')).is_empty());
    assert!(!m.traffic.show_squelched);
    assert_eq!(m.traffic.flows.search_text(), "M");
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p illumio-tui --lib -- traffic_marks marks_tests`
Expected: FAIL: it does not compile (`no field marks on TrafficState`, `cannot find function model_with_marks`).

- [ ] **Step 3: Implement**

In `crates/illumio-tui/src/tui/app.rs`, replace:

```rust
        writes,
    );
    model.set_label_model(
        cfg.scope_keys.clone(),
```

with:

```rust
        writes,
    );
    // marks.toml next to the config file: read at every rebuild (a stat).
    model.traffic.marks =
        crate::marks::MarksStore::new(crate::marks::marks_path(global.config.as_deref()));
    model.set_label_model(
        cfg.scope_keys.clone(),
```

In `crates/illumio-tui/src/tui/model.rs`, replace:

```rust
//! All TUI state. No I/O, no terminal types beyond sizes.
use crate::cli::output::join_list;
use crate::tui::app_picker::AppPicker;
use crate::tui::app_view::AppView;
```

with:

```rust
//! All TUI state. No I/O, no terminal types beyond sizes.
use crate::cli::output::join_list;
use crate::marks::MarksStore;
use crate::tui::app_picker::AppPicker;
use crate::tui::app_view::AppView;
```

In `crates/illumio-tui/src/tui/model.rs`, replace:

```rust
use crate::tui::picker::KeyPicker;
use crate::tui::table::{Column, SortKey, TableView};
use crate::tui::ui::theme::Theme;
use pce::impact::RuleActors;
use pce::models::{
    Actor, DenyRule, Event, EventFilter, IpList, Label, LabelGroup, PendingChange, PolicyScope,
```

with:

```rust
use crate::tui::picker::KeyPicker;
use crate::tui::table::{Column, SortKey, TableView};
use crate::tui::traffic_marks::{MarkedView, Squelch, sum_marks};
use crate::tui::ui::theme::Theme;
use pce::impact::RuleActors;
use pce::marks::{Hit, PceId, RowMarks, annotate_groups, annotate_mesh, annotate_sections};
use pce::models::{
    Actor, DenyRule, Event, EventFilter, IpList, Label, LabelGroup, PendingChange, PolicyScope,
```

In `crates/illumio-tui/src/tui/model.rs`, replace:

```rust
    IpListSize, KeyOrder, NameIndex, RuleKind, RuleSetView, RuleView, StatsSnapshot, ip_list_size,
};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
```

with:

```rust
    IpListSize, KeyOrder, NameIndex, RuleKind, RuleSetView, RuleView, StatsSnapshot, ip_list_size,
};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
```

In `crates/illumio-tui/src/tui/model.rs`, replace:

```rust
    pub src_group: String,
    pub dst_group: String,
}

```

with:

```rust
    pub src_group: String,
    pub dst_group: String,
    /// The flow's winning mark, while marks are active.
    pub mark: Option<Hit>,
}

```

In `crates/illumio-tui/src/tui/model.rs`, replace:

```rust
            dst_group: group_text(&flow.dst, keys),
            flow,
        }
    }
```

with:

```rust
            dst_group: group_text(&flow.dst, keys),
            flow,
            mark: None,
        }
    }
```

In `crates/illumio-tui/src/tui/model.rs`, replace:

```rust
        totals: Vec<(Direction, Totals)>,
        folded: bool,
    },
    Peer {
```

with:

```rust
        totals: Vec<(Direction, Totals)>,
        folded: bool,
        /// The section's rows' marks summed: any highlight, dimmed when all
        /// are quiet. Boxed: a header is otherwise small.
        marks: Option<Box<RowMarks>>,
        /// Blocked: records accepted as blocked (left out of `totals`).
        accepted: u64,
    },
    Peer {
```

In `crates/illumio-tui/src/tui/model.rs`, replace:

```rust
    /// `None` for a peer.
    pub fn header_line(&self) -> Option<String> {
        let RowKind::Header { totals, folded } = &self.kind else {
            return None;
        };
        let parts: Vec<(Direction, &Totals)> = totals.iter().map(|(d, t)| (*d, t)).collect();
        Some(format!(
            "{} {}  {}",
            if *folded { "▹" } else { "▾" },
            self.app.join("/"),
```

with:

```rust
    /// `None` for a peer.
    pub fn header_line(&self) -> Option<String> {
        let RowKind::Header {
            totals,
            folded,
            accepted,
            ..
        } = &self.kind
        else {
            return None;
        };
        let parts: Vec<(Direction, &Totals)> = totals.iter().map(|(d, t)| (*d, t)).collect();
        let accepted = if *accepted > 0 {
            format!(" · {accepted} accepted")
        } else {
            String::new()
        };
        Some(format!(
            "{} {}  {}{accepted}",
            if *folded { "▹" } else { "▾" },
            self.app.join("/"),
```

In `crates/illumio-tui/src/tui/model.rs`, replace:

```rust
                    .collect(),
                folded: is_folded,
            },
            haystack: Arc::from(""),
```

with:

```rust
                    .collect(),
                folded: is_folded,
                marks: sum_marks(
                    group
                        .iter()
                        .flat_map(|s| &s.rows)
                        .map(|r| (r.flows, r.marks.as_ref())),
                )
                .map(Box::new),
                accepted: 0,
            },
            haystack: Arc::from(""),
```

In `crates/illumio-tui/src/tui/model.rs`, replace:

```rust
    /// or the data changes (`refresh_focused`); `None` without a focus.
    focused: Option<Vec<ResolvedFlow>>,
}

```

with:

```rust
    /// or the data changes (`refresh_focused`); `None` without a focus.
    focused: Option<Vec<ResolvedFlow>>,
    /// `marks.toml` (no file until `app::run` gives the path): refreshed
    /// at every `rebuild_views`, one `stat`, never a request.
    pub marks: MarksStore,
    /// The PCE the marks are scoped to: the profile's.
    pub marks_pce: PceId,
    /// When the marks were last judged (expiry).
    pub marks_now: jiff::Timestamp,
    /// `M`: squelched flows shown (dimmed `~`) instead of left out.
    pub show_squelched: bool,
    /// What the marks did to the result on screen; `None` without a valid
    /// file holding an active (or draft-waiting) mark.
    pub marked: Option<MarkedView>,
    /// The flows the views aggregate while squelched ones are left out.
    pub(crate) unsquelched: Option<Vec<ResolvedFlow>>,
    /// A status the marks file asks for; `update` shows it once.
    pub(crate) marks_notice: Option<StatusLine>,
    /// The startup line on marks expiring soon was considered.
    pub(crate) marks_announced: bool,
}

```

In `crates/illumio-tui/src/tui/model.rs`, replace:

```rust
            unfocused: None,
            focused: None,
        }
    }
```

with:

```rust
            unfocused: None,
            focused: None,
            marks: MarksStore::none(),
            marks_pce: PceId {
                host: String::new(),
                org: 0,
            },
            marks_now: jiff::Timestamp::now(),
            show_squelched: false,
            marked: None,
            unsquelched: None,
            marks_notice: None,
            marks_announced: false,
        }
    }
```

In `crates/illumio-tui/src/tui/model.rs`, replace:

```rust

    /// The loaded flows the focus keeps (all of them, borrowed, without
    /// one): what every view aggregates, under the drill and `d`.
    pub fn view_flows(&self) -> &[ResolvedFlow] {
        match &self.focused {
            Some(f) => f,
```

with:

```rust

    /// The loaded flows the focus keeps (all of them, borrowed, without
    /// one), squelched ones left out unless `M`: what every view
    /// aggregates, under the drill and `d`.
    pub fn view_flows(&self) -> &[ResolvedFlow] {
        match &self.unsquelched {
            Some(f) => f,
            None => self.focus_flows(),
        }
    }

    /// The loaded flows the focus keeps, marks not applied: what the
    /// marks are matched against.
    pub fn focus_flows(&self) -> &[ResolvedFlow] {
        match &self.focused {
            Some(f) => f,
```

In `crates/illumio-tui/src/tui/model.rs`, replace:

```rust
    }

    /// Recomputes every view from `data` with the keys in force. Pure;
    /// never requests.
    pub fn rebuild_views(&mut self) {
        self.sync_draft_columns();
        self.keys = self.resolve_keys();
        self.refresh_focused();
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
                group_by_labels(flows, keys),
                mesh(flows, keys, self.mesh_cols),
            )
        };
        self.flows.set_rows(shown);
        self.treemap.groups = tree;
```

with:

```rust
    }

    /// Recomputes every view from `data` with the keys in force. Pure
    /// apart from one `stat` of `marks.toml` (a local file, re-read only
    /// when it changed); never requests.
    pub fn rebuild_views(&mut self) {
        self.sync_draft_columns();
        self.keys = self.resolve_keys();
        self.refresh_focused();
        self.refresh_marks();
        let (shown, tree, mesh_rows, counts) = {
            let keys = &self.keys;
            let flows = self.view_flows();
            let hits = self.marked.as_ref().map(|m| m.hits.as_slice());
            let drilled = |f: &ResolvedFlow| self.drill.as_ref().is_none_or(|d| d.matches(f, keys));
            let shown: Vec<FlowRow> = flows
                .iter()
                .enumerate()
                .filter(|(_, f)| drilled(f))
                .map(|(i, f)| FlowRow {
                    mark: hits.and_then(|h| h.get(i).copied().flatten()),
                    ..FlowRow::new(f.clone(), keys)
                })
                .collect();
            let mut tree = group_by_labels(flows, keys);
            let cols = self.mesh_cols;
            let mut mesh_rows = mesh(flows, keys, cols);
            let counts = self.marks_ctx().map(|(a, mv)| {
                annotate_groups(&mut tree, &mv.marked, &a, |f| group_by_labels(f, keys));
                let gone = annotate_mesh(&mut mesh_rows, &mv.marked, &a, |f| mesh(f, keys, cols));
                let flows = mv.marked.squelched_where(drilled);
                (
                    Squelch { rows: flows, flows },
                    self.squelch(gone, |_| true),
                    self.squelch(0, |_| true),
                )
            });
            (shown, tree, mesh_rows, counts)
        };
        if let (Some(mv), Some((flows, mesh, treemap))) = (self.marked.as_mut(), counts) {
            mv.flows = flows;
            mv.mesh = mesh;
            mv.treemap = treemap;
        }
        self.flows.set_rows(shown);
        self.treemap.groups = tree;
```

In `crates/illumio-tui/src/tui/model.rs`, replace:

```rust
    /// from the flows the focus and the decision filter keep. Pure.
    pub fn rebuild_pairs(&mut self) {
        let (groups, services) = {
            let st = &self.apps_state;
            let flows = self.view_flows();
            let kept = st.decisions.kept(flows);
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
```

with:

```rust
    /// from the flows the focus and the decision filter keep. Pure.
    pub fn rebuild_pairs(&mut self) {
        let (groups, services, counts) = {
            let st = &self.apps_state;
            let keys = &self.keys;
            let flows = self.view_flows();
            let kept = st.decisions.kept(flows);
            let mut groups = group_by_labels(&kept, keys);
            let mut services = if st.by_service {
                group_by_service(&kept, keys)
            } else {
                Vec::new()
            };
            let d = st.decisions;
            let counts = self.marks_ctx().map(|(a, mv)| {
                let pairs = annotate_groups(&mut groups, &mv.marked, &a, |f| {
                    group_by_labels(&d.kept(f), keys)
                });
                let by_service = annotate_groups(&mut services, &mv.marked, &a, |f| {
                    group_by_service(&d.kept(f), keys)
                });
                (
                    self.squelch(pairs, |f| d.keeps(f)),
                    self.squelch(by_service, |f| d.keeps(f)),
                )
            });
            (groups, services, counts)
        };
        if let (Some(mv), Some((pairs, services))) = (self.marked.as_mut(), counts) {
            mv.pairs = pairs;
            mv.services = services;
        }
        self.groups.set_rows(groups);
        self.services.set_rows(services);
```

In `crates/illumio-tui/src/tui/model.rs`, replace:

```rust
    /// loaded flows, the keys in force, the state and the folds. Pure.
    pub fn rebuild_sections(&mut self) {
        let apps = self.apps_rows(&self.apps_state.folded);
        let blocked = self.blocked_rows(&self.blocked_folded);
        self.apps.set_rows(apps);
        self.blocked.set_rows(blocked);
    }

    /// App-to-app's rows under `folded`; none in `pairs`.
    fn apps_rows(&self, folded: &BTreeSet<Vec<String>>) -> Vec<SectionRow> {
        let keys = &self.keys;
        let flows = self.view_flows();
        let st = &self.apps_state;
        match st.by {
            By::Pairs => Vec::new(),
            By::In => section_rows(
                &app_sections(flows, keys, Direction::In, st.decisions),
                folded,
                false,
            ),
            By::Out => section_rows(
                &app_sections(flows, keys, Direction::Out, st.decisions),
                folded,
                false,
            ),
            By::Both => section_rows(&app_sections_both(flows, keys, st.decisions), folded, true),
        }
    }

    /// Blocked's rows under `folded`.
    fn blocked_rows(&self, folded: &BTreeSet<Vec<String>>) -> Vec<SectionRow> {
        section_rows(
            &app_sections(
                self.view_flows(),
                &self.keys,
                Direction::In,
                Decisions::Blocked,
            ),
            folded,
            false,
        )
    }

```

with:

```rust
    /// loaded flows, the keys in force, the state and the folds. Pure.
    pub fn rebuild_sections(&mut self) {
        let (apps, apps_sq) = self.apps_rows(&self.apps_state.folded);
        let (blocked, blocked_sq, open, accepted) = self.blocked_rows(&self.blocked_folded);
        if let Some(mv) = self.marked.as_mut() {
            mv.sections = apps_sq;
            mv.blocked = blocked_sq;
            mv.blocked_open = open;
            mv.accepted = accepted;
        }
        self.apps.set_rows(apps);
        self.blocked.set_rows(blocked);
    }

    /// App-to-app's sections in force (none in `pairs`), from `flows`.
    fn apps_sections(&self, flows: &[ResolvedFlow]) -> Vec<AppSection> {
        let keys = &self.keys;
        let st = &self.apps_state;
        match st.by {
            By::Pairs => Vec::new(),
            By::In => app_sections(flows, keys, Direction::In, st.decisions),
            By::Out => app_sections(flows, keys, Direction::Out, st.decisions),
            By::Both => app_sections_both(flows, keys, st.decisions),
        }
    }

    /// App-to-app's rows under `folded` (none in `pairs`), the marks
    /// joined to them, and its squelched counts.
    fn apps_rows(&self, folded: &BTreeSet<Vec<String>>) -> (Vec<SectionRow>, Squelch) {
        let mut sections = self.apps_sections(self.view_flows());
        let d = self.apps_state.decisions;
        let sq = self
            .marks_ctx()
            .map(|(a, mv)| {
                let gone =
                    annotate_sections(&mut sections, &mv.marked, &a, |f| self.apps_sections(f));
                self.squelch(gone, |f| d.keeps(f))
            })
            .unwrap_or_default();
        let both = self.apps_state.by == By::Both;
        (section_rows(&sections, folded, both), sq)
    }

    /// Blocked's rows under `folded`, the marks joined to them, its
    /// squelched counts, and its records still blocked and accepted: the
    /// accepted ones are taken out of each header's blocked counts.
    fn blocked_rows(&self, folded: &BTreeSet<Vec<String>>) -> (Vec<SectionRow>, Squelch, u64, u64) {
        let keys = &self.keys;
        let blocked = |f: &[ResolvedFlow]| app_sections(f, keys, Direction::In, Decisions::Blocked);
        let mut sections = blocked(self.view_flows());
        let mut accepted: BTreeMap<Vec<String>, u64> = BTreeMap::new();
        let mut sq = Squelch::default();
        if let Some((a, mv)) = self.marks_ctx() {
            let gone = annotate_sections(&mut sections, &mv.marked, &a, blocked);
            sq = self.squelch(gone, |f| Decisions::Blocked.keeps(f));
            for acc in blocked(&self.accepted_flows()) {
                if let Some(s) = sections.iter_mut().find(|s| s.app == acc.app) {
                    s.totals.blocked -= acc.totals.blocked;
                    s.totals.potentially_blocked -= acc.totals.potentially_blocked;
                    accepted.insert(acc.app, acc.totals.flows);
                }
            }
        }
        let open = sections
            .iter()
            .map(|s| s.totals.blocked + s.totals.potentially_blocked)
            .sum();
        let mut rows = section_rows(&sections, folded, false);
        for r in &mut rows {
            if let RowKind::Header { accepted: n, .. } = &mut r.kind {
                *n = accepted.get(&r.app).copied().unwrap_or(0);
            }
        }
        (rows, sq, open, accepted.values().sum())
    }

```

In `crates/illumio-tui/src/tui/model.rs`, replace:

```rust
        let (table, rows) = match self.view {
            TrafficView::Aggregate if self.apps_state.by != By::Pairs => {
                (&self.apps, self.apps_rows(&none))
            }
            TrafficView::Blocked => (&self.blocked, self.blocked_rows(&none)),
            _ => return None,
        };
```

with:

```rust
        let (table, rows) = match self.view {
            TrafficView::Aggregate if self.apps_state.by != By::Pairs => {
                (&self.apps, self.apps_rows(&none).0)
            }
            TrafficView::Blocked => (&self.blocked, self.blocked_rows(&none).0),
            _ => return None,
        };
```

In `crates/illumio-tui/src/tui/model.rs`, replace:

```rust
        traffic.treemap.area = crate::tui::ui::screens::traffic::treemap_area(size);
        traffic.set_key_order(&KeyOrder::new(&scope_keys, crate::config::DEFAULT_TIER_KEY));
        let policy = PolicyState::new(auto_refresh_min_seconds);
        let objects = ObjectsState::new(auto_refresh_min_seconds);
```

with:

```rust
        traffic.treemap.area = crate::tui::ui::screens::traffic::treemap_area(size);
        traffic.set_key_order(&KeyOrder::new(&scope_keys, crate::config::DEFAULT_TIER_KEY));
        traffic.marks_pce = PceId {
            host: profile.host.clone(),
            org: profile.org_id,
        };
        let policy = PolicyState::new(auto_refresh_min_seconds);
        let objects = ObjectsState::new(auto_refresh_min_seconds);
```

In `crates/illumio-tui/src/tui/model.rs`, replace:

```rust
        traffic.seq = self.traffic.seq + 1; // a late result of the old query is stale
        traffic.treemap.area = crate::tui::ui::screens::traffic::treemap_area(self.size);
        self.traffic = traffic;
        self.policy = PolicyState::new(min);
```

with:

```rust
        traffic.seq = self.traffic.seq + 1; // a late result of the old query is stale
        traffic.treemap.area = crate::tui::ui::screens::traffic::treemap_area(self.size);
        // The marks file and `M` are the session's; their PCE the new profile's.
        traffic.marks = std::mem::replace(&mut self.traffic.marks, MarksStore::none());
        traffic.marks_announced = true;
        traffic.show_squelched = self.traffic.show_squelched;
        traffic.marks_pce = PceId {
            host: self.profile.host.clone(),
            org: self.profile.org_id,
        };
        self.traffic = traffic;
        self.policy = PolicyState::new(min);
```

In `crates/illumio-tui/src/tui/test_support.rs`, replace:

```rust
        note: None,
    }
}
```

with:

```rust
        note: None,
    }
}

/// The rich fixture's marks: two squelches (the Corp list; shop's eu
/// web tier to its databases on 5432), a critical on blocked flows into
/// shop/prod's databases (it takes two flows from the squelches),
/// erp/prod reviewed inside itself, erp/dev accepted as blocked, hr to
/// revisit.
pub const RICH_MARKS: &str = r#"
[[mark]]
id = "m-sq1"
action = "squelch"
src = { ip_list = "Corp" }
dst = "any"
comment = "scanner"

[[mark]]
id = "m-sq2"
action = "squelch"
src = { labels = { role = "web", loc = "eu" } }
dst = { labels = { role = "db" } }
service = "5432/tcp"
comment = "app tier to its db"

[[mark]]
id = "m-crit"
action = "critical"
src = "any"
dst = { labels = { app = "shop", env = "prod", role = "db" } }
decision = "blocked"
comment = "payments db"

[[mark]]
id = "m-rev"
action = "reviewed"
src = { labels = { app = "erp", env = "prod" } }
dst = { labels = { app = "erp", env = "prod" } }
comment = "erp inside itself"

[[mark]]
id = "m-acc"
action = "accept-blocked"
src = { labels = { app = "erp", env = "dev" } }
dst = "any"
decision = ["blocked", "potentially_blocked"]
comment = "dev stays out (CHG-1)"

[[mark]]
id = "m-hr"
action = "revisit"
src = { labels = { app = "hr" } }
dst = "any"
comment = "ask hr"
"#;

/// The rich fixture loaded with `marks` written to `dir/marks.toml` (a
/// temp dir the test owns; never the real config), the views rebuilt.
pub fn model_with_marks(dir: &std::path::Path, marks: &str) -> Model {
    let path = dir.join("marks.toml");
    std::fs::write(&path, marks).expect("write marks.toml");
    let mut m = model_with_rich_flows();
    m.traffic.marks = crate::marks::MarksStore::new(path);
    m.traffic.rebuild_views();
    m
}
```

In `crates/illumio-tui/src/tui/traffic_marks.rs`, insert above the tests:

```rust
//! The marks on the Traffic screen: `marks.toml` refreshed at every
//! rebuild (one `stat` of a local file, never a request), the result on
//! screen split by them (squelched flows left out of every view unless
//! `M`), and what each view's meta says about them.
use crate::tui::model::{StatusLine, TrafficState, TrafficView};
use pce::marks::{Action, ActiveMarks, Hit, Marked, RowMarks, partition};
use pce::traffic::ResolvedFlow;
use std::borrow::Cow;

/// A view's `squelched {rows, flows}`: the rows squelch removed (or, with
/// `M`, whose every record is squelched) and the squelched records of
/// the view's input.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Squelch {
    pub rows: u64,
    pub flows: u64,
}

impl Squelch {
    /// `37 rows (412 flows) squelched · M shows them`, or with `M` on
    /// `37 rows (412 flows) squelched, shown ~ · M hides them`; flows only
    /// for the treemap (`412 flows squelched`). `None` without a squelch hit.
    pub fn meta(self, shown: bool, rows: bool) -> Option<String> {
        if self.flows == 0 {
            return None;
        }
        let what = if rows {
            format!("{} rows ({} flows) squelched", self.rows, self.flows)
        } else {
            format!("{} flows squelched", self.flows)
        };
        Some(if shown {
            format!("{what}, shown ~ · M hides them")
        } else {
            format!("{what} · M shows them")
        })
    }
}

/// What the marks did to the result on screen.
#[derive(Debug, Clone, Default)]
pub struct MarkedView {
    /// The marked flows of `focus_flows()`, by winning mark.
    pub marked: Marked,
    /// The hit of each flow of `view_flows()`, in its order.
    pub hits: Vec<Option<Hit>>,
    /// Active marks, and marks waiting for draft decisions (`D`).
    pub active: usize,
    pub waiting: usize,
    pub flows: Squelch,
    pub pairs: Squelch,
    pub services: Squelch,
    pub sections: Squelch,
    pub blocked: Squelch,
    pub mesh: Squelch,
    pub treemap: Squelch,
    /// Blocked: flow records still blocked or potentially blocked, and
    /// those accepted as blocked (not problems).
    pub blocked_open: u64,
    pub accepted: u64,
}

/// A section header's marks: its rows' records per action summed, so a
/// header shows any highlight of its rows and is dimmed only when all are.
pub fn sum_marks<'a>(
    rows: impl IntoIterator<Item = (u64, Option<&'a RowMarks>)>,
) -> Option<RowMarks> {
    let mut out = RowMarks::default();
    let mut any = false;
    for (flows, m) in rows {
        out.flows += flows;
        if let Some(m) = m {
            any = true;
            out.hidden += m.hidden;
            for (i, n) in m.by_action.iter().enumerate() {
                out.by_action[i] += n;
                if out.top[i].is_none() {
                    out.top[i].clone_from(&m.top[i]);
                }
            }
        }
    }
    any.then_some(out)
}

impl TrafficState {
    /// The marks in force for the result on screen: valid file, in scope
    /// for the profile's PCE, unexpired at the last refresh, `draft_decision`
    /// marks only with draft decisions loaded. `None` without a valid file.
    pub fn active_marks(&self) -> Option<ActiveMarks<'_>> {
        self.marks
            .set()
            .map(|s| s.active(Some(&self.marks_pce), self.marks_now, self.draft_loaded()))
    }

    /// The active marks and what they did, while some are active.
    pub fn marks_ctx(&self) -> Option<(ActiveMarks<'_>, &MarkedView)> {
        let mv = self.marked.as_ref()?;
        let a = self.active_marks().filter(|a| !a.is_empty())?;
        Some((a, mv))
    }

    /// One `stat` of `marks.toml` (parsed again only when it changed),
    /// then the result split by the active marks. Pure apart from that
    /// `stat`: a local file, never a request.
    pub(crate) fn refresh_marks(&mut self) {
        let was_error = self.marks.error().cloned();
        let changed = self.marks.refresh();
        if changed {
            match self.marks.error() {
                Some(crate::marks::MarksError::Parse(p)) => {
                    self.marks_notice = Some(StatusLine::warn(format!(
                        "marks.toml invalid ({} problem{}): marks off · :marks shows them",
                        p.len(),
                        if p.len() == 1 { "" } else { "s" }
                    )));
                }
                Some(e) => {
                    self.marks_notice = Some(StatusLine::warn(format!("{e}: marks off")));
                }
                None if was_error.is_some() => {
                    self.marks_notice = Some(StatusLine::info("marks.toml valid again: marks on"));
                }
                None => {}
            }
        }
        self.marks_now = jiff::Timestamp::now();
        // The first load of the session announces marks expiring soon.
        if changed
            && !self.marks_announced
            && let Some(set) = self.marks.set()
        {
            self.marks_announced = true;
            let soon = self.marks_now + jiff::SignedDuration::from_hours(72);
            let n = set
                .marks
                .iter()
                .filter(|m| m.expires.is_some_and(|e| e > self.marks_now && e <= soon))
                .count();
            if n > 0 && self.marks_notice.is_none() {
                self.marks_notice = Some(StatusLine::info(format!(
                    "{n} mark{} expire{} within 3 days · :marks",
                    if n == 1 { "" } else { "s" },
                    if n == 1 { "s" } else { "" }
                )));
            }
        }
        let keep = self.show_squelched;
        let computed = self.active_marks().map(|a| {
            let (active, waiting) = (a.len(), a.waiting_for_draft());
            if a.is_empty() {
                return (
                    None,
                    MarkedView {
                        active,
                        waiting,
                        ..MarkedView::default()
                    },
                );
            }
            let p = partition(self.focus_flows(), &a, keep);
            let unsquelched = match p.kept {
                Cow::Owned(v) => Some(v),
                Cow::Borrowed(_) => None,
            };
            (
                unsquelched,
                MarkedView {
                    marked: p.marked,
                    hits: p.kept_hits,
                    active,
                    waiting,
                    ..MarkedView::default()
                },
            )
        });
        let (unsquelched, marked) = match computed {
            Some((u, m)) => (u, Some(m)),
            None => (None, None),
        };
        self.unsquelched = unsquelched;
        self.marked = marked;
    }

    /// A status the marks file asked for (invalid, valid again, marks
    /// expiring soon): shown once by `update`.
    pub fn take_marks_notice(&mut self) -> Option<StatusLine> {
        self.marks_notice.take()
    }

    /// `M`: squelched flows shown (dimmed `~`) or left out, in every view
    /// and the app overlay. Session only; no request.
    pub fn toggle_squelched(&mut self) -> StatusLine {
        self.show_squelched = !self.show_squelched;
        self.rebuild_views();
        StatusLine::info(if self.show_squelched {
            "squelched rows shown (~) · M hides them"
        } else {
            "squelched rows hidden · M shows them"
        })
    }

    /// The marks' part of a view's panel meta: the squelched counts, the
    /// marks waiting for draft decisions, or `marks off: marks.toml
    /// invalid`. `None` when there is nothing to say.
    pub fn marks_meta(&self, view: TrafficView) -> Option<String> {
        if self.marks.error().is_some() {
            return Some("marks off: marks.toml invalid".into());
        }
        let mv = self.marked.as_ref()?;
        let sq = match view {
            TrafficView::Flows => mv.flows,
            TrafficView::Aggregate if self.apps_state.by != crate::tui::model::By::Pairs => {
                mv.sections
            }
            TrafficView::Aggregate if self.apps_state.by_service => mv.services,
            TrafficView::Aggregate => mv.pairs,
            TrafficView::Mesh => mv.mesh,
            TrafficView::Blocked => mv.blocked,
            TrafficView::Treemap => mv.treemap,
        };
        let mut parts: Vec<String> = Vec::new();
        if view == TrafficView::Blocked && mv.accepted > 0 {
            parts.push(format!(
                "{} blocked flows · {} accepted",
                mv.blocked_open, mv.accepted
            ));
        }
        parts.extend(sq.meta(self.show_squelched, view != TrafficView::Treemap));
        if mv.waiting > 0 {
            parts.push(format!(
                "{} mark{} wait for the draft decision (D)",
                mv.waiting,
                if mv.waiting == 1 { "" } else { "s" }
            ));
        }
        (!parts.is_empty()).then(|| parts.join(" · "))
    }

    /// The squelched records `pred` keeps, with the rows `gone` reported:
    /// one view's counts.
    pub(crate) fn squelch(&self, rows: u64, pred: impl Fn(&ResolvedFlow) -> bool) -> Squelch {
        Squelch {
            rows,
            flows: self
                .marked
                .as_ref()
                .map_or(0, |m| m.marked.squelched_where(pred)),
        }
    }

    /// The flows accepted as blocked (`accept-blocked` hits) of the result.
    pub(crate) fn accepted_flows(&self) -> Vec<ResolvedFlow> {
        self.marked
            .iter()
            .flat_map(|m| &m.marked.subsets)
            .filter(|s| s.action == Action::AcceptBlocked)
            .flat_map(|s| s.flows.iter().cloned())
            .collect()
    }
}

```

In `crates/illumio-tui/src/tui/update.rs`, replace:

```rust
    if traffic_answer && gen_open {
        generator::rebuild_gen(model);
    }
    cmds
```

with:

```rust
    if traffic_answer && gen_open {
        generator::rebuild_gen(model);
    }
    // The marks file changed into (or out of) an invalid state, or marks
    // expire soon: one status line, once.
    if let Some(s) = model.traffic.take_marks_notice() {
        model.status = s;
    }
    cmds
```

In `crates/illumio-tui/src/tui/update.rs`, replace:

```rust
        }
        KeyCode::Char('w') => return flow_rule_key(model),
        KeyCode::Char('g') => {
            model.overlay = Overlay::GroupKeys(open_key_picker(model));
```

with:

```rust
        }
        KeyCode::Char('w') => return flow_rule_key(model),
        // Squelched flows shown or left out: a rebuild, no request.
        KeyCode::Char('M') => model.status = model.traffic.toggle_squelched(),
        KeyCode::Char('g') => {
            model.overlay = Overlay::GroupKeys(open_key_picker(model));
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p illumio-tui --lib -- traffic_marks marks_tests`
Expected: PASS.

- [ ] **Step 5: Run the gate**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: no formatting change, no warning, every test passes.

- [ ] **Step 6: Commit**

```bash
git add crates/illumio-tui/src/tui/app.rs crates/illumio-tui/src/tui/mod.rs crates/illumio-tui/src/tui/model.rs crates/illumio-tui/src/tui/test_support.rs crates/illumio-tui/src/tui/traffic_marks.rs crates/illumio-tui/src/tui/update.rs crates/illumio-tui/src/tui/update/marks_tests.rs
git commit -m "feat(tui): the traffic views apply marks; M shows squelched rows

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

### Task 2: Drawing: the gutter, the panel metas, Blocked's accepted, the treemap, the Flows detail

**Files:**
- Modify: `crates/illumio-tui/src/tui/model.rs`
- Modify: `crates/illumio-tui/src/tui/table.rs`
- Modify: `crates/illumio-tui/src/tui/traffic_marks.rs`
- Modify: `crates/illumio-tui/src/tui/ui/screens/traffic.rs`
- Modify: `crates/illumio-tui/src/tui/ui/widgets/table.rs`
- Test: `crates/illumio-tui/tests/snapshots/tui_snapshots__traffic_marks_blocked_120x34.snap`
- Test: `crates/illumio-tui/tests/snapshots/tui_snapshots__traffic_marks_pairs_120x34.snap`
- Test: `crates/illumio-tui/tests/snapshots/tui_snapshots__traffic_marks_pairs_shown_120x34.snap`
- Test (modify): `crates/illumio-tui/tests/tui_snapshots.rs`

**Interfaces:**
- Consumes: Task 1 (`MarkedView`, `marks_meta`, `FlowRow::mark`, `RowKind::Header::marks`), Plan 1 (`Marker::{glyph, of, action}`, `RowMarks::marker`, `expiry_text`).
- Produces:

```rust
// table.rs
pub marker: Option<fn(&T) -> Option<pce::marks::Marker>>  // TableView field; None by default
// ui/widgets/table.rs
pub fn marker_style(m: Marker, t: &Theme) -> Style;  pub fn dims_row(m: Option<Marker>) -> bool;  pub fn name_style(m: Option<Marker>, t: &Theme) -> Option<Style>;
// traffic_marks.rs
pub fn flow_marker(&FlowRow) -> Option<Marker>; pub fn group_marker(&FlowGroup) -> ..; pub fn mesh_marker(&MeshRow) -> ..; pub fn section_marker(&SectionRow) -> ..;
impl TrafficState { pub fn flow_marks(&self, f: &ResolvedFlow) -> Vec<usize> }
// model.rs: TreemapState { highlights: BTreeMap<(String, String), Marker>, source_highlights: BTreeMap<String, Marker> }, fn highlight(&self, src, dst: Option<&str>)
// ui/screens/traffic.rs: pub fn layout_with(body: Rect, extra: u16) -> TrafficLayout  (the detail's `mark` line)
```

- [ ] **Step 1: Write the failing tests**

In `crates/illumio-tui/src/tui/traffic_marks.rs`, replace:

```rust

    #[test]
    fn a_profile_switch_keeps_the_file_and_m_and_scopes_to_the_new_pce() {
        let (mut m, _dir) = marked_model(RICH_MARKS);
```

with:

```rust

    #[test]
    fn partly_squelched_rows_show_the_hidden_records_beside_the_count() {
        let (m, _dir) = marked_model(RICH_MARKS);
        let t = &m.traffic;
        let dev = t
            .groups
            .rows()
            .iter()
            .find(|g| g.src.join("/") == "shop/dev" && g.dst.join("/") == "shop/dev")
            .unwrap();
        let col = t
            .groups
            .columns
            .iter()
            .position(|c| c.title == "flows")
            .unwrap();
        assert_eq!(t.groups.cell(dev, col), "1 ~1");
        assert_eq!(
            t.groups.full_cells(dev)[col],
            "1",
            ":export writes the count"
        );
        assert!(t.groups.marker.is_some() && t.flows.marker.is_some());
        let plain = crate::tui::test_support::model_with_rich_flows();
        assert!(plain.traffic.groups.marker.is_none(), "no marks, no gutter");
    }

    #[test]
    fn the_panels_say_what_is_squelched_and_the_detail_names_the_mark() {
        use crate::tui::test_support::render_to_string;
        let (mut m, _dir) = marked_model(RICH_MARKS);
        m.traffic.view = TrafficView::Aggregate;
        let out = render_to_string(&m, 160, 40);
        assert!(
            out.contains("4 rows (9 flows) squelched · M shows them"),
            "{out}"
        );
        m.traffic.view = TrafficView::Blocked;
        let out = render_to_string(&m, 160, 40);
        assert!(out.contains("10 blocked flows · 2 accepted"), "{out}");
        assert!(out.contains("· 2 pot.blocked · 2 accepted"), "{out}");
        m.traffic.view = TrafficView::Treemap;
        let out = render_to_string(&m, 160, 40);
        assert!(out.contains("9 flows squelched · M shows them"), "{out}");
        assert_eq!(
            m.traffic.treemap.highlight("shop/dev", Some("shop/prod")),
            Some(Marker::Critical)
        );
        assert_eq!(
            m.traffic.treemap.highlight("hr/prod", None),
            Some(Marker::Revisit)
        );
        assert_eq!(
            m.traffic.treemap.highlight("erp/prod", None),
            None,
            "reviewed is no highlight"
        );
        assert!(out.contains("◆ hr/"), "{out}");
        // The Flows detail: Corp's blocked flow into the shop database is
        // critical and squelched by the Corp mark too.
        m.traffic.view = TrafficView::Flows;
        m.traffic.show_detail = true;
        m.traffic
            .flows
            .select_where(|r| !r.src.ip_lists.is_empty() && r.port == Some(5432));
        let out = render_to_string(&m, 160, 40);
        assert!(
            out.contains("mark       critical m-crit · payments db · never expires · also matches: m-sq1 (squelch)"),
            "{out}"
        );
        assert!(
            out.contains("9 rows (9 flows) squelched · M shows them"),
            "{out}"
        );
    }

    #[test]
    fn an_invalid_file_says_marks_off_in_the_panel() {
        use crate::tui::test_support::render_to_string;
        let (m, _dir) = marked_model("[[mark]]\naction = \"squelch\"\n");
        let out = render_to_string(&m, 120, 34);
        assert!(out.contains("marks off: marks.toml invalid"), "{out}");
    }

    #[test]
    fn a_profile_switch_keeps_the_file_and_m_and_scopes_to_the_new_pce() {
        let (mut m, _dir) = marked_model(RICH_MARKS);
```

In `crates/illumio-tui/src/tui/ui/widgets/table.rs`, replace:

```rust
    }

    #[test]
    fn fit_measures_display_width_not_chars() {
```

with:

```rust
    }

    /// The cells of `buf` on the line holding `needle`: `(symbol, style)`.
    fn line_with(
        buf: &ratatui::buffer::Buffer,
        needle: &str,
    ) -> Vec<(String, ratatui::style::Color, Modifier)> {
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| {
                        let c = &buf[(x, y)];
                        (c.symbol().to_string(), c.fg, c.modifier)
                    })
                    .collect::<Vec<_>>()
            })
            .find(|l| {
                l.iter()
                    .map(|c| c.0.as_str())
                    .collect::<String>()
                    .contains(needle)
            })
            .unwrap_or_else(|| panic!("no line with {needle}"))
    }

    fn marked(n: &u32) -> Option<Marker> {
        match n {
            1 => Some(Marker::Critical),
            2 => Some(Marker::Reviewed),
            _ => None,
        }
    }

    #[test]
    fn the_marks_gutter_draws_glyphs_colours_the_name_cell_and_dims_quiet_rows() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        use ratatui::widgets::Block;
        let t = Theme::dark();
        let mut v = TableView::new(cols());
        v.set_rows(vec![1, 2, 3]);
        v.marker = Some(marked);
        let mut term = Terminal::new(TestBackend::new(60, 6)).unwrap();
        let ok = Style::new().fg(t.ok);
        term.draw(|f| render(f, f.area(), &v, &t, Block::bordered(), &|_| ok))
            .unwrap();
        let buf = term.backend().buffer();
        let crit = line_with(buf, "n1");
        let glyph = crit.iter().find(|c| c.0 == "●").unwrap();
        assert_eq!(glyph.1, t.bad);
        assert!(glyph.2.contains(Modifier::BOLD));
        let name = crit.iter().find(|c| c.0 == "n").unwrap();
        assert_eq!(name.1, t.bad, "the name cell takes the highlight");
        let other = crit.iter().rev().find(|c| c.0 == "1").unwrap();
        assert_eq!(other.1, t.ok, "the other cells keep the row's colour");
        let rev = line_with(buf, "n2");
        assert!(rev.iter().any(|c| c.0 == "✓"));
        assert!(
            rev.iter().filter(|c| c.0 == "n").all(|c| c.1 == t.dim),
            "dimmed"
        );
        let plain = line_with(buf, "n3");
        assert!(plain.iter().filter(|c| c.0 == "n").all(|c| c.1 == t.ok));
        // Without a marker there is no gutter: the first cell starts after
        // the cursor's two cells.
        v.marker = None;
        term.draw(|f| render(f, f.area(), &v, &t, Block::bordered(), &|_| ok))
            .unwrap();
        let text: String = line_with(term.backend().buffer(), "n3")
            .iter()
            .map(|c| c.0.clone())
            .collect();
        assert!(text.starts_with("│  n3"), "{text}");
    }

    #[test]
    fn the_spanning_table_draws_the_gutter_too() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        use ratatui::widgets::Block;
        let t = Theme::dark();
        let mut v = TableView::new(cols());
        v.set_rows(vec![1, 2, 3]);
        v.marker = Some(marked);
        let mut term = Terminal::new(TestBackend::new(60, 6)).unwrap();
        term.draw(|f| {
            render_spanning(
                f,
                f.area(),
                &v,
                &t,
                Block::bordered(),
                &|_| Style::new(),
                &|n| (*n == 3).then(|| (format!("header {n}"), Style::new().fg(t.accent))),
            )
        })
        .unwrap();
        let buf = term.backend().buffer();
        let crit: String = line_with(buf, "n1").iter().map(|c| c.0.clone()).collect();
        assert!(crit.starts_with("│▸ ● n1"), "{crit}");
        let name = line_with(buf, "n1")
            .into_iter()
            .find(|c| c.0 == "n")
            .unwrap();
        assert_eq!(name.1, t.bad);
        let header: String = line_with(buf, "header 3")
            .iter()
            .map(|c| c.0.clone())
            .collect();
        assert!(header.starts_with("│    header 3"), "{header}");
    }

    #[test]
    fn fit_measures_display_width_not_chars() {
```

Create `crates/illumio-tui/tests/snapshots/tui_snapshots__traffic_marks_blocked_120x34.snap`:

```text
---
source: crates/illumio-tui/tests/tui_snapshots.rs
expression: "render_to_string(&m, 120, 34)"
---
 illumio-tui pce.example · org 5 ●                                                                api 5 calls · 20.8 KB
  1 Traffic  │  2 Policy  │  3 Objects  │  4 Workloads  │  5 Events
╭ Filters ────────────────────────────────────────────────────────────────────────── window 24h · cap 1000 · unbounded ╮
│window 24h  src *  dst *  svc *  decision all                                                                         │
╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
╭ Result ─────────────────────────────────────────────────────────────────────────────────────────── 30 flows · 3s ago ╮
│30 flows allowed 8 pot.blocked 6 blocked 6  ██████████████████████████████████████████████████████████████████████████│
╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
╭ Blocked · conns: lifetime ─ in · blocked · 10 blocked flows · 2 accepted · 2 rows (2 flows) squelched · M shows them ╮
│    src                                              dst name               service    flows  conns   last seen       │
│▸   ▾ erp/prod  in · 2 groups · 98 conns · 2 pot.blocked · 2 accepted                                                 │
│  ≈   erp/dev                                        erp-api-p.example      8443/tcp        1      44 10-10 07:22:00  │
│      crm/prod                                       erp-api-p.example      8443/tcp        1      35 10-10 07:25:00  │
│      crm/prod                                       erp-db-p.example       1521/tcp        1      11 10-10 07:04:00  │
│  ≈   erp/dev                                        erp-db-p.example       1521/tcp        1       8 10-10 07:21:00  │
│  ● ▾ shop/prod  in · 3 groups · 24 conns · 2 blocked · 1 pot.blocked                                                 │
│  ◆   hr/prod                                        shop-db-p.example      5432/tcp        1      16 10-10 07:16:00  │
│  ●   shop/dev                                       shop-db-p.example      5432/tcp        1       6 10-10 07:12:00  │
│  ●   ipl:Corp/-                                     shop-db-p.example      5432/tcp        1       2 10-10 07:01:00  │
│  ◆ ▾ hr/dev  in · 1 group · 20 conns · 1 blocked                                                                     │
│  ◆   hr/prod                                        hr-db-d.example        5432/tcp        1      20 10-10 07:15:00  │
│    ▾ api.partner.example/-  in · 1 group · 18 conns · 1 pot.blocked                                                  │
│      crm/dev                                        api.partner.example    443/tcp         1      18 10-10 07:19:00  │
│    ▾ crm/prod  in · 1 group · 12 conns · 1 blocked                                                                   │
│      crm/dev                                        crm-db-p.example       3306/tcp        1      12 10-10 07:20:00  │
│  ◆ ▾ hr/prod  in · 1 group · 5 conns · 1 blocked                                                                     │
│  ◆   hr/dev                                         hr-web-p.example       icmp            1       5 10-10 07:00:30  │
│    ▾ shop/dev  in · 1 group · 3 conns · 1 pot.blocked                                                                │
│      shop/dev                                       shop-db-d.example      22/tcp          1       3 10-10 07:05:00  │
│                                                                                                                      │
│                                                                                                                      │
│                                                                                                                      │
╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
ready                                                 f filter  r run  a auto  v view  g group  / search  : cmd  ? help
```

Create `crates/illumio-tui/tests/snapshots/tui_snapshots__traffic_marks_pairs_120x34.snap`:

```text
---
source: crates/illumio-tui/tests/tui_snapshots.rs
expression: "render_to_string(&m, 120, 34)"
---
 illumio-tui pce.example · org 5 ●                                                                api 5 calls · 20.8 KB
  1 Traffic  │  2 Policy  │  3 Objects  │  4 Workloads  │  5 Events
╭ Filters ────────────────────────────────────────────────────────────────────────── window 24h · cap 1000 · unbounded ╮
│window 24h  src *  dst *  svc *  decision all                                                                         │
╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
╭ Result ─────────────────────────────────────────────────────────────────────────────────────────── 30 flows · 3s ago ╮
│30 flows allowed 8 pot.blocked 6 blocked 6  ██████████████████████████████████████████████████████████████████████████│
╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
╭ App-to-app · conns: lifetime ───────────────────────────────────── pairs · 4 rows (9 flows) squelched · M shows them ╮
│    src                        dst                        flows  conns    allowed pot.bl blocked                      │
│▸   crm/prod                   crm/prod                        2      850       2      0       0                      │
│  ✓ erp/prod                   erp/prod                        2      590       2      0       0                      │
│    shop/prod                  crm/prod                        1      150       1      0       0                      │
│  ◆ hr/prod                    crm/prod                        1       60       1      0       0                      │
│  ≈ erp/dev                    erp/prod                        2       52       0      1       1                      │
│    crm/prod                   erp/prod                        2       46       0      2       0                      │
│    shop/prod                  api.partner.example/-           1       33       1      0       0                      │
│    erp/prod                   api.partner.example/-           1       27       1      0       0                      │
│  ◆ hr/prod                    hr/dev                          1       20       0      0       1                      │
│    crm/dev                    api.partner.example/-           1       18       0      1       0                      │
│  ◆ hr/prod                    shop/prod                       1       16       0      1       0                      │
│    crm/dev                    crm/prod                        1       12       0      0       1                      │
│    erp/prod                   crm/prod                        1        9       0      0       0                      │
│  ● shop/dev                   shop/prod                       1        6       0      0       1                      │
│  ◆ hr/dev                     hr/prod                         1        5       0      0       1                      │
│  ~ shop/dev                   shop/dev                     1 ~1        3       0      1       0                      │
│  ● ipl:Corp/-                 shop/prod                    1 ~1        2       0      0       1                      │
│                                                                                                                      │
│                                                                                                                      │
│                                                                                                                      │
│                                                                                                                      │
│                                                                                                                      │
╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
ready                                                 f filter  r run  a auto  v view  g group  / search  : cmd  ? help
```

Create `crates/illumio-tui/tests/snapshots/tui_snapshots__traffic_marks_pairs_shown_120x34.snap`:

```text
---
source: crates/illumio-tui/tests/tui_snapshots.rs
expression: "render_to_string(&m, 120, 34)"
---
 illumio-tui pce.example · org 5 ●                                                                api 5 calls · 20.8 KB
  1 Traffic  │  2 Policy  │  3 Objects  │  4 Workloads  │  5 Events
╭ Filters ────────────────────────────────────────────────────────────────────────── window 24h · cap 1000 · unbounded ╮
│window 24h  src *  dst *  svc *  decision all                                                                         │
╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
╭ Result ─────────────────────────────────────────────────────────────────────────────────────────── 30 flows · 3s ago ╮
│30 flows allowed 15 pot.blocked 7 blocked 7  █████████████████████████████████████████████████████████████████████████│
╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
╭ App-to-app · conns: lifetime ──────────────────────────── pairs · 4 rows (9 flows) squelched, shown ~ · M hides them ╮
│    src                        dst                        flows  conns    allowed pot.bl blocked                      │
│▸   crm/prod                   crm/prod                        2      850       2      0       0                      │
│  ~ shop/prod                  shop/prod                       4      821       4      0       0                      │
│  ● ipl:Corp/-                 shop/prod                       2      702       1      0       1                      │
│  ✓ erp/prod                   erp/prod                        2      590       2      0       0                      │
│  ~ ipl:Corp/-                 crm/prod                        1      520       1      0       0                      │
│    shop/prod                  crm/prod                        1      150       1      0       0                      │
│    shop/dev                   shop/dev                        2       93       1      1       0                      │
│  ~ ipl:Corp/-                 hr/prod                         1       75       0      1       0                      │
│  ◆ hr/prod                    crm/prod                        1       60       1      0       0                      │
│  ≈ erp/dev                    erp/prod                        2       52       0      1       1                      │
│    crm/prod                   erp/prod                        2       46       0      2       0                      │
│    shop/prod                  api.partner.example/-           1       33       1      0       0                      │
│    erp/prod                   api.partner.example/-           1       27       1      0       0                      │
│  ◆ hr/prod                    hr/dev                          1       20       0      0       1                      │
│    crm/dev                    api.partner.example/-           1       18       0      1       0                      │
│  ◆ hr/prod                    shop/prod                       1       16       0      1       0                      │
│    crm/dev                    crm/prod                        1       12       0      0       1                      │
│    erp/prod                   crm/prod                        1        9       0      0       0                      │
│  ● shop/dev                   shop/prod                       1        6       0      0       1                      │
│  ◆ hr/dev                     hr/prod                         1        5       0      0       1                      │
│  ~ ipl:Corp/-                 erp/prod                        1        4       0      0       1                      │
│                                                                                                                      │
╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
ready                                                 f filter  r run  a auto  v view  g group  / search  : cmd  ? help
```

In `crates/illumio-tui/tests/tui_snapshots.rs`, replace:

```rust
    insta::assert_snapshot!("generator_fan_in_all_workloads_rule_list_80x24", out);
}
```

with:

```rust
    insta::assert_snapshot!("generator_fan_in_all_workloads_rule_list_80x24", out);
}

#[test]
fn traffic_marks_pairs_and_blocked_120x34() {
    use illumio_tui::tui::model::TrafficView;
    use illumio_tui::tui::test_support::{RICH_MARKS, model_with_marks};
    let dir = tempfile::tempdir().unwrap();
    let mut m = model_with_marks(dir.path(), RICH_MARKS);
    m.traffic.view = TrafficView::Aggregate;
    insta::assert_snapshot!("traffic_marks_pairs_120x34", render_to_string(&m, 120, 34));
    m.traffic.view = TrafficView::Blocked;
    insta::assert_snapshot!(
        "traffic_marks_blocked_120x34",
        render_to_string(&m, 120, 34)
    );
    m.traffic.toggle_squelched();
    m.traffic.view = TrafficView::Aggregate;
    insta::assert_snapshot!(
        "traffic_marks_pairs_shown_120x34",
        render_to_string(&m, 120, 34)
    );
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p illumio-tui --lib -- widgets::table traffic_marks && cargo test -p illumio-tui --test tui_snapshots traffic_marks`
Expected: FAIL: it does not compile (`no field marker on TableView`, `cannot find function flow_marks`).

- [ ] **Step 3: Implement**

In `crates/illumio-tui/src/tui/model.rs`, replace:

```rust
    /// terminal size (`ui::screens::traffic::treemap_area`).
    pub area: TreemapRect,
}

```

with:

```rust
    /// terminal size (`ui::screens::traffic::treemap_area`).
    pub area: TreemapRect,
    /// The highlight (critical, important, revisit) of each pair with one,
    /// by (source, destination) text, and of each source: the glyph its
    /// tile shows before its label. Computed at relayout, never per draw.
    pub highlights: BTreeMap<(String, String), pce::marks::Marker>,
    pub source_highlights: BTreeMap<String, pce::marks::Marker>,
}

```

In `crates/illumio-tui/src/tui/model.rs`, replace:

```rust
            has_bytes: false,
            area,
        }
    }
```

with:

```rust
            has_bytes: false,
            area,
            highlights: BTreeMap::new(),
            source_highlights: BTreeMap::new(),
        }
    }

    /// The highlight a tile shows: a destination by its pair, a source by
    /// the strongest of its pairs.
    pub fn highlight(&self, src: &str, dst: Option<&str>) -> Option<pce::marks::Marker> {
        match dst {
            Some(d) => self
                .highlights
                .get(&(src.to_string(), d.to_string()))
                .copied(),
            None => self.source_highlights.get(src).copied(),
        }
    }
```

In `crates/illumio-tui/src/tui/model.rs`, replace:

```rust
            title: "flows",
            width: 6,
            cell: |g| g.flows.to_string(),
            key: |g| SortKey::Num(g.flows),
            right: true,
```

with:

```rust
            title: "flows",
            width: 6,
            cell: |g| flows_cell(g.flows, g.marks.as_ref()),
            key: |g| SortKey::Num(g.flows),
            right: true,
```

In `crates/illumio-tui/src/tui/model.rs`, replace:

```rust
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 5,
            full_title: None,
            full: None,
        },
        Column {
            title: "conns",
            width: 8,
            cell: |g| g.connections.to_string(),
```

with:

```rust
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 5,
            full_title: None,
            full: Some(|g| g.flows.to_string()),
        },
        Column {
            title: "conns",
            width: 8,
            cell: |g| g.connections.to_string(),
```

In `crates/illumio-tui/src/tui/model.rs`, replace:

```rust
}

/// Bytes as `1.2K`, `-` when none were reported.
fn bytes_text(b: u64) -> String {
```

with:

```rust
}

/// A flows cell: the records shown, then `~12` for the squelched ones of
/// the row left out (its sort key stays the shown count; `:export` writes
/// the count alone).
fn flows_cell(flows: u64, m: Option<&RowMarks>) -> String {
    match m.map(|m| m.hidden).filter(|h| *h > 0) {
        Some(h) => format!("{flows} ~{h}"),
        None => flows.to_string(),
    }
}

/// Bytes as `1.2K`, `-` when none were reported.
fn bytes_text(b: u64) -> String {
```

In `crates/illumio-tui/src/tui/model.rs`, replace:

```rust
            key: no_sort,
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 9,
            full_title: None,
            full: None,
        },
        Column {
            title: "flows",
            width: 6,
            cell: |r| r.with_row(|p| p.flows.to_string()),
            key: no_sort,
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 5,
            full_title: None,
            full: None,
        },
        Column {
            title: "conns",
            width: 8,
```

with:

```rust
            key: no_sort,
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 9,
            full_title: None,
            full: None,
        },
        Column {
            title: "flows",
            width: 6,
            cell: |r| r.with_row(|p| flows_cell(p.flows, p.marks.as_ref())),
            key: no_sort,
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 5,
            full_title: None,
            full: None,
        },
        Column {
            title: "conns",
            width: 8,
```

In `crates/illumio-tui/src/tui/model.rs`, replace:

```rust
            hide_below: 0,
            narrow: None,
            narrow_width: 5,
            full_title: None,
            full: None,
        },
        Column {
            title: "conns",
            width: 8,
```

with:

```rust
            hide_below: 0,
            narrow: None,
            narrow_width: 5,
            full_title: None,
            full: Some(|r| r.with_row(|p| p.flows.to_string())),
        },
        Column {
            title: "conns",
            width: 8,
```

In `crates/illumio-tui/src/tui/model.rs`, replace:

```rust
            title: "flows",
            width: 6,
            cell: |r| r.with_row(|p| p.flows.to_string()),
            key: no_sort,
            right: true,
```

with:

```rust
            title: "flows",
            width: 6,
            cell: |r| r.with_row(|p| flows_cell(p.flows, p.marks.as_ref())),
            key: no_sort,
            right: true,
```

In `crates/illumio-tui/src/tui/model.rs`, replace:

```rust
            narrow: None,
            narrow_width: 5,
            full_title: None,
            full: None,
        },
        Column {
            title: "conns",
```

with:

```rust
            narrow: None,
            narrow_width: 5,
            full_title: None,
            full: Some(|r| r.with_row(|p| p.flows.to_string())),
        },
        Column {
            title: "conns",
```

In `crates/illumio-tui/src/tui/model.rs`, replace:

```rust
        };
        tm.cursor = tm.cursor.min(tm.targets().len().saturating_sub(1));
    }

```

with:

```rust
        };
        tm.cursor = tm.cursor.min(tm.targets().len().saturating_sub(1));
        tm.highlights.clear();
        tm.source_highlights.clear();
        for g in &tm.groups {
            let Some(m) = g
                .marks
                .as_ref()
                .and_then(RowMarks::marker)
                .filter(|m| m.action().is_some_and(|a| a.is_highlight()))
            else {
                continue;
            };
            let src = g.src.join("/");
            tm.highlights.insert((src.clone(), g.dst.join("/")), m);
            let strongest = |x: pce::marks::Marker| x.action();
            match tm.source_highlights.get(&src) {
                Some(old) if strongest(*old) <= strongest(m) => {}
                _ => {
                    tm.source_highlights.insert(src, m);
                }
            }
        }
    }

```

In `crates/illumio-tui/src/tui/table.rs`, replace:

```rust
    /// every cell.
    matcher: Option<fn(&T, &str) -> bool>,
}

```

with:

```rust
    /// every cell.
    matcher: Option<fn(&T, &str) -> bool>,
    /// A row's mark, drawn as a 2-cell gutter before the first column
    /// while set (marks active). Not a column: column indices, `s`, the
    /// default sort, search and `:export` do not see it.
    pub marker: Option<fn(&T) -> Option<pce::marks::Marker>>,
}

```

In `crates/illumio-tui/src/tui/table.rs`, replace:

```rust
            search_draft: String::new(),
            matcher: None,
        }
    }
```

with:

```rust
            search_draft: String::new(),
            matcher: None,
            marker: None,
        }
    }
```

In `crates/illumio-tui/src/tui/traffic_marks.rs`, replace:

```rust
//! screen split by them (squelched flows left out of every view unless
//! `M`), and what each view's meta says about them.
use crate::tui::model::{StatusLine, TrafficState, TrafficView};
use pce::marks::{Action, ActiveMarks, Hit, Marked, RowMarks, partition};
use pce::traffic::ResolvedFlow;
use std::borrow::Cow;

```

with:

```rust
//! screen split by them (squelched flows left out of every view unless
//! `M`), and what each view's meta says about them.
use crate::tui::model::{FlowRow, RowKind, SectionRow, StatusLine, TrafficState, TrafficView};
use pce::marks::{Action, ActiveMarks, Hit, Marked, Marker, RowMarks, partition};
use pce::traffic::{FlowGroup, MeshRow, ResolvedFlow};
use std::borrow::Cow;

```

In `crates/illumio-tui/src/tui/traffic_marks.rs`, replace:

```rust
}

/// A section header's marks: its rows' records per action summed, so a
/// header shows any highlight of its rows and is dimmed only when all are.
```

with:

```rust
}

/// The Flows view's gutter: the flow's own hit.
pub fn flow_marker(r: &FlowRow) -> Option<Marker> {
    r.mark.map(|h| Marker::of(h.action))
}

/// App-to-app pairs' (and by service) gutter.
pub fn group_marker(g: &FlowGroup) -> Option<Marker> {
    g.marks.as_ref().and_then(RowMarks::marker)
}

/// The Mesh view's gutter.
pub fn mesh_marker(r: &MeshRow) -> Option<Marker> {
    r.marks.as_ref().and_then(RowMarks::marker)
}

/// A sectioned table's gutter: a header shows its rows' marks summed.
pub fn section_marker(r: &SectionRow) -> Option<Marker> {
    match &r.kind {
        RowKind::Header { marks, .. } => marks.as_deref().and_then(RowMarks::marker),
        RowKind::Peer { row, .. } => row.marks.as_ref().and_then(RowMarks::marker),
    }
}

/// A section header's marks: its rows' records per action summed, so a
/// header shows any highlight of its rows and is dimmed only when all are.
```

In `crates/illumio-tui/src/tui/traffic_marks.rs`, replace:

```rust
        let a = self.active_marks().filter(|a| !a.is_empty())?;
        Some((a, mv))
    }

```

with:

```rust
        let a = self.active_marks().filter(|a| !a.is_empty())?;
        Some((a, mv))
    }

    /// The hit of the selected flow (Flows) and the weaker marks it also
    /// matches.
    pub fn flow_marks(&self, f: &ResolvedFlow) -> Vec<usize> {
        self.active_marks()
            .map(|a| a.matching(f))
            .unwrap_or_default()
    }

```

In `crates/illumio-tui/src/tui/traffic_marks.rs`, replace:

```rust
        self.unsquelched = unsquelched;
        self.marked = marked;
    }

```

with:

```rust
        self.unsquelched = unsquelched;
        self.marked = marked;
        // The gutter is drawn while a mark is active, and only then.
        let on = self.marked.as_ref().is_some_and(|m| m.active > 0);
        self.flows.marker = on.then_some(flow_marker as fn(&FlowRow) -> Option<Marker>);
        let groups = on.then_some(group_marker as fn(&FlowGroup) -> Option<Marker>);
        self.groups.marker = groups;
        self.services.marker = groups;
        self.mesh.marker = on.then_some(mesh_marker as fn(&MeshRow) -> Option<Marker>);
        let sections = on.then_some(section_marker as fn(&SectionRow) -> Option<Marker>);
        self.apps.marker = sections;
        self.blocked.marker = sections;
    }

```

In `crates/illumio-tui/src/tui/ui/screens/traffic.rs`, replace:

```rust
/// table takes the rest, detail 8 (0 when the terminal is short).
pub fn layout(body: Rect) -> TrafficLayout {
    let detail_h = if body.height >= 20 { 8 } else { 0 };
    let rows = Layout::vertical([
        Constraint::Length(3),
```

with:

```rust
/// table takes the rest, detail 8 (0 when the terminal is short).
pub fn layout(body: Rect) -> TrafficLayout {
    layout_with(body, 0)
}

/// `layout` with `extra` lines for the detail pane (its `mark` line while
/// marks are active); the table gives them up, so the treemap's area (the
/// two together) is the same.
pub fn layout_with(body: Rect, extra: u16) -> TrafficLayout {
    let detail_h = if body.height >= 20 { 8 + extra } else { 0 };
    let rows = Layout::vertical([
        Constraint::Length(3),
```

In `crates/illumio-tui/src/tui/ui/screens/traffic.rs`, replace:

```rust

pub fn render(model: &Model, f: &mut Frame, body: Rect) {
    let l = layout(body);
    render_filter_bar(model, f, l.filter);
    render_stats(model, f, l.stats);
```

with:

```rust

pub fn render(model: &Model, f: &mut Frame, body: Rect) {
    let marks_on = model.traffic.marked.as_ref().is_some_and(|m| m.active > 0);
    let l = layout_with(body, u16::from(marks_on));
    render_filter_bar(model, f, l.filter);
    render_stats(model, f, l.stats);
```

In `crates/illumio-tui/src/tui/ui/screens/traffic.rs`, replace:

```rust
        None => TrafficView::Flows.title().to_string(),
    };
    let title = unit_title(&title, &meta, area.width);
    let block = panel(&title, &meta, t, true);
```

with:

```rust
        None => TrafficView::Flows.title().to_string(),
    };
    if let Some(m) = model.traffic.marks_meta(TrafficView::Flows) {
        meta = if meta.is_empty() {
            m
        } else {
            format!("{m} · {meta}")
        };
    }
    let title = unit_title(&title, &meta, area.width);
    let block = panel(&title, &meta, t, true);
```

In `crates/illumio-tui/src/tui/ui/screens/traffic.rs`, replace:

```rust
        parts.push(st.label().to_string());
    }
    parts.push(format!("keys {}", model.traffic.effective_keys().join(",")));
    parts.push("Enter drills".into());
    let meta = crate::tui::ui::widgets::table::header_meta(view);
    if !meta.is_empty() {
```

with:

```rust
        parts.push(st.label().to_string());
    }
    // The marks' counts take the place of the keys and the Enter hint, so
    // the title keeps its room.
    match model.traffic.marks_meta(model.traffic.view) {
        Some(m) => parts.push(m),
        None => {
            parts.push(format!("keys {}", model.traffic.effective_keys().join(",")));
            parts.push("Enter drills".into());
        }
    }
    let meta = crate::tui::ui::widgets::table::header_meta(view);
    if !meta.is_empty() {
```

In `crates/illumio-tui/src/tui/ui/screens/traffic.rs`, replace:

```rust
    let t = &model.theme;
    let meta = crate::tui::ui::widgets::table::header_meta(&model.traffic.mesh);
    let meta = format!(
        "keys {} · u user · p process · Enter drills{}{}",
        model.traffic.effective_keys().join(","),
        if meta.is_empty() { "" } else { " · " },
```

with:

```rust
    let t = &model.theme;
    let meta = crate::tui::ui::widgets::table::header_meta(&model.traffic.mesh);
    // The marks' counts take the place of the key hints.
    let hints = model
        .traffic
        .marks_meta(TrafficView::Mesh)
        .unwrap_or_else(|| "u user · p process · Enter drills".into());
    let meta = format!(
        "keys {} · {hints}{}{}",
        model.traffic.effective_keys().join(","),
        if meta.is_empty() { "" } else { " · " },
```

In `crates/illumio-tui/src/tui/ui/screens/traffic.rs`, replace:

```rust
        None => TrafficView::Treemap.title().to_string(),
    };
    let block = panel_spans(&title, treemap_meta(tm, t), t, true);
    let inner = block.inner(area);
    f.render_widget(block, area);
```

with:

```rust
        None => TrafficView::Treemap.title().to_string(),
    };
    let mut meta = treemap_meta(tm, t);
    if let Some(m) = model.traffic.marks_meta(TrafficView::Treemap) {
        meta.push(Span::styled(format!(" · {m}"), t.dim_style()));
    }
    let block = panel_spans(&title, meta, t, true);
    let inner = block.inner(area);
    f.render_widget(block, area);
```

In `crates/illumio-tui/src/tui/ui/screens/traffic.rs`, replace:

```rust
        cur.is_some_and(|c| c.outer == outer && c.inner == inner_i)
    };
    for (i, tile) in tm.tiles.iter().enumerate() {
        if tile.children.is_empty() {
            draw_fill(f, inner, tile, t, is_cur(i, None));
            continue;
        }
```

with:

```rust
        cur.is_some_and(|c| c.outer == outer && c.inner == inner_i)
    };
    // A tile whose pair (or a source with a pair) has a highlight shows
    // its glyph before the label. Zoomed, the tiles are the destinations.
    let zoom = tm.zoom.as_ref().map(|z| z.join("/"));
    for (i, tile) in tm.tiles.iter().enumerate() {
        if tile.children.is_empty() {
            let mark = match &zoom {
                Some(z) => tm.highlight(z, Some(&tile.label)),
                None => tm.highlight(&tile.label, None),
            };
            draw_fill(f, inner, tile, t, is_cur(i, None), mark);
            continue;
        }
```

In `crates/illumio-tui/src/tui/ui/screens/traffic.rs`, replace:

```rust
            style = style.add_modifier(Modifier::REVERSED);
        }
        let label = clip_text(&tile.label, r.width.saturating_sub(2) as usize);
        f.render_widget(
            Block::bordered()
```

with:

```rust
            style = style.add_modifier(Modifier::REVERSED);
        }
        let label = match tm.highlight(&tile.label, None) {
            Some(m) => format!("{} {}", m.glyph(), tile.label),
            None => tile.label.clone(),
        };
        let label = clip_text(&label, r.width.saturating_sub(2) as usize);
        f.render_widget(
            Block::bordered()
```

In `crates/illumio-tui/src/tui/ui/screens/traffic.rs`, replace:

```rust
        );
        for (j, c) in tile.children.iter().enumerate() {
            draw_fill(f, inner, c, t, is_cur(i, Some(j)));
        }
    }
```

with:

```rust
        );
        for (j, c) in tile.children.iter().enumerate() {
            let mark = tm.highlight(&tile.label, Some(&c.label));
            draw_fill(f, inner, c, t, is_cur(i, Some(j)), mark);
        }
    }
```

In `crates/illumio-tui/src/tui/ui/screens/traffic.rs`, replace:

```rust

/// A filled tile: `▒` in the decision colour, the label on the first row
/// when it fits with a cell either side, the weight on the second.
fn draw_fill(f: &mut Frame, clip: Rect, tile: &Tile, t: &Theme, cursor: bool) {
    let r = cells(tile.rect).intersection(clip);
    if r.is_empty() {
```

with:

```rust

/// A filled tile: `▒` in the decision colour, the label on the first row
/// when it fits with a cell either side (after the glyph of its
/// highlight), the weight on the second.
fn draw_fill(
    f: &mut Frame,
    clip: Rect,
    tile: &Tile,
    t: &Theme,
    cursor: bool,
    mark: Option<pce::marks::Marker>,
) {
    let r = cells(tile.rect).intersection(clip);
    if r.is_empty() {
```

In `crates/illumio-tui/src/tui/ui/screens/traffic.rs`, replace:

```rust
        }
    };
    let mut lines = vec![row(Some(tile.label.clone()))];
    if r.height >= 2 {
        lines.push(row(Some(compact_count(tile.weight))));
```

with:

```rust
        }
    };
    let label = match mark {
        Some(m) => format!("{} {}", m.glyph(), tile.label),
        None => tile.label.clone(),
    };
    let mut lines = vec![row(Some(label))];
    if r.height >= 2 {
        lines.push(row(Some(compact_count(tile.weight))));
```

In `crates/illumio-tui/src/tui/ui/screens/traffic.rs`, replace:

```rust
        &move |_| bad,
    );
}

```

with:

```rust
        &move |_| bad,
    );
}

/// The selected flow's mark: `squelch m-7k2q9x · Nagios NRPE, known ·
/// expires 2027-01-15 (UTC)`, then `also matches: m-… (reviewed)` for the
/// weaker marks that match too. `None` without a hit.
fn mark_line(model: &Model, fl: &crate::tui::model::FlowRow) -> Option<String> {
    let hit = fl.mark?;
    let set = model.traffic.marks.set()?;
    let m = set.marks.get(hit.mark)?;
    let comment = if m.comment.trim().is_empty() {
        "(no comment)".to_string()
    } else {
        m.comment.clone()
    };
    let mut s = format!(
        "{} {} · {comment} · {}",
        hit.action.as_str(),
        m.id,
        pce::marks::expiry_text(m.expires)
    );
    let others: Vec<String> = model
        .traffic
        .flow_marks(&fl.flow)
        .into_iter()
        .filter(|i| *i != hit.mark)
        .filter_map(|i| set.marks.get(i))
        .map(|o| format!("{} ({})", o.id, o.action.as_str()))
        .collect();
    if !others.is_empty() {
        s.push_str(&format!(" · also matches: {}", others.join(", ")));
    }
    Some(s)
}

```

In `crates/illumio-tui/src/tui/ui/screens/traffic.rs`, replace:

```rust
        ])
    };
    let lines = vec![
        row("src", endpoint_line(&fl.src)),
        row("src labels", labels_line(&fl.src, &order)),
```

with:

```rust
        ])
    };
    let mut lines = vec![
        row("src", endpoint_line(&fl.src)),
        row("src labels", labels_line(&fl.src, &order)),
```

In `crates/illumio-tui/src/tui/ui/screens/traffic.rs`, replace:

```rust
        ),
    ];
    f.render_widget(
        Paragraph::new(lines).block(block).wrap(Wrap { trim: true }),
```

with:

```rust
        ),
    ];
    if let Some(text) = mark_line(model, fl) {
        lines.push(row("mark", text));
    }
    f.render_widget(
        Paragraph::new(lines).block(block).wrap(Wrap { trim: true }),
```

In `crates/illumio-tui/src/tui/ui/widgets/table.rs`, replace:

```rust
use crate::tui::table::{Column, TableView};
use crate::tui::ui::theme::Theme;
use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Cell, Paragraph, Row, Table, TableState};
```

with:

```rust
use crate::tui::table::{Column, TableView};
use crate::tui::ui::theme::Theme;
use pce::marks::Marker;
use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Cell, Paragraph, Row, Table, TableState};
```

In `crates/illumio-tui/src/tui/ui/widgets/table.rs`, replace:

```rust
    );
    true
}

```

with:

```rust
    );
    true
}

/// The gutter glyph's style: critical `bad` bold, important `warn`,
/// revisit `accent`, the quiet ones and squelch dim. Decision columns keep
/// their colours.
pub fn marker_style(m: Marker, t: &Theme) -> Style {
    match m {
        Marker::Critical => Style::new().fg(t.bad).add_modifier(Modifier::BOLD),
        Marker::Important => Style::new().fg(t.warn),
        Marker::Revisit => Style::new().fg(t.accent),
        _ => t.dim_style(),
    }
}

/// A whole row is dimmed when every flow in it is accepted, reviewed or
/// (shown) squelched.
pub fn dims_row(m: Option<Marker>) -> bool {
    matches!(
        m,
        Some(Marker::Accepted | Marker::Reviewed | Marker::Squelched)
    )
}

/// The name cell's colour: critical `bad`, important `warn`.
pub fn name_style(m: Option<Marker>, t: &Theme) -> Option<Style> {
    match m {
        Some(Marker::Critical) => Some(Style::new().fg(t.bad)),
        Some(Marker::Important) => Some(Style::new().fg(t.warn)),
        _ => None,
    }
}

/// The gutter of one row: the glyph and a space, or two spaces.
fn gutter_span(m: Option<Marker>, t: &Theme) -> Span<'static> {
    match m {
        Some(m) => Span::styled(format!("{} ", m.glyph()), marker_style(m, t)),
        None => Span::raw("  "),
    }
}

```

In `crates/illumio-tui/src/tui/ui/widgets/table.rs`, replace:

```rust
    }
    let cols = visible_columns(&view.columns, area.width);
    let constraints: Vec<Constraint> = cols
        .iter()
        .map(|&(_, w, _)| {
```

with:

```rust
    }
    let cols = visible_columns(&view.columns, area.width);
    let mut constraints: Vec<Constraint> = cols
        .iter()
        .map(|&(_, w, _)| {
```

In `crates/illumio-tui/src/tui/ui/widgets/table.rs`, replace:

```rust
        .collect();
    let widths: Vec<usize> = cols.iter().map(|&(_, w, _)| w as usize).collect();
    let header = Row::new(
        cols.iter()
            .map(|&(ci, _, _)| Cell::from(view.columns[ci].title))
            .collect::<Vec<_>>(),
    )
    .style(theme.header_style());
    let height = inner.height.saturating_sub(1) as usize; // minus header row
    let start = view.offset().min(view.visible().len());
```

with:

```rust
        .collect();
    let widths: Vec<usize> = cols.iter().map(|&(_, w, _)| w as usize).collect();
    let mut titles: Vec<Cell> = cols
        .iter()
        .map(|&(ci, _, _)| Cell::from(view.columns[ci].title))
        .collect();
    // The marks' gutter: one cell and the column spacing, before the first column.
    if view.marker.is_some() {
        constraints.insert(0, Constraint::Length(1));
        titles.insert(0, Cell::from(""));
    }
    let header = Row::new(titles).style(theme.header_style());
    let height = inner.height.saturating_sub(1) as usize; // minus header row
    let start = view.offset().min(view.visible().len());
```

In `crates/illumio-tui/src/tui/ui/widgets/table.rs`, replace:

```rust
        .map(|&i| {
            let item = &view.rows()[i];
            Row::new(row_cells(view, item, &cols, &widths)).style(row_style(item))
        })
        .collect();
```

with:

```rust
        .map(|&i| {
            let item = &view.rows()[i];
            let mut cells: Vec<Cell> = row_cells(view, item, &cols, &widths)
                .into_iter()
                .map(Cell::from)
                .collect();
            let Some(marker) = view.marker else {
                return Row::new(cells).style(row_style(item));
            };
            let m = marker(item);
            if let (Some(first), Some(s)) = (cells.first_mut(), name_style(m, theme)) {
                *first = first.clone().style(s);
            }
            cells.insert(
                0,
                Cell::from(Span::styled(
                    m.map(Marker::glyph).unwrap_or(" "),
                    m.map_or_else(Style::new, |m| marker_style(m, theme)),
                )),
            );
            let style = if dims_row(m) {
                theme.dim_style()
            } else {
                row_style(item)
            };
            Row::new(cells).style(style)
        })
        .collect();
```

In `crates/illumio-tui/src/tui/ui/widgets/table.rs`, replace:

```rust
    }
    let cols = visible_columns(&view.columns, area.width);
    let avail = inner.width.saturating_sub(2) as usize;
    let fixed: usize =
        cols.iter().map(|c| c.1 as usize).sum::<usize>() + cols.len().saturating_sub(1);
```

with:

```rust
    }
    let cols = visible_columns(&view.columns, area.width);
    // Two cells of cursor, two of the marks' gutter while it is drawn.
    let gutter = view.marker.is_some();
    let avail = inner.width.saturating_sub(if gutter { 4 } else { 2 }) as usize;
    let fixed: usize =
        cols.iter().map(|c| c.1 as usize).sum::<usize>() + cols.len().saturating_sub(1);
```

In `crates/illumio-tui/src/tui/ui/widgets/table.rs`, replace:

```rust
    );
    let mut lines = vec![Line::from(Span::styled(
        format!("  {header}"),
        theme.header_style(),
    ))];
```

with:

```rust
    );
    let mut lines = vec![Line::from(Span::styled(
        format!("{}{header}", if gutter { "    " } else { "  " }),
        theme.header_style(),
    ))];
```

In `crates/illumio-tui/src/tui/ui/widgets/table.rs`, replace:

```rust
    for (pos, &i) in view.visible()[start..end].iter().enumerate() {
        let item = &view.rows()[i];
        let (text, mut style) = match spanning(item) {
            Some((text, style)) => (clip_width(&text, avail), style),
            None => (
                line_of(row_cells(view, item, &cols, &widths)),
                row_style(item),
            ),
        };
        let selected = start + pos == view.selected_pos();
        let mut line = format!("{}{text}", if selected { "▸ " } else { "  " });
        if selected {
            // The cursor row is highlighted across the whole width.
            style = style.patch(theme.selected_style());
            line = fit(&line, inner.width as usize, false);
        }
        lines.push(Line::from(Span::styled(line, style)));
    }
    f.render_widget(Paragraph::new(lines), inner);
```

with:

```rust
    for (pos, &i) in view.visible()[start..end].iter().enumerate() {
        let item = &view.rows()[i];
        let m = view.marker.and_then(|f| f(item));
        let (text, mut style, name) = match spanning(item) {
            Some((text, style)) => (clip_width(&text, avail), style, 0),
            None => (
                line_of(row_cells(view, item, &cols, &widths)),
                row_style(item),
                widths.first().copied().unwrap_or(0),
            ),
        };
        if dims_row(m) {
            style = theme.dim_style();
        }
        let selected = start + pos == view.selected_pos();
        let cursor = if selected { "▸ " } else { "  " };
        let mut spans = vec![Span::styled(cursor, style)];
        if gutter {
            spans.push(gutter_span(m, theme));
        }
        // A highlight colours the name cell (the first column) only.
        let split = clip_width(&text, name).len();
        let (head, tail) = text.split_at(split);
        let head_style = name_style(m, theme).map_or(style, |s| style.patch(s));
        spans.push(Span::styled(head.to_string(), head_style));
        spans.push(Span::styled(tail.to_string(), style));
        if selected {
            // The cursor row is highlighted across the whole width.
            let used: usize = spans.iter().map(Span::width).sum();
            let pad = (inner.width as usize).saturating_sub(used);
            spans.push(Span::styled(" ".repeat(pad), style));
            for s in &mut spans {
                s.style = s.style.patch(theme.selected_style());
            }
        }
        lines.push(Line::from(spans));
    }
    f.render_widget(Paragraph::new(lines), inner);
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p illumio-tui --lib -- widgets::table traffic_marks && cargo test -p illumio-tui --test tui_snapshots`
Expected: PASS; the three new snapshots match their files.

- [ ] **Step 5: Run the gate**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: no formatting change, no warning, every test passes.

- [ ] **Step 6: Commit**

```bash
git add crates/illumio-tui/src/tui/model.rs crates/illumio-tui/src/tui/table.rs crates/illumio-tui/src/tui/traffic_marks.rs crates/illumio-tui/src/tui/ui/screens/traffic.rs crates/illumio-tui/src/tui/ui/widgets/table.rs crates/illumio-tui/tests/snapshots/tui_snapshots__traffic_marks_blocked_120x34.snap crates/illumio-tui/tests/snapshots/tui_snapshots__traffic_marks_pairs_120x34.snap crates/illumio-tui/tests/snapshots/tui_snapshots__traffic_marks_pairs_shown_120x34.snap crates/illumio-tui/tests/tui_snapshots.rs
git commit -m "feat(tui): marks gutter, squelch counts, accepted blocked flows, treemap glyphs, flow detail

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

### Task 3: The app overlay and the generator header

**Files:**
- Modify: `crates/illumio-tui/src/tui/app_view.rs`
- Modify: `crates/illumio-tui/src/tui/gen_view.rs`
- Modify: `crates/illumio-tui/src/tui/ui/widgets/app_overlay.rs`
- Modify: `crates/illumio-tui/src/tui/update.rs`
- Modify: `crates/illumio-tui/src/tui/update/generator.rs`
- Test (modify): `crates/illumio-tui/src/tui/update/marks_tests.rs`

**Interfaces:**
- Consumes: Tasks 1-2 (`active_marks`, `show_squelched`, `toggle_squelched`, `Squelch`, `marker_style`, `dims_row`, `name_style`), Plan 1 (`partition`, `annotate_peers`).
- Produces:

```rust
// app_view.rs
pub fn rebuild_traffic(&mut self, flows: &[ResolvedFlow], partial: bool, marks: Option<(&ActiveMarks, bool)>)  // signature change
pub fn panel_title(&self, side: AppPanel) -> String;   // "Inbound · 2 peers (5 flows) squelched"
// AppView fields: marks_on: bool, show_squelched: bool, squelched: [Squelch; 2]
// gen_view.rs: GenView.squelched: u64; header_title() ends " · 5 squelched (rules include them)"
```

`M` in the app overlay toggles squelched rows there and on the Traffic screen.

- [ ] **Step 1: Write the failing tests**

In `crates/illumio-tui/src/tui/app_view.rs`, replace:

```rust
        let m = model_with_flows();
        let mut v = AppView::new(shop(FocusDirection::Both), "24h".into(), 1);
        v.rebuild_traffic(m.traffic.loaded_flows(), false);
        assert_eq!((v.inbound.len(), v.outbound.len()), (3, 2));
        v.focus.direction = FocusDirection::In;
        v.rebuild_traffic(m.traffic.loaded_flows(), false);
        assert_eq!((v.inbound.len(), v.outbound.len()), (3, 0));
        assert_eq!(v.header_title(), "shop/prod · in · last 24h");
```

with:

```rust
        let m = model_with_flows();
        let mut v = AppView::new(shop(FocusDirection::Both), "24h".into(), 1);
        v.rebuild_traffic(m.traffic.loaded_flows(), false, None);
        assert_eq!((v.inbound.len(), v.outbound.len()), (3, 2));
        v.focus.direction = FocusDirection::In;
        v.rebuild_traffic(m.traffic.loaded_flows(), false, None);
        assert_eq!((v.inbound.len(), v.outbound.len()), (3, 0));
        assert_eq!(v.header_title(), "shop/prod · in · last 24h");
```

In `crates/illumio-tui/src/tui/app_view.rs`, replace:

```rust
        let m = model_with_flows();
        let mut v = AppView::new(shop(FocusDirection::Both), "24h".into(), 1);
        v.rebuild_traffic(m.traffic.loaded_flows(), false);
        assert_eq!(v.panel, AppPanel::Inbound);
        v.move_cursor(5);
```

with:

```rust
        let m = model_with_flows();
        let mut v = AppView::new(shop(FocusDirection::Both), "24h".into(), 1);
        v.rebuild_traffic(m.traffic.loaded_flows(), false, None);
        assert_eq!(v.panel, AppPanel::Inbound);
        v.move_cursor(5);
```

In `crates/illumio-tui/src/tui/update/marks_tests.rs`, replace:

```rust
    assert_eq!(m.traffic.flows.search_text(), "M");
}
```

with:

```rust
    assert_eq!(m.traffic.flows.search_text(), "M");
}

#[test]
fn the_app_overlay_leaves_out_squelched_peers_and_m_toggles_them() {
    use crate::tui::app_view::AppPanel;
    use crate::tui::commands::Command;
    use crate::tui::test_support::{render_to_string, with_policy};
    let (m, _dir) = marked_model(RICH_MARKS);
    let mut m = with_policy(m);
    update(&mut m, Msg::Command(Command::App(Some("shop/prod".into()))));
    let v = m.app_view.as_ref().unwrap();
    assert!(v.marks_on);
    assert!(
        v.inbound.iter().all(|p| p.peer_text() != "shop/prod"),
        "shop's 5432 into itself is squelched"
    );
    assert_eq!(
        v.panel_title(AppPanel::Inbound),
        "Inbound · 2 peers (5 flows) squelched"
    );
    assert_eq!(
        v.panel_title(AppPanel::Outbound),
        "Outbound · 1 peers (4 flows) squelched"
    );
    let out = render_to_string(&m, 120, 40);
    assert!(out.contains("● ipl:Corp/-"), "{out}");
    let before = v.inbound.len();
    assert_eq!(update(&mut m, key('M')), Vec::<Cmd>::new());
    let v = m.app_view.as_ref().unwrap();
    assert_eq!(v.inbound.len(), before + 2);
    assert_eq!(
        v.panel_title(AppPanel::Inbound),
        "Inbound · 2 peers (5 flows) squelched, shown ~"
    );
    assert!(m.traffic.show_squelched, "the Traffic screen follows");
}

#[test]
fn the_generator_counts_squelched_flows_and_its_rules_include_them() {
    use crate::tui::commands::Command;
    use crate::tui::test_support::{model_with_rich_flows, with_policy};
    let open = |m: &mut crate::tui::model::Model| {
        update(m, Msg::Command(Command::App(Some("shop/prod".into()))));
        assert_eq!(update(m, key('G')), Vec::<Cmd>::new());
    };
    let (m, _dir) = marked_model(RICH_MARKS);
    let mut marked = with_policy(m);
    open(&mut marked);
    let mut plain = with_policy(model_with_rich_flows());
    open(&mut plain);
    let (a, b) = (
        marked.gen_view.as_ref().unwrap(),
        plain.gen_view.as_ref().unwrap(),
    );
    assert!(
        a.header_title()
            .ends_with(" · 5 squelched (rules include them)"),
        "{}",
        a.header_title()
    );
    assert!(!b.header_title().contains("squelched"));
    assert_eq!(a.rows, b.rows, "marks are not policy");
    assert_eq!(a.proposal, b.proposal);
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p illumio-tui --lib marks_tests`
Expected: FAIL: it does not compile (`no field marks_on on AppView`, `no method panel_title`).

- [ ] **Step 3: Implement**

In `crates/illumio-tui/src/tui/app_view.rs`, replace:

```rust
//! policy. Built from what is loaded plus at most two requests, each counted
//! here. Pure: `update` fills it, `ui::widgets::app_overlay` draws it.
use pce::app::{AppPeer, AppPolicyRow, AppWorkload, WorkloadSummary, app_peers};
use pce::models::PolicyScope;
use pce::traffic::{Direction, Focus, FocusDirection, ResolvedFlow};
```

with:

```rust
//! policy. Built from what is loaded plus at most two requests, each counted
//! here. Pure: `update` fills it, `ui::widgets::app_overlay` draws it.
use crate::tui::traffic_marks::Squelch;
use pce::app::{AppPeer, AppPolicyRow, AppWorkload, WorkloadSummary, app_peers};
use pce::marks::{ActiveMarks, annotate_peers, partition};
use pce::models::PolicyScope;
use pce::traffic::{Direction, Focus, FocusDirection, ResolvedFlow};
```

In `crates/illumio-tui/src/tui/app_view.rs`, replace:

```rust
    /// blocked flows (in the direction in force) the draft would allow.
    pub draft_fixed: Option<u64>,
}

```

with:

```rust
    /// blocked flows (in the direction in force) the draft would allow.
    pub draft_fixed: Option<u64>,
    /// Marks are active: the peer panels draw the gutter.
    pub marks_on: bool,
    /// `M`: squelched peers are shown, dimmed `~`.
    pub show_squelched: bool,
    /// Per peer panel (inbound, outbound): the peer rows squelch removed
    /// and the squelched flow records of that side.
    pub squelched: [Squelch; 2],
}

```

In `crates/illumio-tui/src/tui/app_view.rs`, replace:

```rust
            cached: 0,
            draft_fixed: None,
        }
    }

    /// Peers from `flows` for the direction in force; a side the direction
    /// leaves out has no rows, and the cursor moves off its panel to the
    /// side shown (`w` and `Enter` act on what is on screen). No request.
    pub fn rebuild_traffic(&mut self, flows: &[ResolvedFlow], partial: bool) {
        let d = self.focus.direction;
        self.panel = match (self.panel, d) {
```

with:

```rust
            cached: 0,
            draft_fixed: None,
            marks_on: false,
            show_squelched: false,
            squelched: [Squelch::default(); 2],
        }
    }

    /// Peers from `flows` for the direction in force; a side the direction
    /// leaves out has no rows, and the cursor moves off its panel to the
    /// side shown (`w` and `Enter` act on what is on screen). With `marks`
    /// (the active marks, and `M`), the peers are those of the flows kept
    /// and carry their marks. No request.
    pub fn rebuild_traffic(
        &mut self,
        flows: &[ResolvedFlow],
        partial: bool,
        marks: Option<(&ActiveMarks, bool)>,
    ) {
        let d = self.focus.direction;
        self.panel = match (self.panel, d) {
```

In `crates/illumio-tui/src/tui/app_view.rs`, replace:

```rust
            (p, _) => p,
        };
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
        self.partial = partial;
        self.draft_fixed = flows
```

with:

```rust
            (p, _) => p,
        };
        let marks = marks.filter(|(a, _)| !a.is_empty());
        let part = marks.map(|(a, keep)| partition(flows, a, keep));
        let kept: &[ResolvedFlow] = part.as_ref().map_or(flows, |p| &p.kept);
        self.marks_on = marks.is_some();
        self.show_squelched = marks.is_some_and(|(_, keep)| keep);
        self.squelched = [Squelch::default(); 2];
        let focus = &self.focus;
        let mut side = |dir: Direction, skip: FocusDirection, i: usize| -> Vec<AppPeer> {
            if d == skip {
                return Vec::new();
            }
            let agg = |f: &[ResolvedFlow]| app_peers(f, focus, dir);
            let mut rows = agg(kept);
            if let (Some((a, _)), Some(p)) = (marks, &part) {
                let rows_gone = annotate_peers(&mut rows, &p.marked, a, agg);
                let on_side = |f: &ResolvedFlow| {
                    let ep = if dir == Direction::In { &f.dst } else { &f.src };
                    pce::traffic::group_key(ep, &focus.keys) == focus.app
                };
                self.squelched[i] = Squelch {
                    rows: rows_gone,
                    flows: p.marked.squelched_where(on_side),
                };
            }
            rows
        };
        let inbound = side(Direction::In, FocusDirection::Out, 0);
        let outbound = side(Direction::Out, FocusDirection::In, 1);
        self.inbound = inbound;
        self.outbound = outbound;
        self.partial = partial;
        self.draft_fixed = flows
```

In `crates/illumio-tui/src/tui/app_view.rs`, replace:

```rust
            }
            _ => None,
        }
    }

    /// `shop/prod · in+out · last 24h`.
```

with:

```rust
            }
            _ => None,
        }
    }

    /// A peer panel's title: `Inbound`, then `3 peers (41 flows)
    /// squelched` (`, shown ~` with `M`) when squelch took some.
    pub fn panel_title(&self, side: AppPanel) -> String {
        let mut t = side.title().to_string();
        if self.partial {
            t.push_str(" · partial");
        }
        let sq = self.squelched[usize::from(side == AppPanel::Outbound)];
        if sq.flows > 0 {
            t.push_str(&format!(
                " · {} peers ({} flows) squelched{}",
                sq.rows,
                sq.flows,
                if self.show_squelched { ", shown ~" } else { "" }
            ));
        }
        t
    }

    /// `shop/prod · in+out · last 24h`.
```

In `crates/illumio-tui/src/tui/gen_view.rs`, replace:

```rust
    /// Why the Any option is unavailable (no built-in Any IP list).
    pub any_missing: Option<String>,
}

```

with:

```rust
    /// Why the Any option is unavailable (no built-in Any IP list).
    pub any_missing: Option<String>,
    /// The app's flows a squelch mark hides elsewhere: the rules include
    /// them (marks are not policy), the header says so.
    pub squelched: u64,
}

```

In `crates/illumio-tui/src/tui/gen_view.rs`, replace:

```rust
            consolidate: BTreeMap::new(),
            any_missing: None,
        }
    }
```

with:

```rust
            consolidate: BTreeMap::new(),
            any_missing: None,
            squelched: 0,
        }
    }
```

In `crates/illumio-tui/src/tui/gen_view.rs`, replace:

```rust
    pub fn header_title(&self) -> String {
        let flows: u64 = self.in_force().map(|r| r.flows).sum();
        format!(
            "Generate · {} · last {} · {flows} flows · {}",
            self.focus.app_text(),
            self.window,
```

with:

```rust
    pub fn header_title(&self) -> String {
        let flows: u64 = self.in_force().map(|r| r.flows).sum();
        let squelched = if self.squelched > 0 {
            format!(" · {} squelched (rules include them)", self.squelched)
        } else {
            String::new()
        };
        format!(
            "Generate · {} · last {} · {flows} flows · {}{squelched}",
            self.focus.app_text(),
            self.window,
```

In `crates/illumio-tui/src/tui/ui/widgets/app_overlay.rs`, replace:

```rust
}

fn peer_line<'a>(p: &AppPeer, cells: usize, t: &Theme) -> Line<'a> {
    let conns = compact_count(p.connections);
    let svc_w = 9;
    // With draft decisions: `+N` flows the draft would allow, `-N` block.
    let draft = crate::tui::model::draft_short(p.draft.as_ref());
    let draft_w = if p.draft.is_some() { 7 } else { 0 };
    let peer_w = cells.saturating_sub(svc_w + conns.len().max(5) + 4 + draft_w);
    let mut spans = vec![Span::styled(
        format!(
            "{} {} {} {:>5}",
            fit(&p.peer_text(), peer_w),
            fit(&p.service, svc_w),
            mark(p.decision),
            conns
        ),
        t.decision_style(p.decision),
    )];
    if let Some(d) = &p.draft {
        let style = if d.draft_allowed > 0 {
```

with:

```rust
}

/// One peer row; with `gutter` (marks active) the mark's glyph first, the
/// peer in the highlight's colour, a quiet row dimmed.
fn peer_line<'a>(p: &AppPeer, cells: usize, t: &Theme, gutter: bool) -> Line<'a> {
    use crate::tui::ui::widgets::table::{dims_row, marker_style, name_style};
    let conns = compact_count(p.connections);
    let svc_w = 9;
    // With draft decisions: `+N` flows the draft would allow, `-N` block.
    let draft = crate::tui::model::draft_short(p.draft.as_ref());
    let draft_w = if p.draft.is_some() { 7 } else { 0 };
    let gutter_w = if gutter { 2 } else { 0 };
    let peer_w = cells.saturating_sub(svc_w + conns.len().max(5) + 4 + draft_w + gutter_w);
    let m = p.marks.as_ref().and_then(pce::marks::RowMarks::marker);
    let style = if dims_row(m) {
        t.dim_style()
    } else {
        t.decision_style(p.decision)
    };
    let mut spans = Vec::new();
    if gutter {
        spans.push(match m {
            Some(m) => Span::styled(format!("{} ", m.glyph()), marker_style(m, t)),
            None => Span::raw("  "),
        });
    }
    spans.push(Span::styled(
        fit(&p.peer_text(), peer_w),
        name_style(m, t).unwrap_or(style),
    ));
    spans.push(Span::styled(
        format!(
            " {} {} {:>5}",
            fit(&p.service, svc_w),
            mark(p.decision),
            conns
        ),
        style,
    ));
    if let Some(d) = &p.draft {
        let style = if d.draft_allowed > 0 {
```

In `crates/illumio-tui/src/tui/ui/widgets/app_overlay.rs`, replace:

```rust
fn render_peers(v: &AppView, side: AppPanel, t: &Theme, f: &mut Frame, area: Rect) {
    let focused = v.panel == side;
    let title = if v.partial {
        format!("{} · partial", side.title())
    } else {
        side.title().to_string()
    };
    let block = panel(&title, "", t, focused);
    let inner = block.inner(area);
```

with:

```rust
fn render_peers(v: &AppView, side: AppPanel, t: &Theme, f: &mut Frame, area: Rect) {
    let focused = v.panel == side;
    let title = v.panel_title(side);
    let block = panel(&title, "", t, focused);
    let inner = block.inner(area);
```

In `crates/illumio-tui/src/tui/ui/widgets/app_overlay.rs`, replace:

```rust
    }
    let w = inner.width as usize;
    let lines: Vec<Line> = rows.iter().map(|p| peer_line(p, w, t)).collect();
    f.render_widget(
        rows_paragraph(
```

with:

```rust
    }
    let w = inner.width as usize;
    let lines: Vec<Line> = rows
        .iter()
        .map(|p| peer_line(p, w, t, v.marks_on))
        .collect();
    f.render_widget(
        rows_paragraph(
```

In `crates/illumio-tui/src/tui/update.rs`, replace:

```rust
    };
    let t = &model.traffic;
    v.rebuild_traffic(t.flows_for(&focus), t.partial_for(&focus));
    let idx = model.index().cloned();
    v.label_hrefs = idx
```

with:

```rust
    };
    let t = &model.traffic;
    let a = t.active_marks();
    v.rebuild_traffic(
        t.flows_for(&focus),
        t.partial_for(&focus),
        a.as_ref().map(|a| (a, t.show_squelched)),
    );
    let idx = model.index().cloned();
    v.label_hrefs = idx
```

In `crates/illumio-tui/src/tui/update.rs`, replace:

```rust
    if let Some(v) = model.app_view.as_mut() {
        let focus = v.focus.clone();
        v.rebuild_traffic(t.flows_for(&focus), t.partial_for(&focus));
    }
}
```

with:

```rust
    if let Some(v) = model.app_view.as_mut() {
        let focus = v.focus.clone();
        let a = t.active_marks();
        v.rebuild_traffic(
            t.flows_for(&focus),
            t.partial_for(&focus),
            a.as_ref().map(|a| (a, t.show_squelched)),
        );
    }
}
```

In `crates/illumio-tui/src/tui/update.rs`, replace:

```rust
        KeyCode::Char('X') => {
            v.focus.direction = v.focus.direction.next();
            rebuild_app_traffic(model);
        }
```

with:

```rust
        KeyCode::Char('X') => {
            v.focus.direction = v.focus.direction.next();
            rebuild_app_traffic(model);
        }
        // Squelched peers shown or left out here and on the Traffic screen.
        KeyCode::Char('M') => {
            model.status = model.traffic.toggle_squelched();
            rebuild_app_traffic(model);
        }
```

In `crates/illumio-tui/src/tui/update/generator.rs`, replace:

```rust
    );
    let fresh = rows(flows, &v.focus, &model.limits.tier_key, &idx, target);
    let (gap, requery, window) = gap_text(model, v);
    let any_missing = pce::generate::consolidate::unavailable(Consolidate::Any, &idx);
    if let Some(v) = model.gen_view.as_mut() {
        v.any_missing = any_missing.map(str::to_string);
        v.set_rows(fresh);
        v.gap = gap;
```

with:

```rust
    );
    let fresh = rows(flows, &v.focus, &model.limits.tier_key, &idx, target);
    // Marks are not policy: the rows above saw every flow; only the
    // header counts the app's squelched ones.
    let squelched = model
        .traffic
        .active_marks()
        .map(|a| {
            flows
                .iter()
                .filter(|f| v.focus.keeps(f))
                .filter(|f| {
                    a.classify(f)
                        .is_some_and(|h| h.action == pce::marks::Action::Squelch)
                })
                .count() as u64
        })
        .unwrap_or(0);
    let (gap, requery, window) = gap_text(model, v);
    let any_missing = pce::generate::consolidate::unavailable(Consolidate::Any, &idx);
    if let Some(v) = model.gen_view.as_mut() {
        v.any_missing = any_missing.map(str::to_string);
        v.squelched = squelched;
        v.set_rows(fresh);
        v.gap = gap;
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p illumio-tui --lib -- marks_tests app_view && cargo test -p illumio-tui --test tui_snapshots`
Expected: PASS; every app overlay snapshot unchanged.

- [ ] **Step 5: Run the gate**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: no formatting change, no warning, every test passes.

- [ ] **Step 6: Commit**

```bash
git add crates/illumio-tui/src/tui/app_view.rs crates/illumio-tui/src/tui/gen_view.rs crates/illumio-tui/src/tui/ui/widgets/app_overlay.rs crates/illumio-tui/src/tui/update.rs crates/illumio-tui/src/tui/update/generator.rs crates/illumio-tui/src/tui/update/marks_tests.rs
git commit -m "feat(tui): marks in the app overlay and the generator header

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

### Task 4: `M` in the help; tracking; final gate

**Files:**
- Modify: `crates/illumio-tui/src/tui/ui/widgets/overlay.rs`
- Modify: `crates/illumio-tui/src/tui/update.rs`
- Test (modify): `crates/illumio-tui/tests/snapshots/tui_snapshots__help_overlay_80x24.snap`
- Test (modify): `crates/illumio-tui/tests/snapshots/tui_snapshots__help_overlay_end_80x24.snap`
- Test (modify): `crates/illumio-tui/tests/snapshots/tui_snapshots__help_overlay_end_light_styled_80x24.snap`

**Interfaces:**
- Consumes: Tasks 1-3.
- Produces: the help line `("Traffic  M", "show or hide squelched rows (~)")` after `Traffic  w` (75 lines); the tracking section.

- [ ] **Step 1: Write the failing tests**

In `crates/illumio-tui/src/tui/update.rs`, replace:

```rust
        use crate::tui::ui::widgets::overlay::{HELP_LINES, help_last_scroll};
        let mut m = model_fixture(); // 80x24
        assert_eq!(HELP_LINES.len(), 74);
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "Events   J"));
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "Traffic  A"));
```

with:

```rust
        use crate::tui::ui::widgets::overlay::{HELP_LINES, help_last_scroll};
        let mut m = model_fixture(); // 80x24
        assert_eq!(HELP_LINES.len(), 75);
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "Traffic  M"));
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "Events   J"));
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "Traffic  A"));
```

In `crates/illumio-tui/src/tui/update.rs`, replace:

```rust
        update(&mut m, key('?'));
        let last = help_last_scroll(24);
        assert_eq!(last, 52, "74 lines, 22 rows inside the box");
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "App-to-app x / X"));
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "App-to-app o"));
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "App-to-app d"));
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "Sections Space / z"));

        for _ in 0..52 {
            update(&mut m, key('j'));
        }
```

with:

```rust
        update(&mut m, key('?'));
        let last = help_last_scroll(24);
        assert_eq!(last, 53, "75 lines, 22 rows inside the box");
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "App-to-app x / X"));
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "App-to-app o"));
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "App-to-app d"));
        assert!(HELP_LINES.iter().any(|(k, _)| *k == "Sections Space / z"));

        for _ in 0..53 {
            update(&mut m, key('j'));
        }
```

In `crates/illumio-tui/tests/snapshots/tui_snapshots__help_overlay_80x24.snap`, replace:

```text
expression: out
---
╭ Keys ────────────────────────────────── 1-22 of 74 · j/k scroll · Esc closes ╮
│1 2 3 4 5                Traffic · Policy · Objects · Workloads · Events      │
│:                        command prompt: :traffic :policy :objects :workloads │
```

with:

```text
expression: out
---
╭ Keys ────────────────────────────────── 1-22 of 75 · j/k scroll · Esc closes ╮
│1 2 3 4 5                Traffic · Policy · Objects · Workloads · Events      │
│:                        command prompt: :traffic :policy :objects :workloads │
```

In `crates/illumio-tui/tests/snapshots/tui_snapshots__help_overlay_end_80x24.snap`, replace:

```text
expression: out
---
╭ Keys ───────────────────────────────── 53-74 of 74 · j/k scroll · Esc closes ╮
│w form   Ctrl-A Ctrl-R   all labels · create the typed ruleset                │
│Policy   d               toggle draft / active                                │
```

with:

```text
expression: out
---
╭ Keys ───────────────────────────────── 54-75 of 75 · j/k scroll · Esc closes ╮
│w form   Ctrl-A Ctrl-R   all labels · create the typed ruleset                │
│Policy   d               toggle draft / active                                │
```

In `crates/illumio-tui/tests/snapshots/tui_snapshots__help_overlay_end_light_styled_80x24.snap`, replace:

```text
expression: "render_styled(&m, 80, 24)"
---
╭ Keys ───────────────────────────────── 53-74 of 74 · j/k scroll · Esc closes ╮
    @0:Rgb(0, 102, 204)/Rgb(250, 250, 247) @2:Rgb(150, 50, 180)/Rgb(250, 250, 247) @6:Rgb(0, 102, 204)/Rgb(250, 250, 247) @41:Rgb(120, 124, 136)/Rgb(250, 250, 247) @78:Rgb(0, 102, 204)/Rgb(250, 250, 247)
│w form   Ctrl-A Ctrl-R   all labels · create the typed ruleset                │
```

with:

```text
expression: "render_styled(&m, 80, 24)"
---
╭ Keys ───────────────────────────────── 54-75 of 75 · j/k scroll · Esc closes ╮
    @0:Rgb(0, 102, 204)/Rgb(250, 250, 247) @2:Rgb(150, 50, 180)/Rgb(250, 250, 247) @6:Rgb(0, 102, 204)/Rgb(250, 250, 247) @41:Rgb(120, 124, 136)/Rgb(250, 250, 247) @78:Rgb(0, 102, 204)/Rgb(250, 250, 247)
│w form   Ctrl-A Ctrl-R   all labels · create the typed ruleset                │
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p illumio-tui help && cargo test -p illumio-tui --test tui_snapshots help_overlay`
Expected: FAIL: the help has 74 lines, the snapshots say 75.

- [ ] **Step 3: Implement**

In `crates/illumio-tui/src/tui/ui/widgets/overlay.rs`, replace:

```rust
        "draft allow rule from a blocked flow or row (writes)",
    ),
    (
        "Form     Ctrl-N Ctrl-P",
```

with:

```rust
        "draft allow rule from a blocked flow or row (writes)",
    ),
    ("Traffic  M", "show or hide squelched rows (~)"),
    (
        "Form     Ctrl-N Ctrl-P",
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p illumio-tui help && cargo test -p illumio-tui --test tui_snapshots help_overlay`
Expected: PASS.

- [ ] **Step 5: Run the gate**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: no formatting change, no warning, every test passes.

- [ ] **Step 6: Track the plan and review**

Append to `tasks/todo.md`:

```markdown
## Flow marks 2: TUI display (plan docs/superpowers/plans/2026-10-17-flow-marks-2-display.md)
- [x] Task 1 TrafficState applies marks; M
- [x] Task 2 gutter, metas, Blocked accepted, treemap, detail
- [x] Task 3 app overlay, generator header
- [x] Task 4 help, tracking, gate
```

Then append `## Review (Flow marks 2)`: what shipped, the test count, the new snapshots, "no marks key sends a request; nothing was written to any PCE".

- [ ] **Step 7: Commit**

```bash
git add crates/illumio-tui/src/tui/ui/widgets/overlay.rs crates/illumio-tui/src/tui/update.rs crates/illumio-tui/tests/snapshots/tui_snapshots__help_overlay_80x24.snap crates/illumio-tui/tests/snapshots/tui_snapshots__help_overlay_end_80x24.snap crates/illumio-tui/tests/snapshots/tui_snapshots__help_overlay_end_light_styled_80x24.snap tasks/todo.md
git commit -m "feat(tui): M in the help; flow marks part 2 tracking

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01XYuSvdRqoZDGp9ZTwrZ2tx"
```

## Self-review notes

- Spec coverage: `TrafficState` marks (refresh at rebuild, partition, `FlowRow::mark`, `RowMarks` on rows), `TableView::marker` and the gutter, `M`, the meta texts, Blocked's accepted counts, the treemap glyphs and meta, the app overlay panels, the generator header, invalid-file warnings, the startup expiry line, the `M` help line: Tasks 1-4. `m`, `:marks` and the other help lines are Plan 3.
- Verification: every step of this plan was applied in order to a clean checkout of main (after the plans before it); each task built clean under `cargo clippy --workspace --all-targets -- -D warnings` with its own tests passing, and the gate passed at the end.
