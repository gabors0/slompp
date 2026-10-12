//! The title bar: SLOPP's mark, the page tabs and the whole-stack actions.

use gpui_kit::component::button::Button;
use gpui_kit::component::{Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{Context, Div, FontWeight, MouseButton, div, px};

use crate::controller::Controller;
use crate::panel::{Page, Panel, on_controller};
use crate::theme::Palette;

/// SLOPP's mark, as in the tray: three rack units joined into an S by a
/// link on the left (top to middle) and one on the right (middle to bottom).
pub fn mark() -> Div {
    const WIDTH: f32 = 18.;
    const UNIT: f32 = 5.;
    const GAP: f32 = 2.;
    // The units' height to their width, as in the design.
    const LINK: f32 = WIDTH * 112. / 428.;
    let link = |left: f32, top_unit: f32| {
        div()
            .absolute()
            .left(px(left))
            .top(px(top_unit * (UNIT + GAP) + UNIT / 2.))
            .w(px(LINK))
            .h(px(UNIT + GAP))
            .bg(Palette::mark_link())
    };
    let unit = || {
        div()
            .w(px(WIDTH))
            .h(px(UNIT))
            .rounded(px(1.))
            .bg(Palette::mark())
    };
    // The links first, so the units cover their ends.
    v_flex()
        .relative()
        .gap(px(GAP))
        .child(link(0., 0.))
        .child(link(WIDTH - LINK, 1.))
        .child(unit())
        .child(unit())
        .child(unit())
}

/// Keeps presses on controls inside the title bar to themselves: the bar
/// starts a window move when the pointer moves while pressed, and on KWin
/// that move swallows the click.
pub fn controls(element: Div) -> Div {
    element.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

pub fn render_nav(panel: &Panel, cx: &Context<Panel>) -> impl IntoElement {
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
    h_flex()
        .gap_5()
        .child(
            h_flex().gap_2().child(mark()).child(
                div()
                    .text_size(px(14.))
                    .font_weight(FontWeight::BOLD)
                    .child("SLOPP"),
            ),
        )
        .child(
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
}
