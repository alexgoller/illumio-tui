//! Saved filters on the command line: `--filter NAME` (the saved filter
//! as the base, flags replacing its fields), `--save-filter NAME` (the
//! effective filter saved, then exit: no request), `--default-filter`
//! (the screen's default as `--filter`; never applied without it), and
//! `filters list|show|rm|default`. The config file only; nothing here
//! contacts the PCE.
use super::context::config_inputs;
use super::error::CliError;
use super::output::{
    Meta, OutputFormat, TableRows, choose_format, meta_line, write_json, write_table,
    write_table_csv,
};
use super::{Command, GlobalArgs};
use crate::config;
use crate::filters::{Saved, Screen, Store};
use clap::{Args, Subcommand};
use serde::Serialize;

/// `--filter`, `--save-filter` and `--force` on `traffic query|groups|mesh`,
/// `policy search`, `events` and `workloads`.
#[derive(Args, Debug, Clone, Default)]
pub struct SavedFilterArgs {
    /// Start from this saved filter of the profile (config file): a flag given here replaces
    /// that field of it (`policy search`: terms and flags are added to its query)
    #[arg(long, value_name = "NAME")]
    pub filter: Option<String>,
    /// Start from the profile's default filter for this screen (`filters default`), as --filter
    /// would; an error when there is none. Without this flag the CLI never applies a default
    #[arg(long, conflicts_with = "filter")]
    pub default_filter: bool,
    /// Save the filter these flags (and --filter) make as NAME in the profile, and exit without
    /// contacting the PCE; prints `saved <screen> filter NAME to <config path>` (JSON:
    /// data.screen, data.name, data.config_path, data.replaced, data.filter). Names: letters,
    /// digits, . _ - (up to 64, not starting with -)
    #[arg(long, value_name = "NAME")]
    pub save_filter: Option<String>,
    /// With --save-filter: replace a saved filter of that name
    #[arg(long, requires = "save_filter")]
    pub force: bool,
}

#[derive(Subcommand, Debug)]
pub enum FiltersCmd {
    /// List the profile's saved filters with a one-line summary each (local, no request)
    List(ListArgs),
    /// Print one saved filter: filters show traffic NAME
    #[command(subcommand)]
    Show(ScreenName),
    /// Delete one saved filter (and the screen's default, when it is that one): filters rm traffic
    /// NAME
    #[command(subcommand)]
    Rm(ScreenName),
    /// Set a screen's default filter (filters default traffic NAME) or clear it (--unset). The TUI
    /// applies it at startup and after :profile; the CLI only with --default-filter
    #[command(subcommand)]
    Default(ScreenDefault),
}

/// The screen whose default is set, and the filter (or `--unset`).
#[derive(Subcommand, Debug, Clone)]
pub enum ScreenDefault {
    /// The Traffic screen's default (traffic query|groups|mesh --default-filter)
    Traffic(DefaultArg),
    /// The Policy screen's default rule search (policy search --default-filter)
    Policy(DefaultArg),
    /// The Events screen's default (events --default-filter)
    Events(DefaultArg),
    /// The Workloads screen's default (workloads --default-filter)
    Workloads(DefaultArg),
}

#[derive(Args, Debug, Clone)]
pub struct DefaultArg {
    /// The saved filter to make the default
    #[arg(required_unless_present = "unset")]
    pub name: Option<String>,
    /// Clear the screen's default instead
    #[arg(long, conflicts_with = "name")]
    pub unset: bool,
}

impl ScreenDefault {
    fn parts(&self) -> (Screen, Option<&str>) {
        let (screen, a) = match self {
            ScreenDefault::Traffic(a) => (Screen::Traffic, a),
            ScreenDefault::Policy(a) => (Screen::Policy, a),
            ScreenDefault::Events(a) => (Screen::Events, a),
            ScreenDefault::Workloads(a) => (Screen::Workloads, a),
        };
        (screen, a.name.as_deref().filter(|_| !a.unset))
    }
}

#[derive(Args, Debug, Clone, Default)]
pub struct ListArgs {
    /// Only this screen's filters: traffic, policy, events, workloads
    #[arg(long, value_parser = ["traffic", "policy", "events", "workloads"])]
    pub screen: Option<String>,
}

/// The screen a saved filter belongs to, and its name.
#[derive(Subcommand, Debug, Clone)]
pub enum ScreenName {
    /// A traffic filter (traffic query|groups|mesh, the Traffic screen)
    Traffic(NameArg),
    /// A rule search (policy search, the Policy screen)
    Policy(NameArg),
    /// An events filter (events, the Events screen)
    Events(NameArg),
    /// A workloads filter (workloads, the Workloads screen)
    Workloads(NameArg),
}

#[derive(Args, Debug, Clone)]
pub struct NameArg {
    /// The saved filter's name
    pub name: String,
}

impl ScreenName {
    fn parts(&self) -> (Screen, &str) {
        match self {
            ScreenName::Traffic(a) => (Screen::Traffic, &a.name),
            ScreenName::Policy(a) => (Screen::Policy, &a.name),
            ScreenName::Events(a) => (Screen::Events, &a.name),
            ScreenName::Workloads(a) => (Screen::Workloads, &a.name),
        }
    }
}

/// The saved filter flags of `command`, and its screen; `None` for a
/// command without them.
pub fn saved_args(command: &Command) -> Option<(Screen, &SavedFilterArgs)> {
    use super::policy::PolicyCmd;
    match command {
        Command::Traffic(t) => t.filter().map(|f| (Screen::Traffic, &f.saved)),
        Command::Policy(PolicyCmd::Search(a)) => Some((Screen::Policy, &a.saved)),
        Command::Events(a) => Some((Screen::Events, &a.saved)),
        Command::Workloads(w) if w.action.is_none() => Some((Screen::Workloads, &w.saved)),
        _ => None,
    }
}

/// `--save-filter`: prints no table.
pub fn saves_filter(command: &Command) -> bool {
    saved_args(command).is_some_and(|(_, a)| a.save_filter.is_some())
}

/// The profile's store: the config file profile the command uses.
fn store(global: &GlobalArgs) -> Result<Store, CliError> {
    let path = global
        .config
        .clone()
        .unwrap_or_else(config::default_config_path);
    if global.env_file.is_some() {
        // Before the env file is read: it cannot make one.
        crate::filters::profile_for(&config::ConfigFile::default(), &path, None, true)
            .map_err(CliError::Args)?;
    }
    let (file, overrides, env) = config_inputs(global)?;
    let named = overrides.profile.clone().or_else(|| {
        env.get("ILLUMIO_PROFILE")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    });
    let profile =
        crate::filters::profile_for(&file, &path, named, false).map_err(CliError::Args)?;
    Ok(Store::new(path, profile))
}

/// Before a command runs: `--filter` merged into it, `--save-filter`
/// saved (then `None`: done, nothing sent). A command without either is
/// given back as it is.
pub fn prepare(global: &GlobalArgs, mut command: Command) -> Result<Option<Command>, CliError> {
    let Some((screen, args)) = saved_args(&command) else {
        return Ok(Some(command));
    };
    let mut args = args.clone();
    if args.filter.is_none() && args.save_filter.is_none() && !args.default_filter {
        return Ok(Some(command));
    }
    let store = store(global)?;
    if args.default_filter {
        args.filter = Some(default_name(&store, screen)?);
    }
    if let Some(name) = &args.filter {
        let saved = store.get(screen, name).map_err(|e| {
            if args.default_filter {
                CliError::Args(format!("--default-filter: {e}"))
            } else {
                e.into()
            }
        })?;
        merge(&mut command, &saved).map_err(|e| {
            CliError::Args(format!(
                "profile {}, {screen} filter {name}: {e}",
                store.profile
            ))
        })?;
    }
    let Some(name) = &args.save_filter else {
        return Ok(Some(command));
    };
    crate::filters::check_name(name).map_err(CliError::Args)?;
    let saved =
        effective(&command).map_err(|e| CliError::Args(format!("--save-filter {name}: {e}")))?;
    let replaced = store.save(name, &saved, args.force)?;
    let out = SaveResult {
        screen: screen.as_str(),
        name: name.clone(),
        profile: store.profile.clone(),
        config_path: store.path.display().to_string(),
        replaced,
        summary: saved.summary(),
        filter: json_of(&saved.to_value()),
    };
    let mut stdout = std::io::stdout().lock();
    match choose_format(global.json, global.table, global.csv) {
        OutputFormat::Json => write_json(&mut stdout, &out, &meta(&store.profile))?,
        _ => {
            use std::io::Write;
            writeln!(
                stdout,
                "saved {screen} filter {name} to {}",
                store.path.display()
            )?;
        }
    }
    Ok(None)
}

/// `--default-filter`: the name of the profile's default for `screen`.
fn default_name(store: &Store, screen: Screen) -> Result<String, CliError> {
    let (_, defaults) = store.load_with_defaults()?;
    let named = defaults
        .get(screen)
        .map_err(|e| CliError::Args(format!("--default-filter: profile {}: {e}", store.profile)))?;
    named.map(String::from).ok_or_else(|| {
        CliError::Args(format!(
            "--default-filter: profile {} has no default {screen} filter; \
             filters default {screen} NAME sets one",
            store.profile
        ))
    })
}

/// `saved` (of the command's screen) as the base of `command`.
fn merge(command: &mut Command, saved: &Saved) -> Result<(), String> {
    use super::policy::PolicyCmd;
    match (command, saved) {
        (Command::Traffic(t), Saved::Traffic(s)) => match t.filter_mut() {
            Some(f) => f.with_saved(s),
            None => Ok(()),
        },
        (Command::Policy(PolicyCmd::Search(a)), Saved::Policy(q)) => a.with_saved(q),
        (Command::Events(a), Saved::Events(s)) => a.with_saved(s),
        (Command::Workloads(w), Saved::Workloads(s)) => w.list.with_saved(s),
        _ => Err("not this command's filter".into()),
    }
}

/// The filter `command` runs with, as it is saved.
fn effective(command: &Command) -> Result<Saved, String> {
    use super::policy::PolicyCmd;
    match command {
        Command::Traffic(t) => match t.filter() {
            Some(f) => f.to_saved().map(Saved::Traffic),
            None => Err("traffic explain has no saved filters".into()),
        },
        Command::Policy(PolicyCmd::Search(a)) => a.to_saved().map(Saved::Policy),
        Command::Events(a) => a.to_saved().map(Saved::Events),
        Command::Workloads(w) => w.list.to_saved().map(Saved::Workloads),
        _ => Err("this command has no saved filters".into()),
    }
}

/// A stored value as JSON.
fn json_of(v: &toml::Value) -> serde_json::Value {
    serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
}

fn meta(profile: &str) -> Meta {
    Meta {
        profile: profile.to_string(),
        cache: "none",
        ..Meta::default()
    }
}

/// What `--save-filter` saved.
#[derive(Debug, Serialize)]
struct SaveResult {
    screen: &'static str,
    name: String,
    profile: String,
    config_path: String,
    replaced: bool,
    summary: String,
    filter: serde_json::Value,
}

/// One `filters list` row; JSON has the stored fields too.
#[derive(Debug, Serialize)]
pub struct FilterRow {
    pub screen: &'static str,
    pub name: String,
    /// The screen's default (`filters default`).
    pub default: bool,
    pub summary: String,
    /// Why the stored filter does not parse.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub filter: serde_json::Value,
}

impl TableRows for Vec<FilterRow> {
    fn headers(&self) -> Vec<&'static str> {
        vec!["screen", "name", "default", "summary"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.iter()
            .map(|r| {
                let summary = match &r.error {
                    Some(e) => format!("does not parse: {e}"),
                    None => r.summary.clone(),
                };
                let default = if r.default { "yes" } else { "" };
                vec![
                    r.screen.to_string(),
                    r.name.clone(),
                    default.to_string(),
                    summary,
                ]
            })
            .collect()
    }
}

/// `filters show`: one filter, a field per row.
#[derive(Debug, Serialize)]
pub struct ShowResult {
    pub screen: &'static str,
    pub name: String,
    pub summary: String,
    pub filter: serde_json::Value,
    #[serde(skip)]
    fields: Vec<(String, String)>,
}

impl TableRows for ShowResult {
    fn headers(&self) -> Vec<&'static str> {
        vec!["field", "value"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        self.fields
            .iter()
            .map(|(k, v)| vec![k.clone(), v.clone()])
            .collect()
    }
}

/// `filters rm`: what was removed.
#[derive(Debug, Serialize)]
pub struct RmResult {
    pub screen: &'static str,
    pub name: String,
    /// It was the screen's default, which is now cleared.
    pub default_cleared: bool,
    pub config_path: String,
}

impl TableRows for RmResult {
    fn headers(&self) -> Vec<&'static str> {
        vec!["removed", "config_path"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        let mut removed = format!("{} filter {}", self.screen, self.name);
        if self.default_cleared {
            removed.push_str(&format!(" and the {} default", self.screen));
        }
        vec![vec![removed, self.config_path.clone()]]
    }
}

/// `filters default`: the screen's default now (`None`: cleared) and before.
#[derive(Debug, Serialize)]
pub struct DefaultResult {
    pub screen: &'static str,
    pub default: Option<String>,
    pub previous: Option<String>,
    pub config_path: String,
}

impl TableRows for DefaultResult {
    fn headers(&self) -> Vec<&'static str> {
        vec!["screen", "default", "previous", "config_path"]
    }
    fn rows(&self) -> Vec<Vec<String>> {
        let name = |n: &Option<String>| n.clone().unwrap_or_else(|| "-".into());
        vec![vec![
            self.screen.to_string(),
            name(&self.default),
            name(&self.previous),
            self.config_path.clone(),
        ]]
    }
}

fn emit<T: Serialize + TableRows>(
    global: &GlobalArgs,
    data: &T,
    profile: &str,
) -> Result<(), CliError> {
    let meta = meta(profile);
    let mut out = std::io::stdout().lock();
    match choose_format(global.json, global.table, global.csv) {
        OutputFormat::Json => write_json(&mut out, data, &meta)?,
        OutputFormat::Table => {
            write_table(&mut out, data)?;
            eprintln!("{}", meta_line(&meta));
        }
        OutputFormat::Csv => {
            write_table_csv(&mut out, data)?;
            eprintln!("{}", meta_line(&meta));
        }
    }
    Ok(())
}

/// A stored field's value as text: lists joined by `, `.
fn field_text(v: &toml::Value) -> String {
    match v {
        toml::Value::String(s) => s.clone(),
        toml::Value::Array(a) => a.iter().map(field_text).collect::<Vec<_>>().join(", "),
        other => other.to_string(),
    }
}

pub fn run(global: &GlobalArgs, cmd: &FiltersCmd) -> Result<(), CliError> {
    let store = store(global)?;
    let (filters, defaults) = store.load_with_defaults()?;
    match cmd {
        FiltersCmd::List(a) => {
            let screens: Vec<Screen> = match a.screen.as_deref().and_then(Screen::parse) {
                Some(s) => vec![s],
                None => Screen::ALL.to_vec(),
            };
            let mut rows = Vec::new();
            for screen in screens {
                let problem = |name: &str, default: bool, e: String| FilterRow {
                    screen: screen.as_str(),
                    name: name.into(),
                    default,
                    summary: String::new(),
                    error: Some(e),
                    filter: serde_json::Value::Null,
                };
                if let Some(e) = filters.shape_error(screen) {
                    rows.push(problem("-", false, e));
                }
                let default = match defaults.get(screen) {
                    Ok(d) => d,
                    Err(e) => {
                        rows.push(problem("-", true, e));
                        None
                    }
                };
                if let Some(d) = default
                    && !filters.of(screen).contains_key(d)
                {
                    let e = format!("the default names no saved {screen} filter; it is ignored");
                    rows.push(problem(d, true, e));
                }
                for (name, v) in filters.of(screen) {
                    let (summary, error) = match Saved::parse(screen, v) {
                        Ok(s) => (s.summary(), None),
                        Err(e) => (String::new(), Some(e)),
                    };
                    rows.push(FilterRow {
                        screen: screen.as_str(),
                        name: name.clone(),
                        default: default == Some(name.as_str()),
                        summary,
                        error,
                        filter: json_of(v),
                    });
                }
            }
            emit(global, &rows, &store.profile)
        }
        FiltersCmd::Show(which) => {
            let (screen, name) = which.parts();
            let saved = store.get(screen, name)?;
            let value = saved.to_value();
            let fields = match &value {
                toml::Value::Table(t) => {
                    t.iter().map(|(k, v)| (k.clone(), field_text(v))).collect()
                }
                v => vec![("query".to_string(), field_text(v))],
            };
            let out = ShowResult {
                screen: screen.as_str(),
                name: name.to_string(),
                summary: saved.summary(),
                filter: json_of(&value),
                fields,
            };
            emit(global, &out, &store.profile)
        }
        FiltersCmd::Rm(which) => {
            let (screen, name) = which.parts();
            let default_cleared = store.remove(screen, name)?;
            let out = RmResult {
                screen: screen.as_str(),
                name: name.to_string(),
                default_cleared,
                config_path: store.path.display().to_string(),
            };
            emit(global, &out, &store.profile)
        }
        FiltersCmd::Default(which) => {
            let (screen, name) = which.parts();
            let previous = store.set_default(screen, name)?;
            let out = DefaultResult {
                screen: screen.as_str(),
                default: name.map(String::from),
                previous,
                config_path: store.path.display().to_string(),
            };
            emit(global, &out, &store.profile)
        }
    }
}
