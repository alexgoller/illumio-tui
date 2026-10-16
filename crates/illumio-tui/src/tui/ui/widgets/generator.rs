//! The policy generator (`G` / `:gen`), btop style: the slider and the
//! target, the inbound and outbound rows with their checkboxes, the
//! preview; `p` swaps the rows for the exact rule list.
use crate::tui::gen_view::GenView;
use crate::tui::model::Model;
use crate::tui::ui::theme::Theme;
use crate::tui::ui::widgets::panel::panel;
use crate::tui::ui::widgets::table::{clip_width, fit as fit_cells};
use pce::generate::{GenRow, Side};
use pce::write::PortSpec;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

/// `s` padded to `w` cells, or cut with `…` when wider.
fn fit(s: &str, w: usize) -> String {
    if Span::raw(s).width() <= w || w == 0 {
        fit_cells(s, w, false)
    } else {
        fit_cells(&format!("{}…", clip_width(s, w - 1)), w, false)
    }
}

/// One row: checkbox, peer, stop (`*` when overridden), services,
/// decisions, the draft's mark, the flags; an outbound row its target.
fn row_line<'a>(v: &GenView, i: usize, scope_keys: &[String], t: &Theme) -> Line<'a> {
    let r: &GenRow = &v.rows[i];
    let stop = r.effective(v.global);
    let check = match (r.checkable, r.checked) {
        (false, _) => " - ",
        (true, true) => "[x]",
        (true, false) => "[ ]",
    };
    let services = if stop.is_all() {
        "all services".to_string()
    } else {
        r.services
            .iter()
            .map(PortSpec::display)
            .collect::<Vec<_>>()
            .join(" ")
    };
    let mut decisions = String::new();
    if r.decisions.allowed > 0 {
        decisions.push_str(&format!("✓ {} ", r.decisions.allowed));
    }
    if r.decisions.blocked > 0 {
        decisions.push_str(&format!("✗ {} ", r.decisions.blocked));
    }
    if let Some(n) = r.decisions.draft_allowed.filter(|n| *n > 0) {
        decisions.push_str(&format!("would allow {n} "));
    }
    let mut spans = vec![
        Span::styled(format!("{check} "), Style::new().fg(t.accent)),
        Span::styled(fit(&r.peer_text(), 18), t.base()),
        Span::raw(" "),
        Span::styled(
            fit(
                &format!("{}{}", stop.text(), if r.stop.is_some() { "*" } else { "" }),
                11,
            ),
            t.dim_style(),
        ),
        Span::raw(" "),
        Span::styled(fit(&services, 18), t.base()),
        Span::raw(" "),
        Span::styled(decisions, t.base()),
    ];
    if let Some(target) = v.row_target(r) {
        spans.push(Span::styled(format!("→ {target} "), t.dim_style()));
    }
    for f in r
        .flag_texts(stop, scope_keys)
        .into_iter()
        .chain(v.rule_flags(i))
    {
        spans.push(Span::styled(format!("⚠ {f} "), Style::new().fg(t.warn)));
    }
    Line::from(spans)
}

pub fn render_generator(model: &Model, f: &mut Frame, area: Rect) {
    let Some(v) = &model.gen_view else {
        return;
    };
    let t = &model.theme;
    let body = crate::tui::ui::layout::chrome(area).map_or(area, |c| c.body);
    f.render_widget(Clear, body);
    let (title, meta) = (v.header_title(), v.header_meta());
    let outer = panel(&title, &meta, t, true).style(t.base());
    let inner = outer.inner(body);
    f.render_widget(outer, body);
    let parts = Layout::vertical([
        Constraint::Length(2),
        Constraint::Fill(1),
        Constraint::Length(2),
    ])
    .split(inner);
    let empty = pce::NameIndex::default();
    let idx = model.index().map_or(&empty, |i| i.as_ref());
    let mut top = vec![
        Line::from(Span::styled(v.slider_line(), t.base())),
        Line::from(Span::styled(
            v.target_line(model.draft_rulesets(), idx),
            t.header_style(),
        )),
    ];
    if let Some(g) = &v.gap {
        top[0] = Line::from(Span::styled(g.clone(), Style::new().fg(t.warn)));
    }
    f.render_widget(Paragraph::new(top), parts[0]);
    let height = usize::from(parts[1].height);
    let (lines, cursor) = match v.list {
        Some(scroll) => {
            let all: Vec<Line> = v
                .rule_list(idx)
                .into_iter()
                .map(|l| Line::from(Span::styled(l, t.base())))
                .collect();
            let first = usize::from(scroll).min(all.len().saturating_sub(height));
            (all.into_iter().skip(first).collect::<Vec<_>>(), None)
        }
        None => {
            let mut all = Vec::new();
            let mut cursor = None;
            for side in [Side::Inbound, Side::Outbound] {
                let title = match side {
                    Side::Inbound => format!("Inbound → {}", v.target_name()),
                    Side::Outbound if v.outbound => "Outbound (o: on)".to_string(),
                    Side::Outbound => "Outbound (o: off)".to_string(),
                };
                all.push(Line::from(Span::styled(title, t.header_style())));
                let rows = v.section_rows(side);
                if side == Side::Outbound && !v.outbound {
                    all.push(Line::from(Span::styled(
                        format!("{} rows hidden: o shows them", rows.len()),
                        t.dim_style(),
                    )));
                    continue;
                }
                for i in rows {
                    if Some(i) == v.selected() && side == v.section {
                        cursor = Some(all.len());
                    }
                    all.push(row_line(v, i, &model.limits.scope_keys, t));
                }
            }
            (all, cursor)
        }
    };
    let first = cursor.map_or(0, |c| c.saturating_sub(height.saturating_sub(1)));
    let shown: Vec<Line> = lines
        .into_iter()
        .enumerate()
        .skip(first)
        .take(height)
        .map(|(i, l)| {
            if Some(i) == cursor {
                l.patch_style(Style::new().add_modifier(Modifier::REVERSED))
            } else {
                l
            }
        })
        .collect();
    f.render_widget(Paragraph::new(shown), parts[1]);
    let keys = if v.list.is_some() {
        "j/k scroll · Esc back to the rows"
    } else {
        "p lists the rules · Enter plans the writes · Space toggles · o outbound"
    };
    let footer = vec![
        Line::from(Span::styled(
            v.footer(),
            if v.error.is_some() {
                Style::new().fg(t.bad)
            } else {
                t.base()
            },
        )),
        Line::from(Span::styled(keys, t.dim_style())),
    ];
    f.render_widget(Paragraph::new(footer), parts[2]);
}
