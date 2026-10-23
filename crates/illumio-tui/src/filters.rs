//! Saved filters: named filters per profile in the config file
//! (`[profiles.<name>.filters.<screen>.<filter>]`) for the TUI's Traffic,
//! Policy, Events and Workloads screens and the CLI commands that match
//! them. Fields hold the TUI editor's text unchanged (`a,b | c, !d`); a
//! policy filter is one `pce::RuleFilter` query. They are kept raw, so one
//! that does not parse fails only where it is used, naming the profile,
//! screen, filter and field. Local files only: nothing here contacts the
//! PCE. Also the traffic filter text's grammar, which the CLI shares.
use crate::config::{self, ConfigError, ConfigFile};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The screens (and commands) that have saved filters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Screen {
    Traffic,
    Policy,
    Events,
    Workloads,
}

impl Screen {
    pub const ALL: [Screen; 4] = [
        Screen::Traffic,
        Screen::Policy,
        Screen::Events,
        Screen::Workloads,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Screen::Traffic => "traffic",
            Screen::Policy => "policy",
            Screen::Events => "events",
            Screen::Workloads => "workloads",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == s.trim())
    }
}

impl std::fmt::Display for Screen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Longest filter name.
pub const MAX_NAME: usize = 64;

/// A name is 1 to 64 of `A-Z a-z 0-9 . _ -` (case-sensitive).
pub fn check_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("a filter name is empty".into());
    }
    if name.chars().count() > MAX_NAME {
        return Err(format!(
            "filter name {name:?} is longer than {MAX_NAME} characters"
        ));
    }
    if let Some(c) = name
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')))
    {
        return Err(format!(
            "filter name {name:?}: {c:?} is not allowed (letters, digits, . _ -)"
        ));
    }
    Ok(())
}

/// A profile's saved filters as the file holds them, per screen and name;
/// each parsed only where it is used ([`Saved::parse`]).
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedFilters {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub traffic: BTreeMap<String, toml::Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub policy: BTreeMap<String, toml::Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub events: BTreeMap<String, toml::Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub workloads: BTreeMap<String, toml::Value>,
    /// Unknown keys, round-tripped verbatim.
    #[serde(default, flatten)]
    pub extra: toml::Table,
}

impl SavedFilters {
    pub fn is_empty(&self) -> bool {
        Screen::ALL.iter().all(|s| self.of(*s).is_empty()) && self.extra.is_empty()
    }

    pub fn of(&self, screen: Screen) -> &BTreeMap<String, toml::Value> {
        match screen {
            Screen::Traffic => &self.traffic,
            Screen::Policy => &self.policy,
            Screen::Events => &self.events,
            Screen::Workloads => &self.workloads,
        }
    }

    fn of_mut(&mut self, screen: Screen) -> &mut BTreeMap<String, toml::Value> {
        match screen {
            Screen::Traffic => &mut self.traffic,
            Screen::Policy => &mut self.policy,
            Screen::Events => &mut self.events,
            Screen::Workloads => &mut self.workloads,
        }
    }

    /// The screen's filter names, sorted.
    pub fn names(&self, screen: Screen) -> Vec<String> {
        self.of(screen).keys().cloned().collect()
    }

    /// The filter `name` of `screen`, parsed; the error names the screen,
    /// the filter and the field.
    pub fn get(&self, screen: Screen, name: &str) -> Result<Saved, String> {
        match self.of(screen).get(name) {
            Some(v) => Saved::parse(screen, v).map_err(|e| format!("{screen} filter {name}: {e}")),
            None => {
                let known = self.names(screen);
                Err(if known.is_empty() {
                    format!("no {screen} filter {name:?}; none saved")
                } else {
                    format!("no {screen} filter {name:?}; saved: {}", known.join(", "))
                })
            }
        }
    }
}

/// A traffic filter: the `f` editor's fields as typed. `window` and
/// `max_results` left out keep the ones in force.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct TrafficSaved {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub src: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dst: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decisions: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transmission: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<u32>,
}

/// An events filter: type terms, severity and window as typed in `f`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct EventsSaved {
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub event_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window: Option<String>,
}

/// A workloads filter: the label filter (`f`) and the local filters
/// (`E` `m` `o` `u`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct WorkloadsSaved {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub labels: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enforcement: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub managed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offline: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unlabeled: Option<bool>,
}

/// A stored filter's fields, read one by one so an error names its field.
struct Fields<'a>(&'a toml::Table);

impl Fields<'_> {
    /// No field but `known`.
    fn only(&self, known: &[&str]) -> Result<(), String> {
        match self.0.keys().find(|k| !known.contains(&k.as_str())) {
            Some(k) => Err(format!(
                "unknown field {k:?} (fields: {})",
                known.join(", ")
            )),
            None => Ok(()),
        }
    }

    fn wrong(k: &str, want: &str, v: &toml::Value) -> String {
        format!("field {k}: expected {want}, found a {}", v.type_str())
    }

    fn text(&self, k: &str) -> Result<Option<String>, String> {
        match self.0.get(k) {
            None => Ok(None),
            Some(toml::Value::String(s)) => Ok(Some(s.clone())),
            Some(v) => Err(Self::wrong(k, "a string", v)),
        }
    }

    /// A list of strings, or one string of comma-separated items.
    fn list(&self, k: &str) -> Result<Option<Vec<String>>, String> {
        match self.0.get(k) {
            None => Ok(None),
            Some(toml::Value::String(s)) => Ok(Some(
                s.split(',')
                    .map(str::trim)
                    .filter(|i| !i.is_empty())
                    .map(String::from)
                    .collect(),
            )),
            Some(toml::Value::Array(a)) => a
                .iter()
                .map(|i| match i {
                    toml::Value::String(s) => Ok(s.trim().to_string()),
                    v => Err(Self::wrong(k, "a list of strings", v)),
                })
                .collect::<Result<Vec<_>, _>>()
                .map(Some),
            Some(v) => Err(Self::wrong(k, "a list of strings", v)),
        }
    }

    fn count(&self, k: &str) -> Result<Option<u32>, String> {
        match self.0.get(k) {
            None => Ok(None),
            Some(toml::Value::Integer(n)) => u32::try_from(*n)
                .map(Some)
                .map_err(|_| format!("field {k}: {n} is out of range")),
            Some(v) => Err(Self::wrong(k, "a number", v)),
        }
    }

    fn flag(&self, k: &str) -> Result<Option<bool>, String> {
        match self.0.get(k) {
            None => Ok(None),
            Some(toml::Value::Boolean(b)) => Ok(Some(*b)),
            Some(v) => Err(Self::wrong(k, "true or false", v)),
        }
    }
}

/// The enforcement values a workloads filter takes (`unmanaged`: no VEN).
pub const ENFORCEMENTS: [&str; 5] = [
    "full",
    "selective",
    "visibility_only",
    "idle",
    pce::models::UNMANAGED,
];

/// One saved filter, parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Saved {
    Traffic(TrafficSaved),
    /// A rule search query (`dst:env=prod deny`).
    Policy(String),
    Events(EventsSaved),
    Workloads(WorkloadsSaved),
}

impl Saved {
    pub fn screen(&self) -> Screen {
        match self {
            Saved::Traffic(_) => Screen::Traffic,
            Saved::Policy(_) => Screen::Policy,
            Saved::Events(_) => Screen::Events,
            Saved::Workloads(_) => Screen::Workloads,
        }
    }

    /// The stored value as `screen`'s filter: the shape only (its fields
    /// and their types); what the text means is checked where it is
    /// applied. The error names the field.
    pub fn parse(screen: Screen, v: &toml::Value) -> Result<Self, String> {
        if screen == Screen::Policy {
            return match v {
                toml::Value::String(s) => Ok(Saved::Policy(s.clone())),
                other => Err(format!(
                    "a policy filter is a query string, not a {}",
                    other.type_str()
                )),
            };
        }
        let toml::Value::Table(t) = v else {
            return Err(format!(
                "expected a table of fields, found a {}",
                v.type_str()
            ));
        };
        let f = Fields(t);
        Ok(match screen {
            Screen::Traffic => {
                f.only(&[
                    "src",
                    "dst",
                    "service",
                    "decisions",
                    "transmission",
                    "window",
                    "max_results",
                ])?;
                Saved::Traffic(TrafficSaved {
                    src: f.text("src")?,
                    dst: f.text("dst")?,
                    service: f.text("service")?,
                    decisions: f.list("decisions")?,
                    transmission: f.list("transmission")?,
                    window: f.text("window")?,
                    max_results: f.count("max_results")?,
                })
            }
            Screen::Events => {
                f.only(&["type", "severity", "window"])?;
                Saved::Events(EventsSaved {
                    event_type: f.text("type")?,
                    severity: f.text("severity")?,
                    window: f.text("window")?,
                })
            }
            Screen::Workloads | Screen::Policy => {
                f.only(&["labels", "enforcement", "managed", "offline", "unlabeled"])?;
                let enforcement = f.text("enforcement")?;
                if let Some(e) = &enforcement
                    && !ENFORCEMENTS.contains(&e.trim())
                {
                    return Err(format!(
                        "field enforcement: {e:?} is not one of {}",
                        ENFORCEMENTS.join(", ")
                    ));
                }
                Saved::Workloads(WorkloadsSaved {
                    labels: f.text("labels")?,
                    enforcement: enforcement.map(|e| e.trim().to_string()),
                    managed: f.flag("managed")?,
                    offline: f.flag("offline")?,
                    unlabeled: f.flag("unlabeled")?,
                })
            }
        })
    }

    pub fn to_value(&self) -> toml::Value {
        let table = |r: Result<toml::Value, toml::ser::Error>| {
            r.unwrap_or_else(|_| toml::Value::Table(toml::Table::new()))
        };
        match self {
            Saved::Traffic(t) => table(toml::Value::try_from(t)),
            Saved::Policy(q) => toml::Value::String(q.clone()),
            Saved::Events(e) => table(toml::Value::try_from(e)),
            Saved::Workloads(w) => table(toml::Value::try_from(w)),
        }
    }

    /// One line: the fields set, in the editor's words (`src env=prod ·
    /// svc !53/udp · window 7d`); `(empty)` when none is.
    pub fn summary(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        let mut text = |name: &str, v: &Option<String>| {
            if let Some(v) = v.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
                parts.push(format!("{name} {v}"));
            }
        };
        match self {
            Saved::Policy(q) => return q.trim().to_string(),
            Saved::Traffic(t) => {
                text("src", &t.src);
                text("dst", &t.dst);
                text("svc", &t.service);
                let list = |v: &Option<Vec<String>>| v.as_ref().map(|l| l.join(","));
                text("decision", &list(&t.decisions));
                text("transmission", &list(&t.transmission));
                text("window", &t.window);
                text("cap", &t.max_results.map(|n| n.to_string()));
            }
            Saved::Events(e) => {
                text("type", &e.event_type);
                text("severity", &e.severity);
                text("window", &e.window);
            }
            Saved::Workloads(w) => {
                text("labels", &w.labels);
                text("enforcement", &w.enforcement);
                if let Some(m) = w.managed {
                    parts.push(if m { "managed" } else { "unmanaged" }.into());
                }
                if w.offline == Some(true) {
                    parts.push("offline".into());
                }
                if w.unlabeled == Some(true) {
                    parts.push("unlabeled".into());
                }
            }
        }
        if parts.is_empty() {
            "(empty)".into()
        } else {
            parts.join(" · ")
        }
    }
}

/// The config file profile saved filters belong to: the one a command
/// uses (`--profile`, `ILLUMIO_PROFILE`, else `default_profile`), when the
/// file has it. An `--env-file` connection, or a profile the file does not
/// have (an env-only one), has none.
pub fn profile_for(
    file: &ConfigFile,
    path: &Path,
    named: Option<String>,
    env_file: bool,
) -> Result<String, String> {
    if env_file {
        return Err(
            "saved filters belong to a config file profile; an --env-file connection has none"
                .into(),
        );
    }
    let (name, _) = config::profile_name(file, named);
    if file.profiles.contains_key(&name) {
        Ok(name)
    } else {
        Err(format!(
            "saved filters belong to a config file profile; {name:?} is not in {}",
            path.display()
        ))
    }
}

/// One profile's saved filters in one config file. Every call reads the
/// file again; a change rewrites it whole (`write_config_file`: atomic,
/// mode 0600), every other setting and unknown key kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Store {
    pub path: PathBuf,
    pub profile: String,
}

impl Store {
    pub fn new(path: PathBuf, profile: impl Into<String>) -> Self {
        Self {
            path,
            profile: profile.into(),
        }
    }

    fn missing(&self) -> ConfigError {
        ConfigError::Message(format!(
            "saved filters belong to a config file profile; {:?} is not in {}",
            self.profile,
            self.path.display()
        ))
    }

    /// The profile's saved filters.
    pub fn load(&self) -> Result<SavedFilters, ConfigError> {
        let file = config::load_config_file(&self.path)?;
        match file.profiles.get(&self.profile) {
            Some(p) => Ok(p.filters.clone()),
            None => Err(self.missing()),
        }
    }

    /// The filter `name` of `screen`, parsed; the error names the profile.
    pub fn get(&self, screen: Screen, name: &str) -> Result<Saved, ConfigError> {
        self.load()?
            .get(screen, name)
            .map_err(|e| ConfigError::Message(format!("profile {}: {e}", self.profile)))
    }

    /// Saves `saved` as `name`; an existing one only with `force`.
    /// Whether one was replaced.
    pub fn save(&self, name: &str, saved: &Saved, force: bool) -> Result<bool, ConfigError> {
        check_name(name).map_err(ConfigError::Message)?;
        let screen = saved.screen();
        self.change(|f| {
            let map = f.of_mut(screen);
            let replaced = map.contains_key(name);
            if replaced && !force {
                return Err(ConfigError::Message(format!(
                    "{screen} filter {name:?} exists in profile {}; --force replaces it",
                    self.profile
                )));
            }
            map.insert(name.to_string(), saved.to_value());
            Ok(replaced)
        })
    }

    /// Deletes the filter `name` of `screen`.
    pub fn remove(&self, screen: Screen, name: &str) -> Result<(), ConfigError> {
        self.change(|f| match f.of_mut(screen).remove(name) {
            Some(_) => Ok(()),
            None => Err(ConfigError::Message(format!(
                "no {screen} filter {name:?} in profile {}",
                self.profile
            ))),
        })
    }

    fn change<T>(
        &self,
        edit: impl FnOnce(&mut SavedFilters) -> Result<T, ConfigError>,
    ) -> Result<T, ConfigError> {
        let mut file = config::load_config_file(&self.path)?;
        let Some(p) = file.profiles.get_mut(&self.profile) else {
            return Err(self.missing());
        };
        let out = edit(&mut p.filters)?;
        config::write_config_file(&self.path, &file)?;
        Ok(out)
    }
}

/// A field's terms: `include` its `|` groups of `,` terms (OR of ANDs),
/// `exclude` every `!` term, whichever group it is written in: Explorer's
/// exclude list is flat, so a `!` leaves its match out of the whole field.
/// No include group (only `!` terms) means every flow except those.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Terms<T> {
    pub include: Vec<Vec<T>>,
    pub exclude: Vec<T>,
}

impl<T> Terms<T> {
    pub fn is_empty(&self) -> bool {
        self.include.is_empty() && self.exclude.is_empty()
    }
}

/// The `|` segments of `text` that hold a term (`a | ,` has one).
pub fn or_segments(text: &str) -> usize {
    text.split('|')
        .filter(|g| g.split(',').any(|t| !t.trim().is_empty()))
        .count()
}

/// `a,b | c, !d` is ((a AND b) OR c) AND NOT d. Empty terms and groups that
/// end up empty (`,` or ` | `) are dropped, so blank-ish text parses to no
/// terms at all. A `!` needs one term after it and starts the term: `a !b`
/// is a missing `,`.
pub fn parse_terms<T>(
    text: &str,
    parse: impl Fn(&str) -> Result<T, String>,
) -> Result<Terms<T>, String> {
    let mut terms = Terms {
        include: Vec::new(),
        exclude: Vec::new(),
    };
    for g in text.split('|') {
        let mut group = Vec::new();
        for e in g.split(',').map(str::trim).filter(|e| !e.is_empty()) {
            let negated = e.strip_prefix('!').map(str::trim_start);
            match negated {
                Some("") => return Err("! needs a term after it, e.g. !app=web".into()),
                Some(rest) if rest.starts_with('!') => {
                    return Err(format!("{e:?}: one ! per term (no double negation)"));
                }
                _ if e[1..].contains('!') => {
                    return Err(format!("{e:?}: use , before ! (env=prod, !app=web)"));
                }
                Some(rest) => terms.exclude.push(parse(rest)?),
                None => group.push(parse(e)?),
            }
        }
        if !group.is_empty() {
            terms.include.push(group);
        }
    }
    Ok(terms)
}

/// The text [`parse_terms`] reads back as `include` and `exclude` (term
/// text): groups joined by ` | `, each `!` term after them.
pub fn terms_text(include: &[Vec<String>], exclude: &[String]) -> String {
    let groups = include
        .iter()
        .map(|g| g.join(","))
        .filter(|g| !g.is_empty());
    let excludes = exclude.iter().map(|e| format!("!{e}"));
    let mut text = groups.collect::<Vec<_>>().join(" | ");
    for e in excludes {
        if !text.is_empty() {
            text.push_str(", ");
        }
        text.push_str(&e);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r#"
default_profile = "demo"
future_top = "kept"

[profiles.demo]
host = "pce.example"
org_id = 3
api_key = "api_1"
api_secret = "s3cr3t-filters-70"
unknown_profile_key = 7

[profiles.demo.limits]
traffic_max_results = 500

[profiles.demo.filters]
future_screen = { a = 1 }

[profiles.demo.filters.traffic.prod-blocked]
src = "env=prod"
dst = "!app=backup"
service = "!53/udp, 8000-8100/tcp"
decisions = ["blocked", "potentially_blocked"]
transmission = ["unicast"]
window = "7d"
max_results = 20000

[profiles.demo.filters.policy]
deny-prod = "dst:env=prod deny"

[profiles.demo.filters.events.noise-free]
type = "!agent.*, !=user.login"
severity = "warning+"
window = "7d"

[profiles.demo.filters.workloads.prod-unenforced]
labels = "env=prod"
enforcement = "visibility_only"
managed = true
offline = false
unlabeled = false

[profiles.lab]
host = "lab.example"
org_id = 1
"#;

    fn home() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, CONFIG).unwrap();
        (dir, path)
    }

    #[test]
    fn names_are_letters_digits_dot_underscore_dash_up_to_64() {
        for ok in ["a", "prod-blocked", "A.b_c-1", &"x".repeat(64)] {
            assert!(check_name(ok).is_ok(), "{ok}");
        }
        for bad in ["", "a b", "a/b", "ünïcode", "a=b", &"x".repeat(65)] {
            assert!(check_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_issue_example_parses_per_screen() {
        let (_d, path) = home();
        let f = Store::new(path, "demo").load().unwrap();
        let Saved::Traffic(t) = f.get(Screen::Traffic, "prod-blocked").unwrap() else {
            panic!()
        };
        assert_eq!(t.service.as_deref(), Some("!53/udp, 8000-8100/tcp"));
        assert_eq!(t.max_results, Some(20000));
        assert_eq!(
            f.get(Screen::Policy, "deny-prod").unwrap(),
            Saved::Policy("dst:env=prod deny".into())
        );
        let Saved::Events(e) = f.get(Screen::Events, "noise-free").unwrap() else {
            panic!()
        };
        assert_eq!(e.event_type.as_deref(), Some("!agent.*, !=user.login"));
        let Saved::Workloads(w) = f.get(Screen::Workloads, "prod-unenforced").unwrap() else {
            panic!()
        };
        assert_eq!(w.managed, Some(true));
        assert!(
            f.get(Screen::Traffic, "nope")
                .unwrap_err()
                .contains("prod-blocked")
        );
    }

    #[test]
    fn a_broken_filter_fails_only_when_used_naming_screen_filter_and_field() {
        let text = format!(
            "{CONFIG}\n[profiles.lab.filters.traffic.bad]\nsrc = 3\n\
             [profiles.lab.filters.workloads.typo]\nlables = \"env=prod\"\n\
             [profiles.lab.filters.policy]\nq = 1\n"
        );
        let file: ConfigFile = toml::from_str(&text).expect("startup is not stopped");
        let f = &file.profiles["lab"].filters;
        let e = f.get(Screen::Traffic, "bad").unwrap_err();
        assert!(e.contains("traffic filter bad") && e.contains("src"), "{e}");
        let e = f.get(Screen::Workloads, "typo").unwrap_err();
        assert!(
            e.contains("workloads filter typo") && e.contains("lables"),
            "{e}"
        );
        assert!(
            f.get(Screen::Policy, "q")
                .unwrap_err()
                .contains("query string")
        );
    }

    #[test]
    fn a_save_round_trips_unknown_keys_other_settings_and_the_secret() {
        let (_d, path) = home();
        let store = Store::new(path.clone(), "demo");
        let saved = Saved::Traffic(TrafficSaved {
            src: Some("app=web | app=api".into()),
            window: Some("1h".into()),
            ..Default::default()
        });
        assert!(!store.save("web", &saved, false).unwrap());
        let text = std::fs::read_to_string(&path).unwrap();
        for kept in [
            "future_top = \"kept\"",
            "unknown_profile_key = 7",
            "api_secret = \"s3cr3t-filters-70\"",
            "traffic_max_results = 500",
            "future_screen",
            "deny-prod",
            "noise-free",
            "prod-unenforced",
        ] {
            assert!(text.contains(kept), "{kept} lost:\n{text}");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert_eq!(store.get(Screen::Traffic, "web").unwrap(), saved);
        // The same name again: refused, then replaced with force.
        let e = store.save("web", &saved, false).unwrap_err().to_string();
        assert!(e.contains("--force"), "{e}");
        assert!(store.save("web", &saved, true).unwrap());
        store.remove(Screen::Traffic, "web").unwrap();
        assert!(store.get(Screen::Traffic, "web").is_err());
        assert!(store.remove(Screen::Traffic, "web").is_err());
        // Other profiles' filters are their own.
        let lab = Store::new(path, "lab");
        assert!(lab.load().unwrap().names(Screen::Traffic).is_empty());
    }

    #[test]
    fn a_profile_outside_the_file_or_an_env_file_has_no_filters() {
        let (_d, path) = home();
        let file = config::load_config_file(&path).unwrap();
        assert_eq!(profile_for(&file, &path, None, false).unwrap(), "demo");
        assert_eq!(
            profile_for(&file, &path, Some("lab".into()), false).unwrap(),
            "lab"
        );
        assert!(
            profile_for(&file, &path, None, true)
                .unwrap_err()
                .contains("--env-file")
        );
        let none = ConfigFile::default();
        assert!(profile_for(&none, &path, None, false).is_err());
        let store = Store::new(path, "ghost");
        assert!(store.load().is_err());
        assert!(
            store
                .save("x", &Saved::Policy("deny".into()), false)
                .is_err()
        );
    }

    #[test]
    fn summaries_name_the_fields_set() {
        let (_d, path) = home();
        let f = Store::new(path, "demo").load().unwrap();
        assert_eq!(
            f.get(Screen::Traffic, "prod-blocked").unwrap().summary(),
            "src env=prod · dst !app=backup · svc !53/udp, 8000-8100/tcp · \
             decision blocked,potentially_blocked · transmission unicast · window 7d · cap 20000"
        );
        assert_eq!(
            f.get(Screen::Workloads, "prod-unenforced")
                .unwrap()
                .summary(),
            "labels env=prod · enforcement visibility_only · managed"
        );
        assert_eq!(Saved::Events(EventsSaved::default()).summary(), "(empty)");
    }

    #[test]
    fn terms_text_reads_back_as_the_same_terms() {
        let inc = vec![vec!["a".to_string(), "b".into()], vec!["c".into()]];
        let exc = vec!["d".to_string(), "e".into()];
        let text = terms_text(&inc, &exc);
        assert_eq!(text, "a,b | c, !d, !e");
        let t = parse_terms(&text, |e| Ok(e.to_string())).unwrap();
        assert_eq!((t.include, t.exclude), (inc, exc));
        assert_eq!(terms_text(&[], &["x".into()]), "!x");
        assert_eq!(terms_text(&[], &[]), "");
    }
}
