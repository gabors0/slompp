//! The notification area icon (`Shell_NotifyIconW`). Its hidden window lives
//! on a thread of its own with its own message loop, like the Linux tray's
//! D-Bus service, so it needs nothing from GPUI's loop and a menu held open
//! doesn't hold up the panel. The controller hands it states through
//! [`TrayHandle::set`]; menu clicks go back as [`TrayCommand`]s.
//!
//! A left click shows the panel, a right click opens the menu.

use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use anyhow::{Context as _, anyhow};
use windows_sys::Win32::Foundation::{ERROR_SUCCESS, HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS,
    DeleteObject, HBITMAP,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION,
    NIN_SELECT, NINF_KEY, NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon,
    DestroyMenu, DestroyWindow, DispatchMessageW, GetMessageW, GetSystemMetrics, HICON, ICONINFO,
    MENUITEMINFOW, MF_ENABLED, MF_GRAYED, MF_SEPARATOR, MF_STRING, MIIM_BITMAP, MSG, PostMessageW,
    PostQuitMessage, RegisterClassExW, RegisterWindowMessageW, SM_CXSMICON, SetForegroundWindow,
    SetMenuItemInfoW, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenuEx,
    TranslateMessage, WM_APP, WM_CLOSE, WM_CONTEXTMENU, WM_DESTROY, WM_NULL, WM_SETTINGCHANGE,
    WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_OVERLAPPED,
};

use super::{MenuEntry, MenuIcon, TrayCommand, TrayState, icon};

/// Enter or space on the focused icon (shellapi.h's `NIN_KEYSELECT`).
const NIN_KEYSELECT: u32 = NIN_SELECT | NINF_KEY;
/// The icon's id among this window's notification icons.
const ICON_ID: u32 = 1;
/// What the shell sends about clicks on the icon.
const WM_TRAY: u32 = WM_APP + 1;
/// A new state from [`TrayHandle::set`].
const WM_STATE: u32 = WM_APP + 2;

/// What the controller and the tray thread share.
struct Shared {
    state: Mutex<TrayState>,
    commands: async_channel::Sender<TrayCommand>,
}

/// The running icon. Dropping it removes the icon.
pub struct TrayHandle {
    /// The tray thread's window (an `HWND`, which isn't `Send`).
    hwnd: usize,
    shared: Arc<Shared>,
    /// Closed when the tray thread is done.
    finished: mpsc::Receiver<()>,
}

impl TrayHandle {
    pub fn set(&self, state: TrayState) {
        *self.shared.state.lock().unwrap() = state;
        // SAFETY: posting to a window; if it is gone this fails harmlessly.
        unsafe { PostMessageW(self.hwnd as HWND, WM_STATE, 0, 0) };
    }
}

impl Drop for TrayHandle {
    /// Takes the icon off the taskbar before the app exits, so no ghost is
    /// left until the mouse passes over it.
    fn drop(&mut self) {
        // SAFETY: as in `set`.
        unsafe { PostMessageW(self.hwnd as HWND, WM_CLOSE, 0, 0) };
        let _ = self.finished.recv_timeout(Duration::from_secs(1));
    }
}

/// Puts the icon in the notification area. Fails when there is no taskbar
/// to put it on.
pub fn spawn(
    state: TrayState,
    commands: async_channel::Sender<TrayCommand>,
) -> anyhow::Result<TrayHandle> {
    let shared = Arc::new(Shared {
        state: Mutex::new(state),
        commands,
    });
    let (ready, started) = mpsc::channel();
    let (finish, finished) = mpsc::channel();
    let thread_shared = shared.clone();
    std::thread::Builder::new()
        .name("tray".into())
        .spawn(move || {
            run(thread_shared, ready);
            drop(finish);
        })
        .context("starting the tray thread")?;
    let hwnd = started
        .recv()
        .context("the tray thread stopped")?
        .map_err(|err: String| anyhow!(err))?;
    Ok(TrayHandle {
        hwnd,
        shared,
        finished,
    })
}

thread_local! {
    /// The tray thread's share, for the window procedure.
    static SHARED: RefCell<Option<Arc<Shared>>> = const { RefCell::new(None) };
    /// The icon on the taskbar now, destroyed when replaced.
    static ICON: Cell<HICON> = const { Cell::new(std::ptr::null_mut()) };
}

/// Sent to top-level windows when Explorer (re)starts the taskbar, which
/// forgets every icon.
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

/// The tray thread: makes the window and the icon, reports the window (or
/// what went wrong) on `ready`, then runs the window's messages until it
/// closes.
fn run(shared: Arc<Shared>, ready: mpsc::Sender<Result<usize, String>>) {
    SHARED.with(|s| *s.borrow_mut() = Some(shared));
    let fail = |what: &str| {
        let _ = ready.send(Err(format!(
            "{what} (error {})",
            std::io::Error::last_os_error()
        )));
    };

    let class = wide("SloppTray");
    // SAFETY: plain Win32 calls with valid, NUL-terminated strings; the
    // class and window are this thread's.
    unsafe {
        TASKBAR_CREATED.store(
            RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()),
            Ordering::Relaxed,
        );
        let instance = GetModuleHandleW(std::ptr::null());
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            ..Default::default()
        };
        if RegisterClassExW(&wc) == 0 {
            return fail("couldn't register the tray's window class");
        }
        // A hidden top-level window rather than a message-only one: only
        // top-level windows hear TaskbarCreated, and the menu needs a
        // window that can take the foreground.
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class.as_ptr(),
            wide("SLOPP").as_ptr(),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            return fail("couldn't create the tray's window");
        }
        // At login the taskbar may not be up yet.
        let mut added = add_icon(hwnd);
        for _ in 0..20 {
            if added {
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
            added = add_icon(hwnd);
        }
        if !added {
            DestroyWindow(hwnd);
            return fail("no taskbar to put an icon on");
        }
        let _ = ready.send(Ok(hwnd as usize));

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    let old = ICON.with(|i| i.replace(std::ptr::null_mut()));
    if !old.is_null() {
        // SAFETY: an icon we made and no longer show.
        unsafe { DestroyIcon(old) };
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_TRAY => {
            // NOTIFYICON_VERSION_4: the event in the low word of lparam, the
            // anchor point in wparam.
            match (lparam & 0xFFFF) as u32 {
                NIN_SELECT | NIN_KEYSELECT => send(TrayCommand::ShowPanel),
                WM_CONTEXTMENU => {
                    let x = (wparam & 0xFFFF) as u16 as i16 as i32;
                    let y = ((wparam >> 16) & 0xFFFF) as u16 as i16 as i32;
                    show_menu(hwnd, x, y);
                }
                _ => {}
            }
            0
        }
        WM_STATE => {
            update_icon(hwnd, NIM_MODIFY);
            0
        }
        WM_SETTINGCHANGE => {
            // The taskbar may have turned light or dark.
            update_icon(hwnd, NIM_MODIFY);
            // SAFETY: passing the message on unchanged.
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_CLOSE => {
            let data = icon_data(hwnd);
            // SAFETY: removing our own icon and destroying our own window.
            unsafe {
                Shell_NotifyIconW(NIM_DELETE, &data);
                DestroyWindow(hwnd);
            }
            0
        }
        WM_DESTROY => {
            // SAFETY: ends this thread's message loop.
            unsafe { PostQuitMessage(0) };
            0
        }
        _ if msg != 0 && msg == TASKBAR_CREATED.load(Ordering::Relaxed) => {
            add_icon(hwnd);
            0
        }
        // SAFETY: the default handling for everything else.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn shared() -> Option<Arc<Shared>> {
    SHARED.with(|s| s.borrow().clone())
}

fn send(command: TrayCommand) {
    if let Some(shared) = shared() {
        let _ = shared.commands.try_send(command);
    }
}

fn state() -> Option<TrayState> {
    Some(shared()?.state.lock().unwrap().clone())
}

/// The icon's identity, for every `Shell_NotifyIconW` call.
fn icon_data(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: ICON_ID,
        ..Default::default()
    }
}

fn add_icon(hwnd: HWND) -> bool {
    if !update_icon(hwnd, NIM_ADD) {
        return false;
    }
    let mut data = icon_data(hwnd);
    data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
    // SAFETY: a filled-in NOTIFYICONDATAW for our icon.
    unsafe { Shell_NotifyIconW(NIM_SETVERSION, &data) != 0 }
}

/// Adds or modifies the icon to show the current state: the mark with the
/// modules' lamps, in the taskbar's ink, and a tooltip of what runs where.
fn update_icon(hwnd: HWND, action: u32) -> bool {
    let Some(state) = state() else {
        return false;
    };
    let ink = icon::Ink::for_panel(super::light_panel(state.light_panel));
    // SAFETY: no preconditions.
    let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16);
    let pixmap = icon::render(size, state.lamps(), ink);
    let hicon = make_icon(&pixmap);

    let mut data = icon_data(hwnd);
    data.uFlags = NIF_ICON | NIF_TIP | NIF_MESSAGE | NIF_SHOWTIP;
    data.uCallbackMessage = WM_TRAY;
    data.hIcon = hicon;
    data.szTip = super::utf16_fit(&format!("SLOPP\n{}", state.summary()));
    // SAFETY: a filled-in NOTIFYICONDATAW for our icon.
    let ok = unsafe { Shell_NotifyIconW(action, &data) != 0 };
    // The shell keeps its own copy: the old icon can go, or the new one if
    // it wasn't taken.
    let unused = if ok {
        ICON.with(|i| i.replace(hicon))
    } else {
        hicon
    };
    if !unused.is_null() {
        // SAFETY: an icon we made and the taskbar doesn't use.
        unsafe { DestroyIcon(unused) };
    }
    ok
}

/// The menu, at `(x, y)`, until something is chosen or it is dismissed.
fn show_menu(hwnd: HWND, x: i32, y: i32) {
    let Some(state) = state() else {
        return;
    };
    // SAFETY: no preconditions.
    let lamp_size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16);
    let mut commands = Vec::new();
    let mut bitmaps = Vec::new();
    // SAFETY: a menu we make, fill, show and destroy here; strings and
    // bitmaps live until it's destroyed.
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return;
        }
        for entry in state.menu() {
            let MenuEntry::Item {
                label,
                icon,
                enabled,
                command,
            } = entry
            else {
                AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
                continue;
            };
            commands.push(command);
            let id = commands.len();
            let flags = MF_STRING | if enabled { MF_ENABLED } else { MF_GRAYED };
            AppendMenuW(menu, flags, id, wide(&label).as_ptr());
            // Classic menus are light whatever the theme, so the lamps get
            // the grey ink. Other items have no picture on Windows.
            if let MenuIcon::Lamp(lamp) = icon {
                let bitmap = make_bitmap(&icon::lamp(lamp_size, lamp, icon::Ink::Grey), true);
                if !bitmap.is_null() {
                    let info = MENUITEMINFOW {
                        cbSize: size_of::<MENUITEMINFOW>() as u32,
                        fMask: MIIM_BITMAP,
                        hbmpItem: bitmap,
                        ..Default::default()
                    };
                    SetMenuItemInfoW(menu, id as u32, 0, &info);
                    bitmaps.push(bitmap);
                }
            }
        }
        // Without the foreground the menu wouldn't close on a click
        // elsewhere; the WM_NULL afterwards is the documented companion.
        SetForegroundWindow(hwnd);
        let chosen = TrackPopupMenuEx(
            menu,
            TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
            x,
            y,
            hwnd,
            std::ptr::null(),
        );
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(menu);
        for bitmap in bitmaps {
            DeleteObject(bitmap);
        }
        if let Some(&command) = usize::try_from(chosen)
            .ok()
            .and_then(|id| id.checked_sub(1))
            .and_then(|i| commands.get(i))
        {
            send(command);
        }
    }
}

/// An icon from a pixmap: its pixels with their alpha, and an empty mask,
/// which alpha makes moot.
fn make_icon(pixmap: &icon::Pixmap) -> HICON {
    let color = make_bitmap(pixmap, false);
    if color.is_null() {
        return std::ptr::null_mut();
    }
    let (width, height) = (pixmap.width, pixmap.height);
    // A 1-bit mask, rows padded to 16 bits.
    let mask_bits = vec![0u8; (width as usize).div_ceil(16) * 2 * height as usize];
    // SAFETY: `mask_bits` holds the 1-bit rows CreateBitmap reads; both
    // bitmaps are deleted once the icon has copied them.
    unsafe {
        let mask = CreateBitmap(width, height, 1, 1, mask_bits.as_ptr().cast());
        let info = ICONINFO {
            fIcon: 1,
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color,
        };
        let icon = CreateIconIndirect(&info);
        DeleteObject(color);
        DeleteObject(mask);
        icon
    }
}

/// A 32-bit top-down DIB of a pixmap. Icons take straight alpha; menu item
/// bitmaps want it premultiplied.
fn make_bitmap(pixmap: &icon::Pixmap, premultiplied: bool) -> HBITMAP {
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: pixmap.width,
            // Negative: the first row is the top one.
            biHeight: -pixmap.height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits = std::ptr::null_mut();
    // SAFETY: a DIB section of the pixmap's size, whose buffer we fill with
    // exactly that many pixels.
    unsafe {
        let bitmap = CreateDIBSection(
            std::ptr::null_mut(),
            &info,
            DIB_RGB_COLORS,
            &mut bits,
            std::ptr::null_mut(),
            0,
        );
        if bitmap.is_null() || bits.is_null() {
            return std::ptr::null_mut();
        }
        let bgra = pixmap.bgra(premultiplied);
        std::ptr::copy_nonoverlapping(bgra.as_ptr(), bits.cast::<u8>(), bgra.len());
        bitmap
    }
}

/// Whether the taskbar is light: Windows sets it apart from the apps'
/// colours (Settings → Personalisation → Colours, "Windows mode"). `None`
/// before Windows 10 1903, which had no light taskbar.
pub(super) fn taskbar_light() -> Option<bool> {
    let mut value = 0u32;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: a DWORD-sized buffer for a DWORD value.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize").as_ptr(),
            wide("SystemUsesLightTheme").as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&raw mut value).cast(),
            &mut size,
        )
    };
    (status == ERROR_SUCCESS).then_some(value != 0)
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
