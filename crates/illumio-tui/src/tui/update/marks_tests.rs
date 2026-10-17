//! The marks' keys on the Traffic screen: `M` toggles squelched rows with
//! a rebuild and never a request; a search being typed takes the key.
use crate::tui::mark_form::{COMMENT, MarkForm, SRC};
use crate::tui::model::{Model, Overlay, TrafficView};
use crate::tui::msg::{Cmd, Msg};
use crate::tui::traffic_marks::tests::{RICH_MARKS, marked_model};
use crate::tui::update;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn key(c: char) -> Msg {
    Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

fn ctrl(c: char) -> Msg {
    Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
}

fn kc(code: KeyCode) -> Msg {
    Msg::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

/// Sends `msg` and asserts it asked for nothing: marks never reach the PCE.
#[track_caller]
fn quiet(m: &mut Model, msg: Msg) {
    assert_eq!(update(m, msg), Vec::<Cmd>::new());
}

#[track_caller]
fn form(m: &Model) -> &MarkForm {
    match &m.overlay {
        Overlay::MarkForm(f) => f,
        o => panic!("no mark form: {o:?}"),
    }
}

/// The form's fields as `(label, value)`, the comment left out.
fn values(m: &Model) -> Vec<(String, String)> {
    form(m)
        .form
        .fields
        .iter()
        .filter(|f| f.label != "Comment")
        .map(|f| (f.label.to_string(), f.value.clone()))
        .collect()
}

fn expect(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect()
}

fn typed(m: &mut Model, text: &str) {
    for c in text.chars() {
        quiet(m, key(c));
    }
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
fn m_opens_the_form_prefilled_from_the_row_in_every_view() {
    use crate::tui::model::By;
    let (mut m, _dir) = marked_model(RICH_MARKS);
    let fields = |action, src, dst, svc, decision, expires| {
        expect(&[
            ("Action", action),
            ("Source", src),
            ("Destination", dst),
            ("Service", svc),
            ("Decision", decision),
            ("Draft", ""),
            ("Expires", expires),
            ("Scope", "this PCE"),
        ])
    };
    // Flows: the first shown flow (shop's web to crm's web on 443).
    quiet(&mut m, key('m'));
    assert_eq!(
        values(&m),
        fields(
            "reviewed",
            "app=shop env=prod",
            "app=crm env=prod",
            "443/tcp",
            "",
            "30d"
        )
    );
    assert!(
        form(&m)
            .form
            .title
            .starts_with("New mark · Flows · shop-web-p.example → crm-web-p.example 443/tcp"),
        "{}",
        form(&m).form.title
    );
    assert_eq!(form(&m).form.active, COMMENT);
    quiet(&mut m, kc(KeyCode::Esc));
    assert_eq!(m.overlay, Overlay::None);
    // App-to-app pairs, then by service.
    m.traffic.view = TrafficView::Aggregate;
    quiet(&mut m, key('m'));
    assert_eq!(
        values(&m),
        fields(
            "reviewed",
            "app=crm env=prod",
            "app=crm env=prod",
            "",
            "",
            "30d"
        )
    );
    quiet(&mut m, kc(KeyCode::Esc));
    quiet(&mut m, key('o'));
    quiet(&mut m, key('m'));
    assert_eq!(form(&m).form.value("Service"), "3306/tcp");
    quiet(&mut m, kc(KeyCode::Esc));
    // Sections `in`: the header fills the app side only; a peer row both.
    m.traffic.apps_state.by = By::In;
    m.traffic.rebuild_sections();
    quiet(&mut m, key('m'));
    assert_eq!(
        values(&m),
        fields("reviewed", "any", "app=crm env=prod", "", "", "30d")
    );
    quiet(&mut m, kc(KeyCode::Esc));
    quiet(&mut m, key('j'));
    quiet(&mut m, key('m'));
    assert_eq!(
        values(&m),
        fields(
            "reviewed",
            "app=crm env=prod",
            "app=crm env=prod",
            "3306/tcp",
            "",
            "30d"
        )
    );
    quiet(&mut m, kc(KeyCode::Esc));
    // Mesh.
    m.traffic.view = TrafficView::Mesh;
    quiet(&mut m, key('m'));
    assert_eq!(form(&m).form.value("Service"), "3306/tcp");
    quiet(&mut m, kc(KeyCode::Esc));
    // Blocked: accept-blocked, the filter's decisions, 90 days.
    m.traffic.view = TrafficView::Blocked;
    quiet(&mut m, key('j'));
    quiet(&mut m, key('m'));
    assert_eq!(
        values(&m),
        fields(
            "accept-blocked",
            "app=erp env=dev",
            "app=erp env=prod",
            "8443/tcp",
            "blocked, potentially_blocked",
            "90d"
        )
    );
    quiet(&mut m, kc(KeyCode::Esc));
    // The treemap drills first.
    m.traffic.view = TrafficView::Treemap;
    quiet(&mut m, key('m'));
    assert_eq!(m.status.text, "m: Enter drills to the flows, mark there");
    assert_eq!(m.overlay, Overlay::None);
}

#[test]
fn m_in_the_app_overlay_marks_peers_and_returns_there() {
    use crate::tui::commands::Command;
    let (mut m, _dir) = marked_model(RICH_MARKS);
    update(&mut m, Msg::Command(Command::App(Some("shop/prod".into()))));
    quiet(&mut m, key('m'));
    assert_eq!(
        values(&m),
        expect(&[
            ("Action", "reviewed"),
            ("Source", "app=hr env=prod"),
            ("Destination", "app=shop env=prod"),
            ("Service", "5432/tcp"),
            ("Decision", "potentially_blocked"),
            ("Draft", ""),
            ("Expires", "30d"),
            ("Scope", "this PCE"),
        ])
    );
    quiet(&mut m, kc(KeyCode::Esc));
    assert_eq!(m.overlay, Overlay::App, "back where it came from");
    quiet(&mut m, kc(KeyCode::Tab));
    quiet(&mut m, kc(KeyCode::Tab));
    quiet(&mut m, key('m'));
    assert_eq!(m.status.text, "m marks traffic rows (Inbound, Outbound)");
    assert_eq!(m.overlay, Overlay::App);
}

#[test]
fn widening_a_side_changes_the_preview() {
    let (mut m, _dir) = marked_model(RICH_MARKS);
    quiet(&mut m, key('m'));
    assert_eq!(
        form(&m).preview,
        vec!["matches 1 row · 1 flow in this result"]
    );
    // To the Source field, then its kinds until `any`.
    for _ in 0..(COMMENT - SRC) {
        quiet(&mut m, kc(KeyCode::BackTab));
    }
    assert_eq!(form(&m).form.active, SRC);
    while form(&m).form.value("Source") != "any" {
        quiet(&mut m, ctrl('t'));
    }
    assert_eq!(
        form(&m).preview,
        vec![
            "matches 3 rows · 3 flows in this result",
            "1 of them go to a stronger mark: revisit m-hr wins",
        ]
    );
    // A squelch: hides them; hr's flow stays shown under its revisit mark.
    quiet(&mut m, kc(KeyCode::BackTab));
    quiet(&mut m, ctrl('u'));
    typed(&mut m, "squelch");
    assert_eq!(
        form(&m).form.value("Expires"),
        "90d",
        "the action's default"
    );
    assert_eq!(
        form(&m).preview,
        vec![
            "matches 3 rows · 3 flows in this result · hides them (M shows)",
            "1 of them stay shown: revisit m-hr wins",
        ]
    );
    // Ctrl-A: every label of the destination's endpoint.
    quiet(&mut m, kc(KeyCode::Tab));
    quiet(&mut m, kc(KeyCode::Tab));
    quiet(&mut m, ctrl('a'));
    assert_eq!(
        form(&m).form.value("Destination"),
        "app=crm datacenter=fra1 env=prod loc=us role=web"
    );
    // Ctrl-U on the service: any service, the same three flows here.
    quiet(&mut m, kc(KeyCode::Tab));
    quiet(&mut m, ctrl('u'));
    assert_eq!(form(&m).form.value("Service"), "");
    assert!(
        form(&m).preview[0].starts_with("matches 3 rows"),
        "{:?}",
        form(&m).preview
    );
    typed(&mut m, "x");
    assert!(
        form(&m)
            .form
            .error
            .as_deref()
            .unwrap()
            .starts_with("Service: "),
        "{:?}",
        form(&m).form.error
    );
    assert!(form(&m).preview.is_empty());
}

#[test]
fn enter_requires_a_comment_then_writes_the_file_and_rebuilds() {
    let (mut m, dir) = marked_model(RICH_MARKS);
    m.traffic.view = TrafficView::Aggregate;
    quiet(&mut m, key('m'));
    quiet(&mut m, kc(KeyCode::Enter));
    assert_eq!(
        form(&m).form.error.as_deref(),
        Some("comment required: why are these flows marked?")
    );
    assert_eq!(form(&m).form.active, COMMENT);
    typed(&mut m, "db traffic is known");
    quiet(&mut m, kc(KeyCode::Tab));
    quiet(&mut m, ctrl('u'));
    typed(&mut m, "squelch");
    quiet(&mut m, kc(KeyCode::Enter));
    assert_eq!(m.overlay, Overlay::None);
    assert!(
        m.status.text.starts_with("mark m-")
            && m.status
                .text
                .ends_with(" saved: squelch, 2 flows · :marks lists marks"),
        "{}",
        m.status.text
    );
    let text = std::fs::read_to_string(dir.path().join("marks.toml")).unwrap();
    assert!(text.contains("comment = \"db traffic is known\""), "{text}");
    assert!(text.contains("pce = \"pce.example\""), "this PCE: {text}");
    assert!(
        m.traffic
            .groups
            .rows()
            .iter()
            .all(|g| !(g.src.join("/") == "crm/prod" && g.dst.join("/") == "crm/prod")),
        "the views rebuilt: the pair is squelched"
    );
}

#[test]
fn a_failed_write_keeps_the_form_open_with_the_error() {
    let (mut m, dir) = marked_model(RICH_MARKS);
    quiet(&mut m, key('m'));
    typed(&mut m, "why");
    std::fs::write(dir.path().join("marks.toml.lock"), "1").unwrap();
    quiet(&mut m, kc(KeyCode::Enter));
    assert_eq!(
        form(&m).form.error.as_deref(),
        Some("marks.toml is locked by another illumio-tui (marks.toml.lock): try again")
    );
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

#[test]
fn the_cursor_stays_on_screen_with_marks_and_the_detail_open() {
    use crate::tui::test_support::render_to_string;
    use crossterm::event::KeyEvent;
    let (mut m, _dir) = marked_model(RICH_MARKS);
    m.size = (80, 24);
    let enter = || Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    // Walk down with the detail closed for a while, then open it: the
    // row it takes from the table must not hide the cursor.
    let n = m.traffic.flows.rows().len();
    for i in 0..n {
        if i == 4 {
            update(&mut m, enter());
            assert!(m.traffic.show_detail);
        }
        if i > 0 {
            update(&mut m, key('j'));
        }
        let out = render_to_string(&m, 80, 24);
        let sel = m.traffic.flows.selected_item().unwrap();
        let col = m
            .traffic
            .flows
            .columns
            .iter()
            .position(|c| c.title == "src");
        let src: String = m
            .traffic
            .flows
            .cell(sel, col.unwrap())
            .chars()
            .take(10)
            .collect();
        let line = out
            .lines()
            .find(|l| l.contains('▸'))
            .unwrap_or_else(|| panic!("the cursor row is off screen:\n{out}"));
        assert!(line.contains(&src), "{src:?} not on {line:?}\n{out}");
    }
}

#[test]
fn a_marks_notice_waits_behind_a_warning_and_joins_an_info() {
    use crate::tui::model::StatusLine;
    let (mut m, _dir) = marked_model("[[mark]]\naction = \"squelch\"\n");
    m.status = StatusLine::warn("query failed");
    update(&mut m, Msg::Tick(std::time::Instant::now()));
    assert_eq!(m.status.text, "query failed", "the warning stays");
    m.status = StatusLine::info("30 flows");
    update(&mut m, Msg::Tick(std::time::Instant::now()));
    assert_eq!(
        m.status,
        StatusLine::warn(
            "30 flows · marks.toml invalid (2 problems): marks off · :marks shows them"
        )
    );
    m.status = StatusLine::info("ready");
    update(&mut m, Msg::Tick(std::time::Instant::now()));
    assert_eq!(m.status.text, "ready", "shown once");
}

#[test]
fn the_app_overlay_judges_only_the_apps_flows() {
    use crate::tui::app_view::AppPanel;
    use crate::tui::commands::Command;
    use crate::tui::test_support::with_policy;
    // A squelch far from shop/prod: the Traffic screen counts it, the
    // overlay of shop/prod has nothing squelched.
    let (m, _dir) = marked_model(
        "[[mark]]\naction = \"squelch\"\nsrc = { labels = { app = \"erp\" } }\ndst = { labels = { app = \"erp\" } }\ncomment = \"x\"\n",
    );
    let mut m = with_policy(m);
    assert!(m.traffic.marked.as_ref().unwrap().flows.flows > 0);
    update(&mut m, Msg::Command(Command::App(Some("shop/prod".into()))));
    let v = m.app_view.as_ref().unwrap();
    assert!(v.marks_on);
    assert_eq!(v.squelched, [Default::default(); 2]);
    assert_eq!(v.panel_title(AppPanel::Inbound), "Inbound");
}
