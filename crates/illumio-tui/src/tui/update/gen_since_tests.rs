//! "New since" in the generator (#81): `N` and its prompt, the cost
//! guard, the one baseline query (both sides with `o`), the header, the
//! rows and rules from the new records only, `N` again leaving without a
//! request, `o` or another filter ending it. Every request is the user's.
use super::*;
use crate::tui::model::{Model, Overlay, PendingAction};
use crate::tui::msg::GenBaselineOutcome;
use crate::tui::test_support::{model_with_window, with_policy};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use pce::traffic::ResolvedFlow;

fn key(c: char) -> Msg {
    Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

fn shift_n() -> Msg {
    Msg::Key(KeyEvent::new(KeyCode::Char('N'), KeyModifiers::SHIFT))
}

fn ctrl(c: char) -> Msg {
    Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
}

fn code(c: KeyCode) -> Msg {
    Msg::Key(KeyEvent::new(c, KeyModifiers::NONE))
}

#[track_caller]
fn quiet(m: &mut Model, msg: Msg) {
    assert_eq!(update(m, msg), Vec::<Cmd>::new());
}

/// The generator open for shop/prod on the rich flows of the last 24h,
/// the rulesets loaded: nothing sent.
fn opened() -> Model {
    let mut m = with_policy(model_with_window("24h"));
    quiet(
        &mut m,
        Msg::Command(crate::tui::commands::Command::Gen(Some("shop/prod".into()))),
    );
    m
}

fn view(m: &Model) -> &crate::tui::gen_view::GenView {
    m.gen_view.as_ref().expect("the generator")
}

fn peers(m: &Model) -> Vec<String> {
    view(m).rows.iter().map(|r| r.peer_text()).collect()
}

/// The app's intra-app records (shop to shop): the baseline saw them.
fn intra(m: &Model) -> Vec<ResolvedFlow> {
    let shop = |e: &pce::traffic::ResolvedEndpoint| {
        e.label_value("app") == Some("shop") && e.label_value("env") == Some("prod")
    };
    m.traffic
        .flows_for(&view(m).focus)
        .iter()
        .filter(|f| shop(&f.src) && shop(&f.dst))
        .cloned()
        .collect()
}

/// `N`, Enter on `7d`: the one baseline query, its `seq`.
fn run_baseline(m: &mut Model) -> (u64, Cmd) {
    quiet(m, shift_n());
    let cmds = update(m, code(KeyCode::Enter));
    let [cmd @ Cmd::RunGenBaseline { seq, .. }] = &cmds[..] else {
        panic!("{cmds:?} {}", m.status.text)
    };
    (*seq, cmd.clone())
}

fn answer(m: &mut Model, seq: u64, flows: Vec<ResolvedFlow>, truncated: bool) {
    quiet(
        m,
        Msg::GenBaselineDone {
            seq,
            result: Ok(GenBaselineOutcome {
                flows,
                truncated,
                incomplete: false,
                hit: false,
                queries: 1,
            }),
        },
    );
}

#[test]
fn n_prompts_and_enter_sends_one_focused_baseline_before_the_window() {
    let mut m = opened();
    quiet(&mut m, shift_n());
    assert_eq!(m.overlay, Overlay::GenBaseline { input: "7d".into() });
    quiet(&mut m, ctrl('n'));
    quiet(&mut m, ctrl('p'));
    assert_eq!(
        m.overlay,
        Overlay::GenBaseline { input: "1w".into() },
        "Ctrl-N/P step through the window steps, as Traffic c"
    );
    // Esc goes back without a request.
    quiet(&mut m, code(KeyCode::Esc));
    assert_eq!(m.overlay, Overlay::Generator);
    let (_, cmd) = run_baseline(&mut m);
    let Cmd::RunGenBaseline {
        filter,
        focus,
        start,
        end,
        ..
    } = cmd
    else {
        unreachable!()
    };
    let range = m.traffic.data.as_ref().unwrap().range.unwrap();
    assert_eq!(
        end, range.start,
        "the baseline ends where the window starts"
    );
    assert_eq!(
        start,
        range
            .start
            .checked_sub(jiff::Span::new().hours(7 * 24))
            .unwrap()
    );
    assert_eq!(filter.window, "7d");
    assert_eq!(focus.app_text(), "shop/prod");
    assert_eq!(focus.direction, FocusDirection::In, "inbound only");
    assert_eq!(m.overlay, Overlay::Generator);
    let v = view(&m);
    assert_eq!(v.requests, 1, "counted");
    assert!(
        v.header_title()
            .ends_with("· incomplete · new since 7d before · baseline running"),
        "{}",
        v.header_title()
    );
    assert!(m.traffic.is_running());
    // Running: Enter plans nothing.
    m.writes = crate::tui::model::WritesState::on();
    quiet(&mut m, code(KeyCode::Enter));
    assert_eq!(
        m.status.text,
        "a query is running: Enter plans once it lands"
    );
}

#[test]
fn the_answer_leaves_out_what_the_baseline_saw_and_n_again_brings_it_back() {
    let mut m = opened();
    let before = peers(&m);
    assert!(before.contains(&"intra-app".to_string()));
    let (seq, _) = run_baseline(&mut m);
    let seen = intra(&m);
    assert_eq!(seen.len(), 4);
    answer(&mut m, seq, seen, false);
    assert!(!m.traffic.is_running());
    let v = view(&m);
    assert!(
        v.header_title()
            .ends_with("· complete · new since 7d before · 4 flows seen before"),
        "{}",
        v.header_title()
    );
    assert!(
        !peers(&m).contains(&"intra-app".to_string()),
        "{:?}",
        peers(&m)
    );
    let p = v.proposal.as_ref().unwrap();
    assert_eq!(p.coverage.seen_before, 4);
    assert!(v.footer().contains("of 8 flows"), "{}", v.footer());
    assert!(
        v.rule_list(m.index().unwrap())
            .last()
            .unwrap()
            .starts_with("not covered: 4 seen before · "),
    );
    assert_eq!(m.status.text, "new since 7d: 4 seen before");
    // `N` again: every flow, no request.
    quiet(&mut m, shift_n());
    assert!(view(&m).since.is_none());
    assert_eq!(peers(&m), before);
    assert_eq!(view(&m).proposal.as_ref().unwrap().coverage.seen_before, 0);
    assert_eq!(m.status.text, "new since off: every flow of the window");
}

#[test]
fn a_truncated_baseline_warns_in_the_header() {
    let mut m = opened();
    let (seq, _) = run_baseline(&mut m);
    answer(&mut m, seq, Vec::new(), true);
    assert!(
        view(&m)
            .header_title()
            .ends_with("0 flows seen before · baseline truncated: new over-reports")
    );
    assert_eq!(m.status.kind, crate::tui::model::StatusKind::Warn);
}

#[test]
fn n_again_while_running_cancels_and_a_late_answer_is_dropped() {
    let mut m = opened();
    let (seq, _) = run_baseline(&mut m);
    quiet(&mut m, shift_n());
    assert!(!m.traffic.is_running(), "cancelled");
    assert!(view(&m).since.is_none());
    answer(&mut m, seq, Vec::new(), false);
    assert!(view(&m).since.is_none(), "stale");
}

#[test]
fn past_the_cost_threshold_it_asks_first() {
    let mut m = opened();
    quiet(&mut m, shift_n());
    quiet(&mut m, ctrl('u'));
    for c in "30d".chars() {
        quiet(&mut m, key(c));
    }
    quiet(&mut m, code(KeyCode::Enter));
    let Overlay::Confirm { reason, action } = &m.overlay else {
        panic!("{:?}", m.overlay)
    };
    assert!(
        reason.ends_with("Run the 30d baseline for shop/prod (1 query)? (y/n)"),
        "{reason}"
    );
    assert!(matches!(action, PendingAction::GenBaseline { .. }));
    quiet(&mut m, key('n'));
    assert_eq!(m.overlay, Overlay::Generator, "back, nothing sent");
    assert!(view(&m).since.is_none());
    quiet(&mut m, shift_n());
    quiet(&mut m, ctrl('u'));
    for c in "30d".chars() {
        quiet(&mut m, key(c));
    }
    quiet(&mut m, code(KeyCode::Enter));
    let cmds = update(&mut m, key('y'));
    assert!(
        matches!(&cmds[..], [Cmd::RunGenBaseline { .. }]),
        "{cmds:?}"
    );
    assert_eq!(m.overlay, Overlay::Generator);
}

#[test]
fn o_or_another_filter_ends_it_without_a_request() {
    let mut m = opened();
    let (seq, _) = run_baseline(&mut m);
    let seen = intra(&m);
    answer(&mut m, seq, seen, false);
    quiet(&mut m, key('o'));
    assert!(view(&m).since.is_none());
    assert_eq!(m.status.text, "o changed the sections: N compares again");
    // With outbound on, N queries both sides.
    let (seq, cmd) = run_baseline(&mut m);
    let Cmd::RunGenBaseline { focus, .. } = cmd else {
        unreachable!()
    };
    assert_eq!(focus.direction, FocusDirection::Both);
    assert_eq!(view(&m).requests, 3, "1 + 2 queries");
    answer(&mut m, seq, Vec::new(), false);
    // Another filter under the result: it ends at the next rebuild.
    m.traffic.data.as_mut().unwrap().filter.window = "1h".into();
    super::generator::rebuild_gen(&mut m);
    assert!(view(&m).since.is_none());
    assert_eq!(
        m.status.text,
        "the traffic filter changed: N compares again"
    );
}

#[test]
fn n_is_refused_on_incomplete_data() {
    let mut m = with_policy(model_with_window("24h"));
    m.traffic.data.as_mut().unwrap().truncated = true;
    quiet(
        &mut m,
        Msg::Command(crate::tui::commands::Command::Gen(Some("shop/prod".into()))),
    );
    quiet(&mut m, shift_n());
    assert_eq!(m.overlay, Overlay::Generator);
    assert!(
        m.status.text.ends_with(": N compares complete data only"),
        "{}",
        m.status.text
    );
}
