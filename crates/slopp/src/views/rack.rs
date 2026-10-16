//! The module rack: one unit per server with its LED, ports, processes and
//! controls.

use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{Disableable as _, Sizable as _, TITLE_BAR_HEIGHT, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, Context, FontWeight, Hsla, Pixels, SharedString, div,
    px,
};
use slopp_core::{DbAdmin, Module, Problem};

use crate::controller::{Controller, Row};
use crate::panel::{Panel, on_controller};
use crate::theme::{self, Lamp, Palette};

const UNIT_HEIGHT: f32 = 76.;
const COMPACT_UNIT_HEIGHT: f32 = 52.;
/// The compact window's content: the title bar, then (with the padding
/// and gap around them) the rack with its border and the bar under it.
pub fn compact_height() -> Pixels {
    TITLE_BAR_HEIGHT
        + px(12. + (Module::ALL.len() as f32 * COMPACT_UNIT_HEIGHT + 2.) + 12. + 24. + 12.)
}
/// The LED's bezel, and the gap from it to the module's name.
const BEZEL_WIDTH: f32 = 18.;
const BEZEL_GAP: f32 = 14.;

pub fn render(ctl: &Controller, cx: &Context<Panel>) -> impl IntoElement {
    let mut rack = v_flex()
        .flex_none()
        .rounded(px(8.))
        .border_1()
        .border_color(Palette::border())
        .bg(Palette::face())
        .overflow_hidden();
    for (i, row) in ctl.rows.iter().enumerate() {
        rack = rack.child(render_unit(ctl, row, i == 0, cx));
    }
    rack
}

fn render_unit(ctl: &Controller, row: &Row, first: bool, cx: &Context<Panel>) -> AnyElement {
    let m = row.module;
    let lamp = row.lamp();
    let stack = ctl.stack();
    let describe = stack
        .describe(m)
        .unwrap_or_else(|| "Not installed".to_string());
    let missing = row
        .problems
        .iter()
        .any(|p| matches!(p, Problem::Missing { .. }));

    let name = v_flex()
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
                        .text_color(Palette::text_tertiary())
                        .child(row.state_label()),
                ),
        )
        .child(
            div()
                .text_size(px(12.))
                .text_color(Palette::text_secondary())
                .truncate()
                .child(describe),
        );
    let face = h_flex().h(px(UNIT_HEIGHT)).child(
        h_flex()
            .flex_1()
            .h_full()
            .pl_4()
            .pr_3()
            .gap_6()
            .child(h_flex().gap(px(BEZEL_GAP)).child(led(m, lamp)).child(name))
            .child(render_ports(ctl, row))
            .child(render_pids(row))
            .child(div().flex_1())
            .child(render_actions(ctl, row, missing, cx)),
    );

    let notes = notes(ctl, row);
    v_flex()
        .when(!first, |this| {
            this.border_t_1().border_color(Palette::border())
        })
        .child(face)
        .when(!notes.is_empty(), |this| {
            this.child(
                v_flex()
                    // Under the module's name.
                    .pl(px(16. + BEZEL_WIDTH + BEZEL_GAP))
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
        })
        .into_any_element()
}

/// A front-panel LED in a recessed bezel: dark when off, lit in the
/// lamp's colour with a glint along its top, pulsing while busy.
fn led(m: Module, lamp: Lamp) -> AnyElement {
    let body = div().w(px(12.)).h(px(5.)).rounded(px(1.));
    let body = match lamp {
        Lamp::Off => body.bg(Palette::border()),
        _ => body.bg(Palette::lamp(lamp)).child(
            div()
                .mx(px(2.))
                .h(px(1.))
                .bg(gpui_kit::white().opacity(0.45)),
        ),
    };
    let body = match lamp {
        Lamp::Busy => body
            .with_animation(
                SharedString::from(format!("led-{}", m.id())),
                Animation::new(Duration::from_millis(1100))
                    .repeat()
                    .with_easing(gpui_kit::pulsating_between(0.3, 1.0)),
                |this, t| this.opacity(t),
            )
            .into_any_element(),
        _ => body.into_any_element(),
    };
    div()
        .flex_none()
        .w(px(BEZEL_WIDTH))
        .h(px(11.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(2.))
        .border_1()
        .border_color(Palette::border())
        .bg(Palette::well())
        .child(body)
        .into_any_element()
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
    let stack = ctl.stack();
    let id = m.id();

    // Disabled rather than left out when there's nothing (Mailpit has no
    // config files), so the units' buttons stay in line.
    let files_menu = |kind: &'static str, files: Vec<slopp_core::stack::NamedFile>| {
        Button::new(SharedString::from(format!("{kind}-{id}")))
            .small()
            .ghost()
            .label(if kind == "config" { "Config" } else { "Logs" })
            .disabled(files.is_empty())
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
        .child(admin(ctl, row, cx))
        .child(files_menu("config", stack.config_files(m)))
        .child(files_menu("logs", stack.log_files(m)))
        .child(div().w_2())
        .child(toggle(ctl, row, missing, cx))
}

/// Start or Stop, or Install while the package manager can install what's
/// missing.
fn toggle(ctl: &Controller, row: &Row, missing: bool, cx: &Context<Panel>) -> AnyElement {
    let m = row.module;
    let running = row.running();
    let stack = ctl.stack();
    let id = m.id();

    if missing && !running && !stack.installable(m).is_empty() {
        let getting = ctl.getting.as_ref().map(|(module, _)| *module);
        return Button::new(SharedString::from(format!("install-{id}")))
            .w(px(76.))
            .small()
            .primary()
            .label("Install")
            .tooltip(format!(
                "Install the missing servers {}",
                slopp_core::install::via(&stack.installable(m))
            ))
            .loading(getting == Some(m))
            .disabled(getting.is_some())
            .on_click(on_controller(cx, move |c, cx| c.install_servers(m, cx)))
            .into_any_element();
    }
    let toggle = Button::new(SharedString::from(format!("toggle-{id}")))
        .w(px(76.))
        .small()
        .label(if running { "Stop" } else { "Start" })
        .loading(row.busy())
        .disabled(row.busy() || (missing && !running))
        .on_click(on_controller(cx, move |c, cx| c.toggle(m, cx)));
    if running {
        toggle.outline().into_any_element()
    } else {
        toggle.primary().into_any_element()
    }
}

/// The module's Admin: the site, the database tools' menu or the inbox.
fn admin(ctl: &Controller, row: &Row, cx: &Context<Panel>) -> AnyElement {
    let running = row.running();
    let id = row.module.id();
    match row.module {
        Module::Apache => Button::new(SharedString::from(format!("admin-{id}")))
            .small()
            .ghost()
            .label("Admin")
            .tooltip("Open the site in your browser")
            .disabled(!running)
            .on_click(on_controller(cx, |c, cx| c.open_site(cx)))
            .into_any_element(),
        Module::MySql => db_admin_menu(ctl, running).into_any_element(),
        Module::Mailpit => Button::new(SharedString::from(format!("admin-{id}")))
            .small()
            .ghost()
            .label("Admin")
            .tooltip("Open Mailpit's inbox in your browser")
            .disabled(!running)
            .on_click(on_controller(cx, |c, cx| c.open_mailpit(cx)))
            .into_any_element(),
    }
}

/// MySQL's Admin menu: open phpMyAdmin or Adminer, download or update them.
fn db_admin_menu(ctl: &Controller, mysql_running: bool) -> impl IntoElement {
    let layout = ctl.stack().layout();
    let installed: Vec<(DbAdmin, bool)> = DbAdmin::ALL
        .into_iter()
        .map(|tool| (tool, tool.installed(layout)))
        .collect();
    let can_open = mysql_running && ctl.row(Module::Apache).running();
    let installing = ctl.installing.is_some();
    Button::new("admin-mysql")
        .small()
        .ghost()
        .label("Admin")
        .dropdown_caret(true)
        .dropdown_menu_with_anchor(gpui_kit::Anchor::TopRight, move |mut menu, _, _| {
            let open = |&(tool, installed): &(DbAdmin, bool)| installed.then_some(tool);
            let opens: Vec<DbAdmin> = installed.iter().filter_map(open).collect();
            for &tool in &opens {
                menu = menu.item(
                    PopupMenuItem::new(format!("Open {}", tool.name()))
                        .disabled(!can_open)
                        .on_click(move |_, _, cx| {
                            Controller::global(cx).update(cx, |c, cx| c.open_db_admin(tool, cx));
                        }),
                );
            }
            if !opens.is_empty() {
                menu = menu.separator();
            }
            // Updating keeps the config and fetches the latest release.
            for &(tool, installed) in &installed {
                let label = if installed {
                    format!("Update {}", tool.name())
                } else {
                    format!("Get {} from {}", tool.name(), tool.site())
                };
                menu = menu.item(PopupMenuItem::new(label).disabled(installing).on_click(
                    move |_, _, cx| {
                        Controller::global(cx).update(cx, |c, cx| c.install_db_admin(tool, cx));
                    },
                ));
            }
            menu
        })
}

/// The compact window's rack: per module its LED, name, state and ports,
/// Admin and Start/Stop; config and logs are in the full window.
pub fn render_compact(ctl: &Controller, cx: &Context<Panel>) -> impl IntoElement {
    let mut rack = v_flex()
        .rounded(px(8.))
        .border_1()
        .border_color(Palette::border())
        .bg(Palette::face())
        .overflow_hidden();
    for (i, row) in ctl.rows.iter().enumerate() {
        rack = rack.child(render_compact_unit(ctl, row, i == 0, cx));
    }
    rack
}

fn render_compact_unit(
    ctl: &Controller,
    row: &Row,
    first: bool,
    cx: &Context<Panel>,
) -> AnyElement {
    let m = row.module;
    let missing = row
        .problems
        .iter()
        .any(|p| matches!(p, Problem::Missing { .. }));
    let ports = ctl
        .stack()
        .ports(m)
        .into_iter()
        .map(|(port, _)| port.to_string())
        .collect::<Vec<_>>()
        .join(" ");

    let name = v_flex()
        .flex_1()
        .min_w_0()
        .child(
            div()
                .text_size(px(14.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(m.name()),
        )
        .child(
            h_flex()
                .gap_2()
                .text_size(px(12.))
                .child(
                    div()
                        .text_color(Palette::text_secondary())
                        .whitespace_nowrap()
                        .child(row.state_label()),
                )
                .child(
                    div()
                        .font_family(theme::MONO)
                        .text_color(if row.running() {
                            Palette::text_secondary()
                        } else {
                            Palette::text_tertiary()
                        })
                        .truncate()
                        .child(ports),
                ),
        );
    let face = h_flex()
        .h(px(COMPACT_UNIT_HEIGHT))
        .pl_3()
        .pr_2()
        .gap(px(BEZEL_GAP - 2.))
        .child(led(m, row.lamp()))
        .child(name)
        .child(
            h_flex()
                .flex_none()
                .gap_1()
                .child(admin(ctl, row, cx))
                .child(toggle(ctl, row, missing, cx)),
        );

    let notes = notes(ctl, row);
    v_flex()
        .when(!first, |this| {
            this.border_t_1().border_color(Palette::border())
        })
        .child(face)
        .when(!notes.is_empty(), |this| {
            this.child(
                v_flex()
                    // Under the module's name.
                    .pl(px(12. + BEZEL_WIDTH + BEZEL_GAP - 2.))
                    .pr_3()
                    .pb_2()
                    .mt(px(-6.))
                    .gap_0p5()
                    .children(notes.into_iter().map(|(text, tone)| {
                        div()
                            .text_size(px(12.))
                            .text_color(tone)
                            .child(SharedString::from(text))
                    })),
            )
        })
        .into_any_element()
}

/// Under the compact rack: the folder and shell buttons, and the
/// whole-stack actions like a dialog's.
pub fn render_compact_bar(ctl: &Controller, cx: &Context<Panel>) -> impl IntoElement {
    let htdocs = ctl.stack().layout().htdocs();
    let busy = ctl.rows.iter().any(|r| r.busy());
    h_flex()
        .flex_none()
        .gap_1()
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
            Button::new("open-shell")
                .xsmall()
                .ghost()
                .label("Shell")
                .tooltip("Open a terminal with mariadb, php, httpd and mailpit on PATH")
                .on_click(on_controller(cx, |c, cx| c.open_shell(cx))),
        )
        .child(div().flex_1())
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

/// Lines under a unit: why it can't start, and an install's or a download's
/// progress.
fn notes(ctl: &Controller, row: &Row) -> Vec<(String, Hsla)> {
    let mut notes: Vec<(String, Hsla)> = row
        .problems
        .iter()
        .map(|p| (p.to_string(), Palette::fault().into()))
        .collect();
    if let Some((module, text)) = &ctl.getting
        && *module == row.module
    {
        notes.push((text.clone(), Palette::text_secondary().into()));
    }
    if row.module == Module::MySql
        && let Some((tool, text)) = &ctl.installing
    {
        notes.push((
            format!("{}: {text}", tool.name()),
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
                .tooltip("Open a terminal with mariadb, php, httpd and mailpit on PATH")
                .on_click(on_controller(cx, |c, cx| c.open_shell(cx))),
        )
}
