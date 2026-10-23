//! A default saved filter per screen: in force before any screen loads (at
//! startup and after `:profile`) without one more request, through the
//! traffic cost confirm when the user runs it, ignored with a warning when
//! it does not resolve; `*` in the picker and `:default` set it. Every
//! config file is a temporary one.
use crate::filters::{Screen as FilterScreen, Store};
use crate::tui::commands::parse;
use crate::tui::model::{Connection, WritesState};
use crate::tui::model::{Model, Overlay, PendingAction, ProfileInfo, Screen, StatusKind};
use crate::tui::msg::{Cmd, Msg};
use crate::tui::test_support::{fixture_index, model_fixture, render_to_string};
use crate::tui::update;
use crate::tui::update::apply_defaults;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::PathBuf;

const CONFIG: &str = r#"
default_profile = "demo"

[profiles.demo]
host = "pce.example"
org_id = 5
api_key = "k"
api_secret = "s3cr3t-tui-74"

[profiles.demo.filter_defaults]
traffic = "web"
policy = "deny-prod"
events = "agents"
workloads = "prod-idle"
future_default = "kept"

[profiles.demo.filters.traffic.web]
src = "app=web"
service = "443/tcp"

[profiles.demo.filters.traffic.month]
src = "app=web"
window = "30d"

[profiles.demo.filters.traffic.broken]
src = "fqdn:x.example"

[profiles.demo.filters.policy]
deny-prod = "dst:env=prod deny"

[profiles.demo.filters.events.agents]
type = "agent"
severity = "err"
window = "7d"

[profiles.demo.filters.workloads.prod-idle]
labels = "env=prod"
enforcement = "idle"

[profiles.lab]
host = "lab.example"
org_id = 1

[profiles.lab.filter_defaults]
traffic = "lab-only"

[profiles.lab.filters.traffic.lab-only]
src = "env=lab"
window = "1h"
"#;

fn cmd(m: &mut Model, text: &str) -> Vec<Cmd> {
    update(m, Msg::Command(parse(text).unwrap()))
}

fn key(c: char) -> Msg {
    Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

/// `CONFIG`, edited by `edit`, in a fresh temporary file.
fn config(edit: impl Fn(String) -> String) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, edit(CONFIG.to_string())).unwrap();
    (dir, path)
}

/// A model as `app::run` builds it: profile `demo`, still connecting.
fn starting(path: Option<&PathBuf>) -> Model {
    let mut m = Model::new_connecting(
        ProfileInfo {
            name: "demo".into(),
            host: "pce.example".into(),
            org_id: 5,
        },
        "dark".into(),
        60,
        1000,
        "24h".into(),
        WritesState::default(),
    );
    if let Some(p) = path {
        m.saved.path = Ok(p.clone());
    }
    m
}

/// The variant names of `cmds` (`LoadIndex`, `LoadEvents`, ...).
fn kinds(cmds: &[Cmd]) -> Vec<String> {
    cmds.iter()
        .map(|c| {
            let d = format!("{c:?}");
            d.split([' ', '(', '{']).next().unwrap_or("").to_string()
        })
        .collect()
}

/// Startup, then every screen visited once: the commands each step gives.
fn session(m: &mut Model) -> Vec<Vec<String>> {
    let mut steps = vec![kinds(&update(m, Msg::Connected))];
    steps.push(kinds(&update(
        m,
        Msg::IndexLoaded {
            index: fixture_index(),
            age_secs: 0,
        },
    )));
    for s in [
        Screen::Events,
        Screen::Workloads,
        Screen::Policy,
        Screen::Objects,
        Screen::Traffic,
    ] {
        steps.push(kinds(&update(m, Msg::SwitchScreen(s))));
    }
    steps
}

#[test]
fn defaults_are_in_force_at_startup_and_add_no_request() {
    let (_d, path) = config(|c| c);
    let mut plain = starting(None);
    apply_defaults(&mut plain);
    let mut m = starting(Some(&path));
    apply_defaults(&mut m);
    // Before anything loads: each screen's filter is its default's.
    assert_eq!(m.traffic.filter.src, "app=web");
    assert_eq!(m.traffic.filter.service, "443/tcp");
    assert_eq!(
        m.traffic.filter.window, "24h",
        "a default without one keeps it"
    );
    assert_eq!(
        m.policy.search.as_ref().map(ToString::to_string).as_deref(),
        Some("dst:env=prod deny")
    );
    assert_eq!(m.events.filter.event_type, "agent");
    assert_eq!(m.events.filter.window, "7d");
    assert_eq!(m.workloads.labels.to_string(), "env=prod");
    assert_eq!(m.workloads.filters.enforcement, Some("idle"));
    assert!(m.saved.warning.is_none());
    // The same commands, step by step, as without defaults: one events
    // load, one workloads load, one rulesets load, and no traffic query.
    let with = session(&mut m);
    assert_eq!(with, session(&mut plain));
    assert!(!with.iter().flatten().any(|k| k == "RunQuery"), "{with:?}");
    assert_eq!(
        with.iter().flatten().filter(|k| *k == "LoadEvents").count(),
        1
    );
    // The first loads went out with the defaults in force, tagged.
    let Some((_, ev)) = &m.saved.applied.events else {
        panic!()
    };
    assert_eq!(ev.severity, "err");
    m.screen = Screen::Traffic;
    assert!(render_to_string(&m, 160, 48).contains("[web]"));
    m.screen = Screen::Workloads;
    assert!(render_to_string(&m, 160, 48).contains("[prod-idle]"));
}

#[test]
fn the_traffic_default_runs_only_when_asked_and_asks_first_when_expensive() {
    let (_d, path) = config(|c| c.replace("traffic = \"web\"", "traffic = \"month\""));
    let mut m = starting(Some(&path));
    apply_defaults(&mut m);
    assert_eq!(m.traffic.filter.window, "30d");
    update(&mut m, Msg::Connected);
    update(
        &mut m,
        Msg::IndexLoaded {
            index: fixture_index(),
            age_secs: 0,
        },
    );
    // `r`: the cost confirm, as for `f` + Enter; nothing sent before `y`.
    assert!(update(&mut m, key('r')).is_empty());
    assert!(
        matches!(
            &m.overlay,
            Overlay::Confirm { reason, action: PendingAction::RunNow(f) }
                if reason.contains("window 30d") && f.window == "30d"
        ),
        "{:?}",
        m.overlay
    );
    update(&mut m, key('n'));
    assert!(m.traffic.data.is_none());
    // A cheap default: `r` is its one query.
    let (_d2, path) = config(|c| c);
    let mut m = starting(Some(&path));
    apply_defaults(&mut m);
    update(&mut m, Msg::Connected);
    let cmds = update(&mut m, key('r'));
    assert_eq!(kinds(&cmds), vec!["RunQuery"]);
}

#[test]
fn a_missing_or_broken_default_is_ignored_with_a_warning() {
    let (_d, path) = config(|c| {
        c.replace("traffic = \"web\"", "traffic = \"broken\"")
            .replace("policy = \"deny-prod\"", "policy = \"gone\"")
            .replace("events = \"agents\"", "events = 3")
    });
    let mut m = starting(Some(&path));
    let before = m.traffic.filter.clone();
    apply_defaults(&mut m);
    assert_eq!(m.traffic.filter, before, "ignored");
    assert!(m.policy.search.is_none());
    assert_eq!(m.events.filter.window, "24h");
    assert_eq!(
        m.workloads.labels.to_string(),
        "env=prod",
        "the good one applies"
    );
    assert_eq!(m.status.kind, StatusKind::Warn);
    let w = m.status.text.clone();
    for part in [
        "profile demo",
        "default traffic filter \"broken\"",
        "source",
        "default policy filter \"gone\"",
        "filter_defaults.events",
        "(ignored)",
    ] {
        assert!(w.contains(part), "{part}: {w}");
    }
    // The connection's messages come first; `ready` says it again, once.
    update(&mut m, Msg::Connected);
    update(
        &mut m,
        Msg::IndexLoaded {
            index: fixture_index(),
            age_secs: 0,
        },
    );
    assert_eq!(
        (m.status.kind, m.status.text.as_str()),
        (StatusKind::Warn, w.as_str())
    );
    assert!(m.saved.warning.is_none());
    // Without a config file profile (an `--env-file` one) there are none.
    let mut m = starting(None);
    apply_defaults(&mut m);
    assert!(m.saved.warning.is_none() && m.traffic.filter.src.is_empty());
}

#[test]
fn a_profile_switch_applies_the_new_profiles_defaults_before_connecting() {
    let (_d, path) = config(|c| c);
    let mut m = model_fixture();
    m.saved.path = Ok(path);
    let limits = m.limits.clone();
    let cmds = update(
        &mut m,
        Msg::ProfileSwitched(Ok((
            ProfileInfo {
                name: "lab".into(),
                host: "lab.example".into(),
                org_id: 1,
            },
            limits,
        ))),
    );
    assert_eq!(kinds(&cmds), vec!["Connect"], "nothing more");
    assert!(matches!(m.connection, Connection::Connecting));
    assert_eq!(m.traffic.filter.src, "env=lab");
    assert_eq!(m.traffic.filter.window, "1h");
    assert!(m.policy.search.is_none(), "demo's defaults are gone");
    assert!(m.events.filter.event_type.is_empty());
    assert_eq!(
        m.saved.applied.traffic.as_ref().map(|t| t.0.as_str()),
        Some("lab-only")
    );
}

#[test]
fn events_defaults_apply_on_top_of_the_profiles_hidden_types() {
    let (_d, path) = config(|c| c);
    let mut m = starting(Some(&path));
    m.limits.event_exclude_types = vec!["=user.login".into(), "agent.*".into()];
    m.events.filter.event_type = m.limits.event_type_filter();
    apply_defaults(&mut m);
    assert_eq!(m.events.filter.event_type, "agent, !=user.login, !agent.*");
    // `:load` is the same.
    let mut m = model_fixture();
    m.saved.path = Ok(path);
    m.limits.event_exclude_types = vec!["=user.login".into()];
    m.screen = Screen::Events;
    cmd(&mut m, "load agents");
    assert_eq!(m.events.filter.event_type, "agent, !=user.login");
}

#[test]
fn star_in_the_picker_sets_and_unsets_the_default_in_the_file() {
    let (_d, path) = config(|c| c);
    let mut m = model_fixture();
    m.saved.path = Ok(path.clone());
    m.screen = Screen::Traffic;
    cmd(&mut m, "fl");
    let Overlay::FilterPicker(p) = &m.overlay else {
        panic!("{:?}", m.overlay)
    };
    assert_eq!(p.default.as_deref(), Some("web"));
    let shown = render_to_string(&m, 160, 48);
    assert!(
        shown.contains("* web") && shown.contains("* default"),
        "{shown}"
    );
    // Sorted broken, month, web: `month` is the second; select it, `*`.
    update(
        &mut m,
        Msg::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
    );
    assert!(update(&mut m, key('*')).is_empty(), "never a request");
    assert_eq!(
        m.status.text,
        "traffic default: month (applied at startup and after :profile)"
    );
    let defaults = |p: &PathBuf| {
        Store::new(p.clone(), "demo")
            .load_with_defaults()
            .unwrap()
            .1
    };
    assert_eq!(
        defaults(&path).get(FilterScreen::Traffic),
        Ok(Some("month"))
    );
    assert!(
        defaults(&path).0.contains_key("future_default"),
        "unknown keys kept"
    );
    let Overlay::FilterPicker(p) = &m.overlay else {
        panic!("the picker stays")
    };
    assert_eq!(p.default.as_deref(), Some("month"));
    // Again on the default: unset.
    update(&mut m, key('*'));
    assert_eq!(m.status.text, "traffic default month cleared");
    assert_eq!(defaults(&path).get(FilterScreen::Traffic), Ok(None));
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("s3cr3t-tui-74"), "{text}");
}

#[test]
fn default_command_sets_and_clears_the_screens_default() {
    let (_d, path) = config(|c| c);
    let mut m = model_fixture();
    m.saved.path = Ok(path.clone());
    m.screen = Screen::Events;
    let store = Store::new(path, "demo");
    cmd(&mut m, "save week");
    assert!(cmd(&mut m, "default week").is_empty());
    assert_eq!(
        store
            .load_with_defaults()
            .unwrap()
            .1
            .get(FilterScreen::Events),
        Ok(Some("week"))
    );
    cmd(&mut m, "default off");
    assert_eq!(m.status.text, "events default week cleared");
    cmd(&mut m, "default off");
    assert_eq!(m.status.text, "no events default to clear");
    cmd(&mut m, "default nope");
    assert_eq!(m.status.kind, StatusKind::Error);
    assert!(
        m.status.text.contains("no events filter \"nope\""),
        "{}",
        m.status.text
    );
    // Nothing applies now: the filter in force is the session's.
    m.screen = Screen::Objects;
    cmd(&mut m, "default x");
    assert_eq!(m.status.kind, StatusKind::Error);
    // `Tab` completes the name after `default `.
    m.screen = Screen::Traffic;
    update(&mut m, key(':'));
    for c in "default mo".chars() {
        update(&mut m, key(c));
    }
    update(
        &mut m,
        Msg::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
    );
    assert_eq!(
        m.overlay,
        Overlay::Prompt {
            input: "default month".into()
        }
    );
}

#[test]
fn deleting_the_default_filter_clears_the_default_and_says_so() {
    let (_d, path) = config(|c| c);
    let mut m = model_fixture();
    m.saved.path = Ok(path.clone());
    m.screen = Screen::Workloads;
    cmd(&mut m, "fl");
    update(&mut m, key('D'));
    update(&mut m, key('y'));
    assert_eq!(
        m.status.text,
        "deleted workloads filter prod-idle; it was the default, now there is none"
    );
    let (_, d) = Store::new(path, "demo").load_with_defaults().unwrap();
    assert_eq!(d.get(FilterScreen::Workloads), Ok(None));
    assert_eq!(d.get(FilterScreen::Traffic), Ok(Some("web")));
}
