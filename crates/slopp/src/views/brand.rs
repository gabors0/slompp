//! The title bar: SLOPP's mark, the page tabs and the whole-stack actions.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    Context, Div, FontWeight, Hsla, MouseButton, div, linear_color_stop, linear_gradient, px, rgb,
};
use slopp_core::Module;

use crate::controller::Controller;
use crate::panel::{self, Page, Panel, ToggleCompact, on_controller};
use crate::theme::{Lamp, Palette};

/// SLOPP's mark, as in the tray: three rack units joined into an S by a
/// link on the left (top to middle) and one on the right (middle to bottom).
/// The units' lamps are Apache's, MySQL's and Mailpit's, top to bottom.
pub fn mark(lamps: [Lamp; 3]) -> Div {
    const WIDTH: f32 = 18.;
    const UNIT: f32 = 5.;
    const GAP: f32 = 2.;
    // From the 512 grid the tray icon is drawn on: units 428 wide and 112
    // high, lamps over their right 285.
    const LINK: f32 = WIDTH * 112. / 428.;
    const LAMP: f32 = WIDTH * 285. / 428.;
    let ink = Palette::mark();
    // Each link is pinned to the edge it shares with the units, so it ends
    // where they do once fractional scales round the edges.
    let link = |right: bool, top_unit: f32| {
        div()
            .absolute()
            .when(right, |this| this.right_0())
            .when(!right, |this| this.left_0())
            .top(px(top_unit * (UNIT + GAP) + UNIT / 2.))
            .w(px(LINK))
            .h(px(UNIT + GAP))
            .bg(rgb(ink.link))
    };
    let unit = |lamp: Lamp| {
        div()
            .relative()
            .w(px(WIDTH))
            .h(px(UNIT))
            .rounded(px(1.))
            .bg(rgb(ink.unit))
            .when_some(ink.lamp(lamp), |this, color| {
                let color = Hsla::from(rgb(color));
                this.child(
                    div()
                        .absolute()
                        .right_0()
                        .top_0()
                        .w(px(LAMP))
                        .h_full()
                        .rounded(px(1.))
                        .bg(linear_gradient(
                            90.,
                            linear_color_stop(color.opacity(0.), 0.),
                            linear_color_stop(color, ink.lamp_solid),
                        )),
                )
            })
    };
    // The links first, so the units cover their ends.
    v_flex()
        .relative()
        .gap(px(GAP))
        .child(link(false, 0.))
        .child(link(true, 1.))
        .children(lamps.map(unit))
}

/// Keeps presses on controls inside the title bar to themselves: the bar
/// starts a window move when the pointer moves while pressed, and on KWin
/// that move swallows the click.
pub fn controls(element: Div) -> Div {
    element.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

/// The app's name, which shows the version (muted, italic) while hovered.
fn title() -> Div {
    const GROUP: &str = "app-title";
    const VERSION: &str = concat!("v", env!("CARGO_PKG_VERSION"));
    // The name keeps its place in the layout while hidden, so the tabs don't
    // move; the version goes over it.
    div()
        .group(GROUP)
        .relative()
        .text_size(px(14.))
        .whitespace_nowrap()
        .child(
            div()
                .font_weight(FontWeight::BOLD)
                .group_hover(GROUP, |this| this.invisible())
                .child("SLOPP"),
        )
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .italic()
                .text_color(Palette::text_secondary())
                .invisible()
                .group_hover(GROUP, |this| this.visible())
                .child(VERSION),
        )
}

/// The mark with the lamps of the modules, and the app's name.
fn brand(ctl: &Controller) -> Div {
    h_flex()
        .gap_2()
        .child(mark(Module::ALL.map(|m| ctl.row(m).lamp())))
        .child(title())
}

/// The compact window's title: no tabs, since it has only the modules.
pub fn render_compact_title(ctl: &Controller) -> impl IntoElement {
    brand(ctl)
}

pub fn render_nav(panel: &Panel, ctl: &Controller, cx: &Context<Panel>) -> impl IntoElement {
    let tab = |id: &'static str, label: &'static str, page: Page, cx: &Context<Panel>| {
        let active = panel.page == page;
        div()
            .id(id)
            .px_3()
            .h(px(28.))
            .flex()
            .items_center()
            .rounded(px(6.))
            .text_size(px(13.))
            .cursor_pointer()
            .when(active, |this| {
                this.bg(Palette::face())
                    .text_color(Palette::text())
                    .font_weight(FontWeight::MEDIUM)
            })
            .when(!active, |this| {
                this.text_color(Palette::text_secondary())
                    .hover(|this| this.text_color(Palette::text()).bg(Palette::wash(0.05)))
            })
            .on_click(cx.listener(move |this, _, _, cx| this.show(page, cx)))
            .child(label)
    };
    h_flex().gap_5().child(brand(ctl)).child(
        controls(h_flex().gap_1())
            .child(tab("tab-modules", "Modules", Page::Modules, cx))
            .child(tab("tab-ports", "Ports", Page::Ports, cx))
            .child(tab("tab-settings", "Settings", Page::Settings, cx)),
    )
}

pub fn render_global_actions(ctl: &Controller, cx: &Context<Panel>) -> impl IntoElement {
    let busy = ctl.rows.iter().any(|r| r.busy());
    controls(h_flex().gap_2())
        .child(
            Button::new("start-all")
                .label("Start all")
                .small()
                .outline()
                .disabled(busy || ctl.all_running())
                .on_click(on_controller(cx, |c, cx| c.start_all(cx))),
        )
        .child(
            Button::new("stop-all")
                .label("Stop all")
                .small()
                .outline()
                .disabled(busy || !ctl.any_running())
                .on_click(on_controller(cx, |c, cx| c.stop_all(cx))),
        )
        .child(render_style_toggle(false, cx))
}

/// Switches between the full window and the compact one.
pub fn render_style_toggle(compact: bool, cx: &Context<Panel>) -> impl IntoElement {
    let (icon, tooltip) = if compact {
        (IconName::Maximize2, "Full window")
    } else {
        (IconName::Minimize2, "Compact window")
    };
    controls(div()).child(
        Button::new("window-style")
            .icon(icon)
            .small()
            .ghost()
            .tooltip_with_action(tooltip, &ToggleCompact, Some(panel::CONTEXT))
            .on_click(
                cx.listener(move |this, _, _, cx| this.set_compact(!compact, Page::Modules, cx)),
            ),
    )
}
