//! Windows tray icon (notification area, near the clock).
//!
//! Closing the window only hides it: the app keeps running — the `Ctrl+Alt+A`
//! shortcut and browser autofill keep working. From the icon:
//! a click opens the window; right-click opens the app's menu
//! ([`crate::tray_menu`]), with the theme and the accent color: **quick access**
//! (copy username, password or code of pinned items, without opening the window),
//! **Open**, **Generate and copy password**, **Sync now**,
//! **Lock**/**Unlock…** and **Quit**.
//!
//! The icon lives on its own thread with an invisible Win32 window (like the
//! global shortcut), and the commands reach the app through a channel.
//! Opening the executable again with the app already running just shows the existing window.

use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::mpsc::UnboundedSender;

use crate::vault_view::CopyShortcut;

/// What the tray asks of the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Show,
    Lock,
    Quit,
    Sync,
    GeneratePassword,
    Copy {
        item_id: String,
        what: CopyShortcut,
    },
    /// Quick search: type this item into the app that had focus.
    Fill {
        item_id: String,
    },
    /// Quick search: show this item in Scout Vault.
    OpenItem {
        item_id: String,
    },
    /// The person confirmed typing into an app that doesn't tell its fields.
    TypeAnyway,
    /// Locked HUD (quick search / auto-type): unlock with this vault password.
    Unlock(HudPassword),
    /// Right-click on the icon: open the menu with its corner at the cursor (screen pixels).
    OpenMenu {
        x: i32,
        y: i32,
    },
}

/// Vault password typed in the HUD. Zeroed when dropped; never printed.
#[derive(Clone, PartialEq, Eq)]
pub struct HudPassword(pub zeroize::Zeroizing<String>);

impl std::fmt::Debug for HudPassword {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HudPassword(***)")
    }
}

/// An item pinned to quick access, as the menu needs it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PinnedEntry {
    pub id: String,
    pub name: String,
    pub has_username: bool,
    pub has_password: bool,
    pub has_totp: bool,
    /// The item's icon (the chosen one, the site's brand or the type's).
    pub icon: Option<crate::item_icons::ItemIcon>,
}

/// What the menu shows: depends on whether the vault is open and on quick access.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MenuState {
    pub unlocked: bool,
    pub pinned: Vec<PinnedEntry>,
}

/// Where to put the menu (top-left corner, in physical pixels) so that it
/// opens next to the cursor and fits in the work area. With the tray at the bottom it
/// goes up from the cursor; with the taskbar at the top or on the left, it goes down
/// or to the right.
pub fn place_menu(cursor: (i32, i32), size: (i32, i32), work: (i32, i32, i32, i32)) -> (i32, i32) {
    let (left, top, right, bottom) = work;
    let (w, h) = size;
    let mut x = cursor.0 - w;
    if x < left {
        x = cursor.0;
    }
    let mut y = cursor.1 - h;
    if y < top {
        y = cursor.1;
    }
    (
        x.clamp(left, (right - w).max(left)),
        y.clamp(top, (bottom - h).max(top)),
    )
}

/// Work area (without the taskbar) and scale of the primary monitor.
pub fn work_area_and_scale() -> ((i32, i32, i32, i32), f32) {
    platform::work_area_and_scale()
}

static SYSTEM: AtomicBool = AtomicBool::new(false);

/// Allows creating the icon and hiding windows (real app only; never in tests).
pub fn enable_system() {
    SYSTEM.store(true, Ordering::Relaxed);
}

pub fn system_enabled() -> bool {
    SYSTEM.load(Ordering::Relaxed)
}

/// Active icon; dropping the value removes it from the tray.
pub struct Tray {
    #[cfg_attr(not(windows), allow(dead_code))]
    hwnd: isize,
}

impl Drop for Tray {
    fn drop(&mut self) {
        platform::remove(self.hwnd);
    }
}

impl Tray {
    /// Windows balloon (one sentence), e.g. "still running in the tray".
    pub fn notify(&self, title: &str, text: &str) {
        platform::balloon(self.hwnd, title, text);
    }
}

/// Creates the icon. Each click/menu sends a [`Command`] through `tx`.
pub fn start(tx: UnboundedSender<Command>) -> Result<Tray, String> {
    platform::start(tx).map(|hwnd| Tray { hwnd })
}

/// If the app is already running (in the tray), asks it to show up and returns `true`.
pub fn wake_existing_instance() -> bool {
    platform::wake_existing()
}

/// Is the app already running (with its tray icon)?
pub fn existing_instance() -> bool {
    platform::find_existing()
}

/// Asks the running app to quit and waits (up to `timeout`) for it to leave.
/// `true` when no instance is left.
pub fn quit_existing_instance(timeout: std::time::Duration) -> bool {
    if !platform::quit_existing() {
        return true;
    }
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if !platform::find_existing() {
            // The icon's window goes first; give the process a moment to end.
            std::thread::sleep(std::time::Duration::from_millis(700));
            return true;
        }
    }
    false
}

/// Hides the app window (gone from the taskbar; the tray icon stays).
pub fn hide_window(window: &gpui_kit::Window) {
    if let Some(hwnd) = hwnd_of(window) {
        platform::show(hwnd, false);
    }
}

/// Shows it again and brings it to the front.
pub fn show_window(window: &gpui_kit::Window) {
    if let Some(hwnd) = hwnd_of(window) {
        platform::show(hwnd, true);
    }
    window.activate_window();
}

/// Is the window hidden in the tray? (outside the real app, always `false`)
pub fn is_hidden(window: &gpui_kit::Window) -> bool {
    system_enabled() && hwnd_of(window).is_some_and(|h| !platform::is_visible(h))
}

/// Is the window minimized? (outside the real app, always `false`)
pub fn is_minimized(window: &gpui_kit::Window) -> bool {
    system_enabled() && hwnd_of(window).is_some_and(platform::is_iconic)
}

/// Floating window (HUD, tray menu) the size of its card: Windows rounds the
/// corners and the app draws the border. There is no transparent margin around
/// the card — Windows shows it as a dark, blurred veil, not as see-through.
pub fn frameless(window: &gpui_kit::Window) {
    if let Some(hwnd) = hwnd_of(window) {
        platform::frameless(hwnd);
    }
}

fn hwnd_of(window: &gpui_kit::Window) -> Option<isize> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
        _ => None,
    }
}

#[cfg(windows)]
mod platform {
    use std::cell::RefCell;

    use tokio::sync::mpsc::UnboundedSender as Sender;

    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::HiDpi::GetDpiForSystem;
    use windows::Win32::UI::Shell::{
        Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD,
        NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
    };
    use windows::Win32::UI::WindowsAndMessaging::*;

    use super::Command;

    const CLASS: PCWSTR = w!("ScoutVaultTray");
    /// Icon message (clicks).
    const WM_TRAY: u32 = WM_APP + 1;
    /// Message from another instance: "show up".
    const WM_WAKE: u32 = WM_APP + 2;
    /// Message from another instance (the installer's `--quit`): "quit".
    const WM_QUIT_APP: u32 = WM_APP + 3;
    const ID: u32 = 1;

    thread_local! {
        static TX: RefCell<Option<Sender<Command>>> = const { RefCell::new(None) };
        static TASKBAR_CREATED: RefCell<u32> = const { RefCell::new(0) };
    }

    fn send(cmd: Command) {
        TX.with(|tx| {
            if let Some(tx) = tx.borrow().as_ref() {
                let _ = tx.send(cmd);
            }
        });
    }

    fn wide<const N: usize>(text: &str) -> [u16; N] {
        let mut out = [0u16; N];
        for (i, c) in text.encode_utf16().take(N - 1).enumerate() {
            out[i] = c;
        }
        out
    }

    fn data(hwnd: HWND) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: ID,
            ..Default::default()
        }
    }

    unsafe fn add_icon(hwnd: HWND) {
        let mut nid = data(hwnd);
        nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        nid.uCallbackMessage = WM_TRAY;
        // Executable icon (resource 1, the same as the taskbar).
        let module = GetModuleHandleW(None).unwrap_or_default();
        nid.hIcon = LoadIconW(Some(module.into()), PCWSTR(1 as _))
            .or_else(|_| LoadIconW(None, IDI_APPLICATION))
            .unwrap_or_default();
        nid.szTip = wide("Scout Vault");
        let _ = Shell_NotifyIconW(NIM_ADD, &nid);
    }

    /// Asks the app to open the menu at the cursor. Bringing the invisible window to
    /// the front gives the app the right to focus the menu (a Windows rule).
    unsafe fn request_menu(hwnd: HWND) {
        let mut pt = Default::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd);
        send(Command::OpenMenu { x: pt.x, y: pt.y });
    }

    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        match msg {
            WM_TRAY => {
                match (lp.0 as u32) & 0xffff {
                    WM_LBUTTONUP => send(Command::Show),
                    WM_RBUTTONUP | WM_CONTEXTMENU => request_menu(hwnd),
                    _ => {}
                }
                LRESULT(0)
            }
            WM_WAKE => {
                send(Command::Show);
                LRESULT(0)
            }
            WM_QUIT_APP => {
                send(Command::Quit);
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            // Explorer restarted: the icon disappears and has to be added again.
            m if m != 0 && m == TASKBAR_CREATED.with(|t| *t.borrow()) => {
                add_icon(hwnd);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }

    pub fn start(tx: Sender<Command>) -> Result<isize, String> {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("scout-vault-tray".into())
            .spawn(move || {
                TX.with(|t| *t.borrow_mut() = Some(tx));
                // SAFETY: registration and message loop of this thread, as Win32 requires.
                unsafe {
                    let module = GetModuleHandleW(None).unwrap_or_default();
                    let class = WNDCLASSW {
                        lpfnWndProc: Some(wndproc),
                        hInstance: module.into(),
                        lpszClassName: CLASS,
                        ..Default::default()
                    };
                    RegisterClassW(&class);
                    TASKBAR_CREATED
                        .with(|t| *t.borrow_mut() = RegisterWindowMessageW(w!("TaskbarCreated")));
                    let hwnd = CreateWindowExW(
                        Default::default(),
                        CLASS,
                        w!("Scout Vault"),
                        Default::default(),
                        0,
                        0,
                        0,
                        0,
                        None,
                        None,
                        Some(module.into()),
                        None,
                    );
                    let hwnd = match hwnd {
                        Ok(h) => h,
                        Err(e) => {
                            let _ = ready_tx.send(Err(e.message()));
                            return;
                        }
                    };
                    add_icon(hwnd);
                    let _ = ready_tx.send(Ok(hwnd.0 as isize));
                    let mut msg = MSG::default();
                    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        ready_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .map_err(|_| "a bandeja não respondeu".to_owned())?
    }

    pub fn remove(hwnd: isize) {
        let hwnd = HWND(hwnd as *mut _);
        // SAFETY: removes the icon and closes the invisible window (the thread ends on its own).
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &data(hwnd));
            let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }

    pub fn balloon(hwnd: isize, title: &str, text: &str) {
        let mut nid = data(HWND(hwnd as *mut _));
        nid.uFlags = NIF_INFO;
        nid.szInfoTitle = wide(title);
        nid.szInfo = wide(text);
        nid.dwInfoFlags = NIIF_INFO;
        // SAFETY: structure filled in above.
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
        }
    }

    pub fn find_existing() -> bool {
        // SAFETY: only looks up another instance's invisible window.
        unsafe { matches!(FindWindowW(CLASS, None), Ok(h) if !h.is_invalid()) }
    }

    pub fn quit_existing() -> bool {
        // SAFETY: only looks up another instance's invisible window and posts a message.
        unsafe {
            match FindWindowW(CLASS, None) {
                Ok(hwnd) if !hwnd.is_invalid() => {
                    PostMessageW(Some(hwnd), WM_QUIT_APP, WPARAM(0), LPARAM(0)).is_ok()
                }
                _ => false,
            }
        }
    }

    pub fn wake_existing() -> bool {
        // SAFETY: only looks up another instance's invisible window and posts a message.
        unsafe {
            match FindWindowW(CLASS, None) {
                Ok(hwnd) if !hwnd.is_invalid() => {
                    PostMessageW(Some(hwnd), WM_WAKE, WPARAM(0), LPARAM(0)).is_ok()
                }
                _ => false,
            }
        }
    }

    pub fn is_iconic(hwnd: isize) -> bool {
        // SAFETY: only queries the state of the app's own window.
        unsafe { IsIconic(HWND(hwnd as *mut _)).as_bool() }
    }

    pub fn is_visible(hwnd: isize) -> bool {
        // SAFETY: only queries the state of the app's own window.
        unsafe {
            windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(HWND(hwnd as *mut _)).as_bool()
        }
    }

    pub fn frameless(hwnd: isize) {
        use windows::Win32::Graphics::Dwm::{
            DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE,
            DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
        };
        let hwnd = HWND(hwnd as *mut _);
        // SAFETY: visual attributes of our own window; 4-byte values.
        // On Windows 10 these attributes don't exist and the call just fails.
        unsafe {
            let corner = DWMWCP_ROUND;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                (&raw const corner).cast(),
                std::mem::size_of_val(&corner) as u32,
            );
            let none = DWMWA_COLOR_NONE;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_BORDER_COLOR,
                (&raw const none).cast(),
                std::mem::size_of_val(&none) as u32,
            );
        }
    }

    pub fn work_area_and_scale() -> ((i32, i32, i32, i32), f32) {
        let mut rect = RECT::default();
        // SAFETY: only reads the work area and the system DPI.
        unsafe {
            let _ = SystemParametersInfoW(
                SPI_GETWORKAREA,
                0,
                Some(&mut rect as *mut RECT as *mut _),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            );
            let scale = GetDpiForSystem() as f32 / 96.;
            (
                (rect.left, rect.top, rect.right, rect.bottom),
                if scale > 0. { scale } else { 1. },
            )
        }
    }

    pub fn show(hwnd: isize, visible: bool) {
        let hwnd = HWND(hwnd as *mut _);
        // SAFETY: HWND of the app's own window.
        unsafe {
            if visible {
                let _ = ShowWindow(hwnd, SW_SHOW);
                if IsIconic(hwnd).as_bool() {
                    let _ = ShowWindow(hwnd, SW_RESTORE);
                }
                let _ = SetForegroundWindow(hwnd);
            } else {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::Command;

    pub fn start(_: tokio::sync::mpsc::UnboundedSender<Command>) -> Result<isize, String> {
        Err("bandeja só no Windows por enquanto".into())
    }
    pub fn remove(_: isize) {}
    pub fn balloon(_: isize, _: &str, _: &str) {}
    pub fn wake_existing() -> bool {
        false
    }
    pub fn find_existing() -> bool {
        false
    }
    pub fn quit_existing() -> bool {
        false
    }
    pub fn show(_: isize, _: bool) {}
    pub fn is_iconic(_: isize) -> bool {
        false
    }
    pub fn is_visible(_: isize) -> bool {
        true
    }
    pub fn frameless(_: isize) {}
    pub fn work_area_and_scale() -> ((i32, i32, i32, i32), f32) {
        ((0, 0, 1920, 1080), 1.)
    }
}

#[cfg(all(test, windows))]
mod tests {
    /// Creates the real icon for a moment (no window, no balloon), checks
    /// that another instance finds it and removes it. Run with `--ignored`.
    #[test]
    #[ignore]
    fn tray_icon_starts_is_found_and_goes_away() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let tray = super::start(tx).expect("criar o ícone");
        // The "second instance" finds the first one and asks it to show up.
        assert!(super::wake_existing_instance());
        let mut got = None;
        for _ in 0..200 {
            if let Ok(c) = rx.try_recv() {
                got = Some(c);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(got, Some(super::Command::Show));
        drop(tray);
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(!super::wake_existing_instance(), "o ícone saiu junto");
    }
}

#[cfg(test)]
mod menu_tests {
    use super::place_menu;

    const WORK: (i32, i32, i32, i32) = (0, 0, 1920, 1040);

    #[test]
    fn opens_above_and_left_of_the_cursor_in_the_tray() {
        assert_eq!(place_menu((1800, 1060), (300, 400), WORK), (1500, 640));
    }

    #[test]
    fn stays_inside_the_work_area() {
        // Cursor in the top corner (taskbar at the top): goes down.
        assert_eq!(
            place_menu((1800, 10), (300, 400), (0, 40, 1920, 1080)),
            (1500, 40)
        );
        // Near the left edge: goes to the right of the cursor.
        assert_eq!(place_menu((100, 1060), (300, 400), WORK), (100, 640));
        // A menu bigger than the screen doesn't go past the left/top.
        assert_eq!(place_menu((50, 50), (3000, 3000), WORK), (0, 0));
    }
}
