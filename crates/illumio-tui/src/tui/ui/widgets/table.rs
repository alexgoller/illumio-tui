//! Renders a `TableView` as a ratatui Table inside a panel block. Column
//! widths come from the column definitions; `width == 0` fills the rest.
use crate::tui::table::TableView;
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
    if inner.height == 0 || inner.width == 0 {
        return;
    }
    if view.is_empty() {
        let msg = if view.search_text().is_empty() {
            "no rows"
        } else {
            "no rows match the search"
        };
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(msg, theme.dim_style()))),
            inner,
        );
        return;
    }
    let widths: Vec<Constraint> = view
        .columns
        .iter()
        .map(|c| {
            if c.width == 0 {
                Constraint::Min(8)
            } else {
                Constraint::Length(c.width)
            }
        })
        .collect();
    let header = Row::new(
        view.columns
            .iter()
            .map(|c| Cell::from(c.title))
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
            let cells: Vec<Cell> = view
                .columns
                .iter()
                .enumerate()
                .map(|(ci, c)| {
                    let text = view.cell(item, ci);
                    let text = if c.right {
                        format!("{text:>w$}", w = c.width as usize)
                    } else {
                        text
                    };
                    Cell::from(text)
                })
                .collect();
            Row::new(cells).style(row_style(item))
        })
        .collect();
    let table = Table::new(rows, widths)
        .header(header)
        .column_spacing(1)
        .row_highlight_style(theme.selected_style())
        .highlight_symbol("▸ ");
    let mut state = TableState::default()
        .with_offset(0)
        .with_selected(Some(view.selected_pos().saturating_sub(start)));
    f.render_stateful_widget(table, inner, &mut state);
}
