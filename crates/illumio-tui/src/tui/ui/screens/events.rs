//! Events screen: one table of PCE audit events, coloured by severity. The
//! detail (pretty JSON) is an overlay, so the table keeps the whole body.
use crate::tui::model::Model;
use crate::tui::ui::widgets::bars::age_text;
use crate::tui::ui::widgets::panel::panel;
use crate::tui::ui::widgets::table::{header_meta, render};
use pce::models::Event;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use std::time::{Duration, Instant};

/// Rows of data visible in the table (minus border and header). Shared by `view` and `update`.
pub fn table_rows(body: Rect) -> usize {
    body.height.saturating_sub(3) as usize
}

/// `1,240`: digits in groups of three.
fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// `cached 5s · 4 rows · window 24h`, with the sort, search and
/// auto-refresh parts. Rows hidden locally are never silent: `hiding 3
/// types · warning+ · 812 of 1,240`; a full page from the PCE says
/// `capped`.
fn meta(model: &Model) -> String {
    let e = &model.events;
    let mut parts = Vec::new();
    let sort = header_meta(&e.table);
    if !sort.is_empty() {
        parts.push(sort);
    }
    if e.loading {
        parts.push("loading…".to_string());
    } else if let Some(l) = e.loaded_at {
        parts.push(format!(
            "cached {}",
            age_text(Duration::from_secs(e.age_secs) + l.elapsed())
        ));
    }
    if e.hidden_types > 0 {
        let s = if e.hidden_types == 1 { "" } else { "s" };
        parts.push(format!("hiding {} type{s}", e.hidden_types));
    }
    if let Ok(sev) = pce::models::SeverityFilter::parse(&e.filter.severity)
        && sev.is_local()
    {
        parts.push(e.filter.severity.trim().to_string());
    }
    let shown = e.table.len();
    parts.push(if shown == e.raw.len() {
        format!("{} rows", thousands(shown))
    } else {
        format!("{} of {}", thousands(shown), thousands(e.raw.len()))
    });
    if e.truncated {
        parts.push(format!("capped at {}", thousands(e.raw.len())));
    }
    parts.push(format!("window {}", e.filter.window));
    let mut meta = parts.join(" · ");
    meta.push_str(&e.auto.meta(Instant::now()));
    meta
}

pub fn render_screen(model: &Model, f: &mut Frame, body: Rect) {
    let t = &model.theme;
    let e = &model.events;
    let meta = meta(model);
    let note = if e.loaded_at.is_none() {
        Some(if e.loading {
            "loading events…"
        } else {
            "no events loaded · r loads them"
        })
    } else if e.raw.is_empty() {
        Some("no events in this window · f edits the filter")
    } else if e.table.rows().is_empty() {
        Some("every loaded event is hidden by the filter · f edits it, t lists the types")
    } else {
        None
    };
    if let Some(note) = note {
        f.render_widget(
            Paragraph::new(Span::styled(note, t.dim_style()))
                .block(panel("Events", &meta, t, true)),
            body,
        );
        return;
    }
    let theme = t.clone();
    render(
        f,
        body,
        &e.table,
        t,
        panel("Events", &meta, t, true),
        &move |ev: &Event| match ev.severity_rank() {
            2 | 3 => Style::new().fg(theme.bad),
            1 => Style::new().fg(theme.warn),
            _ => Style::default(),
        },
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn thousands_groups_digits() {
        assert_eq!(super::thousands(0), "0");
        assert_eq!(super::thousands(812), "812");
        assert_eq!(super::thousands(1240), "1,240");
        assert_eq!(super::thousands(200_000), "200,000");
        assert_eq!(super::thousands(1_234_567), "1,234,567");
    }
}
