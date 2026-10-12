//! The log console under the rack.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{Context, FontWeight, Hsla, div, px};

use crate::controller::Controller;
use crate::log::Kind;
use crate::panel::{Panel, on_controller};
use crate::theme::{self, Palette};

pub fn render(panel: &Panel, ctl: &Controller, cx: &Context<Panel>) -> impl IntoElement {
    let lines = ctl.log.entries().iter().map(|entry| {
        let tone: Hsla = match entry.kind {
            Kind::Info => Palette::text().into(),
            Kind::Ok => Palette::ok().into(),
            Kind::Error => Palette::fault().into(),
            Kind::Detail => Palette::text_secondary().into(),
        };
        let source = entry.module.map(|m| m.name()).unwrap_or("Panel");
        h_flex()
            .items_start()
            .gap_3()
            .child(
                div()
                    .flex_none()
                    .text_color(Palette::text_tertiary())
                    .child(entry.time.clone()),
            )
            .child(
                div()
                    .flex_none()
                    .w(px(52.))
                    .text_color(Palette::text_secondary())
                    .child(source),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_color(tone)
                    .when(entry.kind == Kind::Detail, |this| this.pl_3())
                    .child(entry.text.clone()),
            )
    });

    v_flex()
        .flex_1()
        .min_h(px(120.))
        .rounded(px(8.))
        .border_1()
        .border_color(Palette::border())
        .bg(Palette::well())
        .overflow_hidden()
        .child(
            h_flex()
                .flex_none()
                .justify_between()
                .pl_3()
                .pr_1()
                .h(px(32.))
                .border_b_1()
                .border_color(Palette::border())
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(Palette::text_secondary())
                        .child("Log"),
                )
                .child(
                    Button::new("clear-log")
                        .xsmall()
                        .ghost()
                        .label("Clear")
                        .on_click(on_controller(cx, |c, cx| c.clear_log(cx))),
                ),
        )
        .child(
            div()
                .id("log-lines")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .track_scroll(&panel.log_scroll)
                .px_3()
                .py_2()
                .font_family(theme::MONO)
                .text_size(px(12.5))
                .line_height(px(20.))
                .children(lines),
        )
}
