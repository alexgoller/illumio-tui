//! The JSON envelope every command prints, the table renderer shared with
//! the TUI (same ratatui Table widget, drawn into an off-screen buffer), and
//! the CSV writer `--csv` and the TUI's `:export` share.
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
    Csv,
}

/// clap keeps the three flags exclusive.
pub fn choose_format(json: bool, table: bool, csv: bool) -> OutputFormat {
    if csv {
        OutputFormat::Csv
    } else if table {
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

/// Where label usage came from when the PCE has no usage flags: our own
/// counts. `labels --unused` (meta) and the TUI's `u` (header, status) say it.
pub const USAGE_FROM_COUNTS: &str = "workloads, rules and scopes only";

#[derive(Debug, Clone, Default, Serialize)]
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
    /// Traffic: the window's length (`1d`) when the result is complete,
    /// `null` when it is truncated (it covers an unknown part of it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub covered: Option<Option<String>>,
    /// Traffic: the cap the result was fetched at.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cap: Option<u32>,
    /// Traffic with `--backfill`: every cap tried, in order, the first
    /// query's included; the last one is the result's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cap_steps: Option<Vec<u32>>,
    /// Traffic: a supercluster region did not respond, so rows are missing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub incomplete: Option<bool>,
    /// Traffic with `--app`: the Explorer queries the result took (one per
    /// side: 2 for `both`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queries: Option<u32>,
    /// Traffic with `--draft`: the draft re-evaluation ran, and how many
    /// flow records the draft would allow (`fixed`) or block
    /// (`newly_blocked`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub draft: Option<pce::traffic::DraftSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub write: Option<WriteMeta>,
    /// `labels --unused`: where label usage came from, `pce` (its own
    /// usage flags) or `workloads, rules and scopes only` (a PCE without
    /// them: our own counts).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<&'static str>,
    /// `workloads`: the filters applied locally after the request
    /// (`offline`, `unlabeled (app, env, role)`, `unhealthy`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_filter: Option<Vec<String>>,
    /// Warnings about the arguments that did not stop the command (also
    /// printed to stderr): `workloads --unlabeled` with an unknown key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warnings: Option<Vec<String>>,
    /// The marks applied (`marks` commands, and traffic with a valid file
    /// holding an active mark): how many are active, and the file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marks: Option<MarksMeta>,
    /// Traffic with an active mark: the rows squelch removed and the
    /// squelched flow records of the view (zeros included); `gen`: the
    /// flows only (its rules include them).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub squelched: Option<SquelchedMeta>,
}

/// `meta.squelched`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SquelchedMeta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows: Option<u64>,
    pub flows: u64,
}

/// `meta.marks`.
#[derive(Debug, Clone, Serialize)]
pub struct MarksMeta {
    pub active: usize,
    pub file: String,
    /// Marks with `draft_decision` left out because the result has no
    /// draft decisions (run with `--draft` to apply them).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waiting_draft: Option<usize>,
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
    /// A line printed above the table (not in CSV): what its counts count.
    fn units(&self) -> Option<&'static str> {
        None
    }
    /// Full-width lines printed under each row, outside the column layout
    /// (so never clipped). Empty, or one entry per row.
    fn notes(&self) -> Vec<Vec<String>> {
        Vec::new()
    }
    /// `--csv`'s header row: `headers()` unless the full rows add or
    /// rename columns.
    fn full_headers(&self) -> Vec<&'static str> {
        self.headers()
    }
    /// `--csv`'s rows: every cell whole, where `rows()` abbreviates, folds
    /// a list into a count or joins it for the eye (lists joined with `; `,
    /// numbers plain, `None` empty).
    fn full_rows(&self) -> Vec<Vec<String>> {
        self.rows()
    }
}

/// A list as one CSV cell.
pub fn join_list<S: AsRef<str>>(items: impl IntoIterator<Item = S>) -> String {
    items
        .into_iter()
        .map(|s| s.as_ref().to_string())
        .collect::<Vec<_>>()
        .join("; ")
}

/// One RFC 4180 field: quoted only when it holds `,`, `"`, CR or LF.
fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\r', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// One CSV record, without its line end.
fn csv_record<'a>(cells: impl IntoIterator<Item = &'a str>) -> String {
    cells
        .into_iter()
        .map(csv_field)
        .collect::<Vec<_>>()
        .join(",")
}

/// RFC 4180 CSV: a header row, then one record per row; CRLF line ends,
/// UTF-8 without BOM.
pub fn write_csv(
    out: &mut impl Write,
    headers: &[&str],
    rows: &[Vec<String>],
) -> std::io::Result<()> {
    write!(out, "{}\r\n", csv_record(headers.iter().copied()))?;
    for r in rows {
        write!(out, "{}\r\n", csv_record(r.iter().map(String::as_str)))?;
    }
    Ok(())
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

pub fn write_table<T: TableRows + ?Sized>(out: &mut impl Write, data: &T) -> std::io::Result<()> {
    let headers = data.headers();
    let rows = data.rows();
    let table = render_table(&headers, &rows);
    if let Some(u) = data.units() {
        writeln!(out, "{u}")?;
    }
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

/// A table's full rows as CSV.
pub fn write_table_csv<T: TableRows + ?Sized>(
    out: &mut impl Write,
    data: &T,
) -> std::io::Result<()> {
    write_csv(out, &data.full_headers(), &data.full_rows())
}

/// One line for stderr in table and CSV mode so stdout stays clean.
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
    if let Some(Some(c)) = &meta.covered {
        s.push_str(&format!(" · covers {c}"));
    }
    if let Some(steps) = meta.cap_steps.as_ref().filter(|s| s.len() > 1) {
        let caps: Vec<String> = steps.iter().map(u32::to_string).collect();
        s.push_str(&format!(" · caps {}", caps.join(" → ")));
    }
    if let Some(n) = meta.queries {
        s.push_str(&format!(" · {}", pce::traffic::queries_text(n)));
    }
    match meta.usage {
        Some("pce") => s.push_str(" · usage from the PCE"),
        Some(u) => s.push_str(&format!(" · usage from {u}")),
        None => {}
    }
    if let Some(f) = &meta.local_filter {
        s.push_str(&format!(" · filtered locally: {}", f.join(", ")));
    }
    match (meta.truncated, meta.cap) {
        (true, Some(cap)) => s.push_str(&format!(
            " · WARNING: result truncated, capped at {cap}; --backfill raises the cap"
        )),
        (true, None) => s.push_str(" · WARNING: result truncated"),
        _ => {}
    }
    if meta.incomplete == Some(true) {
        s.push_str(" · WARNING: incomplete, a PCE region did not respond");
    }
    match meta.squelched {
        Some(SquelchedMeta {
            rows: Some(r),
            flows,
        }) if flows > 0 => s.push_str(&format!(" · {r} rows ({flows} flows) squelched")),
        Some(SquelchedMeta { rows: None, flows }) if flows > 0 => {
            s.push_str(&format!(" · {flows} squelched (rules include them)"))
        }
        _ => {}
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

    fn csv(headers: &[&str], rows: &[Vec<String>]) -> String {
        let mut out = Vec::new();
        write_csv(&mut out, headers, rows).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn csv_quotes_only_commas_quotes_and_line_breaks() {
        let rows = vec![vec![
            "plain".to_string(),
            "a,b".into(),
            "say \"hi\"".into(),
            "two\nlines".into(),
            "cr\r".into(),
            "semi; colon".into(),
        ]];
        assert_eq!(
            csv(&["a", "b", "c", "d", "e", "f"], &rows),
            "a,b,c,d,e,f\r\nplain,\"a,b\",\"say \"\"hi\"\"\",\"two\nlines\",\"cr\r\",semi; colon\r\n"
        );
    }

    #[test]
    fn csv_keeps_empty_cells_and_ends_every_line_with_crlf() {
        let rows = vec![
            vec![String::new(), "x".into(), String::new()],
            vec!["1".into(), String::new(), "ü".into()],
        ];
        assert_eq!(csv(&["a", "b", "c"], &rows), "a,b,c\r\n,x,\r\n1,,ü\r\n");
    }

    #[test]
    fn csv_without_rows_is_the_header_alone() {
        assert_eq!(csv(&["key", "value"], &[]), "key,value\r\n");
    }

    #[test]
    fn csv_has_no_bom() {
        assert!(csv(&["h"], &[]).starts_with('h'));
    }

    #[test]
    fn csv_flag_wins_the_format() {
        assert_eq!(choose_format(false, false, true), OutputFormat::Csv);
        assert_eq!(choose_format(false, true, false), OutputFormat::Table);
        assert_eq!(choose_format(true, false, false), OutputFormat::Json);
    }

    #[test]
    fn full_rows_default_to_the_table() {
        struct T;
        impl TableRows for T {
            fn headers(&self) -> Vec<&'static str> {
                vec!["a"]
            }
            fn rows(&self) -> Vec<Vec<String>> {
                vec![vec!["1".into()]]
            }
        }
        let mut out = Vec::new();
        write_table_csv(&mut out, &T).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "a\r\n1\r\n");
        assert_eq!(join_list(["a", "b"]), "a; b");
    }

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
            cap: None,
            cap_steps: None,
            incomplete: None,
            queries: None,
            draft: None,
            write: Some(WriteMeta {
                method: WriteMethod::Post,
                path: "/orgs/5/labels".into(),
                status: 201,
                audit: "x".into(),
            }),
            usage: None,
            local_filter: None,
            warnings: None,
            marks: None,
            squelched: None,
        };
        assert!(meta_line(&meta).contains("POST /orgs/5/labels → 201"));
        let sq = |rows| Meta {
            squelched: Some(SquelchedMeta { rows, flows: 52 }),
            ..Meta::default()
        };
        assert!(meta_line(&sq(Some(3))).ends_with(" · 3 rows (52 flows) squelched"));
        assert!(meta_line(&sq(None)).ends_with(" · 52 squelched (rules include them)"));
    }
}
