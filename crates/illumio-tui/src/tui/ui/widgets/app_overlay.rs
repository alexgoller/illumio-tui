//! The app overlay (`A` / `:app`): one app on one screen, btop style. A
//! summary of its workloads and VENs, inbound and outbound peers side by
//! side, blocked traffic over time, then its policy and its workloads.
use crate::tui::app_view::{AppPanel, AppView, Loadable};
use crate::tui::model::Model;
use crate::tui::ui::theme::Theme;
use crate::tui::ui::widgets::panel::panel;
use pce::app::{AppPeer, PolicyReason, enforcement_word};
use pce::models::{PolicyDecision, VenHealth};
use pce::traffic::compact_count;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::tui::ui::widgets::bars::{spark_cell, sparkline};
use crate::tui::ui::widgets::table::{clip_width, fit as fit_cells};
use pce::traffic::{Timeline, TimelineBucket};

/// Display width, as ratatui measures it.
fn width(s: &str) -> usize {
    Span::raw(s).width()
}

/// `s` padded to `w` cells, or cut with `…` when wider.
fn fit(s: &str, w: usize) -> String {
    if width(s) <= w || w == 0 {
        fit_cells(s, w, false)
    } else {
        fit_cells(&format!("{}…", clip_width(s, w - 1)), w, false)
    }
}

fn mark(d: PolicyDecision) -> &'static str {
    match d {
        PolicyDecision::Allowed => "✓",
        PolicyDecision::PotentiallyBlocked => "~",
        PolicyDecision::Blocked => "✗",
        PolicyDecision::Unknown => "?",
    }
}

/// The rows of a panel, the cursor's row reversed when the panel has the
/// focus, scrolled so the cursor stays visible.
fn rows_paragraph<'a>(
    lines: Vec<Line<'a>>,
    cursor: usize,
    focused: bool,
    height: usize,
) -> Paragraph<'a> {
    let first = cursor.saturating_sub(height.saturating_sub(1));
    let lines: Vec<Line> = lines
        .into_iter()
        .enumerate()
        .skip(first)
        .take(height)
        .map(|(i, l)| {
            if focused && i == cursor {
                l.patch_style(Style::new().add_modifier(Modifier::REVERSED))
            } else {
                l
            }
        })
        .collect();
    Paragraph::new(lines)
}

fn note<'a>(text: impl Into<String>, t: &Theme) -> Paragraph<'a> {
    Paragraph::new(Line::from(Span::styled(text.into(), t.dim_style())))
}

pub fn render_app(model: &Model, f: &mut Frame, area: Rect) {
    let Some(v) = &model.app_view else {
        return;
    };
    let t = &model.theme;
    let body = crate::tui::ui::layout::chrome(area).map_or(area, |c| c.body);
    f.render_widget(Clear, body);
    let title = v.header_title();
    let meta = v.header_meta();
    // The theme's background over the whole body, as the screens have.
    let outer = panel(&title, &meta, t, true).style(t.base());
    let inner = outer.inner(body);
    f.render_widget(outer, body);
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(4),
        Constraint::Length(3),
        Constraint::Min(4),
    ])
    .split(inner);
    render_summary(v, t, f, rows[0]);
    let peers =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(rows[1]);
    render_peers(v, AppPanel::Inbound, t, f, peers[0]);
    render_peers(v, AppPanel::Outbound, t, f, peers[1]);
    render_timeline(v, t, f, rows[2]);
    let lower =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(rows[3]);
    render_policy(v, t, f, lower[0]);
    render_workloads(v, t, f, lower[1]);
}

fn render_summary(v: &AppView, t: &Theme, f: &mut Frame, area: Rect) {
    let lines = match &v.workloads {
        Loadable::Loaded((_, s)) => vec![
            Line::from(vec![
                Span::styled(format!("Workloads {}", s.workloads), t.header_style()),
                Span::raw("   "),
                Span::styled(s.enforcement.text(), t.base()),
                Span::styled(
                    if v.workloads_truncated {
                        "  (truncated)"
                    } else {
                        ""
                    },
                    Style::new().fg(t.warn),
                ),
            ]),
            Line::from(vec![
                Span::styled("VENs  ", t.header_style()),
                Span::styled(s.ven_text(), t.base()),
            ]),
        ],
        Loadable::Loading => vec![Line::from(Span::styled(
            "Workloads  loading…",
            t.dim_style(),
        ))],
        Loadable::Failed(e) => vec![Line::from(Span::styled(
            format!("Workloads  {e} · r retries"),
            Style::new().fg(t.bad),
        ))],
        Loadable::Unavailable(why) => vec![Line::from(Span::styled(
            format!("Workloads  {why}"),
            t.dim_style(),
        ))],
    };
    f.render_widget(Paragraph::new(lines), area);
}

fn peer_line<'a>(p: &AppPeer, cells: usize, t: &Theme) -> Line<'a> {
    let conns = compact_count(p.connections);
    let svc_w = 9;
    let peer_w = cells.saturating_sub(svc_w + conns.len().max(5) + 4);
    Line::from(Span::styled(
        format!(
            "{} {} {} {:>5}",
            fit(&p.peer_text(), peer_w),
            fit(&p.service, svc_w),
            mark(p.decision),
            conns
        ),
        t.decision_style(p.decision),
    ))
}

fn render_peers(v: &AppView, side: AppPanel, t: &Theme, f: &mut Frame, area: Rect) {
    let focused = v.panel == side;
    let title = if v.partial {
        format!("{} · partial", side.title())
    } else {
        side.title().to_string()
    };
    let block = panel(&title, "", t, focused);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let (rows, shown) = match side {
        AppPanel::Inbound => (
            &v.inbound,
            v.focus.direction != pce::traffic::FocusDirection::Out,
        ),
        _ => (
            &v.outbound,
            v.focus.direction != pce::traffic::FocusDirection::In,
        ),
    };
    if !shown {
        let other = if side == AppPanel::Inbound {
            "inbound"
        } else {
            "outbound"
        };
        f.render_widget(note(format!("X shows {other}"), t), inner);
        return;
    }
    if rows.is_empty() {
        f.render_widget(note("no flows in the window", t), inner);
        return;
    }
    let w = inner.width as usize;
    let lines: Vec<Line> = rows.iter().map(|p| peer_line(p, w, t)).collect();
    f.render_widget(
        rows_paragraph(
            lines,
            v.cursor[side.index()],
            focused,
            inner.height as usize,
        ),
        inner,
    );
}

/// The exact timeline's cells: loaded buckets scaled to the largest, in
/// the blocked colour; a capped one (a lower bound) in the warning colour;
/// a missing one `·`. At most `width`, the newest kept.
pub fn exact_sparkline(buckets: &[TimelineBucket], width: usize, t: &Theme) -> Vec<Span<'static>> {
    let max = buckets.iter().filter_map(|b| b.count).max().unwrap_or(0);
    let skip = buckets.len().saturating_sub(width);
    buckets
        .iter()
        .skip(skip)
        .map(|b| match b.count {
            None => Span::styled("·", t.dim_style()),
            Some(v) => {
                let colour = if b.truncated { t.warn } else { t.bad };
                Span::styled(spark_cell(v, max).to_string(), Style::new().fg(colour))
            }
        })
        .collect()
}

/// The panel's meta for an exact timeline: running, partial or complete,
/// and how many buckets are lower bounds.
fn exact_meta(tl: &Timeline, running: bool) -> String {
    let (loaded, total) = (tl.loaded(), tl.buckets.len());
    let mut meta = if running {
        format!("exact · {loaded}/{total} · Esc stops")
    } else if tl.partial {
        format!("exact · partial {loaded}/{total} · t resumes")
    } else {
        format!("exact · {total} buckets")
    };
    let capped = tl.buckets.iter().filter(|b| b.truncated).count();
    if capped > 0 {
        meta.push_str(&format!(" · {capped} capped+"));
    }
    meta
}

/// The blocked timeline: the exact one once `t` loaded any of it (complete,
/// partial or running), else the approximate one from the loaded flows,
/// labelled `approx` with `t: exact`. Either names its worst spike. Every
/// value is derived when the data changes, never here.
fn render_timeline(v: &AppView, t: &Theme, f: &mut Frame, area: Rect) {
    let exact = v.exact.as_ref();
    let meta = exact
        .map(|e| exact_meta(&e.timeline, e.is_running()))
        .unwrap_or_default();
    let block = panel("Blocked over time", &meta, t, false);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let spike = |tl: &Timeline| {
        tl.spike
            .as_ref()
            .map(|s| Span::styled(format!("  {}", s.text()), Style::new().fg(t.bad)))
    };
    if let Some(e) = exact {
        let tl = &e.timeline;
        let tail = spike(tl).or_else(|| {
            (tl.loaded() > 0 && tl.is_quiet())
                .then(|| Span::styled("  nothing blocked", t.dim_style()))
        });
        let room = (inner.width as usize).saturating_sub(tail.as_ref().map_or(0, Span::width));
        let mut spans = exact_sparkline(&tl.buckets, room, t);
        spans.extend(tail);
        f.render_widget(Paragraph::new(Line::from(spans)), inner);
        return;
    }
    let Some(tl) = &v.timeline else {
        f.render_widget(note("no time range in this result", t), inner);
        return;
    };
    if tl.is_quiet() {
        f.render_widget(
            note("no blocked flows in the window · approx · t: exact", t),
            inner,
        );
        return;
    }
    let mut tail: Vec<Span> = spike(tl).into_iter().collect();
    tail.push(Span::styled("  approx · t: exact", t.dim_style()));
    let room = (inner.width as usize).saturating_sub(tail.iter().map(Span::width).sum());
    let mut spans = vec![Span::styled(
        sparkline(&tl.counts(), room),
        Style::new().fg(t.bad),
    )];
    spans.extend(tail);
    f.render_widget(Paragraph::new(Line::from(spans)), inner);
}

fn render_policy(v: &AppView, t: &Theme, f: &mut Frame, area: Rect) {
    let focused = v.panel == AppPanel::Policy;
    let title = format!(
        "{} · {}",
        AppPanel::Policy.title(),
        v.policy_scope.segment()
    );
    let block = panel(&title, "", t, focused);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let rows = match &v.policy {
        Loadable::Loaded(rows) => rows,
        Loadable::Loading => return f.render_widget(note("loading…", t), inner),
        Loadable::Failed(e) => {
            return f.render_widget(
                Paragraph::new(Span::styled(
                    format!("{e} · r retries"),
                    Style::new().fg(t.bad),
                )),
                inner,
            );
        }
        Loadable::Unavailable(why) => return f.render_widget(note(why.clone(), t), inner),
    };
    if rows.is_empty() {
        f.render_widget(note("no ruleset covers this app", t), inner);
        return;
    }
    let w = inner.width as usize;
    let lines: Vec<Line> = rows
        .iter()
        .map(|r| {
            let why = match r.reason {
                PolicyReason::Scope => "",
                PolicyReason::AllWorkloads => " · all",
                PolicyReason::Rules => " · names it",
            };
            let tail = format!(
                "{} rule{}{why}",
                r.rules,
                if r.rules == 1 { "" } else { "s" }
            );
            let name_w = w.saturating_sub(width(&tail) + 5);
            let style = if r.enabled { t.base() } else { t.dim_style() };
            Line::from(Span::styled(
                format!("RS {}  {tail}", fit(&r.name, name_w)),
                style,
            ))
        })
        .collect();
    f.render_widget(
        rows_paragraph(
            lines,
            v.cursor[AppPanel::Policy.index()],
            focused,
            inner.height as usize,
        ),
        inner,
    );
}

fn render_workloads(v: &AppView, t: &Theme, f: &mut Frame, area: Rect) {
    let focused = v.panel == AppPanel::Workloads;
    let block = panel(AppPanel::Workloads.title(), "", t, focused);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let rows = match &v.workloads {
        Loadable::Loaded((rows, _)) => rows,
        Loadable::Loading => return f.render_widget(note("loading…", t), inner),
        Loadable::Failed(e) => {
            return f.render_widget(
                Paragraph::new(Span::styled(
                    format!("{e} · r retries"),
                    Style::new().fg(t.bad),
                )),
                inner,
            );
        }
        Loadable::Unavailable(why) => return f.render_widget(note(why.clone(), t), inner),
    };
    if rows.is_empty() {
        f.render_widget(note("no workload carries these labels", t), inner);
        return;
    }
    let w = inner.width as usize;
    // name, role (7), enforcement (10), the dot: four separators.
    let name_w = w.saturating_sub(7 + 10 + 1 + 3);
    let lines: Vec<Line> = rows
        .iter()
        .map(|wl| {
            let dot = match wl.ven {
                VenHealth::Healthy => Style::new().fg(t.ok),
                VenHealth::Warning => Style::new().fg(t.warn),
                VenHealth::Offline | VenHealth::Error => Style::new().fg(t.bad),
                VenHealth::Unmanaged => t.dim_style(),
            };
            Line::from(vec![
                Span::styled(
                    format!(
                        "{} {} {} ",
                        fit(&wl.name, name_w),
                        fit(wl.role.as_deref().unwrap_or("-"), 7),
                        fit(enforcement_word(&wl.enforcement), 10)
                    ),
                    t.base(),
                ),
                Span::styled("●", dot),
            ])
        })
        .collect();
    f.render_widget(
        rows_paragraph(
            lines,
            v.cursor[AppPanel::Workloads.index()],
            focused,
            inner.height as usize,
        ),
        inner,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::ui::theme::Theme;

    #[test]
    fn fit_pads_and_cuts_by_display_width() {
        assert_eq!(fit("web", 5), "web  ");
        let wide = fit("日本語ab", 4);
        assert_eq!(width(&wide), 4, "{wide:?}");
        assert!(wide.starts_with("日…"), "{wide:?}");
        assert_eq!(fit("abc", 0), "");
        assert_eq!(sparkline(&[0, 1, 7], 2), "▂█", "the newest two");
    }

    fn b(count: Option<u64>, truncated: bool) -> TimelineBucket {
        TimelineBucket {
            start: "2026-10-10T00:00:00Z".parse().unwrap(),
            end: "2026-10-10T01:00:00Z".parse().unwrap(),
            count,
            truncated,
        }
    }

    #[test]
    fn exact_sparkline_dots_the_missing_buckets_and_warns_on_capped_ones() {
        let t = Theme::from_name("dark");
        let spans = exact_sparkline(
            &[b(Some(1), false), b(None, false), b(Some(8), true)],
            10,
            &t,
        );
        let text: String = spans.iter().map(|s| s.content.to_string()).collect();
        assert_eq!(text, "▁·█");
        assert_eq!(spans[0].style.fg, Some(t.bad));
        assert_eq!(spans[1].style, t.dim_style());
        assert_eq!(
            spans[2].style.fg,
            Some(t.warn),
            "a capped bucket is a lower bound"
        );
        let newest = exact_sparkline(
            &[b(Some(1), false), b(None, false), b(Some(8), true)],
            2,
            &t,
        );
        assert_eq!(newest.len(), 2, "the newest buckets that fit");
    }

    #[test]
    fn exact_meta_says_running_partial_complete_and_capped() {
        let buckets = pce::traffic::equal_buckets(
            "2026-10-10T00:00:00Z".parse().unwrap(),
            "2026-10-10T04:00:00Z".parse().unwrap(),
            4,
        );
        let part = pce::traffic::exact_timeline(&buckets, &[Some((1, false)), None, None, None]);
        assert_eq!(exact_meta(&part, true), "exact · 1/4 · Esc stops");
        assert_eq!(exact_meta(&part, false), "exact · partial 1/4 · t resumes");
        let full = pce::traffic::exact_timeline(
            &buckets,
            &[
                Some((1, false)),
                Some((1, true)),
                Some((1, false)),
                Some((1, false)),
            ],
        );
        assert_eq!(exact_meta(&full, false), "exact · 4 buckets · 1 capped+");
    }
}
