//! Windows notices that ask the vault to lock: the session was locked
//! (`Win+L`, lock screen) or the computer is about to sleep/hibernate.
//!
//! An invisible window on its own thread receives `WM_WTSSESSION_CHANGE`
//! (after `WTSRegisterSessionNotification`) and `WM_POWERBROADCAST`; each
//! notice becomes an [`OsEvent`] on the app's channel. Only runs in the real app
//! (never in tests), like the tray.

/// What Windows reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OsEvent {
    /// The Windows session was locked.
    SessionLocked,
    /// The computer is about to sleep or hibernate.
    Suspending,
}

/// Active listener; dropping the value closes the invisible window.
pub struct Watcher {
    #[cfg_attr(not(windows), allow(dead_code))]
    hwnd: isize,
}

impl Drop for Watcher {
    fn drop(&mut self) {
        platform::stop(self.hwnd);
    }
}

/// Starts listening. Each notice sends an [`OsEvent`] through `tx`.
pub fn watch(tx: std::sync::mpsc::Sender<OsEvent>) -> Result<Watcher, String> {
    platform::start(tx).map(|hwnd| Watcher { hwnd })
}

#[cfg(windows)]
mod platform {
    use std::cell::RefCell;
    use std::sync::mpsc::Sender;

    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::RemoteDesktop::{
        WTSRegisterSessionNotification, WTSUnRegisterSessionNotification, NOTIFY_FOR_THIS_SESSION,
    };
    use windows::Win32::UI::WindowsAndMessaging::*;

    use super::OsEvent;

    const CLASS: PCWSTR = w!("ScoutVaultWindowsEvents");
    /// `wParam` of `WM_WTSSESSION_CHANGE`: session locked.
    const WTS_SESSION_LOCK: usize = 0x7;
    /// `wParam` of `WM_POWERBROADCAST`: about to sleep.
    const PBT_APMSUSPEND: usize = 0x4;

    thread_local! {
        static TX: RefCell<Option<Sender<OsEvent>>> = const { RefCell::new(None) };
    }

    fn send(event: OsEvent) {
        TX.with(|tx| {
            if let Some(tx) = tx.borrow().as_ref() {
                let _ = tx.send(event);
            }
        });
    }

    /// Translates a window message into a notice (separate so it can be tested).
    pub fn event_for(msg: u32, wp: usize) -> Option<OsEvent> {
        match (msg, wp) {
            (WM_WTSSESSION_CHANGE, WTS_SESSION_LOCK) => Some(OsEvent::SessionLocked),
            (WM_POWERBROADCAST, PBT_APMSUSPEND) => Some(OsEvent::Suspending),
            _ => None,
        }
    }

    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        if let Some(event) = event_for(msg, wp.0) {
            send(event);
        }
        match msg {
            WM_POWERBROADCAST => LRESULT(1),
            WM_DESTROY => {
                let _ = WTSUnRegisterSessionNotification(hwnd);
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }

    pub fn start(tx: Sender<OsEvent>) -> Result<isize, String> {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("scout-vault-windows-events".into())
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
                    // Regular (invisible) window, not "message-only": those alone
                    // don't receive the power notices broadcast to all windows.
                    let hwnd = match CreateWindowExW(
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
                    ) {
                        Ok(h) => h,
                        Err(e) => {
                            let _ = ready_tx.send(Err(e.message()));
                            return;
                        }
                    };
                    if let Err(e) = WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) {
                        let _ = DestroyWindow(hwnd);
                        let _ = ready_tx.send(Err(e.message()));
                        return;
                    }
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
            .map_err(|_| "o aviso de bloqueio do Windows não respondeu".to_owned())?
    }

    pub fn stop(hwnd: isize) {
        // SAFETY: closes this listener's invisible window (the thread ends on its own).
        unsafe {
            let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }

    #[cfg(test)]
    mod tests {
        use super::{event_for, OsEvent};
        use windows::Win32::UI::WindowsAndMessaging::{WM_POWERBROADCAST, WM_WTSSESSION_CHANGE};

        #[test]
        fn lock_and_suspend_become_events() {
            assert_eq!(
                event_for(WM_WTSSESSION_CHANGE, 0x7),
                Some(OsEvent::SessionLocked)
            );
            // Unlocking (0x8) doesn't lock anything.
            assert_eq!(event_for(WM_WTSSESSION_CHANGE, 0x8), None);
            assert_eq!(event_for(WM_POWERBROADCAST, 0x4), Some(OsEvent::Suspending));
            // Resumed from sleep (0x7 / 0x12): nothing.
            assert_eq!(event_for(WM_POWERBROADCAST, 0x12), None);
        }

        /// Creates the real listener for a moment and drops it. Run with `--ignored`.
        #[test]
        #[ignore]
        fn watcher_starts_and_stops() {
            let (tx, _rx) = std::sync::mpsc::channel();
            let watcher = crate::windows_events::watch(tx).expect("escutar o Windows");
            drop(watcher);
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::OsEvent;

    pub fn start(_: std::sync::mpsc::Sender<OsEvent>) -> Result<isize, String> {
        Err("só no Windows por enquanto".into())
    }
    pub fn stop(_: isize) {}
}
