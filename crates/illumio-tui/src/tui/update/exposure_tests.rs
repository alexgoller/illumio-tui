//! `:exposure` / `9` on the Traffic screen (#82): one query over the risky
//! services' ports in its own cache slot, the Exposure view's rows, Enter
//! into Flows, `E` / `e` explaining a row without a request, squelch, and
//! `Esc` putting back what was on screen without one.
use super::*;
use crate::tui::model::{Drill, Model, Overlay, TrafficView};
use crate::tui::test_support::{
    RICH_MARKS, exposure_flows, model_with_marks, model_with_rich_flows, open_exposure_with,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use pce::traffic::ExposureStatus;

fn key(c: char) -> Msg {
    Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

fn kc(code: KeyCode) -> Msg {
    Msg::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

#[track_caller]
fn quiet(m: &mut Model, msg: Msg) {
    assert_eq!(update(m, msg), Vec::<Cmd>::new());
}

fn rows(m: &Model) -> Vec<(String, String, ExposureStatus)> {
    m.traffic
        .exposure_rows
        .rows()
        .iter()
        .map(|r| (r.service.clone(), r.dst_text(), r.status))
        .collect()
}

#[test]
fn exposure_runs_one_query_in_its_own_slot_and_esc_puts_back_the_result() {
    let mut m = model_with_rich_flows();
    m.traffic.filter.src = "app=hr".into();
    let before = m.traffic.loaded_flows().len();
    let cmds = update(&mut m, Msg::Command(Command::Exposure));
    let [
        Cmd::RunQuery {
            filter, slot, mode, ..
        },
    ] = cmds.as_slice()
    else {
        panic!("one query: {cmds:?}");
    };
    assert_eq!(*slot, pce::EXPOSURE_SLOT);
    assert_eq!(*mode, pce::CacheMode::Use);
    assert_eq!(filter.src, "app=hr", "the sides given stay");
    assert!(
        filter.service.starts_with("3389/tcp | 3389/udp | 445/tcp"),
        "{}",
        filter.service
    );
    assert_eq!(filter.service.matches(" | ").count(), 19, "20 ports");
    assert_eq!(m.traffic.view, TrafficView::Exposure);
    assert!(m.traffic.data.is_none(), "the old result kept aside");

    // Its answer fills the view; exposed rows first.
    let mut m = model_with_rich_flows();
    open_exposure_with(&mut m, exposure_flows());
    let got = rows(&m);
    assert_eq!(
        got[..2],
        [
            ("ssh".into(), "erp/prod".into(), ExposureStatus::Exposed),
            (
                "postgres".into(),
                "shop/prod".into(),
                ExposureStatus::Exposed
            ),
        ],
        "exposed first, then in the list's order"
    );
    assert!(got.windows(2).all(|w| w[0].2 <= w[1].2), "{got:?}");
    // Every key of a Traffic view works on it: `r` re-runs it in its slot.
    let cmds = update(&mut m, key('r'));
    assert!(
        matches!(cmds.as_slice(), [Cmd::RunQuery { slot, .. }] if *slot == pce::EXPOSURE_SLOT),
        "{cmds:?}"
    );
    let mut m = model_with_rich_flows();
    open_exposure_with(&mut m, exposure_flows());
    quiet(&mut m, kc(KeyCode::Esc));
    assert!(m.traffic.exposure.is_none());
    assert_eq!(m.traffic.view, TrafficView::Flows);
    assert_eq!(m.traffic.loaded_flows().len(), before, "no request");
    assert!(m.traffic.filter.service.is_empty());
    let cmds = update(&mut m, key('r'));
    assert!(
        matches!(cmds.as_slice(), [Cmd::RunQuery { slot, .. }] if *slot == "main"),
        "{cmds:?}"
    );
}

#[test]
fn nine_opens_it_and_again_costs_nothing() {
    let mut m = model_with_rich_flows();
    m.screen = crate::tui::model::Screen::Policy;
    let cmds = update(&mut m, key('9'));
    assert!(
        matches!(cmds.as_slice(), [Cmd::RunQuery { .. }]),
        "{cmds:?}"
    );
    assert_eq!(m.screen, crate::tui::model::Screen::Traffic);
    let mut m = model_with_rich_flows();
    open_exposure_with(&mut m, exposure_flows());
    quiet(&mut m, key('v'));
    assert_eq!(m.traffic.view, TrafficView::Flows);
    assert!(
        m.traffic.exposure.is_some(),
        "v leaves the view, not the mode"
    );
    quiet(&mut m, key('9'));
    assert_eq!(m.traffic.view, TrafficView::Exposure);
    assert!(!m.traffic.exposure_rows.is_empty());
}

#[test]
fn enter_drills_into_the_flows_of_the_group_and_service() {
    let mut m = model_with_rich_flows();
    open_exposure_with(&mut m, exposure_flows());
    quiet(&mut m, kc(KeyCode::Enter));
    let t = &m.traffic;
    assert_eq!(t.view, TrafficView::Flows);
    let Some(Drill::Exposure { dst, service }) = &t.drill else {
        panic!("{:?}", t.drill);
    };
    assert_eq!(
        (dst.join("/"), service.name.as_str()),
        ("erp/prod".into(), "ssh")
    );
    assert_eq!(t.drill.as_ref().unwrap().title(), "* → erp/prod ssh");
    assert!(!t.flows.is_empty());
    assert!(
        t.flows
            .rows()
            .iter()
            .all(|r| r.flow.port == Some(22) && &*r.dst_group == "erp/prod")
    );
}

#[test]
fn e_explains_the_rows_exposing_flow_without_a_request() {
    for k in ['e', 'E'] {
        let mut m = model_with_rich_flows();
        open_exposure_with(&mut m, exposure_flows());
        quiet(&mut m, kc(KeyCode::Down));
        quiet(&mut m, key(k));
        let Overlay::Explain(v) = &m.overlay else {
            panic!("{k}: {:?}", m.overlay);
        };
        assert_eq!(v.flow.port, Some(5432));
        assert_eq!(
            v.flow.src.label_value("app"),
            Some("hr"),
            "the exposing flow, not shop's own"
        );
    }
}

#[test]
fn a_malformed_list_is_a_status_and_no_request() {
    let mut m = model_with_rich_flows();
    m.limits.exposure_services = Err("exposure.extra.nfs: \"2049\": give a port".into());
    quiet(&mut m, Msg::Command(Command::Exposure));
    assert!(m.traffic.exposure.is_none());
    assert_eq!(m.status.kind, StatusKind::Error);
    assert!(
        m.status.text.contains("exposure.extra.nfs"),
        "{}",
        m.status.text
    );
}

#[test]
fn squelched_flows_leave_the_rows() {
    let dir = tempfile::tempdir().unwrap();
    let mut m = model_with_marks(dir.path(), RICH_MARKS);
    open_exposure_with(&mut m, exposure_flows());
    let got = rows(&m);
    assert!(
        !got.iter().any(|(s, d, _)| s == "ssh" && d == "erp/prod"),
        "Corp's SSH is squelched: {got:?}"
    );
    quiet(&mut m, key('M'));
    assert!(
        rows(&m)
            .iter()
            .any(|(s, d, st)| s == "ssh" && d == "erp/prod" && *st == ExposureStatus::Exposed),
        "M shows it again"
    );
}
