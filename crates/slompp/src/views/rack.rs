//! The module rack: one unit per server with its status rail and lamp,
//! ports, processes and controls.

use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, Context, FontWeight, Hsla, SharedString, div, px,
};
use slompp_core::{Module, Problem, phpmyadmin};

use crate::controller::{Controller, PmaState, Row};
use crate::panel::{Panel, on_controller};
use crate::theme::{self, Lamp, Palette};

const UNIT_HEIGHT: f32 = 76.;

pub fn render(ctl: &Controller, cx: &Context<Panel>) -> impl IntoElement {
    let mut rack = v_flex()
        .flex_none()
        .rounded(px(8.))
        .border_1()
        .border_color(Palette::border())
        .bg(Palette::face())
        .overflow_hidden();
    let last = ctl.rows.len().saturating_sub(1);
    for (i, row) in ctl.rows.iter().enumerate() {
        rack = rack.child(render_unit(ctl, row, i == 0, i == last, cx));
    }
    rack
}

fn render_unit(
    ctl: &Controller,
    row: &Row,
    first: bool,
    last: bool,
    cx: &Context<Panel>,
) -> AnyElement {
    let m = row.module;
    let lamp = row.lamp();
    let color = Palette::lamp(lamp);
    let stack = ctl.stack();
    let describe = stack
        .describe(m)
        .unwrap_or_else(|| "Not installed".to_string());
    let missing = row
        .problems
        .iter()
        .any(|p| matches!(p, Problem::Missing { .. }));

    let face = h_flex().h(px(UNIT_HEIGHT)).child(
        h_flex()
            .flex_1()
            .h_full()
            .pl_4()
            .pr_3()
            .gap_6()
            .child(lamp_dot(m, lamp, color))
            .child(
                v_flex()
                    .w(px(250.))
                    .gap_0p5()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_baseline()
                            .child(
                                div()
                                    .text_size(px(16.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(m.name()),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(match lamp {
                                        Lamp::Off => Palette::text_tertiary().into(),
                                        _ => color,
                                    })
                                    .child(row.state_label()),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(Palette::text_secondary())
                            .truncate()
                            .child(describe),
                    ),
            )
            .child(render_ports(ctl, row))
            .child(render_pids(row))
            .child(div().flex_1())
            .child(render_actions(ctl, row, missing, cx)),
    );

    let notes = notes(ctl, row);
    let content = v_flex()
        .flex_1()
        .child(face)
        .when(!notes.is_empty(), |this| {
            this.child(
                v_flex()
                    // Lines up under the module's name (measured on screen).
                    .pl(px(58.))
                    .pr_4()
                    .pb_3()
                    .mt(px(-10.))
                    .gap_0p5()
                    .children(notes.into_iter().map(|(text, tone)| {
                        div()
                            .text_size(px(12.))
                            .text_color(tone)
                            .child(SharedString::from(text))
                    })),
            )
        });
    h_flex()
        .items_stretch()
        .when(!first, |this| {
            this.border_t_1().border_color(Palette::border())
        })
        .child(rail(m, lamp, color, first, last))
        .child(content)
        .into_any_element()
}

/// The coloured bar down the unit's left edge: lit while the module runs.
/// `first`/`last` round its outer corners to the rack's (overflow_hidden
/// doesn't clip to rounded corners).
fn rail(m: Module, lamp: Lamp, color: Hsla, first: bool, last: bool) -> AnyElement {
    let base = div()
        .w(px(4.))
        .flex_none()
        .when(first, |this| this.rounded_tl(px(7.)))
        .when(last, |this| this.rounded_bl(px(7.)));
    match lamp {
        Lamp::Off => base.bg(Palette::border()).into_any_element(),
        Lamp::On | Lamp::Fault => base.bg(color).into_any_element(),
        Lamp::Busy => base
            .bg(color)
            .with_animation(
                SharedString::from(format!("rail-{}", m.id())),
                Animation::new(Duration::from_millis(1100))
                    .repeat()
                    .with_easing(gpui_kit::pulsating_between(0.35, 1.0)),
                |this, t| this.opacity(t),
            )
            .into_any_element(),
    }
}

/// The lamp, with a halo when lit.
fn lamp_dot(m: Module, lamp: Lamp, color: Hsla) -> AnyElement {
    let dot = div().size(px(10.)).rounded_full().bg(color);
    let halo = |alpha: f32| {
        div()
            .size(px(22.))
            .flex_none()
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(color.opacity(alpha))
    };
    match lamp {
        Lamp::Off => halo(0.)
            .child(
                div()
                    .size(px(10.))
                    .rounded_full()
                    .border_1()
                    .border_color(color),
            )
            .into_any_element(),
        Lamp::On => halo(0.18).child(dot).into_any_element(),
        Lamp::Fault => halo(0.16).child(dot).into_any_element(),
        Lamp::Busy => halo(0.16)
            .child(dot)
            .with_animation(
                SharedString::from(format!("lamp-{}", m.id())),
                Animation::new(Duration::from_millis(1100))
                    .repeat()
                    .with_easing(gpui_kit::pulsating_between(0.35, 1.0)),
                |this, t| this.opacity(t),
            )
            .into_any_element(),
    }
}

/// A small caption over a monospaced value.
fn reading(label: &'static str, value: impl Into<SharedString>, live: bool) -> impl IntoElement {
    v_flex()
        .gap_0p5()
        .child(
            div()
                .text_size(px(11.))
                .text_color(Palette::text_tertiary())
                .child(label),
        )
        .child(
            div()
                .font_family(theme::MONO)
                .text_size(px(15.))
                .text_color(if live {
                    Palette::text()
                } else {
                    Palette::text_tertiary()
                })
                .child(value.into()),
        )
}

fn render_ports(ctl: &Controller, row: &Row) -> impl IntoElement {
    let stack = ctl.stack();
    let mut ports = h_flex().w(px(150.)).gap_5();
    for (port, label) in stack.ports(row.module) {
        let open = row
            .status
            .as_ref()
            .and_then(|s| s.ports.iter().find(|p| p.port == port))
            .is_some_and(|p| p.open)
            && row.running();
        ports = ports.child(reading(label, port.to_string(), open));
    }
    ports
}

fn render_pids(row: &Row) -> impl IntoElement {
    let pids = row
        .status
        .as_ref()
        .map(|s| s.pids.clone())
        .unwrap_or_default();
    let text = if pids.is_empty() {
        "—".to_string()
    } else {
        pids.iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(" ")
    };
    div().w(px(130.)).child(reading(
        if pids.len() > 1 { "PIDs" } else { "PID" },
        text,
        !pids.is_empty(),
    ))
}

fn render_actions(
    ctl: &Controller,
    row: &Row,
    missing: bool,
    cx: &Context<Panel>,
) -> impl IntoElement {
    let m = row.module;
    let running = row.running();
    let stack = ctl.stack();
    let id = m.id();

    let toggle = Button::new(SharedString::from(format!("toggle-{id}")))
        .w(px(76.))
        .small()
        .label(if running { "Stop" } else { "Start" })
        .loading(row.busy())
        .disabled(row.busy() || (missing && !running))
        .on_click(on_controller(cx, move |c, cx| c.toggle(m, cx)));
    let toggle = if running {
        toggle.outline()
    } else {
        toggle.primary()
    };

    let admin: AnyElement = match m {
        Module::MySql if !phpmyadmin::installed(stack.layout()) => {
            let installing = matches!(ctl.pma, PmaState::Installing(_));
            Button::new("install-pma")
                .small()
                .ghost()
                .label("Get phpMyAdmin")
                .tooltip("Download phpMyAdmin from phpmyadmin.net")
                .loading(installing)
                .disabled(installing)
                .on_click(on_controller(cx, |c, cx| c.install_phpmyadmin(cx)))
                .into_any_element()
        }
        _ => Button::new(SharedString::from(format!("admin-{id}")))
            .small()
            .ghost()
            .label("Admin")
            .tooltip(match m {
                Module::Apache => "Open the site in your browser",
                Module::MySql => "Open phpMyAdmin",
            })
            .disabled(!ctl.row(Module::Apache).running() || (m == Module::MySql && !running))
            .on_click(on_controller(cx, move |c, cx| c.open_admin(m, cx)))
            .into_any_element(),
    };

    let files_menu = |kind: &'static str, files: Vec<slompp_core::stack::NamedFile>| {
        Button::new(SharedString::from(format!("{kind}-{id}")))
            .small()
            .ghost()
            .label(if kind == "config" { "Config" } else { "Logs" })
            .dropdown_caret(true)
            .dropdown_menu_with_anchor(gpui_kit::Anchor::TopRight, move |mut menu, _, _| {
                for file in &files {
                    let path = file.path.clone();
                    let exists = path.exists();
                    menu = menu.item(PopupMenuItem::new(file.label).disabled(!exists).on_click(
                        move |_, _, cx| {
                            let path = path.clone();
                            Controller::global(cx).update(cx, |c, cx| c.open_file(path, cx));
                        },
                    ));
                }
                menu
            })
    };

    h_flex()
        .gap_1()
        .child(admin)
        .child(files_menu("config", stack.config_files(m)))
        .child(files_menu("logs", stack.log_files(m)))
        .child(div().w_2())
        .child(toggle)
}

/// Lines under a unit: why it can't start, and phpMyAdmin's progress.
fn notes(ctl: &Controller, row: &Row) -> Vec<(String, Hsla)> {
    let mut notes: Vec<(String, Hsla)> = row
        .problems
        .iter()
        .map(|p| (p.to_string(), Palette::fault().into()))
        .collect();
    if row.module == Module::MySql
        && let PmaState::Installing(text) = &ctl.pma
    {
        notes.push((
            format!("phpMyAdmin: {text}"),
            Palette::text_secondary().into(),
        ));
    }
    notes
}

/// Under the rack: where the stack lives, and the folder and shell buttons.
pub fn render_stack_bar(ctl: &Controller, cx: &Context<Panel>) -> impl IntoElement {
    let root = ctl.stack().layout().root().display().to_string();
    let htdocs = ctl.stack().layout().htdocs();
    h_flex()
        .flex_none()
        .px_1()
        .gap_2()
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .gap_2()
                .text_size(px(12.))
                .child(div().text_color(Palette::text_tertiary()).child("Stack"))
                .child(
                    div()
                        .font_family(theme::MONO)
                        .text_color(Palette::text_secondary())
                        .truncate()
                        .child(root),
                ),
        )
        .child(
            Button::new("open-htdocs")
                .xsmall()
                .ghost()
                .label("htdocs")
                .tooltip("Open the web root, where your projects go")
                .on_click(on_controller(cx, move |c, cx| {
                    c.open_folder(htdocs.clone(), cx)
                })),
        )
        .child(
            Button::new("open-root")
                .xsmall()
                .ghost()
                .label("Folder")
                .tooltip("Open the stack folder")
                .on_click(on_controller(cx, |c, cx| c.open_root(cx))),
        )
        .child(
            Button::new("open-shell")
                .xsmall()
                .ghost()
                .label("Shell")
                .tooltip("Open a terminal with mariadb, php and httpd on PATH")
                .on_click(on_controller(cx, |c, cx| c.open_shell(cx))),
        )
}
