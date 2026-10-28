//! Compare mode on the Workloads screen (#85): a snapshot's workloads as
//! the baseline (`b` picks it), each row's change in the `Δ` column and a
//! changed one's transitions in `was→now`, the workloads gone since as dim
//! rows with the snapshot's values; `n` narrows to one change, `C` leaves.
//! The loaded list against the file: never a request.
use super::*;
use crate::snapshot::baseline::WlChange;
use pce::traffic::{Change, CompareCounts};

/// The snapshot compare mode is on, and what it shows.
#[derive(Debug, Clone)]
pub struct WlCompare {
    /// The snapshot's id: its UTC time.
    pub id: String,
    pub taken_at: jiff::Timestamp,
    pub label: Option<String>,
    /// The snapshot's workloads.
    pub workloads: Vec<Workload>,
    /// The snapshot's workloads read hit its row cap: gone over-reports.
    pub truncated: bool,
    pub show: ShowChanges,
    /// The rows per change, the filters applied, before `n`.
    pub counts: CompareCounts,
}

impl WlCompare {
    /// A baseline just loaded: every row shown.
    pub fn new(b: crate::tui::msg::WlBaseline) -> Self {
        Self {
            id: b.id,
            taken_at: b.taken_at,
            label: b.label,
            workloads: b.workloads,
            truncated: b.truncated,
            show: ShowChanges::All,
            counts: CompareCounts::default(),
        }
    }

    /// Whether `n` shows a row with `change`.
    pub fn shows(&self, change: Option<&WlChange>) -> bool {
        change.is_none_or(|c| self.show.set().contains(c.change))
    }
}

/// A workload row's change (none outside compare mode).
pub fn wl_change(r: &WlRow) -> Option<&WlChange> {
    r.change.as_ref()
}

/// A gone row: only in the snapshot, drawn dim with its values.
pub fn wl_gone(r: &WorkloadRowView) -> bool {
    r.change.as_ref().is_some_and(|c| c.change == Change::Gone)
}

/// A changed row: its `was→now` cell in the warning colour.
pub fn wl_changed(r: &WlRow) -> bool {
    r.change
        .as_ref()
        .is_some_and(|c| c.change == Change::Changed)
}

/// A changed row's transitions; empty on any other.
pub fn wl_transition(r: &WlRow) -> String {
    r.change
        .as_ref()
        .filter(|c| c.change == Change::Changed)
        .map(WlChange::was_text)
        .unwrap_or_default()
}

/// The `Δ` column of the Workloads table: `NEW`, `CHG`, `GONE`, blank for
/// the same; `:export`'s `change` the word.
pub fn wl_delta_column() -> Column<WlRow> {
    Column {
        title: DELTA_COLUMN,
        width: 4,
        cell: |r| {
            wl_change(r)
                .map(|c| c.change.tag().to_string())
                .unwrap_or_default()
        },
        key: |r| {
            SortKey::Text(
                wl_change(r)
                    .map(|c| c.change.as_str().to_string())
                    .unwrap_or_default(),
            )
        },
        right: false,
        hide_below: 0,
        narrow: None,
        narrow_width: 4,
        full_title: Some("change"),
        full: Some(|r| {
            wl_change(r)
                .map(|c| c.change.as_str().to_string())
                .unwrap_or_default()
        }),
    }
}

/// The `was→now` column: a changed row's transitions, as narrow as its
/// title until `fit_columns` sizes it to the rows.
pub fn wl_transition_column() -> Column<WlRow> {
    Column {
        title: TRANSITION_COLUMN,
        width: TRANSITION_MIN,
        cell: wl_transition,
        key: |r| SortKey::Text(wl_transition(r)),
        right: false,
        hide_below: 0,
        narrow: None,
        narrow_width: TRANSITION_MIN,
        full_title: None,
        full: None,
    }
}

/// The columns that give up room to `was→now`: the hostname keeps its
/// width (it names the row).
pub(super) const WL_TAKERS: &[&str] = &["ip", "name"];

impl WorkloadsState {
    /// Compare mode is on.
    pub fn comparing(&self) -> bool {
        self.compare.is_some()
    }

    /// The table header's compare part: `vs snapshot 10-20 06:00 · 3 new ·
    /// 5 changed · 1 gone` (`· gone only` while `n` narrows).
    pub fn compare_meta(&self) -> Option<String> {
        let c = self.compare.as_ref()?;
        let mut s = format!(
            "vs snapshot {} · {}",
            crate::snapshot::baseline::short_when(c.taken_at),
            c.counts.text()
        );
        if c.show != ShowChanges::All {
            s.push_str(&format!(" · {} only", c.show.label()));
        }
        Some(s)
    }

    /// `snapshot truncated: gone over-reports` (or the list, or both) while
    /// compare mode has a capped side.
    pub fn compare_warning(&self) -> Option<String> {
        let c = self.compare.as_ref()?;
        let side = match (self.truncated, c.truncated) {
            (true, true) => "list and snapshot",
            (true, false) => "list",
            (false, true) => "snapshot",
            (false, false) => return None,
        };
        Some(format!("{side} truncated: new/gone over-report"))
    }
}
