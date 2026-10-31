//! `:news` (#91): the release notes `illumio-tui news` prints, in an
//! overlay. The markdown is laid out plainly, no markdown engine:
//! `##` headings in the heading style, `-` bullets as `•` (`◦` nested),
//! `**` and backticks dropped, long lines wrapped at the box's width.
use crate::updates::{self, Release, Version};
use ratatui::layout::Rect;

/// What `:news` shows; the overlay's scroll is in `Overlay::News`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewsView {
    /// Newest first.
    pub releases: Vec<Release>,
    /// Where they came from when it matters: up to date, or GitHub
    /// unreachable.
    pub note: Option<String>,
}

/// Where the runtime reads releases: `base_url` `None` when the update
/// check is off (the cache alone, no request); `cache` `None` in tests.
/// Built empty, so no runtime but `app::run`'s ever asks GitHub.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpdateSource {
    pub base_url: Option<String>,
    pub cache: Option<std::path::PathBuf>,
}

impl UpdateSource {
    /// The startup check runs only with both.
    pub fn checks(&self) -> Option<(&str, &std::path::Path)> {
        Some((self.base_url.as_deref()?, self.cache.as_deref()?))
    }
}

/// `:news`' content: the releases newer than this binary (the cache, else
/// one fetch when the check is on), else this version's notes.
pub async fn load(source: &UpdateSource) -> NewsView {
    let current = Version::current();
    let newer = updates::newer_releases(source.base_url.as_deref(), source.cache.as_deref()).await;
    let releases = updates::select(None, current, &updates::embedded(), &newer.releases);
    let note = newer.warning.or_else(|| {
        newer
            .releases
            .is_empty()
            .then(|| format!("illumio-tui {current}: the notes of this version"))
    });
    NewsView { releases, note }
}

/// How a line is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// A release: `v0.5.9 — 2026-11-02`.
    Release,
    Heading,
    Text,
    Dim,
}

/// The box on a screen of `area`.
pub fn rect(area: Rect) -> Rect {
    crate::tui::ui::layout::centered(area, 100, area.height.saturating_sub(2))
}

/// Text rows inside the box on a `size` screen: (width, rows).
pub fn inner(size: (u16, u16)) -> (usize, usize) {
    let r = rect(Rect::new(0, 0, size.0, size.1));
    (
        r.width.saturating_sub(2) as usize,
        r.height.saturating_sub(2) as usize,
    )
}

/// `**` and backticks dropped.
fn plainer(text: &str) -> String {
    text.replace("**", "").replace('`', "")
}

/// Every line of `view` wrapped at `width`.
pub fn lines(view: &NewsView, width: usize) -> Vec<(LineKind, String)> {
    let width = width.max(10);
    let mut out: Vec<(LineKind, String)> = Vec::new();
    let mut push = |kind, indent: usize, first: &str, text: &str| {
        let rest = " ".repeat(indent);
        let wrapped = crate::tui::form::wrap_rows(text, width.saturating_sub(indent), usize::MAX);
        for (i, w) in wrapped.into_iter().enumerate() {
            let lead = if i == 0 { first } else { rest.as_str() };
            out.push((kind, format!("{lead}{w}")));
        }
    };
    if let Some(note) = &view.note {
        push(LineKind::Dim, 0, "", note);
        push(LineKind::Text, 0, "", "");
    }
    if view.releases.is_empty() && view.note.is_none() {
        push(LineKind::Dim, 0, "", "no release notes");
    }
    for (i, r) in view.releases.iter().enumerate() {
        if i > 0 {
            push(LineKind::Text, 0, "", "");
        }
        let title = match &r.date {
            Some(d) => format!("v{} — {d}", r.version),
            None => format!("v{}", r.version),
        };
        push(LineKind::Release, 0, "", &title);
        let mut fence = false;
        let mut blank = false;
        for line in r.body.lines() {
            if line.trim_start().starts_with("```") {
                fence = !fence;
                continue;
            }
            if fence {
                push(LineKind::Dim, 2, "  ", line.trim_end());
                continue;
            }
            let trimmed = line.trim_start();
            let depth = (line.len() - trimmed.len()) / 2;
            // One blank line at a time.
            if trimmed.is_empty() {
                if !blank {
                    push(LineKind::Text, 0, "", "");
                }
                blank = true;
                continue;
            }
            blank = false;
            if let Some(h) = trimmed.strip_prefix('#') {
                push(
                    LineKind::Heading,
                    0,
                    "",
                    &plainer(h.trim_start_matches('#').trim()),
                );
            } else if let Some(item) = trimmed
                .strip_prefix("- ")
                .or_else(|| trimmed.strip_prefix("* "))
            {
                let indent = 2 * depth;
                let bullet = format!(
                    "{}{} ",
                    " ".repeat(indent),
                    if depth == 0 { '•' } else { '◦' }
                );
                push(LineKind::Text, indent + 2, &bullet, &plainer(item));
            } else {
                let indent = 2 * depth;
                push(
                    LineKind::Text,
                    indent,
                    &" ".repeat(indent),
                    &plainer(trimmed),
                );
            }
        }
    }
    out
}

/// The last scroll position that still fills the box on a `size` screen.
pub fn last_scroll(view: &NewsView, size: (u16, u16)) -> usize {
    let (width, rows) = inner(size);
    lines(view, width).len().saturating_sub(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(body: &str) -> NewsView {
        NewsView {
            releases: vec![Release {
                version: "0.5.9".into(),
                date: Some("2026-11-02".into()),
                body: body.into(),
            }],
            note: None,
        }
    }

    #[test]
    fn markdown_is_laid_out_plainly() {
        let v = view(
            "## New\n\n- **News** (#91). `:news` shows it.\n  - nested item\n\n\n```\ncode\n```\nplain",
        );
        let l = lines(&v, 60);
        let text: Vec<&str> = l.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(
            text,
            [
                "v0.5.9 — 2026-11-02",
                "New",
                "",
                "• News (#91). :news shows it.",
                "  ◦ nested item",
                "",
                "  code",
                "plain"
            ]
        );
        assert_eq!(l[0].0, LineKind::Release);
        assert_eq!(l[1].0, LineKind::Heading);
        assert_eq!(l[6].0, LineKind::Dim);
    }

    #[test]
    fn long_bullets_wrap_under_their_text() {
        let v = view("- one two three four five six seven eight nine ten");
        let l = lines(&v, 20);
        assert_eq!(l[1].1, "• one two three four");
        assert!(l[2].1.starts_with("  five"), "{:?}", l[2]);
        assert!(l.iter().all(|(_, t)| t.chars().count() <= 20));
    }

    #[test]
    fn the_last_scroll_keeps_the_box_full() {
        let body = (0..100)
            .map(|i| format!("- item {i}\n"))
            .collect::<String>();
        let v = view(&body);
        let (_, rows) = inner((80, 24));
        assert_eq!(rows, 20);
        assert_eq!(last_scroll(&v, (80, 24)), lines(&v, 78).len() - rows);
        assert_eq!(last_scroll(&view("short"), (80, 24)), 0);
    }

    #[test]
    fn the_source_of_a_test_runtime_never_checks() {
        assert_eq!(UpdateSource::default().checks(), None);
        let half = UpdateSource {
            base_url: Some("http://x".into()),
            cache: None,
        };
        assert_eq!(half.checks(), None);
    }
}
