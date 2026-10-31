//! The Volume view (#89): one row per source group, heaviest first, its
//! destinations unfolded under it (`Enter`, `l`, `E`), each with its
//! weight, a bar split by decision and its blocked share. Built from the
//! pairs every view groups by (`pce::traffic::volume`); no request.
use super::*;
use pce::traffic::volume::{self, ByteSplits, Split, VolumeNode, VolumeSource, Weight};

/// A Volume row: a source, or one of its destinations.
#[derive(Debug, Clone, PartialEq)]
pub enum VolumeKind {
    Source {
        /// The source's and every destination's names, lowercased: what
        /// `/` matches, so a source shows when one of its destinations
        /// (folded or not) matches.
        names: Arc<str>,
    },
    Dest,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VolumeRow {
    pub node: Arc<VolumeNode>,
    /// The source's place in the tree: a source and its destinations
    /// share it (one search verdict per source).
    pub source: usize,
    pub kind: VolumeKind,
    /// What the row weighs (`b`).
    pub weight: Weight,
    /// The heaviest source's weight: the scale of every bar.
    pub scale: u64,
}

impl VolumeRow {
    pub fn is_source(&self) -> bool {
        matches!(self.kind, VolumeKind::Source { .. })
    }

    pub fn src(&self) -> &[String] {
        &self.node.group.src
    }

    /// The destination; `None` on a source row.
    pub fn dst(&self) -> Option<&[String]> {
        (!self.is_source()).then_some(self.node.group.dst.as_slice())
    }

    /// The group the row names: the source's, or the destination's.
    pub fn name(&self) -> String {
        self.dst().unwrap_or(self.src()).join("/")
    }

    pub fn split(&self) -> Split {
        self.node.split(self.weight)
    }

    /// Connections or bytes.
    pub fn value(&self) -> u64 {
        self.node.weight(self.weight)
    }

    /// `1.8M` connections, `671K` bytes (`ls -h`).
    pub fn value_text(&self) -> String {
        match self.weight {
            Weight::Connections => compact_count(self.value()),
            Weight::Bytes => pce::traffic::human_bytes(self.value()),
        }
    }

    pub fn change(&self) -> Option<&pce::traffic::RowChange> {
        self.node.group.change.as_ref()
    }
}

/// The Volume view's state besides its table: the weight, the sources
/// unfolded and the tree the rows are laid out from.
#[derive(Debug, Clone, Default)]
pub struct VolumeState {
    pub weight: Weight,
    /// Some loaded flow carried byte counts: `b` may weigh by bytes, and
    /// the meta draws `b bytes` dim when not.
    pub has_bytes: bool,
    /// The sources whose destinations are shown; kept across results.
    pub unfolded: BTreeSet<Vec<String>>,
    /// Every source and destination (compare mode's gone ones too), before
    /// `n`: a fold, `b` or `n` lays the rows out again without regrouping.
    pub(super) tree: Vec<VolumeSource>,
}

impl VolumeState {
    /// Every pair of the tree, folded or not: (source, destination).
    pub fn tree_pairs(&self) -> impl Iterator<Item = (&Vec<String>, &Vec<String>)> {
        self.tree
            .iter()
            .flat_map(|s| s.dsts.iter().map(|d| (&d.group.src, &d.group.dst)))
    }
}

/// `/` on Volume: a source matches by its name or any destination's.
fn volume_matches(r: &VolumeRow, needle: &str) -> bool {
    match &r.kind {
        _ if needle.is_empty() => true,
        VolumeKind::Source { names, .. } => names.contains(needle),
        VolumeKind::Dest => r.name().to_lowercase().contains(needle),
    }
}

fn volume_group(r: &VolumeRow) -> usize {
    r.source
}

/// The Volume table: rows drawn as bars, searched per source.
pub(super) fn volume_table() -> TableView<VolumeRow> {
    TableView::new(volume_columns(Weight::Connections, false, false))
        .with_matcher(volume_matches)
        .with_match_group(volume_group)
}

fn volume_text(r: &VolumeRow) -> SortKey {
    SortKey::Text(r.name())
}

macro_rules! split_column {
    ($title:expr, $field:ident) => {
        Column::<VolumeRow> {
            title: $title,
            width: 8,
            cell: |r| r.split().$field.to_string(),
            key: |r| SortKey::Num(r.split().$field),
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: None,
        }
    };
}

/// What `:export` writes for Volume, and what `/` and the header see: the
/// rows are drawn as bars (`ui::screens::traffic`), not as these cells.
/// `Δ` first in compare mode, `draft` last with draft decisions loaded;
/// the weight column is `conns` or `bytes`, the decision columns in it.
pub fn volume_columns(weight: Weight, draft: bool, cmp: bool) -> Vec<Column<VolumeRow>> {
    let mut cols: Vec<Column<VolumeRow>> = vec![
        // A source sums its destinations: say which a row is, so a sum of
        // the weight column can keep one level.
        Column {
            title: "level",
            width: 0,
            cell: |r| {
                if r.is_source() {
                    "source".into()
                } else {
                    "destination".into()
                }
            },
            key: volume_text,
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: None,
        },
        Column {
            title: "src",
            width: 0,
            cell: |r| r.src().join("/"),
            key: volume_text,
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: None,
        },
        Column {
            title: "dst",
            width: 0,
            cell: |r| r.dst().map(|d| d.join("/")).unwrap_or_default(),
            key: volume_text,
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: None,
        },
        Column {
            title: match weight {
                Weight::Connections => "conns",
                Weight::Bytes => "bytes",
            },
            width: 8,
            cell: |r| r.value().to_string(),
            key: |r| SortKey::Num(r.value()),
            right: true,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: None,
        },
        split_column!("allowed", allowed),
        split_column!("pot_blocked", potentially_blocked),
        split_column!("blocked", blocked),
        split_column!("unknown", unknown),
    ];
    if draft {
        cols.push(Column {
            title: DRAFT_COLUMN,
            width: 9,
            cell: |r| draft_short(r.node.group.draft.as_ref()),
            key: |r| SortKey::Num(r.node.group.draft.map_or(0, |d| d.draft_allowed)),
            right: false,
            hide_below: 0,
            narrow: None,
            narrow_width: 0,
            full_title: None,
            full: Some(|r| draft_counts_cell(r.node.group.draft.as_ref())),
        });
    }
    if cmp {
        cols.insert(0, delta_volume_column());
    }
    cols
}

impl TrafficState {
    /// The Volume tree from `pairs` (every pair of the flows the views
    /// aggregate, marks annotated): compare mode's changes by pair and by
    /// source, each pair's bytes per decision from the same flows (a gone
    /// pair's from the baseline), each source's marks summed from its
    /// pairs'; then its rows.
    pub(super) fn rebuild_volume(&mut self, mut pairs: Vec<FlowGroup>) {
        let sides = self.service_sides.take();
        let keys = &self.keys;
        let recent = self.shown_flows();
        let mut fallback = ByteSplits::new();
        if let Some(b) = self.baseline_view.as_ref() {
            // As App-to-app's pairs on every decision: each service key
            // judged, then a pair from its keys.
            let (all_r, all_b) = match sides {
                Some(sides) => sides,
                None => (group_by_service(recent, keys), group_by_service(b, keys)),
            };
            let mut services = all_r.clone();
            pce::traffic::compare_groups(
                &mut services,
                pce::traffic::Sides::unfiltered(&all_r, &all_b),
            );
            pce::traffic::compare_pairs(&mut pairs, &group_by_labels(b, keys), &services);
            fallback = volume::byte_splits(b, keys);
        }
        let mut tree = volume::tree(&pairs, &volume::byte_splits(recent, keys), &fallback);
        for s in &mut tree {
            Arc::make_mut(&mut s.node).group.marks = sum_marks(
                s.dsts
                    .iter()
                    .map(|d| (d.group.flows, d.group.marks.as_ref())),
            );
        }
        self.volume_state.tree = tree;
        self.relayout_volume();
    }

    /// The Volume rows again from its tree: sorted by the weight (bytes
    /// only when some flow has them), `n` applied, the unfolded sources'
    /// destinations under them; compare mode's counts are the rows so laid
    /// out before `n`. The cursor stays on its row when it is still shown.
    pub fn relayout_volume(&mut self) {
        let has_bytes = self.has_bytes();
        let st = &mut self.volume_state;
        st.has_bytes = has_bytes;
        if !has_bytes {
            st.weight = Weight::Connections;
        }
        let weight = st.weight;
        volume::sort(&mut st.tree, weight);
        let unfolded = &st.unfolded;
        if let Some(cmp) = self.compare.as_mut() {
            let mut counts = pce::traffic::CompareCounts::default();
            for s in &st.tree {
                let open = unfolded.contains(&s.node.group.src);
                let dsts = s.dsts.iter().filter(|_| open);
                for c in std::iter::once(&s.node).chain(dsts) {
                    if let Some(c) = c.group.change {
                        counts.add(c.change);
                    }
                }
            }
            cmp.counts.volume = counts;
        }
        let mut shown = st.tree.clone();
        if let Some(cmp) = self
            .compare
            .as_ref()
            .filter(|_| self.baseline_view.is_some())
        {
            volume::keep_changes(&mut shown, cmp.show.set());
        }
        let scale = volume::scale(&shown, weight);
        let mut rows = Vec::new();
        for (i, s) in shown.into_iter().enumerate() {
            let open = unfolded.contains(&s.node.group.src);
            let mut names = s.node.group.src.join("/").to_lowercase();
            for d in &s.dsts {
                names.push('\n');
                names.push_str(&d.group.dst.join("/").to_lowercase());
            }
            rows.push(VolumeRow {
                node: s.node,
                source: i,
                kind: VolumeKind::Source {
                    names: names.into(),
                },
                weight,
                scale,
            });
            if open {
                rows.extend(s.dsts.into_iter().map(|d| VolumeRow {
                    node: d,
                    source: i,
                    kind: VolumeKind::Dest,
                    weight,
                    scale,
                }));
            }
        }
        let at = self.volume_key();
        let cols = volume_columns(weight, self.draft_loaded(), self.comparing());
        let titles = |c: &[Column<VolumeRow>]| c.iter().map(|c| c.title).collect::<Vec<_>>();
        if titles(&cols) != titles(&self.volume.columns) {
            self.volume.set_columns(cols);
        }
        self.volume.set_rows(rows);
        if let Some(at) = at {
            self.volume.select_where(|r| row_key(r) == at);
        }
    }

    /// The selected Volume row's source and destination.
    fn volume_key(&self) -> Option<(Vec<String>, Option<Vec<String>>)> {
        self.volume.selected_item().map(row_key)
    }

    /// `Enter` / `l` on a source (`unfold`), `h` (`!unfold`): its
    /// destinations shown or hidden; on a destination `h` folds its source
    /// and the cursor goes there. `None` unfold: `Enter`'s and `Space`'s
    /// toggle (`Space` on a destination folds its source). Whether
    /// anything changed.
    pub fn fold_volume(&mut self, unfold: Option<bool>) -> bool {
        let Some(r) = self.volume.selected_item() else {
            return false;
        };
        let src = r.src().to_vec();
        let open = self.volume_state.unfolded.contains(&src);
        let want = unfold.unwrap_or(!open);
        if !r.is_source() && want {
            return false;
        }
        if want == open {
            return false;
        }
        if want {
            self.volume_state.unfolded.insert(src.clone());
        } else {
            self.volume_state.unfolded.remove(&src);
        }
        self.relayout_volume();
        self.volume
            .select_where(|r| r.is_source() && r.src() == src.as_slice());
        true
    }

    /// `z` on Volume: every source unfolded, or every one folded when all
    /// already are. The cursor stays on its source.
    pub fn fold_all_volume(&mut self) {
        let sources: Vec<Vec<String>> = self
            .volume_state
            .tree
            .iter()
            .map(|s| s.node.group.src.clone())
            .collect();
        let st = &mut self.volume_state;
        let all = sources.iter().all(|s| st.unfolded.contains(s));
        if all {
            for s in &sources {
                st.unfolded.remove(s);
            }
        } else {
            st.unfolded.extend(sources);
        }
        let src = self.volume.selected_item().map(|r| r.src().to_vec());
        self.relayout_volume();
        if let Some(src) = src {
            self.volume
                .select_where(|r| r.is_source() && r.src() == src.as_slice());
        }
    }

    /// `b` on Volume: bytes or connections; `false` when no flow has bytes.
    pub fn toggle_volume_weight(&mut self) -> bool {
        if !self.volume_state.has_bytes {
            return false;
        }
        self.volume_state.weight = match self.volume_state.weight {
            Weight::Connections => Weight::Bytes,
            Weight::Bytes => Weight::Connections,
        };
        self.relayout_volume();
        true
    }

    /// `Enter` on a destination row: the Flows view drilled to its pair.
    pub fn volume_drill(&self) -> Option<Drill> {
        let r = self.volume.selected_item()?;
        Some(Drill::Pair(
            r.src().to_vec(),
            r.dst()?.to_vec(),
            Decisions::All,
        ))
    }

    /// The apps of the selected Volume row: its source, then its
    /// destination.
    pub(super) fn volume_apps(&self) -> Vec<GroupKey> {
        let Some(r) = self.volume.selected_item() else {
            return Vec::new();
        };
        let mut apps = vec![r.src().to_vec()];
        apps.extend(r.dst().map(<[String]>::to_vec));
        apps
    }
}

fn row_key(r: &VolumeRow) -> (Vec<String>, Option<Vec<String>>) {
    (r.src().to_vec(), r.dst().map(<[String]>::to_vec))
}
