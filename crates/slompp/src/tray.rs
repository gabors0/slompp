//! The tray icon: a StatusNotifierItem over D-Bus on Linux (which KDE and
//! most panels show), a menu bar item on macOS. Its rack icon has a lamp per
//! module, the tooltip says what runs where, and the menu starts and stops
//! things without the window.
//!
//! Menu clicks come back to the app as [`TrayCommand`]s over a channel (on
//! Linux the tray runs on its own thread).

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "macos")]
pub use macos::{TrayHandle, spawn};

use slompp_core::Module;

use crate::theme::Lamp;

/// What the tray (or a second launch of SLOMPP) asks the app to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    Toggle(Module),
    StartAll,
    StopAll,
    OpenSite,
    OpenPhpMyAdmin,
    ShowPanel,
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayModule {
    pub module: Module,
    pub lamp: Lamp,
    pub running: bool,
    pub busy: bool,
    pub label: &'static str,
    /// Open ports while running.
    pub ports: Vec<u16>,
}

/// Everything the tray shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayState {
    pub modules: Vec<TrayModule>,
    pub site_url: Option<String>,
    /// `None` until phpMyAdmin is installed.
    pub phpmyadmin_url: Option<String>,
}

impl TrayState {
    fn module(&self, module: Module) -> Option<&TrayModule> {
        self.modules.iter().find(|m| m.module == module)
    }

    fn running(&self, module: Module) -> bool {
        self.module(module).is_some_and(|m| m.running)
    }

    fn lamp(&self, module: Module) -> Lamp {
        self.module(module).map_or(Lamp::Off, |m| m.lamp)
    }

    /// One line per module, e.g. `Apache: Running on 8080, 8443`.
    pub fn summary(&self) -> String {
        self.modules
            .iter()
            .map(|m| {
                if m.running && !m.ports.is_empty() && m.label == "Running" {
                    let ports: Vec<String> = m.ports.iter().map(u16::to_string).collect();
                    format!("{}: Running on {}", m.module.name(), ports.join(", "))
                } else {
                    format!("{}: {}", m.module.name(), m.label)
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// A line of the tray's menu.
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuEntry {
    Item {
        label: String,
        enabled: bool,
        command: TrayCommand,
    },
    Separator,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl TrayState {
    /// The menu for this state, the same on every platform.
    pub fn menu(&self) -> Vec<MenuEntry> {
        let item = |label: &str, enabled, command| MenuEntry::Item {
            label: label.to_string(),
            enabled,
            command,
        };
        let mut menu = Vec::new();
        for m in &self.modules {
            let verb = if m.running { "Stop" } else { "Start" };
            menu.push(item(
                &format!("{verb} {}", m.module.name()),
                !m.busy,
                TrayCommand::Toggle(m.module),
            ));
        }
        let busy = self.modules.iter().any(|m| m.busy);
        let all = self.modules.iter().all(|m| m.running);
        let any = self.modules.iter().any(|m| m.running);
        menu.push(MenuEntry::Separator);
        menu.push(item("Start all", !busy && !all, TrayCommand::StartAll));
        menu.push(item("Stop all", !busy && any, TrayCommand::StopAll));
        menu.push(MenuEntry::Separator);
        let apache = self.running(Module::Apache);
        menu.push(item(
            "Open site",
            apache && self.site_url.is_some(),
            TrayCommand::OpenSite,
        ));
        if self.phpmyadmin_url.is_some() {
            menu.push(item(
                "Open phpMyAdmin",
                apache && self.running(Module::MySql),
                TrayCommand::OpenPhpMyAdmin,
            ));
        }
        menu.push(MenuEntry::Separator);
        menu.push(item("Show panel", true, TrayCommand::ShowPanel));
        menu.push(item("Quit SLOMPP", true, TrayCommand::Quit));
        menu
    }
}

#[cfg(target_os = "linux")]
struct SlomppTray {
    state: TrayState,
    commands: async_channel::Sender<TrayCommand>,
}

#[cfg(target_os = "linux")]
impl SlomppTray {
    fn send(&self, command: TrayCommand) {
        let _ = self.commands.try_send(command);
    }
}

#[cfg(target_os = "linux")]
fn item(
    label: impl Into<String>,
    enabled: bool,
    command: TrayCommand,
) -> ksni::MenuItem<SlomppTray> {
    ksni::menu::StandardItem {
        label: label.into(),
        enabled,
        activate: Box::new(move |tray: &mut SlomppTray| tray.send(command)),
        ..Default::default()
    }
    .into()
}

#[cfg(target_os = "linux")]
impl ksni::Tray for SlomppTray {
    fn id(&self) -> String {
        "slompp".into()
    }

    fn title(&self) -> String {
        "SLOMPP".into()
    }

    fn category(&self) -> ksni::Category {
        ksni::Category::ApplicationStatus
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        let apache = self.state.lamp(Module::Apache);
        let mysql = self.state.lamp(Module::MySql);
        [16, 22, 24, 32, 48, 64]
            .into_iter()
            .map(|size| {
                let icon::Pixmap {
                    width,
                    height,
                    data,
                } = icon::render(size, apache, mysql);
                ksni::Icon {
                    width,
                    height,
                    data,
                }
            })
            .collect()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "SLOMPP".into(),
            description: self.state.summary(),
            ..Default::default()
        }
    }

    /// A left click shows the panel.
    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(TrayCommand::ShowPanel);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        self.state
            .menu()
            .into_iter()
            .map(|entry| match entry {
                MenuEntry::Item {
                    label,
                    enabled,
                    command,
                } => item(label, enabled, command),
                MenuEntry::Separator => ksni::MenuItem::Separator,
            })
            .collect()
    }
}

#[cfg(target_os = "linux")]
/// The running tray icon.
pub struct TrayHandle(ksni::blocking::Handle<SlomppTray>);

#[cfg(target_os = "linux")]
impl TrayHandle {
    pub fn set(&self, state: TrayState) {
        self.0.update(|tray| tray.state = state);
    }
}

#[cfg(target_os = "linux")]
/// Puts the icon in the system tray. Fails when nothing on the session bus
/// shows tray icons (e.g. GNOME without the AppIndicator extension).
///
/// `SLOMPP_TRAY_ASSUME=1` registers even without a tray host, for testing
/// over a private D-Bus session.
pub fn spawn(
    state: TrayState,
    commands: async_channel::Sender<TrayCommand>,
) -> anyhow::Result<TrayHandle> {
    use ksni::blocking::TrayMethods as _;
    let assume = std::env::var_os("SLOMPP_TRAY_ASSUME").is_some();
    let handle = SlomppTray { state, commands }
        .assume_sni_available(assume)
        .spawn()
        .map_err(|err| anyhow::anyhow!("no system tray available: {err}"))?;
    Ok(TrayHandle(handle))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
/// The tray icon, drawn in code: three rack units like the panel's mark.
/// The top lamp is Apache's, the middle one MySQL's.
pub mod icon {
    use crate::theme::Lamp;

    type Rgb = [u8; 3];

    /// Unpremultiplied ARGB32 pixels, a byte per channel in that order.
    pub struct Pixmap {
        pub width: i32,
        pub height: i32,
        pub data: Vec<u8>,
    }

    const UNIT: Rgb = [0x9B, 0xA3, 0xB3];
    const LAMP_OFF: Rgb = [0x4A, 0x51, 0x60];

    fn lamp_color(lamp: Lamp) -> Rgb {
        match lamp {
            Lamp::Off => LAMP_OFF,
            Lamp::Busy => [0xE9, 0xB4, 0x4C],
            Lamp::On => [0x4C, 0xC3, 0x8A],
            Lamp::Fault => [0xEF, 0x64, 0x58],
        }
    }

    /// A rounded rectangle in the 32×32 design grid.
    struct Shape {
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        radius: f32,
        color: Rgb,
    }

    impl Shape {
        fn contains(&self, x: f32, y: f32) -> bool {
            if x < self.x0 || x > self.x1 || y < self.y0 || y > self.y1 {
                return false;
            }
            // Distance into the corner circle, if in a corner.
            let cx = x.clamp(self.x0 + self.radius, self.x1 - self.radius);
            let cy = y.clamp(self.y0 + self.radius, self.y1 - self.radius);
            let (dx, dy) = (x - cx, y - cy);
            dx * dx + dy * dy <= self.radius * self.radius
        }
    }

    fn shapes(apache: Lamp, mysql: Lamp) -> Vec<Shape> {
        let mut shapes = Vec::new();
        let rows = [(3.5, Some(apache)), (12.5, Some(mysql)), (21.5, None)];
        for (y, lamp) in rows {
            shapes.push(Shape {
                x0: 3.0,
                y0: y,
                x1: 29.0,
                y1: y + 7.0,
                radius: 2.0,
                color: UNIT,
            });
            if let Some(lamp) = lamp {
                shapes.push(Shape {
                    x0: 17.0,
                    y0: y + 2.0,
                    x1: 26.5,
                    y1: y + 5.0,
                    radius: 1.2,
                    color: lamp_color(lamp),
                });
            }
        }
        shapes
    }

    /// The icon at `size`×`size`, 4×4 supersampled.
    pub fn render(size: i32, apache: Lamp, mysql: Lamp) -> Pixmap {
        const SS: i32 = 4;
        let shapes = shapes(apache, mysql);
        let scale = 32.0 / size as f32;
        let mut data = Vec::with_capacity((size * size * 4) as usize);
        for py in 0..size {
            for px in 0..size {
                // Premultiplied sums over the samples.
                let (mut r, mut g, mut b, mut a) = (0u32, 0u32, 0u32, 0u32);
                for sy in 0..SS {
                    for sx in 0..SS {
                        let x = (px as f32 + (sx as f32 + 0.5) / SS as f32) * scale;
                        let y = (py as f32 + (sy as f32 + 0.5) / SS as f32) * scale;
                        // The topmost shape wins.
                        if let Some(shape) = shapes.iter().rev().find(|s| s.contains(x, y)) {
                            r += u32::from(shape.color[0]);
                            g += u32::from(shape.color[1]);
                            b += u32::from(shape.color[2]);
                            a += 255;
                        }
                    }
                }
                let n = (SS * SS) as u32;
                let alpha = a / n;
                let unpremultiply = |c: u32| (c * 255).checked_div(a).unwrap_or(0) as u8;
                data.extend_from_slice(&[
                    alpha as u8,
                    unpremultiply(r),
                    unpremultiply(g),
                    unpremultiply(b),
                ]);
            }
        }
        Pixmap {
            width: size,
            height: size,
            data,
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn pixel(icon: &Pixmap, x: i32, y: i32) -> [u8; 4] {
            let i = ((y * icon.width + x) * 4) as usize;
            icon.data[i..i + 4].try_into().unwrap()
        }

        #[test]
        fn lamps_show_each_modules_state() {
            let icon = render(32, Lamp::On, Lamp::Fault);
            assert_eq!(icon.data.len(), 32 * 32 * 4);
            // The middle of each lamp, and the corner (transparent).
            assert_eq!(pixel(&icon, 21, 7), [255, 0x4C, 0xC3, 0x8A]);
            assert_eq!(pixel(&icon, 21, 16), [255, 0xEF, 0x64, 0x58]);
            assert_eq!(pixel(&icon, 0, 0)[0], 0);
            // The bottom unit has no lamp.
            assert_eq!(pixel(&icon, 21, 25), [255, 0x9B, 0xA3, 0xB3]);
        }
    }
}

/// No tray on Windows yet.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub struct TrayHandle;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
impl TrayHandle {
    pub fn set(&self, _: TrayState) {}
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn spawn(_: TrayState, _: async_channel::Sender<TrayCommand>) -> anyhow::Result<TrayHandle> {
    anyhow::bail!("the tray icon is only available on Linux and macOS so far")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module(module: Module, running: bool, label: &'static str, ports: &[u16]) -> TrayModule {
        TrayModule {
            module,
            lamp: if running { Lamp::On } else { Lamp::Off },
            running,
            busy: false,
            label,
            ports: ports.to_vec(),
        }
    }

    #[test]
    fn summary_lists_ports_of_running_modules() {
        let state = TrayState {
            modules: vec![
                module(Module::Apache, true, "Running", &[8080, 8443]),
                module(Module::MySql, false, "Stopped", &[]),
            ],
            site_url: None,
            phpmyadmin_url: None,
        };
        assert_eq!(
            state.summary(),
            "Apache: Running on 8080, 8443\nMySQL: Stopped"
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn menu_follows_what_runs() {
        let entry = |label: &str, enabled, command| MenuEntry::Item {
            label: label.to_string(),
            enabled,
            command,
        };
        let state = TrayState {
            modules: vec![
                module(Module::Apache, true, "Running", &[8080]),
                module(Module::MySql, false, "Stopped", &[]),
            ],
            site_url: Some("http://localhost:8080/".into()),
            phpmyadmin_url: None,
        };
        assert_eq!(
            state.menu(),
            vec![
                entry("Stop Apache", true, TrayCommand::Toggle(Module::Apache)),
                entry("Start MySQL", true, TrayCommand::Toggle(Module::MySql)),
                MenuEntry::Separator,
                entry("Start all", true, TrayCommand::StartAll),
                entry("Stop all", true, TrayCommand::StopAll),
                MenuEntry::Separator,
                entry("Open site", true, TrayCommand::OpenSite),
                MenuEntry::Separator,
                entry("Show panel", true, TrayCommand::ShowPanel),
                entry("Quit SLOMPP", true, TrayCommand::Quit),
            ]
        );
    }
}
