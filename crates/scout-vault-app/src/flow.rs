//! Fluxos de login, desbloqueio e sincronização — sem nenhuma UI.
//!
//! As telas só chamam estas funções e mostram o resultado; assim a lógica
//! (inclusive as mensagens de erro) pode ser testada sem janela.

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

/// Sessão autenticada. Guarda só dados **cifrados** e tokens — continua
/// existindo enquanto o cofre está bloqueado.
#[derive(Clone)]
pub struct Session {
    pub client: Arc<Client>,
    pub email: String,
    pub kdf: Kdf,
    pub access_token: String,
    pub refresh_token: String,
    pub sync: Arc<SyncResponse>,
}

/// Cofre aberto: sessão + chaves + dados decifrados.
pub struct Unlocked {
    pub session: Session,
    pub keys: Keys,
    pub vault: Vault,
}

/// Entrega um cofre aberto de uma tela para outra através de um evento.
/// Eventos chegam por referência; a `Cell` permite **mover** as chaves uma
/// única vez, sem cloná-las.
pub struct Handoff(std::cell::Cell<Option<Box<Unlocked>>>);

impl Handoff {
    pub fn new(unlocked: Box<Unlocked>) -> Self {
        Self(std::cell::Cell::new(Some(unlocked)))
    }

    pub fn take(&self) -> Option<Box<Unlocked>> {
        self.0.take()
    }
}

/// Login parado à espera de um segundo fator ou do código de novo dispositivo.
/// Mantém a master key para não refazer a KDF (lenta) a cada tentativa.
pub struct PendingLogin {
    client: Arc<Client>,
    email: String,
    kdf: Kdf,
    master_key: Arc<MasterKey>,
    server_hash: Zeroizing<String>,
    /// Exigido pelo servidor oficial para reenviar o código de 2FA por e-mail.
    email_session_token: Option<String>,
    /// Métodos de 2FA que o servidor ofereceu (para voltar ao passo após um código errado).
    providers: Vec<TwoFactorProvider>,
    /// Método usado na última tentativa.
    last_provider: Option<TwoFactorProvider>,
    /// A última tentativa foi o código de novo dispositivo.
    new_device: bool,
    /// Onde guardar o token de "lembrar este dispositivo" e com qual chave.
    secrets: SecretStore,
    remember_key: String,
    /// Dados que o servidor mandou para cada método de 2FA.
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

    /// Dados que o servidor mandou para um método de 2FA.
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
    /// Código de 2FA ou de novo dispositivo errado: fica no mesmo passo, sem
    /// refazer a KDF nem pedir a senha de novo.
    WrongCode {
        pending: PendingLogin,
        message: String,
    },
}

/// Primeira etapa: prelogin, KDF, login.
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
        .map_err(|e| format!("Falha ao derivar a chave: {e}"))?
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
    // Dispositivo lembrado: tenta pular o 2FA com o token guardado.
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

/// Reenvia o login com o segundo fator e/ou o código de novo dispositivo.
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
            // O token lembrado expirou ou foi revogado: esquece e pede o código.
            if pending.last_provider == Some(TwoFactorProvider::Remember) {
                pending.secrets.delete(&pending.remember_key);
                pending.last_provider = None;
            }
            // O servidor oficial devolve o pedido de 2FA de novo quando um token
            // "lembrado" expira; tratamos igual a um pedido novo.
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

/// Parte final de qualquer login: baixa o cofre e abre com a master key.
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
    // Reserva: se o sync não trouxer a user key, usa a que veio no login.
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
        },
        keys,
        vault,
    })))
}

/// Login por SSO da empresa: navegador → código → tokens → senha mestra.
///
/// O cofre continua sendo aberto com a **senha mestra** (como no CLI oficial).
/// Contas sem senha mestra (Key Connector ou dispositivo confiável) recebem
/// uma mensagem clara.
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
                ApiError::TwoFactorRequired(_) => {
                    "A empresa exige um segundo fator depois do SSO, o que o Scout Vault ainda não \
                     suporta neste caminho."
                        .to_owned()
                }
                other => describe_api_error(other),
            })?
    };

    let options = tokens.user_decryption_options.clone().unwrap_or_default();
    if options.has_master_password == Some(false) {
        return Err(if options.key_connector.is_some() {
            "Esta conta abre o cofre pelo Key Connector da empresa, que o Scout Vault ainda não suporta."
                .to_owned()
        } else if options.trusted_device.is_some() {
            "Esta conta usa \"dispositivo confiável\" em vez de senha mestra, que o Scout Vault (como o \
             CLI oficial) não suporta."
                .to_owned()
        } else {
            "Esta conta ainda não tem senha mestra. Defina uma no cofre web e tente de novo."
                .to_owned()
        });
    }
    if password.is_empty() {
        return Err(
            "Informe a senha mestra: ela é necessária para abrir o cofre depois do SSO.".into(),
        );
    }

    let email = tokens
        .email()
        .ok_or("O servidor não informou o e-mail da conta no login por SSO.")?;
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
            .map_err(|e| format!("Falha ao derivar a chave: {e}"))?
    };
    finish_login(client, email, kdf, Arc::new(master_key), tokens).await
}

/// 2FA por chave de segurança ou Windows Hello: pede a assinatura ao Windows
/// e continua o login. Se a pessoa cancelar, fica no mesmo passo.
pub async fn continue_with_webauthn(
    pending: PendingLogin,
    remember: bool,
) -> Result<LoginOutcome, String> {
    let Some(options) = pending.provider_data(TwoFactorProvider::WebAuthn).cloned() else {
        return Ok(LoginOutcome::WrongCode {
            pending,
            message: "O servidor não mandou os dados da chave de segurança.".into(),
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

/// Pede o código de 2FA por e-mail. O futuro não empresta `pending`, para a
/// tela poder continuar com ele enquanto o pedido roda.
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

/// Desbloqueio offline: refaz a KDF e decifra o sync guardado. Não usa a rede.
/// Pesado de CPU — chame com [`io::blocking`].
pub fn unlock(session: &Session, password: &str) -> Result<(Keys, Vault), String> {
    let mk = MasterKey::derive(password, &session.email, session.kdf)
        .map_err(|e| format!("Falha ao derivar a chave: {e}"))?;
    open_vault(&mk, &session.sync).map_err(describe_vault_error)
}

/// Renova o token e baixa o cofre de novo. Devolve a sessão atualizada e o cofre decifrado.
/// Por que o "Sincronizar" falhou.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncError {
    /// O servidor não aceita mais o refresh token: é preciso entrar de novo.
    Expired,
    Other(String),
}

pub async fn resync(session: Session, keys: Arc<Keys>) -> Result<(Session, Vault), SyncError> {
    let w = write(session, keys, WriteOp::Nothing).await?;
    Ok((w.session, w.vault))
}

/// Arquivo escolhido para anexar ao salvar o item (conteúdo só em memória).
#[derive(Clone)]
pub struct NewFile {
    pub name: String,
    pub contents: Zeroizing<Vec<u8>>,
}

impl NewFile {
    /// Lê um arquivo do disco, respeitando o limite de tamanho dos anexos.
    pub fn read(path: &std::path::Path) -> Result<Self, String> {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "arquivo".into());
        let meta =
            std::fs::metadata(path).map_err(|e| format!("Não foi possível ler {name}: {e}"))?;
        if meta.len() as usize > scout_vault_core::attachment::MAX_SIZE {
            return Err(format!("{name} passa do limite de 500 MB."));
        }
        let contents =
            std::fs::read(path).map_err(|e| format!("Não foi possível ler {name}: {e}"))?;
        Ok(Self {
            name,
            contents: Zeroizing::new(contents),
        })
    }
}

/// Cifra e envia cada arquivo como anexo de `cipher`. Falhas viram avisos
/// (o resto continua), já explicados em português.
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
                Err(e) => Err(format!("não foi possível cifrar: {e}")),
            };
        if let Err(e) = result {
            warnings.push(format!("{}: {e}", file.name));
        }
    }
}

/// Resultado de [`write`].
pub struct Written {
    pub session: Session,
    pub vault: Vault,
    /// Item salvo/alterado (ou pasta criada/renomeada).
    pub saved_id: Option<String>,
    /// Problemas que não desfazem a operação (ex.: um anexo recusado).
    pub warnings: Vec<String>,
}

/// Uma alteração no cofre.
pub enum WriteOp {
    /// Só sincronizar.
    Nothing,
    /// Criar (id vazio) ou gravar um item editado, e anexar arquivos novos.
    Save(Box<scout_vault_core::vault::Item>, Vec<NewFile>),
    Trash(String),
    Restore(String),
    /// Apagar de vez (o item precisa estar na lixeira).
    Delete(String),
    /// Anexar um arquivo ao item.
    /// Anexar arquivos a um item que já existe.
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
    /// Apaga a pasta; os itens dela ficam sem pasta.
    DeleteFolder(String),
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
        .ok_or_else(|| {
            SyncError::Other("Este item não está mais no cofre. Sincronize e tente de novo.".into())
        })
}

/// Baixa e decifra um anexo. O conteúdo só existe em memória.
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
        .ok_or_else(|| SyncError::Other("O anexo não está mais no item.".into()))?;
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
        let key =
            scout_vault_core::attachment::attachment_key(&cipher, &raw, &keys).map_err(|e| {
                SyncError::Other(format!("Não foi possível abrir a chave do anexo: {e}"))
            })?;
        scout_vault_core::attachment::decrypt_file(&bytes, &key)
            .map_err(|e| SyncError::Other(format!("O anexo não pôde ser decifrado: {e}")))
    })
    .await
}

fn sync_error(e: ApiError) -> SyncError {
    match e {
        ApiError::Unauthorized => SyncError::Expired,
        other => SyncError::Other(describe_api_error(other)),
    }
}

/// Renova o token, aplica a alteração no servidor e baixa o cofre de novo.
/// Devolve a sessão atualizada, o cofre decifrado e o id do item salvo.
pub async fn write(session: Session, keys: Arc<Keys>, op: WriteOp) -> Result<Written, SyncError> {
    // Cifra antes de tocar na rede: um erro aqui não deixa nada pela metade.
    let attach_base = match &op {
        WriteOp::AddAttachments { item_id, .. } => Some(base_cipher(&session, item_id)?),
        _ => None,
    };
    let folder_name = match &op {
        WriteOp::CreateFolder(name) | WriteOp::RenameFolder { name, .. } => Some(
            scout_vault_core::edit::encrypt_folder_name(name, &keys)
                .map_err(|e| SyncError::Other(format!("Não foi possível salvar a pasta: {e}")))?,
        ),
        _ => None,
    };
    let body = match &op {
        WriteOp::Save(item, _) => {
            let base = (!item.id.is_empty())
                .then(|| session.sync.ciphers.iter().find(|c| c.id == item.id))
                .flatten()
                .cloned();
            if !item.id.is_empty() && base.is_none() {
                return Err(SyncError::Other(
                    "Este item não está mais no cofre. Sincronize e tente de novo.".into(),
                ));
            }
            let raw = scout_vault_core::edit::encrypt_item(item, base.as_ref(), &keys)
                .map_err(|e| SyncError::Other(format!("Não foi possível cifrar o item: {e}")))?;
            Some(scout_vault_core::edit::request_body(
                &raw,
                &session.sync.profile.id,
            ))
        }
        _ => None,
    };
    let file_keys = keys.clone();
    let (access, refresh, sync, saved_id, warnings) = {
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
                    let saved = if item.id.is_empty() {
                        client.create_cipher(token, &body).await?
                    } else {
                        client.update_cipher(token, &item.id, &body).await?
                    };
                    // Arquivos novos vão depois do item existir, com a chave dele.
                    // Uma falha aqui não desfaz o item: vira aviso.
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
            };
            let sync = client.sync(token).await?;
            Ok::<_, ApiError>((t.access_token, t.refresh_token, sync, saved_id, warnings))
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
        ApiError::Http(e) if e.is_connect() || e.is_timeout() => {
            "Não foi possível conectar ao servidor. Confira o endereço e a sua conexão.".into()
        }
        ApiError::Http(e) => format!("Erro de rede: {e}"),
        ApiError::InvalidCredentials(_) => "E-mail ou senha mestra incorretos.".into(),
        ApiError::InvalidTwoFactorCode(_) => {
            "Código incorreto ou expirado. Confira e tente de novo.".into()
        }
        ApiError::InvalidNewDeviceCode => {
            "Código de verificação incorreto. Use o último que chegou no seu e-mail.".into()
        }
        ApiError::CaptchaRequired => "O servidor pediu um captcha, que o Scout Vault não consegue \
             mostrar. Entre uma vez pelo cofre web (vault.bitwarden.com) e tente de novo."
            .into(),
        ApiError::Unauthorized => "A sessão expirou. Entre novamente.".into(),
        ApiError::RateLimited => {
            "Muitas tentativas seguidas. Espere um minuto e tente de novo.".into()
        }
        ApiError::TwoFactorRequired(_) => "O servidor pediu um segundo fator.".into(),
        ApiError::NewDeviceVerificationRequired => {
            "O servidor pediu o código de verificação enviado por e-mail.".into()
        }
        ApiError::Status { status, body } => {
            // Mostra a mensagem do servidor, se houver: ajuda a diagnosticar contas reais.
            let detail = server_message(&body);
            let lower = detail.as_deref().unwrap_or_default().to_ascii_lowercase();
            if lower.contains("premium") {
                return "Anexos exigem Bitwarden Premium (ou uma organização). Na conta gratuita                         do bitwarden.com não dá para enviar arquivos."
                    .into();
            }
            if lower.contains("storage") || status == 413 {
                return "Não há espaço para esse arquivo: o limite de anexos da conta acabou."
                    .into();
            }
            if detail
                .as_deref()
                .is_some_and(|m| m.to_ascii_lowercase().contains("update your app"))
            {
                return "O servidor do Bitwarden recusou esta versão do Scout Vault por ser antiga. \
                        Atualize o Scout Vault para uma versão mais nova."
                    .into();
            }
            match detail {
                Some(m) => format!("O servidor recusou o pedido (HTTP {status}): {m}"),
                None => format!("O servidor recusou o pedido (HTTP {status})."),
            }
        }
        other => other.to_string(),
    }
}

/// Extrai uma mensagem legível do corpo de erro do servidor (oficial ou Vaultwarden).
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
        VaultError::WrongPassword => "Senha mestra incorreta.".into(),
        VaultError::Parse(ParseError::UnsupportedType(t)) => {
            format!("Esta conta usa um formato de criptografia (tipo {t}) que o Scout Vault ainda não conhece.")
        }
        VaultError::MissingUserKey => "O servidor não enviou a chave da conta. Contas com \
             login só por SSO ou dispositivo confiável ainda não são suportadas."
            .into(),
        other => format!("Não foi possível abrir o cofre: {other}"),
    }
}

/// Nome legível de um provedor de 2FA.
pub fn provider_name(p: TwoFactorProvider) -> &'static str {
    match p {
        TwoFactorProvider::Authenticator => "aplicativo autenticador",
        TwoFactorProvider::Email => "e-mail",
        TwoFactorProvider::Duo | TwoFactorProvider::OrganizationDuo => "Duo",
        TwoFactorProvider::Yubikey => "YubiKey",
        TwoFactorProvider::U2f | TwoFactorProvider::WebAuthn => "chave de segurança (FIDO)",
        TwoFactorProvider::Remember => "dispositivo lembrado",
        TwoFactorProvider::Other(_) => "outro método",
    }
}

/// Endereço do Duo que o servidor mandou para este login (`AuthUrl`).
pub fn duo_auth_url(pending: &PendingLogin, provider: TwoFactorProvider) -> Option<String> {
    let data = pending.provider_data(provider)?;
    data.get("authUrl")
        .or_else(|| data.get("AuthUrl"))
        .and_then(|v| v.as_str())
        .map(str::to_owned)
}

/// Monta o token do Duo (`code|state`) a partir do que a pessoa colou: o
/// endereço da página final (`…duo-redirect-connector.html?…&code=…&state=…`),
/// o link `bitwarden://duo-callback?code=…&state=…` ou o próprio `code|state`.
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

/// Instrução mostrada no passo de 2FA para cada método.
pub fn provider_prompt(p: TwoFactorProvider) -> String {
    match p {
        TwoFactorProvider::WebAuthn | TwoFactorProvider::U2f => {
            "Clique no botão e toque na sua chave de segurança (ou use o Windows Hello).".into()
        }
        TwoFactorProvider::Duo | TwoFactorProvider::OrganizationDuo => {
            "Aprove no Duo pelo navegador e cole aqui o endereço da página final.".into()
        }
        other => format!("Digite o código do seu {}.", provider_name(other)),
    }
}

/// Provedores que este cliente sabe usar (código digitado).
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
    // Autenticador primeiro: é o mais comum e não depende de e-mail.
    out.sort_by_key(|p| p.code());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

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
            "Senha mestra incorreta."
        );
        assert_eq!(
            describe_api_error(ApiError::InvalidCredentials("x".into())),
            "E-mail ou senha mestra incorretos."
        );
    }
}
