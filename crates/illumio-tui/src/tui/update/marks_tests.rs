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
