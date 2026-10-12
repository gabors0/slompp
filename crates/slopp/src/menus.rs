//! The application's menus in the macOS menu bar, with the shortcuts every
//! Mac app answers to.

use gpui_kit::{App, KeyBinding, Menu, MenuItem, Window, actions};

use crate::panel::{Quit, ShowModules, ShowPorts, ShowSettings};

actions!(slopp, [Hide, HideOthers, ShowAll, Minimize, CloseWindow]);

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("alt-cmd-h", HideOthers, None),
        KeyBinding::new("cmd-m", Minimize, None),
        KeyBinding::new("cmd-w", CloseWindow, None),
    ]);
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &Minimize, cx| in_active_window(cx, |window| window.minimize_window()));
    cx.on_action(|_: &CloseWindow, cx| in_active_window(cx, |window| window.remove_window()));

    cx.set_menus([
        Menu::new("SLOPP").items([
            MenuItem::action("Settings…", ShowSettings),
            MenuItem::separator(),
            MenuItem::action("Hide SLOPP", Hide),
            MenuItem::action("Hide Others", HideOthers),
            MenuItem::action("Show All", ShowAll),
            MenuItem::separator(),
            MenuItem::action("Quit SLOPP", Quit),
        ]),
        Menu::new("View").items([
            MenuItem::action("Modules", ShowModules),
            MenuItem::action("Ports", ShowPorts),
            MenuItem::action("Settings", ShowSettings),
        ]),
        Menu::new("Window").items([
            MenuItem::action("Minimize", Minimize),
            MenuItem::action("Close", CloseWindow),
        ]),
    ]);
}

fn in_active_window(cx: &mut App, f: impl FnOnce(&mut Window) + 'static) {
    if let Some(window) = cx.active_window() {
        // The action is dispatched from inside the window's own update.
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, window, _| f(window));
        });
    }
}
