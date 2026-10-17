//! The policy generator (`G` / `:gen`), btop style: the slider and the
//! target, the inbound and outbound rows with their checkboxes, the
//! preview; `p` swaps the rows for the exact rule list.
use crate::tui::gen_view::GenView;
use crate::tui::model::Model;
use crate::tui::ui::theme::Theme;
use crate::tui::ui::widgets::panel::panel;
use crate::tui::ui::widgets::table::{clip_width, fit as fit_cells};
use pce::generate::{GenRow, PeerKind, Side};
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
/// One row, its columns narrower below 100 columns (`narrow`) so the
/// flags fit at 80. `longest`: the widest peer text of all rows.
fn row_line<'a>(
    v: &GenView,
    i: usize,
    scope_keys: &[String],
    t: &Theme,
    width: usize,
    longest: usize,
) -> Line<'a> {
    let narrow = width < 100;
    let (peer_w, services_w) = if narrow { (14, 16) } else { (18, 18) };
    // Wider for a longer peer (`IP list rfc1918 · 7 IPs`), up to 8 more.
    let peer_w = longest.clamp(peer_w, peer_w + 8);
    let r: &GenRow = &v.rows[i];
    let stop = r.effective(v.global);
    let consolidated = v.consolidated_flag(i);
    let check = match (r.checkable, r.checked) {
        // Not checked, but a consolidated rule allows (some of) it.
        _ if consolidated.is_some() && !r.proposed() => "[~]",
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
        Span::styled(fit(&r.peer_text(), peer_w), t.base()),
        Span::raw(" "),
        Span::styled(
            fit(
                &format!("{}{}", stop.text(), if r.stop.is_some() { "*" } else { "" }),
                11,
            ),
            t.dim_style(),
        ),
        Span::raw(" "),
        Span::styled(fit(&services, services_w), t.base()),
        Span::raw(" "),
        Span::styled(decisions, t.base()),
    ];
    // An app peer's own ruleset; IP peers go into the app's (the title says so).
    if let Some(target) = v
        .row_target(r)
        .filter(|_| matches!(r.peer, PeerKind::App(_)))
    {
        spans.push(Span::styled(format!("→ {target} "), t.dim_style()));
    }
    // `consolidated` first: an unchecked row it covers never looks excluded.
    let flags: Vec<String> = consolidated
        .into_iter()
        .chain(r.flag_texts(stop, scope_keys))
        .chain(v.rule_flags(i))
        .collect();
    let used: usize = spans.iter().map(Span::width).sum();
    let all: usize = flags
        .iter()
        .map(|f| Span::raw(format!("⚠ {f} ")).width())
        .sum();
    if flags.is_empty() {
        // Nothing to add: a row that is already wider than the panel is cut
        // by the panel itself.
    } else if used + all <= width {
        for f in flags {
            spans.push(Span::styled(format!("⚠ {f} "), Style::new().fg(t.warn)));
        }
    } else if flags.len() == 1 {
        // One flag past the edge: cut with `…` (`p` shows it whole).
        let room = width.saturating_sub(used).max(4);
        let flag = fit(&format!("⚠ {}", flags[0]), room).trim_end().to_string();
        spans.push(Span::styled(flag, Style::new().fg(t.warn)));
    } else {
        // Not all fit: the first one and how many more (`p`, or a wider
        // terminal, shows them).
        let more = format!(" +{}", flags.len() - 1);
        let room = width.saturating_sub(used + more.len()).max(4);
        let first = format!("⚠ {}", flags[0]);
        let first = if Span::raw(first.as_str()).width() > room {
            fit(&first, room).trim_end().to_string()
        } else {
            first
        };
        spans.push(Span::styled(
            format!("{first}{more}"),
            Style::new().fg(t.warn),
        ));
    }
    Line::from(spans)
}

/// `s` wrapped at `w` cells, word by word; its own indent kept, and
/// continuation lines indented three cells more; a word wider than a line
/// is cut into pieces.
fn wrap_text(s: &str, w: usize) -> Vec<String> {
    let width = |x: &str| Span::raw(x).width();
    if w < 8 || width(s) <= w {
        return vec![s.to_string()];
    }
    let indent: String = s.chars().take_while(|c| *c == ' ').collect();
    let more = format!("{indent}   ");
    let mut out = Vec::new();
    let mut line = indent.clone();
    for word in s.split(' ').filter(|w| !w.is_empty()) {
        let mut word = word.to_string();
        loop {
            let sep = usize::from(!line.is_empty() && !line.ends_with(' '));
            if width(&line) + sep + width(&word) <= w {
                if sep == 1 {
                    line.push(' ');
                }
                line.push_str(&word);
                break;
            }
            if line.trim().is_empty() {
                // The word alone is wider than the line.
                let room = w - width(&line);
                let head = clip_width(&word, room).to_string();
                line.push_str(&head);
                word = word[head.len()..].to_string();
            }
            out.push(std::mem::replace(&mut line, more.clone()));
            if word.is_empty() {
                break;
            }
        }
    }
    if !line.trim().is_empty() {
        out.push(line);
    }
    out
}

/// The panel's inner area and its three parts (slider and target, the
/// rows or the rule list, the footer).
fn layout(v: &GenView, area: Rect) -> (Rect, std::rc::Rc<[Rect]>) {
    let body = crate::tui::ui::layout::chrome(area).map_or(area, |c| c.body);
    let inner = Rect {
        x: body.x + 1,
        y: body.y + 1,
        width: body.width.saturating_sub(2),
        height: body.height.saturating_sub(2),
    };
    let parts = Layout::vertical([
        Constraint::Length(2 + u16::from(v.gap.is_some())),
        Constraint::Fill(1),
        Constraint::Length(2),
    ])
    .split(inner);
    (inner, parts)
}

/// The rule list (`p`), wrapped at the width it is drawn at.
fn list_lines(model: &Model, v: &GenView, width: u16) -> Vec<String> {
    let empty = pce::NameIndex::default();
    let idx = model.index().map_or(&empty, |i| i.as_ref());
    v.rule_list(idx)
        .iter()
        .flat_map(|l| wrap_text(l, usize::from(width)))
        .collect()
}

/// The last scroll position of the rule list: its end on screen.
pub fn rule_list_last_scroll(model: &Model, area: Rect) -> u16 {
    let Some(v) = &model.gen_view else {
        return 0;
    };
    let (_, parts) = layout(v, area);
    let n = list_lines(model, v, parts[1].width).len();
    u16::try_from(n.saturating_sub(usize::from(parts[1].height))).unwrap_or(u16::MAX)
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
    f.render_widget(outer, body);
    let (inner, parts) = layout(v, area);
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
        top.push(Line::from(Span::styled(
            format!("⚠ {g}"),
            Style::new().fg(t.warn),
        )));
    }
    f.render_widget(Paragraph::new(top), parts[0]);
    let height = usize::from(parts[1].height);
    let (lines, first) = match v.list {
        Some(scroll) => {
            let all: Vec<Line> = list_lines(model, v, parts[1].width)
                .into_iter()
                .map(|l| Line::from(Span::styled(l, t.base())))
                .collect();
            let first = usize::from(scroll).min(all.len().saturating_sub(height));
            (all, first)
        }
        None => {
            let mut all = Vec::new();
            let mut cursor = None;
            let mut outbound_title = 0;
            let longest = v
                .rows
                .iter()
                .map(|r| Span::raw(r.peer_text()).width())
                .max()
                .unwrap_or(0);
            for side in [Side::Inbound, Side::Outbound] {
                let title = match side {
                    Side::Inbound => format!("Inbound → {}", v.target_name()),
                    Side::Outbound if v.outbound => {
                        format!("Outbound (o: on) · IP peers → {}", v.target_name())
                    }
                    Side::Outbound => "Outbound (o: off)".to_string(),
                };
                if side == Side::Outbound {
                    outbound_title = all.len();
                }
                all.push(Line::from(Span::styled(title, t.header_style())));
                // The fan-in lines (inbound): the cursor's first stops.
                for c in 0..v.lead(side) {
                    if side == v.section && v.selected_candidate() == Some(c) {
                        cursor = Some(all.len());
                    }
                    let width = usize::from(inner.width);
                    all.push(Line::from(Span::styled(
                        fit(&v.candidate_line(c), width).trim_end().to_string(),
                        Style::new().fg(t.warn),
                    )));
                    // Each warning on its own line, the consequence first.
                    for d in v.candidate_detail(c) {
                        let style = if d.trim_start().starts_with('⚠') {
                            Style::new().fg(t.warn)
                        } else {
                            t.dim_style()
                        };
                        all.push(Line::from(Span::styled(
                            fit(&d, width).trim_end().to_string(),
                            style,
                        )));
                    }
                }
                let rows = v.section_rows(side);
                if side == Side::Outbound && !v.outbound {
                    all.push(Line::from(Span::styled(
                        format!("{} rows hidden: o shows them", rows.len()),
                        t.dim_style(),
                    )));
                    continue;
                }
                if rows.is_empty() {
                    all.push(Line::from(Span::styled("no rows", t.dim_style())));
                }
                for i in rows {
                    if Some(i) == v.selected() && side == v.section {
                        cursor = Some(all.len());
                    }
                    all.push(row_line(
                        v,
                        i,
                        &model.limits.scope_keys,
                        t,
                        usize::from(inner.width),
                        longest,
                    ));
                }
            }
            let mut first = cursor.map_or(0, |c| c.saturating_sub(height.saturating_sub(1)));
            // The outbound section's first row brings its title along.
            if v.section == Side::Outbound && v.cursor[1] == 0 {
                first = first.min(outbound_title);
            }
            // An outbound section without rows yet: its title and `no rows`.
            if v.section == Side::Outbound && cursor.is_none() {
                first = (outbound_title + 2).saturating_sub(height);
            }
            let all = all
                .into_iter()
                .enumerate()
                .map(|(i, l)| {
                    if Some(i) == cursor {
                        l.patch_style(Style::new().add_modifier(Modifier::REVERSED))
                    } else {
                        l
                    }
                })
                .collect();
            (all, first)
        }
    };
    let total = lines.len();
    let below = total.saturating_sub(first + height);
    let shown: Vec<Line> = lines.into_iter().skip(first).take(height).collect();
    f.render_widget(Paragraph::new(shown), parts[1]);
    let mut more = Vec::new();
    if first > 0 {
        more.push(format!("↑ {first} more above"));
    }
    if below > 0 {
        more.push(format!("↓ {below} more below"));
    }
    let keys = if v.list.is_some() {
        "j/k scroll · Esc back to the rows"
    } else if v.planning {
        "planning… · Esc drops the plan"
    } else {
        if v.candidates.is_empty() {
            "p lists the rules · Enter plans the writes · Space toggles · o outbound"
        } else {
            "p lists the rules · Enter plans · Space toggles · c consolidates · o outbound"
        }
    };
    let keys = if more.is_empty() {
        keys.to_string()
    } else {
        format!("{} · {keys}", more.join(" · "))
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
        Line::from(Span::styled(
            fit(&keys, usize::from(inner.width)),
            t.dim_style(),
        )),
    ];
    f.render_widget(Paragraph::new(footer), parts[2]);
}

#[cfg(test)]
mod tests {
    #[test]
    fn wrap_keeps_the_indent_and_indents_continuations() {
        assert_eq!(
            super::wrap_text("   generated: a b c d", 14),
            vec!["   generated:", "      a b c d"]
        );
        assert_eq!(
            super::wrap_text("abcdefghijklmnop", 10),
            vec!["abcdefghij", "   klmnop"]
        );
    }
}
