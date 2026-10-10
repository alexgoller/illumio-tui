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
}

impl<T> Clone for Column<T> {
    fn clone(&self) -> Self {
        Self {
            title: self.title,
            width: self.width,
            cell: self.cell,
            key: self.key,
            right: self.right,
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
        }
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

    fn apply(&mut self) {
        let needle = self.search_text().to_lowercase();
        let mut vis: Vec<usize> = (0..self.rows.len())
            .filter(|&i| {
                needle.is_empty()
                    || self
                        .columns
                        .iter()
                        .any(|c| (c.cell)(&self.rows[i]).to_lowercase().contains(&needle))
            })
            .collect();
        if let Some((col, desc)) = self.sort {
            let key = self.columns[col].key;
            vis.sort_by(|&a, &b| {
                let o = key(&self.rows[a]).cmp(&key(&self.rows[b]));
                if desc { o.reverse() } else { o }
            });
        }
        self.visible = vis;
        if self.visible.is_empty() {
            self.selected = 0;
            self.offset = 0;
        } else {
            self.selected = self.selected.min(self.visible.len() - 1);
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
            },
            Column {
                title: "n",
                width: 5,
                cell: |r: &R| r.n.to_string(),
                key: |r: &R| SortKey::Num(r.n),
                right: true,
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
