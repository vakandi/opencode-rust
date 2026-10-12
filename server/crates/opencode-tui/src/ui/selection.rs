use unicode_width::UnicodeWidthChar;

/// Terminal text selection — tracks a rectangular region in screen coordinates
/// and provides hit-testing + text extraction.
///
/// Selection follows standard terminal behavior:
/// - First row: from start column to end of line
/// - Middle rows: entire line
/// - Last row: from start of line to end column
/// - Single row: from start column to end column

pub struct Selection {
    /// Anchor point (where mouse-down happened).
    anchor: Option<(u16, u16)>,
    /// Current drag endpoint.
    cursor: Option<(u16, u16)>,
    /// True while the mouse button is held down.
    dragging: bool,
}

impl Selection {
    pub fn new() -> Self {
        Self {
            anchor: None,
            cursor: None,
            dragging: false,
        }
    }

    /// Begin a new selection at (row, col).
    pub fn start(&mut self, row: u16, col: u16) {
        self.anchor = Some((row, col));
        self.cursor = Some((row, col));
        self.dragging = true;
    }

    /// Update the drag endpoint.
    pub fn update(&mut self, row: u16, col: u16) {
        if self.dragging {
            self.cursor = Some((row, col));
        }
    }

    /// Mouse button released — keep the selection visible but stop tracking.
    pub fn finalize(&mut self) {
        self.dragging = false;
    }

    /// Dismiss the selection entirely.
    pub fn clear(&mut self) {
        self.anchor = None;
        self.cursor = None;
        self.dragging = false;
    }

    /// True if there is a visible selection (dragging or finalized).
    pub fn is_active(&self) -> bool {
        self.anchor.is_some() && self.cursor.is_some()
    }

    /// True if the user is currently dragging.
    pub fn is_selecting(&self) -> bool {
        self.dragging
    }

    /// Returns the normalized range: (top-left, bottom-right) in reading order.
    fn range(&self) -> Option<((u16, u16), (u16, u16))> {
        match (self.anchor, self.cursor) {
            (Some(a), Some(b)) => {
                if a.0 < b.0 || (a.0 == b.0 && a.1 <= b.1) {
                    Some((a, b))
                } else {
                    Some((b, a))
                }
            }
            _ => None,
        }
    }

    /// Test whether a specific cell is inside the selection.
    pub fn is_selected(&self, row: u16, col: u16) -> bool {
        let ((r0, c0), (r1, c1)) = match self.range() {
            Some(r) => r,
            None => return false,
        };

        if row < r0 || row > r1 {
            return false;
        }

        if r0 == r1 {
            // Single-line selection
            return col >= c0 && col <= c1;
        }

        if row == r0 {
            // First row: from start col to end of line
            return col >= c0;
        }

        if row == r1 {
            // Last row: from start of line to end col
            return col <= c1;
        }

        // Middle rows: entire line
        true
    }

    /// Extract the selected text using a callback that returns the full line
    /// content for a given row number.
    pub fn get_selected_text<F>(&self, get_line: F) -> String
    where
        F: Fn(u16) -> Option<String>,
    {
        let ((r0, c0), (r1, c1)) = match self.range() {
            Some(r) => r,
            None => return String::new(),
        };

        let mut result = String::new();

        for row in r0..=r1 {
            let line = match get_line(row) {
                Some(l) => l,
                None => continue,
            };

            let selected = if r0 == r1 {
                // Single line: clip both ends
                slice_by_columns(&line, c0 as usize, c1.saturating_add(1) as usize)
            } else if row == r0 {
                // First row: from col to end
                slice_from_column(&line, c0 as usize)
            } else if row == r1 {
                // Last row: from start to col
                slice_to_column(&line, c1.saturating_add(1) as usize)
            } else {
                // Middle row: full line
                line
            };

            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str(selected.trim_end());
        }

        result
    }
}

fn slice_by_columns(line: &str, start_col: usize, end_col_exclusive: usize) -> String {
    if start_col >= end_col_exclusive {
        return String::new();
    }
    let start = byte_index_for_column_start(line, start_col);
    let end = byte_index_for_column_end(line, end_col_exclusive);
    if start >= end || start >= line.len() {
        return String::new();
    }
    line[start..end].to_string()
}

fn slice_from_column(line: &str, start_col: usize) -> String {
    let start = byte_index_for_column_start(line, start_col);
    if start >= line.len() {
        return String::new();
    }
    line[start..].to_string()
}

fn slice_to_column(line: &str, end_col_exclusive: usize) -> String {
    let end = byte_index_for_column_end(line, end_col_exclusive);
    if end == 0 {
        return String::new();
    }
    line[..end].to_string()
}

fn byte_index_for_column_start(line: &str, target_col: usize) -> usize {
    if target_col == 0 {
        return 0;
    }

    let mut col = 0usize;
    for (idx, ch) in line.char_indices() {
        let width = UnicodeWidthChar::width(ch).unwrap_or(0).max(1);
        if target_col <= col {
            return idx;
        }
        if target_col < col + width {
            // Selection starts inside a glyph cell: snap to glyph start.
            return idx;
        }
        col += width;
    }
    line.len()
}

fn byte_index_for_column_end(line: &str, target_col: usize) -> usize {
    if target_col == 0 {
        return 0;
    }

    let mut col = 0usize;
    for (idx, ch) in line.char_indices() {
        let width = UnicodeWidthChar::width(ch).unwrap_or(0).max(1);
        let end_idx = idx + ch.len_utf8();
        if target_col <= col {
            return idx;
        }
        if target_col < col + width {
            // Selection ends inside a glyph cell: include the whole glyph.
            return end_idx;
        }
        if target_col == col + width {
            return end_idx;
        }
        col += width;
    }
    line.len()
}

impl Default for Selection {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::Selection;

    #[test]
    fn utf8_glyph_boundary_selection_is_safe() {
        let line = "  ││ Provider error";
        let mut selection = Selection::new();
        selection.start(0, 2);
        selection.update(0, 4);
        selection.finalize();

        let selected = selection.get_selected_text(|row| {
            if row == 0 {
                Some(line.to_string())
            } else {
                None
            }
        });

        assert_eq!(selected, "││");
    }

    #[test]
    fn unicode_wide_char_selection_is_safe() {
        let line = "A你B";
        let mut selection = Selection::new();
        // Select only the wide glyph cell range.
        selection.start(0, 1);
        selection.update(0, 2);
        selection.finalize();

        let selected = selection.get_selected_text(|row| {
            if row == 0 {
                Some(line.to_string())
            } else {
                None
            }
        });

        assert_eq!(selected, "你");
    }
}
