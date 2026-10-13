//! Modal overlays drawn on top of the current screen: help and the `:` prompt.
use crate::tui::diff::{ObjectDiff, Sign};
use crate::tui::filter::{FIELD_NAMES, FilterEditor};
use crate::tui::form::{Form, clip, visible_tail};
use crate::tui::model::{Model, Screen, TrafficView, rule_kind_name};
use crate::tui::picker::KeyPicker;
use crate::tui::ui::layout::centered;
use crate::tui::ui::screens::workloads::plain;
use crate::tui::ui::theme::Theme;
use crate::tui::ui::widgets::panel::panel;
use crate::tui::update::REVERT_WORD;
use pce::RuleView;
use pce::write::{ObjectKind, Plan, WriteRequest};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Wrap};

pub const HELP_LINES: &[(&str, &str)] = &[
    (
        "1 2 3 4 5",
        "Traffic · Policy · Objects · Workloads · Events",
    ),
    (":", "command prompt: :traffic :policy :objects :workloads"),
    ("", ":events  :iplists  :filter <expr>  :window <dur>"),
    ("", ":profile <name>  :draft  :active  :help  :q"),
    ("", ":writes [on|off]  :export[!] <path>"),
    ("", "(the table on screen as CSV; ! overwrites)"),
    ("?", "this help"),
    ("q / Ctrl-C", "quit"),
    ("↑ ↓ PgUp PgDn Home End", "move in the current table"),
    ("s", "cycle sort column and direction"),
    ("/", "search in the current table (Enter keep, Esc clear)"),
    ("r", "refresh the current screen (bypasses the cache)"),
    ("a", "toggle auto-refresh for the current screen"),
    (
        "Traffic  v",
        "cycle Flows · App-to-app · Mesh · Blocked · Treemap",
    ),
    (
        "Traffic  f",
        "edit filter: window src dst service decisions",
    ),
    (
        "Traffic  + / -",
        "widen (fetches the older slice only) / narrow",
    ),
    ("", "window steps 1h 6h 12h 1d 2d 3d 1w 2w 4w 90d"),
    (
        "Traffic  B",
        "backfill a truncated result in slices (asks first)",
    ),
    ("", "newest first, one at a time; Esc stops after one"),
    (
        "Traffic  ] / [",
        "raise / lower the result cap (re-runs the query)",
    ),
    ("", "cap steps 500 1000 2000 5000 10000 25000 … 200000"),
    ("Traffic  g", "pick the label keys every view groups by"),
    (
        "Traffic  Enter",
        "detail (flows) · drill into a row or section header",
    ),
    (
        "App-to-app x / X",
        "pairs · in · out · both (sections per app) · back",
    ),
    (
        "App-to-app o",
        "pairs by service: one row per src, dst, port/proto",
    ),
    (
        "App-to-app d",
        "pairs and sections: all · blocked · blocked only",
    ),
    (
        "Sections Space / z",
        "fold a section · fold or unfold all (also Blocked)",
    ),
    ("Mesh     u / p", "show or hide the user · process columns"),
    (
        "Treemap  hjkl Enter b",
        "move · zoom, then drill · conns/bytes · Esc out",
    ),
    (
        "Traffic  w",
        "draft allow rule from a blocked flow or row (writes)",
    ),
    (
        "Form     Ctrl-N Ctrl-P",
        "complete from the index · cycle back",
    ),
    (
        "Filter   Ctrl-N Ctrl-P",
        "complete the filter fields · cycle back",
    ),
    ("w form   Ctrl-A Ctrl-R", "all labels · new ruleset"),
    ("Policy   d", "toggle draft / active"),
    ("Policy   p", "pending changes"),
    (
        "Policy   Tab / Enter",
        "pane · rule detail · on a ruleset: load deny rules",
    ),
    (
        "Policy   N",
        "new deny rule in the selected ruleset (rules pane)",
    ),
    (
        "Policy   i",
        "rule impact: load the workloads once (1 request)",
    ),
    (
        "Objects  Tab / Enter",
        "next tab: labels, groups, services, IP lists · detail",
    ),
    (
        "Labels   f / i / u",
        "key filter · ignored keys · unused only (asks first)",
    ),
    (
        "Labels   /=text /k=text",
        "search values only · the values of key k",
    ),
    (
        "IP lists Enter",
        "detail: ranges · FQDNs · size · rules using it",
    ),
    (
        "Workloads t/T h H Enter",
        "Traffic as src/dst · unhealthy · health sort · detail",
    ),
    (
        "Events   f / Enter / S",
        "filter · event as JSON · sort by severity",
    ),
    (
        "n / e / x (writes)",
        "new/edit/delete: Objects, IP lists, Workloads, Policy",
    ),
    (
        "Pending  j/k Space Enter",
        "select · mark · diff of a pending entry",
    ),
    (
        "Pending  P",
        "provision the marked entries, or all (writes)",
    ),
    (
        "Pending  R",
        "revert marked entries (draft-only ones are deleted)",
    ),
    ("y / n", "send / cancel a shown request"),
    ("Esc", "close overlay / cancel"),
];

/// The first help line shown when scrolled to the end, for a terminal
/// `height` rows tall (the box is as tall as the lines allow, borders included).
pub fn help_last_scroll(height: u16) -> usize {
    let rows = (HELP_LINES.len() + 2)
        .min(height as usize)
        .saturating_sub(2);
    HELP_LINES.len().saturating_sub(rows)
}

pub fn render_help(model: &Model, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let total = HELP_LINES.len();
    let r = centered(area, area.width.min(84), (total as u16) + 2);
    f.render_widget(Clear, r);
    let rows = r.height.saturating_sub(2) as usize;
    let first = model.help_scroll.min(help_last_scroll(area.height));
    let meta = if total > rows {
        format!(
            "{}-{} of {total} · j/k scroll · Esc closes",
            first + 1,
            (first + rows).min(total)
        )
    } else {
        "Esc closes".to_string()
    };
    let block = panel("Keys", &meta, t, true);
    let lines: Vec<Line> = HELP_LINES
        .iter()
        .skip(first)
        .take(rows)
        .map(|(k, d)| {
            Line::from(vec![
                Span::styled(format!("{k:<25}"), t.header_style()),
                Span::styled(*d, t.base()),
            ])
        })
        .collect();
    f.render_widget(Paragraph::new(lines).block(block).style(t.base()), r);
}

pub fn render_prompt(model: &Model, input: &str, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let r = Rect::new(
        area.x,
        area.y + area.height.saturating_sub(1),
        area.width,
        1,
    );
    f.render_widget(Clear, r);
    let line = Line::from(vec![
        Span::styled(":", t.header_style()),
        Span::styled(input.to_string(), t.base()),
        Span::styled("▏", t.dim_style()),
    ]);
    f.render_widget(Paragraph::new(line).style(t.base()), r);
}

/// The `g` picker: a checkbox per key (the cursor row highlighted), `all
/// labels`, the editable `keys:` line and the key hints. The rows scroll
/// when the box is shorter than the list; the last three lines stay.
pub fn render_key_picker(model: &Model, p: &KeyPicker, f: &mut Frame, area: Rect) {
    draw_key_picker(
        model,
        p,
        ("Group by", "seen in", "flow", "all labels"),
        f,
        area,
    );
}

/// `f` on the Labels tab: the same picker, its counts in labels.
pub fn render_label_key_picker(model: &Model, p: &KeyPicker, f: &mut Frame, area: Rect) {
    draw_key_picker(model, p, ("Label keys", "", "label", "all keys"), f, area);
}

/// `(title, count prefix, count noun, all row)` name what the picker picks.
fn draw_key_picker(
    model: &Model,
    p: &KeyPicker,
    (title, prefix, noun, all_text): (&str, &str, &str, &str),
    f: &mut Frame,
    area: Rect,
) {
    let t = &model.theme;
    let rows = p.entries.len() as u16 + 1;
    let r = centered(area, 42, rows + 5);
    f.render_widget(Clear, r);
    let block = panel(title, "", t, true);
    let inner = block.inner(r);
    f.render_widget(block, r);
    let list_h = inner.height.saturating_sub(3) as usize;
    let first = p
        .cursor
        .min(p.all_row())
        .saturating_sub(list_h.saturating_sub(1));
    let row_style = |i: usize| {
        if i == p.cursor {
            t.selected_style()
        } else {
            t.base()
        }
    };
    let mut lines: Vec<Line> = p
        .entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let mark = if p.is_checked(&e.key) { "[x]" } else { "[ ]" };
            let mut spans = vec![Span::styled(format!("{mark} {:<15}", e.key), row_style(i))];
            if let Some(n) = e.seen {
                let plural = if n == 1 { "" } else { "s" };
                let prefix = if prefix.is_empty() {
                    String::new()
                } else {
                    format!("{prefix} ")
                };
                spans.push(Span::styled(
                    format!(" ({prefix}{n} {noun}{plural})"),
                    t.dim_style(),
                ));
            }
            Line::from(spans)
        })
        .collect();
    let all = if p.all { "[x]" } else { "[ ]" };
    lines.push(Line::from(Span::styled(
        format!("{all} {all_text}"),
        row_style(p.all_row()),
    )));
    let mut out: Vec<Line> = lines.into_iter().skip(first).take(list_h).collect();
    out.push(Line::from(""));
    let text = if p.all {
        all_text.to_string()
    } else {
        p.text.clone()
    };
    let cursor = if p.cursor == p.text_row() { "▏" } else { "" };
    out.push(Line::from(vec![
        Span::styled("keys: ", t.header_style()),
        Span::styled(format!("{text}{cursor}"), row_style(p.text_row())),
    ]));
    out.push(Line::from(Span::styled(
        "Space toggle · Enter apply · Esc close",
        t.dim_style(),
    )));
    f.render_widget(Paragraph::new(out).style(t.base()), inner);
}

pub fn render_filter_editor(model: &Model, e: &FilterEditor, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let strip = e.completion_strip();
    // While completing, the candidates take a row under the active field.
    let r = centered(area, 70, 12 + u16::from(strip.is_some()));
    f.render_widget(Clear, r);
    let block = panel("Edit filter", "Tab next · Enter run · Esc cancel", t, true);
    let mut lines: Vec<Line> = Vec::new();
    for (i, name) in FIELD_NAMES.iter().enumerate() {
        let active = i == e.active;
        let cursor = if active { "▏" } else { "" };
        lines.push(Line::from(vec![
            Span::styled(if active { "▸ " } else { "  " }, Style::new().fg(t.accent)),
            Span::styled(
                format!("{name:<w$}", w = FILTER_LABEL_WIDTH),
                t.header_style(),
            ),
            Span::styled(
                format!("{}{cursor}", e.values[i]),
                if active {
                    Style::new().fg(t.fg).bg(t.selected_bg)
                } else {
                    t.base()
                },
            ),
        ]));
        if let Some(strip) = strip.as_ref().filter(|_| active) {
            // Borders (2), the focus marker (2) and the label column.
            let width = (r.width as usize).saturating_sub(2 + 2 + FILTER_LABEL_WIDTH);
            lines.push(completion_line(strip, 2 + FILTER_LABEL_WIDTH, width, t));
        }
    }
    lines.push(Line::from(""));
    match &e.error {
        Some(err) => lines.push(Line::from(Span::styled(
            err.clone(),
            Style::new().fg(t.bad),
        ))),
        None => {
            let preview = e.to_filter().map(|f| f.cost_hint()).unwrap_or_default();
            lines.push(Line::from(Span::styled(preview, t.dim_style())));
        }
    }
    f.render_widget(Paragraph::new(lines).block(block).style(t.base()), r);
}

/// Label column width inside the filter editor.
const FILTER_LABEL_WIDTH: usize = 12;

/// Label column width inside the form (the `*` of a required field included).
const FORM_LABEL_WIDTH: usize = 14;

pub fn render_form(model: &Model, form: &Form, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    // Two rows per field (value, hint), the error row and the borders.
    let r = centered(area, 70, (form.fields.len() as u16) * 2 + 3);
    f.render_widget(Clear, r);
    let offer = form.ruleset_offer_armed();
    // A long title (a rule in a ruleset) would be cut by the full hint.
    let fits =
        |meta: &str| form.title.chars().count() + meta.chars().count() + 6 <= r.width as usize - 2;
    let meta = match offer {
        Some(_) if fits("Ctrl-R new ruleset · Enter review · Esc cancel") => {
            "Ctrl-R new ruleset · Enter review · Esc cancel"
        }
        Some(_) => "Ctrl-R new ruleset · Esc cancel",
        None if fits("Tab next · Enter review · Esc cancel") => {
            "Tab next · Enter review · Esc cancel"
        }
        None => "Enter review · Esc cancel",
    };
    let block = panel(&form.title, meta, t, true);
    // Borders (2), the focus marker (2), the label column and the cursor (1).
    let width = (r.width as usize).saturating_sub(2 + 2 + FORM_LABEL_WIDTH + 1);
    let mut lines: Vec<Line> = Vec::new();
    for (i, field) in form.fields.iter().enumerate() {
        let on = i == form.active;
        let label = if field.required {
            format!("{}*", field.label)
        } else {
            field.label.to_string()
        };
        let shown = plain(&visible_tail(&field.value, width));
        lines.push(Line::from(vec![
            Span::styled(if on { "▸ " } else { "  " }, Style::new().fg(t.accent)),
            Span::styled(
                format!("{label:<w$}", w = FORM_LABEL_WIDTH),
                t.header_style(),
            ),
            Span::styled(
                format!("{shown}{}", if on { "▏" } else { "" }),
                if on {
                    Style::new().fg(t.fg).bg(t.selected_bg)
                } else {
                    t.base()
                },
            ),
        ]));
        // While completing, the candidates take the hint's row.
        if let Some(strip) = form.completion_strip().filter(|_| on) {
            lines.push(completion_line(&strip, 2 + FORM_LABEL_WIDTH, width + 1, t));
            continue;
        }
        // While Ctrl-R is armed the Ruleset hint says what it creates.
        let hint = match offer {
            Some(o) if field.label == "Ruleset" => {
                format!("Ctrl-R: create it, scoped {}", o.scope).into()
            }
            _ => field.hint.clone(),
        };
        lines.push(Line::from(Span::styled(
            format!(
                "{:w$}{}",
                "",
                clip(&hint, width + 1),
                w = 2 + FORM_LABEL_WIDTH
            ),
            t.dim_style(),
        )));
    }
    lines.push(match &form.error {
        Some(e) => Line::from(Span::styled(plain(e), Style::new().fg(t.bad))),
        None => Line::from(Span::styled(
            match (&form.alt, form.expanded) {
                (None, _) => "* required",
                (Some(_), false) => "* required · Ctrl-A all labels",
                (Some(_), true) => "* required · Ctrl-A model view",
            },
            t.dim_style(),
        )),
    });
    f.render_widget(
        Paragraph::new(lines)
            .block(block)
            .style(t.base())
            .wrap(Wrap { trim: false }),
        r,
    );
}

/// The candidate strip under the field being completed: the shown candidate
/// in the accent colour, the others and `+N more` dim, two blanks apart,
/// `indent` cells in (under the value), cut to `width`.
fn completion_line<'a>(
    strip: &[(String, bool)],
    indent: usize,
    width: usize,
    t: &Theme,
) -> Line<'a> {
    let mut spans = vec![Span::raw(" ".repeat(indent))];
    let mut used = 0;
    for (i, (text, current)) in strip.iter().enumerate() {
        let gap = if i == 0 { 0 } else { 2 };
        let room = width.saturating_sub(used + gap);
        if room == 0 {
            break;
        }
        let shown = clip(&plain(text), room);
        used += gap + shown.chars().count();
        if gap > 0 {
            spans.push(Span::raw("  "));
        }
        spans.push(if *current {
            Span::styled(
                shown,
                Style::new().fg(t.accent).add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled(shown, t.dim_style())
        });
    }
    Line::from(spans)
}

/// Field names of the events editor, in the order of `Overlay::EventsEditor::values`.
const EVENTS_FIELDS: [&str; 3] = ["severity", "type", "window"];

pub fn render_events_editor(
    model: &Model,
    values: &[String; 3],
    active: usize,
    error: &Option<String>,
    f: &mut Frame,
    area: Rect,
) {
    let t = &model.theme;
    let r = centered(area, 70, 9);
    f.render_widget(Clear, r);
    let block = panel(
        "Edit events",
        "Tab next · Enter apply · Esc cancel",
        t,
        true,
    );
    let mut lines: Vec<Line> = EVENTS_FIELDS
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let on = i == active;
            let cursor = if on { "▏" } else { "" };
            Line::from(vec![
                Span::styled(if on { "▸ " } else { "  " }, Style::new().fg(t.accent)),
                Span::styled(format!("{name:<12}"), t.header_style()),
                Span::styled(
                    format!("{}{cursor}", values[i]),
                    if on {
                        Style::new().fg(t.fg).bg(t.selected_bg)
                    } else {
                        t.base()
                    },
                ),
            ])
        })
        .collect();
    lines.push(Line::from(""));
    lines.push(match error {
        Some(err) => Line::from(Span::styled(err.clone(), Style::new().fg(t.bad))),
        None => Line::from(Span::styled(
            "severity empty = any · type matches locally · window e.g. 24h",
            t.dim_style(),
        )),
    });
    f.render_widget(Paragraph::new(lines).block(block).style(t.base()), r);
}

/// Most lines of JSON shown for one event; the rest is cut. A smaller
/// terminal shows fewer: the cut follows the box height.
const EVENT_DETAIL_LINES: usize = 30;

pub fn render_event_detail(
    model: &Model,
    event: &pce::models::Event,
    scroll: usize,
    f: &mut Frame,
    area: Rect,
) {
    let t = &model.theme;
    let r = centered(area, 100, EVENT_DETAIL_LINES as u16 + 2);
    f.render_widget(Clear, r);
    let shown = (r.height.saturating_sub(2) as usize).min(EVENT_DETAIL_LINES);
    let json = serde_json::to_string_pretty(event).unwrap_or_default();
    let total = json.lines().count();
    // The model only stops at the last line; the view keeps the box full.
    let first = scroll.min(total.saturating_sub(shown));
    let lines: Vec<Line> = json
        .lines()
        .skip(first)
        .take(shown)
        .map(|l| Line::from(crate::tui::ui::screens::workloads::plain(l)))
        .collect();
    let meta = if total > shown {
        let last = (first + shown).min(total);
        format!(
            "lines {}-{last} of {total} · j/k scroll · Esc closes",
            first + 1
        )
    } else {
        "Esc closes".to_string()
    };
    let title = event.event_type.as_deref().unwrap_or("event");
    f.render_widget(
        Paragraph::new(lines)
            .block(panel(title, &meta, t, true))
            .style(t.base()),
        r,
    );
}

pub fn render_confirm(model: &Model, reason: &str, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let r = centered(area, 64, 5);
    f.render_widget(Clear, r);
    let block = panel("Confirm", "y / n", t, true);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            reason.to_string(),
            Style::new().fg(t.warn),
        )))
        .block(block)
        .style(t.base())
        .wrap(Wrap { trim: true }),
        r,
    );
}

/// Widest the confirm box gets; borders take two of these columns.
const CONFIRM_WIDTH: u16 = 76;
/// Rows around the body and the summary: borders, `METHOD path`, a blank
/// line.
const CONFIRM_CHROME: u16 = 4;

/// Rows between the summary and the body: the impact line of a rule plan,
/// the typed word of a revert.
fn confirm_extra_rows(plan: &Plan, kind: ObjectKind) -> u16 {
    crate::tui::impact::confirm_rows(plan) + u16::from(kind == ObjectKind::Revert)
}

/// The confirm box as drawn: pinned rows on top, then a scrolled window of
/// `content` (`rows` fit); the first `objects` content lines are the
/// request's objects.
struct ConfirmLayout {
    rect: Rect,
    summary: Vec<String>,
    content: Vec<String>,
    objects: usize,
    rows: usize,
}

/// The confirm box in `area`, every line wrapped to its inner width (the box
/// is the dry run: nothing may be clipped). The summary, the extra rows
/// ([`confirm_extra_rows`]) and `METHOD path` stay pinned; the request's
/// objects (a revert's, one per line, under the summary's head) and the
/// body scroll. Shared by the renderer and the scroll keys so they cannot
/// drift.
fn confirm_write_layout(req: &WriteRequest, area: Rect, extra: u16) -> ConfirmLayout {
    let width = usize::from(CONFIRM_WIDTH.min(area.width).saturating_sub(2));
    let wrap = |l: &str| wrap_chars(&plain(l), width);
    let summary = if req.objects.is_empty() {
        wrap(&req.summary)
    } else {
        // `revert 2 changes: <objects>`: the head, the objects below it.
        let head = req
            .summary
            .split_once(": ")
            .map_or(req.summary.as_str(), |(h, _)| h);
        wrap(&format!("{head}:"))
    };
    let mut content: Vec<String> = req
        .objects
        .iter()
        .flat_map(|o| wrap(&format!("  {o}")))
        .collect();
    let objects = content.len();
    if objects > 0 {
        content.push(String::new());
    }
    content.extend(
        crate::tui::edit::request_body_lines(req)
            .iter()
            .flat_map(|l| wrap(l)),
    );
    let chrome = CONFIRM_CHROME
        .saturating_add(u16::try_from(summary.len()).unwrap_or(u16::MAX))
        .saturating_add(extra);
    let height = u16::try_from(content.len())
        .unwrap_or(u16::MAX)
        .saturating_add(chrome);
    let rect = centered(area, CONFIRM_WIDTH, height);
    let rows = usize::from(rect.height.saturating_sub(chrome));
    ConfirmLayout {
        rect,
        summary,
        content,
        objects,
        rows,
    }
}

/// The first scrolled line shown when scrolled to the end.
pub fn confirm_write_last_scroll(plan: &Plan, kind: ObjectKind, area: Rect) -> usize {
    let Some(req) = plan.request() else {
        return 0;
    };
    let l = confirm_write_layout(req, area, confirm_extra_rows(plan, kind));
    l.content.len().saturating_sub(l.rows)
}

/// `line` in pieces of at most `width` characters; an empty line stays one.
fn wrap_chars(line: &str, width: usize) -> Vec<String> {
    let chars: Vec<char> = line.chars().collect();
    if chars.is_empty() || width == 0 {
        return vec![line.to_string()];
    }
    chars.chunks(width).map(|c| c.iter().collect()).collect()
}

pub fn render_confirm_write(
    model: &Model,
    plan: &Plan,
    kind: ObjectKind,
    scroll: u16,
    f: &mut Frame,
    area: Rect,
) {
    let t = &model.theme;
    let Some(req) = plan.request() else {
        return;
    };
    let revert = kind == ObjectKind::Revert;
    let extra = confirm_extra_rows(plan, kind);
    let l = confirm_write_layout(req, area, extra);
    let r = l.rect;
    f.render_widget(Clear, r);
    let first = (scroll as usize).min(l.content.len().saturating_sub(l.rows));
    let meta = match (revert, l.content.len() > l.rows) {
        (true, true) => "↑/↓ scroll · type revert, Enter · Esc cancel",
        (true, false) => "type revert, Enter · Esc cancel",
        (false, true) => "j/k scroll · y send · n cancel",
        (false, false) => "y send · n cancel",
    };
    let warn = Style::new().fg(t.warn);
    let mut lines: Vec<Line> = l
        .summary
        .into_iter()
        .map(|s| Line::from(Span::styled(s, warn)))
        .collect();
    if revert {
        lines.push(Line::from(vec![
            Span::styled(format!("type {REVERT_WORD} to confirm: "), t.header_style()),
            Span::styled(
                format!("{}▏", plain(&model.policy.revert_typed)),
                Style::new().fg(t.accent),
            ),
        ]));
    } else if extra > 0 {
        // Stored when the confirm opened or the workloads loaded.
        let impact = model.policy.confirm_impact.clone().unwrap_or_default();
        lines.push(Line::from(Span::styled(impact, Style::new().fg(t.accent))));
    }
    lines.push(Line::from(Span::styled(
        format!("{} {}", req.method.as_str(), plain(&req.path)),
        t.header_style(),
    )));
    lines.push(Line::from(""));
    let objects = l.objects;
    lines.extend(
        l.content
            .into_iter()
            .enumerate()
            .skip(first)
            .take(l.rows)
            .map(|(i, s)| Line::from(Span::styled(s, if i < objects { warn } else { t.base() }))),
    );
    f.render_widget(
        Paragraph::new(lines)
            .block(panel(
                if revert {
                    "Revert these changes?"
                } else {
                    "Send this request?"
                },
                meta,
                t,
                true,
            ))
            .style(t.base()),
        r,
    );
}

/// Longest message drawn; the panel holds five wrapped lines.
const ERROR_MAX_CHARS: usize = 300;

pub fn render_error(model: &Model, text: &str, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let r = centered(area, 70, 7);
    f.render_widget(Clear, r);
    let shown: String = if text.chars().count() > ERROR_MAX_CHARS {
        let cut: String = text.chars().take(ERROR_MAX_CHARS - 1).collect();
        format!("{cut}…")
    } else {
        text.to_string()
    };
    let bad = Style::new().fg(t.bad);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(bad)
        .title(Line::from(Span::styled(
            " Error ",
            bad.add_modifier(ratatui::style::Modifier::BOLD),
        )))
        .title_top(Line::from(Span::styled(" Esc closes ", t.dim_style())).right_aligned());
    f.render_widget(
        Paragraph::new(shown)
            .block(block)
            .style(t.base())
            .wrap(Wrap { trim: true }),
        r,
    );
}

fn labelled<'a>(model: &Model, label: &str, value: String) -> Line<'a> {
    let t = &model.theme;
    Line::from(vec![
        Span::styled(format!("{label:<12}"), t.header_style()),
        Span::styled(value, t.base()),
    ])
}

pub fn render_rule_detail(model: &Model, rule: &RuleView, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let r = centered(area, 76, 16);
    f.render_widget(Clear, r);
    let names = |v: &[pce::ResolvedActor]| {
        v.iter()
            .map(|a| a.name.clone())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let services = rule
        .services
        .iter()
        .map(|s| s.name.clone())
        .collect::<Vec<_>>()
        .join(", ");
    let flags = format!(
        "{} · {}",
        if rule.enabled { "enabled" } else { "disabled" },
        if rule.unscoped_consumers {
            "unscoped consumers"
        } else {
            "scoped consumers"
        }
    );
    let mut lines = vec![
        labelled(
            model,
            "description",
            rule.description.clone().unwrap_or_else(|| "(none)".into()),
        ),
        labelled(model, "consumers", names(&rule.consumers)),
        labelled(model, "providers", names(&rule.providers)),
        labelled(model, "services", services),
        labelled(model, "flags", flags),
        labelled(
            model,
            "pending",
            rule.update_type.clone().unwrap_or_else(|| "none".into()),
        ),
        labelled(model, "href", rule.href.clone()),
    ];
    lines.extend(
        crate::tui::impact::rule_detail_rows(model, &rule.href)
            .into_iter()
            .map(|(label, text)| labelled(model, label, text)),
    );
    let block = panel(rule_kind_name(rule.kind), "Esc closes", t, true);
    f.render_widget(
        Paragraph::new(lines)
            .block(block)
            .style(t.base())
            .wrap(Wrap { trim: true }),
        r,
    );
}

/// The pending overlay's key line, the last row inside its box.
const PENDING_KEYS: &str = "j/k select · Space mark · Enter diff · P provision · R revert · Esc";

pub fn render_pending(model: &Model, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let r = centered(area, 70, 16);
    f.render_widget(Clear, r);
    let pending = model.policy.pending.as_deref().unwrap_or(&[]);
    let marked = &model.policy.pending_marked;
    // Rows inside the borders, less the key line.
    let rows = (r.height.saturating_sub(2) as usize).saturating_sub(1);
    let sel = model
        .policy
        .pending_selected
        .min(pending.len().saturating_sub(1));
    // Keep the selection in view.
    let first = sel.saturating_sub(rows.saturating_sub(1));
    let mut lines: Vec<Line> = if pending.is_empty() {
        vec![Line::from(Span::styled("none", t.dim_style()))]
    } else {
        pending
            .iter()
            .enumerate()
            .skip(first)
            .take(rows)
            .map(|(i, c)| {
                Line::from(vec![
                    Span::styled(
                        if i == sel { "▸ " } else { "  " },
                        Style::new().fg(t.accent),
                    ),
                    Span::styled(
                        if marked.contains(&c.href) {
                            "[x] "
                        } else {
                            "[ ] "
                        },
                        Style::new().fg(t.accent),
                    ),
                    Span::styled(format!("{:<16}", c.object_type), t.header_style()),
                    Span::styled(format!("{:<8}", c.change_type), Style::new().fg(t.warn)),
                    Span::styled(c.name.clone().unwrap_or_else(|| c.href.clone()), t.base()),
                ])
            })
            .collect()
    };
    lines.resize(rows, Line::from(""));
    lines.push(Line::from(Span::styled(PENDING_KEYS, t.dim_style())));
    let meta = match (pending.len(), marked.len()) {
        (0, _) => "0".to_string(),
        (n, 0) => format!("{}/{n}", sel + 1),
        (n, m) => format!("{}/{n} · {m} marked", sel + 1),
    };
    f.render_widget(
        Paragraph::new(lines)
            .block(panel("Pending changes", &meta, t, true))
            .style(t.base()),
        r,
    );
}

pub fn render_diff(model: &Model, diff: &ObjectDiff, scroll: u16, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let r = centered(area, 76, (diff.lines.len() as u16).saturating_add(2).max(5));
    f.render_widget(Clear, r);
    let rows = r.height.saturating_sub(2) as usize;
    let first = (scroll as usize).min(diff.lines.len().saturating_sub(rows));
    let lines: Vec<Line> = diff
        .lines
        .iter()
        .skip(first)
        .take(rows)
        .map(|l| {
            let (sign, style) = match l.sign {
                Sign::Added => ("+ ", Style::new().fg(t.ok)),
                Sign::Removed => ("- ", Style::new().fg(t.bad)),
                Sign::Same => ("  ", t.dim_style()),
            };
            Line::from(Span::styled(format!("{sign}{}", plain(&l.text)), style))
        })
        .collect();
    let href = pce::models::Ref::new(&diff.href);
    let kind = match href.object_type() {
        "ip_lists" => "IP list",
        "services" => "service",
        "rule_sets" => "ruleset",
        other => other,
    };
    let title = format!("{kind} {} · {}", plain(&diff.name), diff.change.as_str());
    let meta = if diff.lines.len() > rows {
        "j/k scroll · Esc back · q close"
    } else {
        "Esc back · q close"
    };
    f.render_widget(
        Paragraph::new(lines)
            .block(panel(&title, meta, t, true))
            .style(t.base()),
        r,
    );
}

/// Key hints for the status line. With writes on, Objects, Workloads, Policy and Traffic lead
/// with the edit keys and drop their least used hints so the status text
/// keeps room at 80 columns.
pub fn screen_hint(screen: Screen, writes: bool) -> &'static str {
    match (screen, writes) {
        (Screen::Objects, true) => "n:new e:edit x:del  Tab type  Enter detail",
        (Screen::Workloads, true) => "n:new e:edit x:del  t/T traffic  Enter detail",
        (Screen::Policy, true) => "n/N:new e:edit x:del  d scope  p pending",
        (Screen::Traffic, true) => "w:rule  f filter  r run  v view  g group",
        (Screen::Traffic, _) => "f filter  r run  a auto  v view  g group  / search",
        (Screen::Policy, _) => "d scope  p pending  Enter detail  Tab pane  i impact",
        (Screen::Objects, false) => "Tab type  Enter detail  r refresh  a auto  / search",
        (Screen::Workloads, false) => "t/T traffic  h/H health  Enter detail  r refresh",
        (Screen::Events, _) => "f filter  r refresh  a auto  S severity  Enter detail",
    }
}

/// More key hints for a screen (and the Traffic view), shown after
/// `screen_hint` only when the status line still has room for the status
/// text.
pub fn screen_hint_more(screen: Screen, view: TrafficView) -> &'static str {
    match (screen, view) {
        (Screen::Traffic, TrafficView::Aggregate) => {
            "+/- window  ]/[ cap  x mode  o service  d decision"
        }
        (Screen::Traffic, _) => "+/- window  ]/[ cap",
        _ => "",
    }
}
