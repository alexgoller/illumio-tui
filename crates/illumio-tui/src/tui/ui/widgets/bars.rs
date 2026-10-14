//! Small dense-stat widgets in the btop spirit: a stacked decision bar, a
//! sparkline data helper and relative ages.
use crate::tui::ui::theme::Theme;
use pce::traffic::ResolvedFlow;
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

/// `▁▂▃▅█`: `counts` scaled to the largest, one cell each, at most `width`
/// (the newest buckets kept when they do not fit).
pub fn sparkline(counts: &[u64], width: usize) -> String {
    let max = counts.iter().copied().max().unwrap_or(0).max(1);
    let skip = counts.len().saturating_sub(width);
    counts
        .iter()
        .skip(skip)
        .map(|&v| spark_cell(v, max))
        .collect()
}

/// One sparkline cell: `v` scaled to `max` (at least 1).
pub fn spark_cell(v: u64, max: u64) -> char {
    let i = ((v * 7) / max.max(1)) as usize;
    ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'][i.min(7)]
}

/// Flow counts per time bucket by `last_detected`, oldest bucket first.
pub fn sparkline_buckets(flows: &[ResolvedFlow], buckets: usize) -> Vec<u64> {
    let n = buckets.max(1);
    let mut out = vec![0u64; n];
    let ts: Vec<i64> = flows
        .iter()
        .filter_map(|f| f.last_detected.as_deref())
        .filter_map(|s| s.parse::<jiff::Timestamp>().ok())
        .map(|t| t.as_second())
        .collect();
    let (Some(&min), Some(&max)) = (ts.iter().min(), ts.iter().max()) else {
        return out;
    };
    let span = (max - min).max(1);
    for t in ts {
        let i = (((t - min) * (n as i64 - 1)) / span) as usize;
        out[i.min(n - 1)] += 1;
    }
    out
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
    fn sparkline_buckets_by_last_detected() {
        let flows = crate::tui::test_support::fixture_flows();
        let b = sparkline_buckets(&flows, 4);
        assert_eq!(b.len(), 4);
        assert_eq!(
            b.iter().sum::<u64>(),
            4,
            "every fixture flow has last_detected"
        );
        assert!(b[3] >= 1, "the newest flow lands in the last bucket: {b:?}");
        assert!(sparkline_buckets(&[], 4).iter().all(|&x| x == 0));
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
