//! The marks' keys on the Traffic screen: `M` toggles squelched rows with
//! a rebuild and never a request; a search being typed takes the key.
use crate::tui::model::TrafficView;
use crate::tui::msg::{Cmd, Msg};
use crate::tui::traffic_marks::tests::{RICH_MARKS, marked_model};
use crate::tui::update;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn key(c: char) -> Msg {
    Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

#[test]
fn m_toggles_squelched_rows_in_every_view_without_a_request() {
    let (mut m, _dir) = marked_model(RICH_MARKS);
    for view in [
        TrafficView::Flows,
        TrafficView::Aggregate,
        TrafficView::Mesh,
        TrafficView::Blocked,
        TrafficView::Treemap,
    ] {
        m.traffic.view = view;
        let before = m.traffic.show_squelched;
        assert_eq!(update(&mut m, key('M')), Vec::<Cmd>::new(), "{view:?}");
        assert_eq!(m.traffic.show_squelched, !before);
    }
    assert!(m.status.text.contains("M "), "{}", m.status.text);
}

#[test]
fn a_search_being_typed_takes_m() {
    let (mut m, _dir) = marked_model(RICH_MARKS);
    assert!(update(&mut m, key('/')).is_empty());
    assert!(update(&mut m, key('M')).is_empty());
    assert!(!m.traffic.show_squelched);
    assert_eq!(m.traffic.flows.search_text(), "M");
}

#[test]
fn the_app_overlay_leaves_out_squelched_peers_and_m_toggles_them() {
    use crate::tui::app_view::AppPanel;
    use crate::tui::commands::Command;
    use crate::tui::test_support::{render_to_string, with_policy};
    let (m, _dir) = marked_model(RICH_MARKS);
    let mut m = with_policy(m);
    update(&mut m, Msg::Command(Command::App(Some("shop/prod".into()))));
    let v = m.app_view.as_ref().unwrap();
    assert!(v.marks_on);
    assert!(
        v.inbound.iter().all(|p| p.peer_text() != "shop/prod"),
        "shop's 5432 into itself is squelched"
    );
    assert_eq!(
        v.panel_title(AppPanel::Inbound),
        "Inbound · 2 peers (5 flows) squelched"
    );
    assert_eq!(
        v.panel_title(AppPanel::Outbound),
        "Outbound · 1 peers (4 flows) squelched"
    );
    let out = render_to_string(&m, 120, 40);
    assert!(out.contains("● ipl:Corp/-"), "{out}");
    let before = v.inbound.len();
    assert_eq!(update(&mut m, key('M')), Vec::<Cmd>::new());
    let v = m.app_view.as_ref().unwrap();
    assert_eq!(v.inbound.len(), before + 2);
    assert_eq!(
        v.panel_title(AppPanel::Inbound),
        "Inbound · 2 peers (5 flows) squelched, shown ~"
    );
    assert!(m.traffic.show_squelched, "the Traffic screen follows");
}

#[test]
fn the_generator_counts_squelched_flows_and_its_rules_include_them() {
    use crate::tui::commands::Command;
    use crate::tui::test_support::{model_with_rich_flows, with_policy};
    let open = |m: &mut crate::tui::model::Model| {
        update(m, Msg::Command(Command::App(Some("shop/prod".into()))));
        assert_eq!(update(m, key('G')), Vec::<Cmd>::new());
    };
    let (m, _dir) = marked_model(RICH_MARKS);
    let mut marked = with_policy(m);
    open(&mut marked);
    let mut plain = with_policy(model_with_rich_flows());
    open(&mut plain);
    let (a, b) = (
        marked.gen_view.as_ref().unwrap(),
        plain.gen_view.as_ref().unwrap(),
    );
    assert!(
        a.header_title()
            .ends_with(" · 5 squelched (rules include them)"),
        "{}",
        a.header_title()
    );
    assert!(!b.header_title().contains("squelched"));
    assert_eq!(a.rows, b.rows, "marks are not policy");
    assert_eq!(a.proposal, b.proposal);
}

#[test]
fn the_app_overlay_and_the_generator_pick_up_an_edited_marks_file() {
    use crate::tui::commands::Command;
    use crate::tui::test_support::with_policy;
    let (m, dir) = marked_model("");
    let mut m = with_policy(m);
    assert!(m.traffic.marked.is_none(), "no marks");
    // Edited in another editor while the TUI runs: no traffic rebuild yet.
    std::fs::write(dir.path().join("marks.toml"), RICH_MARKS).unwrap();
    update(&mut m, Msg::Command(Command::App(Some("shop/prod".into()))));
    assert!(m.app_view.as_ref().unwrap().marks_on, "opening reads it");
    assert!(m.traffic.marked.is_some(), "the Traffic screen follows");
    std::fs::write(dir.path().join("marks.toml"), "").unwrap();
    assert_eq!(update(&mut m, key('G')), Vec::<Cmd>::new());
    assert!(m.traffic.marked.is_none(), "the generator reads it too");
    assert!(
        !m.gen_view
            .as_ref()
            .unwrap()
            .header_title()
            .contains("squelched")
    );
}
