//! `:readiness` and the enforcement overlay, btop style. The readiness
//! list is one table of apps; the enforcement overlay has its five steps
//! down the left and the selected step's detail on the right.
use crate::snapshot::baseline::{TrendPoint, sparkline};
use crate::tui::enforce::{EnforceView, Missing, Step, Watch};
use crate::tui::model::Model;
use crate::tui::ui::theme::Theme;
use crate::tui::ui::widgets::panel::panel;
use crate::tui::ui::widgets::table::{clip_width, fit as fit_cells};
use pce::readiness::{AppReadiness, Verdict};
use pce::traffic::compact_count;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};

/// `s` padded to `w` cells, or cut with `…` when wider.
fn fit(s: &str, w: usize) -> String {
    if Span::raw(s).width() <= w || w == 0 {
        fit_cells(s, w, false)
    } else {
        fit_cells(&format!("{}…", clip_width(s, w - 1)), w, false)
    }
}

fn right(s: &str, w: usize) -> String {
    fit_cells(s, w, true)
}

fn verdict_style(v: Verdict, t: &Theme) -> Style {
    match v {
        Verdict::Ready | Verdict::Enforced => Style::new().fg(t.ok),
        Verdict::NeedsRules => Style::new().fg(t.bad),
        Verdict::NeedsProvision => Style::new().fg(t.warn),
        Verdict::NotEvaluated => t.dim_style(),
    }
}

/// `2 req`, `· cached` style meta as the app overlay has it.
fn req_text(n: u32) -> String {
    format!("{n} req")
}

/// The result readiness reads: its window, whether it carries the draft
/// decisions, and what is missing.
fn notes(model: &Model) -> Vec<(String, Style)> {
    let t = &model.theme;
    let mut out = Vec::new();
    let missing = Missing::of(model);
    if missing.any() {
        out.push((
            format!("loading {}: the verdicts wait for them", missing.text()),
            Style::new().fg(t.warn),
        ));
    }
    if model.traffic.data.is_some() && !model.traffic.draft_loaded() {
        out.push((
            "not evaluated: D evaluates the result on screen against the draft".into(),
            Style::new().fg(t.warn),
        ));
    }
    if let Some(d) = model.traffic.data.as_ref() {
        if let Some(what) = d.filter.readiness_narrowing() {
            out.push((
                format!("filtered ({what}): the verdicts rest on part of the flows"),
                Style::new().fg(t.warn),
            ));
        }
        if d.truncated || d.incomplete {
            out.push((
                "the result is truncated or incomplete: flows are missing".into(),
                Style::new().fg(t.warn),
            ));
        }
    }
    if model.traffic.focus_result.is_some() {
        out.push((
            "a focused result is on screen: apps outside its focus are partial".into(),
            t.dim_style(),
        ));
    }
    out
}

fn window_text(model: &Model) -> String {
    model.traffic.data.as_ref().map_or_else(
        || model.traffic.filter.window.clone(),
        |d| d.filter.window.clone(),
    )
}

pub fn render_readiness(model: &Model, f: &mut Frame, area: Rect) {
    let Some(v) = &model.readiness_view else {
        return;
    };
    let t = &model.theme;
    let body = crate::tui::ui::layout::chrome(area).map_or(area, |c| c.body);
    f.render_widget(Clear, body);
    let draft = if model.traffic.draft_loaded() {
        "draft"
    } else {
        "not evaluated"
    };
    let title = format!("Readiness · last {} · {draft}", window_text(model));
    let meta = req_text(v.requests);
    let outer = panel(&title, &meta, t, true).style(t.base());
    let inner = outer.inner(body);
    f.render_widget(outer, body);
    let w = inner.width as usize;
    let mut lines = vec![Line::from(vec![
        Span::styled(v.counts_text(), t.header_style()),
        Span::styled(format!("  · sort {}", v.sort.text()), t.dim_style()),
        Span::styled(
            if v.searching || !v.search.is_empty() {
                format!("  · /{}{}", v.search, if v.searching { "▏" } else { "" })
            } else {
                String::new()
            },
            Style::new().fg(t.accent),
        ),
    ])];
    for (n, style) in notes(model) {
        lines.push(Line::from(Span::styled(fit(&n, w), style)));
    }
    // Wider: the enforcement mix; wider still: the connections in and out.
    let cols = Cols {
        mix: w >= 110,
        conns: w >= 150,
    };
    let longest = v.rows.iter().map(|r| Span::raw(&r.app).width()).max();
    let app_w = (longest.unwrap_or(3) + 2).clamp(10, 28);
    // The trend (#85): two snapshots with readiness at least; a block per
    // snapshot and a gap, where it fits.
    let trend = v.trend.as_ref().filter(|t| t.snapshots >= 2);
    let trend_w = trend.map_or(0, |t| {
        (t.snapshots.clamp(5, crate::snapshot::baseline::TREND_MAX) + 2)
            .min(w.saturating_sub(app_w + cols.width()))
    });
    let warn_w = w.saturating_sub(app_w + cols.width() + trend_w);
    let mut header = format!(
        "{}{}{}",
        fit("app", app_w),
        right("wl", 4),
        right("flows", 6)
    );
    if cols.conns {
        header.push_str(&right("in", 8));
        header.push_str(&right("out", 8));
    }
    header.push_str(&format!(
        "{}{}{}{}",
        right("block", 8),
        right("cov", 7),
        right("pend", 5),
        fit("  verdict", 16)
    ));
    if trend_w > 0 {
        header.push_str(&fit("trend", trend_w));
    }
    if cols.mix {
        header.push_str(&fit("enforcement", 25));
    }
    lines.push(Line::from(Span::styled(
        format!("{header}{}", fit(" warnings", warn_w)),
        t.header_style(),
    )));
    let top = lines.len();
    let height = (inner.height as usize).saturating_sub(top + 1);
    let shown = v.shown();
    if shown.is_empty() {
        lines.push(Line::from(Span::styled(
            if v.rows.is_empty() {
                "no app in the result on screen"
            } else {
                "no app matches the search"
            },
            t.dim_style(),
        )));
    }
    let first = v.cursor.saturating_sub(height.saturating_sub(1));
    for (i, r) in shown.iter().enumerate().skip(first).take(height) {
        let points = trend.and_then(|tr| tr.of(&r.app)).unwrap_or_default();
        let mut line = readiness_row(r, app_w, warn_w, cols, (points, trend_w), t);
        if i == v.cursor {
            line = line.patch_style(Style::new().add_modifier(Modifier::REVERSED));
        }
        lines.push(line);
    }
    let rows = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).split(inner);
    f.render_widget(Paragraph::new(lines), rows[0]);
    f.render_widget(
        Paragraph::new(Span::styled(
            fit(
                "Enter enforce · j/k · s sort · / search · D draft · ? help · Esc back",
                w,
            ),
            t.dim_style(),
        )),
        rows[1],
    );
}

/// An app's trend in `w` cells: a block per snapshot (its gaps, scaled),
/// in its verdict's colour, the newest last; cut from the oldest; padded.
fn trend_spans<'a>(points: &[TrendPoint], w: usize, t: &Theme) -> Vec<Span<'a>> {
    let room = w.saturating_sub(2);
    let points = &points[points.len().saturating_sub(room)..];
    let blocks = sparkline(&points.iter().map(|p| p.gaps).collect::<Vec<_>>());
    let mut out: Vec<Span<'a>> = blocks
        .into_iter()
        .zip(points)
        .map(|(b, p)| Span::styled(b.to_string(), verdict_style(p.verdict, t)))
        .collect();
    out.push(Span::raw(" ".repeat(w - points.len())));
    out
}

/// The readiness table's optional columns.
#[derive(Clone, Copy)]
struct Cols {
    /// The enforcement mix (25 cells).
    mix: bool,
    /// Connections in and out (8 cells each).
    conns: bool,
}

impl Cols {
    /// Every column's width but the app's and the warnings'.
    fn width(self) -> usize {
        4 + 6 + 8 + 7 + 5 + 16 + if self.mix { 25 } else { 0 } + if self.conns { 16 } else { 0 }
    }
}

fn readiness_row<'a>(
    r: &AppReadiness,
    app_w: usize,
    warn_w: usize,
    cols: Cols,
    (points, trend_w): (&[TrendPoint], usize),
    t: &Theme,
) -> Line<'a> {
    let c = &r.coverage;
    let evaluated = r.verdict != Verdict::NotEvaluated || c.ratio.is_some();
    let block = if evaluated {
        compact_count(c.total.would_block)
    } else {
        "-".into()
    };
    let mut nums = format!(
        "{}{}",
        right(&r.workloads.total.to_string(), 4),
        right(&compact_count(r.evidence.flows), 6),
    );
    if cols.conns {
        nums.push_str(&right(&compact_count(c.inbound.connections), 8));
        nums.push_str(&right(&compact_count(c.outbound.connections), 8));
    }
    nums.push_str(&right(&block, 8));
    nums.push_str(&right(&c.percent_text(), 7));
    nums.push_str(&right(&r.pending.count.to_string(), 5));
    let mut spans = vec![
        Span::styled(fit(&r.app, app_w), t.base()),
        Span::styled(nums, t.base()),
        Span::styled(
            fit(&format!("  {}", r.verdict.text()), 16),
            verdict_style(r.verdict, t),
        ),
    ];
    if trend_w > 0 {
        spans.extend(trend_spans(points, trend_w, t));
    }
    if cols.mix {
        spans.push(Span::styled(
            fit(&r.workloads.enforcement.text(), 25),
            t.dim_style(),
        ));
    }
    spans.push(Span::styled(
        fit(&format!(" {}", r.warnings_text()), warn_w),
        Style::new().fg(t.warn),
    ));
    Line::from(spans)
}

pub fn render_enforce(model: &Model, f: &mut Frame, area: Rect) {
    let Some(v) = &model.enforce_view else {
        return;
    };
    let t = &model.theme;
    let body = crate::tui::ui::layout::chrome(area).map_or(area, |c| c.body);
    f.render_widget(Clear, body);
    let verdict = v.readiness.as_ref().map(|r| r.verdict);
    let title = format!("Enforce · {} · last {}", v.app_text(), window_text(model));
    let meta = format!(
        "{} · {}",
        verdict.map_or("-", Verdict::text),
        req_text(v.requests)
    );
    let outer = panel(&title, &meta, t, true).style(t.base());
    let inner = outer.inner(body);
    f.render_widget(outer, body);
    let cols = Layout::horizontal([Constraint::Length(27), Constraint::Fill(1)]).split(inner);
    render_steps(model, v, f, cols[0]);
    let detail_title = format!("{} {}", v.step.index() + 1, v.step.title());
    let block = panel(&detail_title, "", t, true);
    let detail = block.inner(cols[1]);
    f.render_widget(block, cols[1]);
    let rows = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).split(detail);
    let w = detail.width as usize;
    let (lines, hint) = match v.step {
        Step::Readiness => readiness_lines(model, v, w),
        Step::Rules => rules_lines(model, v, w, rows[0].height as usize),
        Step::Provision => provision_lines(model, v, w, rows[0].height as usize),
        Step::Enforce => enforce_lines(model, v, w),
        Step::Watch => watch_lines(model, v, w),
    };
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), rows[0]);
    f.render_widget(
        Paragraph::new(Span::styled(fit(&hint, w), t.dim_style())),
        rows[1],
    );
}

fn render_steps(model: &Model, v: &EnforceView, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let block = panel("Steps", "", t, false);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let missing = Missing::of(model);
    let w = inner.width as usize;
    let mut lines = Vec::new();
    for s in Step::ALL {
        let state = v.step_state(s, &missing);
        let style = match (s, v.readiness.as_ref().map(|r| r.verdict)) {
            (Step::Readiness, Some(vd)) if !missing.any() => verdict_style(vd, t),
            (Step::Rules, Some(Verdict::NeedsRules)) => Style::new().fg(t.bad),
            (Step::Provision, Some(Verdict::NeedsProvision)) => Style::new().fg(t.warn),
            (Step::Enforce, _) if state == "ready" || state == "enforced" => Style::new().fg(t.ok),
            (Step::Enforce, _) => Style::new().fg(t.warn),
            (Step::Watch, _) if state.ends_with(" blocked") && state != "0 blocked" => {
                Style::new().fg(t.bad)
            }
            _ => t.dim_style(),
        };
        let mark = if s == v.step { "▸" } else { " " };
        let head = format!("{mark}{} {}", s.index() + 1, s.title());
        let mut line = Line::from(vec![
            Span::styled(fit(&head, 13), t.base()),
            Span::styled(fit(&state, w.saturating_sub(13)), style),
        ]);
        if s == v.step {
            line = line.patch_style(Style::new().add_modifier(Modifier::BOLD));
        }
        lines.push(line);
    }
    lines.push(Line::from(""));
    for (n, style) in notes(model) {
        lines.push(Line::from(Span::styled(n, style)));
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn kv<'a>(key: &str, value: String, style: Style, t: &Theme) -> Line<'a> {
    Line::from(vec![
        Span::styled(fit(key, 12), t.header_style()),
        Span::styled(value, style),
    ])
}

fn readiness_lines<'a>(model: &Model, v: &EnforceView, _w: usize) -> (Vec<Line<'a>>, String) {
    let t = &model.theme;
    let hint = "Tab/1-5 steps · D draft · ? help · Esc back".to_string();
    let Some(r) = &v.readiness else {
        return (
            vec![Line::from(Span::styled("not computed yet", t.dim_style()))],
            hint,
        );
    };
    let wl = &r.workloads;
    let c = &r.coverage;
    let e = &r.evidence;
    let side = |name: &str, s: &pce::readiness::SideCounts| {
        let mut text = format!(
            "{name} {}: {} allowed · {} block",
            compact_count(s.connections),
            compact_count(s.allowed),
            compact_count(s.would_block),
        );
        if s.accepted > 0 {
            text.push_str(&format!(" · {} accepted", compact_count(s.accepted)));
        }
        text
    };
    let mut lines = vec![
        kv(
            "Verdict",
            r.verdict.text().to_string(),
            verdict_style(r.verdict, t),
            t,
        ),
        kv(
            "Workloads",
            format!(
                "{} · {} managed · {} unmanaged",
                wl.total, wl.managed, wl.unmanaged
            ),
            t.base(),
            t,
        ),
        kv(
            "VENs",
            format!(
                "{} offline · {} without {}",
                wl.offline, wl.missing_tier, model.limits.tier_key
            ),
            if wl.offline > 0 {
                Style::new().fg(t.warn)
            } else {
                t.base()
            },
            t,
        ),
        kv("Enforcement", wl.enforcement.text(), t.base(), t),
        kv(
            "Evidence",
            format!(
                "{} · {} flows · {} conns",
                e.window,
                compact_count(e.flows),
                compact_count(e.connections),
            ),
            if e.little {
                Style::new().fg(t.warn)
            } else {
                t.base()
            },
            t,
        ),
        kv(
            "Coverage",
            format!("{} of the connections", c.percent_text()),
            t.base(),
            t,
        ),
        kv("", side("in ", &c.inbound), t.dim_style(), t),
        kv("", side("out", &c.outbound), t.dim_style(), t),
        kv(
            "Gaps",
            format!("{} tuples would block", r.gap_count),
            if r.gap_count > 0 {
                Style::new().fg(t.bad)
            } else {
                t.base()
            },
            t,
        ),
        kv(
            "Pending",
            if r.pending.count > 0 {
                format!("{} to provision first", r.pending.count)
            } else {
                "none for the app".into()
            },
            if r.pending.count > 0 {
                Style::new().fg(t.warn)
            } else {
                t.base()
            },
            t,
        ),
    ];
    if r.warnings.is_empty() {
        lines.push(kv("Warnings", "none".into(), t.dim_style(), t));
    } else {
        for (i, wn) in r.warnings.iter().enumerate() {
            lines.push(kv(
                if i == 0 { "Warnings" } else { "" },
                wn.text(),
                Style::new().fg(t.warn),
                t,
            ));
        }
    }
    partial_lines(v, &mut lines, t);
    if r.verdict == Verdict::NotEvaluated {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "D evaluates the result on screen against the draft policy",
            Style::new().fg(t.warn),
        )));
    }
    (lines, hint)
}

/// Why the result may miss flows of the app (a move is then typed
/// through).
fn partial_lines<'a>(v: &EnforceView, lines: &mut Vec<Line<'a>>, t: &Theme) {
    for (i, why) in v.partial.iter().enumerate() {
        lines.push(kv(
            if i == 0 { "Partial" } else { "" },
            why.clone(),
            Style::new().fg(t.warn),
            t,
        ));
    }
}

fn scroll_list<'a>(rows: Vec<Line<'a>>, cursor: usize, height: usize) -> Vec<Line<'a>> {
    let first = cursor.saturating_sub(height.saturating_sub(1));
    rows.into_iter()
        .enumerate()
        .skip(first)
        .take(height)
        .map(|(i, l)| {
            if i == cursor {
                l.patch_style(Style::new().add_modifier(Modifier::REVERSED))
            } else {
                l
            }
        })
        .collect()
}

fn rules_lines<'a>(
    model: &Model,
    v: &EnforceView,
    w: usize,
    height: usize,
) -> (Vec<Line<'a>>, String) {
    let t = &model.theme;
    let hint = "G generator · w rule for the gap · m accept-blocked · j/k".to_string();
    let r = v.readiness.as_ref();
    let gaps = r.and_then(|r| r.gaps.as_deref()).unwrap_or_default();
    if r.is_none_or(|r| r.verdict == Verdict::NotEvaluated) {
        return (
            vec![Line::from(Span::styled(
                "no draft decisions: D evaluates the result first",
                Style::new().fg(t.warn),
            ))],
            hint,
        );
    }
    if gaps.is_empty() {
        return (
            vec![Line::from(Span::styled(
                "no gaps: the draft allows (or marks accept) every app flow",
                Style::new().fg(t.ok),
            ))],
            hint,
        );
    }
    let svc_w = 12;
    let peer_w = w.saturating_sub(4 + svc_w + 6 + 8).max(8);
    let mut lines = vec![Line::from(Span::styled(
        format!(
            "{}{}{}{}{}",
            fit("dir", 4),
            fit("peer", peer_w),
            fit("service", svc_w),
            right("flows", 6),
            right("conns", 8)
        ),
        t.header_style(),
    ))];
    let rows: Vec<Line> = gaps
        .iter()
        .map(|g| {
            Line::from(Span::styled(
                format!(
                    "{}{}{}{}{}",
                    fit(g.direction.as_str(), 4),
                    fit(&g.peer_text, peer_w),
                    fit(&g.service, svc_w),
                    right(&compact_count(g.flows), 6),
                    right(&compact_count(g.connections), 8)
                ),
                Style::new().fg(t.bad),
            ))
        })
        .collect();
    lines.extend(scroll_list(
        rows,
        v.cursor[Step::Rules.index()],
        height.saturating_sub(1),
    ));
    (lines, hint)
}

fn provision_lines<'a>(
    model: &Model,
    v: &EnforceView,
    w: usize,
    height: usize,
) -> (Vec<Line<'a>>, String) {
    let t = &model.theme;
    let changes = v
        .readiness
        .as_ref()
        .map(|r| r.pending.changes.as_slice())
        .unwrap_or_default();
    let hint = if changes.is_empty() {
        "nothing to provision for the app".to_string()
    } else {
        format!(
            "P provisions these {} (preview, then y; writes) · j/k",
            changes.len()
        )
    };
    if Missing::of(model).pending {
        return (
            vec![Line::from(Span::styled(
                "loading the pending changes…",
                t.dim_style(),
            ))],
            hint,
        );
    }
    let mut lines = vec![Line::from(Span::styled(
        "Enforcement uses the active policy: the app's unprovisioned changes block it.",
        t.dim_style(),
    ))];
    if changes.is_empty() {
        lines.push(Line::from(Span::styled(
            "no pending change touches the app",
            Style::new().fg(t.ok),
        )));
        return (lines, hint);
    }
    lines.push(Line::from(Span::styled(
        format!(
            "{}{}{}",
            fit("change", 9),
            fit("type", 14),
            fit("name", w.saturating_sub(23))
        ),
        t.header_style(),
    )));
    let rows: Vec<Line> = changes
        .iter()
        .map(|c| {
            Line::from(Span::styled(
                format!(
                    "{}{}{}",
                    fit(&c.change_type, 9),
                    fit(&c.object_type, 14),
                    fit(c.name.as_deref().unwrap_or(&c.href), w.saturating_sub(23))
                ),
                Style::new().fg(t.warn),
            ))
        })
        .collect();
    lines.extend(scroll_list(
        rows,
        v.cursor[Step::Provision.index()],
        height.saturating_sub(2),
    ));
    (lines, hint)
}

fn enforce_lines<'a>(model: &Model, v: &EnforceView, _w: usize) -> (Vec<Line<'a>>, String) {
    let t = &model.theme;
    let radio = |on: bool| if on { "(●)" } else { "( )" };
    let full = v.mode == pce::write::EnforcementMode::Full;
    let mut lines = vec![
        kv(
            "Target",
            format!("{} full   {} selective", radio(full), radio(!full)),
            t.base(),
            t,
        ),
        kv("Visibility", v.visibility_text().to_string(), t.base(), t),
        kv("Plan", v.plan_text.clone(), t.base(), t),
    ];
    if !full {
        lines.push(kv(
            "",
            "selective: readiness judges allow coverage only".into(),
            t.dim_style(),
            t,
        ));
    }
    let missing = Missing::of(model);
    let (gate, style) = match &v.readiness {
        _ if missing.any() => (
            format!("waiting for {}", missing.text()),
            Style::new().fg(t.warn),
        ),
        Some(r) if r.verdict.allows_enforcement() && !v.partial.is_empty() => (
            format!(
                "{} on a partial result: Enter, then type {}",
                r.verdict.text(),
                crate::tui::enforce::ENFORCE_WORD
            ),
            Style::new().fg(t.warn),
        ),
        Some(r) if r.verdict.allows_enforcement() => (
            format!("{}: Enter reviews (bulk confirm)", r.verdict.text()),
            Style::new().fg(t.ok),
        ),
        Some(r) => (
            format!(
                "{}: Enter, then type {}",
                r.verdict.text(),
                crate::tui::enforce::ENFORCE_WORD
            ),
            Style::new().fg(t.bad),
        ),
        None => ("-".into(), t.dim_style()),
    };
    lines.push(kv("Readiness", gate, style, t));
    partial_lines(v, &mut lines, t);
    if let Some(typed) = &v.typing {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(
                format!("type {}: ", crate::tui::enforce::ENFORCE_WORD),
                Style::new().fg(t.bad).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("{typed}▏"), t.base()),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(kv(
        "Record",
        model.records_dir.as_ref().map_or_else(
            || "no place for change records: a move is refused".into(),
            |d| d.display().to_string(),
        ),
        if model.records_dir.is_some() {
            t.dim_style()
        } else {
            Style::new().fg(t.bad)
        },
        t,
    ));
    let writes = if model.writes.enabled {
        ("on".to_string(), t.dim_style())
    } else {
        (
            "off: :writes on arms them".to_string(),
            Style::new().fg(t.warn),
        )
    };
    lines.push(kv("Writes", writes.0, writes.1, t));
    let hint = if v.typing.is_some() {
        "type the word · Enter · Esc cancels".to_string()
    } else {
        "j/k full/selective · v visibility · Enter review · Esc back".to_string()
    };
    (lines, hint)
}

fn watch_lines<'a>(model: &Model, v: &EnforceView, w: usize) -> (Vec<Line<'a>>, String) {
    let t = &model.theme;
    if let Some(p) = &v.rollback {
        let mut lines = vec![Line::from(Span::styled(
            format!(
                "Rollback of {} · {} to restore · {} skipped",
                file_name(&p.record_path),
                p.count,
                p.skipped.len()
            ),
            t.header_style(),
        ))];
        for (to, plan) in &p.plans {
            lines.push(Line::from(Span::styled(
                format!("restore to {}:", to.text()),
                t.base(),
            )));
            for i in &plan.items {
                lines.push(Line::from(Span::styled(
                    format!("  {} · {}", i.name, i.change),
                    Style::new().fg(t.accent),
                )));
            }
        }
        for s in &p.skipped {
            lines.push(Line::from(Span::styled(
                fit(&format!("skipped {} · {}", s.hostname, s.reason), w),
                Style::new().fg(t.warn),
            )));
        }
        if p.needs_count {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled(
                    format!("type {} (the workload count): ", p.count),
                    Style::new().fg(t.bad).add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("{}▏", p.typed), t.base()),
            ]));
        }
        return (lines, "y sends the rollback (writes) · Esc cancels".into());
    }
    let hint = "r blocked since the change (1 query) · R rollback (preview)".to_string();
    let Some((path, rec)) = &v.record else {
        let why = v.record_error.clone().unwrap_or_else(|| {
            format!("no change record for {} yet: step 4 moves it", v.app_text())
        });
        return (vec![Line::from(Span::styled(why, t.dim_style()))], hint);
    };
    let mut lines = vec![
        kv("Record", file_name(path), t.base(), t),
        kv(
            "Moved",
            format!(
                "{} · to {}{}",
                rec.at,
                rec.to.as_ref().map_or_else(|| "?".into(), |m| m.text()),
                if rec.readiness.as_ref().is_some_and(|r| r.forced) {
                    " · forced (not ready)"
                } else {
                    ""
                }
            ),
            t.base(),
            t,
        ),
    ];
    for (i, wl) in rec.workloads.iter().enumerate() {
        lines.push(kv(
            if i == 0 { "Workloads" } else { "" },
            fit(
                &format!(
                    "{} · {} → {} · {}",
                    wl.hostname,
                    wl.before.text(),
                    wl.after.text(),
                    wl.result
                ),
                w.saturating_sub(12),
            ),
            if wl.moved() {
                t.base()
            } else {
                Style::new().fg(t.warn)
            },
            t,
        ));
    }
    if let Some(by) = &v.rolled_back_by {
        lines.push(kv(
            "Rolled back",
            format!("by {}", file_name(by)),
            Style::new().fg(t.warn),
            t,
        ));
    }
    lines.push(Line::from(""));
    match &v.watch {
        Watch::Idle => lines.push(kv(
            "Watch",
            "r: the blocked flows since the change (nothing refreshes by itself)".into(),
            t.dim_style(),
            t,
        )),
        Watch::Running { .. } => {
            lines.push(kv("Watch", "querying…".into(), t.dim_style(), t));
        }
        Watch::Failed(e) => lines.push(kv("Watch", e.clone(), Style::new().fg(t.bad), t)),
        Watch::Done(r) => {
            let b = &r.blocked;
            lines.push(kv(
                "Watch",
                format!(
                    "{} flows · {} conns blocked since {}{}{}",
                    b.flows,
                    b.connections,
                    r.since,
                    if b.accepted > 0 {
                        format!(" · {} accepted by marks", b.accepted)
                    } else {
                        String::new()
                    },
                    if r.truncated {
                        " · capped: more may exist"
                    } else if r.incomplete {
                        " · a region did not answer"
                    } else {
                        ""
                    }
                ),
                if b.flows > 0 {
                    Style::new().fg(t.bad)
                } else {
                    Style::new().fg(t.ok)
                },
                t,
            ));
            for g in &b.tuples {
                lines.push(kv(
                    "",
                    fit(
                        &format!(
                            "{} {} {} · {} flows · {} conns",
                            g.direction.as_str(),
                            g.peer_text,
                            g.service,
                            g.flows,
                            g.connections
                        ),
                        w.saturating_sub(12),
                    ),
                    Style::new().fg(t.bad),
                    t,
                ));
            }
        }
    }
    (lines, hint)
}

fn file_name(p: &std::path::Path) -> String {
    p.file_name()
        .map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into())
}
