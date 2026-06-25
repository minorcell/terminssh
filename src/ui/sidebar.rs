//! Sidebar component: connection list with add/edit/delete.
//!
//! This module provides helper functions for rendering the sidebar.
//! The sidebar is rendered inline by `AppView` to avoid cross-entity complexity.

use gpui::prelude::FluentBuilder;
use gpui::{
    div, px, Context, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement,
    Styled, Window,
};
use gpui_component::{
    button::{Button, ButtonVariant, ButtonVariants as _},
    dialog::DialogButtonProps,
    h_flex,
    input::Input,
    v_flex, ActiveTheme as _, Icon, IconName, Sizable as _, WindowExt as _,
};

use crate::app::AppView;
use crate::config::SshConnection;

/// Render the sidebar: header + search + connection list.
/// Called from `AppView::render()`.
pub fn render_sidebar(
    app: &mut AppView,
    _window: &mut Window,
    cx: &mut Context<AppView>,
) -> impl IntoElement {
    let config = app.config.lock().clone();

    // Read search query and filter connections.
    let search_query = app.search_input.read(cx).value().to_lowercase();
    let filtered: Vec<SshConnection> = config
        .connections
        .iter()
        .filter(|c| {
            if search_query.is_empty() {
                return true;
            }
            c.name.to_lowercase().contains(&search_query)
                || c.host.to_lowercase().contains(&search_query)
                || c.username.to_lowercase().contains(&search_query)
        })
        .cloned()
        .collect();

    // Pre-compute which connections have open terminals.
    let active_ids: std::collections::HashSet<String> = app
        .terminals
        .iter()
        .filter_map(|t| {
            let terminal = t.read(cx);
            Some(terminal.connection.id.clone())
        })
        .collect();
    let active_count = active_ids.len();

    // Extract theme colors before building UI with closures.
    let theme = cx.theme();
    let sidebar_bg = theme.sidebar;
    let border_color = theme.sidebar_border;
    let title_color = theme.sidebar_foreground;
    let muted_color = theme.muted_foreground;
    let group_color = theme.muted_foreground;

    // Build grouped connection list.
    let mut grouped: std::collections::BTreeMap<String, Vec<SshConnection>> =
        std::collections::BTreeMap::new();
    for conn in &filtered {
        let group = conn.group.clone().unwrap_or_else(|| "Default".to_string());
        grouped.entry(group).or_default().push(conn.clone());
    }

    let mut list = v_flex()
        .flex_1()
        .id("connection-list")
        .overflow_y_scroll()
        .pt(px(4.0));
    for (group, conns) in &grouped {
        let group_label = group.to_uppercase();
        list = list.child(
            div()
                .px(px(14.0))
                .pt(px(10.0))
                .pb(px(5.0))
                .text_size(px(11.0))
                .text_color(group_color)
                .child(group_label),
        );
        for conn in conns {
            let is_active = active_ids.contains(&conn.id);
            list = list.child(render_connection_item(conn.clone(), is_active, cx));
        }
    }

    // Determine empty state: no connections at all, or no search results.
    let list_area = if config.connections.is_empty() {
        v_flex()
            .flex_1()
            .items_center()
            .justify_center()
            .gap(px(8.0))
            .child(
                Icon::new(IconName::Network)
                    .size(px(40.0))
                    .text_color(muted_color),
            )
            .child(
                div()
                    .text_color(muted_color)
                    .text_size(px(13.0))
                    .child("No connections yet"),
            )
            .into_any_element()
    } else if filtered.is_empty() {
        v_flex()
            .flex_1()
            .items_center()
            .justify_center()
            .gap(px(8.0))
            .child(
                Icon::new(IconName::Search)
                    .size(px(32.0))
                    .text_color(muted_color),
            )
            .child(
                div()
                    .text_color(muted_color)
                    .text_size(px(13.0))
                    .child("No matching connections"),
            )
            .into_any_element()
    } else {
        list.into_any_element()
    };

    // Clone search_input entity for the Input widget.
    let search_input = app.search_input.clone();

    v_flex()
        .w(px(288.0))
        .h_full()
        .bg(sidebar_bg)
        .border_r_1()
        .border_color(border_color)
        // Header
        .child(
            h_flex()
                .justify_between()
                .items_center()
                .px(px(14.0))
                .py(px(12.0))
                .child(
                    h_flex()
                        .gap(px(8.0))
                        .items_center()
                        .child(
                            Icon::new(IconName::SquareTerminal)
                                .size(px(18.0))
                                .text_color(title_color),
                        )
                        .child(
                            div()
                                .text_color(title_color)
                                .text_size(px(15.0))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child("SSH Manager"),
                        ),
                )
                .child(
                    Button::new("add-connection")
                        .primary()
                        .small()
                        .icon(IconName::Plus)
                        .tooltip("Add connection")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.show_add_dialog(window, cx);
                        })),
                ),
        )
        // Search bar
        .child(
            div().px(px(14.0)).pb(px(10.0)).child(
                Input::new(&search_input)
                    .prefix(Icon::new(IconName::Search).xsmall().text_color(muted_color))
                    .cleanable(true),
            ),
        )
        // Connection list
        .child(list_area)
        // Status bar at bottom
        .child(
            div()
                .px(px(14.0))
                .py(px(9.0))
                .border_t_1()
                .border_color(border_color)
                .text_size(px(11.0))
                .text_color(muted_color)
                .child(format!(
                    "{} saved / {} active",
                    config.connections.len(),
                    active_count
                )),
        )
}

/// Render a single connection item in the sidebar.
fn render_connection_item(
    conn: SshConnection,
    is_active: bool,
    cx: &mut Context<AppView>,
) -> impl IntoElement {
    let conn_for_click = conn.clone();
    let conn_for_edit = conn.clone();
    let conn_id_for_delete = conn.id.clone();
    let conn_name_for_delete = conn.name.clone();
    let theme = cx.theme();
    let fg_color = theme.foreground;
    let muted_color = theme.muted_foreground;
    let hover_bg = theme.list_hover;
    let active_bg = theme.list_active;
    let active_border = theme.primary;
    let success_color = theme.success;
    let muted_dot = theme.muted;

    h_flex()
        .w_full()
        .mx(px(8.0))
        .px(px(10.0))
        .py(px(8.0))
        .items_center()
        .gap(px(10.0))
        .rounded(px(6.0))
        .when(is_active, |s| {
            s.bg(active_bg).border_l_2().border_color(active_border)
        })
        .when(!is_active, |s| s.hover(move |s| s.bg(hover_bg)))
        .cursor_pointer()
        .id(format!("conn-{}", conn.id))
        .on_click(cx.listener(move |this, _, window, cx| {
            this.open_terminal(conn_for_click.clone(), window, cx);
        }))
        // Status dot: green when active, muted when inactive
        .child(
            div()
                .w(px(8.0))
                .h(px(8.0))
                .rounded_full()
                .when(is_active, |s| s.bg(success_color))
                .when(!is_active, |s| s.bg(muted_dot)),
        )
        .child(
            v_flex()
                .flex_1()
                .gap(px(2.0))
                .child(
                    div()
                        .text_color(fg_color)
                        .text_size(px(13.0))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .child(conn.name.clone()),
                )
                .child(
                    div()
                        .text_color(muted_color)
                        .text_size(px(11.0))
                        .child(conn.descriptor()),
                ),
        )
        .child(
            h_flex()
                .flex_shrink_0()
                .gap(px(2.0))
                .id(format!("conn-actions-{}", conn.id))
                .on_click(|_, _, cx| {
                    cx.stop_propagation();
                })
                .child(
                    Button::new(format!("edit-{}", conn.id))
                        .ghost()
                        .small()
                        .icon(IconName::Settings2)
                        .tooltip("Edit connection")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.show_edit_dialog(&conn_for_edit, window, cx);
                        })),
                )
                .child(
                    Button::new(format!("del-{}", conn_id_for_delete))
                        .ghost()
                        .small()
                        .icon(IconName::Delete)
                        .tooltip("Delete connection")
                        .on_click(cx.listener(move |_, _, window, cx| {
                            cx.stop_propagation();
                            let conn_id = conn_id_for_delete.clone();
                            let conn_name = conn_name_for_delete.clone();
                            let app = cx.entity();

                            window.open_alert_dialog(cx, move |alert, _, cx| {
                                let conn_id = conn_id.clone();
                                let app = app.clone();

                                alert
                                    .icon(
                                        Icon::new(IconName::TriangleAlert)
                                            .text_color(cx.theme().danger),
                                    )
                                    .title("Delete connection")
                                    .description(format!(
                                        "Delete \"{}\"? This only removes the saved profile.",
                                        conn_name
                                    ))
                                    .button_props(
                                        DialogButtonProps::default()
                                            .ok_variant(ButtonVariant::Danger)
                                            .ok_text("Delete")
                                            .cancel_text("Cancel")
                                            .show_cancel(true),
                                    )
                                    .on_ok(move |_, _, cx| {
                                        app.update(cx, |app, cx| {
                                            app.delete_connection(&conn_id, cx);
                                        });
                                        true
                                    })
                            });
                        })),
                ),
        )
}
