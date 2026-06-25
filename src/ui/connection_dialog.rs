//! Connection dialog: form for adding/editing SSH connections.
//!
//! This module provides a helper function for rendering the connection dialog.
//! The dialog state (InputState entities, visibility) is held by `AppView`.

use gpui::{div, px, IntoElement, ParentElement, Styled, Window};
use gpui_component::{
    h_flex, v_flex,
    button::{Button, ButtonVariants as _},
    input::{Input, InputState},
    label::Label,
    ActiveTheme as _, IconName, Sizable as _,
};

use crate::app::AppView;

/// Render the connection dialog form.
/// Called from `AppView::render()` when `show_dialog` is true.
pub fn render_connection_dialog(
    app: &mut AppView,
    _window: &mut Window,
    cx: &mut gpui::Context<AppView>,
) -> impl IntoElement {
    let is_edit = app.editing_id.is_some();
    let theme = cx.theme();
    let overlay_bg = theme.overlay;
    let dialog_bg = theme.popover;
    let border_color = theme.border;
    let title_color = theme.foreground;
    let label_color = theme.muted_foreground;
    let radius = theme.radius;

    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .bg(overlay_bg)
        // Click on backdrop closes dialog
        .flex()
        .items_center()
        .justify_center()
        .child(
            v_flex()
                .w(px(480.0))
                .bg(dialog_bg)
                .border_1()
                .border_color(border_color)
                .rounded(radius)
                .p(px(20.0))
                .gap(px(12.0))
                // Stop propagation so clicking inside doesn't close
                .child(
                    div()
                        .text_color(title_color)
                        .text_size(px(16.0))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(if is_edit { "Edit Connection" } else { "Add Connection" }),
                )
                // Name field
                .child(render_field("Name", &app.name_input, label_color))
                // Host field
                .child(render_field("Host", &app.host_input, label_color))
                // Port + Username in a row
                .child(
                    h_flex()
                        .gap(px(12.0))
                        .child(
                            v_flex()
                                .flex_1()
                                .gap(px(4.0))
                                .child(
                                    Label::new("Port")
                                        .text_size(px(12.0))
                                        .text_color(label_color),
                                )
                                .child(Input::new(&app.port_input)),
                        )
                        .child(
                            v_flex()
                                .flex_1()
                                .gap(px(4.0))
                                .child(
                                    Label::new("Username")
                                        .text_size(px(12.0))
                                        .text_color(label_color),
                                )
                                .child(Input::new(&app.username_input)),
                        ),
                )
                // Auth method selector
                .child(
                    v_flex()
                        .gap(px(4.0))
                        .child(
                            Label::new("Authentication")
                                .text_size(px(12.0))
                                .text_color(label_color),
                        )
                        .child(
                            h_flex()
                                .gap(px(8.0))
                                .child(
                                    if app.auth_method_is_password() {
                                        Button::new("auth-password")
                                            .primary()
                                            .small()
                                            .label("Password")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.set_auth_method_password(cx);
                                            }))
                                            .into_any_element()
                                    } else {
                                        Button::new("auth-password")
                                            .ghost()
                                            .small()
                                            .label("Password")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.set_auth_method_password(cx);
                                            }))
                                            .into_any_element()
                                    },
                                )
                                .child(
                                    if app.auth_method_is_key() {
                                        Button::new("auth-key")
                                            .primary()
                                            .small()
                                            .label("Private Key")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.set_auth_method_key(cx);
                                            }))
                                            .into_any_element()
                                    } else {
                                        Button::new("auth-key")
                                            .ghost()
                                            .small()
                                            .label("Private Key")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.set_auth_method_key(cx);
                                            }))
                                            .into_any_element()
                                    },
                                ),
                        ),
                )
                // Auth-specific fields
                .child(if app.auth_method_is_password() {
                    render_field("Password", &app.password_input, label_color).into_any_element()
                } else {
                    render_field("Key Path", &app.key_path_input, label_color).into_any_element()
                })
                // Group field
                .child(render_field("Group (optional)", &app.group_input, label_color))
                // Buttons
                .child(
                    h_flex()
                        .justify_end()
                        .gap(px(8.0))
                        .child(
                            Button::new("dialog-cancel")
                                .ghost()
                                .label("Cancel")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.close_dialog(cx);
                                })),
                        )
                        .child(
                            Button::new("dialog-save")
                                .primary()
                                .icon(IconName::Check)
                                .label(if is_edit { "Update" } else { "Save" })
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.save_connection(window, cx);
                                })),
                        ),
                ),
        )
}

/// Render a labeled input field.
fn render_field(
    label: &str,
    state: &gpui::Entity<InputState>,
    label_color: gpui::Hsla,
) -> impl IntoElement {
    v_flex()
        .gap(px(4.0))
        .child(
            Label::new(label)
                .text_size(px(12.0))
                .text_color(label_color),
        )
        .child(Input::new(state))
}
