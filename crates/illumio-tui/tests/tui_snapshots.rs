use illumio_tui::tui::model::{Connection, Screen};
use illumio_tui::tui::test_support::{model_fixture, render_styled, render_to_string};

#[test]
fn chrome_dark_80x24() {
    let m = model_fixture();
    insta::assert_snapshot!("chrome_dark_80x24", render_to_string(&m, 80, 24));
}

#[test]
fn chrome_dark_160x48() {
    let m = model_fixture();
    insta::assert_snapshot!("chrome_dark_160x48", render_to_string(&m, 160, 48));
}

#[test]
fn chrome_light_styled_80x24() {
    let mut m = model_fixture();
    m.set_theme("light");
    insta::assert_snapshot!("chrome_light_styled_80x24", render_styled(&m, 80, 24));
}

#[test]
fn too_small_terminal_shows_notice_and_does_not_panic() {
    let m = model_fixture();
    let out = render_to_string(&m, 40, 10);
    assert!(out.contains("too small"), "{out}");
    let _ = render_to_string(&m, 1, 1);
    let _ = render_to_string(&m, 0, 0);
}

#[test]
fn disconnected_state_shows_error_in_body() {
    let mut m = model_fixture();
    m.connection = Connection::Disconnected {
        error: "authentication failed (HTTP 401); check API key, secret and org id".into(),
    };
    m.screen = Screen::Traffic;
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains("HTTP 401"), "{out}");
    assert!(out.contains(":profile"), "hint to switch profile: {out}");
}

#[test]
fn table_widget_renders_header_sort_and_selection() {
    use illumio_tui::tui::table::{Column, SortKey, TableView};
    use illumio_tui::tui::ui::widgets::panel::panel;
    use illumio_tui::tui::ui::widgets::table::render;
    use ratatui::{Terminal, backend::TestBackend, style::Style};
    let m = model_fixture();
    let mut v: TableView<(String, u64)> = TableView::new(vec![
        Column {
            title: "name",
            width: 12,
            cell: |r: &(String, u64)| r.0.clone(),
            key: |r: &(String, u64)| SortKey::Text(r.0.clone()),
            right: false,
        },
        Column {
            title: "conns",
            width: 7,
            cell: |r: &(String, u64)| r.1.to_string(),
            key: |r: &(String, u64)| SortKey::Num(r.1),
            right: true,
        },
    ]);
    v.set_rows(
        (0..30)
            .map(|i| (format!("host-{i}"), (30 - i) as u64 * 7))
            .collect(),
    );
    v.sort_by(1, true);
    v.select_next();
    v.select_next();
    v.ensure_visible(6);
    let mut term = Terminal::new(TestBackend::new(40, 10)).unwrap();
    term.draw(|f| {
        render(
            f,
            f.area(),
            &v,
            &m.theme,
            panel("Hosts", "30 rows", &m.theme, true),
            &|_| Style::default(),
        )
    })
    .unwrap();
    let buf = term.backend().buffer();
    let text: String = (0..10)
        .map(|y| {
            (0..40)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
                .trim_end()
                .to_string()
                + "\n"
        })
        .collect();
    insta::assert_snapshot!("table_widget_40x10", text);
}

#[test]
fn help_overlay_80x24() {
    use illumio_tui::tui::model::Overlay;
    let mut m = model_fixture();
    m.overlay = Overlay::Help;
    insta::assert_snapshot!("help_overlay_80x24", render_to_string(&m, 80, 24));
}

#[test]
fn prompt_overlay_80x24() {
    use illumio_tui::tui::model::Overlay;
    let mut m = model_fixture();
    m.overlay = Overlay::Prompt {
        input: "filter app=web".into(),
    };
    let out = render_to_string(&m, 80, 24);
    assert!(out.contains(":filter app=web"), "{out}");
    insta::assert_snapshot!("prompt_overlay_80x24", out);
}
