//! The Traffic screen's Volume view (#89): each row its name, weight, a
//! bar split by decision and its blocked share, laid out for the width the
//! table gets: the bar shrinks first, then names are cut with `…`.
use crate::tui::model::{Model, TrafficView, VolumeKind, VolumeRow, draft_short, is_gone};
use crate::tui::ui::theme::Theme;
use crate::tui::ui::widgets::panel::panel_spans;
use crate::tui::ui::widgets::table::{
    SpanningRow, clip_width, dims_row, header_meta, name_style, render_spanning_header,
};
use pce::models::PolicyDecision;
use pce::traffic::volume::{Weight, bar_cells};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;

/// A destination's indent before its name.
const INDENT: &str = "  → ";
/// The weight's cells (`1023K`, `16383P`: six at most).
const WEIGHT: usize = 6;
/// The least the bar shrinks to before names are cut (it takes the room
/// left otherwise, the width of the terminal); the least a name keeps.
const BAR_MIN: usize = 8;
const NAME_MIN: usize = 12;
/// The bar's glyphs by decision, so the split reads without colour too:
/// allowed, potentially blocked, blocked, unknown.
const GLYPHS: [&str; 4] = ["█", "▓", "▒", "░"];

/// The share column's title: by bytes it says the share is still of the
/// connections.
fn share_title(weight: Weight) -> &'static str {
    match weight {
        Weight::Connections => "blocked",
        Weight::Bytes => "blocked (conns)",
    }
}

fn width(s: &str) -> usize {
    Span::raw(s).width()
}

/// A row's name cell before fitting: a destination indented under its
/// source.
fn name_text(r: &VolumeRow) -> String {
    match r.kind {
        VolumeKind::Source { .. } => r.name(),
        VolumeKind::Dest => format!("{INDENT}{}", r.name()),
    }
}

/// `61% blocked · 12% pb` of the connections, whatever the weight
/// (blocked flows often report no bytes), then ` · draft +3 -1` while
/// draft decisions are loaded and the draft would change something.
pub fn share_text(r: &VolumeRow, draft: bool) -> String {
    let mut s = r.node.blocked_text();
    let d = draft_short(r.node.group.draft.as_ref());
    if draft && !d.is_empty() && d != "-" {
        s.push_str(&format!(" · draft {d}"));
    }
    s
}

/// The widths of one draw: the `Δ` cell's room, the name's, the bar's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VolumeLayout {
    pub delta: usize,
    pub name: usize,
    pub bar: usize,
}

impl VolumeLayout {
    /// The layout of `rows` in `avail` cells: name, weight and share always
    /// shown; the bar takes the room the longest name leaves, shrinking to
    /// `BAR_MIN` before names are cut, names cut no shorter than
    /// `NAME_MIN` before the bar goes.
    pub fn of(rows: &[VolumeRow], avail: usize, delta: bool, draft: bool) -> Self {
        let weight = rows.first().map_or(Weight::Connections, |r| r.weight);
        Self::fit(Self::measure(rows, draft, weight), avail, delta)
    }

    /// The longest name cell and share text of `rows` (the share's title
    /// at least): measured once a draw, whatever the width.
    pub fn measure(rows: &[VolumeRow], draft: bool, weight: Weight) -> (usize, usize) {
        let name = rows.iter().map(|r| width(&name_text(r))).max();
        let share = rows.iter().map(|r| width(&share_text(r, draft))).max();
        let title = width(share_title(weight));
        (name.unwrap_or(0), share.unwrap_or(0).max(title))
    }

    /// `of` from `measure`'s widths.
    pub fn fit((name_full, share): (usize, usize), avail: usize, delta: bool) -> Self {
        let delta = if delta { 5 } else { 0 };
        // Two cells between name, weight, bar and share.
        let rest = avail.saturating_sub(delta + WEIGHT + share + 6);
        let (name, bar) = if name_full + BAR_MIN <= rest {
            (name_full, rest - name_full)
        } else {
            let name = rest.saturating_sub(BAR_MIN).max(NAME_MIN.min(rest));
            (name, rest - name)
        };
        VolumeLayout { delta, name, bar }
    }

    /// The header: `group`, the weight's unit, `volume`, `blocked`, over
    /// their cells.
    pub fn header(&self, weight: Weight) -> String {
        let unit = match weight {
            Weight::Connections => "conns",
            Weight::Bytes => "bytes",
        };
        let mut s = String::new();
        if self.delta > 0 {
            s.push_str(&format!("{:<5}", crate::tui::model::DELTA_COLUMN));
        }
        s.push_str(&fit("group", self.name));
        s.push_str(&format!("  {unit:>WEIGHT$}  "));
        if self.bar > 0 {
            s.push_str(&fit("volume", self.bar));
            s.push_str("  ");
        }
        s.push_str(share_title(weight));
        s
    }

    /// One row as spans: `Δ`, the name (a source bold, a highlight's
    /// colour), the weight, the bar in the decisions' colours, the share.
    /// A gone row is dim throughout.
    pub fn spans(&self, r: &VolumeRow, draft: bool, t: &Theme) -> Vec<Span<'static>> {
        let marker = r
            .node
            .group
            .marks
            .as_ref()
            .and_then(crate::tui::traffic_marks::row_marker);
        // A gone row, or one whose every flow is accepted or reviewed, is
        // dim throughout, as in the other tables.
        let gone = is_gone(r.change()) || dims_row(marker);
        let dim = t.dim_style();
        let plain = if gone { dim } else { Style::new() };
        let mut out = Vec::new();
        if self.delta > 0 {
            let tag = r.change().map(|c| c.change.tag()).unwrap_or_default();
            out.push(Span::styled(format!("{tag:<5}"), plain));
        }
        let mut name = plain;
        if r.is_source() {
            name = name.add_modifier(Modifier::BOLD);
        }
        if let Some(s) = name_style(marker, t).filter(|_| !gone) {
            name = name.patch(s);
        }
        out.push(Span::styled(cut(&name_text(r), self.name), name));
        out.push(Span::styled(
            format!("  {:>WEIGHT$}  ", r.value_text()),
            plain,
        ));
        if self.bar > 0 {
            let cells = bar_cells(&r.split(), r.scale, self.bar);
            let decisions = [
                PolicyDecision::Allowed,
                PolicyDecision::PotentiallyBlocked,
                PolicyDecision::Blocked,
                PolicyDecision::Unknown,
            ];
            for ((n, glyph), d) in cells.iter().zip(GLYPHS).zip(decisions) {
                if *n > 0 {
                    let style = if gone { dim } else { t.decision_style(d) };
                    out.push(Span::styled(glyph.repeat(*n), style));
                }
            }
            let used: usize = cells.iter().sum();
            out.push(Span::raw(" ".repeat(self.bar - used + 2)));
        }
        let share = share_text(r, draft);
        let quiet = gone || r.node.split(Weight::Connections).blocked == 0;
        out.push(Span::styled(share, if quiet { dim } else { Style::new() }));
        out
    }
}

/// `s` padded or cut to `w` cells.
fn fit(s: &str, w: usize) -> String {
    let c = clip_width(s, w);
    let pad = w.saturating_sub(width(&c));
    format!("{c}{}", " ".repeat(pad))
}

/// `s` in `w` cells, cut with `…` where it does not fit.
fn cut(s: &str, w: usize) -> String {
    if width(s) <= w {
        fit(s, w)
    } else {
        fit(&format!("{}…", clip_width(s, w.saturating_sub(1))), w)
    }
}

/// The key hints after the weight.
const HINTS: &str = " · Enter unfolds/drills · z all";

/// `by conns · b bytes · Enter unfolds/drills · z all` (or `by bytes · b
/// conns · …`), then `extra` (the search, the marks' counts), dim like
/// every panel meta; the `b` hint fainter still when no flow carried
/// bytes, since `b` then refuses. The key hints only where all of it fits
/// `room` cells (`?` lists the keys): the weight and `extra` never push
/// the title out.
pub fn volume_meta(
    weight: Weight,
    has_bytes: bool,
    extra: &[String],
    room: usize,
    t: &Theme,
) -> Vec<Span<'static>> {
    let (by, toggle) = match weight {
        Weight::Connections => ("by conns", "b bytes"),
        Weight::Bytes => ("by bytes", "b conns"),
    };
    let dim = t.dim_style();
    let toggle_style = if has_bytes {
        dim
    } else {
        dim.add_modifier(Modifier::DIM)
    };
    let extra: String = extra.iter().map(|e| format!(" · {e}")).collect();
    let hints = width(" · ") + width(toggle) + width(HINTS);
    let long = width(by) + hints + width(&extra);
    let mut out = vec![Span::styled(by, dim)];
    if long <= room {
        out.push(Span::styled(" · ", dim));
        out.push(Span::styled(toggle, toggle_style));
        out.push(Span::styled(HINTS, dim));
    }
    // What does not fit is cut with `…`, never drawn over the title.
    let left = room.saturating_sub(width(by));
    let extra = if width(&extra) <= left {
        extra
    } else if left > width(" · …") {
        format!("{}…", clip_width(&extra, left - 1))
    } else {
        String::new()
    };
    if !extra.is_empty() {
        out.push(Span::styled(extra, dim));
    }
    out
}

pub fn render_volume(model: &Model, f: &mut Frame, area: Rect) {
    let t = &model.theme;
    let x = &model.traffic;
    let table = &x.volume;
    let st = &x.volume_state;
    // The search first: where the meta is cut, the marks' counts go first.
    let mut extra: Vec<String> = Vec::new();
    let search = header_meta(table);
    if !search.is_empty() {
        extra.push(search);
    }
    extra.extend(x.marks_meta(TrafficView::Volume));
    // Two corners, a space either side of the title and the meta, a gap.
    let title = TrafficView::Volume.title();
    let room = (area.width as usize).saturating_sub(width(title) + 7);
    let meta = volume_meta(st.weight, st.has_bytes, &extra, room, t);
    let block = panel_spans(title, meta, t, true);
    let (draft, delta) = (x.draft_loaded(), x.comparing());
    let measured = VolumeLayout::measure(table.rows(), draft, st.weight);
    let layout = |avail| VolumeLayout::fit(measured, avail, delta);
    let dim = t.dim_style();
    let row_style = |r: &VolumeRow| {
        if is_gone(r.change()) {
            dim
        } else {
            Style::new()
        }
    };
    render_spanning_header(
        f,
        area,
        table,
        t,
        block,
        &row_style,
        &|r, avail| -> Option<SpanningRow> {
            Some((layout(avail).spans(r, draft, t), row_style(r)))
        },
        &|avail| layout(avail).header(st.weight),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::test_support::model_with_rich_flows;

    fn meta_text(spans: &[Span]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn text(spans: &[Span]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn the_meta_dims_b_bytes_when_no_flow_has_bytes() {
        let t = Theme::dark();
        let with = volume_meta(Weight::Connections, true, &[], 80, &t);
        assert_eq!(
            meta_text(&with),
            "by conns · b bytes · Enter unfolds/drills · z all"
        );
        assert_eq!(with[2].content, "b bytes");
        assert_eq!(with[2].style, t.dim_style());
        let without = volume_meta(Weight::Connections, false, &[], 80, &t);
        assert_eq!(meta_text(&without), meta_text(&with));
        assert_eq!(without[2].style, t.dim_style().add_modifier(Modifier::DIM));
        assert_eq!(without[0].style, t.dim_style(), "only the b hint greys");
        assert_eq!(
            meta_text(&volume_meta(Weight::Bytes, true, &[], 80, &t)),
            "by bytes · b conns · Enter unfolds/drills · z all"
        );
    }

    #[test]
    fn the_key_hints_give_way_to_the_marks_and_the_search() {
        let t = Theme::dark();
        let extra = ["4 rows (9 flows) squelched · M shows them".to_string()];
        // 80 columns: the title takes 13, 67 left.
        let short = volume_meta(Weight::Connections, true, &extra, 67, &t);
        assert_eq!(
            meta_text(&short),
            "by conns · 4 rows (9 flows) squelched · M shows them"
        );
        let wide = volume_meta(Weight::Connections, true, &extra, 147, &t);
        assert!(
            meta_text(&wide).contains("z all · 4 rows"),
            "{}",
            meta_text(&wide)
        );
        let search = ["/crm".to_string()];
        let both = volume_meta(Weight::Bytes, true, &search, 40, &t);
        assert_eq!(meta_text(&both), "by bytes · /crm");
        // A long search with the marks' counts: cut at the room, never over
        // the title.
        let long = [
            "/a-very-long-search-term▏".to_string(),
            "4 rows (9 flows) squelched · M shows them".to_string(),
        ];
        for room in [0, 5, 12, 30, 67] {
            let m = meta_text(&volume_meta(Weight::Connections, true, &long, room, &t));
            assert!(width(&m) <= room.max(width("by conns")), "{room}: {m}");
            assert!(m.starts_with("by conns"), "{m}");
        }
        let m = meta_text(&volume_meta(Weight::Connections, true, &long, 67, &t));
        assert!(
            m.contains("/a-very-long-search-term") && m.ends_with('…'),
            "{m}"
        );
    }

    #[test]
    fn the_bar_shrinks_first_then_names_are_cut() {
        let mut m = model_with_rich_flows();
        m.traffic.fold_all_volume();
        let rows = m.traffic.volume.rows();
        let (name, share) = VolumeLayout::measure(rows, false, Weight::Connections);
        assert_eq!(name, width("  → api.partner.example/-"));
        // Wide: every name whole, the bar takes the rest of the width.
        let wide = VolumeLayout::of(rows, 200, false, false);
        let full = 200 - (name + WEIGHT + share + 6);
        assert_eq!((wide.name, wide.bar), (name, full));
        // Narrower: the bar gives way first ...
        let at = name + BAR_MIN + WEIGHT + share + 6;
        let l = VolumeLayout::of(rows, at + 5, false, false);
        assert_eq!((l.name, l.bar), (name, BAR_MIN + 5));
        let l = VolumeLayout::of(rows, at, false, false);
        assert_eq!((l.name, l.bar), (name, BAR_MIN));
        // ... then the names, down to `NAME_MIN`, then the bar goes.
        let l = VolumeLayout::of(rows, at - 4, false, false);
        assert_eq!((l.name, l.bar), (name - 4, BAR_MIN));
        let l = VolumeLayout::of(rows, NAME_MIN + WEIGHT + share + 6 + 3, false, false);
        assert_eq!((l.name, l.bar), (NAME_MIN, 3));
        // The `Δ` cell takes five cells from the same room.
        let d = VolumeLayout::of(rows, 200, true, false);
        assert_eq!((d.delta, d.name, d.bar), (5, name, full - 5));
        // Every row fits its width exactly but for the share.
        let t = Theme::dark();
        for avail in [40, 76, 120, 156] {
            let l = VolumeLayout::of(rows, avail, false, false);
            for r in rows {
                let line = text(&l.spans(r, false, &t));
                assert!(
                    width(&line) <= avail.max(l.name + WEIGHT + share + 6),
                    "{avail}: {line}"
                );
                assert!(line.contains(&r.value_text()), "{line}");
                assert!(line.contains("% blocked"), "{line}");
            }
            assert!(width(&l.header(Weight::Connections)) <= avail.max(40));
        }
        let narrow = VolumeLayout::of(rows, 40, false, false);
        let cut: Vec<String> = rows
            .iter()
            .map(|r| text(&narrow.spans(r, false, &t)))
            .filter(|l| l.contains('…'))
            .collect();
        assert!(!cut.is_empty(), "names cut at 40");
    }

    #[test]
    fn a_row_reads_name_weight_bar_and_share() {
        let mut m = model_with_rich_flows();
        m.traffic.fold_all_volume();
        let t = Theme::dark();
        let rows = m.traffic.volume.rows();
        let l = VolumeLayout::of(rows, 120, false, false);
        let corp = text(&l.spans(&rows[0], false, &t));
        assert!(corp.starts_with("ipl:Corp/-"), "{corp}");
        assert!(corp.contains("  1.3K  "), "{corp}");
        let dst = text(&l.spans(&rows[1], false, &t));
        assert!(dst.starts_with("  → "), "{dst}");
        assert!(l.header(Weight::Connections).starts_with("group"));
        assert!(l.header(Weight::Bytes).contains("bytes  volume"));
        assert!(l.header(Weight::Bytes).ends_with("blocked (conns)"));
        assert!(
            l.header(Weight::Connections).ends_with("volume  blocked")
                || l.header(Weight::Connections).ends_with("blocked")
        );
        // The bar's segments are in the decisions' colours.
        let spans = l.spans(&rows[0], false, &t);
        let blocked = spans.iter().find(|s| s.content.starts_with('▒')).unwrap();
        assert_eq!(blocked.style, t.decision_style(PolicyDecision::Blocked));
    }

    #[test]
    fn by_bytes_the_share_stays_the_connections() {
        let mut m = model_with_rich_flows();
        let t = Theme::dark();
        assert!(m.traffic.toggle_volume_weight());
        let rows = m.traffic.volume.rows();
        let corp = rows.iter().find(|r| r.name() == "ipl:Corp/-").unwrap();
        assert_eq!(share_text(corp, false), "1% blocked · 6% pb");
        let l = VolumeLayout::of(rows, 120, false, false);
        assert!(text(&l.spans(corp, false, &t)).ends_with("1% blocked · 6% pb"));
        assert!(l.header(Weight::Bytes).ends_with("blocked (conns)"));
    }
}
