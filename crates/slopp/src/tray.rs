//! The tray icon: a StatusNotifierItem over D-Bus on Linux (which KDE and
//! most panels show), a menu bar item on macOS, a notification area icon on
//! Windows. Its rack icon has a lamp per
//! module, the tooltip says what runs where, and the menu starts and stops
//! things without the window.
//!
//! Menu clicks come back to the app as [`TrayCommand`]s over a channel (on
//! Linux the tray runs on its own thread).

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "macos")]
pub use macos::{TrayHandle, spawn};

#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use windows::{TrayHandle, spawn};

use slopp_core::{DbAdmin, Module};

use crate::theme::Lamp;

/// What the tray (or a second launch of SLOPP) asks the app to do.
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

/// `text` in a fixed UTF-16 buffer with room for a terminating NUL, cut at
/// a character boundary if it's too long (the Windows tooltip's 128).
#[cfg(any(windows, test))]
pub fn utf16_fit<const N: usize>(text: &str) -> [u16; N] {
    let mut buf = [0u16; N];
    let mut len = 0;
    for c in text.chars() {
        let mut units = [0u16; 2];
        let units = c.encode_utf16(&mut units);
        if len + units.len() >= N {
            break;
        }
        buf[len..len + units.len()].copy_from_slice(units);
        len += units.len();
    }
    buf
}

/// What sits in front of a menu item's label.
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuIcon {
    /// A module's lamp, as a dot of its colour (a ring while it's off).
    Lamp(Lamp),
    Start,
    Stop,
    Site,
    Database,
    Panel,
    Quit,
}

/// A line of the tray's menu.
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuEntry {
    Item {
        label: String,
        icon: MenuIcon,
        enabled: bool,
        command: TrayCommand,
    },
    Separator,
}

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
impl TrayState {
    /// The menu for this state, the same on every platform.
    pub fn menu(&self) -> Vec<MenuEntry> {
        let item = |label: &str, icon, enabled, command| MenuEntry::Item {
            label: label.to_string(),
            icon,
            enabled,
            command,
        };
        let busy = self.modules.iter().any(|m| m.busy);
        let all = self.modules.iter().all(|m| m.running);
        let any = self.modules.iter().any(|m| m.running);
        let mut menu = vec![
            item(
                "Start all",
                MenuIcon::Start,
                !busy && !all,
                TrayCommand::StartAll,
            ),
            item(
                "Stop all",
                MenuIcon::Stop,
                !busy && any,
                TrayCommand::StopAll,
            ),
            MenuEntry::Separator,
        ];
        for m in &self.modules {
            let verb = if m.running { "Stop" } else { "Start" };
            menu.push(item(
                &format!("{verb} {}", m.module.name()),
                MenuIcon::Lamp(m.lamp),
                !m.busy,
                TrayCommand::Toggle(m.module),
            ));
        }
        menu.push(MenuEntry::Separator);
        let apache = self.running(Module::Apache);
        menu.push(item(
            "Open site",
            MenuIcon::Site,
            apache,
            TrayCommand::OpenSite,
        ));
        for &tool in &self.db_admins {
            menu.push(item(
                &format!("Open {}", tool.name()),
                MenuIcon::Database,
                apache && self.running(Module::MySql),
                TrayCommand::OpenDbAdmin(tool),
            ));
        }
        menu.push(MenuEntry::Separator);
        menu.push(item(
            "Show panel",
            MenuIcon::Panel,
            true,
            TrayCommand::ShowPanel,
        ));
        menu.push(item("Quit SLOPP", MenuIcon::Quit, true, TrayCommand::Quit));
        menu
    }
}

/// Whether the panel the icon sits on is light, given whether the system's
/// colour scheme is. Desktops don't tell tray icons, and the panel mostly
/// follows the colour scheme, except: GNOME's top bar is always dark, and a
/// Plasma style other than the default one (Breeze, which follows the
/// colour scheme) can be dark or light on its own. Windows sets the
/// taskbar's mode apart from the apps' and says which it is.
pub fn light_panel(light_scheme: bool) -> bool {
    #[cfg(windows)]
    if let Some(light) = windows::taskbar_light() {
        return light;
    }
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
struct SloppTray {
    state: TrayState,
    commands: async_channel::Sender<TrayCommand>,
}

#[cfg(target_os = "linux")]
impl SloppTray {
    fn send(&self, command: TrayCommand) {
        let _ = self.commands.try_send(command);
    }
}

/// Menu icons are 16 pixels; lamps are drawn at twice that for HiDPI panels.
#[cfg(target_os = "linux")]
const LAMP_PIXELS: i32 = 32;

#[cfg(target_os = "linux")]
impl MenuIcon {
    /// The icon's name in the desktop's icon theme; lamps are drawn instead.
    fn theme_name(self) -> &'static str {
        match self {
            MenuIcon::Lamp(_) => "",
            MenuIcon::Start => "media-playback-start",
            MenuIcon::Stop => "media-playback-stop",
            MenuIcon::Site => "internet-web-browser",
            MenuIcon::Database => "server-database",
            MenuIcon::Panel => "view-restore",
            MenuIcon::Quit => "application-exit",
        }
    }
}

#[cfg(target_os = "linux")]
fn item(
    label: impl Into<String>,
    icon: MenuIcon,
    light_panel: bool,
    enabled: bool,
    command: TrayCommand,
) -> ksni::MenuItem<SloppTray> {
    let icon_data = match icon {
        MenuIcon::Lamp(lamp) => {
            icon::lamp(LAMP_PIXELS, lamp, icon::Ink::for_panel(light_panel)).png()
        }
        _ => Vec::new(),
    };
    ksni::menu::StandardItem {
        label: label.into(),
        enabled,
        icon_name: icon.theme_name().into(),
        icon_data,
        activate: Box::new(move |tray: &mut SloppTray| tray.send(command)),
        ..Default::default()
    }
    .into()
}

#[cfg(target_os = "linux")]
impl ksni::Tray for SloppTray {
    fn id(&self) -> String {
        "slopp".into()
    }

    fn title(&self) -> String {
        "SLOPP".into()
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
            title: "SLOPP".into(),
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
                    icon,
                    enabled,
                    command,
                } => item(label, icon, self.state.light_panel, enabled, command),
                MenuEntry::Separator => ksni::MenuItem::Separator,
            })
            .collect()
    }
}

#[cfg(target_os = "linux")]
/// The running tray icon.
pub struct TrayHandle(ksni::blocking::Handle<SloppTray>);

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
/// `SLOPP_TRAY_ASSUME=1` registers even without a tray host, for testing
/// over a private D-Bus session.
pub fn spawn(
    state: TrayState,
    commands: async_channel::Sender<TrayCommand>,
) -> anyhow::Result<TrayHandle> {
    use ksni::blocking::TrayMethods as _;
    let assume = std::env::var_os("SLOPP_TRAY_ASSUME").is_some();
    let handle = SloppTray { state, commands }
        .assume_sni_available(assume)
        .spawn()
        .map_err(|err| anyhow::anyhow!("no system tray available: {err}"))?;
    Ok(TrayHandle(handle))
}

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
/// The tray icon, drawn in code: SLOPP's mark, three rack units joined into
/// an S by a link on the left (top to middle) and one on the right (middle
/// to bottom). The top unit's lamp is Apache's, the middle one's MySQL's.
///
/// The units are solid white on dark panels and black on light ones, like
/// the panel's other icons, with grey links. A lit lamp is a gradient of its
/// colour over the unit's right side, fading out towards the middle.
pub mod icon {
    use crate::theme::{Lamp, MarkInk};

    type Rgb = [u8; 3];

    /// The design grid: the artwork is drawn on a 512×512 canvas.
    const GRID: f32 = 512.0;
    /// The units' left and right edges.
    const LEFT: f32 = 42.0;
    const RIGHT: f32 = 470.0;
    /// Each unit's top edge; units are 112 high, 48 apart.
    const UNITS: [f32; 3] = [42.0, 202.0, 362.0];
    const UNIT_HEIGHT: f32 = 112.0;
    const LINK_WIDTH: f32 = 112.0;
    const RADIUS: f32 = 20.0;
    /// Where a lamp's gradient is fully transparent.
    const LAMP_CLEAR: f32 = 185.0;

    /// What the icon is drawn in.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Ink {
        /// For dark panels.
        White,
        /// For light panels.
        Black,
        /// A macOS template image, which AppKit tints to suit the menu bar:
        /// only alpha counts, so the links are half-transparent and there
        /// are no lamps.
        #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
        Template,
    }

    impl Ink {
        pub fn for_panel(light: bool) -> Self {
            if light { Ink::Black } else { Ink::White }
        }

        fn colors(self) -> &'static MarkInk {
            match self {
                Ink::White => &MarkInk::WHITE,
                Ink::Black | Ink::Template => &MarkInk::BLACK,
            }
        }

        fn unit(self) -> Rgb {
            rgb(self.colors().unit)
        }

        /// The links' colour and alpha.
        fn link(self) -> (Rgb, f32) {
            match self {
                // As dark as the black ink's links on white.
                Ink::Template => ([0x00, 0x00, 0x00], 0.55),
                _ => (rgb(self.colors().link), 1.0),
            }
        }

        /// The colour and alpha of an unlit lamp's ring in the menu.
        fn lamp_off(self) -> (Rgb, f32) {
            self.link()
        }

        /// A lit lamp's colour.
        fn lamp(self, lamp: Lamp) -> Option<Rgb> {
            match self {
                Ink::Template => None,
                _ => self.colors().lamp(lamp).map(rgb),
            }
        }

        /// Where a lamp's gradient becomes solid.
        fn lamp_solid(self) -> f32 {
            LAMP_CLEAR + self.colors().lamp_solid * (RIGHT - LAMP_CLEAR)
        }
    }

    fn rgb(hex: u32) -> Rgb {
        let [_, r, g, b] = hex.to_be_bytes();
        [r, g, b]
    }

    /// Unpremultiplied ARGB32 pixels, a byte per channel in that order.
    pub struct Pixmap {
        pub width: i32,
        pub height: i32,
        pub data: Vec<u8>,
    }

    /// How a shape is filled.
    enum Fill {
        Solid {
            color: Rgb,
            alpha: f32,
        },
        /// A horizontal gradient, transparent at `clear` and opaque from
        /// `solid` on.
        Fade {
            color: Rgb,
            clear: f32,
            solid: f32,
        },
    }

    /// A rounded rectangle in the design grid.
    struct Shape {
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        radius: f32,
        fill: Fill,
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

        fn paint(&self, x: f32) -> (Rgb, f32) {
            match self.fill {
                Fill::Solid { color, alpha } => (color, alpha),
                Fill::Fade {
                    color,
                    clear,
                    solid,
                } => (color, ((x - clear) / (solid - clear)).clamp(0.0, 1.0)),
            }
        }
    }

    fn shapes(apache: Lamp, mysql: Lamp, ink: Ink) -> Vec<Shape> {
        let (link, link_alpha) = ink.link();
        let link = |x0: f32, top: usize| Shape {
            x0,
            // From the middle of one unit to the middle of the next: only
            // the part between them shows.
            y0: UNITS[top] + UNIT_HEIGHT / 2.0,
            x1: x0 + LINK_WIDTH,
            y1: UNITS[top + 1] + UNIT_HEIGHT / 2.0,
            radius: 0.0,
            fill: Fill::Solid {
                color: link,
                alpha: link_alpha,
            },
        };
        // Behind the units.
        let mut shapes = vec![link(LEFT, 0), link(RIGHT - LINK_WIDTH, 1)];
        for (y, lamp) in UNITS.into_iter().zip([Some(apache), Some(mysql), None]) {
            let unit = |x0, fill| Shape {
                x0,
                y0: y,
                x1: RIGHT,
                y1: y + UNIT_HEIGHT,
                radius: RADIUS,
                fill,
            };
            shapes.push(unit(
                LEFT,
                Fill::Solid {
                    color: ink.unit(),
                    alpha: 1.0,
                },
            ));
            if let Some(color) = lamp.and_then(|lamp| ink.lamp(lamp)) {
                shapes.push(unit(
                    LAMP_CLEAR,
                    Fill::Fade {
                        color,
                        clear: LAMP_CLEAR,
                        solid: ink.lamp_solid(),
                    },
                ));
            }
        }
        shapes
    }

    /// The icon at `size`×`size`.
    pub fn render(size: i32, apache: Lamp, mysql: Lamp, ink: Ink) -> Pixmap {
        let shapes = shapes(apache, mysql, ink);
        rasterize(size, |x, y| {
            // Each shape over the ones before it, premultiplied.
            let mut sample = [0.0f32; 4];
            for shape in shapes.iter().filter(|s| s.contains(x, y)) {
                let (color, alpha) = shape.paint(x);
                for c in 0..3 {
                    sample[c] = f32::from(color[c]) * alpha + sample[c] * (1.0 - alpha);
                }
                sample[3] = alpha + sample[3] * (1.0 - alpha);
            }
            sample
        })
    }

    /// A module's lamp for the menu, `size`×`size`: a dot of the lamp's
    /// colour, or a grey ring while it's off, so the two also differ in
    /// shape.
    pub fn lamp(size: i32, lamp: Lamp, ink: Ink) -> Pixmap {
        const CENTER: f32 = GRID / 2.0;
        const DOT_RADIUS: f32 = 150.0;
        const RING_WIDTH: f32 = 56.0;
        let lit = ink.lamp(lamp);
        let (color, alpha) = lit.map_or(ink.lamp_off(), |color| (color, 1.0));
        rasterize(size, |x, y| {
            let distance = (x - CENTER).hypot(y - CENTER);
            let hole = lit.is_none() && distance < DOT_RADIUS - RING_WIDTH;
            if distance > DOT_RADIUS || hole {
                return [0.0; 4];
            }
            let [r, g, b] = color.map(|c| f32::from(c) * alpha);
            [r, g, b, alpha]
        })
    }

    /// A `size`×`size` picture, 4×4 supersampled, of what `sample` gives for
    /// a point of the design grid as premultiplied RGBA (alpha 0 to 1).
    fn rasterize(size: i32, sample: impl Fn(f32, f32) -> [f32; 4]) -> Pixmap {
        const SS: i32 = 4;
        let scale = GRID / size as f32;
        let mut data = Vec::with_capacity((size * size * 4) as usize);
        for py in 0..size {
            for px in 0..size {
                // Premultiplied sums over the samples.
                let (mut r, mut g, mut b, mut a) = (0.0, 0.0, 0.0, 0.0);
                for sy in 0..SS {
                    for sx in 0..SS {
                        let x = (px as f32 + (sx as f32 + 0.5) / SS as f32) * scale;
                        let y = (py as f32 + (sy as f32 + 0.5) / SS as f32) * scale;
                        let sample = sample(x, y);
                        r += sample[0];
                        g += sample[1];
                        b += sample[2];
                        a += sample[3];
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

    #[cfg(any(windows, test))]
    impl Pixmap {
        /// The pixels as Windows bitmaps hold them: blue, green, red,
        /// alpha. Icons take straight alpha, menu item bitmaps premultiplied.
        pub fn bgra(&self, premultiplied: bool) -> Vec<u8> {
            let mut out = Vec::with_capacity(self.data.len());
            for &[a, r, g, b] in self.data.as_chunks::<4>().0 {
                let c = |c: u8| {
                    if premultiplied {
                        (u16::from(c) * u16::from(a) / 255) as u8
                    } else {
                        c
                    }
                };
                out.extend_from_slice(&[c(b), c(g), c(r), a]);
            }
            out
        }
    }

    #[cfg(any(target_os = "linux", test))]
    impl Pixmap {
        /// The picture as a PNG file, which is how menu items over D-Bus
        /// take their icons. The pixels go in uncompressed: it's a few
        /// kilobytes.
        pub fn png(&self) -> Vec<u8> {
            // Each row is a filter byte (0: none) and its RGBA pixels.
            let mut rows = Vec::with_capacity(self.data.len() + self.height as usize);
            for row in self.data.chunks_exact(self.width as usize * 4) {
                rows.push(0);
                for &[a, r, g, b] in row.as_chunks::<4>().0 {
                    rows.extend_from_slice(&[r, g, b, a]);
                }
            }
            // A zlib stream of stored blocks, which hold up to 65535 bytes.
            let mut zlib = vec![0x78, 0x01];
            let mut blocks = rows.chunks(0xFFFF).peekable();
            while let Some(block) = blocks.next() {
                let len = block.len() as u16;
                zlib.push(u8::from(blocks.peek().is_none()));
                zlib.extend_from_slice(&len.to_le_bytes());
                zlib.extend_from_slice(&(!len).to_le_bytes());
                zlib.extend_from_slice(block);
            }
            zlib.extend_from_slice(&adler32(&rows).to_be_bytes());

            let mut header = Vec::with_capacity(13);
            header.extend_from_slice(&(self.width as u32).to_be_bytes());
            header.extend_from_slice(&(self.height as u32).to_be_bytes());
            // 8 bits per channel, RGBA, deflate, adaptive filters, no
            // interlacing.
            header.extend_from_slice(&[8, 6, 0, 0, 0]);

            let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
            for (kind, body) in [(b"IHDR", &header), (b"IDAT", &zlib), (b"IEND", &Vec::new())] {
                png.extend_from_slice(&(body.len() as u32).to_be_bytes());
                let start = png.len();
                png.extend_from_slice(kind);
                png.extend_from_slice(body);
                let crc = crc32(&png[start..]);
                png.extend_from_slice(&crc.to_be_bytes());
            }
            png
        }
    }

    #[cfg(any(target_os = "linux", test))]
    fn adler32(bytes: &[u8]) -> u32 {
        let (mut a, mut b) = (1u32, 0u32);
        for &byte in bytes {
            a = (a + u32::from(byte)) % 65521;
            b = (b + a) % 65521;
        }
        b << 16 | a
    }

    #[cfg(any(target_os = "linux", test))]
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = !0u32;
        for &byte in bytes {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xEDB8_8320 & (crc & 1).wrapping_neg());
            }
        }
        !crc
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn pixel(icon: &Pixmap, x: i32, y: i32) -> [u8; 4] {
            let i = ((y * icon.width + x) * 4) as usize;
            icon.data[i..i + 4].try_into().unwrap()
        }

        // At 32 pixels a pixel is 16 grid units: the units are rows 3–9,
        // 13–19 and 23–29, the left link shows in rows 10–12 at columns
        // 3–9, the right one in rows 20–22 at columns 22–28.

        #[test]
        fn lamps_fade_in_towards_the_right() {
            let icon = render(32, Lamp::On, Lamp::Fault, Ink::White);
            assert_eq!(icon.data.len(), 32 * 32 * 4);
            // Solid white on the left of each unit, the lamp's colour
            // over most of it at the right.
            assert_eq!(pixel(&icon, 6, 6), [255, 0xFF, 0xFF, 0xFF]);
            let [a, r, g, b] = pixel(&icon, 28, 6);
            assert_eq!(a, 255);
            assert!(
                r < 0x30 && (0x98..0xA4).contains(&g) && b < 0x60,
                "{r} {g} {b}"
            );
            let [_, r, g, b] = pixel(&icon, 28, 16);
            assert!(r > 0xD8 && g < 0x48 && b < 0x40, "{r} {g} {b}");
            // Halfway it's a mix.
            let [_, r, _, _] = pixel(&icon, 18, 6);
            assert!((0x60..0xC0).contains(&r), "{r}");
            // The bottom unit has no lamp, the corner is transparent.
            assert_eq!(pixel(&icon, 28, 26), [255, 0xFF, 0xFF, 0xFF]);
            assert_eq!(pixel(&icon, 0, 0)[0], 0);
        }

        #[test]
        fn links_join_the_units_into_an_s() {
            let icon = render(32, Lamp::Off, Lamp::Off, Ink::White);
            assert_eq!(pixel(&icon, 5, 11), [255, 0xB9, 0xB9, 0xB9]);
            assert_eq!(pixel(&icon, 26, 11)[0], 0);
            assert_eq!(pixel(&icon, 26, 21), [255, 0xB9, 0xB9, 0xB9]);
            assert_eq!(pixel(&icon, 5, 21)[0], 0);
            // Off leaves the unit plain.
            assert_eq!(pixel(&icon, 28, 6), [255, 0xFF, 0xFF, 0xFF]);
        }

        #[test]
        fn black_ink_holds_the_lamp_solid_further_in() {
            let icon = render(32, Lamp::Off, Lamp::Busy, Ink::Black);
            assert_eq!(pixel(&icon, 28, 6), [255, 0, 0, 0]);
            assert_eq!(pixel(&icon, 20, 16), [255, 0xB0, 0x7A, 0x12]);
            assert_eq!(pixel(&icon, 5, 11), [255, 0x73, 0x73, 0x73]);
        }

        #[test]
        fn menu_lamps_are_dots_and_off_is_a_ring() {
            let on = lamp(32, Lamp::On, Ink::White);
            assert_eq!(pixel(&on, 16, 16), [255, 0x14, 0x9A, 0x50]);
            assert_eq!(pixel(&on, 16, 8), [255, 0x14, 0x9A, 0x50]);
            assert_eq!(pixel(&on, 0, 0)[0], 0);
            assert_eq!(pixel(&on, 16, 3)[0], 0);
            let off = lamp(32, Lamp::Off, Ink::Black);
            assert_eq!(pixel(&off, 16, 16)[0], 0);
            assert_eq!(pixel(&off, 16, 8), [255, 0x73, 0x73, 0x73]);
        }

        #[test]
        fn png_wraps_the_pixels_in_checked_chunks() {
            // The check values of the two checksums' specifications.
            assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
            assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);

            let png = lamp(32, Lamp::Fault, Ink::White).png();
            assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
            // IHDR: 13 bytes, 32×32.
            assert_eq!(&png[8..16], b"\0\0\0\x0dIHDR");
            assert_eq!(&png[16..24], [0, 0, 0, 32, 0, 0, 0, 32]);
            // IDAT: the zlib header, one final stored block of 32 rows of
            // a filter byte and 32 pixels, and the Adler-32.
            let rows = 32 * (1 + 32 * 4);
            assert_eq!(&png[33..41], [0, 0, 0x10, 0x2B, b'I', b'D', b'A', b'T']);
            assert_eq!(2 + 5 + rows + 4, 0x102B);
            assert_eq!(&png[41..48], [0x78, 0x01, 1, 0x20, 0x10, 0xDF, 0xEF]);
            // The centre pixel, as RGBA.
            let pixel = 48 + 16 * (1 + 32 * 4) + 1 + 16 * 4;
            assert_eq!(&png[pixel..pixel + 4], [0xD9, 0x33, 0x27, 255]);
            assert_eq!(&png[png.len() - 12..png.len() - 4], b"\0\0\0\0IEND");
        }

        #[test]
        fn template_links_are_translucent() {
            let icon = render(32, Lamp::On, Lamp::Off, Ink::Template);
            assert_eq!(pixel(&icon, 5, 11), [140, 0, 0, 0]);
            assert_eq!(pixel(&icon, 28, 6), [255, 0, 0, 0]);
        }
    }
}

/// No tray elsewhere.
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub struct TrayHandle;

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
impl TrayHandle {
    pub fn set(&self, _: TrayState) {}
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub fn spawn(_: TrayState, _: async_channel::Sender<TrayCommand>) -> anyhow::Result<TrayHandle> {
    anyhow::bail!("the tray icon is only available on Linux, macOS and Windows")
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

    #[test]
    fn tooltips_fit_their_buffer() {
        let tip: [u16; 8] = utf16_fit("SLOPP\nApache");
        assert_eq!(String::from_utf16_lossy(&tip[..7]), "SLOPP\nA");
        assert_eq!(tip[7], 0);
        // A character that takes two units isn't split.
        let tip: [u16; 4] = utf16_fit("ab😀");
        assert_eq!(tip, [u16::from(b'a'), u16::from(b'b'), 0, 0]);
    }

    #[test]
    fn windows_bitmaps_are_bgra() {
        let pixmap = icon::Pixmap {
            width: 2,
            height: 1,
            data: vec![255, 10, 20, 30, 128, 200, 100, 50],
        };
        assert_eq!(pixmap.bgra(false), [30, 20, 10, 255, 50, 100, 200, 128]);
        assert_eq!(pixmap.bgra(true), [30, 20, 10, 255, 25, 50, 100, 128]);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn plasma_styles_named_dark_or_light() {
        assert_eq!(plasma_style_is_light("breeze-dark"), Some(false));
        assert_eq!(plasma_style_is_light("breeze-light"), Some(true));
        assert_eq!(plasma_style_is_light("default"), None);
    }

    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    #[test]
    fn menu_follows_what_runs() {
        let entry = |label: &str, icon, enabled, command| MenuEntry::Item {
            label: label.to_string(),
            icon,
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
                entry("Start all", MenuIcon::Start, true, TrayCommand::StartAll),
                entry("Stop all", MenuIcon::Stop, true, TrayCommand::StopAll),
                MenuEntry::Separator,
                entry(
                    "Stop Apache",
                    MenuIcon::Lamp(Lamp::On),
                    true,
                    TrayCommand::Toggle(Module::Apache)
                ),
                entry(
                    "Start MySQL",
                    MenuIcon::Lamp(Lamp::Off),
                    true,
                    TrayCommand::Toggle(Module::MySql)
                ),
                MenuEntry::Separator,
                entry("Open site", MenuIcon::Site, true, TrayCommand::OpenSite),
                entry(
                    "Open Adminer",
                    MenuIcon::Database,
                    false,
                    TrayCommand::OpenDbAdmin(DbAdmin::Adminer)
                ),
                MenuEntry::Separator,
                entry("Show panel", MenuIcon::Panel, true, TrayCommand::ShowPanel),
                entry("Quit SLOPP", MenuIcon::Quit, true, TrayCommand::Quit),
            ]
        );
    }
}
