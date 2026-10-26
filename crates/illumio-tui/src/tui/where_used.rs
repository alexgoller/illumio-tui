//! `W` / `:used` on the Objects screen: what names the selected label,
//! label group, IP list or service (`pce::where_used`), from what is loaded:
//! the draft rulesets as the Policy screen loads them, the label groups and,
//! for a label, the pairing profiles and the workload list or one cached
//! one-row probe. What is missing is loaded once, on the key press; the
//! view is rebuilt as it lands, never on a timer.
use crate::tui::table::{Column, SortKey, TableView};
use pce::where_used::{RefKind, Reference, Target};

/// What the view still waits for, as its header says it.
pub const WAIT_RULESETS: &str = "rulesets";
pub const WAIT_COUNT: &str = "workload count";
pub const WAIT_PROFILES: &str = "pairing profiles";

fn col(title: &'static str, width: u16, cell: fn(&Reference) -> String) -> Column<Reference> {
    Column {
        title,
        width,
        cell,
        key: |_| SortKey::Text(String::new()),
        right: false,
        hide_below: 0,
        narrow: None,
        narrow_width: 0,
        full_title: None,
        full: None,
    }
}

/// The section's name on a row: `scope`, `rule`, `label group`, `pairing
/// profile`, `workloads`.
pub fn section_word(kind: RefKind) -> &'static str {
    match kind {
        RefKind::Scope => "scope",
        RefKind::Rule => "rule",
        RefKind::LabelGroup => "label group",
        RefKind::PairingProfile => "pairing profile",
        RefKind::Workloads => "workloads",
    }
}

/// section, the reference with its `via` ([`Reference::line`]).
pub fn columns() -> Vec<Column<Reference>> {
    vec![
        Column {
            key: |r| {
                SortKey::Num(RefKind::ALL.iter().position(|k| *k == r.kind).unwrap_or(0) as u64)
            },
            ..col("section", 15, |r| section_word(r.kind).to_string())
        },
        Column {
            key: |r| SortKey::Text(r.line()),
            ..col("reference", 0, Reference::line)
        },
        Column {
            key: |r| SortKey::Text(r.href.clone()),
            // `:export` only.
            hide_below: u16::MAX,
            ..col("href", 0, |r| r.href.clone())
        },
    ]
}

/// The open overlay.
#[derive(Debug)]
pub struct WhereUsedView {
    pub target: Target,
    /// `env=prod` for a label, else the object's name.
    pub name: String,
    pub table: TableView<Reference>,
    /// What is still on its way ([`WAIT_RULESETS`], [`WAIT_PROFILES`],
    /// [`WAIT_COUNT`]).
    pub waiting: Vec<&'static str>,
    /// Requests this view sent (counted by the api counter too).
    pub requests: usize,
    /// Rulesets whose deny rules are not loaded (they may name it).
    pub deny_unread: usize,
    /// A load it needed failed: why.
    pub failed: Option<String>,
}

impl WhereUsedView {
    pub fn new(target: Target, name: String) -> Self {
        Self {
            target,
            name,
            table: TableView::new(columns()),
            waiting: Vec::new(),
            requests: 0,
            deny_unread: 0,
            failed: None,
        }
    }

    /// References found, the workloads row not counted.
    pub fn total(&self) -> usize {
        self.table
            .rows()
            .iter()
            .filter(|r| r.kind != RefKind::Workloads)
            .count()
    }

    /// `Rulesets 1 · Rules 2 · Label groups 1 · Workloads 2`: each section
    /// with its count, empty ones left out (Workloads: the count itself).
    pub fn sections(&self) -> String {
        let rows = self.table.rows();
        RefKind::ALL
            .iter()
            .filter_map(|k| {
                let mine: Vec<&Reference> = rows.iter().filter(|r| r.kind == *k).collect();
                let n = match k {
                    RefKind::Workloads => mine.first().map(|r| {
                        pce::where_used::WorkloadCount {
                            count: r.count.unwrap_or(0),
                            exact: r.exact.unwrap_or(true),
                        }
                        .text()
                    }),
                    _ => (!mine.is_empty()).then(|| mine.len().to_string()),
                }?;
                Some(format!("{} {n}", k.title()))
            })
            .collect::<Vec<_>>()
            .join(" · ")
    }

    /// Replaces the rows, the selection kept on the same reference.
    pub fn set_rows(&mut self, rows: Vec<Reference>) {
        let keep = self
            .table
            .selected_item()
            .map(|r| (r.kind, r.href.clone(), r.side));
        self.table.set_rows(rows);
        if let Some((kind, href, side)) = keep {
            self.table
                .select_where(|r| r.kind == kind && r.href == href && r.side == side);
        }
    }
}
