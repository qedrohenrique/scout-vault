//! Filling in Windows apps ("auto-type").
//!
//! With the vault open, `Ctrl+Alt+A` in any window makes Scout Vault pick the
//! login by the window title and **type** `username ⇥ password ↵` into it — like
//! KeePass. If there is more than one candidate (or none), Scout Vault shows up with the
//! list to choose from. Nothing goes through the clipboard.
//!
//! Security: before each typed part, we check that the focused window is
//! still the target; if it changed, we stop without typing the rest. Before
//! the password, Windows UI Automation must say the focused field **is a
//! password field** — otherwise we stop (the cursor could be in a chat box, and
//! `Enter` would send the password). When the app doesn't tell what its fields
//! are, nothing is typed until the person confirms.

use std::sync::Arc;

use scout_vault_core::vault::{Item, ItemKind};
use zeroize::Zeroizing;

/// Global shortcut, to show in the interface.
pub const HOTKEY_LABEL: &str = "Ctrl+Alt+A";

/// Fill request: the window that had focus when the shortcut was used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// `HWND` of the target window (as a number, to cross threads).
    pub window: isize,
    pub title: String,
    /// The focused window was Scout Vault itself.
    pub own_window: bool,
}

/// A part of what will be typed.
pub enum Key {
    Text(Zeroizing<String>),
    /// The password: only typed into a password field (see [`FieldCheck`]).
    Password(Zeroizing<String>),
    Tab,
    Enter,
}

impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Key::Text(t) => write!(f, "Text({} chars)", t.chars().count()),
            Key::Password(t) => write!(f, "Password({} chars)", t.chars().count()),
            Key::Tab => f.write_str("Tab"),
            Key::Enter => f.write_str("Enter"),
        }
    }
}

/// Check that the password goes into a password field?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldCheck {
    Verify,
    /// The person confirmed typing anyway (the app doesn't tell its fields).
    Skip,
}

/// Why typing didn't happen (or stopped).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeError {
    /// Nothing was typed: the app doesn't say whether the field is a password
    /// field. Ask the person before typing.
    CantVerify,
    Failed(String),
}

/// Who actually types (swapped for a fake in tests).
pub type Typer = Arc<dyn Fn(isize, Vec<Key>, FieldCheck) -> Result<(), TypeError> + Send + Sync>;

/// What to type for an item: `username ⇥ password` and, if asked, `↵`.
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
        keys.push(Key::Password(Zeroizing::new(p.to_owned())));
    }
    if press_enter && !keys.is_empty() {
        keys.push(Key::Enter);
    }
    keys
}

/// Can be typed: a login outside the trash, with a username or password.
pub fn typeable(item: &Item) -> bool {
    match &item.kind {
        ItemKind::Login(l) if !item.deleted => {
            l.username.as_deref().is_some_and(|u| !u.is_empty())
                || l.password.as_ref().is_some_and(|p| !p.expose().is_empty())
        }
        _ => false,
    }
}

/// Domain suffixes ignored when taking the site's "name" (`github.com` → `github`).
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

/// How well the item matches the window title (0 = no match).
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

/// Candidates for the window, best to worst (favorites first on a tie).
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
    // Only the top-scoring ones: an exact site beats a guess by the site's name.
    let top = scored.first().map(|(s, _)| *s).unwrap_or(0);
    scored
        .into_iter()
        .filter(|(s, _)| *s == top)
        .map(|(_, i)| i)
        .collect()
}

/// System typist (Windows); on other systems, it only returns an error.
pub fn system_typer() -> Typer {
    Arc::new(|window, keys, check| platform::type_keys(window, &keys, check))
}

static SYSTEM: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Allows registering the global shortcut. Only the real app calls this; in tests the
/// shortcut is never registered with the system.
pub fn enable_system_hotkey() {
    SYSTEM.store(true, std::sync::atomic::Ordering::Relaxed);
}

pub fn system_hotkey_enabled() -> bool {
    SYSTEM.load(std::sync::atomic::Ordering::Relaxed)
}

/// Active global shortcut; dropping this value turns the shortcut off.
pub struct Listener {
    #[cfg_attr(not(windows), allow(dead_code))]
    thread_id: u32,
}

impl Drop for Listener {
    fn drop(&mut self) {
        platform::stop(self.thread_id);
    }
}

/// Turns the global shortcut on; each use sends a [`Request`] through `tx`.
/// Fails if another program already uses the same shortcut.
pub fn listen(tx: std::sync::mpsc::Sender<Request>) -> Result<Listener, String> {
    platform::listen(tx, platform::Hotkey::AutoType).map(|thread_id| Listener { thread_id })
}

/// Turns the quick search shortcut on (`Ctrl+Shift+Space`); the [`Request`] carries the
/// window that had focus (where to type, if the person chooses to fill).
pub fn listen_quick_search(tx: std::sync::mpsc::Sender<Request>) -> Result<Listener, String> {
    platform::listen(tx, platform::Hotkey::QuickSearch).map(|thread_id| Listener { thread_id })
}

#[cfg(windows)]
mod platform {
    use std::time::{Duration, Instant};

    use super::{FieldCheck, TypeError};

    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Threading::GetCurrentProcessId;
    use windows::Win32::UI::Input::KeyboardAndMouse::*;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetMessageW, GetWindowTextW, GetWindowThreadProcessId,
        SetForegroundWindow, MSG, WM_HOTKEY,
    };

    use super::{Key, Request};

    const VK_A: u32 = 0x41;

    /// Which global shortcut (each one on its own thread).
    #[derive(Clone, Copy)]
    pub enum Hotkey {
        AutoType,
        QuickSearch,
    }

    impl Hotkey {
        fn id(self) -> i32 {
            match self {
                Hotkey::AutoType => 0xC0F,
                Hotkey::QuickSearch => 0xC10,
            }
        }

        fn keys(self) -> (HOT_KEY_MODIFIERS, u32) {
            match self {
                Hotkey::AutoType => (MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, VK_A),
                Hotkey::QuickSearch => (MOD_CONTROL | MOD_SHIFT | MOD_NOREPEAT, VK_SPACE.0 as u32),
            }
        }

        fn label(self) -> &'static str {
            match self {
                Hotkey::AutoType => super::HOTKEY_LABEL,
                Hotkey::QuickSearch => crate::quick_search::hotkey_label(),
            }
        }
    }

    fn foreground() -> HWND {
        // SAFETY: no parameters.
        unsafe { GetForegroundWindow() }
    }

    fn title_of(hwnd: HWND) -> String {
        let mut buf = [0u16; 512];
        // SAFETY: valid buffer.
        let n = unsafe { GetWindowTextW(hwnd, &mut buf) };
        String::from_utf16_lossy(&buf[..n.max(0) as usize])
    }

    fn is_own(hwnd: HWND) -> bool {
        let mut pid = 0u32;
        // SAFETY: pointer to a local variable.
        unsafe {
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            pid == GetCurrentProcessId()
        }
    }

    pub fn stop(thread_id: u32) {
        use windows::Win32::Foundation::{LPARAM, WPARAM};
        use windows::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_QUIT};
        // SAFETY: only posts a message; if the thread has already ended, it fails harmlessly.
        let _ = unsafe { PostThreadMessageW(thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
    }

    pub fn listen(tx: std::sync::mpsc::Sender<Request>, hotkey: Hotkey) -> Result<u32, String> {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let hotkey_id = hotkey.id();
        let (mods, vk) = hotkey.keys();
        std::thread::Builder::new()
            .name("scout-vault-autotype-hotkey".into())
            .spawn(move || {
                // The shortcut belongs to this thread; the messages arrive in its queue.
                // SAFETY: simple registration, no window.
                let registered = unsafe { RegisterHotKey(None, hotkey_id, mods, vk) };
                let ok = registered.is_ok();
                // SAFETY: no parameters.
                let thread_id = unsafe { windows::Win32::System::Threading::GetCurrentThreadId() };
                let _ = ready_tx.send(registered.map(|_| thread_id).map_err(|e| e.message()));
                if !ok {
                    return;
                }
                let mut msg = MSG::default();
                // SAFETY: standard message loop of this thread.
                while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
                    if msg.message == WM_HOTKEY && msg.wParam.0 as i32 == hotkey_id {
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
                // SAFETY: same id registered above.
                let _ = unsafe { UnregisterHotKey(None, hotkey_id) };
            })
            .map_err(|e| e.to_string())?;
        match ready_rx.recv_timeout(Duration::from_secs(2)) {
            Ok(Ok(thread_id)) => Ok(thread_id),
            Ok(Err(e)) => Err(t!("autotype.hotkey_taken", hotkey = hotkey.label(), e = e)),
            Err(_) => Err(t!("autotype.hotkey_no_answer").into()),
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
        // SAFETY: valid slice of INPUT.
        let sent = unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
        // Don't leave the password keystrokes in a loose buffer.
        // SAFETY: zeroes the slice's own memory (INPUT is POD).
        unsafe {
            std::ptr::write_bytes(inputs.as_mut_ptr(), 0, inputs.len());
        }
        if sent as usize == inputs.len() {
            Ok(())
        } else {
            Err(t!("autotype.blocked").into())
        }
    }

    /// Waits for the shortcut's Ctrl/Alt/Shift/Win to be released; otherwise the text becomes shortcuts.
    fn wait_modifiers_released() {
        let deadline = Instant::now() + Duration::from_secs(3);
        let held = || {
            [VK_CONTROL, VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN]
                .iter()
                // SAFETY: key state query.
                .any(|vk| unsafe { GetAsyncKeyState(vk.0 as i32) } as u16 & 0x8000 != 0)
        };
        while held() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    /// Windows UI Automation, created once per fill (creating it is the slow part).
    pub(crate) struct Fields {
        uia: Option<windows::Win32::UI::Accessibility::IUIAutomation>,
        com: bool,
    }

    impl Fields {
        pub(crate) fn new() -> Self {
            use windows::Win32::System::Com::{
                CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
            };
            use windows::Win32::UI::Accessibility::CUIAutomation;
            // SAFETY: COM on this thread, released in `drop`.
            unsafe {
                let com = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
                let uia = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok();
                Self { uia, com }
            }
        }

        /// The focused element: (is a text field, is a password field). `None`:
        /// the app doesn't say (no UI Automation support), so it can't be checked.
        pub(crate) fn focused(&self) -> Option<(bool, bool)> {
            use windows::Win32::UI::Accessibility::{
                UIA_ControlTypePropertyId, UIA_EditControlTypeId, UIA_IsPasswordPropertyId,
            };
            // SAFETY: only reads properties of the focused element. Both come in
            // one trip to the other app (cache request), not three.
            unsafe {
                let uia = self.uia.as_ref()?;
                let request = uia.CreateCacheRequest().ok()?;
                request.AddProperty(UIA_IsPasswordPropertyId).ok()?;
                request.AddProperty(UIA_ControlTypePropertyId).ok()?;
                let element = uia.GetFocusedElementBuildCache(&request).ok()?;
                let password = element.CachedIsPassword().ok()?.as_bool();
                let edit = element.CachedControlType().ok()? == UIA_EditControlTypeId;
                Some((edit, password))
            }
        }

        /// Waits a little for the password field: after the Tab (and the first
        /// time an app is asked, e.g. Chromium-based ones) the answer may lag.
        fn wait_for_password(&self) -> Option<bool> {
            let mut last = None;
            for _ in 0..3 {
                last = self.focused().map(|(_, password)| password);
                if last == Some(true) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(80));
            }
            last
        }
    }

    impl Drop for Fields {
        fn drop(&mut self) {
            self.uia = None;
            if self.com {
                // SAFETY: pairs with the successful `CoInitializeEx` in `new`.
                unsafe { windows::Win32::System::Com::CoUninitialize() };
            }
        }
    }

    pub fn type_keys(window: isize, keys: &[Key], check: FieldCheck) -> Result<(), TypeError> {
        let target = HWND(window as *mut _);
        wait_modifiers_released();
        if foreground() != target {
            // SAFETY: HWND received from the system itself.
            let _ = unsafe { SetForegroundWindow(target) };
            std::thread::sleep(Duration::from_millis(150));
        }
        let has_password = keys.iter().any(|k| matches!(k, Key::Password(_)));
        let fields = (check == FieldCheck::Verify && has_password).then(Fields::new);
        // Before typing anything: can this app tell its fields apart? If the
        // focus isn't even on a text field, the app doesn't expose its fields
        // (or the cursor is somewhere odd): ask the person.
        if let Some(fields) = &fields {
            match fields.focused() {
                Some((edit, password)) if edit || password => {}
                _ => return Err(TypeError::CantVerify),
            }
        }
        for key in keys {
            if foreground() != target {
                return Err(TypeError::Failed(t!("autotype.lost_focus").into()));
            }
            if matches!(key, Key::Password(_))
                && fields
                    .as_ref()
                    .is_some_and(|f| f.wait_for_password() != Some(true))
            {
                return Err(TypeError::Failed(t!("autotype.not_password_field").into()));
            }
            let mut inputs: Vec<INPUT> = match key {
                Key::Text(text) | Key::Password(text) => text
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
            send(&mut inputs).map_err(TypeError::Failed)?;
            // Gives the app time to switch fields after the Tab.
            std::thread::sleep(Duration::from_millis(60));
        }
        Ok(())
    }
}

#[cfg(not(windows))]
mod platform {
    use super::{Key, Request};

    #[derive(Clone, Copy)]
    pub enum Hotkey {
        AutoType,
        QuickSearch,
    }

    pub fn listen(_: std::sync::mpsc::Sender<Request>, _: Hotkey) -> Result<u32, String> {
        Err(t!("autotype.windows_only").into())
    }

    pub fn stop(_: u32) {}

    pub fn type_keys(_: isize, _: &[Key], _: super::FieldCheck) -> Result<(), super::TypeError> {
        Err(super::TypeError::Failed(t!("autotype.windows_only").into()))
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
            collection_ids: Vec::new(),
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
                Key::Text(t) | Key::Password(t) => t.to_string(),
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
        // Whole host in the title.
        assert_eq!(names("bb.com.br - Login"), ["Banco"]);
        // "bb" is too short for a guess by the site's name.
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
        // "github" in the title: the item named GitHub (3) and the site github.com (2 — name only).
        let got: Vec<_> = candidates(&items, "GitHub Desktop")
            .into_iter()
            .map(|i| i.name.as_str())
            .collect();
        assert_eq!(got, ["GitHub"]);
    }

    /// Registers the real shortcut with Windows (without typing anything). `--ignored`.
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

    /// The real UI Automation call works (COM set up and released) and
    /// answers for whatever has focus while the tests run.
    #[cfg(windows)]
    #[test]
    fn ui_automation_answers_about_the_focused_field() {
        let start = std::time::Instant::now();
        let fields = super::platform::Fields::new();
        eprintln!("UI Automation criado em {:?}", start.elapsed());
        for _ in 0..3 {
            let start = std::time::Instant::now();
            let answer = fields.focused();
            eprintln!(
                "campo em foco (texto, senha): {answer:?} em {:?}",
                start.elapsed()
            );
        }
    }

    #[test]
    fn key_debug_hides_text() {
        let k = Key::Text(Zeroizing::new("segredo".into()));
        assert_eq!(format!("{k:?}"), "Text(7 chars)");
        let k = Key::Password(Zeroizing::new("segredo".into()));
        assert_eq!(format!("{k:?}"), "Password(7 chars)");
    }
}
