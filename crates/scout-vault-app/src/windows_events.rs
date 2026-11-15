//! Avisos do Windows que pedem para o cofre bloquear: a sessão foi bloqueada
//! (`Win+L`, tela de bloqueio) ou o computador vai suspender/hibernar.
//!
//! Uma janela invisível numa thread própria recebe `WM_WTSSESSION_CHANGE`
//! (depois de `WTSRegisterSessionNotification`) e `WM_POWERBROADCAST`; cada
//! aviso vira um [`OsEvent`] no canal do app. Só roda no app de verdade
//! (nunca nos testes), como a bandeja.

/// O que o Windows avisou.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OsEvent {
    /// A sessão do Windows foi bloqueada.
    SessionLocked,
    /// O computador vai suspender ou hibernar.
    Suspending,
}

/// Escuta ativa; soltar o valor fecha a janela invisível.
pub struct Watcher {
    #[cfg_attr(not(windows), allow(dead_code))]
    hwnd: isize,
}

impl Drop for Watcher {
    fn drop(&mut self) {
        platform::stop(self.hwnd);
    }
}

/// Começa a escutar. Cada aviso manda um [`OsEvent`] por `tx`.
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
    /// `wParam` de `WM_WTSSESSION_CHANGE`: sessão bloqueada.
    const WTS_SESSION_LOCK: usize = 0x7;
    /// `wParam` de `WM_POWERBROADCAST`: vai suspender.
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

    /// Traduz uma mensagem da janela num aviso (separado para testar).
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
                // SAFETY: registro e laço de mensagens desta thread, como pede o Win32.
                unsafe {
                    let module = GetModuleHandleW(None).unwrap_or_default();
                    let class = WNDCLASSW {
                        lpfnWndProc: Some(wndproc),
                        hInstance: module.into(),
                        lpszClassName: CLASS,
                        ..Default::default()
                    };
                    RegisterClassW(&class);
                    // Janela comum (invisível), não "message-only": estas só
                    // não recebem os avisos de energia enviados a todas.
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
        // SAFETY: fecha a janela invisível desta escuta (a thread termina sozinha).
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
            // Desbloquear (0x8) não bloqueia nada.
            assert_eq!(event_for(WM_WTSSESSION_CHANGE, 0x8), None);
            assert_eq!(event_for(WM_POWERBROADCAST, 0x4), Some(OsEvent::Suspending));
            // Voltou da suspensão (0x7 / 0x12): nada.
            assert_eq!(event_for(WM_POWERBROADCAST, 0x12), None);
        }

        /// Cria a escuta de verdade por um instante e solta. Rode com `--ignored`.
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
