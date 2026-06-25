//! Terminal view: renders the terminal grid in gpui and handles keyboard input.

use gpui::{
    actions, div, px, rgb, App, Context, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, KeyBinding, KeyDownEvent, Keystroke, MouseButton,
    ParentElement, Render, Styled, Task, Window,
};
use gpui_component::dock::{Panel, PanelEvent};
use gpui_component::ActiveTheme as _;

use crate::config::SshConnection;
use crate::ssh::session::{SessionStatus, SshSession};
use crate::terminal::ansi::TerminalBackend;
use crate::terminal::grid::{Cell, Color, DEFAULT_BG, DEFAULT_FG};

const TERMINAL_KEY_CONTEXT: &str = "Terminal";

actions!(terminal, [SendTab, SendBackTab]);

enum TerminalEvent {
    Output(Vec<u8>),
    Status(SessionStatus),
    Closed,
}

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", SendTab, Some(TERMINAL_KEY_CONTEXT)),
        KeyBinding::new("shift-tab", SendBackTab, Some(TERMINAL_KEY_CONTEXT)),
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

    fn on_action_send_tab(
        &mut self,
        _: &SendTab,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.send_input_bytes(b"\t", cx);
    }

    fn on_action_send_back_tab(
        &mut self,
        _: &SendBackTab,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.send_input_bytes(b"\x1b[Z", cx);
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

    /// Render a single row of the terminal grid as a horizontal flex of text segments.
    fn render_row(&self, row: &[Cell], row_idx: usize) -> impl IntoElement {
        let cursor_row = self.backend.grid.cursor_row();
        let cursor_col = self.backend.grid.cursor_col();
        let cursor_visible = self.backend.grid.cursor_visible();

        // Group consecutive cells with the same style into text segments.
        let mut segments: Vec<(String, Color, Color)> = Vec::new(); // (text, fg, bg)
        for (col, cell) in row.iter().enumerate() {
            // Handle cursor: reverse the cell at the cursor position.
            let (fg, bg) = if cursor_visible && row_idx == cursor_row && col == cursor_col {
                (cell.effective_bg(), cell.effective_fg()) // Swap fg/bg for cursor
            } else {
                (cell.effective_fg(), cell.effective_bg())
            };

            if let Some(last) = segments.last_mut() {
                if last.1 == fg && last.2 == bg {
                    last.0.push(cell.ch);
                    continue;
                }
            }
            segments.push((cell.ch.to_string(), fg, bg));
        }

        // If the cursor is at the end of the line, add a space for the cursor.
        if cursor_visible && row_idx == cursor_row && cursor_col >= row.len() {
            let bg = DEFAULT_FG; // Cursor block color
            segments.push((" ".to_string(), bg, bg));
        }

        div()
            .flex()
            .flex_row()
            .h(px(20.0))
            .children(segments.into_iter().map(|(text, fg, bg)| {
                let bg_differs = bg != DEFAULT_BG;
                let el = div().text_color(Self::color_to_gpui(fg)).child(text);
                if bg_differs {
                    el.bg(Self::color_to_gpui(bg))
                } else {
                    el
                }
            }))
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let cells = self.backend.grid.cells();
        let theme = cx.theme();
        let mono_font = theme.mono_font_family.clone();
        let mono_size = theme.mono_font_size;

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
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.focus_handle.focus(window, cx);
                    window.prevent_default();
                    cx.stop_propagation();
                }),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.handle_key_down(event, window, cx);
            }))
            .children(
                cells
                    .iter()
                    .enumerate()
                    .map(|(i, row)| self.render_row(row, i)),
            )
    }
}

impl Panel for TerminalView {
    fn panel_name(&self) -> &'static str {
        "terminal"
    }
}

/// Convert a gpui Keystroke to terminal bytes (escape sequences for special keys).
fn keystroke_to_bytes(ks: &Keystroke) -> Vec<u8> {
    // Special keys.
    match ks.key.as_str() {
        "enter" | "return" => return b"\r".to_vec(),
        "backspace" => return b"\x7f".to_vec(),
        "tab" => return b"\t".to_vec(),
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
