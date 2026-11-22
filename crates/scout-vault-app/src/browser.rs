//! Browser autofill: the Scout Vault extension (`extension/` folder) talks
//! to the app through Chrome/Edge *native messaging*.
//!
//! ```text
//! extension ──stdin/stdout──▶ scout-vault.exe (bridge, started by the browser)
//!                               │ local pipe, this Windows user only
//!                               ▼
//!                           running Scout Vault (answers if the vault is unlocked)
//! ```
//!
//! Security:
//! - The browser only starts the bridge for the extension with the registered ID
//!   ([`EXTENSION_ID`], fixed by the public key in `manifest.json`).
//! - The pipe refuses remote connections and has an ACL for the current user only.
//! - The list shows names and usernames; the **password only leaves** when the person clicks
//!   a login, and only if the item has an address on the same site as the tab.
//! - **Saving logins:** when a login form is submitted on a site, the
//!   extension asks the app (`check`) whether it is new or a new password;
//!   the answer carries no password at all. Only after the person's "Save"/"Update"
//!   does the extension send `save`, and the app decides again what to do.

use std::io::{BufRead, BufReader, Read, Write};

use scout_vault_core::vault::{Item, ItemKind};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

static SYSTEM: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Allows registering with the browser and opening the pipe (real app only, never in tests).
pub fn enable_system() {
    SYSTEM.store(true, std::sync::atomic::Ordering::Relaxed);
}

pub fn system_enabled() -> bool {
    SYSTEM.load(std::sync::atomic::Ordering::Relaxed)
}

/// Native messaging host name (must match `popup.js`).
pub const HOST_NAME: &str = "com.scoutvault.desktop";
/// Extension ID, derived from the public key in `extension/manifest.json`.
pub const EXTENSION_ID: &str = "emnfjpfkonpjndbkkhamknedgdljjnoe";
/// The Firefox extension's id (`browser_specific_settings.gecko.id`, set by
/// `tools/package_extension.py`).
pub const FIREFOX_EXTENSION_ID: &str = "scout-vault@qedrohenrique.github.io";
/// The same extension published in the Chrome Web Store / Edge Add-ons gets an id of
/// its own from the store: add it here once published, so the bridge accepts it.
pub const STORE_EXTENSION_IDS: &[&str] = &[];

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Request {
    Status,
    List {
        url: String,
    },
    Fill {
        id: String,
        url: String,
    },
    /// A login was submitted on a site: is it new, a new password or already saved?
    Check {
        url: String,
        username: String,
        password: String,
    },
    /// The person confirmed: save the login (or update its password).
    Save {
        url: String,
        username: String,
        password: String,
    },
    /// Cards and identities to choose from (names only, no numbers or codes).
    Wallet,
    /// A card's data, after the person clicked it.
    #[serde(rename = "fillcard")]
    FillCard {
        id: String,
    },
    /// An identity's data, after the person clicked it.
    #[serde(rename = "fillidentity")]
    FillIdentity {
        id: String,
    },
}

/// What to do with a login submitted on a site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveDecision {
    /// Already saved like this (or can't be saved).
    Nothing,
    New,
    /// Same username on this site with another password.
    Update {
        id: String,
        name: String,
    },
}

/// Decides between save, update or nothing, looking at the site's **editable** logins.
pub fn decide_save(items: &[Item], url: &str, username: &str, password: &str) -> SaveDecision {
    if password.is_empty() || host_of(url).is_none() {
        return SaveDecision::Nothing;
    }
    let user = username.trim().to_lowercase();
    let mut same_user: Option<&Item> = None;
    for item in items {
        let Some(login) = login_for(item, url) else {
            continue;
        };
        let saved_user = login
            .username
            .as_deref()
            .unwrap_or("")
            .trim()
            .to_lowercase();
        if saved_user != user {
            continue;
        }
        if login.password.as_ref().map(|p| p.expose()) == Some(password) {
            return SaveDecision::Nothing;
        }
        same_user.get_or_insert(item);
    }
    match same_user {
        Some(item) => SaveDecision::Update {
            id: item.id.clone(),
            name: item.name.clone(),
        },
        None => SaveDecision::New,
    }
}

/// Address saved in the new login: scheme + host (no path or parameters).
fn origin_of(url: &str) -> String {
    let url = url.trim();
    let (scheme, rest) = url.split_once("://").unwrap_or(("https", url));
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    format!(
        "{}://{}",
        scheme.to_ascii_lowercase(),
        authority.to_ascii_lowercase()
    )
}

/// New login from what was submitted on the site.
pub fn new_login(url: &str, username: &str, password: &str) -> Item {
    let host = host_of(url).unwrap_or_default();
    let username = username.trim();
    Item {
        id: String::new(),
        organization_id: None,
        folder_id: None,
        name: host,
        notes: None,
        favorite: false,
        reprompt: false,
        deleted: false,
        revision_date: None,
        attachments: Vec::new(),
        collection_ids: Vec::new(),
        fields: Vec::new(),
        kind: ItemKind::Login(scout_vault_core::vault::Login {
            username: (!username.is_empty()).then(|| username.to_owned()),
            password: Some(scout_vault_core::Secret::new(password.to_owned())),
            totp: None,
            uris: vec![origin_of(url)],
        }),
    }
}

/// The same login with the new password (the history keeps the old one on save).
pub fn with_password(item: &Item, password: &str) -> Item {
    let mut item = item.clone();
    if let ItemKind::Login(login) = &mut item.kind {
        login.password = Some(scout_vault_core::Secret::new(password.to_owned()));
    }
    item
}

/// Request coming from the browser + where to reply.
pub struct Incoming {
    pub request: Request,
    pub reply: std::sync::mpsc::Sender<Value>,
}

// ---------------------------------------------------------------------------
// Which login applies to which site

/// Host of a URL, lowercase and without `www.`. Without a scheme, assumes https.
pub fn host_of(url: &str) -> Option<String> {
    let url = url.trim();
    let rest = match url.split_once("://") {
        Some((scheme, rest)) => {
            if !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https") {
                return None;
            }
            rest
        }
        None => url,
    };
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?;
    let host = host
        .split(':')
        .next()?
        .trim_end_matches('.')
        .to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_owned();
    (!host.is_empty() && host.contains('.') || host == "localhost").then_some(host)
}

/// Registrable domain (`login.github.com` → `github.com`), by the Public Suffix List
/// **with the private section**: `alice.github.io` and `bob.github.io` are different sites,
/// just like `loja.co.za` and `outra.co.za`. With no registrable domain (IP,
/// `localhost`, a public suffix alone), the whole host applies.
pub fn base_domain(host: &str) -> String {
    if host.parse::<std::net::IpAddr>().is_ok() {
        return host.to_owned();
    }
    psl::domain_str(host).unwrap_or(host).to_owned()
}

/// Does the address saved in the item apply to the page? ("base domain" rule, Bitwarden's default)
pub fn uri_matches(saved: &str, page_url: &str) -> bool {
    match (host_of(saved), host_of(page_url)) {
        (Some(a), Some(b)) => base_domain(&a) == base_domain(&b),
        _ => false,
    }
}

fn login_for<'a>(item: &'a Item, url: &str) -> Option<&'a scout_vault_core::vault::Login> {
    match &item.kind {
        ItemKind::Login(l) if !item.deleted && l.uris.iter().any(|u| uri_matches(u, url)) => {
            Some(l)
        }
        _ => None,
    }
}

/// Cards and identities the browser may see: not in the trash, and not asking for
/// the master password (those stay in the app, like logins).
fn wallet_item(item: &Item) -> bool {
    !item.deleted
        && !item.reprompt
        && matches!(item.kind, ItemKind::Card(_) | ItemKind::Identity(_))
}

/// The last four digits, for the list ("•••• 1234").
fn last_four(number: &str) -> String {
    let digits: Vec<char> = number.chars().filter(char::is_ascii_digit).collect();
    digits[digits.len().saturating_sub(4)..].iter().collect()
}

/// Answers a request. `items` is `None` with the vault locked.
pub fn handle(request: &Request, items: Option<&[Item]>) -> Value {
    let Some(items) = items else {
        return json!({ "locked": true });
    };
    match request {
        Request::Status => json!({ "locked": false }),
        Request::List { url } => {
            let mut found: Vec<&Item> = items
                .iter()
                .filter(|i| login_for(i, url).is_some())
                .collect();
            found.sort_by(|a, b| {
                b.favorite
                    .cmp(&a.favorite)
                    .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            });
            let list: Vec<Value> = found
                .iter()
                .map(|i| {
                    let user = login_for(i, url).and_then(|l| l.username.clone());
                    json!({ "id": i.id, "name": i.name, "username": user })
                })
                .collect();
            json!({ "locked": false, "items": list })
        }
        Request::Fill { id, url } => match items.iter().find(|i| &i.id == id) {
            Some(item) if item.reprompt => json!({
                "error": t!("browser.reprompt")
            }),
            Some(item) => match login_for(item, url) {
                Some(l) => json!({
                    "username": l.username,
                    "password": l.password.as_ref().map(|p| p.expose()),
                }),
                None => json!({ "error": t!("browser.wrong_site") }),
            },
            None => json!({ "error": t!("app.login_not_found") }),
        },
        Request::Check {
            url,
            username,
            password,
        } => match decide_save(items, url, username, password) {
            SaveDecision::Nothing => json!({ "action": "none" }),
            SaveDecision::New => json!({ "action": "save" }),
            SaveDecision::Update { name, .. } => json!({ "action": "update", "name": name }),
        },
        // Saving touches the vault: the app handles it (`ScoutVaultApp::handle_browser`).
        Request::Save { .. } => json!({ "error": t!("browser.unknown_request") }),
        Request::Wallet => {
            let mut cards = Vec::new();
            let mut identities = Vec::new();
            let mut found: Vec<&Item> = items.iter().filter(|i| wallet_item(i)).collect();
            found.sort_by(|a, b| {
                b.favorite
                    .cmp(&a.favorite)
                    .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            });
            for item in found {
                match &item.kind {
                    ItemKind::Card(c) => cards.push(json!({
                        "id": item.id,
                        "name": item.name,
                        "brand": c.brand,
                        "last4": c.number.as_ref().map(|n| last_four(n.expose())),
                    })),
                    ItemKind::Identity(i) => identities.push(json!({
                        "id": item.id,
                        "name": item.name,
                        "detail": i.email.clone().or_else(|| {
                            let name = [i.first_name.as_deref(), i.last_name.as_deref()]
                                .into_iter()
                                .flatten()
                                .collect::<Vec<_>>()
                                .join(" ");
                            (!name.is_empty()).then_some(name)
                        }),
                    })),
                    _ => {}
                }
            }
            json!({ "locked": false, "cards": cards, "identities": identities })
        }
        Request::FillCard { id } => match items.iter().find(|i| &i.id == id) {
            Some(item) if item.reprompt => json!({ "error": t!("browser.reprompt") }),
            Some(Item {
                kind: ItemKind::Card(c),
                deleted: false,
                ..
            }) => json!({
                "cardholderName": c.cardholder_name,
                "brand": c.brand,
                "number": c.number.as_ref().map(|n| n.expose()),
                "expMonth": c.exp_month,
                "expYear": c.exp_year,
                "code": c.code.as_ref().map(|n| n.expose()),
            }),
            _ => json!({ "error": t!("app.login_not_found") }),
        },
        Request::FillIdentity { id } => match items.iter().find(|i| &i.id == id) {
            Some(item) if item.reprompt => json!({ "error": t!("browser.reprompt") }),
            Some(Item {
                kind: ItemKind::Identity(i),
                deleted: false,
                ..
            }) => json!({
                "title": i.title,
                "firstName": i.first_name,
                "middleName": i.middle_name,
                "lastName": i.last_name,
                "company": i.company,
                "email": i.email,
                "phone": i.phone,
                "username": i.username,
                "address1": i.address1,
                "address2": i.address2,
                "address3": i.address3,
                "city": i.city,
                "state": i.state,
                "postalCode": i.postal_code,
                "country": i.country,
            }),
            _ => json!({ "error": t!("app.login_not_found") }),
        },
    }
}

// ---------------------------------------------------------------------------
// Local pipe between the bridge and the app

pub fn pipe_name() -> String {
    let user: String = std::env::var("USERNAME")
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    format!(r"\\.\pipe\scout-vault-navegador-{user}")
}

/// One exchange on the pipe: one line of JSON each way.
fn exchange(stream: &mut (impl Read + Write), request: &Value) -> std::io::Result<Value> {
    let mut line = serde_json::to_vec(request)?;
    line.push(b'\n');
    stream.write_all(&line)?;
    stream.flush()?;
    let mut reader = BufReader::new(stream);
    let mut response = String::new();
    reader.read_line(&mut response)?;
    serde_json::from_str(&response).map_err(std::io::Error::other)
}

/// Bridge side: sends a request to the running app.
pub fn ask_app(request: &Value) -> Value {
    let name = pipe_name();
    for _ in 0..20 {
        match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&name)
        {
            Ok(mut pipe) => {
                // The pipe name is predictable and the namespace is machine-wide:
                // another Windows user could create it before the app.
                // Only talk to a server running as this same user.
                #[cfg(windows)]
                if !ident::pipe_server_is_current_user(&pipe) {
                    return json!({ "error": "closed" });
                }
                return exchange(&mut pipe, request)
                    .unwrap_or_else(|_| json!({ "error": t!("browser.no_answer") }));
            }
            // Busy with another connection: try again.
            Err(e) if e.raw_os_error() == Some(231) => {
                std::thread::sleep(std::time::Duration::from_millis(50))
            }
            Err(_) => return json!({ "error": "closed" }),
        }
    }
    json!({ "error": t!("browser.busy") })
}

/// Reads a message from the browser (4-byte length + JSON).
fn read_message(input: &mut impl Read) -> Option<Value> {
    let mut len = [0u8; 4];
    input.read_exact(&mut len).ok()?;
    let len = u32::from_le_bytes(len) as usize;
    if len > 1024 * 1024 {
        return None;
    }
    let mut buf = vec![0u8; len];
    input.read_exact(&mut buf).ok()?;
    serde_json::from_slice(&buf).ok()
}

fn write_message(output: &mut impl Write, value: &Value) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(value)?;
    output.write_all(&(bytes.len() as u32).to_le_bytes())?;
    output.write_all(&bytes)?;
    output.flush()
}

/// Did the browser start `scout-vault.exe` as a bridge? (`chrome-extension://…` in the argument)
pub fn is_native_host_launch(args: &[String]) -> bool {
    // Chrome/Edge/Brave pass the extension's origin; Firefox passes the manifest's path
    // and the extension's id.
    args.iter()
        .skip(1)
        .any(|a| a.starts_with("chrome-extension://") || a == FIREFOX_EXTENSION_ID)
}

/// Bridge loop: forwards each browser message to the app and returns the answer.
pub fn run_native_host(
    input: &mut impl Read,
    output: &mut impl Write,
    ask: impl Fn(&Value) -> Value,
) {
    while let Some(message) = read_message(input) {
        let response = if serde_json::from_value::<Request>(message.clone()).is_ok() {
            ask(&message)
        } else {
            json!({ "error": t!("browser.unknown_request") })
        };
        if write_message(output, &response).is_err() {
            break;
        }
    }
}

// ---------------------------------------------------------------------------
// Who is who (Windows)

#[cfg(windows)]
mod ident {
    use windows::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL};
    use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
    use windows::Win32::System::Pipes::GetNamedPipeServerProcessId;
    use windows::Win32::System::Threading::{
        GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    /// SID (text, `S-1-5-21-…`) of the user that owns a process.
    fn process_user_sid(process: HANDLE) -> Option<String> {
        // SAFETY: queries only; buffers of the size the API asked for; handles closed.
        unsafe {
            let mut token = HANDLE::default();
            OpenProcessToken(process, TOKEN_QUERY, &mut token).ok()?;
            let mut len = 0u32;
            let _ = GetTokenInformation(token, TokenUser, None, 0, &mut len);
            let mut buf = vec![0u8; len as usize];
            let ok = GetTokenInformation(
                token,
                TokenUser,
                Some(buf.as_mut_ptr().cast()),
                len,
                &mut len,
            );
            let _ = CloseHandle(token);
            ok.ok()?;
            let user = &*(buf.as_ptr() as *const TOKEN_USER);
            let mut text = windows::core::PWSTR::null();
            ConvertSidToStringSidW(user.User.Sid, &mut text).ok()?;
            let sid = text.to_string().ok();
            let _ = windows::Win32::Foundation::LocalFree(Some(HLOCAL(text.0.cast())));
            sid
        }
    }

    /// SID of the current user.
    pub fn current_user_sid() -> Option<String> {
        // SAFETY: pseudo-handle of our own process; no need to close it.
        process_user_sid(unsafe { GetCurrentProcess() })
    }

    /// SID of the user running the server on the other side of a pipe (connected client).
    pub fn pipe_server_sid(pipe: &impl std::os::windows::io::AsRawHandle) -> Option<String> {
        // SAFETY: the pipe handle lives during the call; the process handle is closed here.
        unsafe {
            let mut pid = 0u32;
            GetNamedPipeServerProcessId(HANDLE(pipe.as_raw_handle()), &mut pid).ok()?;
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
            let sid = process_user_sid(process);
            let _ = CloseHandle(process);
            sid
        }
    }

    /// Does the pipe server run as `expected`? Any failure counts as "no".
    pub fn pipe_server_is(pipe: &impl std::os::windows::io::AsRawHandle, expected: &str) -> bool {
        pipe_server_sid(pipe).is_some_and(|sid| sid == expected)
    }

    pub fn pipe_server_is_current_user(pipe: &impl std::os::windows::io::AsRawHandle) -> bool {
        current_user_sid().is_some_and(|me| pipe_server_is(pipe, &me))
    }
}

// ---------------------------------------------------------------------------
// Server in the app (Windows)

/// Pipe server error not shown yet (the app reports it on the next loop iteration).
static SERVER_ERROR: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Takes (and clears) the pipe server error, if any.
pub fn take_server_error() -> Option<String> {
    SERVER_ERROR.lock().ok()?.take()
}

#[cfg(windows)]
mod server {
    use std::sync::mpsc::Sender;

    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::windows::named_pipe::ServerOptions;
    use windows::core::PCWSTR;
    use windows::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};

    use super::ident::current_user_sid;
    use super::{Incoming, Request};

    /// Security descriptor: full access for the current user only.
    fn user_only_descriptor() -> Option<PSECURITY_DESCRIPTOR> {
        let sddl = format!("D:P(A;;GA;;;{})", current_user_sid()?);
        let wide: Vec<u16> = sddl.encode_utf16().chain(Some(0)).collect();
        let mut sd = PSECURITY_DESCRIPTOR::default();
        // SAFETY: zero-terminated string; the descriptor lives while the app runs.
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(wide.as_ptr()),
                SDDL_REVISION_1,
                &mut sd,
                None,
            )
            .ok()?;
        }
        Some(sd)
    }

    /// A pipe instance only the current user can open (`first`: fail if another
    /// process already serves this name). Must run inside the Tokio runtime.
    pub fn create_user_pipe(
        name: &str,
        first: bool,
    ) -> std::io::Result<tokio::net::windows::named_pipe::NamedPipeServer> {
        let sd = user_only_descriptor()
            .ok_or_else(|| std::io::Error::other("não foi possível proteger o pipe"))?;
        let mut attrs = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd.0,
            bInheritHandle: false.into(),
        };
        // SAFETY: `attrs` points to a valid descriptor during the call. The descriptor
        // is small and lives until the app closes.
        unsafe {
            ServerOptions::new()
                .first_pipe_instance(first)
                .reject_remote_clients(true)
                .create_with_security_attributes_raw(
                    name,
                    (&mut attrs as *mut SECURITY_ATTRIBUTES).cast(),
                )
        }
    }

    pub async fn serve(name: String, tx: Sender<Incoming>) -> std::io::Result<()> {
        let create = |first: bool| create_user_pipe(&name, first);
        // `first_pipe_instance`: if another process already created a pipe with this name, fail.
        let mut server = create(true)?;
        loop {
            server.connect().await?;
            let connected = server;
            server = create(false)?;
            let tx = tx.clone();
            tokio::spawn(async move {
                let (read, mut write) = tokio::io::split(connected);
                let mut lines = BufReader::new(read).lines();
                let Ok(Some(line)) = lines.next_line().await else {
                    return;
                };
                let response = match serde_json::from_str::<Request>(&line) {
                    Ok(request) => {
                        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
                        if tx
                            .send(Incoming {
                                request,
                                reply: reply_tx,
                            })
                            .is_err()
                        {
                            return;
                        }
                        tokio::task::spawn_blocking(move || {
                            reply_rx
                                .recv_timeout(std::time::Duration::from_secs(10))
                                .ok()
                        })
                        .await
                        .ok()
                        .flatten()
                        .unwrap_or_else(|| serde_json::json!({ "error": t!("browser.no_answer") }))
                    }
                    Err(_) => serde_json::json!({ "error": t!("browser.unknown_request") }),
                };
                let mut bytes = serde_json::to_vec(&response).unwrap_or_default();
                bytes.push(b'\n');
                let _ = write.write_all(&bytes).await;
                let _ = write.flush().await;
            });
        }
    }
}

#[cfg(windows)]
pub(crate) use server::create_user_pipe;

/// Starts the pipe server (Windows only). Dropping the returned value turns it off.
pub fn start_server(tx: std::sync::mpsc::Sender<Incoming>) -> Option<crate::io::Background> {
    #[cfg(windows)]
    {
        Some(crate::io::spawn(async move {
            if let Err(e) = server::serve(pipe_name(), tx).await {
                eprintln!("servidor do navegador parou: {e}");
                // Pipe already exists (`first_pipe_instance`): another program, maybe from
                // another user, took the name. Report it instead of failing silently.
                let message = if e.raw_os_error() == Some(5) || e.raw_os_error() == Some(231) {
                    "Outro programa está ocupando o canal do navegador. Por segurança, \
                     o preenchimento no navegador ficou desligado até o Scout Vault reiniciar."
                        .to_owned()
                } else {
                    format!("O preenchimento no navegador parou: {e}")
                };
                if let Ok(mut slot) = SERVER_ERROR.lock() {
                    *slot = Some(message);
                }
            }
        }))
    }
    #[cfg(not(windows))]
    {
        let _ = tx;
        None
    }
}

// ---------------------------------------------------------------------------
// Browser registration

/// Host manifest the browser reads to start the bridge.
pub fn host_manifest(exe: &std::path::Path) -> Value {
    let origins: Vec<String> = std::iter::once(EXTENSION_ID)
        .chain(STORE_EXTENSION_IDS.iter().copied())
        .map(|id| format!("chrome-extension://{id}/"))
        .collect();
    json!({
        "name": HOST_NAME,
        "description": "Scout Vault — preencher logins",
        "path": exe.to_string_lossy(),
        "type": "stdio",
        "allowed_origins": origins,
    })
}

/// Firefox's version of the host manifest (it lists extension ids, not origins).
pub fn firefox_host_manifest(exe: &std::path::Path) -> Value {
    json!({
        "name": HOST_NAME,
        "description": "Scout Vault — preencher logins",
        "path": exe.to_string_lossy(),
        "type": "stdio",
        "allowed_extensions": [FIREFOX_EXTENSION_ID],
    })
}

/// Where Firefox looks for the host (its manifest is a separate file).
#[cfg(windows)]
const FIREFOX_REGISTRY_KEY: &str = r"Software\Mozilla\NativeMessagingHosts\com.scoutvault.desktop";
const FIREFOX_MANIFEST: &str = "navegador-host-firefox.json";

/// Registry keys (HKCU) where each browser looks for the host.
#[cfg(windows)]
const REGISTRY_KEYS: &[&str] = &[
    r"Software\Google\Chrome\NativeMessagingHosts\com.scoutvault.desktop",
    r"Software\Microsoft\Edge\NativeMessagingHosts\com.scoutvault.desktop",
    r"Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\com.scoutvault.desktop",
];

/// Registrations under the app's old name ("Cofre"), deleted when registering or undoing.
#[cfg(windows)]
const OLD_REGISTRY_KEYS: &[&str] = &[
    r"Software\Google\Chrome\NativeMessagingHosts\com.cofre.desktop",
    r"Software\Microsoft\Edge\NativeMessagingHosts\com.cofre.desktop",
    r"Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\com.cofre.desktop",
];

/// Writes the manifest to `%APPDATA%\Scout Vault` and points the browsers to it
/// (current user only; no administrator needed).
pub fn register(manifest_dir: &std::path::Path) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(manifest_dir).map_err(|e| e.to_string())?;
    let path = manifest_dir.join("navegador-host.json");
    let text = serde_json::to_string_pretty(&host_manifest(&exe)).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| e.to_string())?;
    let firefox_path = manifest_dir.join(FIREFOX_MANIFEST);
    let text =
        serde_json::to_string_pretty(&firefox_host_manifest(&exe)).map_err(|e| e.to_string())?;
    std::fs::write(&firefox_path, text).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    {
        use windows::core::HSTRING;
        use windows::Win32::System::Registry::{RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ};
        let wide = |p: &std::path::Path| -> Vec<u16> {
            p.to_string_lossy().encode_utf16().chain(Some(0)).collect()
        };
        let value = wide(&path);
        let firefox_value = wide(&firefox_path);
        delete_keys(OLD_REGISTRY_KEYS);
        let keys = REGISTRY_KEYS
            .iter()
            .map(|k| (*k, &value))
            .chain(std::iter::once((FIREFOX_REGISTRY_KEY, &firefox_value)));
        for (key, value) in keys {
            // SAFETY: valid strings; the value is a zero-terminated REG_SZ.
            let status = unsafe {
                RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    &HSTRING::from(key),
                    None,
                    REG_SZ.0,
                    Some(value.as_ptr().cast()),
                    (value.len() * 2) as u32,
                )
            };
            if status.is_err() {
                return Err(t!(
                    "browser.register_failed",
                    status = format!("{status:?}")
                ));
            }
        }
    }
    Ok(())
}

#[cfg(windows)]
fn delete_keys(keys: &[&str]) {
    use windows::core::HSTRING;
    use windows::Win32::System::Registry::{RegDeleteTreeW, HKEY_CURRENT_USER};
    for key in keys {
        // SAFETY: only deletes Scout Vault's own keys.
        let _ = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(*key)) };
    }
}

/// Undoes [`register`].
pub fn unregister(manifest_dir: &std::path::Path) {
    let _ = std::fs::remove_file(manifest_dir.join("navegador-host.json"));
    let _ = std::fs::remove_file(manifest_dir.join(FIREFOX_MANIFEST));
    #[cfg(windows)]
    {
        delete_keys(REGISTRY_KEYS);
        delete_keys(&[FIREFOX_REGISTRY_KEY]);
        delete_keys(OLD_REGISTRY_KEYS);
    }
}

/// Extension folder for "Load unpacked": next to the executable
/// (installation) or the repository's (development).
pub fn extension_dir() -> std::path::PathBuf {
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|p| p.join("extension")))
        .filter(|d| d.join("manifest.json").is_file())
    {
        return dir;
    }
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../extension")
}

#[cfg(test)]
mod tests {
    use super::*;
    use scout_vault_core::vault::Login;
    use scout_vault_core::Secret;

    fn login(id: &str, uris: &[&str]) -> Item {
        Item {
            id: id.into(),
            organization_id: None,
            folder_id: None,
            name: format!("Item {id}"),
            notes: None,
            favorite: id == "fav",
            reprompt: false,
            deleted: false,
            revision_date: None,
            attachments: Vec::new(),
            collection_ids: Vec::new(),
            fields: Vec::new(),
            kind: ItemKind::Login(Login {
                username: Some(format!("user-{id}")),
                password: Some(Secret::new(format!("senha-{id}"))),
                totp: None,
                uris: uris.iter().map(|u| u.to_string()).collect(),
            }),
        }
    }

    #[test]
    fn save_decision_new_update_or_nothing() {
        let items = vec![
            login("a", &["https://github.com"]),
            login("b", &["https://gitlab.com"]),
        ];
        let url = "https://github.com/session";
        // Already saved the same (username ignoring case/spaces).
        assert_eq!(
            decide_save(&items, url, " USER-A ", "senha-a"),
            SaveDecision::Nothing
        );
        // Same username, new password: update.
        assert_eq!(
            decide_save(&items, url, "user-a", "nova"),
            SaveDecision::Update {
                id: "a".into(),
                name: "Item a".into()
            }
        );
        // Another username on the same site, or a site with no saved login: new.
        assert_eq!(decide_save(&items, url, "outra", "x"), SaveDecision::New);
        assert_eq!(
            decide_save(&items, "https://example.org", "user-a", "senha-a"),
            SaveDecision::New
        );
        // No password or no site: nothing.
        assert_eq!(
            decide_save(&items, url, "user-a", ""),
            SaveDecision::Nothing
        );
        assert_eq!(
            decide_save(&items, "javascript:alert(1)", "u", "p"),
            SaveDecision::Nothing
        );
        // The `check` answer never carries a password.
        let r = handle(
            &Request::Check {
                url: url.into(),
                username: "user-a".into(),
                password: "nova".into(),
            },
            Some(&items),
        );
        assert_eq!(r, json!({ "action": "update", "name": "Item a" }));
        assert_eq!(
            handle(
                &Request::Check {
                    url: url.into(),
                    username: "u".into(),
                    password: "p".into()
                },
                None
            ),
            json!({ "locked": true })
        );
    }

    #[test]
    fn new_logins_keep_only_the_site_origin() {
        let item = new_login(
            "https://Login.Example.com:8443/entrar?next=/x#y",
            " eu ",
            "pw",
        );
        assert_eq!(item.name, "login.example.com");
        let ItemKind::Login(l) = &item.kind else {
            panic!()
        };
        assert_eq!(l.username.as_deref(), Some("eu"));
        assert_eq!(l.password.as_ref().unwrap().expose(), "pw");
        assert_eq!(l.uris, ["https://login.example.com:8443"]);
        let no_user = new_login("http://intranet.local/", "", "pw");
        let ItemKind::Login(l) = &no_user.kind else {
            panic!()
        };
        assert_eq!(l.username, None);
        assert_eq!(l.uris, ["http://intranet.local"]);
        let updated = with_password(&login("a", &["https://github.com"]), "nova");
        let ItemKind::Login(l) = &updated.kind else {
            panic!()
        };
        assert_eq!(l.password.as_ref().unwrap().expose(), "nova");
        assert_eq!(updated.id, "a");
    }

    #[test]
    fn bridge_accepts_check_and_save() {
        for message in [
            json!({ "type": "check", "url": "https://a.com", "username": "u", "password": "p" }),
            json!({ "type": "save", "url": "https://a.com", "username": "u", "password": "p" }),
        ] {
            assert!(serde_json::from_value::<Request>(message).is_ok());
        }
    }

    #[test]
    fn hosts_and_base_domains() {
        assert_eq!(
            host_of("https://www.GitHub.com/login?x=1").as_deref(),
            Some("github.com")
        );
        assert_eq!(host_of("github.com").as_deref(), Some("github.com"));
        assert_eq!(
            host_of("https://user:pw@login.bb.com.br:8443/").as_deref(),
            Some("login.bb.com.br")
        );
        assert_eq!(host_of("javascript:alert(1)"), None);
        assert_eq!(host_of("androidapp://com.x"), None);
        assert_eq!(base_domain("login.github.com"), "github.com");
        assert_eq!(base_domain("internet.bb.com.br"), "bb.com.br");
        assert_eq!(base_domain("192.168.0.10"), "192.168.0.10");
        assert!(uri_matches(
            "https://github.com",
            "https://gist.github.com/x"
        ));
        assert!(!uri_matches(
            "https://github.com",
            "https://github.com.evil.io/"
        ));
        assert!(!uri_matches("https://bb.com.br", "https://itau.com.br"));
        // Public suffixes outside a short list and private suffixes (shared
        // hosting): the neighboring site doesn't get the password.
        for (saved, page) in [
            ("https://meubanco.com.co/login", "https://atacante.com.co/"),
            ("https://loja.co.za/", "https://atacante.co.za/"),
            ("https://servico.gob.mx/", "https://atacante.gob.mx/"),
            ("https://minhaempresa.adv.br/", "https://atacante.adv.br/"),
            ("https://alice.github.io/", "https://atacante.github.io/"),
            (
                "https://meuapp.herokuapp.com/",
                "https://atacante.herokuapp.com/",
            ),
            ("https://meuapp.vercel.app/", "https://atacante.vercel.app/"),
            (
                "https://intranet.azurewebsites.net/",
                "https://atacante.azurewebsites.net/",
            ),
        ] {
            assert!(!uri_matches(saved, page), "{saved} não vale em {page}");
        }
        // …but subdomains of the same site still apply.
        assert!(uri_matches(
            "https://alice.github.io/",
            "https://docs.alice.github.io/"
        ));
        assert!(uri_matches(
            "https://bb.com.br",
            "https://internet.bb.com.br/"
        ));
        assert!(uri_matches(
            "http://localhost:8000",
            "http://localhost:3000/"
        ));
        assert!(uri_matches(
            "http://intranet.local",
            "http://app.intranet.local/"
        ));
    }

    #[test]
    fn list_and_fill_only_for_the_right_site() {
        let items = vec![
            login("a", &["https://github.com"]),
            login("fav", &["https://login.github.com/session"]),
            login("b", &["https://gitlab.com"]),
        ];
        let list = handle(
            &Request::List {
                url: "https://github.com/login".into(),
            },
            Some(&items),
        );
        let ids: Vec<&str> = list["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["fav", "a"], "favoritos primeiro");
        assert!(
            list.to_string().find("senha").is_none(),
            "a lista nunca leva senhas"
        );

        let ok = handle(
            &Request::Fill {
                id: "a".into(),
                url: "https://github.com/login".into(),
            },
            Some(&items),
        );
        assert_eq!(ok["password"], "senha-a");
        // Asking for the GitLab password on a GitHub page is refused.
        let wrong = handle(
            &Request::Fill {
                id: "b".into(),
                url: "https://github.com/login".into(),
            },
            Some(&items),
        );
        assert!(wrong.get("password").is_none());
        assert!(wrong["error"].is_string());

        assert_eq!(handle(&Request::Status, None), json!({ "locked": true }));
        assert_eq!(
            handle(
                &Request::List {
                    url: "https://github.com".into()
                },
                None
            )["locked"],
            true
        );
    }

    #[test]
    fn cards_and_identities_list_without_secrets_and_fill_on_request() {
        use scout_vault_core::vault::{Card, Identity};
        let mut card = login("card", &[]);
        card.name = "Visa".into();
        card.kind = ItemKind::Card(Card {
            cardholder_name: Some("Ana Souza".into()),
            brand: Some("Visa".into()),
            number: Some(Secret::new("4111 1111 1111 1234".into())),
            exp_month: Some("7".into()),
            exp_year: Some("2030".into()),
            code: Some(Secret::new("321".into())),
        });
        let mut guarded = card.clone();
        guarded.id = "guarded".into();
        guarded.reprompt = true;
        let mut identity = login("me", &[]);
        identity.name = "Eu".into();
        identity.kind = ItemKind::Identity(Box::new(Identity {
            first_name: Some("Ana".into()),
            email: Some("ana@example.com".into()),
            city: Some("Recife".into()),
            ..Default::default()
        }));
        let items = vec![card, guarded, identity, login("a", &["https://github.com"])];

        let wallet = handle(&Request::Wallet, Some(&items));
        let text = wallet.to_string();
        // Names and the last digits only: no number, no code, no reprompt card.
        assert!(!text.contains("4111") && !text.contains("321"));
        assert!(!text.contains("guarded"));
        assert_eq!(wallet["cards"][0]["last4"], "1234");
        assert_eq!(wallet["identities"][0]["detail"], "ana@example.com");

        let filled = handle(&Request::FillCard { id: "card".into() }, Some(&items));
        assert_eq!(filled["number"], "4111 1111 1111 1234");
        assert_eq!(filled["code"], "321");
        let refused = handle(
            &Request::FillCard {
                id: "guarded".into(),
            },
            Some(&items),
        );
        assert!(refused.get("number").is_none());
        // A login isn't a card.
        assert!(handle(&Request::FillCard { id: "a".into() }, Some(&items))["error"].is_string());
        let me = handle(&Request::FillIdentity { id: "me".into() }, Some(&items));
        assert_eq!(me["city"], "Recife");
        // Locked: nothing.
        assert_eq!(handle(&Request::Wallet, None), json!({ "locked": true }));
        // The wire names the extension uses.
        let parsed: Request =
            serde_json::from_value(json!({"type": "fillcard", "id": "x"})).unwrap();
        assert_eq!(parsed, Request::FillCard { id: "x".into() });
        let parsed: Request = serde_json::from_value(json!({"type": "wallet"})).unwrap();
        assert_eq!(parsed, Request::Wallet);
    }

    #[test]
    fn native_messaging_framing() {
        let mut input = Vec::new();
        for m in [json!({"type": "status"}), json!({"type": "hack"})] {
            let b = serde_json::to_vec(&m).unwrap();
            input.extend_from_slice(&(b.len() as u32).to_le_bytes());
            input.extend_from_slice(&b);
        }
        let mut output = Vec::new();
        run_native_host(&mut &input[..], &mut output, |_| json!({"locked": false}));
        let mut out = &output[..];
        assert_eq!(read_message(&mut out).unwrap(), json!({"locked": false}));
        assert!(read_message(&mut out).unwrap()["error"].is_string());
        assert!(read_message(&mut out).is_none());
    }

    #[test]
    fn manifest_points_to_this_extension_only() {
        let m = host_manifest(std::path::Path::new(r"C:\Scout Vault\scout-vault.exe"));
        assert_eq!(m["name"], HOST_NAME);
        assert_eq!(
            m["allowed_origins"],
            json!([format!("chrome-extension://{EXTENSION_ID}/")])
        );
        assert!(is_native_host_launch(&[
            "scout-vault.exe".into(),
            format!("chrome-extension://{EXTENSION_ID}/")
        ]));
        assert!(!is_native_host_launch(&["scout-vault.exe".into()]));
        // Firefox: the manifest's path and the extension's id.
        let ff = firefox_host_manifest(std::path::Path::new(r"C:\Scout Vault\scout-vault.exe"));
        assert_eq!(ff["allowed_extensions"], json!([FIREFOX_EXTENSION_ID]));
        assert!(ff.get("allowed_origins").is_none());
        assert!(is_native_host_launch(&[
            "scout-vault.exe".into(),
            r"C:\Users\x\AppData\Roaming\Scout Vault\navegador-host-firefox.json".into(),
            FIREFOX_EXTENSION_ID.into(),
        ]));
        assert!(!is_native_host_launch(&[
            "scout-vault.exe".into(),
            "other@extension".into()
        ]));
    }

    #[test]
    fn extension_manifest_key_matches_id() {
        use base64::Engine as _;
        use sha2::Digest as _;
        let manifest: Value = serde_json::from_str(
            &std::fs::read_to_string(extension_dir().join("manifest.json")).unwrap(),
        )
        .unwrap();
        let der = base64::engine::general_purpose::STANDARD
            .decode(manifest["key"].as_str().unwrap())
            .unwrap();
        let digest = sha2::Sha256::digest(&der);
        let id: String = digest[..16]
            .iter()
            .flat_map(|b| [b >> 4, b & 0xf])
            .map(|n| (b'a' + n) as char)
            .collect();
        assert_eq!(id, EXTENSION_ID);
    }

    /// Real pipe (local only, without touching anything in the system): server + bridge.
    #[cfg(windows)]
    #[test]
    fn pipe_round_trip() {
        let name = format!(r"\\.\pipe\scout-vault-teste-{}", std::process::id());
        let (tx, rx) = std::sync::mpsc::channel::<Incoming>();
        let pipe = name.clone();
        let _server = crate::io::spawn(async move {
            let _ = server::serve(pipe, tx).await;
        });
        // "App": answers whatever arrives.
        std::thread::spawn(move || {
            while let Ok(incoming) = rx.recv() {
                let items = vec![login("a", &["https://github.com"])];
                let _ = incoming.reply.send(handle(&incoming.request, Some(&items)));
            }
        });
        std::thread::sleep(std::time::Duration::from_millis(200));
        let mut pipe = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&name)
            .unwrap();
        let response = exchange(
            &mut pipe,
            &json!({"type": "fill", "id": "a", "url": "https://github.com/"}),
        )
        .unwrap();
        assert_eq!(response["username"], "user-a");
        // The bridge checks who runs the server: here it is this same user…
        let me = ident::current_user_sid().unwrap();
        let check = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&name)
            .unwrap();
        assert_eq!(ident::pipe_server_sid(&check).as_deref(), Some(me.as_str()));
        assert!(ident::pipe_server_is_current_user(&check));
        // …and a server from another user (here, SYSTEM) would be refused.
        assert!(!ident::pipe_server_is(&check, "S-1-5-18"));
        drop(check);
        // A second process can't create another server with the same name.
        let again = tokio::runtime::Runtime::new().unwrap().block_on(async {
            tokio::net::windows::named_pipe::ServerOptions::new()
                .first_pipe_instance(true)
                .create(&name)
        });
        assert!(again.is_err());
    }
}
