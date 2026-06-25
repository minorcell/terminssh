//! Main application view: sidebar + terminal area + connection dialog.

use std::sync::Arc;

use gpui::prelude::FluentBuilder;
use gpui::{
    div, px, AnyElement, AppContext, Context, Entity, IntoElement, ParentElement, Render, Styled,
    Window,
};
use gpui_component::{
    button::{Button, ButtonVariants as _},
    h_flex,
    input::InputState,
    v_flex, ActiveTheme as _, Icon, IconName, Sizable as _,
};
use parking_lot::Mutex;
use uuid::Uuid;

use crate::config::{AppConfig, AuthMethod, SshConnection};
use crate::terminal::view::TerminalView;
use crate::ui;

#[cfg(target_os = "macos")]
const TITLEBAR_SAFE_TOP: f32 = 30.0;
#[cfg(not(target_os = "macos"))]
const TITLEBAR_SAFE_TOP: f32 = 0.0;

/// The main application view, integrating sidebar, terminal tabs, and dialog.
pub struct AppView {
    /// Application configuration (shared, persisted to JSON).
    pub config: Arc<Mutex<AppConfig>>,
    /// Tokio runtime handle for spawning SSH sessions.
    pub runtime_handle: tokio::runtime::Handle,
    /// Active terminal views (one per tab).
    pub terminals: Vec<Entity<TerminalView>>,
    /// Index of the currently active terminal tab.
    pub active_terminal: usize,
    /// Whether the connection dialog is visible.
    pub show_dialog: bool,
    /// Pending delete confirmation: (connection id, connection name).
    pub delete_confirm: Option<(String, String)>,
    /// If editing an existing connection, its ID.
    pub editing_id: Option<String>,
    /// Dialog auth method selection: true = password, false = private key.
    pub auth_is_password: bool,
    // -- Dialog input states --
    pub name_input: Entity<InputState>,
    pub host_input: Entity<InputState>,
    pub port_input: Entity<InputState>,
    pub username_input: Entity<InputState>,
    pub password_input: Entity<InputState>,
    pub key_path_input: Entity<InputState>,
    pub group_input: Entity<InputState>,
    /// Search input for filtering connections in the sidebar.
    pub search_input: Entity<InputState>,
}

impl AppView {
    /// Create a new AppView with the given runtime and config.
    pub fn new(
        runtime_handle: tokio::runtime::Handle,
        config: Arc<Mutex<AppConfig>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name_input = cx.new(|cx| InputState::new(window, cx).placeholder("My Server"));
        let host_input = cx.new(|cx| InputState::new(window, cx).placeholder("192.168.1.1"));
        let port_input = cx.new(|cx| InputState::new(window, cx).placeholder("22"));
        let username_input = cx.new(|cx| InputState::new(window, cx).placeholder("root"));
        let password_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Password")
                .masked(true)
        });
        let key_path_input = cx.new(|cx| InputState::new(window, cx).placeholder("~/.ssh/id_rsa"));
        let group_input = cx.new(|cx| InputState::new(window, cx).placeholder("Production"));
        let search_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search connections..."));

        // Re-render AppView when search input changes, so sidebar filters connections.
        cx.observe(&search_input, |_, _, cx| {
            cx.notify();
        })
        .detach();

        Self {
            config,
            runtime_handle,
            terminals: Vec::new(),
            active_terminal: 0,
            show_dialog: false,
            delete_confirm: None,
            editing_id: None,
            auth_is_password: true,
            name_input,
            host_input,
            port_input,
            username_input,
            password_input,
            key_path_input,
            group_input,
            search_input,
        }
    }

    // -- Auth method helpers (called from connection_dialog) --

    pub fn auth_method_is_password(&self) -> bool {
        self.auth_is_password
    }

    pub fn auth_method_is_key(&self) -> bool {
        !self.auth_is_password
    }

    pub fn set_auth_method_password(&mut self, cx: &mut Context<Self>) {
        self.auth_is_password = true;
        cx.notify();
    }

    pub fn set_auth_method_key(&mut self, cx: &mut Context<Self>) {
        self.auth_is_password = false;
        cx.notify();
    }

    // -- Dialog control --

    pub fn close_dialog(&mut self, cx: &mut Context<Self>) {
        self.show_dialog = false;
        self.editing_id = None;
        cx.notify();
    }

    pub fn show_delete_confirm(&mut self, id: String, name: String, cx: &mut Context<Self>) {
        self.delete_confirm = Some((id, name));
        cx.notify();
    }

    pub fn close_delete_confirm(&mut self, cx: &mut Context<Self>) {
        self.delete_confirm = None;
        cx.notify();
    }

    pub fn confirm_delete_connection(&mut self, cx: &mut Context<Self>) {
        if let Some((id, _)) = self.delete_confirm.take() {
            self.delete_connection(&id, cx);
        } else {
            cx.notify();
        }
    }

    pub fn show_add_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editing_id = None;
        self.show_dialog = true;
        self.auth_is_password = true;

        self.name_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.host_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.port_input
            .update(cx, |s, cx| s.set_value("22", window, cx));
        self.username_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.password_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.key_path_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.group_input
            .update(cx, |s, cx| s.set_value("", window, cx));

        cx.notify();
    }

    pub fn show_edit_dialog(
        &mut self,
        conn: &SshConnection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.editing_id = Some(conn.id.clone());
        self.show_dialog = true;
        self.auth_is_password = matches!(conn.auth_method, AuthMethod::Password { .. });

        self.name_input
            .update(cx, |s, cx| s.set_value(&conn.name, window, cx));
        self.host_input
            .update(cx, |s, cx| s.set_value(&conn.host, window, cx));
        self.port_input
            .update(cx, |s, cx| s.set_value(&conn.port.to_string(), window, cx));
        self.username_input
            .update(cx, |s, cx| s.set_value(&conn.username, window, cx));

        match &conn.auth_method {
            AuthMethod::Password { password } => {
                self.password_input
                    .update(cx, |s, cx| s.set_value(password, window, cx));
            }
            AuthMethod::PrivateKey { key_path, .. } => {
                self.key_path_input
                    .update(cx, |s, cx| s.set_value(key_path, window, cx));
            }
        }

        let group = conn.group.as_deref().unwrap_or("");
        self.group_input
            .update(cx, |s, cx| s.set_value(group, window, cx));

        cx.notify();
    }

    pub fn save_connection(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let name = self.name_input.read(cx).value().to_string();
        let host = self.host_input.read(cx).value().to_string();
        let port: u16 = self.port_input.read(cx).value().parse().unwrap_or(22);
        let username = self.username_input.read(cx).value().to_string();
        let group = {
            let g = self.group_input.read(cx).value().to_string();
            if g.is_empty() {
                None
            } else {
                Some(g)
            }
        };

        let auth_method = if self.auth_is_password {
            AuthMethod::Password {
                password: self.password_input.read(cx).value().to_string(),
            }
        } else {
            AuthMethod::PrivateKey {
                key_path: self.key_path_input.read(cx).value().to_string(),
                passphrase: None,
            }
        };

        if let Some(id) = self.editing_id.take() {
            let conn = SshConnection {
                id,
                name,
                host,
                port,
                username,
                auth_method,
                group,
            };
            let _ = self.config.lock().update_connection(conn);
        } else {
            let conn = SshConnection {
                id: Uuid::new_v4().to_string(),
                name,
                host,
                port,
                username,
                auth_method,
                group,
            };
            let _ = self.config.lock().add_connection(conn);
        }

        self.close_dialog(cx);
        cx.notify();
    }

    pub fn delete_connection(&mut self, id: &str, cx: &mut Context<Self>) {
        let _ = self.config.lock().remove_connection(id);
        cx.notify();
    }

    // -- Terminal management --

    pub fn open_terminal(
        &mut self,
        conn: SshConnection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let runtime = self.runtime_handle.clone();
        let terminal = cx.new(|cx| TerminalView::new(&runtime, conn, window, cx));
        cx.focus_view(&terminal, window);
        self.terminals.push(terminal);
        self.active_terminal = self.terminals.len() - 1;
        cx.notify();
    }

    pub fn close_terminal(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.terminals.len() {
            self.terminals.remove(index);
            if self.active_terminal >= self.terminals.len() && !self.terminals.is_empty() {
                self.active_terminal = self.terminals.len() - 1;
            } else if self.terminals.is_empty() {
                self.active_terminal = 0;
            }
            cx.notify();
        }
    }

    // -- Rendering --

    /// Render the tab bar showing all open terminals.
    fn render_tab_bar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let tab_bar_bg = theme.tab_bar;
        let border_color = theme.border;
        let tab_active_bg = theme.tab_active;
        let active_text = theme.tab_active_foreground;
        let inactive_text = theme.tab_foreground;

        let mut tabs = h_flex()
            .w_full()
            .h(px(40.0))
            .bg(tab_bar_bg)
            .border_b_1()
            .border_color(border_color);

        for (i, terminal) in self.terminals.iter().enumerate() {
            let is_active = i == self.active_terminal;
            let conn_name = terminal.read(cx).connection.name.clone();

            tabs = tabs.child(
                h_flex()
                    .px(px(14.0))
                    .h_full()
                    .items_center()
                    .gap(px(8.0))
                    .border_r_1()
                    .border_color(border_color)
                    .when(is_active, |s| s.bg(tab_active_bg))
                    .child(
                        div()
                            .text_color(if is_active {
                                active_text
                            } else {
                                inactive_text
                            })
                            .text_size(px(13.0))
                            .child(conn_name),
                    )
                    .child(
                        Button::new(format!("close-tab-{}", i))
                            .ghost()
                            .small()
                            .icon(IconName::Close)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.close_terminal(i, cx);
                            })),
                    ),
            );
        }

        tabs
    }

    /// Render the terminal area: tab bar + active terminal, or a placeholder.
    fn render_terminal_area(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if self.terminals.is_empty() {
            let muted = cx.theme().muted_foreground;
            v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .gap(px(10.0))
                .bg(cx.theme().background)
                .child(
                    Icon::new(IconName::SquareTerminal)
                        .size(px(42.0))
                        .text_color(muted),
                )
                .child(
                    div()
                        .text_color(muted)
                        .text_size(px(14.0))
                        .child("Select a connection to start"),
                )
                .into_any_element()
        } else {
            v_flex()
                .flex_1()
                .h_full()
                .min_h_0()
                .child(self.render_tab_bar(cx))
                .child(
                    div()
                        .flex_1()
                        .h_full()
                        .min_h_0()
                        .overflow_hidden()
                        .child(self.terminals[self.active_terminal].clone()),
                )
                .into_any_element()
        }
    }

    fn render_delete_confirm_dialog(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (_, name) = self.delete_confirm.clone()?;
        let theme = cx.theme();
        let overlay_bg = theme.overlay;
        let dialog_bg = theme.popover;
        let border_color = theme.border;
        let title_color = theme.foreground;
        let muted_color = theme.muted_foreground;
        let danger_color = theme.danger;
        let radius = theme.radius;

        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .bg(overlay_bg)
                .flex()
                .items_center()
                .justify_center()
                .child(
                    v_flex()
                        .w(px(420.0))
                        .bg(dialog_bg)
                        .border_1()
                        .border_color(border_color)
                        .rounded(radius)
                        .p(px(22.0))
                        .gap(px(14.0))
                        .child(
                            h_flex()
                                .items_center()
                                .gap(px(10.0))
                                .child(
                                    Icon::new(IconName::TriangleAlert)
                                        .size(px(18.0))
                                        .text_color(danger_color),
                                )
                                .child(
                                    div()
                                        .text_color(title_color)
                                        .text_size(px(17.0))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .child("Delete connection"),
                                ),
                        )
                        .child(
                            div()
                                .text_color(muted_color)
                                .text_size(px(13.0))
                                .child(format!(
                                    "Delete \"{}\"? This only removes the saved profile.",
                                    name
                                )),
                        )
                        .child(
                            h_flex()
                                .justify_end()
                                .gap(px(8.0))
                                .child(
                                    Button::new("delete-confirm-cancel")
                                        .ghost()
                                        .label("Cancel")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.close_delete_confirm(cx);
                                        })),
                                )
                                .child(
                                    Button::new("delete-confirm-ok")
                                        .danger()
                                        .icon(IconName::Delete)
                                        .label("Delete")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.confirm_delete_connection(cx);
                                        })),
                                ),
                        ),
                )
                .into_any_element(),
        )
    }
}

impl Render for AppView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Render sidebar (borrows self + cx temporarily, returns owned element).
        let sidebar = ui::sidebar::render_sidebar(self, window, cx);

        // Render terminal area (borrows self + cx temporarily, returns owned element).
        let terminal_area = self.render_terminal_area(window, cx);

        // Assemble root layout.
        let mut root = h_flex()
            .size_full()
            .min_h_0()
            .overflow_hidden()
            .pt(px(TITLEBAR_SAFE_TOP))
            .bg(cx.theme().background)
            .child(sidebar)
            .child(terminal_area);

        // Overlay connection dialog if visible.
        if self.show_dialog {
            let dialog = ui::connection_dialog::render_connection_dialog(self, window, cx);
            root = root.child(dialog);
        }

        if let Some(dialog) = self.render_delete_confirm_dialog(cx) {
            root = root.child(dialog);
        }

        root
    }
}
