//! Saved filters: named filters per profile in the config file
//! (`[profiles.<name>.filters.<screen>.<filter>]`) for the TUI's Traffic,
//! Policy, Events and Workloads screens and the CLI commands that match
//! them. Fields hold the TUI editor's text unchanged (`a,b | c, !d`); a
//! policy filter is one `pce::RuleFilter` query. They are kept raw, so one
//! that does not parse fails only where it is used, naming the profile,
//! screen, filter and field. Local files only: nothing here contacts the
//! PCE. A screen's default (`[profiles.<name>.filter_defaults]`) names one
//! of them: the TUI applies it at startup and after `:profile`, the CLI
//! only with `--default-filter`. Also the traffic filter text's grammar,
//! which the CLI shares.
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

/// A name is 1 to 64 of `A-Z a-z 0-9 . _ -` (case-sensitive), not
/// starting with `-` (a flag to the CLI).
pub fn check_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("a filter name is empty".into());
    }
    if name.starts_with('-') {
        return Err(format!(
            "filter name {name:?} starts with -, which reads as a flag"
        ));
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

/// `v`'s TOML type with its article: `a string`, `an integer`.
pub fn a_type(v: &toml::Value) -> String {
    let t = v.type_str();
    let article = if t.starts_with(['a', 'e', 'i', 'o', 'u']) {
        "an"
    } else {
        "a"
    };
    format!("{article} {t}")
}

/// A profile's saved filters as the file holds them, per screen and name;
/// each parsed only where it is used ([`Saved::parse`]). A screen key that
/// is not a table (`traffic = "x"`) stays in `extra`, and a `filters` that
/// is not one (`filters = "x"`) in `raw`, each written back as it was:
/// those filters fail where they are used, nothing else does.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct SavedFilters {
    pub traffic: BTreeMap<String, toml::Value>,
    pub policy: BTreeMap<String, toml::Value>,
    pub events: BTreeMap<String, toml::Value>,
    pub workloads: BTreeMap<String, toml::Value>,
    /// Unknown keys, round-tripped verbatim.
    pub extra: toml::Table,
    /// The whole value when it is not a table.
    pub raw: Option<toml::Value>,
}

impl Serialize for SavedFilters {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        if let Some(v) = &self.raw {
            return v.serialize(s);
        }
        let mut t = self.extra.clone();
        for screen in Screen::ALL {
            let m = self.of(screen);
            if !m.is_empty() {
                let table = m.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                t.insert(screen.as_str().into(), toml::Value::Table(table));
            }
        }
        t.serialize(s)
    }
}

impl<'de> Deserialize<'de> for SavedFilters {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let mut extra = match toml::Value::deserialize(d)? {
            toml::Value::Table(t) => t,
            other => {
                return Ok(SavedFilters {
                    raw: Some(other),
                    ..Default::default()
                });
            }
        };
        let mut f = SavedFilters::default();
        for screen in Screen::ALL {
            match extra.remove(screen.as_str()) {
                Some(toml::Value::Table(t)) => *f.of_mut(screen) = t.into_iter().collect(),
                Some(other) => {
                    extra.insert(screen.as_str().to_string(), other);
                }
                None => {}
            }
        }
        f.extra = extra;
        Ok(f)
    }
}

impl SavedFilters {
    /// Why no filter can be read: `filters` is not a table.
    pub fn top_error(&self) -> Option<String> {
        self.raw
            .as_ref()
            .map(|v| format!("filters is {}, not a table of screens", a_type(v)))
    }

    /// Why `screen`'s filters cannot be read: `filters` or its key is not
    /// a table.
    pub fn shape_error(&self, screen: Screen) -> Option<String> {
        self.top_error().or_else(|| {
            self.extra
                .get(screen.as_str())
                .map(|v| format!("filters.{screen} is {}, not a table of filters", a_type(v)))
        })
    }

    /// The screen's filters, or why they cannot be read.
    pub fn checked(&self, screen: Screen) -> Result<&BTreeMap<String, toml::Value>, String> {
        match self.shape_error(screen) {
            Some(e) => Err(e),
            None => Ok(self.of(screen)),
        }
    }

    pub fn is_empty(&self) -> bool {
        Screen::ALL.iter().all(|s| self.of(*s).is_empty())
            && self.extra.is_empty()
            && self.raw.is_none()
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
        match self.checked(screen)?.get(name) {
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

/// A profile's default filter per screen (`[profiles.<name>.filter_defaults]`,
/// `traffic = "prod-blocked"`): a filter name. Kept raw, so a value that is
/// not a name, or a `filter_defaults` that is not a table (kept in `raw`),
/// is said where it is used; both, and unknown keys, round-trip verbatim.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct FilterDefaults {
    pub names: toml::Table,
    /// The whole value when it is not a table.
    pub raw: Option<toml::Value>,
}

impl Serialize for FilterDefaults {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match &self.raw {
            Some(v) => v.serialize(s),
            None => self.names.serialize(s),
        }
    }
}

impl<'de> Deserialize<'de> for FilterDefaults {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(match toml::Value::deserialize(d)? {
            toml::Value::Table(names) => FilterDefaults { names, raw: None },
            other => FilterDefaults {
                names: toml::Table::new(),
                raw: Some(other),
            },
        })
    }
}

impl FilterDefaults {
    pub fn is_empty(&self) -> bool {
        self.names.is_empty() && self.raw.is_none()
    }

    /// Why no default can be read: `filter_defaults` is not a table.
    pub fn shape_error(&self) -> Option<String> {
        self.raw.as_ref().map(|v| {
            format!(
                "filter_defaults is {}, not a table of screen = filter name",
                a_type(v)
            )
        })
    }

    /// The screen's default filter name, or why the value is not one.
    pub fn get(&self, screen: Screen) -> Result<Option<&str>, String> {
        if let Some(e) = self.shape_error() {
            return Err(e);
        }
        match self.names.get(screen.as_str()) {
            None => Ok(None),
            Some(toml::Value::String(s)) => Ok(Some(s.as_str())),
            Some(v) => Err(format!(
                "filter_defaults.{screen} is {}, not a filter name",
                a_type(v)
            )),
        }
    }

    /// The default filter of `screen`, parsed: `None` without one; an error
    /// (the default is then ignored) when it names no filter or one that
    /// does not parse.
    pub fn resolve(
        &self,
        filters: &SavedFilters,
        screen: Screen,
    ) -> Result<Option<(String, Saved)>, String> {
        let Some(name) = self.get(screen)? else {
            return Ok(None);
        };
        filters
            .get(screen, name)
            .map(|s| Some((name.to_string(), s)))
            .map_err(|e| format!("default {screen} filter {name:?}: {e}"))
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

/// An events filter: type and actor terms, severity and window as typed
/// in `f`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct EventsSaved {
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub event_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window: Option<String>,
}

/// A workloads filter: the label filter (`f`) and the local filters
/// (`E` `m` `o` `u`, the VEN health `v` and version).
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
    /// VEN health states, any of (`warning`, `error`, ...).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health: Option<Vec<String>>,
    /// The VEN version: exact, or a `*` glob (`23.2.*`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ven_version: Option<String>,
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
        format!("field {k}: expected {want}, found {}", a_type(v))
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
        let table = || match v {
            toml::Value::Table(t) => Ok(Fields(t)),
            other => Err(format!(
                "expected a table of fields, found {}",
                a_type(other)
            )),
        };
        Ok(match screen {
            Screen::Policy => match v {
                toml::Value::String(s) => Saved::Policy(s.clone()),
                other => {
                    return Err(format!(
                        "a policy filter is a query string, not {}",
                        a_type(other)
                    ));
                }
            },
            Screen::Traffic => {
                let f = table()?;
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
                let f = table()?;
                f.only(&["type", "actor", "severity", "window"])?;
                Saved::Events(EventsSaved {
                    event_type: f.text("type")?,
                    actor: f.text("actor")?,
                    severity: f.text("severity")?,
                    window: f.text("window")?,
                })
            }
            Screen::Workloads => {
                let f = table()?;
                f.only(&[
                    "labels",
                    "enforcement",
                    "managed",
                    "offline",
                    "unlabeled",
                    "health",
                    "ven_version",
                ])?;
                let enforcement = f.text("enforcement")?;
                if let Some(e) = &enforcement
                    && !ENFORCEMENTS.contains(&e.trim())
                {
                    return Err(format!(
                        "field enforcement: {e:?} is not one of {}",
                        ENFORCEMENTS.join(", ")
                    ));
                }
                let health = f.list("health")?;
                let health = match health {
                    Some(h) => Some(
                        pce::ven::HealthSet::from_names(&h)
                            .map_err(|e| format!("field health: {e}"))?
                            .names(),
                    ),
                    None => None,
                };
                Saved::Workloads(WorkloadsSaved {
                    labels: f.text("labels")?,
                    enforcement: enforcement.map(|e| e.trim().to_string()),
                    managed: f.flag("managed")?,
                    offline: f.flag("offline")?,
                    unlabeled: f.flag("unlabeled")?,
                    health: health.filter(|h| !h.is_empty()),
                    ven_version: f
                        .text("ven_version")?
                        .map(|v| v.trim().to_string())
                        .filter(|v| !v.is_empty()),
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
                text("actor", &e.actor);
                text("severity", &e.severity);
                text("window", &e.window);
            }
            Saved::Workloads(w) => {
                text("labels", &w.labels);
                text("enforcement", &w.enforcement);
                text("health", &w.health.as_ref().map(|h| h.join(",")));
                text("ven", &w.ven_version);
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
        Ok(self.load_with_defaults()?.0)
    }

    /// The profile's saved filters and its defaults, from one read.
    pub fn load_with_defaults(&self) -> Result<(SavedFilters, FilterDefaults), ConfigError> {
        let mut file = config::load_config_file(&self.path)?;
        match file.profiles.remove(&self.profile) {
            Some(p) => Ok((p.filters, p.filter_defaults)),
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
        self.change(|p| {
            let f = &mut p.filters;
            if let Some(e) = f.shape_error(screen) {
                return Err(ConfigError::Message(format!(
                    "profile {}: {e}",
                    self.profile
                )));
            }
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

    /// Deletes the filter `name` of `screen`, and the screen's default when
    /// it names that filter. Whether the default went too.
    pub fn remove(&self, screen: Screen, name: &str) -> Result<bool, ConfigError> {
        self.change(|p| {
            if p.filters.of_mut(screen).remove(name).is_none() {
                return Err(ConfigError::Message(format!(
                    "no {screen} filter {name:?} in profile {}",
                    self.profile
                )));
            }
            let was_default = p.filter_defaults.get(screen) == Ok(Some(name));
            if was_default {
                p.filter_defaults.names.remove(screen.as_str());
            }
            Ok(was_default)
        })
    }

    /// Makes the saved filter `name` the default of `screen` (it must exist
    /// and parse), or clears the default (`None`). The default it replaced.
    pub fn set_default(
        &self,
        screen: Screen,
        name: Option<&str>,
    ) -> Result<Option<String>, ConfigError> {
        self.change(|p| {
            if let Some(e) = p.filter_defaults.shape_error() {
                return Err(ConfigError::Message(format!(
                    "profile {}: {e}; fix it in {}",
                    self.profile,
                    self.path.display()
                )));
            }
            let previous = p
                .filter_defaults
                .get(screen)
                .ok()
                .flatten()
                .map(String::from);
            match name {
                Some(name) => {
                    p.filters.get(screen, name).map_err(|e| {
                        ConfigError::Message(format!("profile {}: {e}", self.profile))
                    })?;
                    p.filter_defaults
                        .names
                        .insert(screen.as_str().into(), toml::Value::String(name.into()));
                }
                None => {
                    p.filter_defaults.names.remove(screen.as_str());
                }
            }
            Ok(previous)
        })
    }

    fn change<T>(
        &self,
        edit: impl FnOnce(&mut config::Profile) -> Result<T, ConfigError>,
    ) -> Result<T, ConfigError> {
        let mut file = config::load_config_file(&self.path)?;
        let Some(p) = file.profiles.get_mut(&self.profile) else {
            return Err(self.missing());
        };
        let out = edit(p)?;
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
        for bad in ["", "-x", "a b", "a/b", "ünïcode", "a=b", &"x".repeat(65)] {
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
    fn workloads_health_and_ven_version_parse_round_trip_and_name_a_bad_state() {
        let v: toml::Value = toml::from_str(
            "health = \"offline, Warning\"\nven_version = \" 23.2.* \"\noffline = true",
        )
        .unwrap();
        let s = Saved::parse(Screen::Workloads, &v).unwrap();
        let Saved::Workloads(w) = &s else { panic!() };
        assert_eq!(
            w.health.as_deref(),
            Some(&["warning".to_string(), "offline".to_string()][..])
        );
        assert_eq!(w.ven_version.as_deref(), Some("23.2.*"));
        assert_eq!(s.summary(), "health warning,offline · ven 23.2.* · offline");
        assert_eq!(Saved::parse(Screen::Workloads, &s.to_value()).unwrap(), s);
        let bad: toml::Value = toml::from_str("health = [\"warn\"]").unwrap();
        let e = Saved::parse(Screen::Workloads, &bad).unwrap_err();
        assert!(
            e.starts_with("field health:") && e.contains("\"warn\""),
            "{e}"
        );
        let bad: toml::Value = toml::from_str("ven_version = 23").unwrap();
        let e = Saved::parse(Screen::Workloads, &bad).unwrap_err();
        assert!(e.starts_with("field ven_version:"), "{e}");
    }

    #[test]
    fn an_events_actor_parses_round_trips_and_names_a_wrong_shape() {
        let v: toml::Value =
            toml::from_str("actor = \"user:*, !=alex@ryte.de\"\nseverity = \"warning+\"").unwrap();
        let s = Saved::parse(Screen::Events, &v).unwrap();
        let Saved::Events(e) = &s else { panic!() };
        assert_eq!(e.actor.as_deref(), Some("user:*, !=alex@ryte.de"));
        assert_eq!(e.event_type, None);
        assert_eq!(
            s.summary(),
            "actor user:*, !=alex@ryte.de · severity warning+"
        );
        assert_eq!(Saved::parse(Screen::Events, &s.to_value()).unwrap(), s);
        let bad: toml::Value = toml::from_str("actor = 3").unwrap();
        let e = Saved::parse(Screen::Events, &bad).unwrap_err();
        assert!(e.starts_with("field actor:"), "{e}");
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
    fn a_screen_key_of_the_wrong_shape_fails_only_that_screen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let text = CONFIG.replace(
            "[profiles.demo.filters]\nfuture_screen = { a = 1 }",
            "[profiles.demo.filters]\nfuture_screen = { a = 1 }\nevents = \"oops\"",
        )
        .replace(
            "[profiles.demo.filters.events.noise-free]\ntype = \"!agent.*, !=user.login\"\nseverity = \"warning+\"\nwindow = \"7d\"\n",
            "",
        );
        assert!(text.contains("events = \"oops\""), "{text}");
        std::fs::write(&path, &text).unwrap();
        // The file loads: other commands and profiles are not stopped.
        let file = config::load_config_file(&path).expect("loads");
        assert!(file.profiles.contains_key("lab"));
        let store = Store::new(path.clone(), "demo");
        let f = store.load().unwrap();
        let e = f.get(Screen::Events, "x").unwrap_err();
        assert!(e.contains("filters.events is a string"), "{e}");
        assert!(
            f.get(Screen::Traffic, "prod-blocked").is_ok(),
            "other screens work"
        );
        // A save elsewhere keeps the odd value as it was; one there is refused.
        let p = Saved::Policy("deny".into());
        store.save("d", &p, false).unwrap();
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("events = \"oops\"")
        );
        let e = store
            .save("e", &Saved::Events(EventsSaved::default()), false)
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("profile demo") && e.contains("filters.events"),
            "{e}"
        );
    }

    #[test]
    fn defaults_round_trip_with_unknown_keys_and_go_with_their_filter() {
        let (_d, path) = home();
        let text = CONFIG.replace(
            "[profiles.lab]",
            "[profiles.demo.filter_defaults]\nevents = \"noise-free\"\nfuture_default = 1\n\n[profiles.lab]",
        );
        std::fs::write(&path, text).unwrap();
        let store = Store::new(path.clone(), "demo");
        let (f, d) = store.load_with_defaults().unwrap();
        assert_eq!(d.get(Screen::Events), Ok(Some("noise-free")));
        assert_eq!(d.get(Screen::Traffic), Ok(None));
        let (name, saved) = d.resolve(&f, Screen::Events).unwrap().unwrap();
        assert_eq!(
            (name.as_str(), saved.screen()),
            ("noise-free", Screen::Events)
        );
        // Set: the filter must exist; the previous default is returned.
        let e = store
            .set_default(Screen::Traffic, Some("nope"))
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("profile demo") && e.contains("prod-blocked"),
            "{e}"
        );
        assert_eq!(
            store
                .set_default(Screen::Traffic, Some("prod-blocked"))
                .unwrap(),
            None
        );
        assert_eq!(
            store
                .set_default(Screen::Traffic, Some("prod-blocked"))
                .unwrap(),
            Some("prod-blocked".into())
        );
        let text = std::fs::read_to_string(&path).unwrap();
        for kept in [
            "future_default = 1",
            "events = \"noise-free\"",
            "traffic = \"prod-blocked\"",
            "api_secret = \"s3cr3t-filters-70\"",
            "future_top = \"kept\"",
        ] {
            assert!(text.contains(kept), "{kept} lost:\n{text}");
        }
        // Removing a filter that is not the default keeps the default.
        store
            .save("other", &Saved::Events(EventsSaved::default()), false)
            .unwrap();
        assert!(!store.remove(Screen::Events, "other").unwrap());
        // Removing the default's filter clears the default, nothing else.
        assert!(store.remove(Screen::Events, "noise-free").unwrap());
        let (_, d) = store.load_with_defaults().unwrap();
        assert_eq!(d.get(Screen::Events), Ok(None));
        assert_eq!(d.get(Screen::Traffic), Ok(Some("prod-blocked")));
        assert!(d.names.contains_key("future_default"));
        // Unset.
        assert_eq!(
            store.set_default(Screen::Traffic, None).unwrap(),
            Some("prod-blocked".into())
        );
        assert_eq!(store.set_default(Screen::Traffic, None).unwrap(), None);
        let (_, d) = store.load_with_defaults().unwrap();
        assert!(d.get(Screen::Traffic).unwrap().is_none());
    }

    #[test]
    fn a_missing_or_broken_default_is_an_error_where_used_never_at_load() {
        let text = format!(
            "{CONFIG}\n[profiles.lab.filters.traffic.bad]\nsrc = 3\n\
             [profiles.lab.filter_defaults]\ntraffic = \"bad\"\npolicy = \"gone\"\nevents = 3\n"
        );
        let file: ConfigFile = toml::from_str(&text).expect("startup is not stopped");
        let p = &file.profiles["lab"];
        let d = &p.filter_defaults;
        let e = d.resolve(&p.filters, Screen::Traffic).unwrap_err();
        assert!(
            e.contains("default traffic filter \"bad\"") && e.contains("src"),
            "{e}"
        );
        let e = d.resolve(&p.filters, Screen::Policy).unwrap_err();
        assert!(e.contains("\"gone\"") && e.contains("none saved"), "{e}");
        let e = d.resolve(&p.filters, Screen::Events).unwrap_err();
        assert!(e.contains("filter_defaults.events is an integer"), "{e}");
        assert_eq!(d.resolve(&p.filters, Screen::Workloads), Ok(None));
    }

    #[test]
    fn wrong_shaped_filters_or_defaults_load_and_are_written_back_as_they_were() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let text = CONFIG.replace(
            "[profiles.lab]\nhost = \"lab.example\"",
            "[profiles.lab]\nhost = \"lab.example\"\nfilters = \"x\"\nfilter_defaults = [1]",
        );
        std::fs::write(&path, &text).unwrap();
        let file = config::load_config_file(&path).expect("loads");
        let lab = &file.profiles["lab"];
        assert_eq!(
            lab.filters.get(Screen::Traffic, "a").unwrap_err(),
            "filters is a string, not a table of screens"
        );
        assert_eq!(
            lab.filter_defaults.get(Screen::Events).unwrap_err(),
            "filter_defaults is an array, not a table of screen = filter name"
        );
        let store = Store::new(path.clone(), "lab");
        let e = store
            .save("p", &Saved::Policy("deny".into()), false)
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("profile lab") && e.contains("filters is a string"),
            "{e}"
        );
        let e = store
            .set_default(Screen::Traffic, None)
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("profile lab") && e.contains("filter_defaults is an array"),
            "{e}"
        );
        // A change to another profile keeps both as they were.
        Store::new(path.clone(), "demo")
            .save("new", &Saved::Policy("deny".into()), false)
            .unwrap();
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(
            after.contains("filters = \"x\"") && after.contains("filter_defaults = [1]"),
            "{after}"
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
