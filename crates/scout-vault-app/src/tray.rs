//! Ícone na bandeja do Windows (área de notificação, perto do relógio).
//!
//! Fechar a janela só a esconde: o app segue rodando — o atalho `Ctrl+Alt+A`
//! e o preenchimento no navegador continuam funcionando. Pelo ícone:
//! clique abre a janela; o botão direito abre o menu do app
//! ([`crate::tray_menu`]), com o tema e a cor de destaque: **acesso rápido**
//! (copiar usuário, senha ou código dos itens fixados, sem abrir a janela),
//! **Abrir**, **Gerar e copiar senha**, **Sincronizar agora**,
//! **Bloquear**/**Desbloquear…** e **Sair**.
//!
//! O ícone vive numa thread própria com uma janela invisível do Win32 (como o
//! atalho global), e os comandos chegam ao app por um canal.
//! Abrir o executável de novo com o app já rodando só mostra a janela existente.

use std::sync::atomic::{AtomicBool, Ordering};

use gpui_kit::assets::IconName;
use tokio::sync::mpsc::UnboundedSender;

use crate::vault_view::CopyShortcut;

/// O que a bandeja pede ao app.
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
    /// Busca rápida: digitar este item no app que estava em foco.
    Fill {
        item_id: String,
    },
    /// Busca rápida: mostrar este item no Scout Vault.
    OpenItem {
        item_id: String,
    },
    /// Botão direito no ícone: abrir o menu com o canto no cursor (pixels de tela).
    OpenMenu {
        x: i32,
        y: i32,
    },
}

/// Um item fixado no acesso rápido, como o menu precisa dele.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PinnedEntry {
    pub id: String,
    pub name: String,
    pub has_username: bool,
    pub has_password: bool,
    pub has_totp: bool,
    /// Ícone do tipo do item.
    pub icon: Option<IconName>,
}

/// O que o menu mostra: depende de o cofre estar aberto e do acesso rápido.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MenuState {
    pub unlocked: bool,
    pub pinned: Vec<PinnedEntry>,
}

/// Onde pôr o menu (canto de cima à esquerda, em pixels físicos) para que ele
/// abra junto do cursor e caiba na área de trabalho. Na bandeja de baixo ele
/// sobe a partir do cursor; com a barra de tarefas em cima ou à esquerda, desce
/// ou vai para a direita.
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

/// Área de trabalho (sem a barra de tarefas) e escala do monitor principal.
pub fn work_area_and_scale() -> ((i32, i32, i32, i32), f32) {
    platform::work_area_and_scale()
}

static SYSTEM: AtomicBool = AtomicBool::new(false);

/// Permite criar o ícone e esconder janelas (só o app real; nunca nos testes).
pub fn enable_system() {
    SYSTEM.store(true, Ordering::Relaxed);
}

pub fn system_enabled() -> bool {
    SYSTEM.load(Ordering::Relaxed)
}

/// Ícone ativo; soltar o valor o remove da bandeja.
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
    /// Balão do Windows (uma frase), ex.: "continua rodando na bandeja".
    pub fn notify(&self, title: &str, text: &str) {
        platform::balloon(self.hwnd, title, text);
    }
}

/// Cria o ícone. Cada clique/menu manda um [`Command`] por `tx`.
pub fn start(tx: UnboundedSender<Command>) -> Result<Tray, String> {
    platform::start(tx).map(|hwnd| Tray { hwnd })
}

/// Se o app já está rodando (na bandeja), pede para ele aparecer e devolve `true`.
pub fn wake_existing_instance() -> bool {
    platform::wake_existing()
}

/// Esconde a janela do app (some da barra de tarefas; o ícone da bandeja fica).
pub fn hide_window(window: &gpui_kit::Window) {
    if let Some(hwnd) = hwnd_of(window) {
        platform::show(hwnd, false);
    }
}

/// Mostra de novo e traz para a frente.
pub fn show_window(window: &gpui_kit::Window) {
    if let Some(hwnd) = hwnd_of(window) {
        platform::show(hwnd, true);
    }
    window.activate_window();
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
    /// Mensagem do ícone (cliques).
    const WM_TRAY: u32 = WM_APP + 1;
    /// Mensagem de outra instância: "apareça".
    const WM_WAKE: u32 = WM_APP + 2;
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
        // Ícone do executável (recurso 1, o mesmo da barra de tarefas).
        let module = GetModuleHandleW(None).unwrap_or_default();
        nid.hIcon = LoadIconW(Some(module.into()), PCWSTR(1 as _))
            .or_else(|_| LoadIconW(None, IDI_APPLICATION))
            .unwrap_or_default();
        nid.szTip = wide("Scout Vault");
        let _ = Shell_NotifyIconW(NIM_ADD, &nid);
    }

    /// Pede ao app para abrir o menu no cursor. Trazer a janela invisível para
    /// a frente dá ao app o direito de focar o menu (regra do Windows).
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
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            // O Explorer reiniciou: o ícone some e precisa ser posto de novo.
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
        // SAFETY: tira o ícone e fecha a janela invisível (a thread termina sozinha).
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
        // SAFETY: estrutura preenchida acima.
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
        }
    }

    pub fn wake_existing() -> bool {
        // SAFETY: só procura a janela invisível de outra instância e posta uma mensagem.
        unsafe {
            match FindWindowW(CLASS, None) {
                Ok(hwnd) if !hwnd.is_invalid() => {
                    PostMessageW(Some(hwnd), WM_WAKE, WPARAM(0), LPARAM(0)).is_ok()
                }
                _ => false,
            }
        }
    }

    pub fn work_area_and_scale() -> ((i32, i32, i32, i32), f32) {
        let mut rect = RECT::default();
        // SAFETY: só lê a área de trabalho e o DPI do sistema.
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
        // SAFETY: HWND da própria janela do app.
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
    pub fn show(_: isize, _: bool) {}
    pub fn work_area_and_scale() -> ((i32, i32, i32, i32), f32) {
        ((0, 0, 1920, 1080), 1.)
    }
}

#[cfg(all(test, windows))]
mod tests {
    /// Cria o ícone de verdade por um instante (sem janela, sem balão), confere
    /// que outra instância o encontra e remove. Rode com `--ignored`.
    #[test]
    #[ignore]
    fn tray_icon_starts_is_found_and_goes_away() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let tray = super::start(tx).expect("criar o ícone");
        // A "segunda instância" acha a primeira e pede para aparecer.
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
        // Cursor no canto de cima (barra de tarefas em cima): desce.
        assert_eq!(
            place_menu((1800, 10), (300, 400), (0, 40, 1920, 1080)),
            (1500, 40)
        );
        // Perto da borda esquerda: vai para a direita do cursor.
        assert_eq!(place_menu((100, 1060), (300, 400), WORK), (100, 640));
        // Menu maior que a tela não sai pela esquerda/topo.
        assert_eq!(place_menu((50, 50), (3000, 3000), WORK), (0, 0));
    }
}
