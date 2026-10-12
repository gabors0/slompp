//! The control panel window: a view of the [`Controller`] with its pages.

use gpui_kit::component::notification::Notification;
use gpui_kit::component::{TitleBar, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, Entity, FocusHandle, Focusable, KeyBinding, ScrollHandle, Subscription, Window,
    WindowAppearance, actions, div,
};

use crate::controller::{Controller, ControllerEvent};
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
        ShowSettings
    ]
);

const CONTEXT: &str = "Panel";

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        // "secondary" is Cmd on macOS and Ctrl everywhere else.
        KeyBinding::new("secondary-q", Quit, None),
        KeyBinding::new("secondary-1", ShowModules, Some(CONTEXT)),
        KeyBinding::new("secondary-2", ShowPorts, Some(CONTEXT)),
        KeyBinding::new("secondary-3", ShowSettings, Some(CONTEXT)),
        KeyBinding::new("secondary-,", ShowSettings, Some(CONTEXT)),
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
    pub log_scroll: ScrollHandle,
    /// The log revision the console last scrolled down to.
    scrolled_revision: usize,
    settings_view: Entity<SettingsView>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Panel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let controller = Controller::global(cx);
        let stack = controller.read(cx).stack().clone();
        let settings_view = cx.new(|cx| SettingsView::new(&stack, window, cx));
        let focus = cx.focus_handle();
        window.focus(&focus, cx);

        let subscriptions = vec![
            cx.observe(&controller, |_, _, cx| cx.notify()),
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
                theme::apply(wants_light(window.appearance()), cx);
                cx.notify();
            }),
            // A closed window no longer needs the Ports page's polling.
            cx.on_release(|this, cx| {
                this.controller
                    .update(cx, |controller, _| controller.watch_ports = false);
            }),
        ];

        let page = match std::env::var("SLOPP_PAGE").as_deref() {
            Ok("ports") => Page::Ports,
            Ok("settings") => Page::Settings,
            _ => Page::Modules,
        };

        let mut this = Self {
            controller,
            page: Page::Modules,
            log_scroll: ScrollHandle::new(),
            scrolled_revision: usize::MAX,
            settings_view,
            focus,
            _subscriptions: subscriptions,
        };
        this.show(page, cx);
        this
    }

    pub fn show(&mut self, page: Page, cx: &mut Context<Self>) {
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

    fn on_settings_event(
        &mut self,
        _: &Entity<SettingsView>,
        event: &SettingsEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let SettingsEvent::Save(settings) = event;
        let result = self.controller.update(cx, |controller, cx| {
            controller.apply_settings(settings.clone(), cx)
        });
        match result {
            Ok(()) => {
                let stack = self.controller.read(cx).stack().clone();
                self.settings_view
                    .update(cx, |view, cx| view.reload(&stack, window, cx));
                window.push_notification(Notification::success("Settings saved"), cx);
            }
            Err(text) => window.push_notification(Notification::error(text), cx),
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

/// Follows the system, unless `SLOPP_THEME=light|dark` says otherwise.
pub fn wants_light(appearance: WindowAppearance) -> bool {
    match std::env::var("SLOPP_THEME").as_deref() {
        Ok("light") => true,
        Ok("dark") => false,
        _ => matches!(
            appearance,
            WindowAppearance::Light | WindowAppearance::VibrantLight
        ),
    }
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
            .size_full()
            .bg(Palette::bg())
            .text_color(Palette::text())
            .font_family(theme::FONT)
            .child(
                TitleBar::new().child(
                    h_flex()
                        .w_full()
                        .h_full()
                        .pr_2()
                        .justify_between()
                        .child(brand::render_nav(this, ctl, cx))
                        .child(brand::render_global_actions(ctl, cx)),
                ),
            )
            .child(body)
    }
}
