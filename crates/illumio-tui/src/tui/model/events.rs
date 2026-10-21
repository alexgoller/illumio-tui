//! The Events screen's state: its table columns, the filter being edited
//! and the loaded events.
use super::*;
use pce::models::{SeverityFilter, TypeFilter};

/// The severity column, the target of `S`.
pub(super) const SEVERITY_COL: usize = 1;

pub fn event_columns() -> Vec<Column<Event>> {
    vec![
        Column {
            title: "timestamp",
            width: 20,
            cell: |e| {
                e.timestamp
                    .as_deref()
                    .map(|t| t.get(..19).unwrap_or(t).replace('T', " "))
                    .unwrap_or_default()
            },
            key: |e| SortKey::Text(e.timestamp.clone().unwrap_or_default()),
            right: false,
            hide_below: 0,
            narrow: Some(|e| {
                e.timestamp
                    .as_deref()
                    .map(|t| t.get(5..19).unwrap_or(t).replace('T', " "))
                    .unwrap_or_default()
            }),
            narrow_width: 14,
            full_title: None,
            full: Some(|e| e.timestamp.clone().unwrap_or_default()),
        },
        Column {
            title: "severity",
            width: 8,
            cell: |e| e.severity.clone().unwrap_or_default(),
            key: |e| SortKey::Num(u64::from(e.severity_rank())),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: None,
        },
        Column {
            title: "type",
            width: 28,
            cell: |e| e.event_type.clone().unwrap_or_default(),
            key: |e| SortKey::Text(e.event_type.clone().unwrap_or_default()),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 24,
            full_title: None,
            full: None,
        },
        Column {
            title: "actor",
            width: 22,
            cell: |e| e.actor(),
            key: |e| SortKey::Text(e.actor()),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 20,
            full_title: None,
            full: None,
        },
        Column {
            title: "resource",
            width: 0,
            cell: |e| e.resource(),
            key: |e| SortKey::Text(e.resource()),
            right: false,
            // Secondary: only on wide terminals.
            hide_below: 120,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: Some(|e| join_list(e.resources())),
        },
        Column {
            title: "status",
            width: 8,
            cell: |e| e.status.clone().unwrap_or_default(),
            key: |e| SortKey::Text(e.status.clone().unwrap_or_default()),
            right: false,
            hide_below: 100,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: None,
        },
    ]
}

/// What the events editor holds: the severity (one is sent to the PCE,
/// `X+` filters locally), the local event-type terms and the window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventsFilterEdit {
    pub severity: String,
    pub event_type: String,
    pub window: String,
}

impl EventsFilterEdit {
    /// The request filter. `since` is rounded down to the whole minute and
    /// `until` is left open, so applying the same filter again within the cache
    /// TTL produces the same query (and a cache hit) instead of a new one every second.
    /// Checks the local parts too, so a bad type term is never applied.
    pub fn to_filter(&self, now: jiff::Timestamp) -> Result<EventFilter, String> {
        let severity = SeverityFilter::parse(&self.severity)?;
        TypeFilter::parse(&self.event_type).map_err(|e| format!("type: {e}"))?;
        let w = pce::traffic::parse_window(&self.window).map_err(|e| format!("window: {e}"))?;
        let span = jiff::Span::try_from(w).map_err(|e| format!("window: {e}"))?;
        let minute = jiff::Timestamp::from_second(now.as_second().div_euclid(60) * 60)
            .map_err(|e| e.to_string())?;
        let since = minute
            .checked_sub(span)
            .map_err(|e| format!("window: {e}"))?;
        Ok(EventFilter {
            severity: severity.pce_value(),
            event_type: None,
            status: None,
            since: Some(since),
            until: None,
            max_results: None,
        })
    }

    /// What the PCE is asked: the severity it is sent and the window. Two
    /// filters alike in these need no new request to switch between.
    pub fn pce_side(&self) -> (Option<String>, String) {
        let severity = SeverityFilter::parse(&self.severity)
            .ok()
            .and_then(|s| s.pce_value());
        (severity, self.window.trim().to_string())
    }
}

/// Type-filter text with `ty` hidden: the terms naming it (`ty`, `!ty`)
/// dropped and `!ty` added at the end.
pub fn hide_type(text: &str, ty: &str) -> String {
    let mut terms = other_terms(text, ty);
    terms.push(format!("!{ty}"));
    terms.join(", ")
}

/// Type-filter text showing only `ty`: the includes replaced by `ty`, and
/// the excludes that would hide it dropped.
pub fn only_type(text: &str, ty: &str) -> String {
    let mut terms: Vec<String> = other_terms(text, ty)
        .into_iter()
        .filter(|t| {
            t.strip_prefix('!').is_some_and(|ex| {
                TypeFilter::parse(&format!("!{ex}")).is_ok_and(|f| f.keeps_type(ty))
            })
        })
        .collect();
    terms.insert(0, ty.to_string());
    terms.join(", ")
}

/// The terms of `text` (trimmed, empty ones dropped) except those naming
/// `ty` itself, included or excluded.
fn other_terms(text: &str, ty: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .filter(|t| {
            let bare = t.strip_prefix('!').unwrap_or(t).trim_start();
            !bare.eq_ignore_ascii_case(ty)
        })
        .map(String::from)
        .collect()
}

#[derive(Debug)]
pub struct EventsState {
    pub table: TableView<Event>,
    /// The events as fetched; `table` is the local type and severity
    /// projection of these.
    pub raw: Vec<Event>,
    pub filter: EventsFilterEdit,
    /// The `since` and `until` of the last answered request (`until` is empty when open).
    pub window: Option<(String, String)>,
    pub loaded_at: Option<Instant>,
    pub age_secs: u64,
    pub loading: bool,
    /// The PCE returned a full page: more events may exist than were loaded.
    pub truncated: bool,
    /// Distinct event types in `raw` the type filter hides.
    pub hidden_types: usize,
    pub auto: AutoRefresh,
}

impl EventsState {
    /// `type_filter` is the Type field's starting text (the profile's
    /// `events.exclude_types` as `!` terms).
    pub fn new(min_refresh: u64, type_filter: String) -> Self {
        Self {
            table: TableView::new(event_columns()),
            raw: Vec::new(),
            filter: EventsFilterEdit {
                severity: String::new(),
                event_type: type_filter,
                window: "24h".into(),
            },
            window: None,
            loaded_at: None,
            age_secs: 0,
            loading: false,
            truncated: false,
            hidden_types: 0,
            auto: AutoRefresh::new(min_refresh),
        }
    }

    /// Applies the local filters: the type terms and a `X+` severity
    /// floor. Text that does not parse (never applied) keeps everything.
    pub fn rebuild(&mut self) {
        let types = TypeFilter::parse(&self.filter.event_type).unwrap_or_default();
        let severity = SeverityFilter::parse(&self.filter.severity).unwrap_or_default();
        let mut hidden: Vec<&str> = Vec::new();
        let mut rows: Vec<Event> = Vec::new();
        for e in &self.raw {
            let ty = e.event_type.as_deref().unwrap_or("");
            if !types.keeps_type(ty) {
                if !hidden.contains(&ty) {
                    hidden.push(ty);
                }
            } else if severity.keeps(e) {
                rows.push(e.clone());
            }
        }
        self.hidden_types = hidden.len();
        self.table.set_rows(rows);
    }

    /// Every event type in the load with its count, most frequent first
    /// (then by name); events without a type count under `""`.
    pub fn type_counts(&self) -> Vec<(String, usize)> {
        let mut counts: Vec<(String, usize)> = Vec::new();
        for e in &self.raw {
            let ty = e.event_type.as_deref().unwrap_or("");
            match counts.iter_mut().find(|(t, _)| t == ty) {
                Some((_, n)) => *n += 1,
                None => counts.push((ty.to_string(), 1)),
            }
        }
        counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        counts
    }

    /// `x`: hides the type `ty` (no request).
    pub fn hide_type(&mut self, ty: &str) {
        self.filter.event_type = hide_type(&self.filter.event_type, ty);
        self.rebuild();
    }

    /// `o`: shows only the type `ty` (no request).
    pub fn only_type(&mut self, ty: &str) {
        self.filter.event_type = only_type(&self.filter.event_type, ty);
        self.rebuild();
    }

    /// `S`: worst severity first; pressing it again flips the direction.
    pub fn sort_by_severity(&mut self) {
        let desc = self.table.sort() != Some((SEVERITY_COL, true));
        self.table.sort_by(SEVERITY_COL, desc);
    }
}
