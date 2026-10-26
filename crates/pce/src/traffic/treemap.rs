//! A squarified treemap of label-pair groups, in terminal cells: outer
//! tiles per source group, inner tiles per destination. Pure; no requests.
use super::aggregate::{Dominant, FlowGroup, dominant};
use crate::models::PolicyDecision;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

impl Rect {
    pub fn area(self) -> u32 {
        u32::from(self.w) * u32::from(self.h)
    }

    pub fn is_empty(self) -> bool {
        self.w == 0 || self.h == 0
    }

    /// `self` minus a one-cell border; empty when that leaves nothing.
    pub fn inner(self) -> Rect {
        if self.w < 3 || self.h < 3 {
            return Rect::default();
        }
        Rect {
            x: self.x + 1,
            y: self.y + 1,
            w: self.w - 2,
            h: self.h - 2,
        }
    }

    pub fn intersects(self, o: Rect) -> bool {
        self.x < o.x + o.w && o.x < self.x + self.w && self.y < o.y + o.h && o.y < self.y + self.h
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Weight {
    #[default]
    Connections,
    Bytes,
}

impl Weight {
    pub fn of(self, g: &FlowGroup) -> u64 {
        match self {
            Weight::Connections => g.connections,
            Weight::Bytes => g.bytes,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tile {
    /// Group text, `shop/prod`, or `other (N)` for N folded tiles.
    pub label: String,
    /// Connections or bytes.
    pub weight: u64,
    /// x, y, w, h in cells, in the coordinates of the `area` passed in.
    pub rect: Rect,
    pub decision: Dominant,
    /// Destinations, laid out inside `rect.inner()` (none when too small).
    pub children: Vec<Tile>,
}

impl Tile {
    /// Whether this tile stands for several folded ones.
    pub fn is_other(&self) -> bool {
        self.label.starts_with("other (")
    }
}

/// Squarified treemap of `groups` (from `group_by_labels`) into `area`:
/// outer tiles per src, inner per dst. Tiles narrower than 1 cell are
/// dropped and their weight is folded into an `other (N)` tile. `other` is
/// placed with a weight floored at one cell's worth of the total, so a
/// dominant source cannot squeeze it out (its `Tile.weight` stays the true
/// folded sum); a named tile joins `other` only when it gets no cell itself.
/// Zero total weight gives no tiles.
pub fn layout(groups: &[FlowGroup], area: Rect, weight: Weight) -> Vec<Tile> {
    let mut sources: Vec<(String, Vec<&FlowGroup>)> = Vec::new();
    for g in groups {
        let label = g.src.join("/");
        match sources.iter_mut().find(|(l, _)| *l == label) {
            Some((_, v)) => v.push(g),
            None => sources.push((label, vec![g])),
        }
    }
    let items: Vec<Item> = sources
        .into_iter()
        .map(|(label, gs)| {
            let (a, p, b) = gs.iter().fold((0, 0, 0), |(a, p, b), g| {
                (a + g.allowed, p + g.potentially_blocked, b + g.blocked)
            });
            Item {
                label,
                weight: sum(gs.iter().map(|g| weight.of(g))),
                decision: dominant(a, p, b),
                children: gs.iter().map(|g| dst_item(g, weight)).collect(),
            }
        })
        .collect();
    place(items, area)
        .into_iter()
        .map(|(item, rect)| {
            let inner = rect.inner();
            let children = if item.children.is_empty() || inner.is_empty() {
                Vec::new()
            } else {
                place(item.children, inner)
                    .into_iter()
                    .map(|(c, r)| leaf(c, r))
                    .collect()
            };
            Tile {
                label: item.label,
                weight: item.weight,
                rect,
                decision: item.decision,
                children,
            }
        })
        .collect()
}

/// The destinations of the source group `src` as top-level tiles (no
/// children): the treemap zoomed in on one source.
pub fn layout_zoomed(
    groups: &[FlowGroup],
    src: &[String],
    area: Rect,
    weight: Weight,
) -> Vec<Tile> {
    let items: Vec<Item> = groups
        .iter()
        .filter(|g| g.src == src)
        .map(|g| dst_item(g, weight))
        .collect();
    place(items, area)
        .into_iter()
        .map(|(c, r)| leaf(c, r))
        .collect()
}

struct Item {
    label: String,
    weight: u64,
    decision: Dominant,
    children: Vec<Item>,
}

fn dst_item(g: &FlowGroup, weight: Weight) -> Item {
    Item {
        label: g.dst.join("/"),
        weight: weight.of(g),
        decision: g.dominant(),
        children: Vec::new(),
    }
}

fn leaf(item: Item, rect: Rect) -> Tile {
    Tile {
        label: item.label,
        weight: item.weight,
        rect,
        decision: item.decision,
        children: Vec::new(),
    }
}

fn sum(weights: impl Iterator<Item = u64>) -> u64 {
    weights.fold(0, u64::saturating_add)
}

/// Lays `items` out in `area`: zero weights dropped, heaviest first (then
/// by label; equal keys keep their order), and the tail that does not get
/// a whole cell folded into `other (N)`, last. When `other` itself gets no
/// cell even with its floored weight, the floor doubles; a named item that
/// gets no cell folds into `other` with all after it (in the end `other`
/// alone fills the area).
fn place(items: Vec<Item>, area: Rect) -> Vec<(Item, Rect)> {
    if area.is_empty() {
        return Vec::new();
    }
    let mut items: Vec<Item> = items.into_iter().filter(|i| i.weight > 0).collect();
    if items.is_empty() {
        return Vec::new();
    }
    items.sort_by(|a, b| b.weight.cmp(&a.weight).then_with(|| a.label.cmp(&b.label)));
    let n = items.len();
    let total = sum(items.iter().map(|i| i.weight));
    let cells_in_area = u128::from(area.area());
    // One cell's worth of weight; doubled should `other` still get no cell.
    let mut floor = u64::try_from(u128::from(total).div_ceil(cells_in_area))
        .unwrap_or(u64::MAX)
        .max(1);
    let mut kept = n;
    let rects = loop {
        let mut weights: Vec<u64> = items[..kept].iter().map(|i| i.weight).collect();
        if kept < n {
            weights.push(sum(items[kept..].iter().map(|i| i.weight)).max(floor));
        }
        if kept == 0 {
            break vec![area];
        }
        let rects: Vec<Rect> = squarify(&weights, f64::from(area.w), 2.0 * f64::from(area.h))
            .into_iter()
            .map(|e| cells(e, area))
            .collect();
        match rects.iter().position(|r| r.is_empty()) {
            None => break rects,
            Some(i) if i < kept => kept = i,
            Some(_) => floor = floor.saturating_mul(2),
        }
    };
    let rest: Vec<Item> = items.split_off(kept);
    let mut out: Vec<(Item, Rect)> = items.into_iter().zip(rects.iter().copied()).collect();
    if !rest.is_empty() {
        let other = Item {
            label: format!("other ({})", rest.len()),
            weight: sum(rest.iter().map(|i| i.weight)),
            decision: PolicyDecision::Unknown,
            children: Vec::new(),
        };
        out.push((other, rects[kept]));
    }
    out
}

/// `[x0, y0, x1, y1]` in visual units: x in cells, y in half cells, so a
/// cell (about 1:2) counts as square.
type Edges = [f64; 4];

/// Bruls, Huizing, van Wijk: rows along the shorter side of what is left;
/// a tile joins the current row while the worst aspect ratio does not get
/// worse. `weights` are positive and sorted descending; the last row takes
/// exactly what is left, and neighbouring tiles share their edge values.
fn squarify(weights: &[u64], w: f64, h: f64) -> Vec<Edges> {
    let total: f64 = weights.iter().map(|&x| x as f64).sum();
    let scale = w * h / total;
    let areas: Vec<f64> = weights.iter().map(|&x| x as f64 * scale).collect();
    let mut out = Vec::with_capacity(areas.len());
    let (mut rx0, mut ry0, rx1, ry1) = (0.0f64, 0.0f64, w, h);
    let mut i = 0;
    while i < areas.len() {
        let (rw, rh) = (rx1 - rx0, ry1 - ry0);
        let short = rw.min(rh);
        let mut j = i + 1;
        let mut sum = areas[i];
        while j < areas.len() {
            let next = sum + areas[j];
            if worst(&areas[i..=j], next, short) <= worst(&areas[i..j], sum, short) {
                sum = next;
                j += 1;
            } else {
                break;
            }
        }
        let last = j == areas.len();
        if rw >= rh {
            // A column on the left, tiles stacked top to bottom.
            let x_end = if last { rx1 } else { rx0 + sum / rh };
            let mut y = ry0;
            for (k, &a) in areas.iter().enumerate().take(j).skip(i) {
                let y_end = if k == j - 1 { ry1 } else { y + a / sum * rh };
                out.push([rx0, y, x_end, y_end]);
                y = y_end;
            }
            rx0 = x_end;
        } else {
            // A row on top, tiles left to right.
            let y_end = if last { ry1 } else { ry0 + sum / rw };
            let mut x = rx0;
            for (k, &a) in areas.iter().enumerate().take(j).skip(i) {
                let x_end = if k == j - 1 { rx1 } else { x + a / sum * rw };
                out.push([x, ry0, x_end, y_end]);
                x = x_end;
            }
            ry0 = y_end;
        }
        i = j;
    }
    out
}

fn worst(row: &[f64], sum: f64, short: f64) -> f64 {
    row.iter()
        .map(|&a| (short * short * a / (sum * sum)).max(sum * sum / (short * short * a)))
        .fold(0.0, f64::max)
}

/// Visual edges to cells: every edge rounds the same way wherever it
/// appears, so tiles neither overlap nor leave gaps.
fn cells(e: Edges, area: Rect) -> Rect {
    let round = |v: f64| (v + 0.5).floor() as u16;
    let (x0, x1) = (round(e[0]), round(e[2]));
    let (y0, y1) = (round(e[1] / 2.0), round(e[3] / 2.0));
    Rect {
        x: area.x + x0,
        y: area.y + y0,
        w: x1 - x0,
        h: y1 - y0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traffic::aggregate::group_by_labels;
    use crate::traffic::resolved::tests::rich_flows;

    fn rect(x: u16, y: u16, w: u16, h: u16) -> Rect {
        Rect { x, y, w, h }
    }

    fn group(src: &str, dst: &str, connections: u64) -> FlowGroup {
        FlowGroup {
            src: vec![src.into()],
            dst: vec![dst.into()],
            service: None,
            flows: 1,
            connections,
            bytes: 0,
            allowed: 1,
            potentially_blocked: 0,
            blocked: 0,
            conns: Default::default(),
            draft: None,
            marks: None,
            change: None,
        }
    }

    fn rich() -> Vec<FlowGroup> {
        group_by_labels(&rich_flows(), &["app".to_string(), "env".to_string()])
    }

    fn assert_partition(tiles: &[Tile], area: Rect) {
        let covered: u32 = tiles.iter().map(|t| t.rect.area()).sum();
        assert_eq!(covered, area.area(), "the tiles fill the area exactly");
        for (i, a) in tiles.iter().enumerate() {
            assert!(!a.rect.is_empty(), "{a:?}");
            assert!(
                a.rect.x >= area.x
                    && a.rect.y >= area.y
                    && a.rect.x + a.rect.w <= area.x + area.w
                    && a.rect.y + a.rect.h <= area.y + area.h,
                "{a:?} inside {area:?}"
            );
            for b in &tiles[i + 1..] {
                assert!(!a.rect.intersects(b.rect), "{a:?} overlaps {b:?}");
            }
        }
    }

    #[test]
    fn tiles_fill_the_area_without_overlap_at_both_snapshot_sizes() {
        for area in [rect(1, 9, 78, 13), rect(1, 9, 158, 37)] {
            let tiles = layout(&rich(), area, Weight::Connections);
            assert_partition(&tiles, area);
            assert_eq!(tiles[0].label, "ipl:Corp/-", "heaviest first");
            assert_eq!(tiles[0].weight, 1_301);
            assert_eq!(tiles[0].decision, PolicyDecision::Blocked);
            let total: u64 = tiles.iter().map(|t| t.weight).sum();
            assert_eq!(total, 4_109, "folding keeps every connection");
            for t in tiles.iter().filter(|t| !t.children.is_empty()) {
                assert_partition(&t.children, t.rect.inner());
                assert_eq!(t.children.iter().map(|c| c.weight).sum::<u64>(), t.weight);
            }
        }
        let wide = layout(&rich(), rect(1, 9, 158, 37), Weight::Connections);
        assert_eq!(wide.len(), 9, "every source gets a tile at 160x48");
        let shop = wide.iter().find(|t| t.label == "shop/prod").unwrap();
        let dsts: Vec<&str> = shop.children.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(dsts, vec!["shop/prod", "crm/prod", "api.partner.example/-"]);
    }

    #[test]
    fn simple_weights_split_the_area_proportionally() {
        let two = [group("a", "x", 1), group("b", "y", 1)];
        let tiles = layout(&two, rect(0, 0, 10, 4), Weight::Connections);
        let rects: Vec<Rect> = tiles.iter().map(|t| t.rect).collect();
        assert_eq!(rects, vec![rect(0, 0, 5, 4), rect(5, 0, 5, 4)]);
        assert_eq!(
            tiles[0].children[0].rect,
            rect(1, 1, 3, 2),
            "inside the border"
        );
        let three_to_one = [group("b", "y", 1), group("a", "x", 3)];
        let tiles = layout(&three_to_one, rect(0, 0, 8, 2), Weight::Connections);
        let got: Vec<(&str, Rect)> = tiles.iter().map(|t| (t.label.as_str(), t.rect)).collect();
        assert_eq!(got, vec![("a", rect(0, 0, 6, 2)), ("b", rect(6, 0, 2, 2))]);
        assert!(
            tiles.iter().all(|t| t.children.is_empty()),
            "2 rows: no room for a border"
        );
        let one = layout(&[group("a", "x", 5)], rect(2, 3, 5, 4), Weight::Connections);
        assert_eq!(one[0].rect, rect(2, 3, 5, 4));
        assert_eq!(one[0].children[0].rect, rect(3, 4, 3, 2));
    }

    #[test]
    fn small_tiles_fold_into_other_and_a_single_cell_still_draws() {
        let g = rich();
        let tiny = layout(&g, rect(0, 0, 1, 1), Weight::Connections);
        assert_eq!(tiny.len(), 1);
        assert_eq!(tiny[0].label, "other (9)");
        assert_eq!(tiny[0].rect, rect(0, 0, 1, 1));
        assert_eq!(tiny[0].weight, 4_109);
        assert_eq!(tiny[0].decision, PolicyDecision::Unknown);
        assert!(tiny[0].is_other());
        let row = layout(&g, rect(0, 0, 3, 1), Weight::Connections);
        let got: Vec<(&str, Rect)> = row.iter().map(|t| (t.label.as_str(), t.rect)).collect();
        assert_eq!(
            got,
            vec![
                ("ipl:Corp/-", rect(0, 0, 1, 1)),
                ("other (8)", rect(1, 0, 2, 1))
            ]
        );
        let small = layout(&g, rect(0, 0, 10, 2), Weight::Connections);
        assert_partition(&small, rect(0, 0, 10, 2));
        assert_eq!(small.last().unwrap().label, "other (5)", "other comes last");
        assert!(!small[..small.len() - 1].iter().any(Tile::is_other));
    }

    #[test]
    fn zero_weight_and_empty_areas_give_no_tiles() {
        let g = rich();
        assert!(layout(&g, rect(0, 0, 0, 10), Weight::Connections).is_empty());
        assert!(layout(&g, rect(0, 0, 10, 0), Weight::Connections).is_empty());
        assert!(layout(&[], rect(0, 0, 10, 10), Weight::Connections).is_empty());
        assert!(
            layout(
                &[group("a", "x", 0)],
                rect(0, 0, 10, 10),
                Weight::Connections
            )
            .is_empty()
        );
        let by_bytes = layout(&g, rect(0, 0, 78, 13), Weight::Bytes);
        let labels: Vec<&str> = by_bytes.iter().map(|t| t.label.as_str()).collect();
        assert_eq!(
            labels,
            vec![
                "ipl:Corp/-",
                "crm/prod",
                "erp/prod",
                "shop/prod",
                "shop/dev",
                "other (1)"
            ],
            "sources without bytes drop out; hr/prod is too small and folds into other"
        );
        assert_partition(&by_bytes, rect(0, 0, 78, 13));
    }

    #[test]
    fn the_layout_is_deterministic_and_ignores_input_order() {
        let g = rich();
        let mut reversed = g.clone();
        reversed.reverse();
        let area = rect(1, 9, 78, 13);
        assert_eq!(
            layout(&g, area, Weight::Connections),
            layout(&g, area, Weight::Connections)
        );
        assert_eq!(
            layout(&g, area, Weight::Connections),
            layout(&reversed, area, Weight::Connections)
        );
        let ties = [group("b", "y", 2), group("a", "x", 2)];
        let tiles = layout(&ties, rect(0, 0, 10, 4), Weight::Connections);
        assert_eq!(tiles[0].label, "a", "equal weights sort by label");
    }

    #[test]
    fn a_dominant_source_stays_visible_beside_a_tiny_other() {
        let area = rect(0, 0, 78, 13);
        let g = [group("big", "x", 100_000), group("small", "y", 1)];
        let tiles = layout(&g, area, Weight::Connections);
        assert_eq!(tiles.len(), 2);
        assert_eq!(tiles[0].label, "big");
        assert_eq!(tiles[0].weight, 100_000);
        assert_eq!(tiles[1].label, "other (1)");
        assert_eq!(tiles[1].weight, 1);
        assert!(tiles[1].rect.area() >= 1);
        assert_partition(&tiles, area);
    }

    #[test]
    fn three_sources_with_a_huge_weight_do_not_collapse_into_other() {
        let area = rect(0, 0, 158, 40);
        let g = [
            group("a", "x", u64::MAX / 4),
            group("b", "y", 1),
            group("c", "z", 1),
        ];
        let tiles = layout(&g, area, Weight::Connections);
        assert_eq!(tiles[0].label, "a");
        assert_eq!(tiles[0].weight, u64::MAX / 4);
        assert!(!tiles[0].is_other());
        assert_partition(&tiles, area);
    }

    #[test]
    fn weight_sums_saturate_instead_of_overflowing() {
        let area = rect(0, 0, 20, 6);
        let g = [group("a", "x", u64::MAX), group("b", "y", u64::MAX)];
        let tiles = layout(&g, area, Weight::Connections);
        assert_eq!(tiles.len(), 2);
        assert_partition(&tiles, area);
        assert_eq!(sum(tiles.iter().map(|t| t.weight)), u64::MAX);
        let same_src = [group("a", "x", u64::MAX), group("a", "y", u64::MAX)];
        let tiles = layout(&same_src, area, Weight::Connections);
        assert_eq!(tiles[0].weight, u64::MAX);
    }

    #[test]
    fn zooming_lays_out_one_sources_destinations() {
        let g = rich();
        let area = rect(1, 9, 78, 13);
        let src = vec!["shop".to_string(), "prod".to_string()];
        let tiles = layout_zoomed(&g, &src, area, Weight::Connections);
        let got: Vec<(&str, u64)> = tiles.iter().map(|t| (t.label.as_str(), t.weight)).collect();
        assert_eq!(
            got,
            vec![
                ("shop/prod", 821),
                ("crm/prod", 150),
                ("api.partner.example/-", 33)
            ]
        );
        assert!(tiles.iter().all(|t| t.children.is_empty()));
        assert_partition(&tiles, area);
        assert!(layout_zoomed(&g, &["nope".to_string()], area, Weight::Connections).is_empty());
    }
}
