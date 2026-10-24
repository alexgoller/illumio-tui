//! Saved filters in the TUI: `:save` (asking before it replaces one),
//! `:filters` (apply, delete), `:load` and `:filter NAME` through each
//! screen's `f` path, the `[name]` tag until an edit, and a profile
//! switch bringing the new profile's filters. Every config file is a
//! temporary one.
use crate::filters::{Saved, Screen as FilterScreen, Store};
use crate::tui::commands::parse;
use crate::tui::model::{Model, Overlay, PendingAction, ProfileInfo, Screen, StatusKind};
use crate::tui::msg::{Cmd, Msg};
use crate::tui::test_support::{
    model_with_events, model_with_flows, model_with_policy, model_with_workloads, render_to_string,
};
use crate::tui::update;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::PathBuf;

const CONFIG: &str = r#"
default_profile = "demo"

[profiles.demo]
host = "pce.example"
org_id = 5
api_key = "k"
api_secret = "s3cr3t-tui-70"
future_key = "kept"

[profiles.demo.filters.traffic.web]
src = "app=web"
dst = "!app=backup"
service = "443/tcp | 8000-8100/tcp"
decisions = ["blocked", "potentially_blocked"]

[profiles.demo.filters.traffic.month]
src = "app=web"
window = "30d"

[profiles.demo.filters.traffic.broken]
src = "fqdn:x.example"

[profiles.demo.filters.policy]
deny-prod = "dst:env=prod deny"

[profiles.demo.filters.events.agents]
type = "agent"
window = "24h"

[profiles.demo.filters.events.week]
severity = "err"
window = "7d"

[profiles.demo.filters.workloads.prod-idle]
labels = "env=prod"
enforcement = "idle"

[profiles.lab]
host = "lab.example"
org_id = 1

[profiles.lab.filters.traffic.lab-only]
src = "env=lab"
"#;

fn cmd(m: &mut Model, text: &str) -> Vec<Cmd> {
    update(m, Msg::Command(parse(text).unwrap()))
}

fn key(c: char) -> Msg {
    Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

fn kc(code: KeyCode) -> Msg {
    Msg::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

/// `m` reading `CONFIG` from a fresh temporary file.
fn with_config(mut m: Model) -> (Model, tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, CONFIG).unwrap();
    m.saved.path = Ok(path.clone());
    (m, dir, path)
}

fn store(path: &std::path::Path, profile: &str) -> Store {
    Store::new(path.to_path_buf(), profile)
}

fn runs_one_query(cmds: &[Cmd]) -> bool {
    cmds.len() == 1 && matches!(cmds[0], Cmd::RunQuery { .. })
}

#[test]
fn save_writes_the_traffic_filter_in_force_and_asks_before_replacing() {
    let (mut m, _d, path) = with_config(model_with_flows());
    m.traffic.filter.src = "app=api | app=web".into();
    m.traffic.filter.service = "!53/udp".into();
    assert!(cmd(&mut m, "save api").is_empty(), "never a request");
    assert_eq!(m.status.text, "saved traffic filter api");
    let Saved::Traffic(t) = store(&path, "demo")
        .get(FilterScreen::Traffic, "api")
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(t.src.as_deref(), Some("app=api | app=web"));
    assert_eq!(t.service.as_deref(), Some("!53/udp"));
    assert_eq!(t.window.as_deref(), Some("24h"));
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.contains("s3cr3t-tui-70") && text.contains("future_key"),
        "{text}"
    );
    // Saved, it is the applied one.
    assert!(render_to_string(&m, 160, 48).contains("[api]"));

    // The name again: a confirm; `n` keeps the file as it is.
    m.traffic.filter.src = "app=db".into();
    assert!(cmd(&mut m, "save api").is_empty());
    assert!(matches!(
        &m.overlay,
        Overlay::Confirm { action: PendingAction::SaveFilter(n, _), .. } if n == "api"
    ));
    assert!(update(&mut m, key('n')).is_empty());
    assert_eq!(m.overlay, Overlay::None);
    let Saved::Traffic(t) = store(&path, "demo")
        .get(FilterScreen::Traffic, "api")
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(t.src.as_deref(), Some("app=api | app=web"));
    // `y` replaces it.
    cmd(&mut m, "save api");
    assert!(update(&mut m, key('y')).is_empty());
    let Saved::Traffic(t) = store(&path, "demo")
        .get(FilterScreen::Traffic, "api")
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(t.src.as_deref(), Some("app=db"));
    // A bad name is refused before the file is read.
    cmd(&mut m, "save a/b");
    assert_eq!(m.status.kind, StatusKind::Error);
}

#[test]
fn load_runs_one_query_through_the_editor_path_and_the_tag_goes_with_an_edit() {
    let (mut m, _d, _p) = with_config(model_with_flows());
    let cmds = cmd(&mut m, "load web");
    assert!(runs_one_query(&cmds), "{cmds:?}");
    let f = &m.traffic.filter;
    assert_eq!(f.src, "app=web");
    assert_eq!(f.service, "443/tcp | 8000-8100/tcp");
    assert_eq!(f.decisions_text(), "blocked,potentially_blocked");
    assert_eq!(f.window, "24h", "the window in force: the filter sets none");
    assert_eq!(f.max_results, 1000, "the cap in force");
    assert!(render_to_string(&m, 160, 48).contains("[web]"));
    // Any change to the filter drops the name.
    m.traffic.filter.dst = "app=db".into();
    assert!(!render_to_string(&m, 160, 48).contains("[web]"));
}

#[test]
fn a_saved_window_past_the_threshold_asks_like_the_editor_and_a_bad_one_names_its_field() {
    let (mut m, _d, _p) = with_config(model_with_flows());
    assert!(cmd(&mut m, "load month").is_empty(), "asked first");
    assert!(matches!(
        &m.overlay,
        Overlay::Confirm { action: PendingAction::RunQuery(f), .. } if f.window == "30d"
    ));
    assert!(runs_one_query(&update(&mut m, key('y'))));
    assert!(render_to_string(&m, 160, 48).contains("[month]"));

    let (mut m, _d, _p) = with_config(model_with_flows());
    assert!(cmd(&mut m, "load broken").is_empty());
    assert_eq!(m.status.kind, StatusKind::Error);
    let e = &m.status.text;
    assert!(
        e.contains("profile demo") && e.contains("traffic filter broken") && e.contains("source"),
        "{e}"
    );
    assert!(cmd(&mut m, "load nope").is_empty());
    assert!(
        m.status.text.contains("saved: broken, month, web"),
        "{}",
        m.status.text
    );
}

#[test]
fn filter_name_loads_a_saved_filter_and_an_expression_still_sets_the_source() {
    let (mut m, _d, _p) = with_config(model_with_flows());
    assert!(runs_one_query(&cmd(&mut m, "filter web")));
    assert_eq!(m.traffic.filter.dst, "!app=backup");
    let (mut m, _d, _p) = with_config(model_with_flows());
    assert!(runs_one_query(&cmd(&mut m, "filter app=db")));
    assert_eq!(m.traffic.filter.src, "app=db");
    assert_eq!(m.traffic.filter.dst, "");
}

#[test]
fn the_picker_applies_a_policy_search_locally_and_deletes_after_a_confirm() {
    let (mut m, _d, path) = with_config(model_with_policy());
    m.screen = Screen::Policy;
    assert!(cmd(&mut m, "fl").is_empty());
    let Overlay::FilterPicker(p) = &m.overlay else {
        panic!("{:?}", m.overlay)
    };
    assert_eq!(p.entries.len(), 1);
    assert_eq!(p.entries[0].summary, "dst:env=prod deny");
    let out = render_to_string(&m, 100, 30);
    assert!(
        out.contains("Saved policy filters") && out.contains("deny-prod  dst:env=prod deny"),
        "{out}"
    );
    assert!(
        update(&mut m, kc(KeyCode::Enter)).is_empty(),
        "local, no request"
    );
    assert_eq!(m.overlay, Overlay::None);
    assert!(m.policy.search.is_some());
    assert!(render_to_string(&m, 160, 48).contains("[deny-prod]"));

    // `D`, `n`: back to the picker, nothing deleted.
    cmd(&mut m, "filters");
    assert!(update(&mut m, key('D')).is_empty());
    assert!(matches!(
        &m.overlay,
        Overlay::Confirm { action: PendingAction::DeleteFilter(_, n), .. } if n == "deny-prod"
    ));
    let out = render_to_string(&m, 100, 30);
    assert!(
        out.contains("Delete policy filter deny-prod?") && out.contains("Saved policy f"),
        "the picker stays behind: {out}"
    );
    update(&mut m, key('n'));
    assert!(matches!(m.overlay, Overlay::FilterPicker(_)));
    assert!(
        store(&path, "demo")
            .get(FilterScreen::Policy, "deny-prod")
            .is_ok()
    );
    // `D`, `y`: gone from the file, the last one: the picker closes.
    update(&mut m, key('D'));
    assert!(update(&mut m, key('y')).is_empty());
    assert_eq!(m.status.text, "deleted policy filter deny-prod");
    assert_eq!(m.overlay, Overlay::None);
    assert!(
        store(&path, "demo")
            .get(FilterScreen::Policy, "deny-prod")
            .is_err()
    );
    assert!(!render_to_string(&m, 160, 48).contains("[deny-prod]"));
    cmd(&mut m, "fl");
    assert!(
        m.status.text.contains("no saved policy filters"),
        "{}",
        m.status.text
    );
}

#[test]
fn events_filter_locally_or_ask_once_when_the_window_changes() {
    let (mut m, _d, _p) = with_config(model_with_events());
    m.screen = Screen::Events;
    assert!(cmd(&mut m, "load agents").is_empty(), "same window: local");
    assert_eq!(m.events.filter.event_type, "agent");
    assert!(render_to_string(&m, 160, 48).contains("[agents]"));
    let cmds = cmd(&mut m, "load week");
    assert!(
        cmds.len() == 1 && matches!(cmds[0], Cmd::LoadEvents { .. }),
        "{cmds:?}"
    );
    assert_eq!(
        m.events.filter.event_type, "",
        "a missing type is the default"
    );
    assert_eq!(m.events.filter.window, "7d");
}

#[test]
fn workloads_filters_apply_locally_and_save_back_the_same() {
    let (mut m, _d, path) = with_config(model_with_workloads());
    m.screen = Screen::Workloads;
    assert!(cmd(&mut m, "load prod-idle").is_empty());
    assert_eq!(m.workloads.filters.enforcement, Some("idle"));
    assert_eq!(m.workloads.labels.to_string(), "env=prod");
    assert!(render_to_string(&m, 160, 48).contains("[prod-idle]"));
    update(&mut m, key('E'));
    assert!(
        !render_to_string(&m, 160, 48).contains("[prod-idle]"),
        "edited"
    );
    cmd(&mut m, "save other");
    let Saved::Workloads(w) = store(&path, "demo")
        .get(FilterScreen::Workloads, "other")
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(w.labels.as_deref(), Some("env=prod"));
    assert_eq!(w.enforcement.as_deref(), Some("unmanaged"));
}

#[test]
fn a_profile_switch_brings_its_own_filters_and_drops_the_tags() {
    let (mut m, _d, _p) = with_config(model_with_flows());
    cmd(&mut m, "load web");
    let limits = m.limits.clone();
    update(
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
    assert_eq!(m.saved.applied, Default::default());
    m.screen = Screen::Traffic;
    cmd(&mut m, "fl");
    let Overlay::FilterPicker(p) = &m.overlay else {
        panic!("{:?}", m.overlay)
    };
    let names: Vec<&str> = p.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["lab-only"]);
}

#[test]
fn without_a_config_profile_saved_filters_say_why() {
    let mut m = model_with_flows();
    cmd(&mut m, "save x");
    assert_eq!(m.status.kind, StatusKind::Error);
    assert!(m.status.text.contains("saved filters"), "{}", m.status.text);
    let (mut m, _d, _p) = with_config(model_with_flows());
    m.profile.name = "ghost".into();
    cmd(&mut m, "fl");
    assert!(
        m.status.text.contains("\"ghost\" is not in"),
        "{}",
        m.status.text
    );
    m.screen = Screen::Objects;
    cmd(&mut m, "fl");
    assert!(
        m.status.text.contains("Traffic, Policy"),
        "{}",
        m.status.text
    );
}

#[test]
fn tab_in_the_prompt_completes_the_screens_saved_names() {
    let (mut m, _d, _p) = with_config(model_with_flows());
    update(&mut m, key(':'));
    for c in "load w".chars() {
        update(&mut m, key(c));
    }
    update(&mut m, kc(KeyCode::Tab));
    assert_eq!(
        m.overlay,
        Overlay::Prompt {
            input: "load web".into()
        }
    );
}

#[test]
fn a_wrong_shaped_screen_key_is_said_where_it_is_used() {
    let (mut m, _d, path) = with_config(model_with_events());
    let text = std::fs::read_to_string(&path).unwrap().replace(
        "[profiles.demo.filters.events.agents]",
        "[profiles.demo.filters]\nworkloads = 3\n\n[profiles.demo.filters.events.agents]",
    );
    let text = text.replace(
        "[profiles.demo.filters.workloads.prod-idle]\nlabels = \"env=prod\"\nenforcement = \"idle\"\n",
        "",
    );
    std::fs::write(&path, text).unwrap();
    m.screen = Screen::Workloads;
    cmd(&mut m, "fl");
    assert_eq!(m.status.kind, StatusKind::Error);
    assert!(
        m.status.text.contains("profile demo") && m.status.text.contains("filters.workloads"),
        "{}",
        m.status.text
    );
    m.screen = Screen::Events;
    cmd(&mut m, "fl");
    assert!(
        matches!(m.overlay, Overlay::FilterPicker(_)),
        "other screens work"
    );
}

#[test]
fn an_events_actor_saves_and_loads_and_a_filter_without_one_clears_it() {
    let (mut m, _d, path) = with_config(model_with_events());
    m.screen = Screen::Events;
    m.events
        .table
        .select_where(|e| e.event_type.as_deref() == Some("user.login"));
    assert!(update(&mut m, key('u')).is_empty());
    assert_eq!(m.events.filter.actor, "=user:user-1");
    cmd(&mut m, "save people");
    let Saved::Events(e) = store(&path, "demo")
        .get(FilterScreen::Events, "people")
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(e.actor.as_deref(), Some("=user:user-1"));
    assert!(cmd(&mut m, "load agents").is_empty());
    assert_eq!(m.events.filter.actor, "", "a missing actor is the default");
    assert_eq!(m.events.table.len(), 1);
    assert!(cmd(&mut m, "load people").is_empty(), "local, no request");
    assert_eq!(m.events.filter.actor, "=user:user-1");
    assert_eq!(m.events.table.len(), 1);
    assert!(render_to_string(&m, 160, 48).contains("[people]"));
}
