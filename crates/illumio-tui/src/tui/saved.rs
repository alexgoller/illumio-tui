//! Saved filters in the TUI (`:save`, `:filters`, `:load`): the filter in
//! force on a screen as the config file stores it and back, the `[name]`
//! tag while an applied filter is unchanged, the `:filters` picker, and
//! each screen's default (`*` in the picker, `:default`).
//! Pure but for [`SavedState::store`]'s file reads; applying goes through
//! the screens' own `f` paths in `update`.
use crate::filters::{
    EventsSaved, Saved, SavedFilters, Screen as FilterScreen, Store, TrafficSaved, WorkloadsSaved,
};
use crate::tui::filter::{FilterEditor, TrafficFilter};
use crate::tui::model::{EventsFilterEdit, Screen, WorkloadFilters};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::PathBuf;

/// The screen's saved filters, if it has them.
pub fn filter_screen(s: Screen) -> Option<FilterScreen> {
    match s {
        Screen::Traffic => Some(FilterScreen::Traffic),
        Screen::Policy => Some(FilterScreen::Policy),
        Screen::Events => Some(FilterScreen::Events),
        Screen::Workloads => Some(FilterScreen::Workloads),
        Screen::Objects => None,
    }
}

/// The filters applied from a saved one, by name, each with the filter it
/// put in force: the screen shows `[name]` while that is still the one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Applied {
    pub traffic: Option<(String, TrafficFilter)>,
    /// The search's text (`RuleFilter`'s `Display`).
    pub policy: Option<(String, String)>,
    pub events: Option<(String, EventsFilterEdit)>,
    /// The local filters and the label filter's text.
    pub workloads: Option<(String, (WorkloadFilters, String))>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedState {
    /// The config file, or why the session has no saved filters (an
    /// `--env-file` connection; tests until they set one).
    pub path: Result<PathBuf, String>,
    pub applied: Applied,
    /// Why a default filter was ignored, said again once the names load
    /// (the connection's own messages replace the status line before).
    pub warning: Option<String>,
}

impl Default for SavedState {
    fn default() -> Self {
        Self {
            path: Err("no config file".into()),
            applied: Applied::default(),
            warning: None,
        }
    }
}

impl SavedState {
    /// The current profile's store: the config file's profile of the same
    /// name, so after `:profile X` the filters are X's.
    pub fn store(&self, profile: &str) -> Result<Store, String> {
        match &self.path {
            Ok(p) => Ok(Store::new(p.clone(), profile)),
            Err(e) => Err(format!("saved filters: {e}")),
        }
    }

    /// The profile's filters, read from the file now.
    pub fn load(&self, profile: &str) -> Result<SavedFilters, String> {
        self.store(profile)?.load().map_err(|e| e.to_string())
    }
}

/// The name of the saved filter `screen` shows while the filter it put in
/// force is unchanged; an edit drops it.
pub fn tag(model: &crate::tui::Model, screen: Screen) -> Option<&str> {
    let a = &model.saved.applied;
    match screen {
        Screen::Traffic => a
            .traffic
            .as_ref()
            .filter(|(_, f)| *f == model.traffic.filter)
            .map(|(n, _)| n.as_str()),
        Screen::Policy => a
            .policy
            .as_ref()
            .filter(|(_, q)| {
                Some(q)
                    == model
                        .policy
                        .search
                        .as_ref()
                        .map(ToString::to_string)
                        .as_ref()
            })
            .map(|(n, _)| n.as_str()),
        Screen::Events => a
            .events
            .as_ref()
            .filter(|(_, e)| *e == model.events.filter)
            .map(|(n, _)| n.as_str()),
        Screen::Workloads => a
            .workloads
            .as_ref()
            .filter(|(_, (f, l))| {
                *f == model.workloads.filters && *l == model.workloads.labels.to_string()
            })
            .map(|(n, _)| n.as_str()),
        Screen::Objects => None,
    }
}

/// The filter in force on `screen` as it would be saved; `None` on a
/// screen without saved filters, or Policy without a search.
pub fn current(model: &crate::tui::Model, screen: Screen) -> Option<Saved> {
    Some(match screen {
        Screen::Traffic => Saved::Traffic(traffic_saved(&model.traffic.filter)),
        Screen::Policy => Saved::Policy(model.policy.search.as_ref()?.to_string()),
        Screen::Events => Saved::Events(events_saved(&model.events.filter)),
        Screen::Workloads => Saved::Workloads(workloads_saved(
            &model.workloads.filters,
            &model.workloads.labels.to_string(),
        )),
        Screen::Objects => return None,
    })
}

/// Records `name` as applied on `screen` with the filter in force now.
pub fn mark_applied(model: &mut crate::tui::Model, screen: Screen, name: &str) {
    let name = name.to_string();
    let a = &mut model.saved.applied;
    match screen {
        Screen::Traffic => a.traffic = Some((name, model.traffic.filter.clone())),
        Screen::Policy => a.policy = model.policy.search.as_ref().map(|f| (name, f.to_string())),
        Screen::Events => a.events = Some((name, model.events.filter.clone())),
        Screen::Workloads => {
            a.workloads = Some((
                name,
                (
                    // As saved: a message filter is not part of it.
                    WorkloadFilters {
                        message: None,
                        ..model.workloads.filters.clone()
                    },
                    model.workloads.labels.to_string(),
                ),
            ))
        }
        Screen::Objects => {}
    }
}

/// The traffic filter as saved: every field set but the empty ones (the
/// window and the cap always).
pub fn traffic_saved(f: &TrafficFilter) -> TrafficSaved {
    let text = |s: &str| Some(s.trim().to_string()).filter(|s| !s.is_empty());
    TrafficSaved {
        src: text(&f.src),
        dst: text(&f.dst),
        service: text(&f.service),
        decisions: (f.decisions_text() != "all")
            .then(|| f.decisions.iter().map(|d| d.as_str().to_string()).collect()),
        transmission: f.has_transmission().then(|| {
            f.transmission
                .iter()
                .map(|t| t.as_str().to_string())
                .collect()
        }),
        window: Some(f.window.clone()),
        max_results: Some(f.max_results),
    }
}

/// The saved traffic filter as the `f` editor's Enter reads it: its
/// fields typed into the editor (a missing one empty, `all`), the window
/// and the cap `current`'s unless it sets them; checked the same way.
pub fn traffic_filter(s: &TrafficSaved, current: &TrafficFilter) -> Result<TrafficFilter, String> {
    let mut e = FilterEditor::from_filter(&TrafficFilter::defaults(
        &current.window,
        current.max_results,
    ));
    let list = |l: &Option<Vec<String>>| match l {
        Some(l) if !l.is_empty() => l.join(","),
        _ => "all".to_string(),
    };
    e.values = [
        s.window.clone().unwrap_or_else(|| current.window.clone()),
        s.src.clone().unwrap_or_default(),
        s.dst.clone().unwrap_or_default(),
        s.service.clone().unwrap_or_default(),
        list(&s.decisions),
        list(&s.transmission),
        s.max_results.unwrap_or(current.max_results).to_string(),
    ];
    e.to_filter()
}

pub fn events_saved(e: &EventsFilterEdit) -> EventsSaved {
    let text = |s: &str| Some(s.trim().to_string()).filter(|s| !s.is_empty());
    EventsSaved {
        event_type: text(&e.event_type),
        severity: text(&e.severity),
        window: text(&e.window),
    }
}

/// The saved events filter as the editor holds it; a missing field is the
/// default: any severity, 24h. `types` (the profile's hidden types, `!a,
/// !b`) apply on top of the saved type terms: each one the text lacks is
/// added after them.
pub fn events_edit(s: &EventsSaved, types: &str) -> EventsFilterEdit {
    EventsFilterEdit {
        severity: s.severity.clone().unwrap_or_default().trim().to_string(),
        event_type: with_terms(s.event_type.as_deref().unwrap_or(""), types),
        window: s
            .window
            .clone()
            .unwrap_or_else(|| "24h".into())
            .trim()
            .to_string(),
    }
}

/// `text`'s `,` terms, then each term of `extra` it does not hold.
fn with_terms(text: &str, extra: &str) -> String {
    let terms = |t: &str| -> Vec<String> {
        t.split(',')
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(String::from)
            .collect()
    };
    let mut out = terms(text);
    for t in terms(extra) {
        if !out.contains(&t) {
            out.push(t);
        }
    }
    out.join(", ")
}

pub fn workloads_saved(f: &WorkloadFilters, labels: &str) -> WorkloadsSaved {
    WorkloadsSaved {
        labels: Some(labels.trim().to_string()).filter(|l| !l.is_empty()),
        enforcement: f.enforcement.map(String::from),
        managed: f.managed,
        offline: f.offline.then_some(true),
        unlabeled: f.unlabeled.then_some(true),
        health: Some(f.health.names()).filter(|h| !h.is_empty()),
        ven_version: f.ven_version.clone(),
    }
}

/// The saved workloads filter's local filters and label text; a missing
/// field is off.
pub fn workload_filters(s: &WorkloadsSaved) -> (WorkloadFilters, String) {
    let enforcement = s.enforcement.as_deref().and_then(|e| {
        crate::filters::ENFORCEMENTS
            .iter()
            .copied()
            .find(|k| *k == e.trim())
    });
    (
        WorkloadFilters {
            enforcement,
            managed: s.managed,
            offline: s.offline.unwrap_or(false),
            unlabeled: s.unlabeled.unwrap_or(false),
            // Parsed already: names only the known states.
            health: s
                .health
                .as_deref()
                .and_then(|h| pce::ven::HealthSet::from_names(h).ok())
                .unwrap_or_default(),
            ven_version: s.ven_version.clone(),
            message: None,
        },
        s.labels.clone().unwrap_or_default().trim().to_string(),
    )
}

/// One picker row: the name and its summary (or why it does not parse).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerEntry {
    pub name: String,
    pub summary: String,
}

/// `:filters`: the current screen's saved filters, the default marked
/// `*`. Typing narrows (any case), `↑`/`↓` move, `Enter` applies, `*`
/// makes the selected one the default (or no longer), `D` deletes (after
/// a confirm), `Esc` closes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterPicker {
    pub screen: FilterScreen,
    pub input: String,
    pub entries: Vec<PickerEntry>,
    /// The screen's default filter.
    pub default: Option<String>,
    /// Index into `matches()`.
    pub selected: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerOutcome {
    Open(FilterPicker),
    Close,
    Apply(String),
    Delete(FilterPicker, String),
    /// `*`: the default toggled on this filter.
    Default(FilterPicker, String),
}

impl FilterPicker {
    pub fn new(screen: FilterScreen, filters: &SavedFilters, default: Option<&str>) -> Self {
        let entries = filters
            .names(screen)
            .into_iter()
            .map(|name| {
                let summary = match filters.get(screen, &name) {
                    Ok(s) => s.summary(),
                    Err(e) => format!("does not parse: {e}"),
                };
                PickerEntry { name, summary }
            })
            .collect();
        Self {
            screen,
            input: String::new(),
            entries,
            default: default.map(String::from),
            selected: 0,
        }
    }

    /// The picker reopened on the text typed before.
    pub fn with_input(mut self, input: &str) -> Self {
        self.input = input.to_string();
        self.selected = 0;
        self
    }

    /// The entries whose name or summary holds the typed text, any case.
    pub fn matches(&self) -> Vec<&PickerEntry> {
        let needle = self.input.trim().to_lowercase();
        self.entries
            .iter()
            .filter(|e| {
                e.name.to_lowercase().contains(&needle)
                    || e.summary.to_lowercase().contains(&needle)
            })
            .collect()
    }

    fn selected_name(&self) -> Option<String> {
        self.matches().get(self.selected).map(|e| e.name.clone())
    }

    pub fn key(mut self, k: KeyEvent) -> PickerOutcome {
        match k.code {
            KeyCode::Esc => PickerOutcome::Close,
            KeyCode::Enter => match self.selected_name() {
                Some(name) => PickerOutcome::Apply(name),
                None => PickerOutcome::Open(self),
            },
            // Typing narrows any case, so a capital D is free for delete.
            KeyCode::Char('D') => match self.selected_name() {
                Some(name) => PickerOutcome::Delete(self, name),
                None => PickerOutcome::Open(self),
            },
            // No name holds a `*`, so it is free too.
            KeyCode::Char('*') => match self.selected_name() {
                Some(name) => PickerOutcome::Default(self, name),
                None => PickerOutcome::Open(self),
            },
            KeyCode::Down | KeyCode::Up => {
                let n = self.matches().len();
                if n > 0 {
                    self.selected = if k.code == KeyCode::Down {
                        (self.selected + 1) % n
                    } else {
                        (self.selected + n - 1) % n
                    };
                }
                PickerOutcome::Open(self)
            }
            KeyCode::Backspace => {
                self.input.pop();
                self.selected = 0;
                PickerOutcome::Open(self)
            }
            KeyCode::Char('u') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                self.input.clear();
                self.selected = 0;
                PickerOutcome::Open(self)
            }
            KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) => {
                self.input.push(c);
                self.selected = 0;
                PickerOutcome::Open(self)
            }
            _ => PickerOutcome::Open(self),
        }
    }
}

/// `:load <Tab>`: the names of `names` starting with `typed`, completed to
/// their longest common start (`None` when nothing starts so).
pub fn complete_name(names: &[String], typed: &str) -> Option<String> {
    let hits: Vec<&String> = names.iter().filter(|n| n.starts_with(typed)).collect();
    let first = hits.first()?;
    let mut end = first.len();
    for h in &hits[1..] {
        end = first
            .char_indices()
            .zip(h.chars())
            .take_while(|((_, a), b)| a == b)
            .last()
            .map_or(0, |((i, a), _)| i + a.len_utf8())
            .min(end);
    }
    Some(first[..end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pce::models::{PolicyDecision, Transmission};

    fn k(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn filters() -> SavedFilters {
        toml::from_str(
            "[traffic.web]\nsrc = \"app=web\"\n[traffic.Db]\ndst = \"app=db\"\n\
             [traffic.broken]\nsrc = 1\n",
        )
        .unwrap()
    }

    #[test]
    fn a_traffic_filter_round_trips_and_keeps_the_window_and_cap_it_lacks() {
        let mut f = TrafficFilter::defaults("24h", 1000);
        f.src = "app=web,env=prod | ipl:Corp".into();
        f.service = "!53/udp, 8000-8100/tcp".into();
        f.decisions = vec![PolicyDecision::Blocked];
        f.transmission = vec![Transmission::Unicast];
        let s = traffic_saved(&f);
        assert_eq!(s.decisions, Some(vec!["blocked".into()]));
        assert_eq!(s.dst, None);
        assert_eq!(
            traffic_filter(&s, &TrafficFilter::defaults("1h", 5)).unwrap(),
            f
        );
        let bare = TrafficSaved {
            dst: Some("app=db".into()),
            ..Default::default()
        };
        let now = TrafficFilter::defaults("7d", 500);
        let g = traffic_filter(&bare, &now).unwrap();
        assert_eq!((g.window.as_str(), g.max_results), ("7d", 500));
        assert_eq!(g.decisions_text(), "all");
        let bad = TrafficSaved {
            src: Some("fqdn:x.example".into()),
            ..Default::default()
        };
        assert!(traffic_filter(&bad, &now).unwrap_err().contains("source"));
    }

    #[test]
    fn events_and_workloads_round_trip_and_default_what_is_missing() {
        let e = EventsFilterEdit {
            severity: "warning+".into(),
            event_type: "!agent.*".into(),
            window: "7d".into(),
        };
        assert_eq!(events_edit(&events_saved(&e), "!agent.*"), e);
        // The profile's hidden types apply on top, each once.
        assert_eq!(
            events_edit(&events_saved(&e), "!agent.*, !=user.login").event_type,
            "!agent.*, !=user.login"
        );
        assert_eq!(events_edit(&events_saved(&e), "").event_type, "!agent.*");
        let d = events_edit(&EventsSaved::default(), "!agent.*");
        assert_eq!(
            (
                d.severity.as_str(),
                d.event_type.as_str(),
                d.window.as_str()
            ),
            ("", "!agent.*", "24h")
        );
        let w = WorkloadFilters {
            enforcement: Some("idle"),
            managed: Some(false),
            offline: true,
            unlabeled: false,
            health: pce::ven::HealthSet::parse("error,warning").unwrap(),
            ven_version: Some("23.2.*".into()),
            message: None,
        };
        let s = workloads_saved(&w, "env=prod !app=x");
        assert_eq!(s.unlabeled, None);
        assert_eq!(
            s.health.as_deref(),
            Some(&["warning".to_string(), "error".to_string()][..])
        );
        assert_eq!(workload_filters(&s), (w.clone(), "env=prod !app=x".into()));
        // A message filter is a look from `:ven`, never saved.
        let with_message = WorkloadFilters {
            message: Some("policy_sync: x".into()),
            ..w.clone()
        };
        assert_eq!(workloads_saved(&with_message, "env=prod !app=x"), s);
    }

    #[test]
    fn the_picker_narrows_any_case_applies_and_asks_to_delete() {
        let p = FilterPicker::new(FilterScreen::Traffic, &filters(), Some("web"));
        assert_eq!(p.entries.len(), 3);
        assert_eq!(p.default.as_deref(), Some("web"));
        assert!(
            p.entries[1].summary.contains("does not parse"),
            "{:?}",
            p.entries
        );
        let p = match p.key(k(KeyCode::Char('d'))) {
            PickerOutcome::Open(p) => p,
            o => panic!("{o:?}"),
        };
        let p = match p.key(k(KeyCode::Char('b'))) {
            PickerOutcome::Open(p) => p,
            o => panic!("{o:?}"),
        };
        assert_eq!(
            p.matches()
                .iter()
                .map(|e| e.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Db"],
            "names and summaries, any case"
        );
        assert_eq!(
            p.clone().key(k(KeyCode::Enter)),
            PickerOutcome::Apply("Db".into())
        );
        match p
            .clone()
            .key(KeyEvent::new(KeyCode::Char('D'), KeyModifiers::SHIFT))
        {
            PickerOutcome::Delete(_, name) => assert_eq!(name, "Db"),
            o => panic!("{o:?}"),
        }
        match p.key(k(KeyCode::Char('*'))) {
            PickerOutcome::Default(p, name) => {
                assert_eq!((name.as_str(), p.input.as_str()), ("Db", "db"))
            }
            o => panic!("{o:?}"),
        }
        let p = FilterPicker::new(FilterScreen::Traffic, &filters(), None).key(k(KeyCode::Esc));
        assert_eq!(p, PickerOutcome::Close);
    }

    #[test]
    fn names_complete_to_their_common_start() {
        let names = vec!["prod-a".to_string(), "prod-b".into(), "lab".into()];
        assert_eq!(complete_name(&names, "pr").as_deref(), Some("prod-"));
        assert_eq!(complete_name(&names, "l").as_deref(), Some("lab"));
        assert_eq!(complete_name(&names, "x"), None);
    }
}
