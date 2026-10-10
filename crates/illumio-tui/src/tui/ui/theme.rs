//! Colour tokens. Two palettes; everything in `ui` reads colours from here.
use ratatui::style::{Color, Modifier, Style};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    pub name: &'static str,
    pub bg: Color,
    pub fg: Color,
    pub dim: Color,
    pub accent: Color,
    pub border: Color,
    pub border_focus: Color,
    pub title: Color,
    pub ok: Color,
    pub warn: Color,
    pub bad: Color,
    pub selected_bg: Color,
    pub header_fg: Color,
}

impl Theme {
    pub fn dark() -> Self {
        Self {
            name: "dark",
            bg: Color::Rgb(16, 18, 24),
            fg: Color::Rgb(220, 222, 228),
            dim: Color::Rgb(120, 126, 140),
            accent: Color::Rgb(97, 175, 239),
            border: Color::Rgb(70, 76, 92),
            border_focus: Color::Rgb(97, 175, 239),
            title: Color::Rgb(198, 120, 221),
            ok: Color::Rgb(152, 195, 121),
            warn: Color::Rgb(229, 192, 123),
            bad: Color::Rgb(224, 108, 117),
            selected_bg: Color::Rgb(44, 50, 66),
            header_fg: Color::Rgb(86, 182, 194),
        }
    }

    pub fn light() -> Self {
        Self {
            name: "light",
            bg: Color::Rgb(250, 250, 247),
            fg: Color::Rgb(40, 42, 48),
            dim: Color::Rgb(120, 124, 136),
            accent: Color::Rgb(0, 102, 204),
            border: Color::Rgb(180, 184, 196),
            border_focus: Color::Rgb(0, 102, 204),
            title: Color::Rgb(150, 50, 180),
            ok: Color::Rgb(40, 140, 60),
            warn: Color::Rgb(176, 120, 0),
            bad: Color::Rgb(200, 40, 50),
            selected_bg: Color::Rgb(222, 230, 245),
            header_fg: Color::Rgb(0, 120, 140),
        }
    }

    pub fn from_name(name: &str) -> Self {
        if name.eq_ignore_ascii_case("light") {
            Self::light()
        } else {
            Self::dark()
        }
    }

    pub fn base(&self) -> Style {
        Style::new().fg(self.fg).bg(self.bg)
    }
    pub fn dim_style(&self) -> Style {
        Style::new().fg(self.dim)
    }
    pub fn title_style(&self) -> Style {
        Style::new().fg(self.title).add_modifier(Modifier::BOLD)
    }
    pub fn header_style(&self) -> Style {
        Style::new().fg(self.header_fg).add_modifier(Modifier::BOLD)
    }
    pub fn selected_style(&self) -> Style {
        Style::new()
            .bg(self.selected_bg)
            .add_modifier(Modifier::BOLD)
    }
    pub fn decision_style(&self, d: pce::models::PolicyDecision) -> Style {
        use pce::models::PolicyDecision::*;
        match d {
            Allowed => Style::new().fg(self.ok),
            PotentiallyBlocked => Style::new().fg(self.warn),
            Blocked => Style::new().fg(self.bad),
            Unknown => Style::new().fg(self.dim),
        }
    }
}
