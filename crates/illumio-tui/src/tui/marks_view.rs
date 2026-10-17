//! The `:marks` overlay's state: every mark of `marks.toml` as a table
//! row (active ones first, in file order, then the out-of-scope and the
//! expired ones, greyed), with its hits in the loaded result; or the
//! problems of an invalid file. Pure: built from what is loaded.
use crate::tui::model::TrafficState;
use crate::tui::table::{Column, SortKey, TableView};
use jiff::Timestamp;
use pce::marks::{Action, Mark, Marker};

/// One mark on the overlay.
#[derive(Debug, Clone)]
pub struct MarkRow {
    pub mark: Mark,
    pub active: bool,
    /// `this PCE`, `any PCE`, `other PCE`.
    pub scope: String,
    /// `never`, `12d left`, or the date an expired mark ended.
    pub expires: String,
    /// `412`, `40 · 3 to stronger`, `–` for an inactive mark.
    pub hits: String,
    pub hit_count: u64,
    /// Of `hit_count`, the flows a stronger mark wins.
    pub stronger: u64,
}

/// `hits` below 120 columns: `40 ↑3` (3 to stronger), `412`, `–`.
fn hits_narrow(r: &MarkRow) -> String {
    match (r.active, r.stronger) {
        (false, _) => "–".into(),
        (true, 0) => r.hit_count.to_string(),
        (true, s) => format!("{} ↑{s}", r.hit_count),
    }
}

/// Below 120 columns the match keeps its room: `hits` and `comment`
/// narrow, `scope` goes below 100.
pub fn mark_columns() -> Vec<Column<MarkRow>> {
    let col = |title, width, cell: fn(&MarkRow) -> String, key, right| Column {
        title,
        width,
        cell,
        key,
        right,
        hide_below: 0,
        narrow: None,
        narrow_width: 0,
        full: None,
        full_title: None,
    };
    let mut cols = vec![
        col(
            "action",
            15,
            |r| r.mark.action.as_str().to_string(),
            |r| SortKey::Num(r.mark.action.index() as u64),
            false,
        ),
        col(
            "match",
            0,
            |r| r.mark.match_text(),
            |r| SortKey::Text(r.mark.match_text()),
            false,
        ),
        col(
            "scope",
            9,
            |r| r.scope.clone(),
            |r| SortKey::Text(r.scope.clone()),
            false,
        ),
        col(
            "expires",
            10,
            |r| r.expires.clone(),
            |r| {
                SortKey::Num(
                    r.mark
                        .expires
                        .map_or(u64::MAX, |t| u64::try_from(t.as_second()).unwrap_or(0)),
                )
            },
            false,
        ),
        col(
            "hits",
            18,
            |r| r.hits.clone(),
            |r| SortKey::Num(r.hit_count),
            true,
        ),
        col(
            "comment",
            22,
            |r| {
                if r.mark.comment.trim().is_empty() {
                    "(no comment)".into()
                } else {
                    r.mark.comment.clone()
                }
            },
            |r| SortKey::Text(r.mark.comment.clone()),
            false,
        ),
    ];
    cols[2].hide_below = 100;
    cols[4].narrow = Some(hits_narrow);
    cols[4].narrow_width = 9;
    cols[5].narrow_width = 16;
    cols
}

/// The gutter: an active mark's own glyph.
fn row_marker(r: &MarkRow) -> Option<Marker> {
    r.active.then(|| Marker::of(r.mark.action))
}

#[derive(Debug)]
pub struct MarksView {
    pub table: TableView<MarkRow>,
    /// `:marks <action>`: that action only.
    pub filter: Option<Action>,
    /// An invalid or unreadable file: its problems instead of rows.
    pub problems: Option<Vec<String>>,
    /// The file's path.
    pub path: String,
    pub active: usize,
    pub expired: usize,
}

/// `12d left`; under a day `<1d left`.
fn left(e: Timestamp, now: Timestamp) -> String {
    let days = (e.as_second() - now.as_second()) / 86_400;
    if days < 1 {
        "<1d left".into()
    } else {
        format!("{days}d left")
    }
}

impl MarksView {
    pub fn new(filter: Option<Action>) -> Self {
        let mut table = TableView::new(mark_columns());
        table.marker = Some(row_marker);
        Self {
            table,
            filter,
            problems: None,
            path: String::new(),
            active: 0,
            expired: 0,
        }
    }

    /// The rows again from the marks file and the loaded result; the
    /// cursor, sort and search stay. Classifies the loaded flows once per
    /// active mark: local, no request.
    pub fn rebuild(&mut self, t: &TrafficState, now: Timestamp) {
        self.path = t.marks.path().display().to_string();
        let Some(set) = t.marks.set() else {
            self.problems = Some(t.marks.error().map(|e| e.lines()).unwrap_or_default());
            self.table.set_rows(Vec::new());
            return;
        };
        self.problems = None;
        let active = set.active(Some(&t.marks_pce), now, t.draft_loaded());
        let flows = t.loaded_flows();
        let mut rows: Vec<MarkRow> = set
            .marks
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let on = active.is_active(i);
                let (hits, n, strong) = if on {
                    let mut n = 0u64;
                    let mut stronger = 0u64;
                    for f in flows.iter().filter(|f| m.matches(f)) {
                        n += 1;
                        if active.classify(f).is_some_and(|h| h.mark != i) {
                            stronger += 1;
                        }
                    }
                    let text = if stronger > 0 {
                        format!("{n} · {stronger} to stronger")
                    } else {
                        n.to_string()
                    };
                    (text, n, stronger)
                } else {
                    ("–".to_string(), 0, 0)
                };
                MarkRow {
                    active: on,
                    scope: match &m.scope {
                        None => "any PCE".into(),
                        Some(s) if s.covers(&t.marks_pce) => "this PCE".into(),
                        Some(_) => "other PCE".into(),
                    },
                    expires: match m.expires {
                        None => "never".into(),
                        Some(e) if m.is_expired(now) => pce::marks::expiry_date(e),
                        Some(e) => left(e, now),
                    },
                    hits,
                    hit_count: n,
                    stronger: strong,
                    mark: m.clone(),
                }
            })
            .filter(|r| self.filter.is_none_or(|a| r.mark.action == a))
            .collect();
        self.active = rows.iter().filter(|r| r.active).count();
        self.expired = rows.iter().filter(|r| r.mark.is_expired(now)).count();
        // Active first, in file order; then the rest, in file order.
        rows.sort_by_key(|r| !r.active);
        self.table.set_rows(rows);
    }

    /// `Marks · 14 active · 3 expired · marks.toml`.
    pub fn title(&self) -> String {
        let file = std::path::Path::new(&self.path)
            .file_name()
            .map_or_else(|| "marks.toml".into(), |n| n.to_string_lossy().to_string());
        match &self.problems {
            Some(_) => format!("Marks · {file} invalid"),
            None => format!(
                "Marks · {} active · {} expired · {file}",
                self.active, self.expired
            ),
        }
    }

    /// `:marks revisit filters`, or the filter in force.
    pub fn meta(&self) -> String {
        match self.filter {
            Some(a) => format!("{} only · :marks shows all", a.as_str()),
            None => ":marks revisit filters".into(),
        }
    }

    pub fn selected(&self) -> Option<&MarkRow> {
        self.table.selected_item()
    }
}
