//! Screen chrome layout. Returns None when the terminal is too small.
use ratatui::layout::{Constraint, Layout, Rect};

pub const MIN_WIDTH: u16 = 60;
pub const MIN_HEIGHT: u16 = 16;

#[derive(Debug, Clone, Copy)]
pub struct Chrome {
    pub header: Rect,
    pub tabs: Rect,
    pub body: Rect,
    pub status: Rect,
}

pub fn chrome(area: Rect) -> Option<Chrome> {
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        return None;
    }
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(5),
        Constraint::Length(1),
    ])
    .split(area);
    Some(Chrome {
        header: rows[0],
        tabs: rows[1],
        body: rows[2],
        status: rows[3],
    })
}

/// A centred rectangle of at most `w`×`h` inside `area` (for overlays).
pub fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    let x = area.x + (area.width - w) / 2;
    let y = area.y + (area.height - h) / 2;
    Rect::new(x, y, w, h)
}
