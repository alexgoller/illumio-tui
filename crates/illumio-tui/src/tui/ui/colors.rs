//! Colour depth. The theme is 24-bit RGB; a terminal without it (Apple
//! Terminal before macOS 26 reads `48;2;R;G;B` as separate attributes, so a
//! component of 44 paints a blue background) gets every colour mapped to
//! the nearest of the xterm 256, after the frame is drawn.
use ratatui::buffer::Buffer;
use ratatui::style::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Depth {
    TrueColor,
    Ansi256,
}

impl Depth {
    /// `ILLUMIO_TUI_COLORS` (`truecolor`/`24bit` or `256`) when set, else
    /// 256 for Apple Terminal, else true colour.
    pub fn detect() -> Self {
        Self::from_env(
            std::env::var("ILLUMIO_TUI_COLORS").ok().as_deref(),
            std::env::var("TERM_PROGRAM").ok().as_deref(),
        )
    }

    pub fn from_env(choice: Option<&str>, term_program: Option<&str>) -> Self {
        match choice
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("256" | "ansi256" | "8bit") => return Self::Ansi256,
            Some("truecolor" | "24bit" | "rgb") => return Self::TrueColor,
            _ => {}
        }
        if term_program == Some("Apple_Terminal") {
            Self::Ansi256
        } else {
            Self::TrueColor
        }
    }
}

/// Maps every RGB colour in `buf` to its nearest xterm-256 index.
pub fn downgrade(buf: &mut Buffer) {
    for cell in buf.content.iter_mut() {
        cell.fg = to_256(cell.fg);
        cell.bg = to_256(cell.bg);
    }
}

fn to_256(c: Color) -> Color {
    match c {
        Color::Rgb(r, g, b) => Color::Indexed(nearest(r, g, b)),
        other => other,
    }
}

/// The nearest of the 6x6x6 cube (16–231) and the grey ramp (232–255).
pub fn nearest(r: u8, g: u8, b: u8) -> u8 {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let step = |v: u8| -> usize {
        LEVELS
            .iter()
            .enumerate()
            .min_by_key(|(_, l)| (i32::from(**l) - i32::from(v)).abs())
            .map_or(0, |(i, _)| i)
    };
    let (ri, gi, bi) = (step(r), step(g), step(b));
    let cube = (LEVELS[ri], LEVELS[gi], LEVELS[bi]);
    let avg = (u16::from(r) + u16::from(g) + u16::from(b)) / 3;
    let gi_ramp = if avg < 8 {
        0
    } else {
        ((avg - 8) / 10).min(23) as u8
    };
    let grey = 8 + 10 * gi_ramp;
    let dist = |(x, y, z): (u8, u8, u8)| {
        let d = |a: u8, b: u8| (i32::from(a) - i32::from(b)).pow(2);
        d(x, r) + d(y, g) + d(z, b)
    };
    if dist((grey, grey, grey)) < dist(cube) {
        232 + gi_ramp
    } else {
        16 + 36 * ri as u8 + 6 * gi as u8 + bi as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;
    use ratatui::style::Style;

    #[test]
    fn apple_terminal_gets_256_unless_told_otherwise() {
        assert_eq!(
            Depth::from_env(None, Some("Apple_Terminal")),
            Depth::Ansi256
        );
        assert_eq!(Depth::from_env(None, Some("iTerm.app")), Depth::TrueColor);
        assert_eq!(Depth::from_env(None, None), Depth::TrueColor);
        assert_eq!(
            Depth::from_env(Some("truecolor"), Some("Apple_Terminal")),
            Depth::TrueColor
        );
        assert_eq!(
            Depth::from_env(Some(" 256 "), Some("WezTerm")),
            Depth::Ansi256
        );
    }

    #[test]
    fn colours_map_to_the_cube_or_the_grey_ramp() {
        assert_eq!(nearest(0, 0, 0), 16);
        assert_eq!(nearest(255, 255, 255), 231);
        assert_eq!(nearest(255, 0, 0), 196);
        assert_eq!(nearest(0, 135, 255), 33);
        assert_eq!(nearest(128, 128, 128), 244);
        assert_eq!(nearest(16, 18, 24), 233);
    }

    #[test]
    fn a_downgraded_buffer_has_no_rgb_left() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 3, 1));
        buf.set_string(
            0,
            0,
            "ab",
            Style::new()
                .fg(Color::Rgb(255, 0, 0))
                .bg(Color::Rgb(0, 0, 0)),
        );
        buf[(2, 0)].set_fg(Color::Yellow);
        downgrade(&mut buf);
        assert_eq!(buf[(0, 0)].fg, Color::Indexed(196));
        assert_eq!(buf[(0, 0)].bg, Color::Indexed(16));
        assert_eq!(buf[(2, 0)].fg, Color::Yellow, "named colours stay");
        assert!(
            buf.content
                .iter()
                .all(|c| !matches!(c.fg, Color::Rgb(..)) && !matches!(c.bg, Color::Rgb(..)))
        );
    }
}
