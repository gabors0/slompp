//! Settings: ports, the stack folder, editor and terminal, autostart, the
//! panel's appearance, and paths to the server programs when they can't be
//! found automatically.

use std::path::PathBuf;

use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Selectable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, Context, Entity, EventEmitter, FontWeight, SharedString, Window, div, px,
};
use slopp_core::settings::{Appearance, Binaries, Ports};
use slopp_core::toolchain::PhpHandler;
use slopp_core::{Layout, Module, Settings, Stack};

use crate::theme::{self, Palette};

pub enum SettingsEvent {
    Save(Box<Settings>),
    /// Picked in the Appearance group, which applies and saves at once.
    Appearance(Appearance),
}

struct Field {
    state: Entity<InputState>,
}

impl Field {
    fn new(
        value: Option<String>,
        placeholder: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<SettingsView>,
    ) -> Self {
        let placeholder = placeholder.into();
        let state = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(placeholder)
                .default_value(value.unwrap_or_default())
        });
        Self { state }
    }

    fn text(&self, cx: &gpui_kit::App) -> String {
        self.state.read(cx).value().trim().to_string()
    }

    fn optional(&self, cx: &gpui_kit::App) -> Option<String> {
        Some(self.text(cx)).filter(|s| !s.is_empty())
    }

    fn path(&self, cx: &gpui_kit::App) -> Option<PathBuf> {
        self.optional(cx).map(PathBuf::from)
    }
}

pub struct SettingsView {
    http: Field,
    https: Field,
    mysql: Field,
    smtp: Field,
    mailpit_web: Field,
    root: Field,
    editor: Field,
    terminal: Field,
    httpd: Field,
    apache_modules: Field,
    php_fpm: Field,
    php_module: Field,
    mariadbd: Field,
    mailpit: Field,
    autostart_apache: bool,
    autostart_mysql: bool,
    autostart_mailpit: bool,
    stop_on_exit: bool,
    close_to_tray: bool,
    appearance: Appearance,
    error: Option<String>,
}

impl EventEmitter<SettingsEvent> for SettingsView {}

fn path_text(p: &Option<PathBuf>) -> Option<String> {
    p.as_ref().map(|p| p.display().to_string())
}

/// "Found: …" for a program SLOPP located by itself.
fn found(path: Option<&std::path::Path>) -> String {
    match path {
        Some(p) => format!("Found: {}", p.display()),
        None => "Not found".to_string(),
    }
}

impl SettingsView {
    pub fn new(stack: &Stack, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let s = stack.settings();
        let tools = stack.tools();
        let ports = s.ports;
        let b = &s.binaries;
        let (fpm, module) = match tools.php.as_ref().map(|p| &p.handler) {
            Some(PhpHandler::Fpm(p)) => (Some(p.as_path()), None),
            Some(PhpHandler::Module(p)) => (None, Some(p.as_path())),
            None => (None, None),
        };
        let httpd = tools.httpd.as_ref();
        Self {
            http: Field::new(Some(ports.http.to_string()), "80", window, cx),
            https: Field::new(Some(ports.https.to_string()), "443", window, cx),
            mysql: Field::new(Some(ports.mysql.to_string()), "3306", window, cx),
            smtp: Field::new(Some(ports.smtp.to_string()), "1025", window, cx),
            mailpit_web: Field::new(Some(ports.mailpit.to_string()), "8025", window, cx),
            root: Field::new(
                path_text(&s.root),
                Layout::default_root().display().to_string(),
                window,
                cx,
            ),
            editor: Field::new(
                s.editor.clone(),
                if cfg!(target_os = "macos") {
                    "The default text editor (e.g. code, Zed, Sublime Text)"
                } else if cfg!(windows) {
                    "The default app, or Notepad (e.g. code, notepad++)"
                } else {
                    "The desktop's default app (e.g. code, kate, gnome-text-editor)"
                },
                window,
                cx,
            ),
            terminal: Field::new(
                s.terminal.clone(),
                if cfg!(target_os = "macos") {
                    "Terminal (e.g. iTerm, Ghostty, kitty)"
                } else if cfg!(windows) {
                    "Command Prompt (e.g. pwsh, wt)"
                } else {
                    "Found automatically (e.g. konsole, kitty, gnome-terminal)"
                },
                window,
                cx,
            ),
            httpd: Field::new(
                path_text(&b.httpd),
                found(httpd.map(|h| h.bin.as_path())),
                window,
                cx,
            ),
            apache_modules: Field::new(
                path_text(&b.apache_modules),
                found(httpd.and_then(|h| h.modules.as_deref())),
                window,
                cx,
            ),
            php_fpm: Field::new(path_text(&b.php_fpm), found(fpm), window, cx),
            php_module: Field::new(
                path_text(&b.php_module),
                match module {
                    Some(p) => found(Some(p)),
                    None => "Not used: PHP runs through PHP-FPM".to_string(),
                },
                window,
                cx,
            ),
            mariadbd: Field::new(
                path_text(&b.mariadbd),
                found(tools.mariadb.as_ref().map(|m| m.server.as_path())),
                window,
                cx,
            ),
            mailpit: Field::new(
                path_text(&b.mailpit),
                found(tools.mailpit.as_ref().map(|m| m.bin.as_path())),
                window,
                cx,
            ),
            autostart_apache: s.autostart.contains(&Module::Apache),
            autostart_mysql: s.autostart.contains(&Module::MySql),
            autostart_mailpit: s.autostart.contains(&Module::Mailpit),
            stop_on_exit: s.stop_on_exit,
            close_to_tray: s.close_to_tray,
            appearance: s.appearance,
            error: None,
        }
    }

    /// Shows the saved settings (and newly found programs) again.
    pub fn reload(&mut self, stack: &Stack, window: &mut Window, cx: &mut Context<Self>) {
        *self = Self::new(stack, window, cx);
        cx.notify();
    }

    fn collect(&self, cx: &gpui_kit::App) -> Result<Settings, String> {
        let port = |field: &Field, name: &str| -> Result<u16, String> {
            let text = field.text(cx);
            match text.parse::<u16>() {
                Ok(p) if p > 0 => Ok(p),
                _ => Err(format!("The {name} port must be a number from 1 to 65535.")),
            }
        };
        let ports = Ports {
            http: port(&self.http, "HTTP")?,
            https: port(&self.https, "HTTPS")?,
            mysql: port(&self.mysql, "MySQL")?,
            smtp: port(&self.smtp, "SMTP")?,
            mailpit: port(&self.mailpit_web, "Mailpit")?,
        };
        let all = [
            ports.http,
            ports.https,
            ports.mysql,
            ports.smtp,
            ports.mailpit,
        ];
        if all.iter().enumerate().any(|(i, p)| all[..i].contains(p)) {
            return Err("Each port can only be used once.".into());
        }
        let mut autostart = Vec::new();
        if self.autostart_apache {
            autostart.push(Module::Apache);
        }
        if self.autostart_mysql {
            autostart.push(Module::MySql);
        }
        if self.autostart_mailpit {
            autostart.push(Module::Mailpit);
        }
        let root = self.root.path(cx);
        if let Some(root) = &root
            && !root.is_absolute()
        {
            return Err("The stack folder must be a full path, like /home/you/slopp.".into());
        }
        Ok(Settings {
            root,
            ports,
            binaries: Binaries {
                httpd: self.httpd.path(cx),
                apache_modules: self.apache_modules.path(cx),
                php_fpm: self.php_fpm.path(cx),
                php_module: self.php_module.path(cx),
                mariadbd: self.mariadbd.path(cx),
                mariadb_install_db: None,
                mariadb_client: None,
                mailpit: self.mailpit.path(cx),
            },
            editor: self.editor.optional(cx),
            terminal: self.terminal.optional(cx),
            autostart,
            stop_on_exit: self.stop_on_exit,
            close_to_tray: self.close_to_tray,
            appearance: self.appearance,
        })
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        match self.collect(cx) {
            Ok(settings) => {
                self.error = None;
                cx.emit(SettingsEvent::Save(Box::new(settings)));
            }
            Err(err) => self.error = Some(err),
        }
        cx.notify();
    }
}

/// A settings group: what it is on the left, its controls on the right.
fn section(
    title: &'static str,
    about: &'static str,
    controls: impl IntoElement,
    first: bool,
) -> impl IntoElement {
    h_flex()
        .items_start()
        .gap_8()
        .py_5()
        .when(!first, |this| {
            this.border_t_1().border_color(Palette::border())
        })
        .child(
            v_flex()
                .w(px(220.))
                .flex_none()
                .gap_1()
                .child(
                    div()
                        .text_size(px(14.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .line_height(px(18.))
                        .text_color(Palette::text_secondary())
                        .child(about),
                ),
        )
        .child(v_flex().flex_1().min_w_0().gap_3().child(controls))
}

/// A labelled input.
fn labelled(label: &'static str, field: &Field, mono: bool) -> impl IntoElement {
    v_flex()
        .gap_1()
        .child(
            div()
                .text_size(px(12.))
                .text_color(Palette::text_secondary())
                .child(label),
        )
        .child(
            Input::new(&field.state)
                .small()
                .when(mono, |this| this.font_family(theme::MONO)),
        )
}

fn toggle(
    id: &'static str,
    label: &'static str,
    on: bool,
    cx: &mut Context<SettingsView>,
    set: fn(&mut SettingsView, bool),
) -> AnyElement {
    h_flex()
        .gap_3()
        .child(Switch::new(id).checked(on).on_change(cx.listener(
            move |this, value: &bool, _, cx| {
                set(this, *value);
                cx.notify();
            },
        )))
        .child(div().text_size(px(13.)).child(label))
        .into_any_element()
}

impl Render for SettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let port = |label, field| div().w(px(100.)).child(labelled(label, field, true));
        let ports = h_flex()
            .gap_3()
            .child(port("HTTP", &self.http))
            .child(port("HTTPS", &self.https))
            .child(port("MySQL", &self.mysql))
            .child(port("SMTP", &self.smtp))
            .child(port("Mailpit", &self.mailpit_web));

        let startup = v_flex()
            .gap_3()
            .child(toggle(
                "autostart-apache",
                "Start Apache when SLOPP opens",
                self.autostart_apache,
                cx,
                |this, v| this.autostart_apache = v,
            ))
            .child(toggle(
                "autostart-mysql",
                "Start MySQL when SLOPP opens",
                self.autostart_mysql,
                cx,
                |this, v| this.autostart_mysql = v,
            ))
            .child(toggle(
                "autostart-mailpit",
                "Start Mailpit when SLOPP opens",
                self.autostart_mailpit,
                cx,
                |this, v| this.autostart_mailpit = v,
            ))
            .child(toggle(
                "close-to-tray",
                if cfg!(target_os = "macos") {
                    "Keep SLOPP in the menu bar when its window closes"
                } else {
                    "Keep SLOPP in the system tray when its window closes"
                },
                self.close_to_tray,
                cx,
                |this, v| this.close_to_tray = v,
            ))
            .child(toggle(
                "stop-on-exit",
                "Stop the servers when SLOPP quits",
                self.stop_on_exit,
                cx,
                |this, v| this.stop_on_exit = v,
            ));

        // The picked one is filled, like a switch that's on.
        let appearance = ButtonGroup::new("appearance")
            .small()
            .children(Appearance::ALL.map(|a| {
                let picked = a == self.appearance;
                Button::new(a.name())
                    .label(a.name())
                    .when(picked, |this| this.primary())
                    .when(!picked, |this| this.outline())
                    .selected(picked)
            }))
            .on_click(cx.listener(|this, clicked: &Vec<usize>, _, cx| {
                if let Some(&a) = clicked.first().and_then(|&i| Appearance::ALL.get(i))
                    && a != this.appearance
                {
                    this.appearance = a;
                    cx.emit(SettingsEvent::Appearance(a));
                    cx.notify();
                }
            }));

        let footer = h_flex()
            .gap_3()
            .py_4()
            .border_t_1()
            .border_color(Palette::border())
            .child(
                Button::new("save-settings")
                    .primary()
                    .small()
                    .label("Save settings")
                    .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
            )
            .when_some(self.error.clone(), |this, error| {
                this.child(
                    div()
                        .text_size(px(13.))
                        .text_color(Palette::fault())
                        .child(error),
                )
            });

        div()
            .id("settings")
            .size_full()
            .overflow_y_scroll()
            .child(
                v_flex()
                    .max_w(px(860.))
                    .px_6()
                    .pt_2()
                    .child(section(
                        "Ports",
                        "Where Apache, MySQL and Mailpit listen (SMTP for mail, Mailpit for its page). Running servers pick up new ports when restarted.",
                        ports,
                        true,
                    ))
                    .child(section(
                        "Stack folder",
                        "Holds htdocs, the configuration, databases and logs. Leave empty for the default.",
                        labelled("Folder", &self.root, true),
                        false,
                    ))
                    .child(section(
                        "Opening files",
                        "The commands used for the Config and Logs menus and the Shell button.",
                        v_flex()
                            .gap_3()
                            .child(labelled("Editor", &self.editor, false))
                            .child(labelled("Terminal", &self.terminal, false)),
                        false,
                    ))
                    .child(section("Startup", "What happens when SLOPP opens and quits.", startup, false))
                    .child(section(
                        "Appearance",
                        "Ayu Dark or Ayu Light, or whichever matches the system. Changes apply right away.",
                        h_flex().child(appearance),
                        false,
                    ))
                    .child(section(
                        "Programs",
                        "SLOPP looks for Apache, PHP, MariaDB and Mailpit on your PATH. Set a path here to use a different one.",
                        v_flex()
                            .gap_3()
                            .child(labelled("Apache (httpd)", &self.httpd, true))
                            .child(labelled("Apache modules folder", &self.apache_modules, true))
                            .child(labelled("PHP-FPM", &self.php_fpm, true))
                            .child(labelled(
                                if cfg!(windows) {
                                    "PHP Apache module (php8apache2_4.dll)"
                                } else {
                                    "PHP Apache module (libphp.so)"
                                },
                                &self.php_module,
                                true,
                            ))
                            .child(labelled("MariaDB server (mariadbd)", &self.mariadbd, true))
                            .child(labelled("Mailpit", &self.mailpit, true)),
                        false,
                    ))
                    .child(footer),
            )
    }
}
