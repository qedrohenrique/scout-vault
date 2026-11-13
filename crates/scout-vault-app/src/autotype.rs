//! Preenchimento em apps do Windows ("auto-type").
//!
//! Com o cofre aberto, `Ctrl+Alt+A` em qualquer janela faz o Scout Vault escolher o
//! login pelo título da janela e **digitar** `usuário ⇥ senha ↵` nela — como o
//! KeePass. Se houver mais de um candidato (ou nenhum), o Scout Vault aparece com a
//! lista para escolher. Nada passa pela área de transferência.
//!
//! Segurança: antes de cada parte digitada, conferimos que a janela em foco
//! ainda é a de destino; se mudou, paramos sem digitar o resto.

use std::sync::Arc;

use scout_vault_core::vault::{Item, ItemKind};
use zeroize::Zeroizing;

/// Atalho global, para mostrar na interface.
pub const HOTKEY_LABEL: &str = "Ctrl+Alt+A";

/// Pedido de preenchimento: a janela que estava em foco quando o atalho foi usado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// `HWND` da janela de destino (como número, para cruzar threads).
    pub window: isize,
    pub title: String,
    /// A janela em foco era o próprio Scout Vault.
    pub own_window: bool,
}

/// Uma parte do que será digitado.
pub enum Key {
    Text(Zeroizing<String>),
    Tab,
    Enter,
}

impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Key::Text(t) => write!(f, "Text({} chars)", t.chars().count()),
            Key::Tab => f.write_str("Tab"),
            Key::Enter => f.write_str("Enter"),
        }
    }
}

/// Quem digita de verdade (trocado por um falso nos testes).
pub type Typer = Arc<dyn Fn(isize, Vec<Key>) -> Result<(), String> + Send + Sync>;

/// O que digitar para um item: `usuário ⇥ senha` e, se pedido, `↵`.
pub fn sequence(item: &Item, press_enter: bool) -> Vec<Key> {
    let ItemKind::Login(l) = &item.kind else {
        return Vec::new();
    };
    let mut keys = Vec::new();
    let user = l.username.as_deref().filter(|u| !u.is_empty());
    let pass = l
        .password
        .as_ref()
        .map(|p| p.expose())
        .filter(|p| !p.is_empty());
    if let Some(u) = user {
        keys.push(Key::Text(Zeroizing::new(u.to_owned())));
        if pass.is_some() {
            keys.push(Key::Tab);
        }
    }
    if let Some(p) = pass {
        keys.push(Key::Text(Zeroizing::new(p.to_owned())));
    }
    if press_enter && !keys.is_empty() {
        keys.push(Key::Enter);
    }
    keys
}

/// Pode ser digitado: login fora da lixeira, com usuário ou senha.
pub fn typeable(item: &Item) -> bool {
    match &item.kind {
        ItemKind::Login(l) if !item.deleted => {
            l.username.as_deref().is_some_and(|u| !u.is_empty())
                || l.password.as_ref().is_some_and(|p| !p.expose().is_empty())
        }
        _ => false,
    }
}

/// Sufixos de domínio ignorados ao tirar o "nome" do site (`github.com` → `github`).
const SUFFIXES: &[&str] = &[
    "com", "net", "org", "br", "gov", "edu", "io", "co", "app", "dev", "me", "uk", "eu", "us",
    "info", "biz", "tv", "gg", "pt", "de", "fr", "es", "it", "jp",
];

fn host(uri: &str) -> Option<String> {
    let rest = uri.split_once("://").map(|(_, r)| r).unwrap_or(uri);
    let host = rest
        .split(['/', '?', '#', ':'])
        .next()?
        .to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_owned();
    (!host.is_empty()).then_some(host)
}

fn site_name(host: &str) -> Option<&str> {
    host.split('.')
        .rev()
        .find(|label| !SUFFIXES.contains(label))
        .filter(|label| label.len() >= 3)
}

/// Quão bem o item combina com o título da janela (0 = não combina).
pub fn score(item: &Item, title: &str) -> u8 {
    if !typeable(item) {
        return 0;
    }
    let title = title.to_lowercase();
    let mut best = 0;
    let name = item.name.trim().to_lowercase();
    if name.chars().count() >= 3 && title.contains(&name) {
        best = 3;
    }
    if let ItemKind::Login(l) = &item.kind {
        for uri in &l.uris {
            let Some(host) = host(uri) else { continue };
            if title.contains(&host) {
                best = best.max(3);
            } else if site_name(&host).is_some_and(|n| title.contains(n)) {
                best = best.max(2);
            }
        }
    }
    best
}

/// Candidatos para a janela, do melhor para o pior (favoritos primeiro no empate).
pub fn candidates<'a>(items: &'a [Item], title: &str) -> Vec<&'a Item> {
    let mut scored: Vec<(u8, &Item)> = items
        .iter()
        .map(|i| (score(i, title), i))
        .filter(|(s, _)| *s > 0)
        .collect();
    scored.sort_by(|(sa, a), (sb, b)| {
        sb.cmp(sa)
            .then(b.favorite.cmp(&a.favorite))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    // Só os de melhor pontuação: um site exato vence um palpite pelo nome do site.
    let top = scored.first().map(|(s, _)| *s).unwrap_or(0);
    scored
        .into_iter()
        .filter(|(s, _)| *s == top)
        .map(|(_, i)| i)
        .collect()
}

/// Digitador do sistema (Windows); nos outros sistemas, só devolve erro.
pub fn system_typer() -> Typer {
    Arc::new(|window, keys| platform::type_keys(window, &keys))
}

static SYSTEM: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Permite registrar o atalho global. Só o app real chama isto; nos testes o
/// atalho nunca é registrado no sistema.
pub fn enable_system_hotkey() {
    SYSTEM.store(true, std::sync::atomic::Ordering::Relaxed);
}

pub fn system_hotkey_enabled() -> bool {
    SYSTEM.load(std::sync::atomic::Ordering::Relaxed)
}

/// Atalho global ativo; soltar este valor desliga o atalho.
pub struct Listener {
    #[cfg_attr(not(windows), allow(dead_code))]
    thread_id: u32,
}

impl Drop for Listener {
    fn drop(&mut self) {
        platform::stop(self.thread_id);
    }
}

/// Liga o atalho global; cada uso manda um [`Request`] por `tx`.
/// Falha se outro programa já usa o mesmo atalho.
pub fn listen(tx: std::sync::mpsc::Sender<Request>) -> Result<Listener, String> {
    platform::listen(tx).map(|thread_id| Listener { thread_id })
}

#[cfg(windows)]
mod platform {
    use std::time::{Duration, Instant};

    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Threading::GetCurrentProcessId;
    use windows::Win32::UI::Input::KeyboardAndMouse::*;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetMessageW, GetWindowTextW, GetWindowThreadProcessId,
        SetForegroundWindow, MSG, WM_HOTKEY,
    };

    use super::{Key, Request};

    const HOTKEY_ID: i32 = 0xC0F;
    const VK_A: u32 = 0x41;

    fn foreground() -> HWND {
        // SAFETY: sem parâmetros.
        unsafe { GetForegroundWindow() }
    }

    fn title_of(hwnd: HWND) -> String {
        let mut buf = [0u16; 512];
        // SAFETY: buffer válido.
        let n = unsafe { GetWindowTextW(hwnd, &mut buf) };
        String::from_utf16_lossy(&buf[..n.max(0) as usize])
    }

    fn is_own(hwnd: HWND) -> bool {
        let mut pid = 0u32;
        // SAFETY: ponteiro para variável local.
        unsafe {
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            pid == GetCurrentProcessId()
        }
    }

    pub fn stop(thread_id: u32) {
        use windows::Win32::Foundation::{LPARAM, WPARAM};
        use windows::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_QUIT};
        // SAFETY: só posta uma mensagem; se a thread já acabou, falha sem efeito.
        let _ = unsafe { PostThreadMessageW(thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
    }

    pub fn listen(tx: std::sync::mpsc::Sender<Request>) -> Result<u32, String> {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("scout-vault-autotype-hotkey".into())
            .spawn(move || {
                // O atalho pertence a esta thread; as mensagens chegam na fila dela.
                // SAFETY: registro simples, sem janela.
                let registered = unsafe {
                    RegisterHotKey(None, HOTKEY_ID, MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, VK_A)
                };
                let ok = registered.is_ok();
                // SAFETY: sem parâmetros.
                let thread_id = unsafe { windows::Win32::System::Threading::GetCurrentThreadId() };
                let _ = ready_tx.send(registered.map(|_| thread_id).map_err(|e| e.message()));
                if !ok {
                    return;
                }
                let mut msg = MSG::default();
                // SAFETY: laço de mensagens padrão desta thread.
                while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
                    if msg.message == WM_HOTKEY && msg.wParam.0 as i32 == HOTKEY_ID {
                        let hwnd = foreground();
                        let request = Request {
                            window: hwnd.0 as isize,
                            title: title_of(hwnd),
                            own_window: is_own(hwnd),
                        };
                        if tx.send(request).is_err() {
                            break;
                        }
                    }
                }
                // SAFETY: mesmo id registrado acima.
                let _ = unsafe { UnregisterHotKey(None, HOTKEY_ID) };
            })
            .map_err(|e| e.to_string())?;
        match ready_rx.recv_timeout(Duration::from_secs(2)) {
            Ok(Ok(thread_id)) => Ok(thread_id),
            Ok(Err(e)) => Err(format!(
                "O atalho {} já está em uso por outro programa ({e}).",
                super::HOTKEY_LABEL
            )),
            Err(_) => Err("O atalho global não respondeu.".into()),
        }
    }

    fn key_input(vk: VIRTUAL_KEY, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: scan,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    fn send(inputs: &mut [INPUT]) -> Result<(), String> {
        // SAFETY: slice válido de INPUT.
        let sent = unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
        // Não deixa as teclas da senha num buffer solto.
        // SAFETY: zera a própria memória do slice (INPUT é POD).
        unsafe {
            std::ptr::write_bytes(inputs.as_mut_ptr(), 0, inputs.len());
        }
        if sent as usize == inputs.len() {
            Ok(())
        } else {
            Err("O Windows bloqueou a digitação (a janela de destino pode estar rodando como administrador).".into())
        }
    }

    /// Espera soltar Ctrl/Alt/Shift/Win do atalho; senão o texto vira atalhos.
    fn wait_modifiers_released() {
        let deadline = Instant::now() + Duration::from_secs(3);
        let held = || {
            [VK_CONTROL, VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN]
                .iter()
                // SAFETY: consulta de estado de tecla.
                .any(|vk| unsafe { GetAsyncKeyState(vk.0 as i32) } as u16 & 0x8000 != 0)
        };
        while held() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    pub fn type_keys(window: isize, keys: &[Key]) -> Result<(), String> {
        let target = HWND(window as *mut _);
        wait_modifiers_released();
        if foreground() != target {
            // SAFETY: HWND recebido do próprio sistema.
            let _ = unsafe { SetForegroundWindow(target) };
            std::thread::sleep(Duration::from_millis(150));
        }
        for key in keys {
            if foreground() != target {
                return Err("A janela de destino perdeu o foco; parei de digitar.".into());
            }
            let mut inputs: Vec<INPUT> = match key {
                Key::Text(text) => text
                    .encode_utf16()
                    .flat_map(|unit| {
                        [
                            key_input(VIRTUAL_KEY(0), unit, KEYEVENTF_UNICODE),
                            key_input(VIRTUAL_KEY(0), unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP),
                        ]
                    })
                    .collect(),
                Key::Tab => vec![
                    key_input(VK_TAB, 0, KEYBD_EVENT_FLAGS(0)),
                    key_input(VK_TAB, 0, KEYEVENTF_KEYUP),
                ],
                Key::Enter => vec![
                    key_input(VK_RETURN, 0, KEYBD_EVENT_FLAGS(0)),
                    key_input(VK_RETURN, 0, KEYEVENTF_KEYUP),
                ],
            };
            send(&mut inputs)?;
            // Dá tempo para o app trocar de campo depois do Tab.
            std::thread::sleep(Duration::from_millis(60));
        }
        Ok(())
    }
}

#[cfg(not(windows))]
mod platform {
    use super::{Key, Request};

    pub fn listen(_: std::sync::mpsc::Sender<Request>) -> Result<u32, String> {
        Err("O preenchimento em apps só funciona no Windows por enquanto.".into())
    }

    pub fn stop(_: u32) {}

    pub fn type_keys(_: isize, _: &[Key]) -> Result<(), String> {
        Err("O preenchimento em apps só funciona no Windows por enquanto.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scout_vault_core::vault::Login;
    use scout_vault_core::Secret;

    fn login(name: &str, user: &str, pass: &str, uris: &[&str]) -> Item {
        Item {
            id: name.to_owned(),
            organization_id: None,
            folder_id: None,
            name: name.to_owned(),
            notes: None,
            favorite: false,
            reprompt: false,
            deleted: false,
            revision_date: None,
            attachments: Vec::new(),
            fields: Vec::new(),
            kind: ItemKind::Login(Login {
                username: (!user.is_empty()).then(|| user.to_owned()),
                password: (!pass.is_empty()).then(|| Secret::new(pass.to_owned())),
                totp: None,
                uris: uris.iter().map(|u| u.to_string()).collect(),
            }),
        }
    }

    fn describe(keys: &[Key]) -> Vec<String> {
        keys.iter()
            .map(|k| match k {
                Key::Text(t) => t.to_string(),
                Key::Tab => "⇥".into(),
                Key::Enter => "↵".into(),
            })
            .collect()
    }

    #[test]
    fn sequence_is_user_tab_password_enter() {
        let item = login("GitHub", "eu", "s3nha", &[]);
        assert_eq!(describe(&sequence(&item, true)), ["eu", "⇥", "s3nha", "↵"]);
        assert_eq!(describe(&sequence(&item, false)), ["eu", "⇥", "s3nha"]);
        let only_pass = login("Wi-Fi", "", "abc", &[]);
        assert_eq!(describe(&sequence(&only_pass, true)), ["abc", "↵"]);
        let only_user = login("X", "eu", "", &[]);
        assert_eq!(describe(&sequence(&only_user, false)), ["eu"]);
    }

    #[test]
    fn matches_by_name_and_site() {
        let items = vec![
            login("Steam", "jogador", "x", &[]),
            login("Conta Git", "eu", "x", &["https://github.com/login"]),
            login("Banco", "eu", "x", &["https://www.bb.com.br"]),
            login("Discord", "eu", "x", &["https://discord.com"]),
        ];
        let names = |title: &str| -> Vec<String> {
            candidates(&items, title)
                .into_iter()
                .map(|i| i.name.clone())
                .collect()
        };
        assert_eq!(names("Steam"), ["Steam"]);
        assert_eq!(
            names("Sign in to GitHub · GitHub — Mozilla Firefox"),
            ["Conta Git"]
        );
        // Host inteiro no título.
        assert_eq!(names("bb.com.br - Login"), ["Banco"]);
        // "bb" é curto demais para palpite pelo nome do site.
        assert!(names("Bloco de notas bb").is_empty());
        assert!(names("Documento - Word").is_empty());
    }

    #[test]
    fn exact_matches_beat_guesses_and_trash_is_ignored() {
        let mut deleted = login("GitHub velho", "x", "y", &["https://github.com"]);
        deleted.deleted = true;
        let items = vec![
            login("Palpite", "a", "b", &["https://github.com"]),
            login("GitHub", "a", "b", &[]),
            deleted,
        ];
        // "github" no título: o item chamado GitHub (3) e o site github.com (2 — só o nome).
        let got: Vec<_> = candidates(&items, "GitHub Desktop")
            .into_iter()
            .map(|i| i.name.as_str())
            .collect();
        assert_eq!(got, ["GitHub"]);
    }

    /// Registra o atalho de verdade no Windows (sem digitar nada). `--ignored`.
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn hotkey_registers_conflicts_and_releases() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let first = listen(tx.clone()).expect("registrar");
        let second = listen(tx.clone());
        assert!(
            second.is_err(),
            "o mesmo atalho não pode ser registrado duas vezes"
        );
        drop(first);
        std::thread::sleep(std::time::Duration::from_millis(200));
        let again = listen(tx).expect("depois de soltar, registra de novo");
        drop(again);
    }

    #[test]
    fn key_debug_hides_text() {
        let k = Key::Text(Zeroizing::new("segredo".into()));
        assert_eq!(format!("{k:?}"), "Text(7 chars)");
    }
}
