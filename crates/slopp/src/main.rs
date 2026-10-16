//! SLOPP: an XAMPP-style control panel for Apache, PHP, MariaDB and
//! phpMyAdmin, built with GPUI.

// No console window next to the panel on Windows; debug builds keep one for
// the log.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(target_os = "linux")]
mod appimage;
mod controller;
mod crash;
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
    crash::install(crash::log_path());
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

    #[cfg(target_os = "linux")]
    appimage::integrate();

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
        cx.on_window_closed(|cx, closed| {
            // Not the panel itself when it was swapped for the other style.
            let panel = &mut cx.global_mut::<PanelWindow>().0;
            if panel.is_some_and(|window| window.window_id() != closed) {
                return;
            }
            *panel = None;
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
        TrayCommand::OpenMailpit => controller.update(cx, |c, cx| c.open_mailpit(cx)),
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
    let page = match std::env::var("SLOPP_PAGE").as_deref() {
        Ok("ports") => panel::Page::Ports,
        Ok("settings") => panel::Page::Settings,
        _ => panel::Page::Modules,
    };
    open_panel(page, cx);
}

/// Replaces the panel window with one in the window style now set, on
/// `page`.
fn reopen_panel(page: panel::Page, cx: &mut App) {
    let old = cx.global::<PanelWindow>().0;
    open_panel(page, cx);
    if let Some(old) = old {
        // After the new one is up, so this doesn't count as closing the panel.
        let _ = old.update(cx, |_, window, _| window.remove_window());
    }
}

fn open_panel(page: panel::Page, cx: &mut App) {
    let compact = Controller::global(cx).read(cx).settings().compact;
    // Compact is a small dialog-like window with just the rack.
    let (content, min_size) = if compact {
        let content = size(px(460.), views::rack::compact_height());
        (content, content)
    } else {
        (size(px(1080.), px(720.)), size(px(900.), px(520.)))
    };
    // With client-side decorations on Linux the window's size includes the
    // shadow gpui-component draws around it; its frame's border is inside
    // the window geometry, which the minimum size is in.
    let (shadow, border) = if cfg!(target_os = "linux") {
        (px(CLIENT_SHADOW), px(1.))
    } else {
        (px(0.), px(0.))
    };
    let framed = |s: Size<Pixels>, by: Pixels| size(s.width + by * 2., s.height + by * 2.);
    let min_size = framed(min_size, border);
    let size = framed(content, shadow + border);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::centered(size, cx)),
        window_min_size: Some(min_size),
        window_decorations: Some(WindowDecorations::Client),
        app_id: Some("org.slopp.Slopp".into()),
        #[cfg(target_os = "linux")]
        icon: x11_icon(),
        titlebar: Some(TitlebarOptions {
            title: Some("SLOPP".into()),
            ..TitleBar::title_bar_options()
        }),
        ..TitleBar::window_options()
    };
    match gpui_kit::open_window(options, cx, |window, cx| {
        // Before any configure from a Wayland compositor: the first
        // render sets this, and a configure that comes earlier would take
        // the whole size for the window's frame, shadow included, so the
        // window would be bigger on some compositors than on others.
        #[cfg(target_os = "linux")]
        window.set_client_inset(px(CLIENT_SHADOW));
        panel::apply_theme(window, cx);
        cx.new(|cx| panel::Panel::new(page, window, cx))
    }) {
        Ok((window, _)) => {
            cx.global_mut::<PanelWindow>().0 = Some(window);
            cx.activate(true);
        }
        Err(err) => tracing::error!("couldn't open the window: {err:#}"),
    }
}

/// The shadow gpui-component's window border draws around a window with
/// client-side decorations on Linux (its `SHADOW_SIZE`).
const CLIENT_SHADOW: f32 = 20.;

/// The window's icon on X11, for taskbars that show the window's own icon
/// rather than its desktop file's (Wayland has only the latter).
#[cfg(target_os = "linux")]
fn x11_icon() -> Option<std::sync::Arc<image::RgbaImage>> {
    const PNG: &[u8] = include_bytes!("../assets/linux/org.slopp.Slopp.png");
    match image::load_from_memory_with_format(PNG, image::ImageFormat::Png) {
        Ok(icon) => Some(std::sync::Arc::new(icon.into_rgba8())),
        Err(err) => {
            tracing::warn!("couldn't decode the window icon: {err}");
            None
        }
    }
}
