//! `:hygiene`: the labels, label groups, IP lists and services nothing
//! names, and the workloads missing label keys (`pce::hygiene`), from what
//! is loaded: the object lists (the name index, or the Objects tabs once
//! loaded), the draft rulesets as the Policy screen loads them, the label
//! groups, the pairing profiles and the workload list. What is missing is
//! loaded once, on `:hygiene`; the view is rebuilt as it lands, never on a
//! timer.
use crate::tui::table::{Column, SortKey, TableView};
use pce::hygiene::{Finding, Kind};

/// What the view still waits for, as its header says it.
pub const WAIT_RULESETS: &str = "rulesets";
pub const WAIT_PROFILES: &str = "pairing profiles";
pub const WAIT_WORKLOADS: &str = "workloads";

/// The object kinds, in their section's order.
pub const OBJECT_KINDS: [Kind; 4] = [Kind::Label, Kind::LabelGroup, Kind::IpList, Kind::Service];

fn col(title: &'static str, width: u16, cell: fn(&Finding) -> String) -> Column<Finding> {
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

/// kind, name, reason (`unused`, or the keys missing).
pub fn columns() -> Vec<Column<Finding>> {
    vec![
        Column {
            key: |r| SortKey::Num(Kind::ALL.iter().position(|k| *k == r.kind).unwrap_or(0) as u64),
            ..col("kind", 12, |r| r.kind.word().to_string())
        },
        Column {
            key: |r| SortKey::Text(r.name.to_ascii_lowercase()),
            ..col("name", 0, |r| r.name.clone())
        },
        Column {
            key: |r| SortKey::Text(r.reason.clone()),
            ..col("reason", 28, |r| match &r.missing {
                Some(keys) => format!("missing {}", keys.join(", ")),
                None => r.reason.clone(),
            })
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
pub struct HygieneView {
    pub table: TableView<Finding>,
    /// The label keys every workload is expected to carry.
    pub keys: Vec<String>,
    /// Unused rows per object kind ([`OBJECT_KINDS`]).
    pub unused: Vec<(Kind, usize)>,
    /// Workloads lacking each key, in `keys`' order.
    pub missing: Vec<(String, usize)>,
    /// The workloads checked; `None` before the list is here.
    pub workloads: Option<usize>,
    /// The workload list was cut short: the rest are not checked, and a
    /// label only they carry is listed unused.
    pub workloads_truncated: bool,
    /// What is still on its way ([`WAIT_RULESETS`], [`WAIT_PROFILES`],
    /// [`WAIT_WORKLOADS`]).
    pub waiting: Vec<&'static str>,
    /// Requests this view sent (counted by the api counter too).
    pub requests: usize,
    /// Rulesets whose deny rules are not loaded (they may name more).
    pub deny_unread: usize,
    /// A load it needed failed: why.
    pub failed: Option<String>,
    /// The user moved the cursor since the overlay opened: rows landing
    /// keep it on the same row; until then it stays on the first.
    pub moved: bool,
}

impl HygieneView {
    pub fn new(keys: Vec<String>) -> Self {
        Self {
            table: TableView::new(columns()),
            keys,
            unused: Vec::new(),
            missing: Vec::new(),
            workloads: None,
            workloads_truncated: false,
            waiting: Vec::new(),
            requests: 0,
            deny_unread: 0,
            failed: None,
            moved: false,
        }
    }

    /// Unused objects (candidates).
    pub fn unused_total(&self) -> usize {
        self.unused.iter().map(|(_, n)| n).sum()
    }

    /// Workloads missing one or more keys.
    pub fn missing_total(&self) -> usize {
        self.table
            .rows()
            .iter()
            .filter(|r| r.kind == Kind::Workload)
            .count()
    }

    /// `Unused: labels 1 · label groups 0 · IP lists 0 · services 1`.
    pub fn unused_line(&self) -> String {
        let kinds: Vec<String> = self
            .unused
            .iter()
            .map(|(k, n)| format!("{} {n}", k.plural()))
            .collect();
        format!("Unused: {}", kinds.join(" · "))
    }

    /// `Missing labels: role 2 · app 2 · env 2 · loc 4 (of 4 workloads)`;
    /// before the workloads are here, the keys alone.
    pub fn missing_line(&self) -> String {
        match self.workloads {
            Some(n) => {
                let keys: Vec<String> = self
                    .missing
                    .iter()
                    .map(|(k, c)| format!("{k} {c}"))
                    .collect();
                let more = if self.workloads_truncated { "+" } else { "" };
                format!(
                    "Missing labels: {} (of {n}{more} workloads)",
                    keys.join(" · ")
                )
            }
            None => format!("Missing labels: {}", self.keys.join(", ")),
        }
    }

    /// The selected row's identity: kind and href.
    pub fn selected_id(&self) -> Option<(Kind, String)> {
        self.table.selected_item().map(|r| (r.kind, r.href.clone()))
    }

    /// Replaces the rows (loads land in any order). The cursor stays on
    /// the first row until the user moves it; after that, on the same row,
    /// wherever it lands.
    pub fn set_rows(&mut self, rows: Vec<Finding>) {
        let keep = self.selected_id().filter(|_| self.moved);
        self.table.set_rows(rows);
        match keep {
            Some((kind, href)) => self
                .table
                .select_where(|r| r.kind == kind && r.href == href),
            None => self.table.home(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finding(kind: Kind, name: &str, missing: Option<&[&str]>) -> Finding {
        Finding {
            kind,
            name: name.into(),
            href: format!("/orgs/5/x/{name}"),
            reason: match missing {
                Some(m) => format!("missing labels: {}", m.join(",")),
                None => "unused".into(),
            },
            missing: missing.map(|m| m.iter().map(|k| k.to_string()).collect()),
        }
    }

    #[test]
    fn the_header_lines_count_each_kind_and_key() {
        let mut v = HygieneView::new(vec!["app".into(), "env".into()]);
        assert_eq!(v.missing_line(), "Missing labels: app, env");
        v.unused = vec![(Kind::Label, 1), (Kind::IpList, 2)];
        v.missing = vec![("app".into(), 1), ("env".into(), 0)];
        v.workloads = Some(3);
        v.set_rows(vec![
            finding(Kind::Label, "loc=eu", None),
            finding(Kind::Workload, "web01", Some(&["app"])),
        ]);
        assert_eq!(v.unused_line(), "Unused: labels 1 · IP lists 2");
        assert_eq!(
            v.missing_line(),
            "Missing labels: app 1 · env 0 (of 3 workloads)"
        );
        v.workloads_truncated = true;
        assert!(v.missing_line().ends_with("(of 3+ workloads)"));
        assert_eq!((v.unused_total(), v.missing_total()), (3, 1));
        let reason = (columns()[2].cell)(&v.table.rows()[1]);
        assert_eq!(reason, "missing app");
    }
}
