//! SLOPP: an XAMPP-style control panel for Apache, PHP, MariaDB and
//! phpMyAdmin, built with GPUI.

// No console window next to the panel on Windows; debug builds keep one for
// the log.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod controller;
mod instance;
mod log;
#[cfg(target_os = "macos")]
mod menus;
mod panel;
mod theme;
mod tray;
mod views;

use gpui_kit::component::TitleBar;
use gpui_kit::*;

use crate::controller::Controller;
use crate::tray::TrayCommand;

/// The panel window, while one is open.
#[derive(Default)]
struct PanelWindow(Option<AnyWindowHandle>);

impl Global for PanelWindow {}

fn main() {
    let filter = tracing_subscriber::EnvFilter::try_from_env("SLOPP_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn,slopp=info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let (commands, inbox) = async_channel::unbounded::<TrayCommand>();
    match instance::claim() {
        Ok(instance::Instance::Secondary) => {
            println!("SLOPP is already running; showing its window.");
            return;
        }
        Ok(instance::Instance::Primary(listener)) => instance::serve(listener, commands.clone()),
        Err(err) => tracing::warn!("couldn't check for a running SLOPP: {err:#}"),
    }

    let app = gpui_kit::application().with_assets(gpui_kit::assets::AllAssets);
    // macOS: a click on the Dock icon while the window is closed.
    app.on_reopen(show_panel);
    app.run(move |cx| {
        gpui_kit::init(cx);
        // Quitting is decided in on_window_closed: with a tray icon the
        // app outlives its window.
        cx.set_quit_mode(QuitMode::Explicit);
        theme::init(cx);
        panel::bind_keys(cx);
        #[cfg(target_os = "macos")]
        menus::init(cx);
        cx.set_global(PanelWindow::default());

        let controller = Controller::init(cx);
        let state = controller.read(cx).tray_snapshot(cx);
        match tray::spawn(state, commands.clone()) {
            Ok(tray) => controller.update(cx, |c, cx| c.set_tray(tray, cx)),
            Err(err) => tracing::info!("{err:#}; closing the window will quit"),
        }

        cx.spawn(async move |cx| {
            while let Ok(command) = inbox.recv().await {
                cx.update(|cx| handle(command, cx));
            }
        })
        .detach();

        // Closing the window keeps SLOPP in the tray, if there is one
        // and Settings allows it; otherwise it quits, like XAMPP's panel.
        // The servers keep running either way unless Settings says not.
        cx.on_window_closed(|cx, _| {
            cx.global_mut::<PanelWindow>().0 = None;
            let controller = Controller::global(cx);
            let ctl = controller.read(cx);
            if !(ctl.has_tray() && ctl.settings().close_to_tray) {
                cx.quit();
            }
        })
        .detach();
        cx.on_app_quit(|cx| {
            Controller::global(cx).update(cx, |c, _| c.remove_tray());
            async { instance::release() }
        })
        .detach();

        if std::env::var_os("SLOPP_HIDDEN").is_none() {
            show_panel(cx);
        }
    });
}

fn handle(command: TrayCommand, cx: &mut App) {
    let controller = Controller::global(cx);
    match command {
        TrayCommand::Toggle(m) => controller.update(cx, |c, cx| c.toggle(m, cx)),
        TrayCommand::StartAll => controller.update(cx, |c, cx| c.start_all(cx)),
        TrayCommand::StopAll => controller.update(cx, |c, cx| c.stop_all(cx)),
        TrayCommand::OpenSite => controller.update(cx, |c, cx| c.open_site(cx)),
        TrayCommand::OpenDbAdmin(tool) => controller.update(cx, |c, cx| c.open_db_admin(tool, cx)),
        TrayCommand::ShowPanel => show_panel(cx),
        TrayCommand::Quit => cx.quit(),
    }
}

/// Brings the panel window forward, opening it if it was closed.
fn show_panel(cx: &mut App) {
    if let Some(window) = cx.global::<PanelWindow>().0
        && window
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return;
    }

    let options = WindowOptions {
        window_bounds: Some(WindowBounds::centered(size(px(1080.), px(720.)), cx)),
        window_min_size: Some(size(px(900.), px(520.))),
        window_decorations: Some(WindowDecorations::Client),
        app_id: Some("org.slopp.Slopp".into()),
        titlebar: Some(TitlebarOptions {
            title: Some("SLOPP".into()),
            ..TitleBar::title_bar_options()
        }),
        ..TitleBar::window_options()
    };
    match gpui_kit::open_window(options, cx, |window, cx| {
        panel::apply_theme(window, cx);
        cx.new(|cx| panel::Panel::new(window, cx))
    }) {
        Ok((window, _)) => {
            cx.global_mut::<PanelWindow>().0 = Some(window);
            cx.activate(true);
        }
        Err(err) => tracing::error!("couldn't open the window: {err:#}"),
    }
}
