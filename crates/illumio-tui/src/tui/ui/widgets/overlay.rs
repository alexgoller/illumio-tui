//! Modal overlays drawn on top of the current screen: help and the `:` prompt.
use crate::tui::app_picker::AppPicker;
use crate::tui::diff::{ObjectDiff, Sign};
use crate::tui::edit::EditIntent;
use crate::tui::event_diff::{DetailLine, detail_lines};
use crate::tui::filter::{FIELD_NAMES, FilterEditor};
use crate::tui::form::{Form, clip, visible_tail, wrap_rows};
use crate::tui::help::{
    HelpContext, HelpPage, HelpRow, fit_names, key_width, other_topics, page_rows,
};
use crate::tui::model::{Model, Screen, TrafficView, rule_kind_name};
use crate::tui::picker::KeyPicker;
use crate::tui::ui::layout::centered;
use crate::tui::ui::screens::workloads::plain;
use crate::tui::ui::theme::Theme;
use crate::tui::ui::widgets::panel::panel;
use crate::tui::update::REVERT_WORD;
use pce::RuleView;
use pce::write::{Batch, ObjectKind, Plan, WriteRequest};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Wrap};

/// The help box for `area`: its page, rectangle, key column and the rows
/// left for the page once the footer line is in.
struct HelpLayout {
    ctx: HelpContext,
    rows: Vec<HelpRow>,
    rect: Rect,
    key_width: usize,
    footer: Option<String>,
    body: usize,
}

fn help_layout(model: &Model, area: Rect) -> HelpLayout {
    let ctx = HelpContext::of(model);
    let page = model.help_page;
    let rows = page_rows(page, ctx);
    let key_width = key_width(&rows);
    let widest = rows
        .iter()
        .map(|r| match r {
            HelpRow::Key(_, d) => key_width + d.chars().count(),
            HelpRow::Heading(h) => h.chars().count(),
            HelpRow::Blank => 0,
        })
        .max()
        .unwrap_or(0);
    let has_footer = page != HelpPage::All;
    let w = (widest + 3).clamp(60, 96) as u16;
    let h = (rows.len() + usize::from(has_footer) + 2).min(u16::MAX as usize) as u16;
    let rect = centered(area, w, h);
    let inner = rect.height.saturating_sub(2) as usize;
    let width = rect.width.saturating_sub(2) as usize;
    // Only whole names: what does not fit is counted (`+N`) or left out.
    let footer = match page {
        _ if inner < 2 => None,
        HelpPage::Context => {
            let names: Vec<&str> = other_topics(page, ctx).iter().map(|t| t.title()).collect();
            fit_names("more: ", &names, width)
        }
        HelpPage::Group(t) => [
            format!("next: {} · prev: {}", t.next().title(), t.prev().title()),
            format!("next: {}", t.next().title()),
        ]
        .into_iter()
        .find(|f| f.chars().count() <= width),
        HelpPage::All => None,
    };
    let body = inner - usize::from(footer.is_some());
    HelpLayout {
        ctx,
        rows,
        rect,
        key_width,
        footer,
        body,
    }
}

/// The first help line shown when scrolled to the end of the current page,
/// for a terminal of `size` (columns, rows).
pub fn help_last_scroll(model: &Model, size: (u16, u16)) -> usize {
    let l = help_layout(model, Rect::new(0, 0, size.0, size.1));
    l.rows.len().saturating_sub(l.body)
}

/// The page lines the help box shows at once (`PgDn` / `PgUp` step).
pub fn help_body_rows(model: &Model, size: (u16, u16)) -> usize {
    help_layout(model, Rect::new(0, 0, size.0, size.1)).body
}

/// The `?` help: the page's groups under their headings, scrolled; the
/// title names the context or group, the footer the other groups, the
/// bottom border the help's keys. Lines are cut at the box, never wrapped.
pub fn render_help(model: &Model, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let l = help_layout(model, area);
    let r = l.rect;
    f.render_widget(Clear, r);
    let total = l.rows.len();
    let first = model.help_scroll.min(total.saturating_sub(l.body));
    let meta = if total > l.body {
        format!("{}-{} of {total}", first + 1, (first + l.body).min(total))
    } else {
        String::new()
    };
    let title = match model.help_page {
        HelpPage::Context => format!("Keys · {}", l.ctx.name()),
        HelpPage::Group(g) => format!("Keys · {}", g.title()),
        HelpPage::All => "Keys · all".to_string(),
    };
    let toggle = if model.help_page == HelpPage::All {
        "a back"
    } else {
        "a all"
    };
    let keys = format!(" Tab next · S-Tab prev · {toggle} · j/k scroll · Esc ");
    let block = panel(&title, &meta, t, true)
        .title_bottom(Line::from(Span::styled(keys, t.dim_style())).right_aligned());
    let inner_width = r.width.saturating_sub(2) as usize;
    let kw = l.key_width;
    let mut lines: Vec<Line> = l
        .rows
        .iter()
        .skip(first)
        .take(l.body)
        .map(|row| match row {
            HelpRow::Heading(h) => Line::from(Span::styled(
                h.to_uppercase(),
                t.title_style().add_modifier(Modifier::BOLD),
            )),
            HelpRow::Key(k, d) => Line::from(vec![
                Span::styled(
                    format!("{:<kw$}", clip(k, kw.saturating_sub(1))),
                    t.header_style(),
                ),
                Span::styled(*d, t.base()),
            ]),
            HelpRow::Blank => Line::default(),
        })
        .collect();
    if let Some(footer) = &l.footer {
        // The footer stays on the box's last row.
        lines.resize(l.body, Line::default());
        lines.push(Line::from(Span::styled(
            clip(footer, inner_width),
            t.dim_style(),
        )));
    }
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

/// `F` without an app, `:focus` alone: the typed text and the apps it keeps.
pub fn render_app_picker(model: &Model, p: &AppPicker, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let matches = p.matches();
    let shown = matches.len().clamp(1, 10) as u16;
    let r = centered(area, area.width.min(60), shown + 4);
    f.render_widget(Clear, r);
    let block = panel("Focus on an app", "Enter · Tab completes · Esc", t, true);
    let mut lines = vec![
        Line::from(vec![
            Span::styled("app: ", t.header_style()),
            Span::styled(format!("{}▏", p.input), t.base()),
        ]),
        Line::from(""),
    ];
    if matches.is_empty() {
        lines.push(Line::from(Span::styled(
            "no app of the loaded result matches",
            t.dim_style(),
        )));
    }
    let first = p.selected.saturating_sub(9);
    for (i, app) in matches.iter().enumerate().skip(first).take(10) {
        let style = if i == p.selected {
            t.base().add_modifier(Modifier::REVERSED)
        } else {
            t.base()
        };
        lines.push(Line::from(Span::styled(format!("  {app}"), style)));
    }
    f.render_widget(Paragraph::new(lines).block(block).style(t.base()), r);
}

/// `:filters`: the screen's saved filters, each with its summary.
pub fn render_filter_picker(
    model: &Model,
    p: &crate::tui::saved::FilterPicker,
    f: &mut Frame,
    area: Rect,
) {
    let t = &model.theme;
    let matches = p.matches();
    let shown = matches.len().clamp(1, 10) as u16;
    let r = centered(area, area.width.saturating_sub(4).min(100), shown + 4);
    f.render_widget(Clear, r);
    let title = format!("Saved {} filters", p.screen);
    let block = panel(&title, "Enter apply · * default · D delete · Esc", t, true);
    let mut lines = vec![
        Line::from(vec![
            Span::styled("filter: ", t.header_style()),
            Span::styled(format!("{}▏", p.input), t.base()),
        ]),
        Line::from(""),
    ];
    if matches.is_empty() {
        lines.push(Line::from(Span::styled(
            "no saved filter matches",
            t.dim_style(),
        )));
    }
    let width = matches
        .iter()
        .map(|e| e.name.chars().count())
        .max()
        .unwrap_or(0);
    let first = p.selected.saturating_sub(9);
    for (i, e) in matches.iter().enumerate().skip(first).take(10) {
        let style = if i == p.selected {
            t.base().add_modifier(Modifier::REVERSED)
        } else {
            t.base()
        };
        let mark = if p.default.as_deref() == Some(e.name.as_str()) {
            '*'
        } else {
            ' '
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{mark} {:<width$}", e.name), style),
            Span::styled(format!("  {}", e.summary), t.dim_style()),
        ]));
    }
    f.render_widget(Paragraph::new(lines).block(block).style(t.base()), r);
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
            let parsed = e.to_filter().ok();
            let preview = parsed.as_ref().map(|f| f.cost_hint()).unwrap_or_default();
            lines.push(Line::from(Span::styled(preview, t.dim_style())));
            // The editor's last row: how a `!` term applies.
            if let Some(note) = parsed.as_ref().and_then(|f| f.exclude_note()) {
                lines.push(Line::from(Span::styled(note, t.dim_style())));
            }
        }
    }
    f.render_widget(Paragraph::new(lines).block(block).style(t.base()), r);
}

/// `f` on Workloads: the label filter on one line, the completion strip
/// under it while completing, then the parse error, the unknown labels
/// (dim: they match nothing) or the syntax.
pub fn render_label_filter(
    model: &Model,
    e: &crate::tui::label_editor::LabelFilterEditor,
    f: &mut Frame,
    area: Rect,
) {
    let t = &model.theme;
    let strip = e.completion_strip();
    let r = centered(area, 72, 6 + u16::from(strip.is_some()));
    f.render_widget(Clear, r);
    let block = panel(
        "Label filter",
        "Ctrl-N complete · Enter apply · Esc cancel",
        t,
        true,
    );
    let inner = (r.width as usize).saturating_sub(2);
    // The end of a long filter stays in view, where the cursor is.
    let label = "labels ";
    let room = inner.saturating_sub(2 + label.len() + 1);
    let shown: String = {
        let chars: Vec<char> = e.value.chars().collect();
        let skip = chars.len().saturating_sub(room);
        chars[skip..].iter().collect()
    };
    let mut lines = vec![Line::from(vec![
        Span::styled("▸ ", Style::new().fg(t.accent)),
        Span::styled(label, t.header_style()),
        Span::styled(format!("{shown}▏"), Style::new().fg(t.fg).bg(t.selected_bg)),
    ])];
    if let Some(strip) = strip.as_ref() {
        let width = inner.saturating_sub(2 + label.len());
        lines.push(completion_line(strip, 2 + label.len(), width, t));
    }
    lines.push(Line::from(""));
    let note = match (&e.error, e.parse()) {
        (Some(err), _) => Line::from(Span::styled(plain(err), Style::new().fg(t.bad))),
        (None, Err(err)) => Line::from(Span::styled(plain(&err), t.dim_style())),
        (None, Ok(_)) => {
            let unknown = &e.unknown;
            if unknown.is_empty() {
                Line::from(Span::styled(
                    "comma: any of · space: all of · ! excludes · key=- no label",
                    t.dim_style(),
                ))
            } else {
                Line::from(Span::styled(
                    format!("no match: {}", unknown.join(", ")),
                    t.dim_style(),
                ))
            }
        }
    };
    lines.push(note);
    lines.push(Line::from(Span::styled(
        "local: no request · c on Workloads clears every filter",
        t.dim_style(),
    )));
    f.render_widget(Paragraph::new(lines).block(block).style(t.base()), r);
}

/// Label column width inside the filter editor.
const FILTER_LABEL_WIDTH: usize = 13;

/// Label column width inside the form (the `*` of a required field included).
const FORM_LABEL_WIDTH: usize = 14;

pub fn render_form(model: &Model, form: &Form, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let offer = form.ruleset_offer_armed();
    // While Ctrl-R is armed the Ruleset hint says what it creates.
    let hint_of = |field: &crate::tui::form::Field| match offer {
        Some(o) if field.label == "Ruleset" => format!("Ctrl-R: create it, scoped {}", o.scope),
        _ => field.hint.to_string(),
    };
    // Borders (2), the focus marker (2), the label column and the cursor (1).
    let width = (centered(area, 70, 1).width as usize).saturating_sub(2 + 2 + FORM_LABEL_WIDTH + 1);
    // A hint or the error longer than its row takes a second one.
    let hints: Vec<Vec<String>> = form
        .fields
        .iter()
        .map(|field| wrap_rows(&hint_of(field), width + 1, 2))
        .collect();
    let error = form
        .error
        .as_deref()
        .map(|e| wrap_rows(&plain(e), width + 1 + 2 + FORM_LABEL_WIDTH, 2));
    // An allow rule's Scope notes under Consumers, live: what the consumers
    // mean (`true`: warnings, in the warn colour).
    let scope_rows: Vec<(String, bool)> = model
        .rule_scope_notes(form)
        .map(|(meaning, warnings)| {
            let mut rows: Vec<(String, bool)> = wrap_rows(&plain(&meaning), width + 1, 2)
                .into_iter()
                .map(|r| (r, false))
                .collect();
            for w in warnings {
                rows.extend(
                    wrap_rows(&plain(&format!("⚠ {w}")), width + 1, 2)
                        .into_iter()
                        .map(|r| (r, true)),
                );
            }
            rows
        })
        .unwrap_or_default();
    // A value row and its hint rows per field, the scope notes, the error
    // rows and the borders.
    let rows = hints.iter().map(|h| 1 + h.len()).sum::<usize>()
        + scope_rows.len()
        + error.as_ref().map_or(1, Vec::len)
        + 2;
    let r = centered(area, 70, u16::try_from(rows).unwrap_or(u16::MAX));
    f.render_widget(Clear, r);
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
        // While completing, the candidates take the hint's first row.
        let strip = form.completion_strip().filter(|_| on);
        for (n, hint) in hints[i].iter().enumerate() {
            match &strip {
                Some(s) if n == 0 => {
                    lines.push(completion_line(s, 2 + FORM_LABEL_WIDTH, width + 1, t));
                }
                Some(_) => lines.push(Line::from("")),
                None => lines.push(Line::from(Span::styled(
                    format!("{:w$}{hint}", "", w = 2 + FORM_LABEL_WIDTH),
                    t.dim_style(),
                ))),
            }
        }
        if field.label == "Consumers" {
            for (row, warn) in &scope_rows {
                lines.push(Line::from(Span::styled(
                    format!("{:w$}{row}", "", w = 2 + FORM_LABEL_WIDTH),
                    if *warn {
                        Style::new().fg(t.warn)
                    } else {
                        Style::new().fg(t.accent)
                    },
                )));
            }
        }
    }
    match error {
        Some(rows) => lines.extend(
            rows.into_iter()
                .map(|e| Line::from(Span::styled(e, Style::new().fg(t.bad)))),
        ),
        None => lines.push(Line::from(Span::styled(
            match (&form.alt, form.expanded) {
                _ if matches!(form.intent, EditIntent::BulkLabels(_)) => {
                    "empty keeps · - removes · Ctrl-N values · Ctrl-A new label"
                }
                _ if !matches!(form.intent, EditIntent::RuleFromFlow(_)) => "* required",
                (_, false) => "* required · Ctrl-N names · Ctrl-R new ruleset · Ctrl-A all labels",
                (_, true) => "* required · Ctrl-N names · Ctrl-R new ruleset · Ctrl-A model view",
            },
            t.dim_style(),
        ))),
    }

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
pub(crate) fn completion_line<'a>(
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

/// Field names of the events editor, in the order of `EventsEditor::values`.
const EVENTS_FIELDS: [&str; 3] = ["severity", "type", "window"];

/// The events editor: severity, type, window; the completion strip under
/// the Type field while completing; then the error or the syntax.
pub fn render_events_editor(
    model: &Model,
    e: &crate::tui::events_editor::EventsEditor,
    f: &mut Frame,
    area: Rect,
) {
    let t = &model.theme;
    let strip = e.completion_strip();
    let r = centered(area, 76, 9 + u16::from(strip.is_some()));
    f.render_widget(Clear, r);
    let block = panel(
        "Edit events",
        "Tab next · Ctrl-N type · Enter apply · Esc cancel",
        t,
        true,
    );
    let inner = (r.width as usize).saturating_sub(2);
    const LABEL: usize = 12;
    // The end of a long value stays in view, where the cursor is.
    let room = inner.saturating_sub(2 + LABEL + 1);
    let mut lines: Vec<Line> = Vec::new();
    for (i, name) in EVENTS_FIELDS.iter().enumerate() {
        let on = i == e.active;
        let cursor = if on { "▏" } else { "" };
        let chars: Vec<char> = e.values[i].chars().collect();
        let shown: String = chars[chars.len().saturating_sub(room)..].iter().collect();
        lines.push(Line::from(vec![
            Span::styled(if on { "▸ " } else { "  " }, Style::new().fg(t.accent)),
            Span::styled(format!("{name:<LABEL$}"), t.header_style()),
            Span::styled(
                format!("{shown}{cursor}"),
                if on {
                    Style::new().fg(t.fg).bg(t.selected_bg)
                } else {
                    t.base()
                },
            ),
        ]));
        if on && let Some(strip) = &strip {
            lines.push(completion_line(strip, 2 + LABEL, room, t));
        }
    }
    lines.push(Line::from(""));
    match &e.error {
        Some(err) => lines.push(Line::from(Span::styled(
            err.clone(),
            Style::new().fg(t.bad),
        ))),
        None => {
            for hint in [
                "severity: err (asks the PCE) · warning+ that and worse (local)",
                "type: sec_policy, rule_set · !user.* hides · =exact · *.delete",
                "window e.g. 24h · empty severity or type = any",
            ] {
                lines.push(Line::from(Span::styled(hint, t.dim_style())));
            }
        }
    }
    f.render_widget(Paragraph::new(lines).block(block).style(t.base()), r);
}

/// The key line of the type summary.
const EVENT_TYPES_KEYS: &str = "Enter only this · x/! hide · Esc close";

/// `t` on Events: every event type of the load with its count, most
/// frequent first; the types the filter hides are dim and say so.
pub fn render_event_types(model: &Model, selected: usize, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let e = &model.events;
    let types = e.type_counts();
    let filter = pce::models::TypeFilter::parse(&e.filter.event_type).unwrap_or_default();
    let by_severity = e.severity_hidden_types();
    let r = centered(
        area,
        64,
        (types.len() as u16).saturating_add(3).clamp(6, 24),
    );
    f.render_widget(Clear, r);
    // Rows inside the borders, less the key line.
    let rows = (r.height.saturating_sub(2) as usize).saturating_sub(1);
    let sel = selected.min(types.len().saturating_sub(1));
    let first = sel.saturating_sub(rows.saturating_sub(1));
    let width = (r.width as usize).saturating_sub(2);
    let count_w = types.first().map_or(1, |(_, n)| n.to_string().len()).max(5);
    let name_w = width.saturating_sub(2 + count_w + 2 + 10);
    let mut lines: Vec<Line> = types
        .iter()
        .enumerate()
        .skip(first)
        .take(rows)
        .map(|(i, (ty, n))| {
            let hidden = !filter.keeps_type(ty);
            let severity = !hidden && by_severity.contains(ty);
            let name = if ty.is_empty() { "(no type)" } else { ty };
            let style = if hidden || severity {
                t.dim_style()
            } else {
                t.base()
            };
            Line::from(vec![
                Span::styled(
                    if i == sel { "▸ " } else { "  " },
                    Style::new().fg(t.accent),
                ),
                Span::styled(format!("{:<name_w$}", clip(&plain(name), name_w)), style),
                Span::styled(format!("{n:>count_w$}"), style),
                if hidden {
                    Span::styled("  hidden", Style::new().fg(t.warn))
                } else if severity {
                    Span::styled("  (severity)", t.dim_style())
                } else {
                    Span::raw("")
                },
            ])
        })
        .collect();
    lines.resize(rows, Line::from(""));
    lines.push(Line::from(Span::styled(EVENT_TYPES_KEYS, t.dim_style())));
    let meta = format!(
        "{} types · {} events{}",
        types.len(),
        e.raw.len(),
        if e.truncated { " · capped" } else { "" }
    );
    f.render_widget(
        Paragraph::new(lines)
            .block(panel("Event types", &meta, t, true))
            .style(t.base()),
        r,
    );
}

/// The key line of the VEN fleet.
const VEN_KEYS: &str = "Enter filters the list to it · j/k move · Esc close";

/// A health state's colour, as the Workloads rows use it.
pub fn health_style(h: pce::models::VenHealth, t: &Theme) -> Style {
    use pce::models::VenHealth;
    match h {
        VenHealth::Healthy => Style::new().fg(t.ok),
        VenHealth::Warning => Style::new().fg(t.warn),
        VenHealth::Error | VenHealth::Offline => Style::new().fg(t.bad),
        VenHealth::Unmanaged => t.dim_style(),
    }
}

/// `:ven`: the VEN fleet of the loaded workloads in four sections
/// (health, versions newest first, enforcement, health messages); the
/// cursor is on a bucket, the list scrolls to keep it shown.
pub fn render_ven(model: &Model, selected: usize, f: &mut Frame, area: Rect) {
    use crate::tui::ven_view::{VenBucket, VenLine, bucket_lines, lines as ven_lines};
    let t = &model.theme;
    let s = &model.workloads.ven;
    let all = ven_lines(s);
    let at = bucket_lines(&all);
    let sel_line = at
        .get(selected.min(at.len().saturating_sub(1)))
        .copied()
        .unwrap_or(0);
    let r = centered(area, 72, (all.len() as u16).saturating_add(3).clamp(8, 30));
    f.render_widget(Clear, r);
    // Rows inside the borders, less the key line.
    let rows = (r.height.saturating_sub(2) as usize).saturating_sub(1);
    let first = sel_line.saturating_sub(rows.saturating_sub(1));
    let width = (r.width as usize).saturating_sub(2);
    let count_w = 6;
    let tag_w = 8;
    let name_w = width.saturating_sub(2 + tag_w + count_w + 1);
    let mut lines: Vec<Line> = all
        .iter()
        .enumerate()
        .skip(first)
        .take(rows)
        .map(|(i, l)| match l {
            VenLine::Section(title, note) => {
                let mut spans = vec![Span::styled(title.to_string(), t.header_style())];
                if !note.is_empty() {
                    spans.push(Span::styled(format!("  {note}"), t.dim_style()));
                }
                Line::from(spans)
            }
            VenLine::Note(text) => Line::from(Span::styled(format!("  {text}"), t.dim_style())),
            VenLine::Bucket { bucket, tag, count } => {
                let (name, style) = match bucket {
                    VenBucket::Health(h) => (h.as_str().to_string(), health_style(*h, t)),
                    VenBucket::Version(v) => (plain(v), t.base()),
                    VenBucket::Enforcement(e) => (e.clone(), t.base()),
                    VenBucket::Message(m) => (plain(m), t.base()),
                };
                let style = if *count == 0 { t.dim_style() } else { style };
                let tag_style = match *tag {
                    "error" => Style::new().fg(t.bad),
                    "warning" | "behind" => Style::new().fg(t.warn),
                    "newest" => Style::new().fg(t.ok),
                    _ => t.dim_style(),
                };
                Line::from(vec![
                    Span::styled(
                        if i == sel_line { "▸ " } else { "  " },
                        Style::new().fg(t.accent),
                    ),
                    Span::styled(format!("{:<name_w$}", clip(&name, name_w)), style),
                    Span::styled(format!("{tag:>tag_w$}"), tag_style),
                    Span::styled(format!("{count:>count_w$}"), style),
                ])
            }
        })
        .collect();
    lines.resize(rows, Line::from(""));
    lines.push(Line::from(Span::styled(VEN_KEYS, t.dim_style())));
    let meta = if s.truncated {
        format!("{} VENs of {} loaded (capped)", s.vens, s.workloads)
    } else {
        format!("{} VENs · {} workloads", s.vens, s.workloads)
    };
    f.render_widget(
        Paragraph::new(lines)
            .block(panel("VEN health", &meta, t, true))
            .style(t.base()),
        r,
    );
}

/// Most lines shown for one event; the rest scrolls. A smaller terminal
/// shows fewer: the cut follows the box height.
const EVENT_DETAIL_LINES: usize = 30;

/// The event as its change diff (titles in the header style, `+`/`-` in
/// the pending diff's colours), or as pretty JSON when `raw`.
pub fn render_event_detail(
    model: &Model,
    event: &pce::models::Event,
    scroll: usize,
    raw: bool,
    f: &mut Frame,
    area: Rect,
) {
    let t = &model.theme;
    let r = centered(area, 100, EVENT_DETAIL_LINES as u16 + 2);
    f.render_widget(Clear, r);
    let shown = (r.height.saturating_sub(2) as usize).min(EVENT_DETAIL_LINES);
    let all: Vec<Line> = if raw {
        serde_json::to_string_pretty(event)
            .unwrap_or_default()
            .lines()
            .map(|l| Line::from(plain(l)))
            .collect()
    } else {
        detail_lines(event, &model.event_names())
            .into_iter()
            .map(|l| match l {
                DetailLine::Title(text) => Line::from(Span::styled(plain(&text), t.header_style())),
                DetailLine::Line(d) => {
                    let (sign, style) = match d.sign {
                        Sign::Added => ("+ ", Style::new().fg(t.ok)),
                        Sign::Removed => ("- ", Style::new().fg(t.bad)),
                        Sign::Same => ("  ", t.base()),
                    };
                    Line::from(Span::styled(format!("{sign}{}", plain(&d.text)), style))
                }
            })
            .collect()
    };
    let total = all.len();
    // The model only stops at the last line; the view keeps the box full.
    let first = scroll.min(total.saturating_sub(shown));
    let lines: Vec<Line> = all.into_iter().skip(first).take(shown).collect();
    let toggle = if raw { "J diff" } else { "J json" };
    let table = &model.events.table;
    let at = format!(
        "{}/{}",
        table.selected_pos() + 1,
        table.visible().len().max(1)
    );
    let meta = if total > shown {
        let last = (first + shown).min(total);
        format!(
            "{at} · j/k event · ^d/^u lines {}-{last}/{total} · {toggle} · Esc",
            first + 1
        )
    } else {
        format!("{at} · j/k event · {toggle} · Esc closes")
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
pub(crate) fn wrap_chars(line: &str, width: usize) -> Vec<String> {
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

/// The batch confirm as drawn: the summary pinned on top and every line
/// below it, each wrapped to the inner width (the box is the dry run:
/// nothing may be clipped), and the scrolled rows that fit.
struct BatchLayout {
    rect: Rect,
    summary: Vec<String>,
    content: Vec<String>,
    rows: usize,
}

fn confirm_batch_layout(batch: &Batch, area: Rect) -> BatchLayout {
    let width = usize::from(CONFIRM_WIDTH.min(area.width).saturating_sub(2));
    // The caller's warning first (what changed since the preview).
    let mut summary: Vec<String> = batch
        .note
        .iter()
        .flat_map(|n| wrap_chars(&plain(&format!("⚠ {n}")), width))
        .collect();
    summary.extend(wrap_chars(&plain(&batch.summary()), width));
    let content: Vec<String> = batch
        .lines()
        .iter()
        .flat_map(|l| wrap_chars(&plain(l), width))
        .collect();
    // Borders and the pinned summary.
    let chrome = u16::try_from(summary.len())
        .unwrap_or(u16::MAX)
        .saturating_add(2);
    let height = u16::try_from(content.len())
        .unwrap_or(u16::MAX)
        .saturating_add(chrome);
    let rect = centered(area, CONFIRM_WIDTH, height);
    let rows = usize::from(rect.height.saturating_sub(chrome));
    BatchLayout {
        rect,
        summary,
        content,
        rows,
    }
}

/// The first line shown when the batch confirm is scrolled to the end.
pub fn confirm_batch_last_scroll(batch: &Batch, area: Rect) -> usize {
    let l = confirm_batch_layout(batch, area);
    l.content.len().saturating_sub(l.rows)
}

/// The keys of a scrolled box, and how many lines are still below.
pub(crate) fn scroll_meta(below: usize, keys: &str) -> String {
    if below > 0 {
        format!("↓ {below} more · j/k scroll · {keys}")
    } else {
        keys.to_string()
    }
}

/// `Send these requests?`: the batch's summary pinned, then one block per
/// request (`N/M METHOD path`, its summary, the override denies that
/// block it, its body), scrolled; the title says how much is still below.
pub fn render_confirm_batch(model: &Model, batch: &Batch, scroll: u16, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let BatchLayout {
        rect: r,
        summary,
        content,
        rows,
    } = confirm_batch_layout(batch, area);
    f.render_widget(Clear, r);
    let first = usize::from(scroll).min(content.len().saturating_sub(rows));
    let below = content.len().saturating_sub(first + rows);
    let meta = scroll_meta(below, "y send all · n cancel");
    let mut lines: Vec<Line> = summary
        .into_iter()
        .map(|s| Line::from(Span::styled(s, Style::new().fg(t.warn))))
        .collect();
    lines.extend(content.into_iter().skip(first).take(rows).map(|s| {
        // `2/6 POST …`: a block's head; `  ! blocked: …`: a deny.
        let head = s.split_once(' ').is_some_and(|(n, _)| {
            n.contains('/') && n.chars().next().is_some_and(|c| c.is_ascii_digit())
        });
        let style = if head {
            t.header_style()
        } else if s.starts_with("  ! ") {
            Style::new().fg(t.warn)
        } else {
            t.base()
        };
        Line::from(Span::styled(s, style))
    }));
    f.render_widget(
        Paragraph::new(lines)
            .block(panel("Send these requests?", &meta, t, true))
            .style(t.base()),
        r,
    );
}

/// The batch report as drawn: every line wrapped, the rows that fit.
fn batch_report_layout(lines: &[String], area: Rect) -> (Rect, Vec<String>, usize) {
    let width = usize::from(CONFIRM_WIDTH.min(area.width).saturating_sub(2));
    let content: Vec<String> = lines
        .iter()
        .flat_map(|l| wrap_chars(&plain(l), width))
        .collect();
    let height = u16::try_from(content.len())
        .unwrap_or(u16::MAX)
        .saturating_add(2);
    let rect = centered(area, CONFIRM_WIDTH, height);
    let rows = usize::from(rect.height.saturating_sub(2));
    (rect, content, rows)
}

/// The first line shown when the batch report is scrolled to the end.
pub fn batch_report_last_scroll(lines: &[String], area: Rect) -> usize {
    let (_, content, rows) = batch_report_layout(lines, area);
    content.len().saturating_sub(rows)
}

/// `Batch result`: each step and what became of it, then how to undo.
pub fn render_batch_report(model: &Model, scroll: u16, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let lines = model.batch_report.clone().unwrap_or_default();
    let (r, content, rows) = batch_report_layout(&lines, area);
    f.render_widget(Clear, r);
    let first = usize::from(scroll).min(content.len().saturating_sub(rows));
    let below = content.len().saturating_sub(first + rows);
    let meta = scroll_meta(below, "Esc closes");
    let shown: Vec<Line> = content
        .into_iter()
        .skip(first)
        .take(rows)
        .map(|s| {
            let style = if s.contains("FAILED") || s.contains("NOT ") {
                Style::new().fg(t.bad)
            } else if s.starts_with("    ") || s.is_empty() {
                t.base()
            } else {
                t.header_style()
            };
            Line::from(Span::styled(s, style))
        })
        .collect();
    f.render_widget(
        Paragraph::new(shown)
            .block(panel("Batch result", &meta, t, true))
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
    // An allow rule's consumers are intra- or extra-scope; a deny rule as
    // loaded carries no such flag.
    let enabled = if rule.enabled { "enabled" } else { "disabled" };
    let flags = match rule.kind {
        pce::RuleKind::Allow => format!(
            "{enabled} · {}",
            crate::rule_scope::RuleScope::from_unscoped(rule.unscoped_consumers).wording()
        ),
        pce::RuleKind::Deny | pce::RuleKind::OverrideDeny => enabled.to_string(),
    };
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
        (Screen::Workloads, true) => "Space mark  L labels  M enforce  n/e/X  x group",
        (Screen::Policy, true) => "n/N:new e:edit x:del  d scope  p pending",
        (Screen::Traffic, true) => "w:rule  f filter  r run  v view  g group",
        (Screen::Traffic, _) => "f filter  r run  a auto  v view  g group  / search",
        (Screen::Policy, _) => "d scope  p pending  Enter detail  Tab pane  i impact",
        (Screen::Objects, false) => "Tab type  Enter detail  r refresh  a auto  / search",
        (Screen::Workloads, false) => "t/T traffic  x group  f labels  Enter detail",
        (Screen::Events, _) => "f filter  x/o hide/only type  t types  Enter detail",
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
        (Screen::Traffic, _) => "+/- window  ]/[ cap  D draft  :fl saved",
        (Screen::Workloads, _) => "h/H/v health  :ven  E/m/o/u  c clear",
        (Screen::Events, _) => "S severity  r refresh  a auto  :fl saved",
        (Screen::Policy, _) => ":fl saved",
        _ => "",
    }
}
