//! Small dense-stat widgets in the btop spirit: a stacked decision bar and
//! relative ages.
use crate::tui::ui::theme::Theme;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use std::time::Duration;

pub fn age_text(age: Duration) -> String {
    let s = age.as_secs();
    if s < 60 {
        format!("{s}s ago")
    } else if s < 3600 {
        format!("{}m ago", s / 60)
    } else {
        format!("{}h ago", s / 3600)
    }
}

/// A `width`-cell bar split proportionally: allowed, potentially blocked, blocked.
pub fn decision_bar(
    allowed: u64,
    pot: u64,
    blocked: u64,
    width: usize,
    t: &Theme,
) -> Line<'static> {
    let total = allowed + pot + blocked;
    if total == 0 || width == 0 {
        return Line::from(Span::styled("░".repeat(width), t.dim_style()));
    }
    let w = width as u64;
    let mut a = (allowed * w / total) as usize;
    let mut p = (pot * w / total) as usize;
    let mut b = (blocked * w / total) as usize;
    // Give rounding leftovers to the largest non-zero segment so the bar fills.
    let mut rest = width.saturating_sub(a + p + b);
    for (count, seg) in [(allowed, &mut a), (pot, &mut p), (blocked, &mut b)] {
        if count > 0 && rest > 0 {
            *seg += rest;
            rest = 0;
        }
    }
    Line::from(vec![
        Span::styled("█".repeat(a), Style::new().fg(t.ok)),
        Span::styled("█".repeat(p), Style::new().fg(t.warn)),
        Span::styled("█".repeat(b), Style::new().fg(t.bad)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn age_text_scales() {
        assert_eq!(age_text(Duration::from_secs(3)), "3s ago");
        assert_eq!(age_text(Duration::from_secs(125)), "2m ago");
        assert_eq!(age_text(Duration::from_secs(7_300)), "2h ago");
    }

    #[test]
    fn decision_bar_widths_sum_to_width() {
        let t = crate::tui::ui::theme::Theme::dark();
        let line = decision_bar(10, 5, 5, 20, &t);
        let total: usize = line.spans.iter().map(|s| s.content.chars().count()).sum();
        assert_eq!(total, 20);
        let empty = decision_bar(0, 0, 0, 10, &t);
        assert_eq!(
            empty
                .spans
                .iter()
                .map(|s| s.content.chars().count())
                .sum::<usize>(),
            10
        );
    }
}
