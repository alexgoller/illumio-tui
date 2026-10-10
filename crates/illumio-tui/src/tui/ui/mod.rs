//! `view(&Model, &mut Frame)`: pure rendering. Screens are added in later
//! tasks; this file owns the chrome (header, tabs, status, too-small notice).
pub mod layout;
pub mod theme;
pub mod widgets;

use crate::tui::model::{Connection, Model, Screen};
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
    let left = Line::from(vec![
        Span::styled(" illumio-tui ", t.title_style()),
        Span::styled(
            format!("{} · org {} ", model.profile.host, model.profile.org_id),
            Style::new().fg(t.fg),
        ),
        conn,
    ]);
    let left_w = left.width() as u16;
    let right_w = right.len() as u16;
    f.render_widget(Paragraph::new(left).style(t.base()), area);
    if area.width > left_w + right_w {
        let r = Rect::new(area.x + area.width - right_w, area.y, right_w, 1);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(right, t.dim_style()))).style(t.base()),
            r,
        );
    }
}

fn render_tabs(model: &Model, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let titles: Vec<Line> = Screen::ALL
        .iter()
        .enumerate()
        .map(|(i, s)| Line::from(format!(" {} {} ", i + 1, s.title())))
        .collect();
    let tabs = Tabs::new(titles)
        .select(model.screen.index())
        .style(Style::new().fg(t.dim).bg(t.bg))
        .highlight_style(
            Style::new()
                .fg(t.accent)
                .add_modifier(ratatui::style::Modifier::BOLD),
        )
        .divider(Span::raw("│"));
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
                "Use :profile <name> to switch profile, or q to quit.",
                t.dim_style(),
            )),
        ];
        f.render_widget(
            Paragraph::new(text).block(block).wrap(Wrap { trim: true }),
            area,
        );
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
