use std::sync::Arc;

use gpui::AppContext;
use gpui_component::{Root, Theme, ThemeMode};
use gpui_component_assets::Assets;
use parking_lot::Mutex;

mod app;
mod config;
mod ssh;
mod terminal;
mod ui;

use crate::app::AppView;
use crate::config::AppConfig;

fn main() {
    env_logger::init();

    // Create a dedicated tokio runtime for SSH operations (russh requires tokio).
    // gpui has its own async system; we bridge them via crossbeam channels.
    let runtime = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    let runtime_handle = runtime.handle().clone();

    // Load or create the application configuration.
    let config = Arc::new(Mutex::new(AppConfig::load().unwrap_or_default()));

    let app = gpui_platform::application()
        .with_assets(Assets)
        .with_quit_mode(gpui::QuitMode::LastWindowClosed);

    app.run(move |cx| {
        // This must be called before using any GPUI Component features.
        gpui_component::init(cx);
        crate::terminal::view::init(cx);
        // Use dark theme to match terminal application aesthetic.
        Theme::change(ThemeMode::Dark, None, cx);

        let window_options = gpui::WindowOptions {
            window_bounds: Some(gpui::WindowBounds::centered(
                gpui::size(gpui::px(1200.), gpui::px(800.)),
                cx,
            )),
            titlebar: Some(gpui::TitlebarOptions {
                title: Some("SSH Manager".into()),
                appears_transparent: true,
                traffic_light_position: None,
            }),
            ..Default::default()
        };

        let config = config.clone();
        let runtime_handle = runtime_handle.clone();

        cx.spawn(async move |cx| {
            cx.open_window(window_options, |window, cx| {
                let view = cx.new(|cx| {
                    AppView::new(runtime_handle, config, window, cx)
                });
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("Failed to open window");
        })
        .detach();
    });
}
