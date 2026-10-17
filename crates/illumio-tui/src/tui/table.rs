//! Pure table state shared by every screen: rows, a search filter, a sort
//! column, a selection and a scroll offset. No rendering here.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum SortKey {
    Text(String),
    Num(u64),
}

pub struct Column<T> {
    pub title: &'static str,
    /// Preferred width in cells; 0 means "take the remaining space".
    pub width: u16,
    pub cell: fn(&T) -> String,
    pub key: fn(&T) -> SortKey,
    pub right: bool,
    /// Hidden when the table area is narrower than this (0 = always shown).
    pub hide_below: u16,
    /// Shorter cell text used below 120 columns.
    pub narrow: Option<fn(&T) -> String>,
    /// Width used below 120 columns when > 0.
    pub narrow_width: u16,
    /// The whole value for `:export` where `cell` abbreviates it (`6.5K`,
    /// `3 hosts`, a short time); `None` exports `cell`.
    pub full: Option<fn(&T) -> String>,
    /// `:export`'s header for `full` when its value differs in kind
    /// (`heartbeat_age_secs` for a `3m` cell); `None` keeps `title`.
    pub full_title: Option<&'static str>,
}

impl<T> Clone for Column<T> {
    fn clone(&self) -> Self {
        Self {
            title: self.title,
            width: self.width,
            cell: self.cell,
            key: self.key,
            right: self.right,
            hide_below: self.hide_below,
            narrow: self.narrow,
            narrow_width: self.narrow_width,
            full: self.full,
            full_title: self.full_title,
        }
    }
}

impl<T> std::fmt::Debug for Column<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Column")
            .field("title", &self.title)
            .field("width", &self.width)
            .finish()
    }
}

#[derive(Debug)]
pub struct TableView<T> {
    pub columns: Vec<Column<T>>,
    rows: Vec<T>,
    visible: Vec<usize>,
    selected: usize,
    offset: usize,
    sort: Option<(usize, bool)>,
    search: String,
    searching: bool,
    search_draft: String,
    /// Decides which rows are shown, given the lowercased search text
    /// (possibly empty); `None` keeps the default substring match over
    /// every cell.
    matcher: Option<fn(&T, &str) -> bool>,
    /// A row's mark, drawn as a 2-cell gutter before the first column
    /// while set (marks active). Not a column: column indices, `s`, the
    /// default sort, search and `:export` do not see it.
    pub marker: Option<fn(&T) -> Option<pce::marks::Marker>>,
}

impl<T> TableView<T> {
    pub fn new(columns: Vec<Column<T>>) -> Self {
        Self {
            columns,
            rows: Vec::new(),
            visible: Vec::new(),
            selected: 0,
            offset: 0,
            sort: None,
            search: String::new(),
            searching: false,
            search_draft: String::new(),
            matcher: None,
            marker: None,
        }
    }

    /// The table with its own row filter in place of the substring search.
    pub fn with_matcher(mut self, matcher: fn(&T, &str) -> bool) -> Self {
        self.matcher = Some(matcher);
        self
    }

    /// Other columns (a view gaining or losing one): the sort stays when
    /// its column still exists.
    pub fn set_columns(&mut self, columns: Vec<Column<T>>) {
        if self.sort.is_some_and(|(i, _)| i >= columns.len()) {
            self.sort = None;
        }
        self.columns = columns;
        self.apply();
    }

    pub fn set_rows(&mut self, rows: Vec<T>) {
        self.rows = rows;
        self.apply();
    }

    pub fn rows(&self) -> &[T] {
        &self.rows
    }

    pub fn len(&self) -> usize {
        self.visible.len()
    }

    pub fn is_empty(&self) -> bool {
        self.visible.is_empty()
    }

    pub fn visible(&self) -> &[usize] {
        &self.visible
    }

    pub fn offset(&self) -> usize {
        self.offset
    }

    pub fn selected_index(&self) -> Option<usize> {
        self.visible.get(self.selected).copied()
    }

    pub fn selected_pos(&self) -> usize {
        self.selected
    }

    pub fn selected_item(&self) -> Option<&T> {
        self.selected_index().map(|i| &self.rows[i])
    }

    pub fn select_next(&mut self) {
        if !self.visible.is_empty() {
            self.selected = (self.selected + 1).min(self.visible.len() - 1);
        }
    }

    /// Moves the cursor to the first visible row `pred` accepts, if any.
    pub fn select_where(&mut self, pred: impl Fn(&T) -> bool) {
        if let Some(pos) = self.visible.iter().position(|&i| pred(&self.rows[i])) {
            self.selected = pos;
        }
    }

    pub fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn page_down(&mut self, n: usize) {
        if !self.visible.is_empty() {
            self.selected = (self.selected + n.max(1)).min(self.visible.len() - 1);
        }
    }

    pub fn page_up(&mut self, n: usize) {
        self.selected = self.selected.saturating_sub(n.max(1));
    }

    pub fn home(&mut self) {
        self.selected = 0;
    }

    pub fn end(&mut self) {
        self.selected = self.visible.len().saturating_sub(1);
    }

    pub fn sort(&self) -> Option<(usize, bool)> {
        self.sort
    }

    /// `s` cycles: col0 asc, col0 desc, col1 asc, … then unsorted.
    pub fn sort_next(&mut self) {
        self.sort = match self.sort {
            None => Some((0, false)),
            Some((c, false)) => Some((c, true)),
            Some((c, true)) if c + 1 < self.columns.len() => Some((c + 1, false)),
            Some(_) => None,
        };
        self.apply();
    }

    pub fn sort_by(&mut self, col: usize, desc: bool) {
        self.sort = (col < self.columns.len()).then_some((col, desc));
        self.apply();
    }

    pub fn start_search(&mut self) {
        self.searching = true;
        self.search_draft = self.search.clone();
    }

    pub fn is_searching(&self) -> bool {
        self.searching
    }

    pub fn search_text(&self) -> &str {
        if self.searching {
            &self.search_draft
        } else {
            &self.search
        }
    }

    pub fn search_push(&mut self, c: char) {
        self.search_draft.push(c);
        self.apply();
    }

    pub fn search_pop(&mut self) {
        self.search_draft.pop();
        self.apply();
    }

    pub fn search_commit(&mut self) {
        self.search = std::mem::take(&mut self.search_draft);
        self.searching = false;
        self.apply();
    }

    pub fn search_cancel(&mut self) {
        self.search_draft.clear();
        self.searching = false;
        self.apply();
    }

    pub fn search_clear(&mut self) {
        self.search.clear();
        self.search_draft.clear();
        self.searching = false;
        self.apply();
    }

    /// Moves the scroll offset so the selected row is inside `height` rows.
    pub fn ensure_visible(&mut self, height: usize) {
        let h = height.max(1);
        if self.selected < self.offset {
            self.offset = self.selected;
        } else if self.selected >= self.offset + h {
            self.offset = self.selected + 1 - h;
        }
        let max_offset = self.visible.len().saturating_sub(h);
        self.offset = self.offset.min(max_offset);
    }

    pub fn cell(&self, row: &T, col: usize) -> String {
        (self.columns[col].cell)(row)
    }

    /// Every column's whole value for `row` (`full`, else `cell`),
    /// whatever the width.
    pub fn full_cells(&self, row: &T) -> Vec<String> {
        self.columns
            .iter()
            .map(|c| c.full.unwrap_or(c.cell)(row))
            .collect()
    }

    /// What `:export` writes: every column's title, and the shown rows
    /// (search applied, in sort order) as `full_cells`.
    pub fn export(&self) -> (Vec<&'static str>, Vec<Vec<String>>) {
        let titles = self
            .columns
            .iter()
            .map(|c| c.full_title.unwrap_or(c.title))
            .collect();
        let rows = self
            .visible
            .iter()
            .map(|&i| self.full_cells(&self.rows[i]))
            .collect();
        (titles, rows)
    }

    /// The positions in `rows` this view would show, in its order: its
    /// search (or matcher) and sort applied to other rows.
    pub fn visible_of(&self, rows: &[T]) -> Vec<usize> {
        let needle = self.search_text().to_lowercase();
        let mut vis: Vec<usize> = (0..rows.len())
            .filter(|&i| {
                if let Some(m) = self.matcher {
                    return m(&rows[i], &needle);
                }
                needle.is_empty()
                    || self
                        .columns
                        .iter()
                        .any(|c| (c.cell)(&rows[i]).to_lowercase().contains(&needle))
            })
            .collect();
        if let Some((col, desc)) = self.sort {
            let key = self.columns[col].key;
            vis.sort_by(|&a, &b| {
                let o = key(&rows[a]).cmp(&key(&rows[b]));
                if desc { o.reverse() } else { o }
            });
        }
        vis
    }

    fn apply(&mut self) {
        self.visible = self.visible_of(&self.rows);
        if self.visible.is_empty() {
            self.selected = 0;
            self.offset = 0;
        } else {
            self.selected = self.selected.min(self.visible.len() - 1);
            // Fewer rows can pull the selection above the scroll offset:
            // never scroll past it, or the table draws empty.
            self.offset = self.offset.min(self.selected);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq)]
    struct R {
        name: &'static str,
        n: u64,
    }

    fn view() -> TableView<R> {
        let cols = vec![
            Column {
                title: "name",
                width: 10,
                cell: |r: &R| r.name.to_string(),
                key: |r: &R| SortKey::Text(r.name.to_string()),
                right: false,
                hide_below: 0,
                narrow: None,
                narrow_width: 0,
                full_title: None,
                full: None,
            },
            Column {
                title: "n",
                width: 5,
                cell: |r: &R| r.n.to_string(),
                key: |r: &R| SortKey::Num(r.n),
                right: true,
                hide_below: 0,
                narrow: None,
                narrow_width: 0,
                full_title: None,
                full: None,
            },
        ];
        let mut v = TableView::new(cols);
        v.set_rows(vec![
            R {
                name: "web01",
                n: 310,
            },
            R {
                name: "db01",
                n: 12,
            },
            R { name: "nas", n: 3 },
            R {
                name: "web02",
                n: 5,
            },
        ]);
        v
    }

    #[test]
    fn fewer_rows_never_leave_the_offset_past_the_selection() {
        let mut v = view();
        let many: Vec<R> = (0..100).map(|n| R { name: "r", n }).collect();
        v.set_rows(many.clone());
        for _ in 0..90 {
            v.select_next();
        }
        v.ensure_visible(10);
        assert_eq!(v.offset(), 81);
        v.set_rows(many[..5].to_vec());
        assert_eq!(v.selected_pos(), 4);
        assert!(v.offset() <= v.selected_pos(), "{}", v.offset());
    }

    #[test]
    fn selection_moves_and_clamps() {
        let mut v = view();
        assert_eq!(v.selected_item().unwrap().name, "web01");
        v.select_prev();
        assert_eq!(v.selected_item().unwrap().name, "web01");
        v.select_next();
        v.select_next();
        assert_eq!(v.selected_item().unwrap().name, "nas");
        v.end();
        assert_eq!(v.selected_item().unwrap().name, "web02");
        v.select_next();
        assert_eq!(v.selected_item().unwrap().name, "web02");
        v.home();
        assert_eq!(v.selected_item().unwrap().name, "web01");
        v.page_down(2);
        assert_eq!(v.selected_item().unwrap().name, "nas");
        v.page_up(10);
        assert_eq!(v.selected_item().unwrap().name, "web01");
    }

    #[test]
    fn sort_cycles_columns_and_directions() {
        let mut v = view();
        v.sort_next();
        assert_eq!(v.sort(), Some((0, false)));
        assert_eq!(v.selected_item().unwrap().name, "db01");
        v.sort_next();
        assert_eq!(v.sort(), Some((0, true)));
        assert_eq!(v.selected_item().unwrap().name, "web02");
        v.sort_next();
        assert_eq!(v.sort(), Some((1, false)));
        assert_eq!(v.selected_item().unwrap().n, 3);
        v.sort_next();
        assert_eq!(v.selected_item().unwrap().n, 310);
        v.sort_next();
        assert_eq!(v.sort(), None, "cycle wraps to unsorted");
        assert_eq!(v.selected_item().unwrap().name, "web01");
    }

    #[test]
    fn search_filters_across_cells_case_insensitively() {
        let mut v = view();
        v.start_search();
        assert!(v.is_searching());
        for c in "WEB".chars() {
            v.search_push(c);
        }
        assert_eq!(v.len(), 2, "filter applies while typing");
        v.search_commit();
        assert!(!v.is_searching());
        assert_eq!(v.search_text(), "WEB");
        assert_eq!(v.len(), 2);
        v.search_clear();
        assert_eq!(v.len(), 4);
        v.start_search();
        v.search_push('z');
        assert_eq!(v.len(), 0);
        assert!(v.selected_item().is_none());
        v.search_cancel();
        assert_eq!(v.len(), 4);
        assert_eq!(v.search_text(), "");
    }

    #[test]
    fn a_matcher_replaces_the_substring_search_and_sees_every_row() {
        let mut v = view().with_matcher(|r: &R, needle: &str| {
            r.n > 4 && (needle.is_empty() || r.name.starts_with(needle))
        });
        v.set_rows(view().rows().to_vec());
        assert_eq!(v.len(), 3, "the matcher also runs without a search");
        v.start_search();
        v.search_push('W');
        assert_eq!(v.len(), 2, "lowercased needle");
        v.search_push('e');
        v.search_push('b');
        v.search_push('0');
        v.search_push('2');
        assert_eq!(v.selected_item().unwrap().name, "web02");
    }

    #[test]
    fn export_keeps_search_and_sort_and_prefers_full_cells() {
        let mut v = view();
        v.columns[1].hide_below = 200;
        v.columns[1].full = Some(|r: &R| format!("{} exactly", r.n));
        v.sort_by(1, true);
        v.start_search();
        v.search_push('w');
        v.search_commit();
        let (titles, rows) = v.export();
        assert_eq!(titles, ["name", "n"]);
        assert_eq!(
            rows,
            [["web01", "310 exactly"], ["web02", "5 exactly"]].map(|r| r.map(String::from))
        );
    }

    #[test]
    fn offset_follows_selection() {
        let mut v = view();
        v.end();
        v.ensure_visible(2);
        assert_eq!(v.offset(), 2);
        v.home();
        v.ensure_visible(2);
        assert_eq!(v.offset(), 0);
    }

    #[test]
    fn set_rows_keeps_selection_in_range() {
        let mut v = view();
        v.end();
        v.set_rows(vec![R { name: "only", n: 1 }]);
        assert_eq!(v.selected_item().unwrap().name, "only");
        v.set_rows(vec![]);
        assert!(v.selected_item().is_none());
        assert_eq!(v.selected_index(), None);
    }
}
