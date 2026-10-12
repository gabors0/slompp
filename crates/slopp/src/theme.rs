//! SLOPP's look: a server rack. Steel-blue chassis, module faces a step
//! lighter, and status lamps as the one source of colour. XAMPP orange marks
//! the brand and the main action, nothing else.

use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{App, Hsla, Rgba, hsla, px, rgb};

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

pub struct Palette;

impl Palette {
    /// The chassis: window background.
    pub fn bg() -> Rgba {
        pick(rgb(0x181C25), rgb(0xE8EBF0))
    }
    /// A module's face plate and other raised surfaces.
    pub fn face() -> Rgba {
        pick(rgb(0x20252F), rgb(0xFFFFFF))
    }
    /// Recessed areas: the console, inputs.
    pub fn well() -> Rgba {
        pick(rgb(0x12151C), rgb(0xF4F6F9))
    }
    pub fn border() -> Rgba {
        pick(rgb(0x2D3341), rgb(0xD2D7E0))
    }
    pub fn text() -> Rgba {
        pick(rgb(0xE6E9EF), rgb(0x1A1F2A))
    }
    pub fn text_secondary() -> Rgba {
        pick(rgb(0x9BA3B3), rgb(0x4E5767))
    }
    pub fn text_tertiary() -> Rgba {
        pick(rgb(0x687084), rgb(0x868E9E))
    }
    /// XAMPP orange.
    pub fn accent() -> Rgba {
        pick(rgb(0xF07B2C), rgb(0xD45F17))
    }
    pub fn ok() -> Rgba {
        pick(rgb(0x4CC38A), rgb(0x1F8F58))
    }
    pub fn busy() -> Rgba {
        pick(rgb(0xE9B44C), rgb(0xB07A12))
    }
    pub fn fault() -> Rgba {
        pick(rgb(0xEF6458), rgb(0xC5372B))
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
        c.primary_foreground = hsla(0., 0., 1., 1.);
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
        c.switch = Palette::border().into();
        c.success = Palette::ok().into();
        c.danger = Palette::fault().into();
        c.warning = Palette::busy().into();
        c.scrollbar = Hsla::transparent_black();
        c.scrollbar_thumb = Palette::wash(0.18);
        c.scrollbar_thumb_hover = Palette::wash(0.3);
        c.window_border = Palette::border().into();
    });
}
