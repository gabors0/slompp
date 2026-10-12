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

use slompp_core::{DbAdmin, Module};

use crate::theme::Lamp;

/// What the tray (or a second launch of SLOMPP) asks the app to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    Toggle(Module),
    StartAll,
    StopAll,
    OpenSite,
    OpenDbAdmin(DbAdmin),
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
    /// The database admin tools that are installed.
    pub db_admins: Vec<DbAdmin>,
    /// Whether the icon sits on a light panel, so it's drawn in black.
    pub light_panel: bool,
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
        menu.push(item("Open site", apache, TrayCommand::OpenSite));
        for &tool in &self.db_admins {
            menu.push(item(
                &format!("Open {}", tool.name()),
                apache && self.running(Module::MySql),
                TrayCommand::OpenDbAdmin(tool),
            ));
        }
        menu.push(MenuEntry::Separator);
        menu.push(item("Show panel", true, TrayCommand::ShowPanel));
        menu.push(item("Quit SLOMPP", true, TrayCommand::Quit));
        menu
    }
}

/// Whether the panel the icon sits on is light, given whether the system's
/// colour scheme is. Desktops don't tell tray icons, and the panel mostly
/// follows the colour scheme, except: GNOME's top bar is always dark, and a
/// Plasma style other than the default one (Breeze, which follows the
/// colour scheme) can be dark or light on its own.
pub fn light_panel(light_scheme: bool) -> bool {
    #[cfg(target_os = "linux")]
    {
        let desktops = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
        let desktop = |name: &str| desktops.split(':').any(|d| d.eq_ignore_ascii_case(name));
        if desktop("GNOME") {
            return false;
        }
        if desktop("KDE")
            && let Some(light) = plasma_style().as_deref().and_then(plasma_style_is_light)
        {
            return light;
        }
    }
    light_scheme
}

/// The Plasma style's name, from `plasmarc`.
#[cfg(target_os = "linux")]
fn plasma_style() -> Option<String> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::home_dir().map(|home| home.join(".config")))?;
    let text = std::fs::read_to_string(config.join("plasmarc")).ok()?;
    let mut in_theme = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_theme = line == "[Theme]";
        } else if in_theme && let Some(name) = line.strip_prefix("name=") {
            return Some(name.trim().to_string());
        }
    }
    None
}

/// Whether a Plasma style has a light panel, for the ones whose name says.
#[cfg(target_os = "linux")]
fn plasma_style_is_light(name: &str) -> Option<bool> {
    let name = name.to_ascii_lowercase();
    if name.contains("dark") {
        Some(false)
    } else if name.contains("light") {
        Some(true)
    } else {
        None
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
        let ink = icon::Ink::for_panel(self.state.light_panel);
        [16, 22, 24, 32, 48, 64]
            .into_iter()
            .map(|size| {
                let icon::Pixmap {
                    width,
                    height,
                    data,
                } = icon::render(size, apache, mysql, ink);
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
///
/// The units are solid white on dark panels and black on light ones, like
/// the panel's other icons. A lamp is a slot cut out of its unit, empty when
/// stopped and filled with the lamp's colour otherwise, in shades that stand
/// out against the unit around them.
pub mod icon {
    use crate::theme::Lamp;

    type Rgb = [u8; 3];

    /// What the icon is drawn in.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Ink {
        /// For dark panels.
        White,
        /// For light panels.
        Black,
    }

    impl Ink {
        pub fn for_panel(light: bool) -> Self {
            if light { Ink::Black } else { Ink::White }
        }

        fn color(self) -> Rgb {
            match self {
                Ink::White => [0xFF, 0xFF, 0xFF],
                Ink::Black => [0x00, 0x00, 0x00],
            }
        }

        /// A lit lamp's colour: deeper on white units, brighter on black.
        fn lamp(self, lamp: Lamp) -> Option<Rgb> {
            let (on_white, on_black) = match lamp {
                Lamp::Off => return None,
                Lamp::Busy => ([0xC7, 0x84, 0x00], [0xE9, 0xB4, 0x4C]),
                Lamp::On => ([0x14, 0x9A, 0x50], [0x4C, 0xC3, 0x8A]),
                Lamp::Fault => ([0xD9, 0x33, 0x27], [0xEF, 0x64, 0x58]),
            };
            Some(match self {
                Ink::White => on_white,
                Ink::Black => on_black,
            })
        }
    }

    /// Unpremultiplied ARGB32 pixels, a byte per channel in that order.
    pub struct Pixmap {
        pub width: i32,
        pub height: i32,
        pub data: Vec<u8>,
    }

    /// A rounded rectangle in the 32×32 design grid. Alpha 0 cuts a hole.
    struct Shape {
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        radius: f32,
        color: Rgb,
        alpha: f32,
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

    fn shapes(apache: Lamp, mysql: Lamp, ink: Ink) -> Vec<Shape> {
        let mut shapes = Vec::new();
        let rows = [(3.5, Some(apache)), (12.5, Some(mysql)), (21.5, None)];
        for (y, lamp) in rows {
            shapes.push(Shape {
                x0: 3.0,
                y0: y,
                x1: 29.0,
                y1: y + 7.0,
                radius: 2.0,
                color: ink.color(),
                alpha: 1.0,
            });
            let Some(lamp) = lamp else { continue };
            let (color, alpha) = match ink.lamp(lamp) {
                Some(color) => (color, 1.0),
                None => (ink.color(), 0.0),
            };
            shapes.push(Shape {
                x0: 16.5,
                y0: y + 1.75,
                x1: 26.5,
                y1: y + 5.25,
                radius: 1.2,
                color,
                alpha,
            });
        }
        shapes
    }

    /// The icon at `size`×`size`, 4×4 supersampled.
    pub fn render(size: i32, apache: Lamp, mysql: Lamp, ink: Ink) -> Pixmap {
        const SS: i32 = 4;
        let shapes = shapes(apache, mysql, ink);
        let scale = 32.0 / size as f32;
        let mut data = Vec::with_capacity((size * size * 4) as usize);
        for py in 0..size {
            for px in 0..size {
                // Premultiplied sums over the samples.
                let (mut r, mut g, mut b, mut a) = (0.0, 0.0, 0.0, 0.0);
                for sy in 0..SS {
                    for sx in 0..SS {
                        let x = (px as f32 + (sx as f32 + 0.5) / SS as f32) * scale;
                        let y = (py as f32 + (sy as f32 + 0.5) / SS as f32) * scale;
                        // The topmost shape wins.
                        if let Some(shape) = shapes.iter().rev().find(|s| s.contains(x, y)) {
                            r += f32::from(shape.color[0]) * shape.alpha;
                            g += f32::from(shape.color[1]) * shape.alpha;
                            b += f32::from(shape.color[2]) * shape.alpha;
                            a += shape.alpha;
                        }
                    }
                }
                let unpremultiply = |c: f32| if a > 0.0 { (c / a).round() as u8 } else { 0 };
                data.extend_from_slice(&[
                    (a * 255.0 / (SS * SS) as f32).round() as u8,
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
            let icon = render(32, Lamp::On, Lamp::Fault, Ink::White);
            assert_eq!(icon.data.len(), 32 * 32 * 4);
            // The middle of each lamp, and the corner (transparent).
            assert_eq!(pixel(&icon, 21, 7), [255, 0x14, 0x9A, 0x50]);
            assert_eq!(pixel(&icon, 21, 16), [255, 0xD9, 0x33, 0x27]);
            assert_eq!(pixel(&icon, 0, 0)[0], 0);
            // The bottom unit has no lamp, just the solid unit.
            assert_eq!(pixel(&icon, 21, 25), [255, 0xFF, 0xFF, 0xFF]);
        }

        #[test]
        fn off_is_a_hole_and_black_ink_gets_bright_lamps() {
            let icon = render(32, Lamp::Off, Lamp::Busy, Ink::Black);
            // Apache's slot is cut out of its unit.
            assert_eq!(pixel(&icon, 21, 7)[0], 0);
            assert_eq!(pixel(&icon, 10, 7), [255, 0, 0, 0]);
            assert_eq!(pixel(&icon, 21, 16), [255, 0xE9, 0xB4, 0x4C]);
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
            db_admins: Vec::new(),
            light_panel: false,
        };
        assert_eq!(
            state.summary(),
            "Apache: Running on 8080, 8443\nMySQL: Stopped"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn plasma_styles_named_dark_or_light() {
        assert_eq!(plasma_style_is_light("breeze-dark"), Some(false));
        assert_eq!(plasma_style_is_light("breeze-light"), Some(true));
        assert_eq!(plasma_style_is_light("default"), None);
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
            db_admins: vec![DbAdmin::Adminer],
            light_panel: false,
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
                entry(
                    "Open Adminer",
                    false,
                    TrayCommand::OpenDbAdmin(DbAdmin::Adminer)
                ),
                MenuEntry::Separator,
                entry("Show panel", true, TrayCommand::ShowPanel),
                entry("Quit SLOMPP", true, TrayCommand::Quit),
            ]
        );
    }
}
