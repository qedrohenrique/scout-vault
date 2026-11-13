//! Preencher no navegador: a extensão do Scout Vault (pasta `extension/`) conversa
//! com o app pelo *native messaging* do Chrome/Edge.
//!
//! ```text
//! extensão ──stdin/stdout──▶ scout-vault.exe (ponte, iniciado pelo navegador)
//!                               │ pipe local, só deste usuário do Windows
//!                               ▼
//!                           Scout Vault aberto (responde se o cofre estiver desbloqueado)
//! ```
//!
//! Segurança:
//! - O navegador só inicia a ponte para a extensão com o ID registrado
//!   ([`EXTENSION_ID`], fixo pela chave pública no `manifest.json`).
//! - O pipe recusa conexões remotas e tem ACL só para o usuário atual.
//! - A lista mostra nomes e usuários; a **senha só sai** quando a pessoa clica
//!   num login, e só se o item tiver um endereço do mesmo site da aba.

use std::io::{BufRead, BufReader, Read, Write};

use scout_vault_core::vault::{Item, ItemKind};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

static SYSTEM: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Permite registrar no navegador e abrir o pipe (só o app real, nunca nos testes).
pub fn enable_system() {
    SYSTEM.store(true, std::sync::atomic::Ordering::Relaxed);
}

pub fn system_enabled() -> bool {
    SYSTEM.load(std::sync::atomic::Ordering::Relaxed)
}

/// Nome do host de native messaging (precisa bater com `popup.js`).
pub const HOST_NAME: &str = "com.scoutvault.desktop";
/// ID da extensão, derivado da chave pública em `extension/manifest.json`.
pub const EXTENSION_ID: &str = "emnfjpfkonpjndbkkhamknedgdljjnoe";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Request {
    Status,
    List { url: String },
    Fill { id: String, url: String },
}

/// Pedido vindo do navegador + onde responder.
pub struct Incoming {
    pub request: Request,
    pub reply: std::sync::mpsc::Sender<Value>,
}

// ---------------------------------------------------------------------------
// Qual login vale para qual site

/// Domínios de segundo nível comuns (`.com.br`, `.co.uk`…): o "site" é o rótulo antes deles.
const MULTI_SUFFIXES: &[&str] = &[
    "com.br", "net.br", "org.br", "gov.br", "edu.br", "co.uk", "org.uk", "ac.uk", "gov.uk",
    "com.au", "net.au", "org.au", "co.jp", "co.nz", "com.mx", "com.ar", "co.in", "com.pt",
];

/// Host de uma URL, em minúsculas e sem `www.`. Sem esquema, assume https.
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

/// Domínio registrável aproximado (`login.github.com` → `github.com`).
pub fn base_domain(host: &str) -> String {
    if host.parse::<std::net::IpAddr>().is_ok() {
        return host.to_owned();
    }
    let labels: Vec<&str> = host.split('.').collect();
    let take = if labels.len() >= 3
        && MULTI_SUFFIXES.contains(&labels[labels.len() - 2..].join(".").as_str())
    {
        3
    } else {
        2
    };
    labels[labels.len().saturating_sub(take)..].join(".")
}

/// O endereço salvo no item vale para a página? (regra "domínio base", o padrão do Bitwarden)
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

/// Responde um pedido. `items` é `None` com o cofre bloqueado.
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
            Some(item) => match login_for(item, url) {
                Some(l) => json!({
                    "username": l.username,
                    "password": l.password.as_ref().map(|p| p.expose()),
                }),
                None => json!({ "error": "Este login não é deste site." }),
            },
            None => json!({ "error": "Login não encontrado." }),
        },
    }
}

// ---------------------------------------------------------------------------
// Pipe local entre a ponte e o app

pub fn pipe_name() -> String {
    let user: String = std::env::var("USERNAME")
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    format!(r"\\.\pipe\scout-vault-navegador-{user}")
}

/// Uma troca no pipe: uma linha de JSON de cada lado.
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

/// Lado da ponte: manda um pedido ao app aberto.
pub fn ask_app(request: &Value) -> Value {
    let name = pipe_name();
    for _ in 0..20 {
        match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&name)
        {
            Ok(mut pipe) => {
                return exchange(&mut pipe, request)
                    .unwrap_or_else(|_| json!({ "error": "O Scout Vault não respondeu." }));
            }
            // Ocupado com outra conexão: tenta de novo.
            Err(e) if e.raw_os_error() == Some(231) => {
                std::thread::sleep(std::time::Duration::from_millis(50))
            }
            Err(_) => return json!({ "error": "closed" }),
        }
    }
    json!({ "error": "O Scout Vault está ocupado." })
}

/// Lê uma mensagem do navegador (4 bytes de tamanho + JSON).
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

/// O navegador iniciou o `scout-vault.exe` como ponte? (`chrome-extension://…` no argumento)
pub fn is_native_host_launch(args: &[String]) -> bool {
    args.iter()
        .skip(1)
        .any(|a| a.starts_with("chrome-extension://"))
}

/// Laço da ponte: repassa cada mensagem do navegador ao app e devolve a resposta.
pub fn run_native_host(
    input: &mut impl Read,
    output: &mut impl Write,
    ask: impl Fn(&Value) -> Value,
) {
    while let Some(message) = read_message(input) {
        let response = if serde_json::from_value::<Request>(message.clone()).is_ok() {
            ask(&message)
        } else {
            json!({ "error": "Pedido desconhecido." })
        };
        if write_message(output, &response).is_err() {
            break;
        }
    }
}

// ---------------------------------------------------------------------------
// Servidor no app (Windows)

#[cfg(windows)]
mod server {
    use std::sync::mpsc::Sender;

    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::windows::named_pipe::ServerOptions;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL};
    use windows::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        SDDL_REVISION_1,
    };
    use windows::Win32::Security::{
        GetTokenInformation, TokenUser, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY,
        TOKEN_USER,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    use super::{Incoming, Request};

    /// SID do usuário atual em texto (`S-1-5-21-…`).
    fn current_user_sid() -> Option<String> {
        // SAFETY: chamadas de consulta do próprio processo; buffers do tamanho pedido pela API.
        unsafe {
            let mut token = HANDLE::default();
            OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).ok()?;
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

    /// Descritor de segurança: acesso total só para o usuário atual.
    fn user_only_descriptor() -> Option<PSECURITY_DESCRIPTOR> {
        let sddl = format!("D:P(A;;GA;;;{})", current_user_sid()?);
        let wide: Vec<u16> = sddl.encode_utf16().chain(Some(0)).collect();
        let mut sd = PSECURITY_DESCRIPTOR::default();
        // SAFETY: string terminada em zero; o descritor vive enquanto o app roda.
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

    pub async fn serve(name: String, tx: Sender<Incoming>) -> std::io::Result<()> {
        let sd = user_only_descriptor()
            .ok_or_else(|| std::io::Error::other("não foi possível proteger o pipe"))?;
        // O descritor vive até o app fechar; guardado como número para o
        // futuro poder trocar de thread.
        let sd_addr = sd.0 as usize;
        let create = |first: bool| {
            let mut attrs = SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: sd_addr as *mut std::ffi::c_void,
                bInheritHandle: false.into(),
            };
            // SAFETY: `attrs` aponta para um descritor válido durante a chamada.
            unsafe {
                ServerOptions::new()
                    .first_pipe_instance(first)
                    .reject_remote_clients(true)
                    .create_with_security_attributes_raw(
                        &name,
                        (&mut attrs as *mut SECURITY_ATTRIBUTES).cast(),
                    )
            }
        };
        // `first_pipe_instance`: se outro processo já criou um pipe com esse nome, falha.
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
                        .unwrap_or_else(
                            || serde_json::json!({ "error": "O Scout Vault não respondeu." }),
                        )
                    }
                    Err(_) => serde_json::json!({ "error": "Pedido desconhecido." }),
                };
                let mut bytes = serde_json::to_vec(&response).unwrap_or_default();
                bytes.push(b'\n');
                let _ = write.write_all(&bytes).await;
                let _ = write.flush().await;
            });
        }
    }
}

/// Liga o servidor do pipe (só Windows). Soltar o valor devolvido desliga.
pub fn start_server(tx: std::sync::mpsc::Sender<Incoming>) -> Option<crate::io::Background> {
    #[cfg(windows)]
    {
        Some(crate::io::spawn(async move {
            if let Err(e) = server::serve(pipe_name(), tx).await {
                eprintln!("servidor do navegador parou: {e}");
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
// Registro no navegador

/// Manifesto do host que o navegador lê para iniciar a ponte.
pub fn host_manifest(exe: &std::path::Path) -> Value {
    json!({
        "name": HOST_NAME,
        "description": "Scout Vault — preencher logins",
        "path": exe.to_string_lossy(),
        "type": "stdio",
        "allowed_origins": [format!("chrome-extension://{EXTENSION_ID}/")],
    })
}

/// Chaves do Registro (HKCU) em que cada navegador procura o host.
#[cfg(windows)]
const REGISTRY_KEYS: &[&str] = &[
    r"Software\Google\Chrome\NativeMessagingHosts\com.scoutvault.desktop",
    r"Software\Microsoft\Edge\NativeMessagingHosts\com.scoutvault.desktop",
    r"Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\com.scoutvault.desktop",
];

/// Registros do nome antigo do app ("Cofre"), apagados ao registrar ou desfazer.
#[cfg(windows)]
const OLD_REGISTRY_KEYS: &[&str] = &[
    r"Software\Google\Chrome\NativeMessagingHosts\com.cofre.desktop",
    r"Software\Microsoft\Edge\NativeMessagingHosts\com.cofre.desktop",
    r"Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\com.cofre.desktop",
];

/// Grava o manifesto em `%APPDATA%\Scout Vault` e aponta os navegadores para ele
/// (só no usuário atual; não precisa de administrador).
pub fn register(manifest_dir: &std::path::Path) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(manifest_dir).map_err(|e| e.to_string())?;
    let path = manifest_dir.join("navegador-host.json");
    let text = serde_json::to_string_pretty(&host_manifest(&exe)).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    {
        use windows::core::HSTRING;
        use windows::Win32::System::Registry::{RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ};
        let value: Vec<u16> = path
            .to_string_lossy()
            .encode_utf16()
            .chain(Some(0))
            .collect();
        delete_keys(OLD_REGISTRY_KEYS);
        for key in REGISTRY_KEYS {
            // SAFETY: strings válidas; o valor é uma REG_SZ terminada em zero.
            let status = unsafe {
                RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    &HSTRING::from(*key),
                    None,
                    REG_SZ.0,
                    Some(value.as_ptr().cast()),
                    (value.len() * 2) as u32,
                )
            };
            if status.is_err() {
                return Err(format!(
                    "não foi possível registrar no navegador ({status:?})"
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
        // SAFETY: só apaga chaves do próprio Scout Vault.
        let _ = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(*key)) };
    }
}

/// Desfaz o [`register`].
pub fn unregister(manifest_dir: &std::path::Path) {
    let _ = std::fs::remove_file(manifest_dir.join("navegador-host.json"));
    #[cfg(windows)]
    {
        delete_keys(REGISTRY_KEYS);
        delete_keys(OLD_REGISTRY_KEYS);
    }
}

/// Pasta da extensão para "Carregar sem compactação": ao lado do executável
/// (instalação) ou a do repositório (desenvolvimento).
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
        // Pedir a senha do GitLab numa página do GitHub é recusado.
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

    /// Pipe de verdade (só local, sem mexer em nada do sistema): servidor + ponte.
    #[cfg(windows)]
    #[test]
    fn pipe_round_trip() {
        let name = format!(r"\\.\pipe\scout-vault-teste-{}", std::process::id());
        let (tx, rx) = std::sync::mpsc::channel::<Incoming>();
        let pipe = name.clone();
        let _server = crate::io::spawn(async move {
            let _ = server::serve(pipe, tx).await;
        });
        // "App": responde o que chegar.
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
        // Um segundo processo não consegue criar outro servidor com o mesmo nome.
        let again = tokio::runtime::Runtime::new().unwrap().block_on(async {
            tokio::net::windows::named_pipe::ServerOptions::new()
                .first_pipe_instance(true)
                .create(&name)
        });
        assert!(again.is_err());
    }
}
