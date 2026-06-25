//! ANSI escape sequence parser using vte.
//!
//! Implements `vte::Perform` to translate escape sequences into grid operations.

use vte::{Perform, Params};

use crate::terminal::grid::{TerminalGrid, Color, xterm_256, ANSI_16, DEFAULT_FG, DEFAULT_BG};

/// The terminal backend: holds the grid and implements the vte Perform trait.
/// The parser (vte::Parser) is kept separately to avoid borrow conflicts.
pub struct TerminalBackend {
    pub grid: TerminalGrid,
    /// Terminal title (set by OSC 0/2).
    pub title: String,
    /// Whether we're in the alternate screen.
    alternate_screen: bool,
    /// Saved main screen (for alternate screen restore).
    saved_grid: Option<TerminalGrid>,
}

impl TerminalBackend {
    pub fn new(rows: usize, cols: usize) -> Self {
        Self {
            grid: TerminalGrid::new(rows, cols),
            title: String::new(),
            alternate_screen: false,
            saved_grid: None,
        }
    }

    pub fn resize(&mut self, rows: usize, cols: usize) {
        self.grid.resize(rows, cols);
    }

    pub fn rows(&self) -> usize {
        self.grid.rows()
    }

    pub fn cols(&self) -> usize {
        self.grid.cols()
    }
}

/// Helper: extract a parameter value at the given index, or return default.
fn get_param(params: &Params, idx: usize, default: u16) -> u16 {
    let mut iter = params.iter();
    for _ in 0..idx {
        iter.next();
    }
    iter.next()
        .and_then(|p| p.first().copied())
        .unwrap_or(default)
}

/// Helper: collect all top-level params into a Vec for easy indexing.
fn collect_params(params: &Params) -> Vec<u16> {
    params.iter()
        .map(|p| p.first().copied().unwrap_or(0))
        .collect()
}

impl Perform for TerminalBackend {
    fn print(&mut self, c: char) {
        self.grid.write_char(c);
    }

    fn execute(&mut self, byte: u8) {
        // C0 control characters
        match byte {
            0x08 => self.grid.backspace(),        // BS  - Backspace
            0x09 => self.grid.tab(),              // HT  - Horizontal Tab
            0x0A => self.grid.line_feed(),        // LF  - Line Feed
            0x0B => self.grid.line_feed(),        // VT  - Vertical Tab (treated as LF)
            0x0C => self.grid.line_feed(),        // FF  - Form Feed (treated as LF)
            0x0D => self.grid.carriage_return(),  // CR  - Carriage Return
            0x07 => { /* BEL - bell, ignore */ }
            _ => { /* ignore other control chars */ }
        }
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], _ignore: bool, c: char) {
        let p = collect_params(params);

        // Check for private mode (intermediate '?')
        let is_private = intermediates.contains(&b'?');

        if is_private {
            // Private mode set/reset: ESC[?{n}h or ESC[?{n}l
            match c {
                'h' => {
                    for &mode in &p {
                        match mode {
                            25 => self.grid.set_cursor_visible(true),
                            1049 => {
                                // Enter alternate screen
                                if !self.alternate_screen {
                                    let rows = self.grid.rows();
                                    let cols = self.grid.cols();
                                    self.saved_grid = Some(std::mem::replace(
                                        &mut self.grid,
                                        TerminalGrid::new(rows, cols),
                                    ));
                                    self.alternate_screen = true;
                                }
                            }
                            2004 => { /* Enable bracketed paste mode - TODO */ }
                            _ => {}
                        }
                    }
                }
                'l' => {
                    for &mode in &p {
                        match mode {
                            25 => self.grid.set_cursor_visible(false),
                            1049 => {
                                // Leave alternate screen
                                if self.alternate_screen {
                                    if let Some(saved) = self.saved_grid.take() {
                                        self.grid = saved;
                                    }
                                    self.alternate_screen = false;
                                }
                            }
                            2004 => { /* Disable bracketed paste mode - TODO */ }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
            return;
        }

        match c {
            // Cursor movement
            'A' => { // CUU - Cursor Up
                let n = get_param(params, 0, 1) as usize;
                self.grid.cursor_up(n);
            }
            'B' => { // CUD - Cursor Down
                let n = get_param(params, 0, 1) as usize;
                self.grid.cursor_down(n);
            }
            'C' => { // CUF - Cursor Forward (Right)
                let n = get_param(params, 0, 1) as usize;
                self.grid.cursor_right(n);
            }
            'D' => { // CUB - Cursor Back (Left)
                let n = get_param(params, 0, 1) as usize;
                self.grid.cursor_left(n);
            }
            'E' => { // CNL - Cursor Next Line
                let n = get_param(params, 0, 1) as usize;
                self.grid.cursor_down(n);
                self.grid.carriage_return();
            }
            'F' => { // CPL - Cursor Previous Line
                let n = get_param(params, 0, 1) as usize;
                self.grid.cursor_up(n);
                self.grid.carriage_return();
            }
            'G' => { // CHA - Cursor Horizontal Absolute
                let col = get_param(params, 0, 1) as usize;
                self.grid.cursor_to(self.grid.cursor_row(), col.saturating_sub(1));
            }
            'H' | 'f' => { // CUP - Cursor Position
                let row = get_param(params, 0, 1) as usize;
                let col = get_param(params, 1, 1) as usize;
                self.grid.cursor_to(row.saturating_sub(1), col.saturating_sub(1));
            }
            'd' => { // VPA - Vertical Position Absolute
                let row = get_param(params, 0, 1) as usize;
                self.grid.cursor_to(row.saturating_sub(1), self.grid.cursor_col());
            }

            // Erase
            'J' => { // ED - Erase in Display
                let mode = get_param(params, 0, 0);
                match mode {
                    0 => self.grid.clear_to_end_of_screen(),
                    1 => self.grid.clear_to_cursor(),
                    2 | 3 => self.grid.clear_screen(),
                    _ => {}
                }
            }
            'K' => { // EL - Erase in Line
                let mode = get_param(params, 0, 0);
                match mode {
                    0 => self.grid.clear_to_end_of_line(),
                    1 => self.grid.clear_to_start_of_line(),
                    2 => self.grid.clear_line(),
                    _ => {}
                }
            }

            // Line/character operations
            'L' => { // IL - Insert Lines
                let n = get_param(params, 0, 1) as usize;
                self.grid.insert_lines(n);
            }
            'M' => { // DL - Delete Lines
                let n = get_param(params, 0, 1) as usize;
                self.grid.delete_lines(n);
            }
            'P' => { // DCH - Delete Characters
                let n = get_param(params, 0, 1) as usize;
                self.grid.delete_chars(n);
            }
            '@' => { // ICH - Insert Characters
                let n = get_param(params, 0, 1) as usize;
                self.grid.insert_chars(n);
            }
            'X' => { // ECH - Erase Characters
                let n = get_param(params, 0, 1) as usize;
                self.grid.erase_chars(n);
            }

            // Scrolling
            'S' => { // SU - Scroll Up
                let n = get_param(params, 0, 1) as usize;
                self.grid.scroll_up(n);
            }
            'T' => { // SD - Scroll Down
                let n = get_param(params, 0, 1) as usize;
                self.grid.scroll_down(n);
            }

            // Scroll region
            'r' => { // DECSTBM - Set Scroll Region
                let top = get_param(params, 0, 1) as usize;
                let bottom = get_param(params, 1, 0) as usize;
                self.grid.set_scroll_region(top, bottom);
            }

            // SGR - Select Graphic Rendition (colors and attributes)
            'm' => {
                self.handle_sgr(&p);
            }

            _ => {
                // Unhandled CSI sequence - ignore
            }
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        if params.is_empty() {
            return;
        }

        // OSC 0 or 2: Set window title
        // Format: ESC ] 0 ; title BEL  or  ESC ] 2 ; title BEL
        let cmd = params[0];
        if cmd == b"0" || cmd == b"2" {
            if params.len() > 1 {
                if let Ok(title) = std::str::from_utf8(params[1]) {
                    self.title = title.to_string();
                }
            }
        }
        // Other OSC sequences (colors, etc.) are ignored for now
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], _ignore: bool, byte: u8) {
        match (intermediates, byte) {
            ([], b'M') => {
                // RI - Reverse Index (move cursor up one line, scroll if at top)
                if self.grid.cursor_row() == 0 {
                    self.grid.scroll_down(1);
                } else {
                    self.grid.cursor_up(1);
                }
            }
            ([], b'D') => {
                // IND - Index (move cursor down one line, scroll if at bottom)
                self.grid.line_feed();
            }
            ([], b'E') => {
                // NEL - Next Line
                self.grid.carriage_return();
                self.grid.line_feed();
            }
            ([b'7'], b'7') | ([], b'7') => {
                // DECSC - Save Cursor
                // TODO: save cursor position and attributes
            }
            ([], b'8') => {
                // DECRC - Restore Cursor
                // TODO: restore cursor position and attributes
            }
            ([b'('], b'B') => {
                // SCS - Set Character Set (US ASCII) - no-op for us
            }
            _ => {
                // Unhandled ESC sequence - ignore
            }
        }
    }
}

impl TerminalBackend {
    /// Handle SGR (Select Graphic Rendition) parameters.
    fn handle_sgr(&mut self, params: &[u16]) {
        if params.is_empty() {
            self.grid.reset_attributes();
            return;
        }

        let mut i = 0;
        while i < params.len() {
            let code = params[i];

            match code {
                0 => {
                    // Reset all attributes
                    self.grid.reset_attributes();
                }
                1 => self.grid.set_bold(true),
                2 => { /* Dim - treat as normal */ }
                3 => self.grid.set_italic(true),
                4 => self.grid.set_underline(true),
                5 => { /* Slow blink - ignore */ }
                7 => self.grid.set_reverse(true),
                9 => { /* Strikethrough - ignore */ }
                22 => self.grid.set_bold(false),
                23 => self.grid.set_italic(false),
                24 => self.grid.set_underline(false),
                25 => { /* Not blinking - ignore */ }
                27 => self.grid.set_reverse(false),
                29 => { /* Not strikethrough - ignore */ }

                // Standard foreground colors (30-37)
                30..=37 => {
                    let idx = (code - 30) as usize;
                    self.grid.set_current_fg(ANSI_16[idx]);
                }
                // Standard background colors (40-47)
                40..=47 => {
                    let idx = (code - 40) as usize;
                    self.grid.set_current_bg(ANSI_16[idx]);
                }
                // Bright foreground colors (90-97)
                90..=97 => {
                    let idx = (code - 90 + 8) as usize;
                    self.grid.set_current_fg(ANSI_16[idx]);
                }
                // Bright background colors (100-107)
                100..=107 => {
                    let idx = (code - 100 + 8) as usize;
                    self.grid.set_current_bg(ANSI_16[idx]);
                }

                38 | 48 => {
                    // Extended color: 38;5;n (256-color) or 38;2;r;g;b (RGB)
                    // Same for 48 (background)
                    if i + 1 < params.len() {
                        let color_mode = params[i + 1];
                        let color = if color_mode == 5 {
                            // 256-color mode: 38;5;n
                            if i + 2 < params.len() {
                                Some(xterm_256(params[i + 2] as u8))
                            } else {
                                None
                            }
                            // Advance past the two extra params
                        } else if color_mode == 2 {
                            // RGB mode: 38;2;r;g;b
                            if i + 4 < params.len() {
                                Some(Color::rgb(
                                    params[i + 2] as u8,
                                    params[i + 3] as u8,
                                    params[i + 4] as u8,
                                ))
                            } else {
                                None
                            }
                        } else {
                            None
                        };

                        if let Some(c) = color {
                            if code == 38 {
                                self.grid.set_current_fg(c);
                            } else {
                                self.grid.set_current_bg(c);
                            }
                        }

                        // Skip consumed parameters
                        if color_mode == 5 {
                            i += 2;
                        } else if color_mode == 2 {
                            i += 4;
                        }
                    }
                }

                39 => {
                    // Default foreground
                    self.grid.set_current_fg(DEFAULT_FG);
                }
                49 => {
                    // Default background
                    self.grid.set_current_bg(DEFAULT_BG);
                }

                _ => {
                    // Unknown SGR code - ignore
                }
            }
            i += 1;
        }
    }
}
