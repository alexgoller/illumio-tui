//! The JSON envelope every command prints, and the table renderer shared
//! with the TUI (same ratatui Table widget, drawn into an off-screen buffer).
use pce::WriteMethod;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Rect};
use ratatui::widgets::{Row, Table, Widget};
use serde::Serialize;
use std::io::Write;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Json,
    Table,
}

pub fn choose_format(json: bool, table: bool) -> OutputFormat {
    if table {
        OutputFormat::Table
    } else if json || !std::io::IsTerminal::is_terminal(&std::io::stdout()) {
        OutputFormat::Json
    } else {
        OutputFormat::Table
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Window {
    pub start: String,
    pub end: String,
}

/// What a write command did, for `meta.write`.
#[derive(Debug, Clone, Serialize)]
pub struct WriteMeta {
    pub method: WriteMethod,
    pub path: String,
    pub status: u16,
    /// Audit log location.
    pub audit: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Meta {
    pub profile: String,
    pub elapsed_ms: u128,
    pub api_calls: u64,
    pub bytes: u64,
    /// `hit`, `miss`, `bypass`, or `none` (not cacheable).
    pub cache: &'static str,
    pub truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window: Option<Window>,
    /// Traffic: how much of `window` the flows cover (`4m`; the window
    /// itself when the result is complete).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub covered: Option<String>,
    /// Traffic with `--backfill`: the slices merged into the result (the
    /// truncated first query is replaced, so it does not count).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slices: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub write: Option<WriteMeta>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Envelope<'a, T: Serialize> {
    pub data: &'a T,
    pub meta: &'a Meta,
}

/// Anything the CLI can print as a table.
pub trait TableRows {
    fn headers(&self) -> Vec<&'static str>;
    fn rows(&self) -> Vec<Vec<String>>;
    /// Full-width lines printed under each row, outside the column layout
    /// (so never clipped). Empty, or one entry per row.
    fn notes(&self) -> Vec<Vec<String>> {
        Vec::new()
    }
}

const MAX_COL: usize = 60;

fn clip(s: &str) -> String {
    if s.chars().count() <= MAX_COL {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(MAX_COL - 1).collect::<String>())
    }
}

/// A table as text. A row of one cell in a table of several columns is a
/// heading line (a section header): printed whole in the first column, the
/// other cells empty, and neither clipped nor counted in the widths.
pub fn render_table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let heading = |r: &Vec<String>| r.len() == 1 && headers.len() > 1;
    let headings: Vec<Option<String>> = rows
        .iter()
        .map(|r| heading(r).then(|| r[0].clone()))
        .collect();
    let rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            if heading(r) {
                Vec::new()
            } else {
                r.iter().map(|c| clip(c)).collect()
            }
        })
        .collect();
    let widths: Vec<u16> = headers
        .iter()
        .enumerate()
        .map(|(i, h)| {
            rows.iter()
                .map(|r| r.get(i).map_or(0, |c| c.chars().count()))
                .chain(std::iter::once(h.chars().count()))
                .max()
                .unwrap_or(0) as u16
        })
        .collect();
    let spacing: u16 = 2;
    let total: u16 = widths.iter().sum::<u16>() + spacing * (widths.len().saturating_sub(1) as u16);
    let height = rows.len() as u16 + 1;
    let area = Rect::new(0, 0, total.max(1), height);
    let mut buf = Buffer::empty(area);
    let table = Table::new(
        rows.iter().map(|r| Row::new(r.clone())).collect::<Vec<_>>(),
        widths
            .iter()
            .map(|w| Constraint::Length(*w))
            .collect::<Vec<_>>(),
    )
    .header(Row::new(
        headers.iter().map(|h| h.to_string()).collect::<Vec<_>>(),
    ))
    .column_spacing(spacing);
    table.render(area, &mut buf);
    let mut lines: Vec<String> = (0..height)
        .map(|y| {
            (0..area.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect();
    for (i, h) in headings.into_iter().enumerate() {
        if let Some(h) = h {
            lines[i + 1] = h;
        }
    }
    lines.insert(1, "─".repeat(total as usize));
    if rows.is_empty() {
        lines.push("(no rows)".to_string());
    }
    lines.join("\n")
}

pub fn write_json<T: Serialize>(
    out: &mut impl Write,
    data: &T,
    meta: &Meta,
) -> std::io::Result<()> {
    serde_json::to_writer_pretty(&mut *out, &Envelope { data, meta })?;
    writeln!(out)
}

pub fn write_table<T: TableRows>(out: &mut impl Write, data: &T) -> std::io::Result<()> {
    let headers = data.headers();
    let rows = data.rows();
    let table = render_table(&headers, &rows);
    let notes = data.notes();
    if notes.iter().all(Vec::is_empty) {
        return writeln!(out, "{table}");
    }
    // Two lines of header and rule, then one line per row.
    for (i, line) in table.lines().enumerate() {
        writeln!(out, "{line}")?;
        if let Some(extra) = i.checked_sub(2).and_then(|r| notes.get(r)) {
            for l in extra {
                writeln!(out, "{l}")?;
            }
        }
    }
    Ok(())
}

/// One line for stderr in table mode so stdout stays clean.
pub fn meta_line(meta: &Meta) -> String {
    let kb = meta.bytes as f64 / 1024.0;
    let mut s = format!(
        "{} · {} api call{} · {kb:.1} KB · cache {} · {} ms",
        meta.profile,
        meta.api_calls,
        if meta.api_calls == 1 { "" } else { "s" },
        meta.cache,
        meta.elapsed_ms
    );
    if let Some(c) = &meta.covered {
        s.push_str(&format!(" · covers {c}"));
    }
    if let Some(n) = meta.slices {
        s.push_str(&format!(" · {n} slice{}", if n == 1 { "" } else { "s" }));
    }
    if meta.truncated {
        s.push_str(" · WARNING: result truncated");
    }
    if let Some(w) = &meta.write {
        s.push_str(&format!(
            " · {} {} → {}",
            w.method.as_str(),
            w.path,
            w.status
        ));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_line_shows_the_write() {
        let meta = Meta {
            profile: "demo".into(),
            elapsed_ms: 1,
            api_calls: 1,
            bytes: 0,
            cache: "none",
            truncated: false,
            window: None,
            covered: None,
            slices: None,
            write: Some(WriteMeta {
                method: WriteMethod::Post,
                path: "/orgs/5/labels".into(),
                status: 201,
                audit: "x".into(),
            }),
        };
        assert!(meta_line(&meta).contains("POST /orgs/5/labels → 201"));
    }
}
