//! The menu bar item (`NSStatusItem`). AppKit belongs to the main thread,
//! where GPUI runs the app, so this needs no thread of its own: the item is
//! made and updated from the controller, and AppKit calls [`MenuTarget`]
//! when a menu item is chosen.

use std::cell::RefCell;

use anyhow::Context as _;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AnyThread as _, DefinedClass, MainThreadMarker, MainThreadOnly, define_class};
use objc2::{msg_send, sel};
use objc2_app_kit::{
    NSBitmapImageRep, NSDeviceRGBColorSpace, NSImage, NSMenu, NSMenuItem, NSSquareStatusItemLength,
    NSStatusBar, NSStatusItem,
};
use objc2_foundation::{NSObject, NSObjectProtocol, NSSize, NSString};
use slompp_core::Module;

use super::{MenuEntry, TrayCommand, TrayState, icon};
use crate::theme::Lamp;

/// The menu bar is 22 points high; icons in it are 18.
const ICON_POINTS: f64 = 18.0;
/// Rendered at twice that for Retina displays.
const ICON_PIXELS: i32 = 36;

struct Ivars {
    commands: async_channel::Sender<TrayCommand>,
    /// What each menu item does, by the item's tag.
    actions: RefCell<Vec<TrayCommand>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and MenuTarget
    // doesn't implement Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "SlomppMenuTarget"]
    #[ivars = Ivars]
    struct MenuTarget;

    impl MenuTarget {
        #[unsafe(method(choose:))]
        fn choose(&self, item: &NSMenuItem) {
            let ivars = self.ivars();
            let command = usize::try_from(item.tag())
                .ok()
                .and_then(|tag| ivars.actions.borrow().get(tag).copied());
            if let Some(command) = command {
                let _ = ivars.commands.try_send(command);
            }
        }
    }

    unsafe impl NSObjectProtocol for MenuTarget {}
);

impl MenuTarget {
    fn new(mtm: MainThreadMarker, commands: async_channel::Sender<TrayCommand>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars {
            commands,
            actions: RefCell::new(Vec::new()),
        });
        // SAFETY: NSObject's init takes no arguments and returns the object.
        unsafe { msg_send![super(this), init] }
    }
}

/// The running menu bar item.
pub struct TrayHandle {
    item: Retained<NSStatusItem>,
    menu: Retained<NSMenu>,
    target: Retained<MenuTarget>,
}

impl TrayHandle {
    pub fn set(&self, state: TrayState) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        if let Some(button) = self.item.button(mtm) {
            let image = image(&state);
            button.setImage(image.as_deref());
            // Without an image the item would be invisible.
            let title = if image.is_some() { "" } else { "SLOMPP" };
            button.setTitle(&NSString::from_str(title));
            button.setToolTip(Some(&NSString::from_str(&state.summary())));
        }

        let mut actions = Vec::new();
        self.menu.removeAllItems();
        for entry in state.menu() {
            let MenuEntry::Item {
                label,
                enabled,
                command,
            } = entry
            else {
                self.menu.addItem(&NSMenuItem::separatorItem(mtm));
                continue;
            };
            // SAFETY: `choose:` is the method MenuTarget defines, taking the
            // menu item that sent it.
            let item = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    &NSString::from_str(&label),
                    Some(sel!(choose:)),
                    &NSString::new(),
                )
            };
            let target: &AnyObject = &self.target;
            // SAFETY: the target outlives the menu: both live in this handle.
            unsafe { item.setTarget(Some(target)) };
            item.setTag(actions.len() as isize);
            item.setEnabled(enabled);
            self.menu.addItem(&item);
            actions.push(command);
        }
        *self.target.ivars().actions.borrow_mut() = actions;
    }
}

impl Drop for TrayHandle {
    fn drop(&mut self) {
        NSStatusBar::systemStatusBar().removeStatusItem(&self.item);
    }
}

/// Puts the item in the menu bar. Clicking it opens the menu, as menu bar
/// items do; "Show panel" brings the window back.
pub fn spawn(
    state: TrayState,
    commands: async_channel::Sender<TrayCommand>,
) -> anyhow::Result<TrayHandle> {
    let mtm =
        MainThreadMarker::new().context("the menu bar item must be made on the main thread")?;
    let item = NSStatusBar::systemStatusBar().statusItemWithLength(NSSquareStatusItemLength);
    let menu = NSMenu::new(mtm);
    // We say which items are enabled; AppKit would enable all with a target.
    menu.setAutoenablesItems(false);
    item.setMenu(Some(&menu));
    let handle = TrayHandle {
        item,
        menu,
        target: MenuTarget::new(mtm, commands),
    };
    handle.set(state);
    Ok(handle)
}

/// The rack icon with the modules' lamps. While nothing is lit it's a
/// template image, which AppKit draws black or white to suit the menu bar;
/// lit lamps have colour, so then it gets the ink for the system's
/// appearance.
fn image(state: &TrayState) -> Option<Retained<NSImage>> {
    let (apache, mysql) = (state.lamp(Module::Apache), state.lamp(Module::MySql));
    let template = apache == Lamp::Off && mysql == Lamp::Off;
    let ink = if template {
        icon::Ink::Black
    } else {
        icon::Ink::for_panel(state.light_panel)
    };
    let pixmap = icon::render(ICON_PIXELS, apache, mysql, ink);
    let (width, height) = (pixmap.width as isize, pixmap.height as isize);
    // SAFETY: null planes make the representation allocate its own buffer.
    let rep = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            std::ptr::null_mut(),
            width,
            height,
            8,
            4,
            true,
            false,
            NSDeviceRGBColorSpace,
            width * 4,
            32,
        )
    }?;
    let buffer = rep.bitmapData();
    if buffer.is_null() {
        return None;
    }
    // SAFETY: the buffer holds `height` rows of `width * 4` bytes, as asked.
    let pixels = unsafe { std::slice::from_raw_parts_mut(buffer, pixmap.data.len()) };
    // ARGB to the representation's premultiplied RGBA.
    let (rgba, argb) = (
        pixels.as_chunks_mut::<4>().0,
        pixmap.data.as_chunks::<4>().0,
    );
    for (dst, &[a, r, g, b]) in rgba.iter_mut().zip(argb) {
        let premultiply = |c: u8| (u16::from(c) * u16::from(a) / 255) as u8;
        *dst = [premultiply(r), premultiply(g), premultiply(b), a];
    }
    let size = NSSize::new(ICON_POINTS, ICON_POINTS);
    rep.setSize(size);
    let image = NSImage::initWithSize(NSImage::alloc(), size);
    image.addRepresentation(&rep);
    image.setTemplate(template);
    Some(image)
}
