//! Main application view: sidebar + terminal area + connection dialog.

use std::sync::Arc;

use gpui::{
    div, px, AnyElement, AppContext, Context, Entity, IntoElement, ParentElement, Render,
    Styled, Window,
};
use gpui::prelude::FluentBuilder;
use gpui_component::{
    h_flex, v_flex,
    button::{Button, ButtonVariants as _},
    input::InputState,
    ActiveTheme as _, Icon, IconName, Sizable as _,
};
use parking_lot::Mutex;
use uuid::Uuid;

use crate::config::{AppConfig, AuthMethod, SshConnection};
use crate::terminal::view::TerminalView;
use crate::ui;

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
        let name_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("My Server"));
        let host_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("192.168.1.1"));
        let port_input = cx.new(|cx| InputState::new(window, cx).placeholder("22"));
        let username_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("root"));
        let password_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Password"));
        let key_path_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("~/.ssh/id_rsa"));
        let group_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Production"));
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

    pub fn show_add_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editing_id = None;
        self.show_dialog = true;
        self.auth_is_password = true;

        self.name_input.update(cx, |s, cx| s.set_value("", window, cx));
        self.host_input.update(cx, |s, cx| s.set_value("", window, cx));
        self.port_input.update(cx, |s, cx| s.set_value("22", window, cx));
        self.username_input.update(cx, |s, cx| s.set_value("", window, cx));
        self.password_input.update(cx, |s, cx| s.set_value("", window, cx));
        self.key_path_input.update(cx, |s, cx| s.set_value("", window, cx));
        self.group_input.update(cx, |s, cx| s.set_value("", window, cx));

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

        self.name_input.update(cx, |s, cx| s.set_value(&conn.name, window, cx));
        self.host_input.update(cx, |s, cx| s.set_value(&conn.host, window, cx));
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
        self.group_input.update(cx, |s, cx| s.set_value(group, window, cx));

        cx.notify();
    }

    pub fn save_connection(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let name = self.name_input.read(cx).value().to_string();
        let host = self.host_input.read(cx).value().to_string();
        let port: u16 = self
            .port_input
            .read(cx)
            .value()
            .parse()
            .unwrap_or(22);
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
        let terminal =
            cx.new(|cx| TerminalView::new(&runtime, conn, window, cx));
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
            .h(px(36.0))
            .bg(tab_bar_bg)
            .border_b_1()
            .border_color(border_color);

        for (i, terminal) in self.terminals.iter().enumerate() {
            let is_active = i == self.active_terminal;
            let conn_name = terminal.read(cx).connection.name.clone();

            tabs = tabs.child(
                h_flex()
                    .px(px(12.0))
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
    fn render_terminal_area(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.terminals.is_empty() {
            let muted = cx.theme().muted_foreground;
            v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .gap(px(12.0))
                .bg(cx.theme().background)
                .child(
                    Icon::new(IconName::SquareTerminal)
                        .size(px(48.0))
                        .text_color(muted),
                )
                .child(
                    div()
                        .text_color(muted)
                        .text_size(px(15.0))
                        .child("Select a connection from the sidebar to start"),
                )
                .into_any_element()
        } else {
            v_flex()
                .flex_1()
                .child(self.render_tab_bar(cx))
                .child(
                    div()
                        .flex_1()
                        .child(self.terminals[self.active_terminal].clone()),
                )
                .into_any_element()
        }
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
            .bg(cx.theme().background)
            .child(sidebar)
            .child(terminal_area);

        // Overlay connection dialog if visible.
        if self.show_dialog {
            let dialog = ui::connection_dialog::render_connection_dialog(self, window, cx);
            root = root.child(dialog);
        }

        root
    }
}
