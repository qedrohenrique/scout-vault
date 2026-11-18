//! SSH agent: serves the open vault's SSH keys to `ssh`, `git` and friends on
//! Windows' OpenSSH pipe (`\\.\pipe\openssh-ssh-agent`), the way 1Password does.
//!
//! - Keys only while the vault is open: locking empties the list.
//! - Every signature is asked first ([`Approval`]): the app shows who wants which
//!   key; "Allow" may also ask for Windows Hello. "Remember" skips the question for
//!   that program and key for a few minutes (until the vault locks).
//! - The pipe only accepts the current user, and nothing can add, remove or export
//!   keys through it ([`scout_vault_core::ssh_agent`] refuses those requests).
//!
//! Windows' own "OpenSSH Authentication Agent" service uses the same pipe: while it
//! runs, Scout Vault can't serve it ([`Status::PipeBusy`]); the user has to stop
//! that service (the app doesn't change system services).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui_kit::{App, Global};
use scout_vault_core::ssh_agent::AgentKey;
use scout_vault_core::vault::{ItemKind, Vault};
use tokio::sync::{mpsc, oneshot};

/// The pipe Windows' OpenSSH (`ssh.exe`) looks for when `SSH_AUTH_SOCK` isn't set.
pub const PIPE: &str = r"\\.\pipe\openssh-ssh-agent";
/// How long "remember" lasts.
pub const REMEMBER_FOR: Duration = Duration::from_secs(10 * 60);
/// How long a request waits for an answer.
pub const ANSWER_WITHIN: Duration = Duration::from_secs(120);

/// The program asking for a key.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Client {
    /// `ssh.exe`
    pub program: String,
    /// Who started it (`git.exe`, `pwsh.exe`…), when Windows says.
    pub parent: Option<String>,
}

impl Client {
    /// "ssh.exe (git.exe)"
    pub fn describe(&self) -> String {
        let program = if self.program.is_empty() {
            t!("ssh_agent.unknown_program").to_owned()
        } else {
            self.program.clone()
        };
        match &self.parent {
            Some(parent) => format!("{program} ({parent})"),
            None => program,
        }
    }
}

/// A signature waiting for the user's answer.
pub struct Approval {
    pub item_id: String,
    pub key_name: String,
    pub fingerprint: String,
    pub client: Client,
    reply: oneshot::Sender<bool>,
}

impl Approval {
    /// Answers the program (`true` = sign).
    pub fn answer(self, allow: bool) {
        let _ = self.reply.send(allow);
    }

    /// The program gave up waiting (or disconnected).
    pub fn is_stale(&self) -> bool {
        self.reply.is_closed()
    }
}

/// How the agent is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Off,
    Running,
    /// Another agent (Windows' OpenSSH service) holds the pipe.
    PipeBusy,
    Failed(String),
}

type KeyList = Arc<Mutex<Vec<Arc<AgentKey>>>>;

struct Agent {
    pipe: String,
    requests: Option<mpsc::UnboundedSender<Approval>>,
    keys: KeyList,
    /// What the key list was built from (to rebuild only when it changes).
    keys_from: Option<u64>,
    server: Option<crate::io::Background>,
    status: Arc<Mutex<Status>>,
    /// (key fingerprint, program) → until when it is allowed without asking.
    remembered: HashMap<(String, Client), Instant>,
}

impl Global for Agent {}

fn agent(cx: &mut App) -> &mut Agent {
    if !cx.has_global::<Agent>() {
        cx.set_global(Agent {
            pipe: PIPE.to_owned(),
            requests: None,
            keys: KeyList::default(),
            keys_from: None,
            server: None,
            status: Arc::new(Mutex::new(Status::Off)),
            remembered: HashMap::new(),
        });
    }
    cx.global_mut::<Agent>()
}

/// Where signature requests go (the app's approval prompt), and on which pipe to
/// serve (tests use their own).
pub fn install(pipe: &str, requests: mpsc::UnboundedSender<Approval>, cx: &mut App) {
    let a = agent(cx);
    a.pipe = pipe.to_owned();
    a.requests = Some(requests);
}

/// Serves on another pipe (tests); takes effect the next time it is turned on.
pub fn set_pipe(pipe: &str, cx: &mut App) {
    agent(cx).pipe = pipe.to_owned();
}

pub fn status(cx: &App) -> Status {
    cx.try_global::<Agent>()
        .and_then(|a| a.status.lock().ok().map(|s| s.clone()))
        .unwrap_or(Status::Off)
}

/// Turns the agent on or off.
pub fn set_enabled(on: bool, cx: &mut App) {
    let a = agent(cx);
    a.server = None;
    set_status(&a.status, Status::Off);
    if on {
        if let Some(requests) = a.requests.clone() {
            let status = a.status.clone();
            a.server = start(a.pipe.clone(), a.keys.clone(), requests, status);
        }
    }
    cx.refresh_windows();
}

fn set_status(slot: &Mutex<Status>, status: Status) {
    if let Ok(mut s) = slot.lock() {
        *s = status;
    }
}

/// Fingerprint of the vault's SSH items, to notice changes.
fn keys_signature(vault: &Vault) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for item in &vault.items {
        if let (ItemKind::SshKey(k), false) = (&item.kind, item.deleted) {
            item.id.hash(&mut h);
            item.name.hash(&mut h);
            item.revision_date.hash(&mut h);
            k.fingerprint.hash(&mut h);
        }
    }
    h.finish()
}

/// The keys the agent offers: the open vault's SSH keys (`None` = locked, no keys).
pub fn set_keys(vault: Option<&Vault>, cx: &mut App) {
    let a = agent(cx);
    let from = vault.map(keys_signature);
    if from == a.keys_from {
        return;
    }
    a.keys_from = from;
    let keys: Vec<Arc<AgentKey>> = vault
        .map(|v| {
            v.items
                .iter()
                .filter(|i| !i.deleted)
                .filter_map(|i| match &i.kind {
                    ItemKind::SshKey(k) => {
                        let private = k.private_key.as_ref()?;
                        AgentKey::new(&i.id, &i.name, private.expose()).map(Arc::new)
                    }
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    if vault.is_none() {
        // Locked: what was remembered is forgotten too.
        a.remembered.clear();
    }
    if let Ok(mut list) = a.keys.lock() {
        *list = keys;
    }
}

/// How many keys the agent offers right now.
pub fn key_count(cx: &App) -> usize {
    cx.try_global::<Agent>()
        .and_then(|a| a.keys.lock().ok().map(|k| k.len()))
        .unwrap_or(0)
}

/// Was this program allowed to use this key a moment ago?
pub fn is_remembered(approval: &Approval, cx: &App) -> bool {
    cx.try_global::<Agent>().is_some_and(|a| {
        a.remembered
            .get(&(approval.fingerprint.clone(), approval.client.clone()))
            .is_some_and(|until| Instant::now() < *until)
    })
}

/// Lets this program use this key without asking, for [`REMEMBER_FOR`].
pub fn remember(approval: &Approval, cx: &mut App) {
    let until = Instant::now() + REMEMBER_FOR;
    agent(cx).remembered.insert(
        (approval.fingerprint.clone(), approval.client.clone()),
        until,
    );
}

/// Answers one request (pure: the server loop and the tests use it).
async fn respond(
    body: &[u8],
    keys: &KeyList,
    client: &Client,
    requests: &mpsc::UnboundedSender<Approval>,
) -> Vec<u8> {
    use scout_vault_core::ssh_agent::{self as proto, Request};
    match proto::parse(body) {
        Some(Request::Identities) => {
            let list = keys.lock().map(|k| k.clone()).unwrap_or_default();
            proto::identities(list.iter().map(|k| (k.blob.as_slice(), k.name.as_str())))
        }
        Some(Request::Sign { key, data, flags }) => {
            let found = keys
                .lock()
                .ok()
                .and_then(|list| list.iter().find(|k| k.blob == key).cloned());
            let Some(key) = found else {
                return proto::failure();
            };
            let (reply, answer) = oneshot::channel();
            let sent = requests.send(Approval {
                item_id: key.item_id.clone(),
                key_name: key.name.clone(),
                fingerprint: key.fingerprint.clone(),
                client: client.clone(),
                reply,
            });
            if sent.is_err() {
                return proto::failure();
            }
            match tokio::time::timeout(ANSWER_WITHIN, answer).await {
                Ok(Ok(true)) => key
                    .sign(&data, flags)
                    .map(|s| proto::signed(&s))
                    .unwrap_or_else(proto::failure),
                _ => proto::failure(),
            }
        }
        _ => proto::failure(),
    }
}

#[cfg(windows)]
fn start(
    pipe: String,
    keys: KeyList,
    requests: mpsc::UnboundedSender<Approval>,
    status: Arc<Mutex<Status>>,
) -> Option<crate::io::Background> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    // The first instance is created here, so a busy pipe shows up right away.
    let first = {
        let _guard = crate::io::enter();
        crate::browser::create_user_pipe(&pipe, true)
    };
    let mut server = match first {
        Ok(server) => server,
        Err(e) => {
            let busy = matches!(e.raw_os_error(), Some(5) | Some(231));
            set_status(
                &status,
                if busy {
                    Status::PipeBusy
                } else {
                    Status::Failed(e.to_string())
                },
            );
            return None;
        }
    };
    set_status(&status, Status::Running);
    Some(crate::io::spawn(async move {
        loop {
            if let Err(e) = server.connect().await {
                set_status(&status, Status::Failed(e.to_string()));
                return;
            }
            let mut connected = server;
            server = match crate::browser::create_user_pipe(&pipe, false) {
                Ok(next) => next,
                Err(e) => {
                    set_status(&status, Status::Failed(e.to_string()));
                    return;
                }
            };
            let client = platform::client_of(&connected);
            let (keys, requests) = (keys.clone(), requests.clone());
            tokio::spawn(async move {
                // One connection carries many requests, each with a length prefix.
                loop {
                    let mut len = [0u8; 4];
                    if connected.read_exact(&mut len).await.is_err() {
                        return;
                    }
                    let len = u32::from_be_bytes(len) as usize;
                    if len == 0 || len > scout_vault_core::ssh_agent::MAX_MESSAGE {
                        return;
                    }
                    let mut body = vec![0u8; len];
                    if connected.read_exact(&mut body).await.is_err() {
                        return;
                    }
                    let answer = respond(&body, &keys, &client, &requests).await;
                    let framed = scout_vault_core::ssh_agent::frame(&answer);
                    if connected.write_all(&framed).await.is_err() {
                        return;
                    }
                }
            });
        }
    }))
}

#[cfg(not(windows))]
fn start(
    _: String,
    _: KeyList,
    _: mpsc::UnboundedSender<Approval>,
    status: Arc<Mutex<Status>>,
) -> Option<crate::io::Background> {
    set_status(&status, Status::Failed("só no Windows por enquanto".into()));
    None
}

#[cfg(windows)]
mod platform {
    use std::os::windows::io::AsRawHandle;

    use windows::core::PWSTR;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::Pipes::GetNamedPipeClientProcessId;
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };

    use super::Client;

    fn file_name(path: &str) -> String {
        path.rsplit(['\\', '/']).next().unwrap_or(path).to_owned()
    }

    fn image_of(pid: u32) -> Option<String> {
        // SAFETY: only reads the name of a process's executable; the handle is closed.
        unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
            let mut buf = [0u16; 1024];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(
                process,
                PROCESS_NAME_WIN32,
                PWSTR(buf.as_mut_ptr()),
                &mut len,
            );
            let _ = CloseHandle(process);
            ok.ok()?;
            Some(file_name(&String::from_utf16_lossy(&buf[..len as usize])))
        }
    }

    /// The parent process's id and executable name, from a process snapshot.
    fn parent_of(pid: u32) -> Option<String> {
        // SAFETY: a snapshot of the process list, walked and closed.
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()?;
            let mut entry = PROCESSENTRY32W {
                dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
                ..Default::default()
            };
            let mut entries = Vec::new();
            if Process32FirstW(snap, &mut entry).is_ok() {
                loop {
                    let end = entry
                        .szExeFile
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(entry.szExeFile.len());
                    entries.push((
                        entry.th32ProcessID,
                        entry.th32ParentProcessID,
                        String::from_utf16_lossy(&entry.szExeFile[..end]),
                    ));
                    if Process32NextW(snap, &mut entry).is_err() {
                        break;
                    }
                }
            }
            let _ = CloseHandle(snap);
            let parent = entries.iter().find(|e| e.0 == pid)?.1;
            entries.iter().find(|e| e.0 == parent).map(|e| e.2.clone())
        }
    }

    pub fn client_of(pipe: &impl AsRawHandle) -> Client {
        let mut pid = 0u32;
        // SAFETY: asks Windows who is on the other end of our own pipe.
        let ok =
            unsafe { GetNamedPipeClientProcessId(HANDLE(pipe.as_raw_handle()), &mut pid).is_ok() };
        if !ok || pid == 0 {
            return Client::default();
        }
        Client {
            program: image_of(pid).unwrap_or_default(),
            parent: parent_of(pid),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{respond, Client, KeyList};
    use scout_vault_core::ssh_agent::AgentKey;
    use std::sync::Arc;

    fn keys() -> KeyList {
        let pem = scout_vault_core::ssh::generate().private_key;
        let key = AgentKey::new("item-1", "GitHub", &pem).expect("test key");
        Arc::new(std::sync::Mutex::new(vec![Arc::new(key)]))
    }

    fn sign_request(blob: &[u8]) -> Vec<u8> {
        let mut body = vec![13];
        body.extend_from_slice(&(blob.len() as u32).to_be_bytes());
        body.extend_from_slice(blob);
        body.extend_from_slice(&4u32.to_be_bytes());
        body.extend_from_slice(b"data");
        body.extend_from_slice(&0u32.to_be_bytes());
        body
    }

    fn run<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
            .block_on(f)
    }

    #[test]
    fn signing_waits_for_the_answer() {
        let keys = keys();
        let blob = keys.lock().unwrap()[0].blob.clone();
        let client = Client {
            program: "ssh.exe".into(),
            parent: Some("git.exe".into()),
        };
        assert_eq!(client.describe(), "ssh.exe (git.exe)");
        for allow in [true, false] {
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
            let expected = client.clone();
            let answer = run(async {
                let decide = tokio::spawn(async move {
                    let approval: super::Approval = rx.recv().await.unwrap();
                    assert_eq!(approval.key_name, "GitHub");
                    assert_eq!(approval.client, expected);
                    approval.answer(allow);
                });
                let answer = respond(&sign_request(&blob), &keys, &client, &tx).await;
                decide.await.unwrap();
                answer
            });
            assert_eq!(answer[0], if allow { 14 } else { 5 });
        }
    }

    #[test]
    fn unknown_keys_and_other_requests_fail_without_asking() {
        let keys = keys();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let client = Client::default();
        let answer = run(respond(&sign_request(b"other"), &keys, &client, &tx));
        assert_eq!(answer, [5]);
        // "Add a key", "remove all": refused.
        assert_eq!(run(respond(&[17], &keys, &client, &tx)), [5]);
        assert_eq!(run(respond(&[19], &keys, &client, &tx)), [5]);
        assert!(rx.try_recv().is_err());
        // The list has the key, with the item's name.
        let list = run(respond(&[11], &keys, &client, &tx));
        assert_eq!(&list[..5], [12, 0, 0, 0, 1]);
        assert!(list.ends_with(b"GitHub"));
    }
}
