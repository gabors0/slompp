//! The control panel window: a view of the [`Controller`] with its pages.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{TitleBar, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, Entity, FocusHandle, Focusable, KeyBinding, ScrollHandle, Subscription, Window,
    WindowAppearance, actions, div,
};
use slopp_core::settings::Appearance;

use crate::controller::{Controller, ControllerEvent, UpdateState};
use crate::theme::{self, Palette};
use crate::views::settings::{SettingsEvent, SettingsView};
use crate::views::{brand, console, ports, rack};

actions!(
    slopp,
    [
        Quit,
        StartAll,
        StopAll,
        ShowModules,
        ShowPorts,
        ShowSettings,
        ToggleCompact
    ]
);

pub const CONTEXT: &str = "Panel";

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        // "secondary" is Cmd on macOS and Ctrl everywhere else.
        KeyBinding::new("secondary-q", Quit, None),
        KeyBinding::new("secondary-1", ShowModules, Some(CONTEXT)),
        KeyBinding::new("secondary-2", ShowPorts, Some(CONTEXT)),
        KeyBinding::new("secondary-3", ShowSettings, Some(CONTEXT)),
        KeyBinding::new("secondary-,", ShowSettings, Some(CONTEXT)),
        KeyBinding::new("secondary-shift-m", ToggleCompact, Some(CONTEXT)),
    ]);
    cx.on_action(|_: &Quit, cx| cx.quit());
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Modules,
    Ports,
    Settings,
}

pub struct Panel {
    pub controller: Entity<Controller>,
    pub page: Page,
    /// The small window with just the rack; always on the Modules page.
    pub compact: bool,
    pub log_scroll: ScrollHandle,
    /// The log revision the console last scrolled down to.
    scrolled_revision: usize,
    settings_view: Entity<SettingsView>,
    /// Whether this window shows the notice of a new release.
    update_notice: bool,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Panel {
    pub fn new(page: Page, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let controller = Controller::global(cx);
        let stack = controller.read(cx).stack().clone();
        let compact = stack.settings().compact;
        let settings_view = cx.new(|cx| SettingsView::new(&stack, window, cx));
        let focus = cx.focus_handle();
        window.focus(&focus, cx);

        let subscriptions = vec![
            cx.observe_in(&controller, window, |this, _, window, cx| {
                this.announce_update(window, cx);
                cx.notify();
            }),
            cx.subscribe_in(&controller, window, |_, _, event, window, cx| {
                let ControllerEvent::Notify { text, error } = event;
                let note = if *error {
                    Notification::error(text.clone())
                } else {
                    Notification::success(text.clone())
                };
                window.push_notification(note, cx);
            }),
            cx.subscribe_in(&settings_view, window, Self::on_settings_event),
            cx.observe_window_appearance(window, |_, window, cx| {
                apply_theme(window, cx);
                cx.notify();
            }),
            // A closed window no longer needs the Ports page's polling
            // (unless its successor in the other style is on that page).
            cx.on_release(|this, cx| {
                if this.page == Page::Ports {
                    this.controller
                        .update(cx, |controller, _| controller.watch_ports = false);
                }
            }),
        ];

        let mut this = Self {
            controller,
            page: Page::Modules,
            compact,
            log_scroll: ScrollHandle::new(),
            scrolled_revision: usize::MAX,
            settings_view,
            update_notice: false,
            focus,
            _subscriptions: subscriptions,
        };
        this.show(if compact { Page::Modules } else { page }, cx);
        // Once the window is up: an update found while it was closed.
        cx.defer_in(window, |this, window, cx| this.announce_update(window, cx));
        this
    }

    /// Tells about a new release once per version, with a way to the
    /// Updates group in Settings; takes the notice away once updating
    /// starts there.
    fn announce_update(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        struct UpdateNotice;
        let ctl = self.controller.read(cx);
        let available = match &ctl.update {
            UpdateState::Available(release) => Some(release.version.clone()),
            UpdateState::Installing { .. } | UpdateState::Ready { .. } => None,
            _ => return,
        };
        let fresh = available.is_some() && ctl.announced != available;
        let can_update = ctl.install.can_update();
        let Some(version) = available else {
            if std::mem::take(&mut self.update_notice) {
                window.remove_notification::<UpdateNotice>(cx);
            }
            return;
        };
        if !fresh {
            return;
        }
        let message = if can_update {
            "Update it from Settings with one click."
        } else {
            "See Settings for how to get it."
        };
        self.controller
            .update(cx, |c, _| c.announced = Some(version.clone()));
        let panel = cx.entity().downgrade();
        let note = Notification::new()
            .id::<UpdateNotice>()
            .title(format!("SLOPP {version} is available"))
            .message(message)
            .action(move |_, _, cx| {
                let panel = panel.clone();
                Button::new("update-settings")
                    .primary()
                    .label("Settings")
                    .on_click(cx.listener(move |note, _, window, cx| {
                        note.dismiss(window, cx);
                        panel
                            .update(cx, |panel, cx| {
                                panel.show(Page::Settings, cx);
                                panel
                                    .settings_view
                                    .update(cx, |view, cx| view.show_updates(cx));
                            })
                            .ok();
                    }))
            });
        window.push_notification(note, cx);
        self.update_notice = true;
    }

    pub fn show(&mut self, page: Page, cx: &mut Context<Self>) {
        // The compact window has only the modules: the others need the full
        // one.
        if self.compact && page != Page::Modules {
            self.set_compact(false, page, cx);
            return;
        }
        self.page = page;
        let watch = page == Page::Ports;
        self.controller.update(cx, |controller, cx| {
            controller.watch_ports = watch;
            if watch {
                controller.refresh_ports(cx);
            }
        });
        cx.notify();
    }

    /// Saves the window style and swaps this window for one in it, on
    /// `page`.
    pub fn set_compact(&mut self, compact: bool, page: Page, cx: &mut Context<Self>) {
        let result = self
            .controller
            .update(cx, |controller, cx| controller.set_compact(compact, cx));
        if result.is_ok() {
            // Not from inside this window's update, which closes it.
            cx.defer(move |cx| crate::reopen_panel(page, cx));
        }
    }

    fn on_settings_event(
        &mut self,
        _: &Entity<SettingsView>,
        event: &SettingsEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            SettingsEvent::Save(settings) => {
                let result = self.controller.update(cx, |controller, cx| {
                    controller.apply_settings(settings.as_ref().clone(), cx)
                });
                match result {
                    Ok(()) => {
                        let stack = self.controller.read(cx).stack().clone();
                        self.settings_view
                            .update(cx, |view, cx| view.reload(&stack, window, cx));
                        apply_theme(window, cx);
                        window.push_notification(Notification::success("Settings saved"), cx);
                    }
                    Err(text) => window.push_notification(Notification::error(text), cx),
                }
            }
            // Saved on its own, leaving the rest of the form as it is.
            SettingsEvent::Appearance(appearance) => {
                let mut settings = self.controller.read(cx).settings().clone();
                settings.appearance = *appearance;
                let result = self
                    .controller
                    .update(cx, |controller, cx| controller.apply_settings(settings, cx));
                match result {
                    Ok(()) => apply_theme(window, cx),
                    Err(text) => window.push_notification(Notification::error(text), cx),
                }
            }
        }
    }
}

/// A click handler that runs `f` on the controller.
pub fn on_controller<E: ?Sized + 'static>(
    cx: &Context<Panel>,
    f: impl Fn(&mut Controller, &mut Context<Controller>) + 'static,
) -> impl Fn(&E, &mut Window, &mut App) + 'static {
    cx.listener(move |this: &mut Panel, _: &E, _, cx| {
        this.controller
            .update(cx, |controller, cx| f(controller, cx));
    })
}

/// `SLOPP_THEME=light|dark`, which overrides the system and the setting.
fn theme_override() -> Option<bool> {
    match std::env::var("SLOPP_THEME").as_deref() {
        Ok("light") => Some(true),
        Ok("dark") => Some(false),
        _ => None,
    }
}

/// Whether the system is in light mode, unless `SLOPP_THEME` says otherwise.
pub fn system_light(appearance: WindowAppearance) -> bool {
    theme_override().unwrap_or(matches!(
        appearance,
        WindowAppearance::Light | WindowAppearance::VibrantLight
    ))
}

/// Whether the panel is light: the Appearance setting, which may follow the
/// system.
pub fn wants_light(appearance: WindowAppearance, setting: Appearance) -> bool {
    match setting {
        Appearance::System => system_light(appearance),
        Appearance::Dark => theme_override().unwrap_or(false),
        Appearance::Light => theme_override().unwrap_or(true),
    }
}

/// Restyles the app for the window's appearance and the setting.
pub fn apply_theme(window: &Window, cx: &mut App) {
    let setting = Controller::global(cx).read(cx).settings().appearance;
    theme::apply(wants_light(window.appearance(), setting), cx);
}

impl Focusable for Panel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Panel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let controller = self.controller.clone();
        let ctl = controller.read(cx);
        // The console follows new lines.
        if self.scrolled_revision != ctl.log.revision {
            self.scrolled_revision = ctl.log.revision;
            self.log_scroll.scroll_to_bottom();
        }
        let this = &*self;
        let body = match this.page {
            Page::Modules if this.compact => v_flex()
                .flex_1()
                .min_h_0()
                .p_3()
                .gap_3()
                .child(
                    div()
                        .id("compact-rack")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .child(rack::render_compact(ctl, cx)),
                )
                .child(rack::render_compact_bar(ctl, cx))
                .into_any_element(),
            Page::Modules => v_flex()
                .flex_1()
                .min_h_0()
                .p_4()
                .gap_3()
                .child(rack::render(ctl, cx))
                .child(rack::render_stack_bar(ctl, cx))
                .child(console::render(this, ctl, cx))
                .into_any_element(),
            Page::Ports => ports::render(ctl, cx).into_any_element(),
            Page::Settings => div()
                .flex_1()
                .min_h_0()
                .child(this.settings_view.clone())
                .into_any_element(),
        };

        v_flex()
            .id("panel")
            .key_context(CONTEXT)
            .track_focus(&this.focus)
            .on_action(on_controller::<StartAll>(cx, |c, cx| c.start_all(cx)))
            .on_action(on_controller::<StopAll>(cx, |c, cx| c.stop_all(cx)))
            .on_action(cx.listener(|this, _: &ShowModules, _, cx| this.show(Page::Modules, cx)))
            .on_action(cx.listener(|this, _: &ShowPorts, _, cx| this.show(Page::Ports, cx)))
            .on_action(cx.listener(|this, _: &ShowSettings, _, cx| this.show(Page::Settings, cx)))
            .on_action(cx.listener(|this, _: &ToggleCompact, _, cx| {
                this.set_compact(!this.compact, Page::Modules, cx)
            }))
            .size_full()
            .bg(Palette::bg())
            .text_color(Palette::text())
            .font_family(theme::FONT)
            .child(
                TitleBar::new().child(h_flex().w_full().h_full().pr_2().justify_between().map(
                    |bar| {
                        if this.compact {
                            bar.child(brand::render_compact_title(ctl))
                                .child(brand::render_style_toggle(true, cx))
                        } else {
                            bar.child(brand::render_nav(this, ctl, cx))
                                .child(brand::render_global_actions(ctl, cx))
                        }
                    },
                )),
            )
            .child(body)
    }
}
