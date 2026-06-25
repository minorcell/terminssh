//! Terminal grid: stores cell data, cursor position, and scrollback.

use std::collections::VecDeque;

/// An RGB color value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Convert to a hex string for debugging.
    pub fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

/// Default foreground/background colors (dark theme).
pub const DEFAULT_FG: Color = Color::rgb(0xd3, 0xd7, 0xcf);
pub const DEFAULT_BG: Color = Color::rgb(0x1a, 0x1b, 0x26);

/// The standard 16-color ANSI palette (GNOME terminal colors).
pub static ANSI_16: [Color; 16] = [
    Color::rgb(0x35, 0x35, 0x35), // 0  Black
    Color::rgb(0xcc, 0x00, 0x00), // 1  Red
    Color::rgb(0x4e, 0x9a, 0x06), // 2  Green
    Color::rgb(0xc4, 0xa0, 0x00), // 3  Yellow
    Color::rgb(0x34, 0x65, 0xa4), // 4  Blue
    Color::rgb(0x75, 0x50, 0x7b), // 5  Magenta
    Color::rgb(0x06, 0x98, 0x9a), // 6  Cyan
    Color::rgb(0xd3, 0xd7, 0xcf), // 7  White
    Color::rgb(0x55, 0x57, 0x53), // 8  Bright Black
    Color::rgb(0xef, 0x29, 0x29), // 9  Bright Red
    Color::rgb(0x8a, 0xe2, 0x34), // 10 Bright Green
    Color::rgb(0xfc, 0xe9, 0x4f), // 11 Bright Yellow
    Color::rgb(0x72, 0x9f, 0xcf), // 12 Bright Blue
    Color::rgb(0xad, 0x7f, 0xa8), // 13 Bright Magenta
    Color::rgb(0x34, 0xe2, 0xe2), // 14 Bright Cyan
    Color::rgb(0xee, 0xee, 0xec), // 15 Bright White
];

/// Look up a 256-color palette index.
pub fn xterm_256(idx: u8) -> Color {
    match idx {
        0..=15 => ANSI_16[idx as usize],
        16..=231 => {
            // 6x6x6 RGB cube
            let idx = idx - 16;
            let r = idx / 36;
            let g = (idx / 6) % 6;
            let b = idx % 6;
            let val = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            Color::rgb(val(r), val(g), val(b))
        }
        232..=255 => {
            // Grayscale ramp
            let v = 8 + (idx - 232) * 10;
            Color::rgb(v, v, v)
        }
    }
}

/// A single cell in the terminal grid.
#[derive(Clone, Copy, Debug)]
pub struct Cell {
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub reverse: bool,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: ' ',
            fg: DEFAULT_FG,
            bg: DEFAULT_BG,
            bold: false,
            italic: false,
            underline: false,
            reverse: false,
        }
    }
}

impl Cell {
    /// Get the effective foreground color (accounting for reverse video).
    pub fn effective_fg(&self) -> Color {
        if self.reverse {
            self.bg
        } else {
            self.fg
        }
    }

    /// Get the effective background color (accounting for reverse video).
    pub fn effective_bg(&self) -> Color {
        if self.reverse {
            self.fg
        } else {
            self.bg
        }
    }
}

/// The terminal grid: a 2D array of cells with cursor and scrollback.
pub struct TerminalGrid {
    /// Visible cells: rows × cols.
    cells: Vec<Vec<Cell>>,
    /// Scrollback history (oldest first).
    scrollback: VecDeque<Vec<Cell>>,
    /// Maximum scrollback lines to keep.
    max_scrollback: usize,
    /// Cursor position.
    cursor_row: usize,
    cursor_col: usize,
    /// Grid dimensions.
    rows: usize,
    cols: usize,
    /// Current rendering attributes (used by the ANSI parser).
    current_fg: Color,
    current_bg: Color,
    current_bold: bool,
    current_italic: bool,
    current_underline: bool,
    current_reverse: bool,
    /// Cursor visibility.
    cursor_visible: bool,
    /// Scroll region (top, bottom) inclusive, for DECSTBM.
    scroll_top: usize,
    scroll_bottom: usize,
}

impl TerminalGrid {
    /// Create a new grid with the given dimensions.
    pub fn new(rows: usize, cols: usize) -> Self {
        let cells = (0..rows)
            .map(|_| (0..cols).map(|_| Cell::default()).collect())
            .collect();
        Self {
            cells,
            scrollback: VecDeque::new(),
            max_scrollback: 10000,
            cursor_row: 0,
            cursor_col: 0,
            rows,
            cols,
            current_fg: DEFAULT_FG,
            current_bg: DEFAULT_BG,
            current_bold: false,
            current_italic: false,
            current_underline: false,
            current_reverse: false,
            cursor_visible: true,
            scroll_top: 0,
            scroll_bottom: rows.saturating_sub(1),
        }
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn cols(&self) -> usize {
        self.cols
    }

    pub fn cursor_row(&self) -> usize {
        self.cursor_row
    }

    pub fn cursor_col(&self) -> usize {
        self.cursor_col
    }

    pub fn cursor_visible(&self) -> bool {
        self.cursor_visible
    }

    pub fn set_cursor_visible(&mut self, visible: bool) {
        self.cursor_visible = visible;
    }

    /// Get a reference to the visible cells.
    pub fn cells(&self) -> &[Vec<Cell>] {
        &self.cells
    }

    /// Get the scrollback lines.
    pub fn scrollback(&self) -> &VecDeque<Vec<Cell>> {
        &self.scrollback
    }

    // --- Current attribute accessors/setters (for ANSI parser) ---

    pub fn current_fg(&self) -> Color {
        self.current_fg
    }

    pub fn set_current_fg(&mut self, c: Color) {
        self.current_fg = c;
    }

    pub fn current_bg(&self) -> Color {
        self.current_bg
    }

    pub fn set_current_bg(&mut self, c: Color) {
        self.current_bg = c;
    }

    pub fn reset_attributes(&mut self) {
        self.current_fg = DEFAULT_FG;
        self.current_bg = DEFAULT_BG;
        self.current_bold = false;
        self.current_italic = false;
        self.current_underline = false;
        self.current_reverse = false;
    }

    pub fn set_bold(&mut self, on: bool) {
        self.current_bold = on;
    }

    pub fn set_italic(&mut self, on: bool) {
        self.current_italic = on;
    }

    pub fn set_underline(&mut self, on: bool) {
        self.current_underline = on;
    }

    pub fn set_reverse(&mut self, on: bool) {
        self.current_reverse = on;
    }

    // --- Cursor operations ---

    /// Move cursor to a specific position (0-indexed).
    pub fn cursor_to(&mut self, row: usize, col: usize) {
        self.cursor_row = row.min(self.rows.saturating_sub(1));
        self.cursor_col = col.min(self.cols.saturating_sub(1));
    }

    /// Move cursor up by n lines (clamped, won't leave scroll region).
    pub fn cursor_up(&mut self, n: usize) {
        let min_row = self.scroll_top;
        self.cursor_row = self.cursor_row.saturating_sub(n).max(min_row);
    }

    /// Move cursor down by n lines.
    pub fn cursor_down(&mut self, n: usize) {
        let max_row = self.scroll_bottom;
        self.cursor_row = (self.cursor_row + n).min(max_row);
    }

    /// Move cursor right by n columns.
    pub fn cursor_right(&mut self, n: usize) {
        self.cursor_col = (self.cursor_col + n).min(self.cols.saturating_sub(1));
    }

    /// Move cursor left by n columns.
    pub fn cursor_left(&mut self, n: usize) {
        self.cursor_col = self.cursor_col.saturating_sub(n);
    }

    /// Carriage return: move cursor to column 0.
    pub fn carriage_return(&mut self) {
        self.cursor_col = 0;
    }

    /// Move cursor to the beginning of the next line.
    pub fn line_feed(&mut self) {
        if self.cursor_row == self.scroll_bottom {
            self.scroll_up(1);
        } else if self.cursor_row < self.rows - 1 {
            self.cursor_row += 1;
        }
    }

    /// Backspace: move cursor left by 1 (clamped at 0).
    pub fn backspace(&mut self) {
        if self.cursor_col > 0 {
            self.cursor_col -= 1;
        }
    }

    /// Tab: advance to the next 8-column tab stop.
    pub fn tab(&mut self) {
        let next = (self.cursor_col / 8 + 1) * 8;
        self.cursor_col = next.min(self.cols.saturating_sub(1));
    }

    // --- Cell operations ---

    /// Write a character at the cursor position using current attributes.
    /// Advances the cursor. Handles line wrapping.
    pub fn write_char(&mut self, ch: char) {
        // Ensure cursor is in bounds.
        if self.cursor_row >= self.rows || self.cursor_col >= self.cols {
            return;
        }

        let cell = &mut self.cells[self.cursor_row][self.cursor_col];
        cell.ch = ch;
        cell.fg = self.current_fg;
        cell.bg = self.current_bg;
        cell.bold = self.current_bold;
        cell.italic = self.current_italic;
        cell.underline = self.current_underline;
        cell.reverse = self.current_reverse;

        // Advance cursor with wrapping.
        self.cursor_col += 1;
        if self.cursor_col >= self.cols {
            // Wrap to next line.
            self.cursor_col = 0;
            self.line_feed();
        }
    }

    /// Clear the entire screen and reset cursor to (0,0).
    pub fn clear_screen(&mut self) {
        for row in &mut self.cells {
            for cell in row.iter_mut() {
                *cell = Cell::default();
            }
        }
        self.cursor_row = 0;
        self.cursor_col = 0;
    }

    /// Clear from cursor to end of screen.
    pub fn clear_to_end_of_screen(&mut self) {
        // Clear from cursor to end of current row.
        if self.cursor_row < self.rows {
            for col in self.cursor_col..self.cols {
                self.cells[self.cursor_row][col] = Cell::default();
            }
            // Clear all rows below.
            for row in (self.cursor_row + 1)..self.rows {
                for cell in &mut self.cells[row] {
                    *cell = Cell::default();
                }
            }
        }
    }

    /// Clear from start of screen to cursor.
    pub fn clear_to_cursor(&mut self) {
        // Clear all rows above.
        for row in 0..self.cursor_row {
            for cell in &mut self.cells[row] {
                *cell = Cell::default();
            }
        }
        // Clear from start of current row to cursor.
        if self.cursor_row < self.rows {
            for col in 0..=self.cursor_col.min(self.cols.saturating_sub(1)) {
                self.cells[self.cursor_row][col] = Cell::default();
            }
        }
    }

    /// Clear the current line.
    pub fn clear_line(&mut self) {
        if self.cursor_row < self.rows {
            for cell in &mut self.cells[self.cursor_row] {
                *cell = Cell::default();
            }
        }
    }

    /// Clear from cursor to end of line.
    pub fn clear_to_end_of_line(&mut self) {
        if self.cursor_row < self.rows {
            for col in self.cursor_col..self.cols {
                self.cells[self.cursor_row][col] = Cell::default();
            }
        }
    }

    /// Clear from start of line to cursor.
    pub fn clear_to_start_of_line(&mut self) {
        if self.cursor_row < self.rows {
            for col in 0..=self.cursor_col.min(self.cols.saturating_sub(1)) {
                self.cells[self.cursor_row][col] = Cell::default();
            }
        }
    }

    // --- Scrolling ---

    /// Scroll the scroll region up by n lines (content moves up, blank lines appear at bottom).
    pub fn scroll_up(&mut self, n: usize) {
        let top = self.scroll_top;
        let bottom = self.scroll_bottom;
        let n = n.min(bottom - top + 1);

        // Move lines up.
        for row in top..=(bottom - n) {
            self.cells.swap(row, row + n);
        }
        // Clear the bottom n lines.
        for row in (bottom + 1 - n)..=bottom {
            for cell in &mut self.cells[row] {
                *cell = Cell::default();
            }
        }
    }

    /// Scroll the scroll region down by n lines (content moves down, blank lines appear at top).
    pub fn scroll_down(&mut self, n: usize) {
        let top = self.scroll_top;
        let bottom = self.scroll_bottom;
        let n = n.min(bottom - top + 1);

        // Move lines down (iterate in reverse to avoid overwrite).
        for row in (top + n..=bottom).rev() {
            self.cells.swap(row, row - n);
        }
        // Clear the top n lines.
        for row in top..(top + n) {
            for cell in &mut self.cells[row] {
                *cell = Cell::default();
            }
        }
    }

    /// Set the scroll region (DECSTBM). 1-indexed in the protocol, converted to 0-indexed.
    pub fn set_scroll_region(&mut self, top: usize, bottom: usize) {
        let top = top.saturating_sub(1).min(self.rows.saturating_sub(1));
        let bottom = if bottom == 0 {
            self.rows.saturating_sub(1)
        } else {
            (bottom - 1).min(self.rows.saturating_sub(1))
        };
        if top < bottom {
            self.scroll_top = top;
            self.scroll_bottom = bottom;
            // Move cursor to home.
            self.cursor_row = 0;
            self.cursor_col = 0;
        }
    }

    // --- Line operations ---

    /// Insert n blank lines at the cursor row (within scroll region).
    pub fn insert_lines(&mut self, n: usize) {
        if self.cursor_row < self.scroll_top || self.cursor_row > self.scroll_bottom {
            return;
        }
        let n = n.min(self.scroll_bottom - self.cursor_row + 1);
        // Shift lines down.
        for row in (self.cursor_row + n..=self.scroll_bottom).rev() {
            self.cells.swap(row, row - n);
        }
        // Clear the inserted lines.
        for row in self.cursor_row..(self.cursor_row + n) {
            for cell in &mut self.cells[row] {
                *cell = Cell::default();
            }
        }
    }

    /// Delete n lines at the cursor row (within scroll region).
    pub fn delete_lines(&mut self, n: usize) {
        if self.cursor_row < self.scroll_top || self.cursor_row > self.scroll_bottom {
            return;
        }
        let n = n.min(self.scroll_bottom - self.cursor_row + 1);
        // Shift lines up.
        for row in self.cursor_row..=(self.scroll_bottom - n) {
            self.cells.swap(row, row + n);
        }
        // Clear the bottom n lines.
        for row in (self.scroll_bottom + 1 - n)..=self.scroll_bottom {
            for cell in &mut self.cells[row] {
                *cell = Cell::default();
            }
        }
    }

    /// Delete n characters at the cursor position (shifts remaining chars left).
    pub fn delete_chars(&mut self, n: usize) {
        if self.cursor_row >= self.rows || self.cursor_col >= self.cols {
            return;
        }
        let n = n.min(self.cols - self.cursor_col);
        let row = &mut self.cells[self.cursor_row];
        for col in self.cursor_col..(self.cols - n) {
            row.swap(col, col + n);
        }
        for col in (self.cols - n)..self.cols {
            row[col] = Cell::default();
        }
    }

    /// Insert n blank characters at the cursor position (shifts remaining chars right).
    pub fn insert_chars(&mut self, n: usize) {
        if self.cursor_row >= self.rows || self.cursor_col >= self.cols {
            return;
        }
        let n = n.min(self.cols - self.cursor_col);
        let row = &mut self.cells[self.cursor_row];
        for col in (self.cursor_col + n..self.cols).rev() {
            row.swap(col, col - n);
        }
        for col in self.cursor_col..(self.cursor_col + n) {
            row[col] = Cell::default();
        }
    }

    /// Erase n characters from the cursor position (replaces with blanks, doesn't shift).
    pub fn erase_chars(&mut self, n: usize) {
        if self.cursor_row >= self.rows {
            return;
        }
        let end = (self.cursor_col + n).min(self.cols);
        for col in self.cursor_col..end {
            self.cells[self.cursor_row][col] = Cell::default();
        }
    }

    // --- Resize ---

    /// Resize the grid. Preserves existing content where possible.
    pub fn resize(&mut self, new_rows: usize, new_cols: usize) {
        if new_rows == self.rows && new_cols == self.cols {
            return;
        }

        // Adjust columns in each existing row.
        for row in &mut self.cells {
            if new_cols > self.cols {
                row.resize(new_cols, Cell::default());
            } else if new_cols < self.cols {
                row.truncate(new_cols);
            }
        }

        // Adjust number of rows.
        if new_rows > self.rows {
            // Move excess rows to scrollback if shrinking, or add blank rows if growing.
            for _ in self.rows..new_rows {
                self.cells.push(vec![Cell::default(); new_cols]);
            }
        } else if new_rows < self.rows {
            // Move removed rows to scrollback.
            for row in self.cells.drain(new_rows..) {
                self.scrollback.push_back(row);
                if self.scrollback.len() > self.max_scrollback {
                    self.scrollback.pop_front();
                }
            }
        }

        self.rows = new_rows;
        self.cols = new_cols;
        self.cursor_row = self.cursor_row.min(new_rows.saturating_sub(1));
        self.cursor_col = self.cursor_col.min(new_cols.saturating_sub(1));
        self.scroll_top = 0;
        self.scroll_bottom = new_rows.saturating_sub(1);
    }
}
