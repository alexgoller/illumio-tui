//! `cargo xtask changelog [--check]`: renders `docs/changelog.html`, the
//! site's changelog page, from `CHANGELOG.md` at the repository root.
//!
//! The Markdown converter covers exactly what the release notes use:
//! headings, nested `-` lists, paragraphs, `> ` quotes, fenced code blocks,
//! `` `code` ``, `**bold**`, `*italic*`, `[text](url)` links, bare
//! `http(s)://` URLs and `#NN` issue references. Everything else is text,
//! HTML-escaped. No dependencies, so the output is stable across builds.
use std::fmt::Write as _;
use std::path::PathBuf;

const ISSUES: &str = "https://github.com/alexgoller/illumio-tui/issues/";
const OUTPUT: &str = "docs/changelog.html";

/// The repository root: two levels above this crate's manifest.
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Writes `docs/changelog.html`, or with `check` compares it and exits 1
/// when it is stale.
pub fn run(check: bool) {
    let root = root();
    let md = std::fs::read_to_string(root.join("CHANGELOG.md")).expect("read CHANGELOG.md");
    let html = render_page(&md);
    let out = root.join(OUTPUT);
    if check {
        let current = std::fs::read_to_string(&out).unwrap_or_default();
        if current.replace("\r\n", "\n") != html {
            eprintln!("{OUTPUT} is stale: run `cargo xtask changelog` and commit it");
            std::process::exit(1);
        }
        println!("{OUTPUT} is up to date");
    } else {
        std::fs::write(&out, &html).unwrap_or_else(|e| panic!("write {OUTPUT}: {e}"));
        println!("wrote {OUTPUT}");
    }
}

/// One `## ` section of CHANGELOG.md: `## v0.5.3 — 2026-10-23`.
#[derive(Debug, PartialEq)]
struct Release {
    id: String,
    title: String,
    date: Option<String>,
    body: String,
}

/// Splits CHANGELOG.md into the intro (without the `# ` title) and its
/// `## ` sections. Headings inside fenced code are not sections.
fn split(md: &str) -> (String, Vec<Release>) {
    let mut intro = String::new();
    let mut releases: Vec<Release> = Vec::new();
    let mut fence = false;
    for line in md.lines() {
        if line.trim_start().starts_with("```") {
            fence = !fence;
        }
        if !fence && let Some(heading) = line.strip_prefix("## ") {
            let (title, date) = match heading.split_once(" — ") {
                Some((t, d)) => (t.trim().to_string(), Some(d.trim().to_string())),
                None => (heading.trim().to_string(), None),
            };
            releases.push(Release {
                id: slug(&title),
                title,
                date,
                body: String::new(),
            });
            continue;
        }
        let target = match releases.last_mut() {
            Some(r) => &mut r.body,
            None if !fence && line.starts_with("# ") => continue,
            None => &mut intro,
        };
        target.push_str(line);
        target.push('\n');
    }
    (intro, releases)
}

/// An anchor id: `v0.5.3` stays as it is; other text becomes lowercase
/// letters, digits, `.` and `-`.
fn slug(title: &str) -> String {
    let mut s = String::new();
    for c in title.chars() {
        if c.is_ascii_alphanumeric() || c == '.' {
            s.push(c.to_ascii_lowercase());
        } else if !s.ends_with('-') && !s.is_empty() {
            s.push('-');
        }
    }
    s.trim_end_matches('-').to_string()
}

/// HTML-escapes text for element content and attribute values.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

/// Finds `pat` in `s` from byte `from`, skipping over `` `code` `` spans.
fn find_outside_code(s: &str, from: usize, pat: &str) -> Option<usize> {
    let mut i = from;
    while i < s.len() {
        let rest = &s[i..];
        if rest.starts_with(pat) {
            return Some(i);
        }
        if rest.starts_with('`')
            && let Some(end) = rest[1..].find('`')
        {
            i += end + 2;
            continue;
        }
        i += rest.chars().next().map_or(1, char::len_utf8);
    }
    None
}

/// Renders the inline Markdown of one paragraph, list item or heading.
/// Inside link text (`in_link`), no further links are made.
fn inline(s: &str, in_link: bool) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < s.len() {
        let rest = &s[i..];
        let prev = s[..i].chars().next_back();
        // `code`: literal, escaped, nothing else applies inside.
        if let Some(r) = rest.strip_prefix('`')
            && let Some(end) = r.find('`')
        {
            let _ = write!(out, "<code>{}</code>", esc(&r[..end]));
            i += end + 2;
            continue;
        }
        // **bold**
        if let Some(r) = rest.strip_prefix("**")
            && let Some(end) = find_outside_code(r, 0, "**")
            && end > 0
        {
            let _ = write!(out, "<strong>{}</strong>", inline(&r[..end], in_link));
            i += end + 4;
            continue;
        }
        // *italic*: opens before a non-space, closes after one.
        if let Some(r) = rest.strip_prefix('*')
            && r.starts_with(|c: char| !c.is_whitespace() && c != '*')
            && let Some(end) = find_outside_code(r, 0, "*")
            && !r[..end].ends_with(char::is_whitespace)
        {
            let _ = write!(out, "<em>{}</em>", inline(&r[..end], in_link));
            i += end + 2;
            continue;
        }
        if !in_link {
            // [text](url)
            if let Some(r) = rest.strip_prefix('[')
                && let Some(mid) = find_outside_code(r, 0, "](")
                && let Some(close) = r[mid + 2..].find(')')
            {
                let url = &r[mid + 2..mid + 2 + close];
                let _ = write!(
                    out,
                    "<a href=\"{}\">{}</a>",
                    esc(url),
                    inline(&r[..mid], true)
                );
                i += 1 + mid + 2 + close + 1;
                continue;
            }
            // A bare URL, without trailing punctuation.
            let boundary = prev.is_none_or(|c| !c.is_alphanumeric() && c != '/');
            if boundary && (rest.starts_with("https://") || rest.starts_with("http://")) {
                let end = rest
                    .find(|c: char| c.is_whitespace() || c == '<' || c == '>')
                    .unwrap_or(rest.len());
                let url = rest[..end].trim_end_matches(['.', ',', ';', ':', ')', '!', '?']);
                let _ = write!(out, "<a href=\"{0}\">{0}</a>", esc(url));
                i += url.len();
                continue;
            }
            // #NN: an issue or pull request of this repository.
            let word_start = prev.is_none_or(|c| !c.is_alphanumeric() && c != '&' && c != '#');
            if word_start && let Some(r) = rest.strip_prefix('#') {
                let digits = r.len() - r.trim_start_matches(|c: char| c.is_ascii_digit()).len();
                let after = r[digits..].chars().next();
                if digits > 0 && after.is_none_or(|c| !c.is_alphanumeric() && c != '_') {
                    let n = &r[..digits];
                    let _ = write!(out, "<a href=\"{ISSUES}{n}\">#{n}</a>");
                    i += 1 + digits;
                    continue;
                }
            }
        }
        let c = rest.chars().next().expect("non-empty");
        out.push_str(&esc(c.encode_utf8(&mut [0; 4])));
        i += c.len_utf8();
    }
    out
}

/// Block-level state of the converter.
#[derive(Default)]
struct Blocks {
    out: String,
    /// Indents of the open `<ul>`s, outermost first; each has an open `<li>`.
    lists: Vec<usize>,
    /// Text lines of the paragraph (or list item text) being collected.
    para: Vec<String>,
    /// Lines of the `> ` quote being collected.
    quote: Vec<String>,
}

impl Blocks {
    fn flush_para(&mut self) {
        if self.para.is_empty() {
            return;
        }
        let text = inline(&self.para.join(" "), false);
        self.para.clear();
        if self.lists.is_empty() {
            let _ = writeln!(self.out, "<p>{text}</p>");
        } else {
            self.out.push_str(&text);
        }
    }

    fn flush_quote(&mut self) {
        if self.quote.is_empty() {
            return;
        }
        let text = inline(&self.quote.join(" "), false);
        self.quote.clear();
        let _ = writeln!(self.out, "<blockquote><p>{text}</p></blockquote>");
    }

    /// Closes the lists nested deeper than `indent` (all with `None`).
    fn close_lists(&mut self, indent: Option<usize>) {
        while let Some(&top) = self.lists.last() {
            if indent.is_some_and(|i| top <= i) {
                break;
            }
            self.out.push_str("</li>\n</ul>\n");
            self.lists.pop();
        }
    }
}

/// Converts the Markdown subset the release notes use to HTML.
pub fn markdown_to_html(md: &str) -> String {
    let mut b = Blocks::default();
    // The fence's own indent, stripped from the lines inside it.
    let mut fence: Option<usize> = None;
    for line in md.lines() {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        if let Some(fi) = fence {
            if trimmed.starts_with("```") {
                b.out.push_str("</code></pre>\n");
                fence = None;
            } else {
                let strip = line.len() - line.trim_start_matches(' ').len();
                let _ = writeln!(b.out, "{}", esc(&line[strip.min(fi)..]));
            }
            continue;
        }
        if trimmed.is_empty() {
            b.flush_para();
            b.flush_quote();
            continue;
        }
        if let Some(lang) = trimmed.strip_prefix("```") {
            b.flush_para();
            b.flush_quote();
            // The fence belongs to the deepest item indented less than it.
            b.close_lists(indent.checked_sub(1));
            let lang = lang.trim();
            if lang.is_empty() {
                b.out.push_str("<pre><code>");
            } else {
                let _ = write!(b.out, "<pre><code class=\"language-{}\">", esc(lang));
            }
            fence = Some(indent);
            continue;
        }
        if let Some(item) = trimmed.strip_prefix("- ") {
            b.flush_para();
            b.flush_quote();
            b.close_lists(Some(indent));
            match b.lists.last() {
                Some(&top) if top == indent => b.out.push_str("</li>\n<li>"),
                _ => {
                    b.out.push_str(if b.lists.is_empty() {
                        "<ul>\n<li>"
                    } else {
                        "\n<ul>\n<li>"
                    });
                    b.lists.push(indent);
                }
            }
            b.para.push(item.trim().to_string());
            continue;
        }
        if indent > 0 && !b.lists.is_empty() {
            // A continuation line of the deepest item indented less than it.
            if b.lists.last().is_some_and(|&top| top >= indent) {
                b.flush_para();
                b.close_lists(Some(indent - 1));
            }
            b.para.push(trimmed.trim_end().to_string());
            continue;
        }
        if !b.lists.is_empty() {
            b.flush_para();
            b.close_lists(None);
        }
        if let Some(q) = trimmed.strip_prefix('>') {
            b.flush_para();
            b.quote.push(q.trim().to_string());
            continue;
        }
        b.flush_quote();
        let hashes = trimmed.len() - trimmed.trim_start_matches('#').len();
        if (1..=6).contains(&hashes) && trimmed[hashes..].starts_with(' ') {
            b.flush_para();
            let text = inline(trimmed[hashes..].trim(), false);
            let _ = writeln!(b.out, "<h{hashes}>{text}</h{hashes}>");
            continue;
        }
        b.para.push(trimmed.trim_end().to_string());
    }
    if fence.is_some() {
        b.out.push_str("</code></pre>\n");
    }
    b.flush_para();
    b.flush_quote();
    b.close_lists(None);
    b.out
}

const NAV: &[(&str, &str)] = &[
    ("install", "1"),
    ("screens", "2"),
    ("workflows", "3"),
    ("start", "4"),
    ("cli", "5"),
    ("recipes", "6"),
    ("writes", "7"),
    ("configure", "8"),
    ("protection", "9"),
    ("roadmap", "10"),
];

/// The whole `docs/changelog.html`: the site's head, header and footer,
/// the intro with a table of contents, and one panel per release.
pub fn render_page(md: &str) -> String {
    let (intro, releases) = split(md);
    let mut h = String::new();
    h.push_str(
        r##"<!doctype html>
<!-- Generated from CHANGELOG.md by `cargo xtask changelog`. Do not edit. -->
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Changelog · illumio-tui</title>
<meta name="description" content="Every illumio-tui release, newest first: what is new, what changed and what was fixed.">
<meta name="theme-color" content="#0e1622">
<link rel="canonical" href="https://alexgoller.github.io/illumio-tui/changelog.html">
<link rel="icon" href="assets/mark.svg" type="image/svg+xml">
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=IBM+Plex+Sans:ital,wght@0,400;0,600;1,400&amp;family=Martian+Mono:wdth,wght@75..112.5,400..800&amp;display=swap">
<link rel="stylesheet" href="assets/style.css">
</head>
<body>
<a class="skip" href="#main">Skip to content</a>

<header class="topbar">
  <div class="wrap">
    <a class="brand" href="./"><img src="assets/mark.svg" alt="" width="20" height="20">illumio-tui</a>
    <nav aria-label="Sections">
"##,
    );
    for (id, key) in NAV {
        let _ = writeln!(h, "      <a href=\"./#{id}\"><kbd>{key}</kbd>{id}</a>");
    }
    h.push_str("      <a href=\"changelog.html\" aria-current=\"page\">changelog</a>\n");
    h.push_str("    </nav>\n  </div>\n</header>\n\n<main id=\"main\">\n<div class=\"wrap\">\n\n");

    h.push_str(
        "<section class=\"panel changelog-intro\" aria-labelledby=\"changelog-h\">\n  \
         <h2 id=\"changelog-h\">Changelog</h2>\n",
    );
    h.push_str(&markdown_to_html(&intro));
    h.push_str("<p><a href=\"./\">&larr; Back to the site</a></p>\n");
    h.push_str("<nav class=\"toc\" aria-label=\"Releases\">\n<ul>\n");
    for r in &releases {
        let date = r
            .date
            .as_deref()
            .map(|d| format!(" <span>{}</span>", esc(d)))
            .unwrap_or_default();
        let _ = writeln!(
            h,
            "<li><a href=\"#{}\">{}</a>{date}</li>",
            esc(&r.id),
            esc(&r.title)
        );
    }
    h.push_str("</ul>\n</nav>\n</section>\n\n");

    for r in &releases {
        let id = esc(&r.id);
        let date = r
            .date
            .as_deref()
            .map(|d| format!(" <span class=\"date\">{}</span>", esc(d)))
            .unwrap_or_default();
        let _ = writeln!(
            h,
            "<section class=\"panel release\" id=\"{id}\" aria-labelledby=\"{id}-h\">\n  \
             <h2 id=\"{id}-h\"><a href=\"#{id}\">{}</a>{date}</h2>",
            esc(&r.title)
        );
        h.push_str(&markdown_to_html(&r.body));
        h.push_str("</section>\n\n");
    }

    h.push_str(
        r#"<p class="changelog-back"><a href="./">&larr; Back to the site</a> · <a href="https://github.com/alexgoller/illumio-tui/releases">GitHub Releases</a></p>

</div>
</main>

<footer>
  <div class="wrap">
    <span>illumio-tui, GPL-3.0-only</span>
    <span>By Alex Goller. Built with Claude Code.</span>
  </div>
</footer>
</body>
</html>
"#,
    );
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_html_in_text_and_code() {
        assert_eq!(
            markdown_to_html("a <b> & \"c\" `<path>`"),
            "<p>a &lt;b&gt; &amp; &quot;c&quot; <code>&lt;path&gt;</code></p>\n"
        );
    }

    #[test]
    fn headings_paragraphs_and_line_joins() {
        let html = markdown_to_html("### New\n\nOne line\nand the next.\n\n#### Small\n#nope");
        assert_eq!(
            html,
            "<h3>New</h3>\n<p>One line and the next.</p>\n<h4>Small</h4>\n<p>#nope</p>\n"
        );
    }

    #[test]
    fn bold_italic_code_and_links() {
        let html = inline(
            "**`:export <p>` in the TUI (#15).** *More info* [v0.1.12](https://x/y?a=1&b=2)",
            false,
        );
        assert_eq!(
            html,
            "<strong><code>:export &lt;p&gt;</code> in the TUI (<a href=\"https://github.com/alexgoller/illumio-tui/issues/15\">#15</a>).</strong> \
             <em>More info</em> <a href=\"https://x/y?a=1&amp;b=2\">v0.1.12</a>"
        );
    }

    #[test]
    fn issue_references_only_as_words() {
        let link = |n: &str| format!("<a href=\"{ISSUES}{n}\">#{n}</a>");
        assert_eq!(
            inline("#7, (#42) #5.", false),
            format!("{}, ({}) {}.", link("7"), link("42"), link("5"))
        );
        // Not inside words, code, link text, or before letters.
        assert_eq!(
            inline("skills#1 `#3` #4a #x", false),
            "skills#1 <code>#3</code> #4a #x"
        );
        assert_eq!(
            inline("[winget-pkgs #456974](https://u)", false),
            "<a href=\"https://u\">winget-pkgs #456974</a>"
        );
    }

    #[test]
    fn bare_urls_without_trailing_punctuation() {
        assert_eq!(
            inline(
                "**Full Changelog**: https://github.com/a/b/compare/v1...v2.",
                false
            ),
            "<strong>Full Changelog</strong>: <a href=\"https://github.com/a/b/compare/v1...v2\">https://github.com/a/b/compare/v1...v2</a>."
        );
    }

    #[test]
    fn asterisks_that_are_not_emphasis() {
        assert_eq!(inline("a * b and 2*3", false), "a * b and 2*3");
        assert_eq!(
            inline("`!agent.*` and *", false),
            "<code>!agent.*</code> and *"
        );
    }

    #[test]
    fn nested_lists_with_continuations() {
        let md = "- one\n  - one.a\n    more\n  - one.b\n- two\n\nAfter.";
        assert_eq!(
            markdown_to_html(md),
            "<ul>\n<li>one\n<ul>\n<li>one.a more</li>\n<li>one.b</li>\n</ul>\n</li>\n<li>two</li>\n</ul>\n<p>After.</p>\n"
        );
    }

    #[test]
    fn fenced_code_inside_a_list_item() {
        let md = "- Stored:\n    ```toml\n    [a]\n    b = \"<c>\"\n\n    d = 1\n    ```\n  - next";
        assert_eq!(
            markdown_to_html(md),
            "<ul>\n<li>Stored:<pre><code class=\"language-toml\">[a]\nb = &quot;&lt;c&gt;&quot;\n\nd = 1\n</code></pre>\n\n<ul>\n<li>next</li>\n</ul>\n</li>\n</ul>\n"
        );
    }

    #[test]
    fn fenced_code_at_top_level_closes_lists() {
        let md = "- a\n```\n# not a heading\n```\n";
        assert_eq!(
            markdown_to_html(md),
            "<ul>\n<li>a</li>\n</ul>\n<pre><code># not a heading\n</code></pre>\n"
        );
    }

    #[test]
    fn quotes() {
        assert_eq!(
            markdown_to_html("> **Superseded by [v2](https://u):** use v2.\n\nNext."),
            "<blockquote><p><strong>Superseded by <a href=\"https://u\">v2</a>:</strong> use v2.</p></blockquote>\n<p>Next.</p>\n"
        );
    }

    #[test]
    fn splits_releases_and_ignores_headings_in_code() {
        let md = "# Changelog\n\nIntro.\n\n## v0.2.0 — 2026-10-16\n\n### New\n```\n## v9 not a release\n```\n## v0.1.0 — 2026-10-12\n\nFirst.\n";
        let (intro, rs) = split(md);
        assert_eq!(intro, "\nIntro.\n\n");
        assert_eq!(rs.len(), 2);
        assert_eq!(rs[0].id, "v0.2.0");
        assert_eq!(rs[0].date.as_deref(), Some("2026-10-16"));
        assert!(rs[0].body.contains("## v9 not a release"));
        assert_eq!(
            rs[1],
            Release {
                id: "v0.1.0".into(),
                title: "v0.1.0".into(),
                date: Some("2026-10-12".into()),
                body: "\nFirst.\n".into()
            }
        );
        assert_eq!(slug("Unreleased changes!"), "unreleased-changes");
    }

    #[test]
    fn page_has_toc_anchors_and_the_site_frame() {
        let page = render_page(
            "# Changelog\n\nIntro.\n\n## v0.5.3 — 2026-10-23\n\n### New\n\n- x (#70)\n",
        );
        assert!(page.contains("<link rel=\"stylesheet\" href=\"assets/style.css\">"));
        assert!(page.contains("<li><a href=\"#v0.5.3\">v0.5.3</a> <span>2026-10-23</span></li>"));
        assert!(page.contains("<section class=\"panel release\" id=\"v0.5.3\""));
        assert!(page.contains("<h3>New</h3>"));
        assert!(page.contains("issues/70\">#70</a>"));
        assert!(page.contains("<a href=\"./\">&larr; Back to the site</a>"));
        assert!(page.contains("<a href=\"./#roadmap\"><kbd>10</kbd>roadmap</a>"));
    }

    /// The committed page is what the committed CHANGELOG.md renders to
    /// (the same check as `cargo xtask changelog --check`).
    #[test]
    fn committed_page_is_current() {
        let root = root();
        let md = std::fs::read_to_string(root.join("CHANGELOG.md")).unwrap();
        let html = std::fs::read_to_string(root.join(OUTPUT)).unwrap();
        assert!(
            html.replace("\r\n", "\n") == render_page(&md),
            "{OUTPUT} is stale: run `cargo xtask changelog`"
        );
    }
}
