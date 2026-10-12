//! Ports: every program listening on a TCP port (XAMPP's Netstat window),
//! with SLOMPP's own ports called out.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{Context, FontWeight, SharedString, div, px};
use slompp_core::Module;

use crate::controller::Controller;
use crate::panel::{Panel, on_controller};
use crate::theme::{self, Palette};

pub fn render(ctl: &Controller, cx: &Context<Panel>) -> impl IntoElement {
    let stack = ctl.stack();
    // Which module each configured port belongs to.
    let ours: Vec<(u16, Module, &'static str)> = Module::ALL
        .into_iter()
        .flat_map(|m| {
            stack
                .ports(m)
                .into_iter()
                .map(move |(port, label)| (port, m, label))
        })
        .collect();

    let header = h_flex()
        .px_4()
        .h(px(32.))
        .flex_none()
        .border_b_1()
        .border_color(Palette::border())
        .text_size(px(12.))
        .text_color(Palette::text_tertiary())
        .child(div().w(px(90.)).child("Port"))
        .child(div().w(px(260.)).child("Address"))
        .child(div().w(px(90.)).child("PID"))
        .child(div().flex_1().child("Program"));

    let rows = ctl.listeners.iter().enumerate().map(|(i, l)| {
        let owner = ours.iter().find(|(port, ..)| *port == l.port());
        let unknown = || "—".to_string();
        h_flex()
            .id(("listener", i))
            .px_4()
            .h(px(30.))
            .flex_none()
            .when(i > 0, |this| {
                this.border_t_1().border_color(Palette::border())
            })
            .text_size(px(13.))
            .child(
                div()
                    .w(px(90.))
                    .font_family(theme::MONO)
                    .when(owner.is_some(), |this| this.font_weight(FontWeight::MEDIUM))
                    .child(l.port().to_string()),
            )
            .child(
                div()
                    .w(px(260.))
                    .font_family(theme::MONO)
                    .text_color(Palette::text_secondary())
                    .truncate()
                    .child(l.addr.ip().to_string()),
            )
            .child(
                div()
                    .w(px(90.))
                    .font_family(theme::MONO)
                    .text_color(Palette::text_secondary())
                    .child(l.pid.map(|p| p.to_string()).unwrap_or_else(unknown)),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_2()
                    .child(
                        div().truncate().child(
                            l.process
                                .clone()
                                .unwrap_or_else(|| "Another user's program".to_string()),
                        ),
                    )
                    .when_some(owner, |this, (_, m, label)| {
                        this.child(
                            div()
                                .px_1p5()
                                .rounded(px(4.))
                                .text_size(px(11.))
                                .bg(Palette::wash(0.08))
                                .text_color(Palette::text_secondary())
                                .child(SharedString::from(if *label == m.name() {
                                    label.to_string()
                                } else {
                                    format!("{} {label}", m.name())
                                })),
                        )
                    }),
            )
    });

    let empty = ctl.listeners.is_empty();
    v_flex()
        .flex_1()
        .min_h_0()
        .p_4()
        .gap_3()
        .child(
            h_flex()
                .flex_none()
                .justify_between()
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(Palette::text_secondary())
                        .child("Programs listening for connections on this computer"),
                )
                .child(
                    Button::new("refresh-ports")
                        .small()
                        .ghost()
                        .label("Refresh")
                        .on_click(on_controller(cx, |c, cx| c.refresh_ports(cx))),
                ),
        )
        .child(
            v_flex()
                .flex_1()
                .min_h_0()
                .rounded(px(8.))
                .border_1()
                .border_color(Palette::border())
                .bg(Palette::face())
                .overflow_hidden()
                .child(header)
                .child(
                    div()
                        .id("ports-list")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .when(empty, |this| {
                            this.child(
                                div()
                                    .p_4()
                                    .text_size(px(13.))
                                    .text_color(Palette::text_secondary())
                                    .child("Looking for listening ports…"),
                            )
                        })
                        .children(rows),
                ),
        )
}
