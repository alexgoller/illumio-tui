//! `I` on Policy: the workloads behind a rule's impact, one side at a
//! time (providers, consumers), from the cached workload list and the
//! loaded rulesets: the same evaluation as the `P/C` counts, never a
//! request. A side made of IP lists says so instead of listing nothing.
use crate::cli::output::join_list;
use crate::tui::impact::{Source, loaded_rule, source};
use crate::tui::model::{Model, rule_line};
use crate::tui::table::{Column, SortKey, TableView};
use pce::impact::{ImpactInputs, RuleActors, Side, rule_matches};
use pce::models::{Actor, Workload};
use pce::{KeyOrder, NameIndex};

/// One matched workload as the list shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchedRow {
    pub href: String,
    /// The hostname, else the name (`display_name`).
    pub hostname: String,
    /// `env=prod role=web app=shop`, keys in the profile's order.
    pub labels: String,
    /// Every label, `; `-joined, for `:export`.
    pub label_list: String,
    /// The effective enforcement (`unmanaged` without a VEN).
    pub enforcement: String,
    pub online: Option<bool>,
}

impl MatchedRow {
    pub fn new(w: &Workload, order: &KeyOrder) -> Self {
        Self {
            href: w.href.clone(),
            hostname: w.display_name(),
            labels: w.label_text(order),
            label_list: join_list(w.label_entries(order)),
            enforcement: w.enforcement().unwrap_or_default().to_string(),
            online: w.online,
        }
    }

    fn online_text(&self) -> String {
        match self.online {
            Some(true) => "yes",
            Some(false) => "no",
            None => "-",
        }
        .to_string()
    }
}

fn col(title: &'static str, width: u16, cell: fn(&MatchedRow) -> String) -> Column<MatchedRow> {
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

/// hostname, labels, enforcement, online (and the href for `:export`).
pub fn matched_columns() -> Vec<Column<MatchedRow>> {
    vec![
        Column {
            key: |r| SortKey::Text(r.hostname.clone()),
            ..col("hostname", 24, |r| r.hostname.clone())
        },
        Column {
            key: |r| SortKey::Text(r.labels.clone()),
            full: Some(|r| r.label_list.clone()),
            ..col("labels", 0, |r| r.labels.clone())
        },
        Column {
            key: |r| SortKey::Text(r.enforcement.clone()),
            ..col("enforcement", 15, |r| r.enforcement.clone())
        },
        Column {
            key: |r| SortKey::Num(u64::from(r.online.unwrap_or(false))),
            full: Some(|r| r.online.map(|b| b.to_string()).unwrap_or_default()),
            ..col("online", 6, MatchedRow::online_text)
        },
        Column {
            key: |r| SortKey::Text(r.href.clone()),
            // `:export` only.
            hide_below: u16::MAX,
            ..col("href", 0, |r| r.href.clone())
        },
    ]
}

/// The open list: both sides evaluated once, the shown one in `table`.
#[derive(Debug)]
pub struct MatchedView {
    pub rule_href: String,
    /// The rule on one line, as the rules pane names it.
    pub rule: String,
    pub side: Side,
    pub providers: Vec<MatchedRow>,
    pub consumers: Vec<MatchedRow>,
    /// Per side what the list cannot show: IP lists, virtual services.
    pub provider_note: Option<String>,
    pub consumer_note: Option<String>,
    /// The workload list was truncated: the lists are lower bounds.
    pub truncated: bool,
    pub table: TableView<MatchedRow>,
    /// `:` typed in the list: the command being typed (`export <path>`).
    pub prompt: Option<String>,
}

impl MatchedView {
    pub fn rows(&self, side: Side) -> &[MatchedRow] {
        match side {
            Side::Provider => &self.providers,
            Side::Consumer => &self.consumers,
        }
    }

    pub fn note(&self, side: Side) -> Option<&str> {
        match side {
            Side::Provider => self.provider_note.as_deref(),
            Side::Consumer => self.consumer_note.as_deref(),
        }
    }

    /// Shows `side`, from the top, without a search.
    pub fn show(&mut self, side: Side) {
        self.side = side;
        self.table.search_clear();
        self.table.set_rows(self.rows(side).to_vec());
        self.table.home();
    }
}

/// `providers` / `consumers`.
pub fn side_word(side: Side) -> &'static str {
    match side {
        Side::Provider => "providers",
        Side::Consumer => "consumers",
    }
}

pub fn other(side: Side) -> Side {
    match side {
        Side::Provider => Side::Consumer,
        Side::Consumer => Side::Provider,
    }
}

/// What a side's list cannot show: a side of IP lists only (nothing to
/// list: addresses, not workloads), IP lists besides workloads, labels
/// resolved as virtual services.
pub fn side_note(
    actors: &[Actor],
    as_workloads: bool,
    idx: &NameIndex,
    side: Side,
) -> Option<String> {
    let word = side_word(side);
    let lists: Vec<&str> = actors
        .iter()
        .filter_map(|a| a.ip_list.as_ref())
        .map(|r| idx.ip_list_name(&r.href).unwrap_or(&r.href))
        .collect();
    if !lists.is_empty() && actors.iter().all(|a| a.ip_list.is_some()) {
        return Some(format!(
            "the {word} are IP lists ({}): addresses, not workloads; nothing to list",
            lists.join(", ")
        ));
    }
    let mut notes = Vec::new();
    if !lists.is_empty() {
        notes.push(format!("also IP lists, not listed: {}", lists.join(", ")));
    }
    if !as_workloads {
        notes.push(format!(
            "{word} labels resolve as virtual services: only workload actors count"
        ));
    }
    (!notes.is_empty()).then(|| notes.join(" · "))
}

/// The list for the loaded rule `href`, showing `side`; or why there is
/// none (the workloads not cached: `i` loads them; the rule not loaded).
pub fn build(model: &Model, href: &str, side: Side) -> Result<MatchedView, String> {
    let (workloads, truncated) = match source(&model.workloads) {
        Source::Ready {
            workloads,
            truncated,
        } => (workloads, truncated),
        Source::Loading => return Err("workloads still loading".into()),
        Source::Cold | Source::Failed(_) => {
            return Err("press i first: the impact needs the workloads (1 request)".into());
        }
    };
    let idx = model.index().ok_or("names not loaded yet")?;
    let (scopes, actors): (&[Vec<Actor>], RuleActors) =
        loaded_rule(&model.policy, href).ok_or("rule not loaded")?;
    let inputs = ImpactInputs {
        workloads,
        index: idx,
        truncated,
    };
    let (_, matched) = rule_matches(scopes, &actors, &inputs);
    let order = model.limits.key_order();
    let rows = |ix: &[usize]| -> Vec<MatchedRow> {
        ix.iter()
            .map(|&i| MatchedRow::new(&workloads[i], &order))
            .collect()
    };
    let rule = model
        .policy
        .rules
        .rows()
        .iter()
        .find(|r| r.view.href == href)
        .map_or_else(|| href.to_string(), |r| rule_line(&r.view));
    let mut v = MatchedView {
        rule_href: href.to_string(),
        rule,
        side,
        providers: rows(&matched.providers),
        consumers: rows(&matched.consumers),
        provider_note: side_note(
            &actors.providers,
            actors.providers_as_workloads,
            idx,
            Side::Provider,
        ),
        consumer_note: side_note(
            &actors.consumers,
            actors.consumers_as_workloads,
            idx,
            Side::Consumer,
        ),
        truncated,
        table: TableView::new(matched_columns()),
        prompt: None,
    };
    v.show(side);
    Ok(v)
}
