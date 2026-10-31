use super::*;
use crate::tui::model::StatusKind;
use crate::tui::news::NewsView;
use crate::tui::test_support::{fixture_index, model_fixture, render_to_string};
use crate::updates::Release;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn kc(code: KeyCode) -> Msg {
    Msg::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn long_news() -> NewsView {
    NewsView {
        releases: vec![Release {
            version: "0.5.9".into(),
            date: Some("2026-11-02".into()),
            body: (0..60).map(|i| format!("- item {i}\n")).collect(),
        }],
        note: None,
        warning: None,
    }
}

#[test]
fn news_asks_the_runtime_and_opens_on_the_answer() {
    let mut m = model_fixture();
    assert_eq!(
        update(&mut m, Msg::Command(Command::News)),
        vec![Cmd::LoadNews]
    );
    assert!(m.status.text.contains("news"), "{}", m.status.text);
    // A second one while the first is out asks nothing.
    assert!(update(&mut m, Msg::Command(Command::News)).is_empty());
    assert!(m.status.text.contains("still reading"), "{}", m.status.text);
    assert!(update(&mut m, Msg::NewsLoaded(long_news())).is_empty());
    assert_eq!(m.overlay, Overlay::News { scroll: 0 });
    assert_eq!(m.news, long_news());
    assert!(!m.news_loading);
    update(&mut m, kc(KeyCode::Esc));
    assert_eq!(
        update(&mut m, Msg::Command(Command::News)),
        vec![Cmd::LoadNews]
    );
}

#[test]
fn a_note_is_information_and_help_lists_the_scroll_keys() {
    let mut m = model_fixture();
    let view = NewsView {
        note: Some("illumio-tui 0.5.8: the notes of this version".into()),
        ..long_news()
    };
    update(&mut m, Msg::NewsLoaded(view));
    assert_eq!(m.status.kind, StatusKind::Info);
    update(&mut m, kc(KeyCode::Char('?')));
    assert_eq!(
        crate::tui::help::HelpContext::of(&m),
        crate::tui::help::HelpContext::News
    );
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("scroll a long text"), "{out}");
}

#[test]
fn news_scrolls_within_its_lines_and_esc_closes() {
    let mut m = model_fixture();
    update(&mut m, Msg::NewsLoaded(long_news()));
    let last = crate::tui::news::last_scroll(&m.news, m.size);
    assert!(last > 0);
    update(&mut m, kc(KeyCode::Char('j')));
    assert_eq!(m.overlay, Overlay::News { scroll: 1 });
    update(&mut m, kc(KeyCode::Char('k')));
    update(&mut m, kc(KeyCode::Char('k')));
    assert_eq!(m.overlay, Overlay::News { scroll: 0 }, "the top stops");
    update(&mut m, kc(KeyCode::Char('G')));
    assert_eq!(m.overlay, Overlay::News { scroll: last });
    update(&mut m, kc(KeyCode::PageDown));
    assert_eq!(m.overlay, Overlay::News { scroll: last }, "the end stops");
    update(&mut m, kc(KeyCode::Char('g')));
    assert_eq!(m.overlay, Overlay::News { scroll: 0 });
    update(&mut m, kc(KeyCode::Char('?')));
    assert_eq!(m.overlay, Overlay::Help);
    update(&mut m, kc(KeyCode::Esc));
    assert_eq!(
        m.overlay,
        Overlay::News { scroll: 0 },
        "the help returns to it"
    );
    update(&mut m, kc(KeyCode::Esc));
    assert_eq!(m.overlay, Overlay::None);
}

#[test]
fn news_never_opens_over_another_overlay() {
    let mut m = model_fixture();
    m.overlay = Overlay::Marks;
    update(&mut m, Msg::NewsLoaded(long_news()));
    assert_eq!(m.overlay, Overlay::Marks);
    assert!(m.status.text.contains(":news"), "{}", m.status.text);
}

#[test]
fn an_unreachable_github_is_a_warning() {
    let mut m = model_fixture();
    let view = NewsView {
        releases: Vec::new(),
        note: None,
        warning: Some("cannot reach GitHub: timeout; showing only the built-in notes".into()),
    };
    update(&mut m, Msg::NewsLoaded(view));
    assert_eq!(m.status.kind, StatusKind::Warn);
}

#[test]
fn a_newer_release_shows_in_the_header() {
    let mut m = model_fixture();
    assert!(!render_to_string(&m, 80, 24).contains('↑'));
    assert!(update(&mut m, Msg::UpdateFound("0.5.9".into())).is_empty());
    let out = render_to_string(&m, 80, 24);
    assert!(
        out.lines().next().unwrap().contains("↑ 0.5.9 · api"),
        "{out}"
    );
}

#[test]
fn updated_to_is_said_once_at_the_first_ready() {
    let mut m = model_fixture();
    m.updated_to = Some("0.5.8".into());
    let ready = || Msg::IndexLoaded {
        index: fixture_index(),
        age_secs: 0,
    };
    let cmds = update(&mut m, ready());
    assert_eq!(m.status.text, "updated to 0.5.8 · :news for what's new");
    assert_eq!(m.status.kind, StatusKind::Info);
    // Recorded once shown, not before.
    assert!(
        cmds.contains(&Cmd::RecordVersion("0.5.8".into())),
        "{cmds:?}"
    );
    let cmds = update(&mut m, ready());
    assert_eq!(m.status.text, "ready");
    assert!(!cmds.iter().any(|c| matches!(c, Cmd::RecordVersion(_))));
}
