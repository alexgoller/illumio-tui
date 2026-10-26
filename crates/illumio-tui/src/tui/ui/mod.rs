//! `view(&Model, &mut Frame)`: pure rendering. Screens are added in later
//! tasks; this file owns the chrome (header, tabs, status, too-small notice).
pub mod colors;
pub mod layout;
pub mod screens;
pub mod theme;
pub mod widgets;

use crate::tui::model::{Connection, Model, Overlay, Screen};
use layout::chrome;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Tabs, Wrap};

pub fn view(model: &Model, f: &mut Frame) {
    let area = f.area();
    let t = &model.theme;
    f.render_widget(Paragraph::new("").style(t.base()), area);
    let Some(c) = chrome(area) else {
        let msg = format!(
            "terminal too small ({}×{}); need at least {}×{}",
            area.width,
            area.height,
            layout::MIN_WIDTH,
            layout::MIN_HEIGHT
        );
        f.render_widget(
            Paragraph::new(msg)
                .style(t.base())
                .wrap(Wrap { trim: true }),
            area,
        );
        return;
    };
    render_header(model, f, c.header);
    render_tabs(model, f, c.tabs);
    render_body(model, f, c.body);
    widgets::status::render(model, f, c.status);
    match &model.overlay {
        Overlay::None => {}
        Overlay::Help => widgets::overlay::render_help(model, f, area),
        Overlay::Prompt { input } => widgets::overlay::render_prompt(model, input, f, area),
        Overlay::Baseline { input } => {
            widgets::overlay::render_baseline_prompt(model, input, f, area)
        }
        Overlay::GroupKeys(picker) => widgets::overlay::render_key_picker(model, picker, f, area),
        Overlay::AppPicker(p) => widgets::overlay::render_app_picker(model, p, f, area),
        Overlay::FilterPicker(p) => widgets::overlay::render_filter_picker(model, p, f, area),
        Overlay::App => widgets::app_overlay::render_app(model, f, area),
        Overlay::Generator => widgets::generator::render_generator(model, f, area),
        Overlay::LabelKeys(picker) => {
            widgets::overlay::render_label_key_picker(model, picker, f, area)
        }
        Overlay::FilterEditor(e) => widgets::overlay::render_filter_editor(model, e, f, area),
        Overlay::LabelFilter(e) => widgets::overlay::render_label_filter(model, e, f, area),
        Overlay::RuleFilter(e) => screens::policy::render_rule_filter(model, e, f, area),
        Overlay::Confirm { reason, action } => {
            // Deleting a mark: the `:marks` list stays behind the question.
            if matches!(action, crate::tui::model::PendingAction::DeleteMark(_)) {
                widgets::marks::render_marks(model, f, area);
            }
            // Deleting a saved filter: the picker stays behind the question.
            if let crate::tui::model::PendingAction::DeleteFilter(p, _) = action {
                widgets::overlay::render_filter_picker(model, p, f, area);
            }
            widgets::overlay::render_confirm(model, reason, f, area)
        }
        Overlay::RuleDetail(rule) => widgets::overlay::render_rule_detail(model, rule, f, area),
        Overlay::Matched => screens::policy::render_matched(model, f, area),
        Overlay::WhereUsed => screens::objects::render_where_used(model, f, area),
        Overlay::Pending => widgets::overlay::render_pending(model, f, area),
        Overlay::Error(text) => widgets::overlay::render_error(model, text, f, area),
        Overlay::EventsEditor(e) => widgets::overlay::render_events_editor(model, e, f, area),
        Overlay::EventTypes { selected, actors } => {
            widgets::overlay::render_event_types(model, *selected, *actors, f, area)
        }
        Overlay::Ven { selected } => widgets::overlay::render_ven(model, *selected, f, area),
        Overlay::EventDetail { event, scroll, raw } => {
            widgets::overlay::render_event_detail(model, event, *scroll, *raw, f, area)
        }
        Overlay::Form(form) => widgets::overlay::render_form(model, form, f, area),
        Overlay::MarkForm(mf) => {
            // The overlay the form came from stays behind it.
            match mf.ctx.origin {
                crate::tui::mark_form::MarkOrigin::Marks => {
                    widgets::marks::render_marks(model, f, area)
                }
                crate::tui::mark_form::MarkOrigin::App if model.app_view.is_some() => {
                    widgets::app_overlay::render_app(model, f, area)
                }
                _ => {}
            }
            widgets::marks::render_mark_form(model, mf, f, area)
        }
        Overlay::Marks => widgets::marks::render_marks(model, f, area),
        Overlay::ConfirmWrite { plan, kind, scroll } => {
            widgets::overlay::render_confirm_write(model, plan, *kind, *scroll, f, area)
        }
        Overlay::ConfirmBatch { batch, scroll } => {
            widgets::overlay::render_confirm_batch(model, batch, *scroll, f, area)
        }
        Overlay::BatchReport { scroll } => {
            widgets::overlay::render_batch_report(model, *scroll, f, area)
        }
        Overlay::EnforcePicker(p) => widgets::bulk::render_picker(model, p, f, area),
        Overlay::ConfirmBulk(c) => widgets::bulk::render_confirm(model, c, f, area),
        Overlay::Diff { diff, scroll } => {
            widgets::overlay::render_diff(model, diff, *scroll, f, area)
        }
        Overlay::Readiness => widgets::enforce::render_readiness(model, f, area),
        Overlay::Enforce => widgets::enforce::render_enforce(model, f, area),
        Overlay::Explain(v) => {
            // Opened from the app overlay: it stays behind.
            if v.from_app && model.app_view.is_some() {
                widgets::app_overlay::render_app(model, f, area);
            }
            widgets::explain::render(model, v, f, area)
        }
    }
}

fn render_header(model: &Model, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let conn = match &model.connection {
        Connection::Connecting => Span::styled("connecting", t.dim_style()),
        Connection::Connected { index: Some(_), .. } => Span::styled("●", Style::new().fg(t.ok)),
        Connection::Connected { index: None, .. } => Span::styled("◐", Style::new().fg(t.warn)),
        Connection::Disconnected { .. } => Span::styled("○ disconnected", Style::new().fg(t.bad)),
    };
    let kb = model.stats.bytes as f64 / 1024.0;
    let right = format!("api {} calls · {kb:.1} KB ", model.stats.calls);
    let mut spans = vec![Span::styled(" illumio-tui ", t.title_style())];
    // Before the host: a long hostname must never push the safety marker off.
    if model.writes.enabled {
        spans.push(Span::styled("✎ writes", Style::new().fg(t.warn)));
        spans.push(Span::styled(" · ", Style::new().fg(t.fg)));
    }
    spans.push(Span::styled(
        format!("{} · org {} ", model.profile.host, model.profile.org_id),
        Style::new().fg(t.fg),
    ));
    spans.push(conn);
    let left = Line::from(spans);
    let left_w = left.width() as u16;
    let right_w = Line::from(right.as_str()).width() as u16;
    f.render_widget(Paragraph::new(left).style(t.base()), area);
    if area.width > left_w + right_w {
        let r = Rect::new(area.x + area.width - right_w, area.y, right_w, 1);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(right, t.dim_style()))).style(t.base()),
            r,
        );
    }
}

/// Tab labels for a row `width` cells wide: the full titles when the whole
/// row fits, else the short ones. `Tabs` draws each between `TAB_PAD` cells.
fn tab_labels(width: u16) -> Vec<String> {
    let labels = |title: fn(Screen) -> &'static str| -> Vec<String> {
        Screen::ALL
            .iter()
            .enumerate()
            .map(|(i, s)| format!(" {} {} ", i + 1, title(*s)))
            .collect()
    };
    let full = labels(Screen::title);
    if row_width(&full) <= width {
        full
    } else {
        labels(Screen::short_title)
    }
}

/// Cells the whole tab row takes: every tab and the dividers between them.
fn row_width(labels: &[String]) -> u16 {
    let divider = TAB_DIVIDER.chars().count() as u16;
    let tabs: u16 = labels.iter().map(|l| tab_width(l)).sum();
    tabs + divider * (labels.len() as u16).saturating_sub(1)
}

/// Padding cells either side of a label, and the divider between tabs; shared
/// by `render_tabs` and `tab_at` so hit-testing cannot drift from the drawing.
const TAB_PAD: &str = " ";
const TAB_DIVIDER: &str = "│";

/// Cells one tab occupies, padding included.
fn tab_width(label: &str) -> u16 {
    (label.chars().count() + 2 * TAB_PAD.chars().count()) as u16
}

/// Which tab a click at column `x` on the tabs row hits, for a row `width` cells wide.
/// Dividers, the space past the last tab and clipped cells hit nothing.
pub fn tab_at(x: u16, width: u16) -> Option<Screen> {
    if x >= width {
        return None;
    }
    let divider = TAB_DIVIDER.chars().count() as u16;
    let mut start = 0u16;
    for (label, screen) in tab_labels(width).iter().zip(Screen::ALL) {
        let w = tab_width(label);
        if x >= start && x < start + w {
            return Some(screen);
        }
        start += w + divider;
    }
    None
}

/// A column inside the given tab's label on a row `width` cells wide (test helper for `tab_at`).
pub fn tab_x(screen: Screen, width: u16) -> u16 {
    let divider = TAB_DIVIDER.chars().count() as u16;
    let mut start = 0u16;
    for (label, s) in tab_labels(width).iter().zip(Screen::ALL) {
        if s == screen {
            return start + TAB_PAD.chars().count() as u16;
        }
        start += tab_width(label) + divider;
    }
    0
}

fn render_tabs(model: &Model, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let titles: Vec<Line> = tab_labels(area.width).into_iter().map(Line::from).collect();
    let tabs = Tabs::new(titles)
        .select(model.screen.index())
        .style(Style::new().fg(t.dim).bg(t.bg))
        .highlight_style(
            Style::new()
                .fg(t.accent)
                .add_modifier(ratatui::style::Modifier::BOLD),
        )
        .padding(TAB_PAD, TAB_PAD)
        .divider(Span::raw(TAB_DIVIDER));
    f.render_widget(tabs, area);
}

fn render_body(model: &Model, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    if let Connection::Disconnected { error } = &model.connection {
        let block = widgets::panel::panel("Not connected", "", t, true);
        let text = vec![
            Line::from(Span::styled(error.clone(), Style::new().fg(t.bad))),
            Line::from(""),
            Line::from(Span::styled(
                "r reconnects · :profile <name> switches profile · q quits.",
                t.dim_style(),
            )),
        ];
        f.render_widget(
            Paragraph::new(text).block(block).wrap(Wrap { trim: true }),
            area,
        );
        return;
    }
    if model.screen == Screen::Traffic && model.is_connected() {
        screens::traffic::render(model, f, area);
        return;
    }
    if model.screen == Screen::Policy && model.is_connected() {
        screens::policy::render_screen(model, f, area);
        return;
    }
    if model.screen == Screen::Objects && model.is_connected() {
        screens::objects::render_screen(model, f, area);
        return;
    }
    if model.screen == Screen::Workloads && model.is_connected() {
        screens::workloads::render_screen(model, f, area);
        return;
    }
    if model.screen == Screen::Events && model.is_connected() {
        screens::events::render_screen(model, f, area);
        return;
    }
    let block = widgets::panel::panel(model.screen.title(), "", t, true);
    let text = match &model.connection {
        Connection::Connecting => "connecting…",
        Connection::Connected { index: None, .. } => "loading objects…",
        _ => "",
    };
    f.render_widget(Paragraph::new(text).block(block).style(t.base()), area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_at_matches_the_rendered_row() {
        for s in Screen::ALL {
            assert_eq!(tab_at(tab_x(s, 80), 80), Some(s));
        }
        let first_len = tab_width(&tab_labels(80)[0]);
        assert_eq!(tab_at(first_len, 80), None, "divider");
        assert_eq!(tab_at(79, 80), None, "past the last tab");
        let x = tab_x(Screen::Workloads, 80);
        assert_eq!(tab_at(x, x), None, "clipped by a narrow terminal");
        // The rendered row puts each label exactly where tab_at expects it,
        // with the short titles at 64 columns and the full ones from 80.
        let m = crate::tui::test_support::model_fixture();
        for (w, h) in [(64u16, 24u16), (80, 24), (160, 48)] {
            let out = crate::tui::test_support::render_to_string(&m, w, h);
            let row: Vec<char> = out.lines().nth(1).unwrap().chars().collect();
            for s in Screen::ALL {
                let x = tab_x(s, w) as usize;
                assert!(row[x + 1].is_ascii_digit(), "the label's number follows x");
                assert_eq!(row[x - 1], ' ', "padding cell before the label");
                assert_eq!(tab_at(x as u16 + 1, w), Some(s));
            }
        }
    }

    #[test]
    fn the_tab_row_shortens_only_when_the_full_row_does_not_fit() {
        let full = tab_labels(160);
        assert_eq!(full.len(), 5);
        assert_eq!(row_width(&full), 69, "five full titles and four dividers");
        assert_eq!(full[2], " 3 Objects ");
        assert_eq!(full[4], " 5 Events ");
        assert_eq!(tab_labels(80), full, "the full row fits at 80 columns");
        assert_eq!(tab_labels(69)[4], " 5 Events ");
        assert_eq!(tab_labels(68)[4], " 5 Ev ");
        assert_eq!(tab_labels(68)[0], " 1 Traf ");
        assert_eq!(row_width(&tab_labels(60)), 48);
    }
}
