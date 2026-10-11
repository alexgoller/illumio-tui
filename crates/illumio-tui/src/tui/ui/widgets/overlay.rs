//! Modal overlays drawn on top of the current screen: help and the `:` prompt.
use crate::tui::diff::{ObjectDiff, Sign};
use crate::tui::filter::{FIELD_NAMES, FilterEditor};
use crate::tui::form::{Form, visible_tail};
use crate::tui::model::{Model, Screen, rule_kind_name};
use crate::tui::ui::layout::centered;
use crate::tui::ui::screens::workloads::plain;
use crate::tui::ui::widgets::panel::panel;
use pce::RuleView;
use pce::write::{Plan, WriteRequest};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Wrap};

pub const HELP_LINES: &[(&str, &str)] = &[
    (
        "1 2 3 4 5",
        "Traffic · Policy · Objects · Workloads · Events",
    ),
    (":", "command prompt: :traffic :policy :objects :workloads"),
    ("", ":events  :filter <expr>  :window <dur>"),
    ("", ":profile <name>  :draft  :active  :help  :q"),
    ("?", "this help"),
    ("q / Ctrl-C", "quit"),
    ("↑ ↓ PgUp PgDn Home End", "move in the current table"),
    ("s", "cycle sort column and direction"),
    ("/", "search in the current table (Enter keep, Esc clear)"),
    ("r", "refresh the current screen (bypasses the cache)"),
    ("a", "toggle auto-refresh for the current screen"),
    ("Traffic  v", "cycle Flows · App-to-app · Blocked"),
    (
        "Traffic  f",
        "edit filter: window src dst service decisions",
    ),
    ("Traffic  g", "change the aggregate label keys"),
    (
        "Traffic  Enter",
        "detail (flows) · drill into a pair (aggregate)",
    ),
    ("Policy   d", "toggle draft / active"),
    ("Policy   p", "pending changes"),
    (
        "Policy   Tab / Enter",
        "pane · rule detail · on a ruleset: load deny rules",
    ),
    ("Objects  Tab / Enter", "next object type · detail"),
    (
        "Workloads t/T h H Enter",
        "Traffic as src/dst · unhealthy · health sort · detail",
    ),
    (
        "Events   f / Enter / S",
        "filter · event as JSON · sort by severity",
    ),
    (
        "n / e / x",
        "new / edit / delete (Objects, Workloads; needs writes)",
    ),
    (
        "Pending  j/k Enter",
        "select · diff of a pending IP list or service",
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
                Span::styled(format!("{k:<24}"), t.header_style()),
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

pub fn render_group_prompt(model: &Model, input: &str, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let r = Rect::new(
        area.x,
        area.y + area.height.saturating_sub(1),
        area.width,
        1,
    );
    f.render_widget(Clear, r);
    let line = Line::from(vec![
        Span::styled("group keys: ", t.header_style()),
        Span::styled(input.to_string(), t.base()),
        Span::styled("▏", t.dim_style()),
    ]);
    f.render_widget(Paragraph::new(line).style(t.base()), r);
}

pub fn render_filter_editor(model: &Model, e: &FilterEditor, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let r = centered(area, 70, 12);
    f.render_widget(Clear, r);
    let block = panel("Edit filter", "Tab next · Enter run · Esc cancel", t, true);
    let mut lines: Vec<Line> = FIELD_NAMES
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let active = i == e.active;
            let cursor = if active { "▏" } else { "" };
            Line::from(vec![
                Span::styled(if active { "▸ " } else { "  " }, Style::new().fg(t.accent)),
                Span::styled(format!("{name:<12}"), t.header_style()),
                Span::styled(
                    format!("{}{cursor}", e.values[i]),
                    if active {
                        Style::new().fg(t.fg).bg(t.selected_bg)
                    } else {
                        t.base()
                    },
                ),
            ])
        })
        .collect();
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

/// Label column width inside the form (the `*` of a required field included).
const FORM_LABEL_WIDTH: usize = 14;

pub fn render_form(model: &Model, form: &Form, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    // Two rows per field (value, hint), the error row and the borders.
    let r = centered(area, 70, (form.fields.len() as u16) * 2 + 3);
    f.render_widget(Clear, r);
    let block = panel(&form.title, "Tab next · Enter review · Esc cancel", t, true);
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
        lines.push(Line::from(Span::styled(
            format!("{:w$}{}", "", field.hint, w = 2 + FORM_LABEL_WIDTH),
            t.dim_style(),
        )));
    }
    lines.push(match &form.error {
        Some(e) => Line::from(Span::styled(plain(e), Style::new().fg(t.bad))),
        None => Line::from(Span::styled("* required", t.dim_style())),
    });
    f.render_widget(
        Paragraph::new(lines)
            .block(block)
            .style(t.base())
            .wrap(Wrap { trim: false }),
        r,
    );
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
/// Rows around the body: borders, the summary, `METHOD path`, a blank line.
const CONFIRM_CHROME: u16 = 5;

/// The confirm box in `area`, the request body wrapped to its inner width
/// (the box is the dry run: nothing may be clipped), and how many body rows
/// fit. Shared by the renderer and the scroll keys so they cannot drift.
fn confirm_write_layout(req: &WriteRequest, area: Rect) -> (Rect, Vec<String>, usize) {
    let width = usize::from(CONFIRM_WIDTH.min(area.width).saturating_sub(2));
    let body: Vec<String> = crate::tui::edit::request_body_lines(req)
        .iter()
        .flat_map(|l| wrap_chars(&plain(l), width))
        .collect();
    let height = u16::try_from(body.len())
        .unwrap_or(u16::MAX)
        .saturating_add(CONFIRM_CHROME);
    let r = centered(area, CONFIRM_WIDTH, height);
    let rows = usize::from(r.height.saturating_sub(CONFIRM_CHROME));
    (r, body, rows)
}

/// The first wrapped body line shown when scrolled to the end.
pub fn confirm_write_last_scroll(req: &WriteRequest, area: Rect) -> usize {
    let (_, body, rows) = confirm_write_layout(req, area);
    body.len().saturating_sub(rows)
}

/// `line` in pieces of at most `width` characters; an empty line stays one.
fn wrap_chars(line: &str, width: usize) -> Vec<String> {
    let chars: Vec<char> = line.chars().collect();
    if chars.is_empty() || width == 0 {
        return vec![line.to_string()];
    }
    chars.chunks(width).map(|c| c.iter().collect()).collect()
}

pub fn render_confirm_write(model: &Model, plan: &Plan, scroll: u16, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let Some(req) = plan.request() else {
        return;
    };
    let (r, body, rows) = confirm_write_layout(req, area);
    f.render_widget(Clear, r);
    let first = (scroll as usize).min(body.len().saturating_sub(rows));
    let meta = if body.len() > rows {
        "j/k scroll · y send · n cancel"
    } else {
        "y send · n cancel"
    };
    let mut lines = vec![
        Line::from(Span::styled(plain(&req.summary), Style::new().fg(t.warn))),
        Line::from(Span::styled(
            format!("{} {}", req.method.as_str(), plain(&req.path)),
            t.header_style(),
        )),
        Line::from(""),
    ];
    lines.extend(
        body.into_iter()
            .skip(first)
            .take(rows)
            .map(|l| Line::from(Span::styled(l, t.base()))),
    );
    f.render_widget(
        Paragraph::new(lines)
            .block(panel("Send this request?", meta, t, true))
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
    let r = centered(area, 76, 14);
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
    let lines = vec![
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
    let block = panel(rule_kind_name(rule.kind), "Esc closes", t, true);
    f.render_widget(
        Paragraph::new(lines)
            .block(block)
            .style(t.base())
            .wrap(Wrap { trim: true }),
        r,
    );
}

pub fn render_pending(model: &Model, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let r = centered(area, 70, 16);
    f.render_widget(Clear, r);
    let pending = model.policy.pending.as_deref().unwrap_or(&[]);
    let rows = r.height.saturating_sub(2) as usize;
    let sel = model
        .policy
        .pending_selected
        .min(pending.len().saturating_sub(1));
    // Keep the selection in view.
    let first = sel.saturating_sub(rows.saturating_sub(1));
    let lines: Vec<Line> = if pending.is_empty() {
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
                    Span::styled(format!("{:<16}", c.object_type), t.header_style()),
                    Span::styled(format!("{:<8}", c.change_type), Style::new().fg(t.warn)),
                    Span::styled(c.name.clone().unwrap_or_else(|| c.href.clone()), t.base()),
                ])
            })
            .collect()
    };
    let meta = if pending.is_empty() {
        "0 · Esc closes".to_string()
    } else {
        format!("{}/{} · Enter diff · Esc closes", sel + 1, pending.len())
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

/// Key hints for the status line. With writes on, Objects and Workloads lead
/// with the edit keys and drop their least used hints so the status text
/// keeps room at 80 columns.
pub fn screen_hint(screen: Screen, writes: bool) -> &'static str {
    match (screen, writes) {
        (Screen::Objects, true) => "n:new e:edit x:del  Tab type  Enter detail",
        (Screen::Workloads, true) => "n:new e:edit x:del  t/T traffic  Enter detail",
        (Screen::Traffic, _) => "f filter  r run  a auto  v view  s sort  / search",
        (Screen::Policy, _) => "d scope  p pending  Enter detail  Tab pane  a auto",
        (Screen::Objects, false) => "Tab type  Enter detail  r refresh  a auto  / search",
        (Screen::Workloads, false) => "t/T traffic  h/H health  Enter detail  r refresh",
        (Screen::Events, _) => "f filter  r refresh  a auto  S severity  Enter detail",
    }
}
