//! The JSON envelope every command prints, and the table renderer shared
//! with the TUI (same ratatui Table widget, drawn into an off-screen buffer).
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

#[derive(Debug, Clone, Serialize)]
pub struct Window {
    pub start: String,
    pub end: String,
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
}

const MAX_COL: usize = 60;

fn clip(s: &str) -> String {
    if s.chars().count() <= MAX_COL {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(MAX_COL - 1).collect::<String>())
    }
}

pub fn render_table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| r.iter().map(|c| clip(c)).collect())
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
    writeln!(out, "{}", render_table(&headers, &data.rows()))
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
    if meta.truncated {
        s.push_str(" · WARNING: result truncated");
    }
    s
}
