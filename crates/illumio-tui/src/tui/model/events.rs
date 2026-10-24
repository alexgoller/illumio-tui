//! The Events screen's state: its table columns, the filter being edited
//! and the loaded events.
use super::*;
use pce::models::{EventActorFilter, SeverityFilter, TermText, TypeFilter, actor_names};

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
/// `X+` filters locally), the local event-type and actor terms and the
/// window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventsFilterEdit {
    pub severity: String,
    pub event_type: String,
    pub actor: String,
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
        EventActorFilter::parse(&self.actor).map_err(|e| format!("actor: {e}"))?;
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

/// Type-filter text with `ty` hidden: the excludes naming it dropped and
/// `!=ty` (exact) added at the end. The includes stay, so hiding the type
/// that is the include empties the view instead of widening it.
pub fn hide_type(text: &str, ty: &str) -> String {
    hide_term(text, ty, |b| b.eq_ignore_ascii_case(ty))
}

/// Type-filter text showing only `ty` (`=ty`, exact): the includes
/// replaced, and the excludes that would hide it dropped.
pub fn only_type(text: &str, ty: &str) -> String {
    only_term(
        text,
        ty,
        |b| b.eq_ignore_ascii_case(ty),
        |t| TypeFilter::parse(t).is_ok_and(|f| f.keeps_type(ty)),
    )
}

/// Actor-filter text with `actor` hidden, as [`hide_type`]: a term naming
/// it by its whole string or by the name after its kind counts as naming it.
pub fn hide_actor(text: &str, actor: &str) -> String {
    hide_term(text, actor, |b| names_actor(b, actor))
}

/// Actor-filter text showing only `actor` (`=actor`), as [`only_type`].
pub fn only_actor(text: &str, actor: &str) -> String {
    only_term(
        text,
        actor,
        |b| names_actor(b, actor),
        |t| EventActorFilter::parse(t).is_ok_and(|f| f.keeps_actor(actor)),
    )
}

/// `body` is one of the names `actor` answers to, any case.
fn names_actor(body: &str, actor: &str) -> bool {
    actor_names(actor)
        .iter()
        .any(|n| n.eq_ignore_ascii_case(body))
}

/// `text` with `value` hidden: the excludes `names` (true for a term body
/// naming `value`) dropped, `!=value` added at the end.
fn hide_term(text: &str, value: &str, names: impl Fn(&str) -> bool) -> String {
    let mut terms = terms_except(text, &names, true);
    terms.push(format!("!={value}"));
    terms.join(", ")
}

/// `text` showing only `value`: `=value` first, then the excludes that
/// `keeps` (true for a term that keeps `value`).
fn only_term(
    text: &str,
    value: &str,
    names: impl Fn(&str) -> bool,
    keeps: impl Fn(&str) -> bool,
) -> String {
    let mut terms: Vec<String> = terms_except(text, &names, false)
        .into_iter()
        .filter(|t| TermText::parse(t).is_ok_and(|p| p.exclude) && keeps(t))
        .collect();
    terms.insert(0, format!("={value}"));
    terms.join(", ")
}

/// The terms of `text` (trimmed, empty ones dropped) except those whose
/// body `names` the value itself (`ty`, `=ty`, `! = ty`, any case); with
/// `excludes_only` only the excluding ones are dropped.
fn terms_except(text: &str, names: &dyn Fn(&str) -> bool, excludes_only: bool) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .filter(|t| match TermText::parse(t) {
            Ok(p) => !(names(&p.body) && (p.exclude || !excludes_only)),
            Err(_) => true,
        })
        .map(String::from)
        .collect()
}

#[derive(Debug)]
pub struct EventsState {
    pub table: TableView<Event>,
    /// The events as fetched; `table` is the local type, actor and
    /// severity projection of these.
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
    /// What the PCE was asked (`EventsFilterEdit::pce_side`) by the
    /// request in flight or last sent, and by the last answered one: a
    /// filter edit refilters without a request only when the loaded
    /// events answered the same.
    pub requested: Option<(Option<String>, String)>,
    pub answered: Option<(Option<String>, String)>,
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
                actor: String::new(),
                window: "24h".into(),
            },
            window: None,
            loaded_at: None,
            age_secs: 0,
            loading: false,
            truncated: false,
            hidden_types: 0,
            requested: None,
            answered: None,
            auto: AutoRefresh::new(min_refresh),
        }
    }

    /// Applies the local filters: the type and actor terms and a `X+`
    /// severity floor. Text that does not parse (never applied) keeps
    /// everything.
    pub fn rebuild(&mut self) {
        let types = TypeFilter::parse(&self.filter.event_type).unwrap_or_default();
        let actors = self.actor_filter();
        let severity = SeverityFilter::parse(&self.filter.severity).unwrap_or_default();
        let mut hidden: Vec<&str> = Vec::new();
        let mut rows: Vec<Event> = Vec::new();
        for e in &self.raw {
            let ty = e.event_type.as_deref().unwrap_or("");
            if !types.keeps_type(ty) {
                if !hidden.contains(&ty) {
                    hidden.push(ty);
                }
            } else if severity.keeps(e) && actors.keeps(e) {
                rows.push(e.clone());
            }
        }
        self.hidden_types = hidden.len();
        self.table.set_rows(rows);
    }

    /// The actor filter in force; text that does not parse keeps all.
    pub fn actor_filter(&self) -> EventActorFilter {
        EventActorFilter::parse(&self.filter.actor).unwrap_or_default()
    }

    /// Every event type in the load with its count, most frequent first
    /// (then by name); events without a type count under `""`.
    pub fn type_counts(&self) -> Vec<(String, usize)> {
        counts(
            self.raw
                .iter()
                .map(|e| e.event_type.clone().unwrap_or_default()),
        )
    }

    /// Every actor in the load (`Event::actor`) with its count, as
    /// [`type_counts`](Self::type_counts).
    pub fn actor_counts(&self) -> Vec<(String, usize)> {
        counts(self.raw.iter().map(Event::actor))
    }

    /// The values of the `t` overlay's view (actors with `by_actor`, else
    /// types) their own filter keeps but whose every event another local
    /// filter hides, each with the filter to blame: `type`, `actor` or
    /// `severity` (the first that hides all of them), else `filters`.
    pub fn hidden_elsewhere(&self, by_actor: bool) -> Vec<(String, &'static str)> {
        let types = TypeFilter::parse(&self.filter.event_type).unwrap_or_default();
        let actors = self.actor_filter();
        let severity = SeverityFilter::parse(&self.filter.severity).unwrap_or_default();
        // Per value: whether an event passes all filters, and each of
        // type, actor and severity.
        let mut seen: std::collections::HashMap<String, (bool, [bool; 3])> =
            std::collections::HashMap::new();
        for e in &self.raw {
            let actor = e.actor();
            let ty = e.event_type.as_deref().unwrap_or("");
            let pass = [
                types.keeps_type(ty),
                actors.keeps_actor(&actor),
                severity.keeps(e),
            ];
            let own = if by_actor { 1 } else { 0 };
            if !pass[own] {
                continue;
            }
            let key = if by_actor { actor } else { ty.to_string() };
            let entry = seen.entry(key).or_insert((false, [false; 3]));
            entry.0 |= pass.iter().all(|p| *p);
            for (any, p) in entry.1.iter_mut().zip(pass) {
                *any |= p;
            }
        }
        let mut out: Vec<(String, &'static str)> = seen
            .into_iter()
            .filter(|(_, (all, _))| !all)
            .map(|(v, (_, any))| {
                let why = ["type", "actor", "severity"]
                    .into_iter()
                    .zip(any)
                    .find(|(_, a)| !a)
                    .map_or("filters", |(w, _)| w);
                (v, why)
            })
            .collect();
        out.sort();
        out
    }

    /// The window of the events shown: the last answered request's, else
    /// the filter's.
    pub fn shown_window(&self) -> &str {
        self.answered
            .as_ref()
            .map_or(self.filter.window.as_str(), |(_, w)| w.as_str())
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

    /// `U`: hides the actor `actor` (no request).
    pub fn hide_actor(&mut self, actor: &str) {
        self.filter.actor = hide_actor(&self.filter.actor, actor);
        self.rebuild();
    }

    /// `u`: shows only the actor `actor` (no request).
    pub fn only_actor(&mut self, actor: &str) {
        self.filter.actor = only_actor(&self.filter.actor, actor);
        self.rebuild();
    }

    /// `S`: worst severity first; pressing it again flips the direction.
    pub fn sort_by_severity(&mut self) {
        let desc = self.table.sort() != Some((SEVERITY_COL, true));
        self.table.sort_by(SEVERITY_COL, desc);
    }
}

/// Each distinct value with its count, most frequent first, then by name.
fn counts(values: impl Iterator<Item = String>) -> Vec<(String, usize)> {
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for v in values {
        *counts.entry(v).or_default() += 1;
    }
    let mut counts: Vec<(String, usize)> = counts.into_iter().collect();
    counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    counts
}
