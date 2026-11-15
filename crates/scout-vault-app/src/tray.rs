//! Ícone na bandeja do Windows (área de notificação, perto do relógio).
//!
//! Fechar a janela só a esconde: o app segue rodando — o atalho `Ctrl+Alt+A`
//! e o preenchimento no navegador continuam funcionando. Pelo ícone:
//! clique abre a janela; o botão direito abre um menu com **Abrir**, o
//! **acesso rápido** (copiar usuário, senha ou código dos itens fixados, sem
//! abrir a janela), **Gerar e copiar senha**, **Sincronizar agora**,
//! **Bloquear**/**Desbloquear…** e **Sair**.
//!
//! O menu é descrito por [`entries`] a partir de um [`MenuState`] que o app
//! mantém atualizado; os nomes dos itens fixados ficam só na memória.
//!
//! O ícone vive numa thread própria com uma janela invisível do Win32 (como o
//! atalho global), e os comandos chegam ao app por um canal.
//! Abrir o executável de novo com o app já rodando só mostra a janela existente.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use crate::vault_view::CopyShortcut;

/// O que a bandeja pede ao app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Show,
    Lock,
    Quit,
    Sync,
    GeneratePassword,
    Copy { item_id: String, what: CopyShortcut },
}

/// Um item fixado no acesso rápido, como o menu precisa dele.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PinnedEntry {
    pub id: String,
    pub name: String,
    pub has_username: bool,
    pub has_password: bool,
    pub has_totp: bool,
}

/// O que o menu mostra: depende de o cofre estar aberto e do acesso rápido.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MenuState {
    pub unlocked: bool,
    pub pinned: Vec<PinnedEntry>,
}

/// Uma linha do menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Action {
        label: String,
        command: Command,
        default: bool,
    },
    Separator,
    Submenu {
        label: String,
        entries: Vec<Entry>,
    },
}

fn action(label: impl Into<String>, command: Command) -> Entry {
    Entry::Action {
        label: label.into(),
        command,
        default: false,
    }
}

/// Maior nome de item mostrado no menu (o resto vira "…").
const MAX_LABEL: usize = 40;

/// Nome pronto para o menu do Win32: encurtado e com `&` escapado (sozinho,
/// `&` sublinha a letra seguinte como atalho).
pub fn menu_label(name: &str) -> String {
    let name = name.trim();
    let short: String = if name.chars().count() > MAX_LABEL {
        let mut cut: String = name.chars().take(MAX_LABEL - 1).collect();
        cut.push('…');
        cut
    } else {
        name.to_owned()
    };
    short.replace('&', "&&")
}

/// O menu do botão direito para este estado.
pub fn entries(state: &MenuState) -> Vec<Entry> {
    let mut out = vec![Entry::Action {
        label: "Abrir Scout Vault".into(),
        command: Command::Show,
        default: true,
    }];
    if state.unlocked {
        let quick: Vec<Entry> = state
            .pinned
            .iter()
            .filter_map(|p| {
                let mut copy = Vec::new();
                for (has, label, what) in [
                    (p.has_username, "Copiar usuário", CopyShortcut::Username),
                    (p.has_password, "Copiar senha", CopyShortcut::Password),
                    (p.has_totp, "Copiar código", CopyShortcut::TotpCode),
                ] {
                    if has {
                        copy.push(action(
                            label,
                            Command::Copy {
                                item_id: p.id.clone(),
                                what,
                            },
                        ));
                    }
                }
                (!copy.is_empty()).then(|| Entry::Submenu {
                    label: menu_label(&p.name),
                    entries: copy,
                })
            })
            .collect();
        out.push(Entry::Separator);
        if !quick.is_empty() {
            out.push(Entry::Submenu {
                label: "Acesso rápido".into(),
                entries: quick,
            });
        }
        out.push(action("Gerar e copiar senha", Command::GeneratePassword));
        out.push(action("Sincronizar agora", Command::Sync));
        out.push(Entry::Separator);
        out.push(action("Bloquear", Command::Lock));
    } else {
        out.push(action("Desbloquear…", Command::Show));
    }
    out.push(Entry::Separator);
    out.push(action("Sair", Command::Quit));
    out
}

static MENU: Mutex<MenuState> = Mutex::new(MenuState {
    unlocked: false,
    pinned: Vec::new(),
});

/// Atualiza o que o menu mostra (o app chama a cada mudança de tela ou de pinos).
pub fn set_menu_state(state: MenuState) {
    let mut current = MENU.lock().unwrap_or_else(|e| e.into_inner());
    if *current != state {
        *current = state;
    }
}

/// O estado atual do menu (os testes conferem por aqui).
pub fn menu_state() -> MenuState {
    MENU.lock().unwrap_or_else(|e| e.into_inner()).clone()
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
pub fn start(tx: std::sync::mpsc::Sender<Command>) -> Result<Tray, String> {
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
    use std::sync::mpsc::Sender;

    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::Shell::{
        Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD,
        NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
    };
    use windows::Win32::UI::WindowsAndMessaging::*;

    use super::{Command, Entry};

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

    fn wide_z(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Monta o menu nativo; `commands[id - 1]` é o comando de cada linha.
    unsafe fn build(menu: HMENU, entries: &[Entry], commands: &mut Vec<Command>) {
        for entry in entries {
            match entry {
                Entry::Separator => {
                    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
                }
                Entry::Action {
                    label,
                    command,
                    default,
                } => {
                    commands.push(command.clone());
                    let id = commands.len();
                    let text = wide_z(label);
                    let _ = AppendMenuW(menu, MF_STRING, id, PCWSTR(text.as_ptr()));
                    if *default {
                        let _ = SetMenuDefaultItem(menu, id as u32, 0);
                    }
                }
                Entry::Submenu { label, entries } => {
                    let Ok(sub) = CreatePopupMenu() else { continue };
                    build(sub, entries, commands);
                    let text = wide_z(label);
                    // O submenu passa a ser do menu pai (sai junto no DestroyMenu).
                    let _ = AppendMenuW(
                        menu,
                        MF_STRING | MF_POPUP,
                        sub.0 as usize,
                        PCWSTR(text.as_ptr()),
                    );
                }
            }
        }
    }

    unsafe fn show_menu(hwnd: HWND) {
        let Ok(menu) = CreatePopupMenu() else { return };
        let mut commands = Vec::new();
        build(menu, &super::entries(&super::menu_state()), &mut commands);
        let mut pt = Default::default();
        let _ = GetCursorPos(&mut pt);
        // Sem isso o menu não fecha ao clicar fora (comportamento documentado do Win32).
        let _ = SetForegroundWindow(hwnd);
        let chosen = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
            pt.x,
            pt.y,
            None,
            hwnd,
            None,
        );
        let _ = DestroyMenu(menu);
        if let Some(command) = (chosen.0 as usize)
            .checked_sub(1)
            .and_then(|i| commands.get(i))
        {
            send(command.clone());
        }
    }

    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        match msg {
            WM_TRAY => {
                match (lp.0 as u32) & 0xffff {
                    WM_LBUTTONUP => send(Command::Show),
                    WM_RBUTTONUP | WM_CONTEXTMENU => show_menu(hwnd),
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

    pub fn start(_: std::sync::mpsc::Sender<Command>) -> Result<isize, String> {
        Err("bandeja só no Windows por enquanto".into())
    }
    pub fn remove(_: isize) {}
    pub fn balloon(_: isize, _: &str, _: &str) {}
    pub fn wake_existing() -> bool {
        false
    }
    pub fn show(_: isize, _: bool) {}
}

#[cfg(all(test, windows))]
mod tests {
    /// Cria o ícone de verdade por um instante (sem janela, sem balão), confere
    /// que outra instância o encontra e remove. Rode com `--ignored`.
    #[test]
    #[ignore]
    fn tray_icon_starts_is_found_and_goes_away() {
        let (tx, rx) = std::sync::mpsc::channel();
        let tray = super::start(tx).expect("criar o ícone");
        // A "segunda instância" acha a primeira e pede para aparecer.
        assert!(super::wake_existing_instance());
        assert_eq!(
            rx.recv_timeout(std::time::Duration::from_secs(2)),
            Ok(super::Command::Show)
        );
        drop(tray);
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(!super::wake_existing_instance(), "o ícone saiu junto");
    }
}

#[cfg(test)]
mod menu_tests {
    use super::{entries, menu_label, Command, Entry, MenuState, PinnedEntry};
    use crate::vault_view::CopyShortcut;

    fn labels(entries: &[Entry]) -> Vec<String> {
        entries
            .iter()
            .map(|e| match e {
                Entry::Action { label, .. } | Entry::Submenu { label, .. } => label.clone(),
                Entry::Separator => "-".into(),
            })
            .collect()
    }

    #[test]
    fn locked_menu_only_opens_and_quits() {
        let menu = entries(&MenuState::default());
        assert_eq!(
            labels(&menu),
            ["Abrir Scout Vault", "Desbloquear…", "-", "Sair"]
        );
        assert!(matches!(
            &menu[0],
            Entry::Action {
                command: Command::Show,
                default: true,
                ..
            }
        ));
    }

    #[test]
    fn unlocked_menu_has_quick_access_with_only_existing_fields() {
        let state = MenuState {
            unlocked: true,
            pinned: vec![
                PinnedEntry {
                    id: "a".into(),
                    name: "Banco & Cia".into(),
                    has_username: true,
                    has_password: true,
                    has_totp: false,
                },
                // Nota sem nada para copiar: não aparece.
                PinnedEntry {
                    id: "n".into(),
                    name: "Nota".into(),
                    ..Default::default()
                },
            ],
        };
        let menu = entries(&state);
        assert_eq!(
            labels(&menu),
            [
                "Abrir Scout Vault",
                "-",
                "Acesso rápido",
                "Gerar e copiar senha",
                "Sincronizar agora",
                "-",
                "Bloquear",
                "-",
                "Sair"
            ]
        );
        let Entry::Submenu { entries: quick, .. } = &menu[2] else {
            panic!()
        };
        assert_eq!(labels(quick), ["Banco && Cia"]);
        let Entry::Submenu { entries: copy, .. } = &quick[0] else {
            panic!()
        };
        assert_eq!(labels(copy), ["Copiar usuário", "Copiar senha"]);
        assert!(matches!(
            &copy[1],
            Entry::Action {
                command: Command::Copy {
                    item_id,
                    what: CopyShortcut::Password
                },
                ..
            } if item_id == "a"
        ));
        // Sem itens fixados, sem submenu.
        let menu = entries(&MenuState {
            unlocked: true,
            pinned: Vec::new(),
        });
        assert!(!labels(&menu).contains(&"Acesso rápido".to_owned()));
    }

    #[test]
    fn long_names_are_cut_and_ampersands_escaped() {
        assert_eq!(menu_label("  A&B  "), "A&&B");
        let long = "x".repeat(60);
        let cut = menu_label(&long);
        assert_eq!(cut.chars().count(), 40);
        assert!(cut.ends_with('…'));
    }
}
