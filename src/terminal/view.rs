//! Terminal view: renders the terminal grid in gpui and handles keyboard input.

use gpui::{
    actions, div, font, px, rgb, App, Bounds, ClipboardItem, Context, EventEmitter, FocusHandle,
    Focusable, InteractiveElement, IntoElement, KeyBinding, KeyDownEvent, Keystroke, MouseButton,
    MouseDownEvent, MouseMoveEvent, ParentElement, Pixels, Render, SharedString, Styled,
    StyledText, Task, TextRun, Window,
};
use std::time::{Duration, Instant};

use gpui_component::dock::{Panel, PanelEvent};
use gpui_component::ActiveTheme as _;

use crate::config::SshConnection;
use crate::ssh::session::{SessionStatus, SshSession};
use crate::terminal::ansi::TerminalBackend;
use crate::terminal::grid::{Cell, Color, DEFAULT_BG, DEFAULT_FG};

const TERMINAL_KEY_CONTEXT: &str = "Terminal";
const SELECTION_FG: Color = Color::rgb(0xff, 0xff, 0xff);
const SELECTION_BG: Color = Color::rgb(0x3d, 0x59, 0x82);

actions!(terminal, [SendTab, SendBackTab, CopySelection]);

enum TerminalEvent {
    Output(Vec<u8>),
    Status(SessionStatus),
    Closed,
}

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", SendTab, Some(TERMINAL_KEY_CONTEXT)),
        KeyBinding::new("shift-tab", SendBackTab, Some(TERMINAL_KEY_CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-c", CopySelection, Some(TERMINAL_KEY_CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-shift-c", CopySelection, Some(TERMINAL_KEY_CONTEXT)),
    ]);
}

/// The terminal view: a gpui view that displays terminal output and captures keyboard input.
pub struct TerminalView {
    /// The vte ANSI parser + terminal grid backend.
    backend: TerminalBackend,
    /// The vte parser instance (separate from backend to avoid borrow conflicts).
    parser: vte::Parser,
    /// The SSH session (if connected).
    session: Option<SshSession>,
    /// Current session status.
    status: SessionStatus,
    /// Connection info for display.
    pub connection: SshConnection,
    /// Focus handle for keyboard input.
    focus_handle: FocusHandle,
    /// Selection anchor/head in visible grid coordinates.
    selection_anchor: Option<(usize, usize)>,
    selection_head: Option<(usize, usize)>,
    selection_dragging: bool,
    selection_changed: bool,
    /// Cached row bounds from GPUI prepaint for mouse selection hit testing.
    row_bounds: Vec<Bounds<Pixels>>,
    /// Measured monospace cell width for mouse selection hit testing.
    cell_width: Pixels,
    /// Used to dedupe Tab if both a key binding action and key_down are delivered.
    last_tab_action_at: Option<Instant>,
    last_back_tab_action_at: Option<Instant>,
    /// The output event task (kept alive to prevent cancellation).
    _event_sub: Option<Task<()>>,
}

impl TerminalView {
    /// Create a new terminal view and connect to the given SSH host.
    pub fn new(
        runtime: &tokio::runtime::Handle,
        connection: SshConnection,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let backend = TerminalBackend::new(24, 80);
        let parser = vte::Parser::new();
        let focus_handle = cx.focus_handle();

        // Create SSH session.
        let session = SshSession::connect(runtime, &connection);
        let output_rx = session.output_receiver();
        let status_rx = session.status_receiver();

        // Wait for SSH events instead of polling while idle.
        let poll = cx.spawn(async move |this, cx| {
            loop {
                let output_rx = output_rx.clone();
                let status_rx = status_rx.clone();
                let event = cx
                    .background_executor()
                    .spawn(async move { wait_for_session_event(output_rx, status_rx) })
                    .await;

                let result = this.update(cx, |this, cx| {
                    let mut had_data = false;
                    let mut had_status = false;
                    let closed = matches!(event, TerminalEvent::Closed);

                    match event {
                        TerminalEvent::Output(data) => {
                            this.apply_output(&data);
                            had_data = true;
                        }
                        TerminalEvent::Status(status) => {
                            this.status = status;
                            had_status = true;
                        }
                        TerminalEvent::Closed => {}
                    }

                    // Drain anything that arrived while the UI update was queued.
                    let mut output_batches = Vec::new();
                    let mut statuses = Vec::new();
                    if let Some(session) = &this.session {
                        while let Some(data) = session.try_recv_output() {
                            output_batches.push(data);
                        }

                        while let Some(status) = session.try_recv_status() {
                            statuses.push(status);
                        }
                    }
                    for data in output_batches {
                        this.apply_output(&data);
                        had_data = true;
                    }
                    for status in statuses {
                        this.status = status;
                        had_status = true;
                    }

                    if had_data || had_status {
                        cx.notify();
                    }

                    closed
                });

                match result {
                    Ok(false) => {}
                    Ok(true) | Err(_) => {
                        // The session ended or the view was dropped.
                        break;
                    }
                }
            }
        });

        Self {
            backend,
            parser,
            session: Some(session),
            status: SessionStatus::Connecting,
            connection,
            focus_handle,
            selection_anchor: None,
            selection_head: None,
            selection_dragging: false,
            selection_changed: false,
            row_bounds: Vec::new(),
            cell_width: px(1.0),
            last_tab_action_at: None,
            last_back_tab_action_at: None,
            _event_sub: Some(poll),
        }
    }

    fn apply_output(&mut self, data: &[u8]) {
        for &byte in data {
            self.parser.advance(&mut self.backend, byte);
        }
    }

    fn send_input_bytes(&self, bytes: &[u8], cx: &mut Context<Self>) {
        if let Some(session) = &self.session {
            session.send_input(bytes.to_vec());
        }
        cx.stop_propagation();
    }

    fn on_action_send_tab(&mut self, _: &SendTab, _window: &mut Window, cx: &mut Context<Self>) {
        self.last_tab_action_at = Some(Instant::now());
        self.send_input_bytes(b"\t", cx);
    }

    fn on_action_send_back_tab(
        &mut self,
        _: &SendBackTab,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.last_back_tab_action_at = Some(Instant::now());
        self.send_input_bytes(b"\x1b[Z", cx);
    }

    fn on_action_copy_selection(
        &mut self,
        _: &CopySelection,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(text) = self.selected_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }

        cx.stop_propagation();
    }

    /// Handle a key down event: convert to terminal escape sequence and send to SSH.
    pub fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = &self.session else { return };

        let ks = &event.keystroke;
        if ks.key == "tab" {
            let last_action = if ks.modifiers.shift {
                &mut self.last_back_tab_action_at
            } else {
                &mut self.last_tab_action_at
            };

            let action_already_sent = last_action
                .take()
                .is_some_and(|sent_at| sent_at.elapsed() < Duration::from_millis(100));
            if !action_already_sent {
                if ks.modifiers.shift {
                    session.send_input(b"\x1b[Z".to_vec());
                } else {
                    session.send_input(b"\t".to_vec());
                }
            }

            cx.stop_propagation();
            return;
        }

        let data = keystroke_to_bytes(ks);

        if !data.is_empty() {
            session.send_input(data);
            cx.stop_propagation();
        }
    }

    /// Convert a terminal Color to a gpui color.
    fn color_to_gpui(c: Color) -> gpui::Rgba {
        rgb(((c.r as u32) << 16) | ((c.g as u32) << 8) | (c.b as u32))
    }

    fn color_to_hsla(c: Color) -> gpui::Hsla {
        Self::color_to_gpui(c).into()
    }

    fn begin_selection(&mut self, row: usize, col: usize, cx: &mut Context<Self>) {
        self.selection_anchor = Some((row, col));
        self.selection_head = Some((row, col));
        self.selection_dragging = true;
        self.selection_changed = false;
        cx.notify();
        cx.stop_propagation();
    }

    fn update_row_bounds(&mut self, bounds: Vec<Bounds<Pixels>>) {
        self.row_bounds = bounds;
    }

    fn update_cell_width(
        &mut self,
        mono_font: SharedString,
        mono_size: Pixels,
        window: &mut Window,
        cx: &App,
    ) {
        let sample = SharedString::new_static("M");
        let shaped = window.text_system().shape_line(
            sample.clone(),
            mono_size,
            &[TextRun {
                len: sample.len(),
                font: font(mono_font),
                color: cx.theme().foreground,
                background_color: None,
                underline: None,
                strikethrough: None,
            }],
            None,
        );

        self.cell_width = px(window.pixel_snap(shaped.width).as_f32().max(1.0));
    }

    fn cell_at_position(&self, position: gpui::Point<Pixels>) -> Option<(usize, usize)> {
        let rows = self.backend.grid.cells();
        for (row_idx, bounds) in self.row_bounds.iter().enumerate() {
            if row_idx >= rows.len() {
                break;
            }

            let row_top = bounds.origin.y;
            let row_bottom = bounds.origin.y + bounds.size.height;
            if position.y < row_top || position.y >= row_bottom {
                continue;
            }

            let cols = rows[row_idx].len();
            if cols == 0 {
                return None;
            }

            let relative_x = position.x - bounds.origin.x;
            let col = col_at_x(relative_x, self.cell_width, cols)?;
            return Some((row_idx, col));
        }

        None
    }

    fn handle_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_handle.focus(window, cx);
        window.prevent_default();

        if let Some((row, col)) = self.cell_at_position(event.position) {
            self.begin_selection(row, col, cx);
        } else {
            self.selection_anchor = None;
            self.selection_head = None;
            cx.notify();
            cx.stop_propagation();
        }
    }

    fn handle_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !event.dragging() {
            return;
        }

        if let Some((row, col)) = self.cell_at_position(event.position) {
            self.extend_selection(row, col, cx);
        }
    }

    fn extend_selection(&mut self, row: usize, col: usize, cx: &mut Context<Self>) {
        if !self.selection_dragging {
            return;
        }

        let position = (row, col);
        if self.selection_head != Some(position) {
            self.selection_head = Some(position);
            self.selection_changed = true;
            cx.notify();
        }

        cx.stop_propagation();
    }

    fn finish_selection(&mut self, cx: &mut Context<Self>) {
        if !self.selection_dragging {
            return;
        }

        self.selection_dragging = false;
        if !self.selection_changed {
            self.selection_anchor = None;
            self.selection_head = None;
        }

        cx.notify();
        cx.stop_propagation();
    }

    fn normalized_selection(&self) -> Option<((usize, usize), (usize, usize))> {
        let anchor = self.selection_anchor?;
        let head = self.selection_head?;

        if anchor <= head {
            Some((anchor, head))
        } else {
            Some((head, anchor))
        }
    }

    fn is_cell_selected(&self, row: usize, col: usize) -> bool {
        let Some(((start_row, start_col), (end_row, end_col))) = self.normalized_selection() else {
            return false;
        };

        if row < start_row || row > end_row {
            return false;
        }

        let after_start = row > start_row || col >= start_col;
        let before_end = row < end_row || col <= end_col;
        after_start && before_end
    }

    fn selected_text(&self) -> Option<String> {
        let ((start_row, start_col), (end_row, end_col)) = self.normalized_selection()?;
        let cells = self.backend.grid.cells();
        if cells.is_empty() {
            return None;
        }

        let mut lines = Vec::new();
        let end_row = end_row.min(cells.len().saturating_sub(1));
        for row_idx in start_row.min(end_row)..=end_row {
            let row = &cells[row_idx];
            if row.is_empty() {
                lines.push(String::new());
                continue;
            }

            let first_col = if row_idx == start_row { start_col } else { 0 };
            let last_col = if row_idx == end_row {
                end_col
            } else {
                row.len().saturating_sub(1)
            };

            if first_col >= row.len() || first_col > last_col {
                lines.push(String::new());
                continue;
            }

            let last_col = last_col.min(row.len().saturating_sub(1));
            let mut line = row[first_col..=last_col]
                .iter()
                .map(|cell| cell.ch)
                .collect::<String>();
            while line.ends_with(' ') {
                line.pop();
            }
            lines.push(line);
        }

        let text = lines.join("\n");
        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    }

    /// Render a single row of the terminal grid as one shaped text element.
    fn render_row(
        &self,
        row: &[Cell],
        row_idx: usize,
        mono_font: SharedString,
    ) -> impl IntoElement {
        let cursor_row = self.backend.grid.cursor_row();
        let cursor_col = self.backend.grid.cursor_col();
        let cursor_visible = self.backend.grid.cursor_visible();
        let terminal_font = font(mono_font);

        let mut text = String::with_capacity(row.len());
        let mut runs = Vec::new();
        for (col, cell) in row.iter().enumerate() {
            let selected = self.is_cell_selected(row_idx, col);
            let (mut fg, mut bg) = if cursor_visible && row_idx == cursor_row && col == cursor_col {
                (cell.effective_bg(), cell.effective_fg()) // Swap fg/bg for cursor
            } else {
                (cell.effective_fg(), cell.effective_bg())
            };

            if selected {
                fg = SELECTION_FG;
                bg = SELECTION_BG;
            }

            text.push(cell.ch);
            push_text_run(
                &mut runs,
                TextRun {
                    len: cell.ch.len_utf8(),
                    font: terminal_font.clone(),
                    color: Self::color_to_hsla(fg),
                    background_color: (bg != DEFAULT_BG).then(|| Self::color_to_hsla(bg)),
                    underline: None,
                    strikethrough: None,
                },
            );
        }

        // If the cursor is at the end of the line, add a space for the cursor.
        if cursor_visible && row_idx == cursor_row && cursor_col >= row.len() {
            let bg = DEFAULT_FG;
            text.push(' ');
            push_text_run(
                &mut runs,
                TextRun {
                    len: 1,
                    font: terminal_font,
                    color: Self::color_to_hsla(bg),
                    background_color: Some(Self::color_to_hsla(bg)),
                    underline: None,
                    strikethrough: None,
                },
            );
        }

        div()
            .h(px(20.0))
            .whitespace_nowrap()
            .child(StyledText::new(text).with_runs(runs))
    }
}

fn wait_for_session_event(
    output_rx: crossbeam_channel::Receiver<Vec<u8>>,
    status_rx: crossbeam_channel::Receiver<SessionStatus>,
) -> TerminalEvent {
    crossbeam_channel::select! {
        recv(output_rx) -> msg => match msg {
            Ok(data) => TerminalEvent::Output(data),
            Err(_) => TerminalEvent::Closed,
        },
        recv(status_rx) -> msg => match msg {
            Ok(status) => TerminalEvent::Status(status),
            Err(_) => TerminalEvent::Closed,
        },
    }
}

impl Focusable for TerminalView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<PanelEvent> for TerminalView {}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let mono_font = theme.mono_font_family.clone();
        let mono_size = theme.mono_font_size;
        self.update_cell_width(mono_font.clone(), mono_size, window, cx);
        let cells = self.backend.grid.cells();
        let rows = cells
            .iter()
            .enumerate()
            .map(|(i, row)| self.render_row(row, i, mono_font.clone()))
            .collect::<Vec<_>>();
        let terminal = cx.entity();

        div()
            .size_full()
            .flex_1()
            .h_full()
            .bg(rgb(((DEFAULT_BG.r as u32) << 16)
                | ((DEFAULT_BG.g as u32) << 8)
                | (DEFAULT_BG.b as u32)))
            .p(px(4.0))
            .flex()
            .flex_col()
            .font_family(mono_font)
            .text_size(mono_size)
            .line_height(px(20.0))
            .track_focus(&self.focus_handle)
            .key_context(TERMINAL_KEY_CONTEXT)
            .on_action(cx.listener(Self::on_action_send_tab))
            .on_action(cx.listener(Self::on_action_send_back_tab))
            .on_action(cx.listener(Self::on_action_copy_selection))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::handle_mouse_down))
            .on_mouse_move(cx.listener(Self::handle_mouse_move))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.finish_selection(cx);
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.finish_selection(cx);
                }),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.handle_key_down(event, window, cx);
            }))
            .on_children_prepainted(move |bounds, _, cx| {
                terminal.update(cx, |this, _| {
                    this.update_row_bounds(bounds);
                });
            })
            .children(rows)
    }
}

impl Panel for TerminalView {
    fn panel_name(&self) -> &'static str {
        "terminal"
    }
}

fn push_text_run(runs: &mut Vec<TextRun>, run: TextRun) {
    if run.len == 0 {
        return;
    }

    if let Some(last) = runs.last_mut() {
        if last.font == run.font
            && last.color == run.color
            && last.background_color == run.background_color
            && last.underline == run.underline
            && last.strikethrough == run.strikethrough
        {
            last.len += run.len;
            return;
        }
    }

    runs.push(run);
}

/// Convert a gpui Keystroke to terminal bytes (escape sequences for special keys).
fn keystroke_to_bytes(ks: &Keystroke) -> Vec<u8> {
    // Special keys.
    match ks.key.as_str() {
        "enter" | "return" => return b"\r".to_vec(),
        "backspace" => return b"\x7f".to_vec(),
        "escape" => return b"\x1b".to_vec(),
        "up" => return b"\x1b[A".to_vec(),
        "down" => return b"\x1b[B".to_vec(),
        "right" => return b"\x1b[C".to_vec(),
        "left" => return b"\x1b[D".to_vec(),
        "home" => return b"\x1b[H".to_vec(),
        "end" => return b"\x1b[F".to_vec(),
        "delete" => return b"\x1b[3~".to_vec(),
        "pageup" => return b"\x1b[5~".to_vec(),
        "pagedown" => return b"\x1b[6~".to_vec(),
        "insert" => return b"\x1b[2~".to_vec(),
        _ => {}
    }

    if ks.modifiers.platform {
        return Vec::new();
    }

    // Ctrl+key combinations: Ctrl+A = 0x01, Ctrl+B = 0x02, etc.
    if ks.modifiers.control && !ks.modifiers.alt && !ks.modifiers.platform {
        // For Ctrl+key, use the key character.
        let key_lower = ks.key.to_lowercase();
        if let Some(c) = key_lower.chars().next() {
            if c.is_ascii_alphabetic() {
                return vec![(c as u8) - b'a' + 1];
            }
        }
        // Ctrl+special combos
        match ks.key.as_str() {
            "space" => return vec![0x00],
            "[" => return vec![0x1b],
            "\\" => return vec![0x1c],
            "]" => return vec![0x1d],
            "^" => return vec![0x1e],
            "_" => return vec![0x1f],
            _ => {}
        }
    }

    // Regular character input: use key_char if available.
    if let Some(ch) = &ks.key_char {
        // Alt prefix (Meta key in terminals).
        if ks.modifiers.alt {
            let mut bytes = vec![0x1b];
            bytes.extend(ch.as_bytes());
            return bytes;
        }
        return ch.as_bytes().to_vec();
    }

    // Fallback: no data to send.
    Vec::new()
}

fn col_at_x(relative_x: Pixels, cell_width: Pixels, cols: usize) -> Option<usize> {
    if cols == 0 {
        return None;
    }

    let cell_width = px(cell_width.as_f32().max(1.0));
    let col = (relative_x / cell_width).floor() as isize;
    Some(col.clamp(0, cols.saturating_sub(1) as isize) as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn col_at_x_uses_measured_cell_width() {
        assert_eq!(col_at_x(px(45.0), px(9.0), 80), Some(5));
    }

    #[test]
    fn col_at_x_clamps_outside_row() {
        assert_eq!(col_at_x(px(-8.0), px(9.0), 80), Some(0));
        assert_eq!(col_at_x(px(900.0), px(9.0), 80), Some(79));
    }
}
