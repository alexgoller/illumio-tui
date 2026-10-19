//! The Events screen's state: its table columns, the filter being edited
//! and the loaded events.
use super::*;

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

/// What the events editor holds: the server-side severity and window, and the
/// local `type` substring.
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
    pub fn to_filter(&self, now: jiff::Timestamp) -> Result<EventFilter, String> {
        let w = pce::traffic::parse_window(&self.window).map_err(|e| format!("window: {e}"))?;
        let span = jiff::Span::try_from(w).map_err(|e| format!("window: {e}"))?;
        let minute = jiff::Timestamp::from_second(now.as_second().div_euclid(60) * 60)
            .map_err(|e| e.to_string())?;
        let since = minute
            .checked_sub(span)
            .map_err(|e| format!("window: {e}"))?;
        Ok(EventFilter {
            severity: Some(self.severity.trim().to_string()).filter(|s| !s.is_empty()),
            event_type: None,
            status: None,
            since: Some(since),
            until: None,
            max_results: None,
        })
    }
}

#[derive(Debug)]
pub struct EventsState {
    pub table: TableView<Event>,
    /// The events as fetched; `table` is the local `type` projection of these.
    pub raw: Vec<Event>,
    pub filter: EventsFilterEdit,
    /// The `since` and `until` of the last answered request (`until` is empty when open).
    pub window: Option<(String, String)>,
    pub loaded_at: Option<Instant>,
    pub age_secs: u64,
    pub loading: bool,
    pub auto: AutoRefresh,
}

impl EventsState {
    pub fn new(min_refresh: u64) -> Self {
        Self {
            table: TableView::new(event_columns()),
            raw: Vec::new(),
            filter: EventsFilterEdit {
                severity: String::new(),
                event_type: String::new(),
                window: "24h".into(),
            },
            window: None,
            loaded_at: None,
            age_secs: 0,
            loading: false,
            auto: AutoRefresh::new(min_refresh),
        }
    }

    /// Applies the local type substring filter.
    pub fn rebuild(&mut self) {
        let needle = self.filter.event_type.trim().to_ascii_lowercase();
        let rows: Vec<Event> = self
            .raw
            .iter()
            .filter(|e| {
                needle.is_empty()
                    || e.event_type
                        .as_deref()
                        .unwrap_or("")
                        .to_ascii_lowercase()
                        .contains(&needle)
            })
            .cloned()
            .collect();
        self.table.set_rows(rows);
    }

    /// `S`: worst severity first; pressing it again flips the direction.
    pub fn sort_by_severity(&mut self) {
        let desc = self.table.sort() != Some((SEVERITY_COL, true));
        self.table.sort_by(SEVERITY_COL, desc);
    }
}
