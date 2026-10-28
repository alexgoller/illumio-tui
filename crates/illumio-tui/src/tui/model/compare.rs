//! Compare mode on the Traffic screen (`c`): the baseline result, which
//! rows each view shows (`n`), the counts the Result header names, and the
//! `Δ` column of every view but the Treemap.
use super::*;
use pce::traffic::{Change, ChangeSet, CompareCounts, RowChange};

/// `n`: every row, or one change only.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ShowChanges {
    #[default]
    All,
    New,
    Changed,
    Gone,
}

impl ShowChanges {
    /// `n`: all → new → changed → gone → all.
    pub fn next(self) -> Self {
        match self {
            ShowChanges::All => ShowChanges::New,
            ShowChanges::New => ShowChanges::Changed,
            ShowChanges::Changed => ShowChanges::Gone,
            ShowChanges::Gone => ShowChanges::All,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ShowChanges::All => "all",
            ShowChanges::New => "new",
            ShowChanges::Changed => "changed",
            ShowChanges::Gone => "gone",
        }
    }

    pub fn set(self) -> ChangeSet {
        match self {
            ShowChanges::All => ChangeSet::ALL,
            ShowChanges::New => ChangeSet::of(&[Change::New]),
            ShowChanges::Changed => ChangeSet::of(&[Change::Changed]),
            ShowChanges::Gone => ChangeSet::of(&[Change::Gone]),
        }
    }
}

/// The baseline's answer as loaded.
#[derive(Debug, Clone)]
pub struct BaselineData {
    pub flows: Vec<ResolvedFlow>,
    pub truncated: bool,
    pub incomplete: bool,
    pub hit: bool,
    /// The filter it ran with (its cap: `B` raises it).
    pub filter: TrafficFilter,
    /// The window it covered: `[recent start − length, recent start)`.
    pub range: Range,
}

/// The rows per change of each view, before `n`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ViewCounts {
    pub flows: CompareCounts,
    pub pairs: CompareCounts,
    pub services: CompareCounts,
    pub apps: CompareCounts,
    pub mesh: CompareCounts,
    pub blocked: CompareCounts,
}

/// Compare mode: on from `c` until `C`, a filter or window change, or a
/// focused result. The baseline is queried only by `c` (and `B` on a
/// truncated one); `r` keeps it.
#[derive(Debug, Clone)]
pub struct CompareState {
    /// The baseline's length as typed (`7d`).
    pub length: String,
    /// The filter compare mode is for, whatever its cap: another one ends
    /// it.
    pub filter: TrafficFilter,
    /// `None` while the baseline query runs.
    pub baseline: Option<BaselineData>,
    pub show: ShowChanges,
    pub counts: ViewCounts,
}

impl CompareState {
    /// Whether `f` is the filter compare mode is for (any cap).
    pub fn same_filter(&self, f: &TrafficFilter) -> bool {
        TrafficFilter {
            max_results: self.filter.max_results,
            ..f.clone()
        } == self.filter
    }
}

/// The title of the column every view but the Treemap gains in compare
/// mode.
pub const DELTA_COLUMN: &str = "Δ";

fn delta_cell(c: Option<&RowChange>) -> String {
    c.map(|c| c.change.tag().to_string()).unwrap_or_default()
}

fn delta_key(c: Option<&RowChange>) -> SortKey {
    SortKey::Text(c.map(|c| c.change.as_str().to_string()).unwrap_or_default())
}

/// `:export`'s cell: `allowed→potentially_blocked` for a changed row, else
/// the change.
fn delta_full(c: Option<&RowChange>) -> String {
    c.map(RowChange::text).unwrap_or_default()
}

/// The title of the column after `Δ` in compare mode: a `CHG` row's
/// decisions then and now (#79).
pub const TRANSITION_COLUMN: &str = "was→now";

/// The transition column's width at least: its title.
pub(super) const TRANSITION_MIN: u16 = 7;

/// Whether a row is `CHG`: its transition cell is filled, in the warning
/// colour.
pub fn is_changed(c: Option<&RowChange>) -> bool {
    c.is_some_and(|c| c.change == Change::Changed)
}

/// A `CHG` row's `baseline set→recent set` (`allow→allow,block`); a
/// `CHG` plain pair's what changed instead (`+1 svc · allow→block`, #80);
/// empty on any other row.
pub fn transition_text(c: Option<&RowChange>) -> String {
    let Some(c) = c.filter(|c| c.change == Change::Changed) else {
        return String::new();
    };
    if c.pair.is_some() {
        return c.detail().unwrap_or_default();
    }
    c.baseline_decision
        .zip(c.recent_decision)
        .map(|(b, r)| format!("{}→{}", b.short_text(), r.short_text()))
        .unwrap_or_default()
}

/// A `CHG` pair or service row (its transition cell in the warning colour).
pub(super) fn changed_group(g: &FlowGroup) -> bool {
    is_changed(g.change.as_ref())
}

/// A `CHG` peer or flow row of a sectioned table.
pub(super) fn changed_section_row(r: &SectionRow) -> bool {
    is_changed(r.change())
}

/// A `CHG` mesh row.
pub(super) fn changed_mesh_row(r: &MeshRow) -> bool {
    is_changed(r.change.as_ref())
}

/// A `CHG` flow.
pub(super) fn changed_flow(r: &FlowRow) -> bool {
    is_changed(r.change.as_ref())
}

/// The Mesh view's table for `cols`, its `CHG` transition cells warned.
pub(super) fn mesh_table(cols: MeshColumns) -> TableView<MeshRow> {
    TableView::new(mesh_columns(cols)).with_warn_cell(TRANSITION_COLUMN, changed_mesh_row)
}

/// The transition column over `change`, as narrow as its title until
/// `fit_columns` sizes it to the rows.
macro_rules! transition_column {
    ($t:ty, $change:expr) => {
        Column::<$t> {
            title: TRANSITION_COLUMN,
            width: TRANSITION_MIN,
            cell: |r| transition_text($change(r)),
            key: |r| SortKey::Text(transition_text($change(r))),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: TRANSITION_MIN,
            full_title: None,
            full: None,
        }
    };
}

pub fn transition_flow_column() -> Column<FlowRow> {
    transition_column!(FlowRow, flow_change)
}

pub fn transition_section_column() -> Column<SectionRow> {
    transition_column!(SectionRow, section_change)
}

pub fn transition_group_column() -> Column<FlowGroup> {
    transition_column!(FlowGroup, group_change)
}

pub fn transition_mesh_column() -> Column<MeshRow> {
    transition_column!(MeshRow, mesh_change)
}

/// The width `visible_columns` draws `c` at: its narrow one below 120.
fn drawn_width<T>(c: &mut Column<T>, narrow: bool) -> &mut u16 {
    if narrow && c.narrow_width > 0 {
        &mut c.narrow_width
    } else {
        &mut c.width
    }
}

/// The narrowest a column gives up room to the transition column.
const ROOM_FLOOR: u16 = 8;

/// `cols` laid out for a table `area` cells wide (borders and the cursor
/// taken off, and the marks' gutter when `gutter`): the transition column as wide as `longest`, the `takers`
/// narrowed (one cell at a time, in turn, never below 8) to make room, the
/// transition cut where they cannot give more. A fill column counts as 8.
fn fit_at<T>(cols: &mut [Column<T>], area: u16, longest: u16, takers: &[&str], gutter: bool) {
    let narrow = area < 120;
    // Borders and the cursor; the gutter is one cell and a gap.
    let budget = area.saturating_sub(if gutter { 6 } else { 4 });
    let vis = crate::tui::ui::widgets::table::visible_columns(cols, area);
    let Some(at) = cols.iter().position(|c| c.title == TRANSITION_COLUMN) else {
        return;
    };
    let others: u16 = vis
        .iter()
        .filter(|v| v.0 != at)
        .map(|v| if v.1 == 0 { 8 } else { v.1 })
        .sum::<u16>()
        + vis.len().saturating_sub(1) as u16;
    let mut want = longest.max(TRANSITION_MIN);
    let mut over = (others + want).saturating_sub(budget);
    let shown: Vec<usize> = vis
        .iter()
        .map(|v| v.0)
        .filter(|&i| takers.contains(&cols[i].title))
        .collect();
    while over > 0 {
        let mut gave = false;
        for &i in &shown {
            let w = drawn_width(&mut cols[i], narrow);
            if over > 0 && *w > ROOM_FLOOR {
                *w -= 1;
                over -= 1;
                gave = true;
            }
        }
        if !gave {
            break;
        }
    }
    want = want.saturating_sub(over).max(TRANSITION_MIN);
    *drawn_width(&mut cols[at], narrow) = want;
}

/// `cols` (built afresh) with the transition column, if any, sized to the
/// longest transition among `table`'s rows (left out when every row's is
/// empty), so the table fits at 80
/// columns (and at 120 wide), its marks' gutter included; set on `table` when they differ from its own
/// (the sort stays).
pub(super) fn fit_columns<T>(
    table: &mut TableView<T>,
    mut cols: Vec<Column<T>>,
    takers: &[&str],
    transition: fn(&T) -> String,
) {
    if cols.iter().any(|c| c.title == TRANSITION_COLUMN) {
        let longest = table
            .rows()
            .iter()
            .map(|r| transition(r).chars().count())
            .max()
            .unwrap_or(0)
            .min(u16::MAX as usize) as u16;
        if longest == 0 {
            // #80: no row has a transition: no column, the others keep
            // their widths.
            cols.retain(|c| c.title != TRANSITION_COLUMN);
        } else {
            let gutter = table.marker.is_some();
            fit_at(&mut cols, 80, longest, takers, gutter);
            fit_at(&mut cols, 120, longest, takers, gutter);
        }
    }
    let same = table.columns.len() == cols.len()
        && table.columns.iter().zip(&cols).all(|(a, b)| {
            a.title == b.title
                && a.width == b.width
                && a.narrow_width == b.narrow_width
                && a.hide_below == b.hide_below
                && a.narrow.is_some() == b.narrow.is_some()
        });
    if !same {
        table.set_columns(cols);
    }
}

pub(super) fn flow_transition(r: &FlowRow) -> String {
    transition_text(flow_change(r))
}

pub(super) fn section_transition(r: &SectionRow) -> String {
    transition_text(section_change(r))
}

pub(super) fn group_transition(g: &FlowGroup) -> String {
    transition_text(group_change(g))
}

pub(super) fn mesh_transition(r: &MeshRow) -> String {
    transition_text(mesh_change(r))
}

pub(super) fn flow_change(r: &FlowRow) -> Option<&RowChange> {
    r.change.as_ref()
}

pub(super) fn section_change(r: &SectionRow) -> Option<&RowChange> {
    r.change()
}

pub(super) fn group_change(g: &FlowGroup) -> Option<&RowChange> {
    g.change.as_ref()
}

pub(super) fn mesh_change(r: &MeshRow) -> Option<&RowChange> {
    r.change.as_ref()
}

/// A `Δ` column over `change`.
macro_rules! delta_column {
    ($t:ty, $change:expr) => {
        Column::<$t> {
            title: DELTA_COLUMN,
            width: 4,
            cell: |r| delta_cell($change(r)),
            key: |r| delta_key($change(r)),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 4,
            full_title: Some("change"),
            full: Some(|r| delta_full($change(r))),
        }
    };
}

pub fn delta_flow_column() -> Column<FlowRow> {
    delta_column!(FlowRow, flow_change)
}

/// Sections' rows: a peer's change, a flow's (Flows by app); none on a
/// header.
pub fn delta_section_column() -> Column<SectionRow> {
    delta_column!(SectionRow, section_change)
}

pub fn delta_group_column() -> Column<FlowGroup> {
    delta_column!(FlowGroup, group_change)
}

pub fn delta_mesh_column() -> Column<MeshRow> {
    delta_column!(MeshRow, mesh_change)
}

/// The Flows columns with `src` and `dst` narrowed to make room for the
/// `Δ` column (5 cells with its gap).
pub(super) fn make_room_for_delta<T>(mut cols: Vec<Column<T>>, on: bool) -> Vec<Column<T>> {
    if on {
        for c in &mut cols {
            if matches!(c.title, "src" | "dst") {
                c.width = c.width.saturating_sub(3);
            }
        }
    }
    cols
}

/// `cols` with `delta` first and the transition column after it when
/// `on`.
pub(super) fn with_delta<T>(
    mut cols: Vec<Column<T>>,
    (delta, transition): (Column<T>, Column<T>),
    on: bool,
) -> Vec<Column<T>> {
    if on {
        cols.insert(0, delta);
        cols.insert(1, transition);
    }
    cols
}

/// A gone row: drawn dim, with the baseline's values.
pub fn is_gone(c: Option<&RowChange>) -> bool {
    c.is_some_and(|c| c.change == Change::Gone)
}

impl TrafficState {
    /// The rows per change of the view on screen; `None` on the Treemap
    /// (it has no `Δ`).
    pub fn view_counts(&self) -> Option<CompareCounts> {
        let c = &self.compare.as_ref()?.counts;
        Some(match self.view {
            TrafficView::Flows => c.flows,
            TrafficView::Aggregate if self.apps_state.by != By::Pairs => c.apps,
            TrafficView::Aggregate if self.apps_state.by_service => c.services,
            TrafficView::Aggregate => c.pairs,
            TrafficView::Mesh => c.mesh,
            TrafficView::Blocked => c.blocked,
            TrafficView::Treemap | TrafficView::Exposure => return None,
        })
    }

    /// The Result header's compare part: `vs 7d before · 12 new · 3
    /// changed · 5 gone` (`· gone only` while `n` narrows), `vs 7d before
    /// · baseline running`, or on the Treemap `vs 7d before · no Δ here`.
    pub fn compare_meta(&self) -> Option<String> {
        self.compare_meta_with(true)
    }

    /// `compare_meta`, without where the baseline ends unless `end` (a
    /// narrow header).
    pub fn compare_meta_with(&self, end: bool) -> Option<String> {
        let cmp = self.compare.as_ref()?;
        let Some(b) = &cmp.baseline else {
            return Some(format!("vs {} before · baseline running", cmp.length));
        };
        // Where the baseline ends: after `r` the recent start has moved on.
        let head = if end {
            let end = short_time_narrow(&pce::traffic::format_timestamp(b.range.end));
            format!("vs {} before {end}", cmp.length)
        } else {
            format!("vs {} before", cmp.length)
        };
        let Some(counts) = self.view_counts() else {
            return Some(format!("{head} · no Δ here"));
        };
        let mut s = format!("{head} · {}", counts.text());
        if cmp.show != ShowChanges::All {
            s.push_str(&format!(" · {} only", cmp.show.label()));
        }
        Some(s)
    }

    /// `baseline truncated: new/gone over-report` (or `recent`, or both)
    /// while compare mode has a truncated side; `baseline incomplete: a PCE
    /// region did not respond` too.
    pub fn compare_warning(&self) -> Option<String> {
        let b = self.compare.as_ref()?.baseline.as_ref()?;
        let recent = self.data.as_ref().is_some_and(|d| d.truncated);
        let mut parts = Vec::new();
        let side = match (recent, b.truncated) {
            (true, true) => Some("recent and baseline"),
            (true, false) => Some("recent"),
            (false, true) => Some("baseline"),
            (false, false) => None,
        };
        if let Some(side) = side {
            parts.push(format!("{side} truncated: new/gone over-report"));
        }
        if b.incomplete {
            parts.push("baseline incomplete: a PCE region did not respond".to_string());
        }
        (!parts.is_empty()).then(|| parts.join(" · "))
    }
}
