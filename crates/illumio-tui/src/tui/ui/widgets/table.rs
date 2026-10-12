//! Renders a `TableView` as a ratatui Table inside a panel block. Column
//! widths come from the column definitions; `width == 0` fills the rest.
use crate::tui::table::{Column, TableView};
use crate::tui::ui::theme::Theme;
use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Cell, Paragraph, Row, Table, TableState};

pub fn header_meta<T>(view: &TableView<T>) -> String {
    let mut parts = Vec::new();
    if let Some((c, desc)) = view.sort() {
        parts.push(format!(
            "sort {} {}",
            view.columns[c].title,
            if desc { "↓" } else { "↑" }
        ));
    }
    if view.is_searching() {
        parts.push(format!("/{}▏", view.search_text()));
    } else if !view.search_text().is_empty() {
        parts.push(format!("/{}", view.search_text()));
    }
    parts.join(" · ")
}

/// Columns to draw at this width: `(column index, width, use narrow cell)`.
/// Sort and search keep using the full column set; only drawing adapts.
pub fn visible_columns<T>(columns: &[Column<T>], width: u16) -> Vec<(usize, u16, bool)> {
    let narrow = width < 120;
    columns
        .iter()
        .enumerate()
        .filter(|(_, c)| width >= c.hide_below)
        .map(|(i, c)| {
            let w = if narrow && c.narrow_width > 0 {
                c.narrow_width
            } else {
                c.width
            };
            (i, w, narrow)
        })
        .collect()
}

/// `no rows` / `no rows match the search` drawn in `inner` when the view
/// is empty; whether it was.
fn render_empty<T>(f: &mut Frame, inner: Rect, view: &TableView<T>, theme: &Theme) -> bool {
    if !view.is_empty() {
        return false;
    }
    let msg = if view.search_text().is_empty() {
        "no rows"
    } else {
        "no rows match the search"
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(msg, theme.dim_style()))),
        inner,
    );
    true
}

/// `s` cut to at most `w` cells of display width (ratatui's measure).
fn clip_width(s: &str, w: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let cw = Span::raw(c.to_string()).width();
        if used + cw > w {
            break;
        }
        used += cw;
        out.push(c);
    }
    out
}

/// `s` clipped and padded to exactly `w` cells, right-aligned when `right`.
fn fit(s: &str, w: usize, right: bool) -> String {
    let s = clip_width(s, w);
    let pad = " ".repeat(w.saturating_sub(Span::raw(s.as_str()).width()));
    if right { pad + &s } else { s + &pad }
}

/// One row's cells for the drawn columns `cols`: the narrow cell where the
/// column has one and the table is narrow, fitted to `widths` (`0`: as is,
/// for a fill column the table widget sizes).
fn row_cells<T>(
    view: &TableView<T>,
    item: &T,
    cols: &[(usize, u16, bool)],
    widths: &[usize],
) -> Vec<String> {
    cols.iter()
        .zip(widths)
        .map(|(&(ci, _, narrow), &w)| {
            let c = &view.columns[ci];
            let text = match (narrow, c.narrow) {
                (true, Some(n)) => n(item),
                _ => view.cell(item, ci),
            };
            if w == 0 { text } else { fit(&text, w, c.right) }
        })
        .collect()
}

pub fn render<T>(
    f: &mut Frame,
    area: Rect,
    view: &TableView<T>,
    theme: &Theme,
    block: Block,
    row_style: &dyn Fn(&T) -> Style,
) {
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 || render_empty(f, inner, view, theme) {
        return;
    }
    let cols = visible_columns(&view.columns, area.width);
    let constraints: Vec<Constraint> = cols
        .iter()
        .map(|&(_, w, _)| {
            if w == 0 {
                Constraint::Min(8)
            } else {
                Constraint::Length(w)
            }
        })
        .collect();
    let widths: Vec<usize> = cols.iter().map(|&(_, w, _)| w as usize).collect();
    let header = Row::new(
        cols.iter()
            .map(|&(ci, _, _)| Cell::from(view.columns[ci].title))
            .collect::<Vec<_>>(),
    )
    .style(theme.header_style());
    let height = inner.height.saturating_sub(1) as usize; // minus header row
    let start = view.offset().min(view.visible().len());
    let end = (start + height).min(view.visible().len());
    let rows: Vec<Row> = view.visible()[start..end]
        .iter()
        .map(|&i| {
            let item = &view.rows()[i];
            Row::new(row_cells(view, item, &cols, &widths)).style(row_style(item))
        })
        .collect();
    let table = Table::new(rows, constraints)
        .header(header)
        .column_spacing(1)
        .row_highlight_style(theme.selected_style())
        .highlight_symbol("▸ ");
    let mut state = TableState::default()
        .with_offset(0)
        .with_selected(Some(view.selected_pos().saturating_sub(start)));
    f.render_stateful_widget(table, inner, &mut state);
}

/// Like `render`, for tables some of whose rows are one line across every
/// column (`spanning` returns it with its style): drawn as lines, columns
/// laid out the same way (fixed widths, `width == 0` takes the rest, one
/// cell apart, the `▸ ` cursor), measured in display width.
pub fn render_spanning<T>(
    f: &mut Frame,
    area: Rect,
    view: &TableView<T>,
    theme: &Theme,
    block: Block,
    row_style: &dyn Fn(&T) -> Style,
    spanning: &dyn Fn(&T) -> Option<(String, Style)>,
) {
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 || render_empty(f, inner, view, theme) {
        return;
    }
    let cols = visible_columns(&view.columns, area.width);
    let avail = inner.width.saturating_sub(2) as usize;
    let fixed: usize =
        cols.iter().map(|c| c.1 as usize).sum::<usize>() + cols.len().saturating_sub(1);
    let fill = avail.saturating_sub(fixed).max(8);
    let widths: Vec<usize> = cols
        .iter()
        .map(|&(_, w, _)| if w == 0 { fill } else { w as usize })
        .collect();
    let line_of = |cells: Vec<String>| -> String {
        clip_width(&cells.join(" "), avail).trim_end().to_string()
    };
    let header = line_of(
        cols.iter()
            .zip(&widths)
            .map(|(&(ci, _, _), &w)| fit(view.columns[ci].title, w, false))
            .collect(),
    );
    let mut lines = vec![Line::from(Span::styled(
        format!("  {header}"),
        theme.header_style(),
    ))];
    let height = inner.height.saturating_sub(1) as usize;
    let start = view.offset().min(view.visible().len());
    let end = (start + height).min(view.visible().len());
    for (pos, &i) in view.visible()[start..end].iter().enumerate() {
        let item = &view.rows()[i];
        let (text, mut style) = match spanning(item) {
            Some((text, style)) => (clip_width(&text, avail), style),
            None => (
                line_of(row_cells(view, item, &cols, &widths)),
                row_style(item),
            ),
        };
        let selected = start + pos == view.selected_pos();
        let mut line = format!("{}{text}", if selected { "▸ " } else { "  " });
        if selected {
            // The cursor row is highlighted across the whole width.
            style = style.patch(theme.selected_style());
            line = fit(&line, inner.width as usize, false);
        }
        lines.push(Line::from(Span::styled(line, style)));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::table::SortKey;

    fn cols() -> Vec<Column<u32>> {
        vec![
            Column {
                title: "a",
                width: 10,
                cell: |n| n.to_string(),
                key: |n| SortKey::Num(u64::from(*n)),
                right: false,
                hide_below: 0,
                narrow: Some(|n| format!("n{n}")),
                narrow_width: 4,
            },
            Column {
                title: "b",
                width: 10,
                cell: |n| n.to_string(),
                key: |n| SortKey::Num(u64::from(*n)),
                right: false,
                hide_below: 100,
                narrow: None,
                narrow_width: 0,
            },
            Column {
                title: "c",
                width: 6,
                cell: |n| n.to_string(),
                key: |n| SortKey::Num(u64::from(*n)),
                right: false,
                hide_below: 0,
                narrow: None,
                narrow_width: 0,
            },
        ]
    }

    #[test]
    fn fit_measures_display_width_not_chars() {
        assert_eq!(
            fit("日本語", 4, false),
            "日本",
            "two wide glyphs fill four cells"
        );
        assert_eq!(fit("日本語", 5, false), "日本 ");
        assert_eq!(fit("42", 4, true), "  42");
        assert_eq!(fit("← a", 3, false), "← a");
        assert_eq!(clip_width("abc", 0), "");
    }

    #[test]
    fn narrow_width_hides_and_shortens_columns() {
        assert_eq!(
            visible_columns(&cols(), 60),
            vec![(0, 4, true), (2, 6, true)]
        );
        assert_eq!(
            visible_columns(&cols(), 120),
            vec![(0, 10, false), (1, 10, false), (2, 6, false)]
        );
    }

    #[test]
    fn render_at_60_uses_the_narrow_cell_and_drops_hidden_column() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        use ratatui::widgets::Block;
        let mut v = TableView::new(cols());
        v.set_rows(vec![42]);
        let theme = Theme::dark();
        let mut term = Terminal::new(TestBackend::new(60, 5)).unwrap();
        term.draw(|f| {
            render(f, f.area(), &v, &theme, Block::bordered(), &|_| {
                Style::new()
            })
        })
        .unwrap();
        let buf = term.backend().buffer();
        let text: String = (0..buf.area.height)
            .flat_map(|y| (0..buf.area.width).map(move |x| (x, y)))
            .map(|(x, y)| buf[(x, y)].symbol().to_string())
            .collect();
        assert!(text.contains("n42"), "{text}");
        assert!(!text.contains(" b "), "{text}");
    }
}
