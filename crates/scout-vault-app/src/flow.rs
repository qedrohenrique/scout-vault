//! Login, unlock and sync flows — without any UI.
//!
//! The screens only call these functions and show the result; that way the logic
//! (including the error messages) can be tested without a window.

use std::sync::Arc;

use scout_vault_core::api::{
    ApiError, Client, SecondFactor, TwoFactorChallenge, TwoFactorProvider,
};
use scout_vault_core::crypto::{CryptoError, MasterKey};
use scout_vault_core::enc_string::ParseError;
use scout_vault_core::sync::SyncResponse;
use scout_vault_core::vault::{Keys, Vault, VaultError};
use scout_vault_core::Kdf;
use zeroize::Zeroizing;

use crate::io;
use crate::secrets::{remember_key, SecretStore};

/// Authenticated session. Holds only **encrypted** data and tokens — it keeps
/// existing while the vault is locked.
#[derive(Clone)]
pub struct Session {
    pub client: Arc<Client>,
    pub email: String,
    pub kdf: Kdf,
    pub access_token: String,
    pub refresh_token: String,
    pub sync: Arc<SyncResponse>,
    /// When the last full login (email, password, 2FA) happened, in Unix seconds.
    pub logged_in_at: u64,
}

/// Now, in Unix seconds.
pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Open vault: session + keys + decrypted data.
pub struct Unlocked {
    pub session: Session,
    pub keys: Keys,
    pub vault: Vault,
}

/// Hands an open vault from one screen to another through an event.
/// Events arrive by reference; the `Cell` allows **moving** the keys only
/// once, without cloning them.
pub struct Handoff(std::cell::Cell<Option<Box<Unlocked>>>);

impl Handoff {
    pub fn new(unlocked: Box<Unlocked>) -> Self {
        Self(std::cell::Cell::new(Some(unlocked)))
    }

    pub fn take(&self) -> Option<Box<Unlocked>> {
        self.0.take()
    }
}

/// Login paused waiting for a second factor or the new device code.
/// Keeps the master key so the (slow) KDF isn't redone on every attempt.
pub struct PendingLogin {
    client: Arc<Client>,
    email: String,
    kdf: Kdf,
    master_key: Arc<MasterKey>,
    server_hash: Zeroizing<String>,
    /// Required by the official server to resend the 2FA code by email.
    email_session_token: Option<String>,
    /// 2FA methods the server offered (to go back to the step after a wrong code).
    providers: Vec<TwoFactorProvider>,
    /// Method used on the last attempt.
    last_provider: Option<TwoFactorProvider>,
    /// The last attempt was the new device code.
    new_device: bool,
    /// Where to store the "remember this device" token and under which key.
    secrets: SecretStore,
    remember_key: String,
    /// Data the server sent for each 2FA method.
    provider_data: Vec<(TwoFactorProvider, serde_json::Value)>,
}

impl PendingLogin {
    pub fn email(&self) -> &str {
        &self.email
    }

    pub fn providers(&self) -> &[TwoFactorProvider] {
        &self.providers
    }

    pub fn last_provider(&self) -> Option<TwoFactorProvider> {
        self.last_provider
    }

    pub fn is_new_device(&self) -> bool {
        self.new_device
    }

    /// Data the server sent for a 2FA method.
    pub fn provider_data(&self, provider: TwoFactorProvider) -> Option<&serde_json::Value> {
        self.provider_data
            .iter()
            .find(|(p, _)| *p == provider)
            .map(|(_, v)| v)
    }
}

pub enum LoginOutcome {
    Done(Box<Unlocked>),
    NeedsTwoFactor {
        pending: PendingLogin,
        providers: Vec<TwoFactorProvider>,
    },
    NeedsNewDeviceCode(PendingLogin),
    /// Wrong 2FA or new device code: stays on the same step, without
    /// redoing the KDF or asking for the password again.
    WrongCode {
        pending: PendingLogin,
        message: String,
    },
}

/// First stage: prelogin, KDF, login.
pub async fn start_login(
    client: Arc<Client>,
    email: String,
    password: Zeroizing<String>,
    secrets: SecretStore,
) -> Result<LoginOutcome, String> {
    let email = email.trim().to_owned();
    let kdf = {
        let client = client.clone();
        let email = email.clone();
        io::run(async move { client.prelogin(&email).await })
            .await
            .map_err(describe_api_error)?
    };

    let (master_key, server_hash) = {
        let email = email.clone();
        io::blocking(move || {
            let mk = MasterKey::derive(&password, &email, kdf)?;
            let hash = Zeroizing::new(mk.server_hash(&password));
            Ok::<_, CryptoError>((mk, hash))
        })
        .await
        .map_err(|e| t!("flow.kdf_failed", e = e))?
    };

    let pending_client_urls = client.urls().clone();
    let email_for_key = email.clone();
    let pending = PendingLogin {
        client,
        email,
        kdf,
        master_key: Arc::new(master_key),
        server_hash,
        email_session_token: None,
        providers: Vec::new(),
        last_provider: None,
        new_device: false,
        remember_key: remember_key(&client_identity(&pending_client_urls), &email_for_key),
        secrets,
        provider_data: Vec::new(),
    };
    // Remembered device: try to skip 2FA with the stored token.
    let remembered = pending
        .secrets
        .get(&pending.remember_key)
        .map(|token| SecondFactor {
            provider: TwoFactorProvider::Remember,
            token,
            remember: false,
        });
    continue_login(pending, remembered, None).await
}

/// Resends the login with the second factor and/or the new device code.
pub async fn continue_login(
    mut pending: PendingLogin,
    second_factor: Option<SecondFactor>,
    new_device_code: Option<String>,
) -> Result<LoginOutcome, String> {
    pending.last_provider = second_factor.as_ref().map(|s| s.provider);
    pending.new_device = new_device_code.is_some();
    let result = {
        let client = pending.client.clone();
        let email = pending.email.clone();
        let hash = pending.server_hash.clone();
        io::run(async move {
            client
                .login_password(
                    &email,
                    &hash,
                    second_factor.as_ref(),
                    new_device_code.as_deref(),
                )
                .await
        })
        .await
    };

    let tokens = match result {
        Ok(tokens) => tokens,
        Err(ApiError::TwoFactorRequired(TwoFactorChallenge {
            providers,
            email_session_token,
            provider_data,
        })) => {
            pending.provider_data = provider_data;
            if email_session_token.is_some() {
                pending.email_session_token = email_session_token;
            }
            // The remembered token expired or was revoked: forget it and ask for the code.
            if pending.last_provider == Some(TwoFactorProvider::Remember) {
                pending.secrets.delete(&pending.remember_key);
                pending.last_provider = None;
            }
            // The official server returns the 2FA request again when a "remembered"
            // token expires; we treat it like a new request.
            pending.providers = providers.clone();
            return Ok(LoginOutcome::NeedsTwoFactor { pending, providers });
        }
        Err(e @ (ApiError::InvalidTwoFactorCode(_) | ApiError::InvalidNewDeviceCode)) => {
            return Ok(LoginOutcome::WrongCode {
                pending,
                message: describe_api_error(e),
            });
        }
        Err(ApiError::NewDeviceVerificationRequired) => {
            return Ok(LoginOutcome::NeedsNewDeviceCode(pending))
        }
        Err(e) => return Err(describe_api_error(e)),
    };

    if let Some(token) = &tokens.two_factor_token {
        pending.secrets.set(&pending.remember_key, token);
    }
    finish_login(
        pending.client,
        pending.email,
        pending.kdf,
        pending.master_key,
        tokens,
    )
    .await
}

/// Final part of any login: downloads the vault and opens it with the master key.
async fn finish_login(
    client: Arc<Client>,
    email: String,
    kdf: Kdf,
    master_key: Arc<MasterKey>,
    tokens: scout_vault_core::api::Tokens,
) -> Result<LoginOutcome, String> {
    let sync = {
        let client = client.clone();
        let token = tokens.access_token.clone();
        io::run(async move { client.sync(&token).await })
            .await
            .map_err(describe_api_error)?
    };

    let mut sync = sync;
    // Fallback: if the sync doesn't bring the user key, use the one that came with the login.
    if sync.profile.key.is_none() {
        sync.profile.key = tokens.key.clone();
    }
    let sync = Arc::new(sync);

    let (keys, vault) = {
        let sync = sync.clone();
        io::blocking(move || open_vault(&master_key, &sync))
            .await
            .map_err(describe_vault_error)?
    };

    Ok(LoginOutcome::Done(Box::new(Unlocked {
        session: Session {
            client,
            email,
            kdf,
            access_token: tokens.access_token,
            refresh_token: tokens.refresh_token,
            sync,
            logged_in_at: now_secs(),
        },
        keys,
        vault,
    })))
}

/// Company SSO login: browser → code → tokens → master password.
///
/// The vault is still opened with the **master password** (as in the official CLI).
/// Accounts without a master password (Key Connector or trusted device) get
/// a clear message.
pub async fn start_sso_login(
    client: Arc<Client>,
    identifier: String,
    password: Zeroizing<String>,
) -> Result<LoginOutcome, String> {
    let identifier = identifier.trim().to_owned();
    let vault_url = client.urls().vault.clone();
    let code = io::blocking(move || crate::sso::authorize(&vault_url, &identifier)).await?;

    let tokens = {
        let client = client.clone();
        io::run(async move { client.login_sso(&code).await })
            .await
            .map_err(|e| match e {
                ApiError::TwoFactorRequired(_) => t!("flow.sso_2fa_unsupported").to_owned(),
                other => describe_api_error(other),
            })?
    };

    let options = tokens.user_decryption_options.clone().unwrap_or_default();
    if options.has_master_password == Some(false) {
        return Err(if options.key_connector.is_some() {
            t!("flow.key_connector_unsupported").to_owned()
        } else if options.trusted_device.is_some() {
            t!("flow.trusted_device_unsupported").to_owned()
        } else {
            t!("flow.no_master_password").to_owned()
        });
    }
    if password.is_empty() {
        return Err(t!("flow.sso_password_required").into());
    }

    let email = tokens.email().ok_or(t!("flow.sso_no_email"))?;
    let kdf = {
        let client = client.clone();
        let email = email.clone();
        io::run(async move { client.prelogin(&email).await })
            .await
            .map_err(describe_api_error)?
    };
    let master_key = {
        let email = email.clone();
        io::blocking(move || MasterKey::derive(&password, &email, kdf))
            .await
            .map_err(|e| t!("flow.kdf_failed", e = e))?
    };
    finish_login(client, email, kdf, Arc::new(master_key), tokens).await
}

/// 2FA with a security key or Windows Hello: asks Windows for the signature
/// and continues the login. If the person cancels, it stays on the same step.
pub async fn continue_with_webauthn(
    pending: PendingLogin,
    remember: bool,
) -> Result<LoginOutcome, String> {
    let Some(options) = pending.provider_data(TwoFactorProvider::WebAuthn).cloned() else {
        return Ok(LoginOutcome::WrongCode {
            pending,
            message: t!("flow.webauthn_no_data").into(),
        });
    };
    let origin = pending.client.urls().vault.clone();
    match io::blocking(move || crate::webauthn::sign(&options, &origin)).await {
        Ok(token) => {
            let second = SecondFactor {
                provider: TwoFactorProvider::WebAuthn,
                token,
                remember,
            };
            continue_login(pending, Some(second), None).await
        }
        Err(message) => Ok(LoginOutcome::WrongCode { pending, message }),
    }
}

/// Asks for the 2FA code by email. The future doesn't borrow `pending`, so the
/// screen can keep using it while the request runs.
pub fn send_email_code(
    pending: &PendingLogin,
) -> impl std::future::Future<Output = Result<(), String>> + 'static {
    let client = pending.client.clone();
    let email = pending.email.clone();
    let hash = pending.server_hash.clone();
    let token = pending.email_session_token.clone();
    async move {
        io::run(async move {
            client
                .send_email_login_code(&email, &hash, token.as_deref())
                .await
        })
        .await
        .map_err(describe_api_error)
    }
}

/// Offline unlock: redoes the KDF and decrypts the stored sync. Doesn't use the network.
/// CPU heavy — call it with [`io::blocking`].
pub fn unlock(session: &Session, password: &str) -> Result<(Keys, Vault), String> {
    let mk = MasterKey::derive(password, &session.email, session.kdf)
        .map_err(|e| t!("flow.kdf_failed", e = e))?;
    open_vault(&mk, &session.sync).map_err(describe_vault_error)
}

/// Renews the token and downloads the vault again. Returns the updated session and the decrypted vault.
/// Why "Sync" failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncError {
    /// The server no longer accepts the refresh token: you have to sign in again.
    Expired,
    Other(String),
}

pub async fn resync(session: Session, keys: Arc<Keys>) -> Result<(Session, Vault), SyncError> {
    let w = write(session, keys, WriteOp::Nothing).await?;
    Ok((w.session, w.vault))
}

/// File chosen to attach when saving the item (contents in memory only).
#[derive(Clone)]
pub struct NewFile {
    pub name: String,
    pub contents: Zeroizing<Vec<u8>>,
}

impl NewFile {
    /// Reads a file from disk, respecting the attachment size limit.
    pub fn read(path: &std::path::Path) -> Result<Self, String> {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| t!("flow.file").into());
        let meta =
            std::fs::metadata(path).map_err(|e| t!("flow.read_failed", name = name, e = e))?;
        if meta.len() as usize > scout_vault_core::attachment::MAX_SIZE {
            return Err(t!("flow.file_too_big", name = name));
        }
        let contents =
            std::fs::read(path).map_err(|e| t!("flow.read_failed", name = name, e = e))?;
        Ok(Self {
            name,
            contents: Zeroizing::new(contents),
        })
    }
}

/// Encrypts and uploads each file as an attachment of `cipher`. Failures become warnings
/// (the rest goes on), already explained for the person.
async fn upload_files(
    client: &Client,
    token: &str,
    cipher: &scout_vault_core::sync::CipherRaw,
    keys: &Keys,
    files: Vec<NewFile>,
    warnings: &mut Vec<String>,
) {
    for file in files {
        let result =
            match scout_vault_core::attachment::prepare(cipher, keys, &file.name, &file.contents) {
                Ok(new) => async {
                    let up = client
                        .create_attachment(
                            token,
                            &cipher.id,
                            &new.key,
                            &new.file_name,
                            new.data.len(),
                        )
                        .await?;
                    client
                        .upload_attachment(token, &cipher.id, &up, &new.file_name, new.data)
                        .await
                }
                .await
                .map_err(describe_api_error),
                Err(e) => Err(t!("flow.encrypt_failed", e = e)),
            };
        if let Err(e) = result {
            warnings.push(format!("{}: {e}", file.name));
        }
    }
}

/// Result of [`write`].
pub struct Written {
    pub session: Session,
    pub vault: Vault,
    /// Item saved/changed (or folder created/renamed).
    pub saved_id: Option<String>,
    /// Problems that don't undo the operation (e.g. a rejected attachment).
    pub warnings: Vec<String>,
    /// Vault revision date right after this sync (basis of background sync).
    pub revision: Option<i64>,
}

/// Result of "did the vault change on the server?".
pub struct Revision {
    /// Revision date (ms).
    pub date: i64,
    /// Tokens renewed along the way (the access token had expired).
    pub tokens: Option<(String, Option<String>)>,
}

/// Asks the server when the vault changed (cheap; doesn't download the vault).
/// Uses the current access token and only renews it if it has expired.
pub async fn check_revision(session: &Session) -> Result<Revision, SyncError> {
    let client = session.client.clone();
    let (access, refresh_token) = (session.access_token.clone(), session.refresh_token.clone());
    io::run(async move {
        if !access.is_empty() {
            match client.revision_date(&access).await {
                Ok(date) => return Ok(Revision { date, tokens: None }),
                Err(ApiError::Unauthorized) => {}
                Err(e) => return Err(e),
            }
        }
        let t = client.refresh(&refresh_token).await?;
        let date = client.revision_date(&t.access_token).await?;
        Ok(Revision {
            date,
            tokens: Some((t.access_token, t.refresh_token)),
        })
    })
    .await
    .map_err(sync_error)
}

/// A change to the vault.
pub enum WriteOp {
    /// Just sync.
    Nothing,
    /// Create (empty id) or save an edited item, and attach new files.
    Save(Box<scout_vault_core::vault::Item>, Vec<NewFile>),
    Trash(String),
    Restore(String),
    /// Delete permanently (the item must be in the trash).
    Delete(String),
    /// Attach a file to the item.
    /// Attach files to an item that already exists.
    AddAttachments {
        item_id: String,
        files: Vec<NewFile>,
    },
    RemoveAttachment {
        item_id: String,
        attachment_id: String,
    },
    CreateFolder(String),
    RenameFolder {
        id: String,
        name: String,
    },
    /// Deletes the folder; its items end up with no folder.
    DeleteFolder(String),
    /// Moves the item to another folder (`None` = no folder).
    MoveItem {
        item_id: String,
        folder_id: Option<String>,
    },
    /// Renames several folders at once (moving a folder, with its subfolders,
    /// into another: in Bitwarden the hierarchy is the name "Parent/Child").
    /// The returned id is the first one's.
    RenameFolders(Vec<(String, String)>),
    /// Adds many items at once (import), with their folders by name; `existing`
    /// are the vault's folders, reused when the name matches.
    Import {
        items: Vec<scout_vault_core::import::Imported>,
        existing: Vec<scout_vault_core::vault::Folder>,
    },
}

fn base_cipher(
    session: &Session,
    id: &str,
) -> Result<scout_vault_core::sync::CipherRaw, SyncError> {
    session
        .sync
        .ciphers
        .iter()
        .find(|c| c.id == id)
        .cloned()
        .ok_or_else(|| SyncError::Other(t!("flow.item_gone").into()))
}

/// Checks the master password without network: derives the key and sees whether it opens the
/// profile's user key (for items that ask for the password again). The KDF runs off the UI.
pub async fn check_master_password(session: &Session, password: Zeroizing<String>) -> bool {
    let (email, kdf, sync) = (session.email.clone(), session.kdf, session.sync.clone());
    io::blocking(move || {
        MasterKey::derive(&password, &email, kdf)
            .ok()
            .is_some_and(|mk| Keys::unlock(&mk, &sync.profile).is_ok())
    })
    .await
}

/// Downloads and decrypts an attachment. The contents exist only in memory.
pub async fn download_attachment(
    session: Session,
    keys: Arc<Keys>,
    item_id: String,
    attachment_id: String,
) -> Result<Zeroizing<Vec<u8>>, SyncError> {
    let cipher = base_cipher(&session, &item_id)?;
    let raw = scout_vault_core::attachment::raw_attachments(&cipher)
        .into_iter()
        .find(|a| a.id == attachment_id)
        .ok_or_else(|| SyncError::Other(t!("flow.attachment_gone").into()))?;
    let bytes = {
        let client = session.client.clone();
        let refresh_token = session.refresh_token.clone();
        let (item_id, attachment_id) = (item_id.clone(), attachment_id.clone());
        io::run(async move {
            let t = client.refresh(&refresh_token).await?;
            let url = client
                .attachment_url(&t.access_token, &item_id, &attachment_id)
                .await?;
            client.download(&url).await
        })
        .await
        .map_err(sync_error)?
    };
    io::blocking(move || {
        let key = scout_vault_core::attachment::attachment_key(&cipher, &raw, &keys)
            .map_err(|e| SyncError::Other(t!("flow.attachment_key_failed", e = e)))?;
        scout_vault_core::attachment::decrypt_file(&bytes, &key)
            .map_err(|e| SyncError::Other(t!("flow.attachment_decrypt_failed", e = e)))
    })
    .await
}

fn sync_error(e: ApiError) -> SyncError {
    match e {
        ApiError::Unauthorized => SyncError::Expired,
        other => SyncError::Other(describe_api_error(other)),
    }
}

/// Renews the token, applies the change on the server and downloads the vault again.
/// Returns the updated session, the decrypted vault and the id of the saved item.
pub async fn write(session: Session, keys: Arc<Keys>, op: WriteOp) -> Result<Written, SyncError> {
    // Encrypt before touching the network: an error here leaves nothing half done.
    let attach_base = match &op {
        WriteOp::AddAttachments { item_id, .. } => Some(base_cipher(&session, item_id)?),
        _ => None,
    };
    let folder_name = match &op {
        WriteOp::CreateFolder(name) | WriteOp::RenameFolder { name, .. } => Some(
            scout_vault_core::edit::encrypt_folder_name(name, &keys)
                .map_err(|e| SyncError::Other(t!("flow.folder_save_failed", e = e)))?,
        ),
        _ => None,
    };
    let renamed = match &op {
        WriteOp::RenameFolders(list) => list
            .iter()
            .map(|(id, name)| {
                scout_vault_core::edit::encrypt_folder_name(name, &keys)
                    .map(|enc| (id.clone(), enc))
                    .map_err(|e| SyncError::Other(t!("flow.folder_save_failed", e = e)))
            })
            .collect::<Result<Vec<_>, _>>()?,
        _ => Vec::new(),
    };
    let favorite = match &op {
        WriteOp::MoveItem { item_id, .. } => base_cipher(&session, item_id)?.favorite,
        _ => false,
    };
    let body = match &op {
        WriteOp::Save(item, _) => {
            let base = (!item.id.is_empty())
                .then(|| session.sync.ciphers.iter().find(|c| c.id == item.id))
                .flatten()
                .cloned();
            if !item.id.is_empty() && base.is_none() {
                return Err(SyncError::Other(t!("flow.item_gone").into()));
            }
            if item.organization_id.is_some() && item.collection_ids.is_empty() {
                return Err(SyncError::Other(t!("editor.collection_required").into()));
            }
            let raw = scout_vault_core::edit::encrypt_item(item, base.as_ref(), &keys)
                .map_err(|e| SyncError::Other(t!("flow.item_encrypt_failed", e = e)))?;
            Some(scout_vault_core::edit::request_body(
                &raw,
                &session.sync.profile.id,
            ))
        }
        _ => None,
    };
    let import_body = match &op {
        WriteOp::Import { items, existing } => Some(
            scout_vault_core::import::request(items, existing, &keys, &session.sync.profile.id)
                .map_err(|e| SyncError::Other(t!("flow.item_encrypt_failed", e = e)))?,
        ),
        _ => None,
    };
    let file_keys = keys.clone();
    // Edited organization item: did the collections change?
    let collections_changed = match &op {
        WriteOp::Save(item, _) if !item.id.is_empty() => {
            let mut before: Vec<String> = session
                .sync
                .ciphers
                .iter()
                .find(|c| c.id == item.id)
                .and_then(|c| c.extra.get("collectionIds"))
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();
            let mut after = item.collection_ids.clone();
            before.sort();
            after.sort();
            before != after
        }
        _ => false,
    };
    let (access, refresh, sync, saved_id, warnings, revision) = {
        let client = session.client.clone();
        let refresh_token = session.refresh_token.clone();
        io::run(async move {
            let mut warnings: Vec<String> = Vec::new();
            let t = client.refresh(&refresh_token).await?;
            let token = &t.access_token;
            let saved_id = match op {
                WriteOp::Nothing => None,
                WriteOp::Save(item, files) => {
                    let body = body.expect("cifrado acima");
                    let in_org = item.organization_id.is_some();
                    let saved = if item.id.is_empty() && in_org {
                        // New organization item: born already in the chosen collections.
                        client
                            .create_cipher_in_org(token, &body, &item.collection_ids)
                            .await?
                    } else if item.id.is_empty() {
                        client.create_cipher(token, &body).await?
                    } else {
                        let saved = client.update_cipher(token, &item.id, &body).await?;
                        if in_org && collections_changed {
                            client
                                .update_collections(token, &item.id, &item.collection_ids)
                                .await?;
                        }
                        saved
                    };
                    // New files go after the item exists, with its key.
                    // A failure here doesn't undo the item: it becomes a warning.
                    upload_files(&client, token, &saved, &file_keys, files, &mut warnings).await;
                    Some(saved.id)
                }
                WriteOp::Trash(id) => {
                    client.trash_cipher(token, &id).await?;
                    Some(id)
                }
                WriteOp::Restore(id) => {
                    client.restore_cipher(token, &id).await?;
                    Some(id)
                }
                WriteOp::Delete(id) => {
                    client.delete_cipher(token, &id).await?;
                    None
                }
                WriteOp::AddAttachments { item_id, files } => {
                    let base = attach_base.expect("procurado acima");
                    upload_files(&client, token, &base, &file_keys, files, &mut warnings).await;
                    Some(item_id)
                }
                WriteOp::RemoveAttachment {
                    item_id,
                    attachment_id,
                } => {
                    client
                        .delete_attachment(token, &item_id, &attachment_id)
                        .await?;
                    Some(item_id)
                }
                WriteOp::CreateFolder(_) => {
                    let name = folder_name.expect("cifrado acima");
                    Some(client.create_folder(token, &name).await?.id)
                }
                WriteOp::RenameFolder { id, .. } => {
                    let name = folder_name.expect("cifrado acima");
                    client.rename_folder(token, &id, &name).await?;
                    Some(id)
                }
                WriteOp::DeleteFolder(id) => {
                    client.delete_folder(token, &id).await?;
                    None
                }
                WriteOp::MoveItem { item_id, folder_id } => {
                    client
                        .move_cipher_to_folder(token, &item_id, folder_id.as_deref(), favorite)
                        .await?;
                    Some(item_id)
                }
                WriteOp::RenameFolders(_) => {
                    for (id, name) in &renamed {
                        client.rename_folder(token, id, name).await?;
                    }
                    renamed.first().map(|(id, _)| id.clone())
                }
                WriteOp::Import { .. } => {
                    let body = import_body.expect("cifrado acima");
                    client.import_ciphers(token, &body).await?;
                    None
                }
            };
            let sync = client.sync(token).await?;
            // Basis for background sync (if it fails, the next check gets it right).
            let revision = client.revision_date(token).await.ok();
            Ok::<_, ApiError>((
                t.access_token,
                t.refresh_token,
                sync,
                saved_id,
                warnings,
                revision,
            ))
        })
        .await
        .map_err(sync_error)?
    };
    let mut sync = sync;
    if sync.profile.key.is_none() {
        sync.profile.key = session.sync.profile.key.clone();
    }
    let sync = Arc::new(sync);
    let vault = {
        let sync = sync.clone();
        io::blocking(move || Vault::decrypt(&sync, &keys)).await
    };
    Ok(Written {
        session: Session {
            access_token: access,
            refresh_token: refresh.unwrap_or(session.refresh_token.clone()),
            sync,
            ..session
        },
        vault,
        saved_id,
        warnings,
        revision,
    })
}

fn client_identity(urls: &scout_vault_core::api::ServerUrls) -> String {
    urls.identity.clone()
}

fn open_vault(mk: &MasterKey, sync: &SyncResponse) -> Result<(Keys, Vault), VaultError> {
    let keys = Keys::unlock(mk, &sync.profile)?;
    let vault = Vault::decrypt(sync, &keys);
    Ok((keys, vault))
}

pub fn describe_api_error(e: ApiError) -> String {
    match e {
        ApiError::Http(e) if e.is_connect() || e.is_timeout() => t!("flow.cant_connect").into(),
        ApiError::Http(e) => t!("flow.network_error", e = e),
        ApiError::InvalidCredentials(_) => t!("flow.invalid_credentials").into(),
        ApiError::InvalidTwoFactorCode(_) => t!("flow.invalid_2fa").into(),
        ApiError::InvalidNewDeviceCode => t!("flow.invalid_device_code").into(),
        ApiError::CaptchaRequired => t!("flow.captcha").into(),
        ApiError::Unauthorized => t!("flow.session_expired").into(),
        ApiError::RateLimited => t!("flow.rate_limited").into(),
        ApiError::TwoFactorRequired(_) => t!("flow.2fa_required").into(),
        ApiError::NewDeviceVerificationRequired => t!("flow.device_code_required").into(),
        ApiError::Status { status, body } => {
            // Shows the server's message, if any: helps diagnose real accounts.
            let detail = server_message(&body);
            let lower = detail.as_deref().unwrap_or_default().to_ascii_lowercase();
            if lower.contains("premium") {
                return t!("flow.premium_required").into();
            }
            if lower.contains("storage") || status == 413 {
                return t!("flow.no_storage").into();
            }
            if detail
                .as_deref()
                .is_some_and(|m| m.to_ascii_lowercase().contains("update your app"))
            {
                return t!("flow.outdated").into();
            }
            match detail {
                Some(m) => t!("flow.server_refused_detail", status = status, m = m),
                None => t!("flow.server_refused", status = status),
            }
        }
        other => other.to_string(),
    }
}

/// Extracts a readable message from the server's error body (official or Vaultwarden).
fn server_message(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let pick = |v: &serde_json::Value, keys: &[&str]| {
        keys.iter()
            .find_map(|k| v.get(*k).and_then(|m| m.as_str()))
            .map(str::to_owned)
    };
    let model = v.get("ErrorModel").or_else(|| v.get("errorModel"));
    model
        .and_then(|m| pick(m, &["Message", "message"]))
        .or_else(|| pick(&v, &["message", "Message", "error_description"]))
        .filter(|m| !m.is_empty())
        .map(|m| m.chars().take(200).collect())
}

pub fn describe_vault_error(e: VaultError) -> String {
    match e {
        VaultError::WrongPassword => t!("vault.wrong_password").into(),
        VaultError::Parse(ParseError::UnsupportedType(t)) => {
            t!("flow.unsupported_crypto", t = t)
        }
        VaultError::MissingUserKey => t!("flow.missing_user_key").into(),
        other => t!("flow.open_failed", e = other),
    }
}

/// Readable name of a 2FA provider.
pub fn provider_name(p: TwoFactorProvider) -> &'static str {
    match p {
        TwoFactorProvider::Authenticator => t!("flow.provider_authenticator"),
        TwoFactorProvider::Email => t!("flow.provider_email"),
        TwoFactorProvider::Duo | TwoFactorProvider::OrganizationDuo => "Duo",
        TwoFactorProvider::Yubikey => "YubiKey",
        TwoFactorProvider::U2f | TwoFactorProvider::WebAuthn => t!("flow.provider_fido"),
        TwoFactorProvider::Remember => t!("flow.provider_remember"),
        TwoFactorProvider::Other(_) => t!("flow.provider_other"),
    }
}

/// Duo address the server sent for this login (`AuthUrl`).
pub fn duo_auth_url(pending: &PendingLogin, provider: TwoFactorProvider) -> Option<String> {
    let data = pending.provider_data(provider)?;
    data.get("authUrl")
        .or_else(|| data.get("AuthUrl"))
        .and_then(|v| v.as_str())
        .map(str::to_owned)
}

/// Builds the Duo token (`code|state`) from what the person pasted: the
/// final page address (`…duo-redirect-connector.html?…&code=…&state=…`),
/// the `bitwarden://duo-callback?code=…&state=…` link or the `code|state` itself.
pub fn duo_token(input: &str) -> Option<String> {
    let input = input.trim();
    if let Some((_, query)) = input.split_once('?') {
        let query = query.split('#').next().unwrap_or(query);
        let code = crate::sso::query_param(query, "code")?;
        let state = crate::sso::query_param(query, "state")?;
        return Some(format!("{code}|{state}"));
    }
    let (code, state) = input.split_once('|')?;
    (!code.is_empty() && !state.is_empty()).then(|| input.to_owned())
}

/// Instruction shown at the 2FA step for each method.
pub fn provider_prompt(p: TwoFactorProvider) -> String {
    match p {
        TwoFactorProvider::WebAuthn | TwoFactorProvider::U2f => t!("flow.prompt_webauthn").into(),
        TwoFactorProvider::Duo | TwoFactorProvider::OrganizationDuo => t!("flow.prompt_duo").into(),
        other => t!("flow.prompt_code", provider = provider_name(other)),
    }
}

/// Providers this client knows how to use (typed code).
pub fn supported_providers(providers: &[TwoFactorProvider]) -> Vec<TwoFactorProvider> {
    let mut out: Vec<_> = providers
        .iter()
        .copied()
        .filter(|p| {
            matches!(
                p,
                TwoFactorProvider::Authenticator
                    | TwoFactorProvider::Email
                    | TwoFactorProvider::Yubikey
                    | TwoFactorProvider::Duo
                    | TwoFactorProvider::OrganizationDuo
            ) || (cfg!(windows) && *p == TwoFactorProvider::WebAuthn)
        })
        .collect();
    // Authenticator first: it is the most common and doesn't depend on email.
    out.sort_by_key(|p| p.code());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Messages for the person can't have a hole of spaces in the middle
    /// (leftover of a badly done line break in the code becomes a gap in the toast).
    #[test]
    fn messages_have_no_runs_of_spaces() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut bad = Vec::new();
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            for (n, line) in text.lines().enumerate() {
                if line.trim_start().starts_with("//") {
                    continue;
                }
                // Odd parts of a line split at `"` are the contents of the strings.
                let in_string = line.split('"').skip(1).step_by(2);
                if in_string.into_iter().any(|s| s.trim().contains("   ")) {
                    bad.push(format!("{}:{}", path.display(), n + 1));
                }
            }
        }
        // The translation catalogs too: no double spaces, not even at the ends
        // (" ou " is the exception, on purpose).
        for lang in crate::i18n::Language::ALL {
            for (key, text) in crate::i18n::catalog(lang) {
                if text.contains("  ") || (key != "login.or" && text.trim() != text) {
                    bad.push(format!("{}: {key}", lang.code()));
                }
            }
        }
        assert!(bad.is_empty(), "espaços sobrando em: {bad:?}");
    }

    #[test]
    fn supported_providers_filters_and_orders() {
        let got = supported_providers(&[
            TwoFactorProvider::WebAuthn,
            TwoFactorProvider::Email,
            TwoFactorProvider::Duo,
            TwoFactorProvider::Authenticator,
        ]);
        let mut expected = vec![
            TwoFactorProvider::Authenticator,
            TwoFactorProvider::Email,
            TwoFactorProvider::Duo,
        ];
        if cfg!(windows) {
            expected.push(TwoFactorProvider::WebAuthn);
        }
        assert_eq!(got, expected);
    }

    #[test]
    fn duo_token_from_pasted_url() {
        let url = "https://vault.bitwarden.com/duo-redirect-connector.html?client=desktop&deeplinkScheme=bitwarden&code=ab%2Fc&state=st%3D1";
        assert_eq!(duo_token(url).as_deref(), Some("ab/c|st=1"));
        assert_eq!(
            duo_token("bitwarden://duo-callback?code=x&state=y").as_deref(),
            Some("x|y")
        );
        assert_eq!(duo_token(" x|y ").as_deref(), Some("x|y"));
        assert_eq!(duo_token("https://exemplo.com/?code=x"), None);
        assert_eq!(duo_token("123456"), None);
    }

    #[test]
    fn server_message_is_extracted() {
        assert_eq!(
            server_message(r#"{"ErrorModel":{"Message":"Algo deu errado"}}"#).as_deref(),
            Some("Algo deu errado")
        );
        assert_eq!(
            server_message(r#"{"message":"rate limited"}"#).as_deref(),
            Some("rate limited")
        );
        assert_eq!(server_message("<html>"), None);
    }

    #[test]
    fn attachment_refusals_are_explained() {
        let premium = describe_api_error(ApiError::Status {
            status: 400,
            body: r#"{"message":"Premium access is required to use this feature."}"#.into(),
        });
        assert!(premium.contains("Premium"), "{premium}");
        let storage = describe_api_error(ApiError::Status {
            status: 400,
            body: r#"{"message":"Not enough storage available."}"#.into(),
        });
        assert!(storage.contains("espaço"), "{storage}");
    }

    #[test]
    fn outdated_client_is_explained() {
        let msg = describe_api_error(ApiError::Status {
            status: 400,
            body: r#"{"Object":"error","Message":"Please update your app to continue using Bitwarden"}"#
                .into(),
        });
        assert!(msg.contains("versão do Scout Vault"), "{msg}");
    }

    #[test]
    fn unknown_encryption_has_a_clear_message() {
        let e = VaultError::Parse(ParseError::UnsupportedType("9".into()));
        assert!(describe_vault_error(e).contains("tipo 9"));
    }

    #[test]
    fn wrong_password_message() {
        assert_eq!(
            describe_vault_error(VaultError::WrongPassword),
            "Senha do cofre incorreta."
        );
        assert_eq!(
            describe_api_error(ApiError::InvalidCredentials("x".into())),
            "E-mail ou senha do cofre incorretos."
        );
    }
}
