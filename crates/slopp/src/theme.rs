//! SLOPP's look: a server rack in Ayu's colours (Ayu Dark and Ayu Light, as
//! published in the `ayu` package, 9.1). Module faces are Ayu's panels, the
//! console and inputs its sunk surface, and status lamps the one source of
//! colour. Ayu's orange stands in for XAMPP's and marks the brand and the
//! main action, nothing else.

use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{App, Hsla, Rgba, px, rgb};

pub const FONT: &str = "IBM Plex Sans";
pub const MONO: &str = "IBM Plex Mono";

static LIGHT: AtomicBool = AtomicBool::new(false);

pub fn is_light() -> bool {
    LIGHT.load(Ordering::Relaxed)
}

fn pick<T>(dark: T, light: T) -> T {
    if is_light() { light } else { dark }
}

/// What a module's lamp shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lamp {
    Off,
    /// Starting or stopping.
    Busy,
    On,
    Fault,
}

/// The colours of SLOPP's mark, the S of three rack units: white units for
/// dark backgrounds, dark grey ones (Ayu Light's text colour) for light.
/// The title bar and the tray icon
/// draw it the same way.
pub struct MarkInk {
    pub unit: u32,
    /// The links between the units.
    pub link: u32,
    pub on: u32,
    pub busy: u32,
    pub fault: u32,
    /// Where a lit lamp's gradient becomes opaque, as a fraction of the way
    /// from its transparent end: grey units keep it solid for longer, so
    /// the colour holds up against the grey.
    pub lamp_solid: f32,
}

impl MarkInk {
    pub const WHITE: MarkInk = MarkInk {
        unit: 0xFFFFFF,
        link: 0xB9B9B9,
        on: 0x149A50,
        busy: 0xF4A000,
        fault: 0xD93327,
        lamp_solid: 1.0,
    };
    pub const GREY: MarkInk = MarkInk {
        unit: 0x5C6166,
        link: 0xA4A7AA,
        on: 0x1F8F58,
        busy: 0xEFAC01,
        fault: 0xC5372B,
        lamp_solid: 0.4,
    };

    /// A lit lamp's colour.
    pub fn lamp(&self, lamp: Lamp) -> Option<u32> {
        match lamp {
            Lamp::Off => None,
            Lamp::Busy => Some(self.busy),
            Lamp::On => Some(self.on),
            Lamp::Fault => Some(self.fault),
        }
    }
}

/// Ayu's colours. Where Ayu has no token for a role, the value is a mix of
/// two of its tokens, as noted.
pub struct Palette;

impl Palette {
    /// The chassis: window background (`ui.bg`).
    pub fn bg() -> Rgba {
        pick(rgb(0x0D1017), rgb(0xF8F9FA))
    }
    /// A module's face plate and other raised surfaces (`ui.panel.bg` in
    /// dark, `ui.popup.bg` in light, where the panel is all but the
    /// background's colour).
    pub fn face() -> Rgba {
        pick(rgb(0x141821), rgb(0xFFFFFF))
    }
    /// Recessed areas: the console, inputs (`surface.sunk`).
    pub fn well() -> Rgba {
        pick(rgb(0x010102), rgb(0xEBEEF0))
    }
    /// `ui.line`; light's is translucent, here over `ui.bg`.
    pub fn border() -> Rgba {
        pick(rgb(0x1B1F29), rgb(0xE7EAED))
    }
    /// Controls that are off, such as a switch's track (`palette.gray`).
    pub fn control() -> Rgba {
        pick(rgb(0x404953), rgb(0xC5C5C8))
    }
    /// `editor.fg`.
    pub fn text() -> Rgba {
        pick(rgb(0xBFBDB6), rgb(0x5C6166))
    }
    /// Halfway between `editor.fg` and `ui.fg`.
    pub fn text_secondary() -> Rgba {
        pick(rgb(0x8C9097), rgb(0x6F7882))
    }
    /// `ui.fg`.
    pub fn text_tertiary() -> Rgba {
        pick(rgb(0x5A6378), rgb(0x828E9F))
    }
    /// Ayu's orange (`syntax.keyword`), in place of XAMPP's.
    pub fn accent() -> Rgba {
        pick(rgb(0xFF8F40), rgb(0xFA8532))
    }
    /// Text on the accent: dark in both schemes, as white on Ayu's orange
    /// is too faint to read.
    pub fn on_accent() -> Rgba {
        rgb(0x0D1017)
    }
    /// The mark's colours for the window's appearance.
    pub fn mark() -> &'static MarkInk {
        pick(&MarkInk::WHITE, &MarkInk::GREY)
    }
    /// `syntax.string`; light uses green's darkest step, to stay readable
    /// as text.
    pub fn ok() -> Rgba {
        pick(rgb(0xAAD94C), rgb(0x719700))
    }
    /// `syntax.func`; light uses yellow's darkest step.
    pub fn busy() -> Rgba {
        pick(rgb(0xFFB454), rgb(0xB37C00))
    }
    /// `common.error`.
    pub fn fault() -> Rgba {
        pick(rgb(0xD95757), rgb(0xE65050))
    }

    pub fn lamp(lamp: Lamp) -> Hsla {
        match lamp {
            Lamp::Off => Palette::text_tertiary().into(),
            Lamp::Busy => Palette::busy().into(),
            Lamp::On => Palette::ok().into(),
            Lamp::Fault => Palette::fault().into(),
        }
    }

    /// A wash of the text colour, for hover fills.
    pub fn wash(alpha: f32) -> Hsla {
        Hsla::from(Palette::text()).opacity(alpha)
    }
}

const FONTS: &[&[u8]] = &[
    include_bytes!("../assets/fonts/IBMPlexSans-Regular.ttf"),
    include_bytes!("../assets/fonts/IBMPlexSans-Italic.ttf"),
    include_bytes!("../assets/fonts/IBMPlexSans-Medium.ttf"),
    include_bytes!("../assets/fonts/IBMPlexSans-SemiBold.ttf"),
    include_bytes!("../assets/fonts/IBMPlexSans-Bold.ttf"),
    include_bytes!("../assets/fonts/IBMPlexMono-Regular.ttf"),
    include_bytes!("../assets/fonts/IBMPlexMono-Medium.ttf"),
];

pub fn init(cx: &mut App) {
    let fonts = FONTS.iter().map(|bytes| Cow::Borrowed(*bytes)).collect();
    if let Err(err) = cx.text_system().add_fonts(fonts) {
        tracing::warn!("failed to register bundled fonts: {err:#}");
    }
}

/// Switches between the dark and light palettes and restyles gpui-component.
pub fn apply(light: bool, cx: &mut App) {
    LIGHT.store(light, Ordering::Relaxed);
    Theme::change(
        if light {
            ThemeMode::Light
        } else {
            ThemeMode::Dark
        },
        None,
        cx,
    );
    Theme::update(cx, |theme| {
        theme.font_family = FONT.into();
        theme.mono_font_family = MONO.into();
        theme.font_size = px(14.);
        theme.radius = px(6.);
        theme.radius_lg = px(8.);
        theme.shadow = false;

        let accent: Hsla = Palette::accent().into();
        let c = &mut theme.colors;
        c.background = Palette::bg().into();
        c.foreground = Palette::text().into();
        c.muted = Palette::well().into();
        c.muted_foreground = Palette::text_secondary().into();
        c.border = Palette::border().into();
        c.input = Palette::border().into();
        c.ring = accent.opacity(0.7);
        c.caret = accent;
        c.selection = accent.opacity(0.3);
        c.primary = accent;
        c.primary_hover = accent.blend(Hsla::white().opacity(0.1));
        c.primary_active = accent.blend(Hsla::black().opacity(0.12));
        c.primary_foreground = Palette::on_accent().into();
        c.button_primary = c.primary;
        c.button_primary_hover = c.primary_hover;
        c.button_primary_active = c.primary_active;
        c.button_primary_foreground = c.primary_foreground;
        c.secondary = Palette::face().into();
        c.secondary_hover = Palette::wash(0.08).blend(Palette::face().into());
        c.secondary_active = Palette::wash(0.12).blend(Palette::face().into());
        c.secondary_foreground = Palette::text().into();
        c.button_secondary = c.secondary;
        c.button_secondary_hover = c.secondary_hover;
        c.button_secondary_active = c.secondary_active;
        c.button_secondary_foreground = c.secondary_foreground;
        c.popover = Palette::face().into();
        c.popover_foreground = Palette::text().into();
        c.list_hover = Palette::wash(0.06);
        c.list_active = accent.opacity(0.16);
        c.list_active_border = accent.opacity(0.5);
        c.accent = Palette::wash(0.07);
        c.accent_foreground = Palette::text().into();
        c.title_bar = Palette::bg().into();
        c.title_bar_border = Palette::border().into();
        c.switch = Palette::control().into();
        c.success = Palette::ok().into();
        // The window's close button: hovered it is filled with `danger` and
        // its cross takes `danger_foreground`, which gpui-component's dark
        // theme makes red too.
        let danger: Hsla = Palette::fault().into();
        c.danger = danger;
        c.danger_hover = danger.blend(Hsla::white().opacity(0.1));
        c.danger_active = danger.blend(Hsla::black().opacity(0.12));
        c.danger_foreground = Hsla::white();
        c.warning = Palette::busy().into();
        c.scrollbar = Hsla::transparent_black();
        c.scrollbar_thumb = Palette::wash(0.18);
        c.scrollbar_thumb_hover = Palette::wash(0.3);
        c.window_border = Palette::border().into();
    });
}
