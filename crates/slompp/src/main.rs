//! SLOMPP: an XAMPP-style control panel for Apache, PHP, MariaDB and
//! phpMyAdmin, built with GPUI.

mod log;
mod panel;
mod theme;
mod views;

use gpui_kit::component::TitleBar;
use gpui_kit::*;

fn main() {
    let filter = tracing_subscriber::EnvFilter::try_from_env("SLOMPP_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn,slompp=info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(|cx| {
            gpui_kit::init(cx);
            theme::init(cx);
            panel::bind_keys(cx);

            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1080.), px(720.)), cx)),
                window_min_size: Some(size(px(900.), px(520.))),
                window_decorations: Some(WindowDecorations::Client),
                app_id: Some("org.slompp.Slompp".into()),
                titlebar: Some(TitlebarOptions {
                    title: Some("SLOMPP".into()),
                    ..TitleBar::title_bar_options()
                }),
                ..TitleBar::window_options()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                theme::apply(panel::wants_light(window.appearance()), cx);
                cx.new(|cx| panel::Panel::new(window, cx))
            })
            .expect("failed to open window");
            // Closing the window quits, like XAMPP's panel (the servers keep
            // running unless Settings says otherwise).
            cx.on_window_closed(|cx, _| cx.quit()).detach();
            cx.activate(true);
        });
}
