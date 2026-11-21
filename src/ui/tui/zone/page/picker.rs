// The list skeleton: one column of rows the reader can filter, move over and scroll.

// The invariant: `visible` always holds the indices of `rows` the filter words keep.
pub struct Picker<Row> {
    rows: Vec<Row>,
    visible: Vec<usize>,
    filter: String,
    selected: usize, // Position inside `visible`, never past its end.
    scroll: usize,   // First drawn row; the page writes it back from its draw.
}

impl<Row> Default for Picker<Row> {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            visible: Vec::new(),
            filter: String::new(),
            selected: 0,
            scroll: 0,
        }
    }
}

impl<Row> Picker<Row> {
    // The whole list arrived: the visible set is rebuilt, the window stays where the page left it.
    pub fn replace(&mut self, rows: Vec<Row>, haystack: impl Fn(&Row) -> String) {
        self.rows = rows;
        self.rebuild(haystack);
    }

    // The reader edits the words; the visible set and the window follow.
    pub fn edit_filter(
        &mut self,
        edit: impl FnOnce(&mut String),
        haystack: impl Fn(&Row) -> String,
    ) {
        edit(&mut self.filter);
        self.rebuild(haystack);
        self.scroll = 0;
    }

    // The list did not arrive: rows and positions go, the reader's words stay.
    pub fn clear_rows(&mut self) {
        self.rows.clear();
        self.visible.clear();
        self.selected = 0;
        self.scroll = 0;
    }

    // Nothing of the last list stays: rows, words and both positions.
    pub fn clear(&mut self) {
        self.clear_rows();
        self.filter.clear();
    }

    // Move the cursor by `delta` rows, stopping at either end.
    pub fn move_by(&mut self, delta: i64) {
        if self.visible.is_empty() {
            return;
        }
        let last = self.visible.len() - 1;
        let next = self.selected as i64 + delta;
        self.selected = next.clamp(0, last as i64) as usize;
    }

    pub fn first(&mut self) {
        self.selected = 0;
    }

    pub fn last(&mut self) {
        self.selected = self.visible.len().saturating_sub(1);
    }

    // Put the cursor at `at`, or at the last row when that is past the end.
    pub fn select(&mut self, at: usize) {
        self.selected = at.min(self.visible.len().saturating_sub(1));
    }

    // Keep the window on the cursor: one screen of rows at most.
    pub fn settle_scroll(&mut self, body: usize) {
        if self.visible.is_empty() {
            self.scroll = 0;
            return;
        }
        let body = body.max(1);
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + body {
            self.scroll = self.selected + 1 - body;
        }
        self.scroll = self.scroll.min(self.visible.len().saturating_sub(body));
    }

    // Row `i` of the visible set.
    pub fn row_at(&self, i: usize) -> Option<&Row> {
        let k = *self.visible.get(i)?;
        self.rows.get(k)
    }

    // The row under the cursor.
    pub fn row(&self) -> Option<&Row> {
        self.row_at(self.selected)
    }

    // The on-screen position of the first visible row this picks out.
    pub fn position(&self, pick: impl Fn(&Row) -> bool) -> Option<usize> {
        (0..self.visible.len()).find(|i| self.row_at(*i).is_some_and(|row| pick(row)))
    }

    pub fn filter(&self) -> &str {
        &self.filter
    }

    pub fn visible_len(&self) -> usize {
        self.visible.len()
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn scroll(&self) -> usize {
        self.scroll
    }

    pub fn set_scroll(&mut self, top: usize) {
        self.scroll = top;
    }

    // Recompute the visible set; the cursor is clamped, the window is left where it is.
    fn rebuild(&mut self, haystack: impl Fn(&Row) -> String) {
        let needle = self.filter.trim().to_lowercase();
        self.visible = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| needle.is_empty() || haystack(row).contains(&needle))
            .map(|(i, _)| i)
            .collect();
        self.selected = self.selected.min(self.visible.len().saturating_sub(1));
    }
}
