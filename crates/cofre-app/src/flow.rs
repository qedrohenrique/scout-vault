//! Fluxos de login, desbloqueio e sincronização — sem nenhuma UI.
//!
//! As telas só chamam estas funções e mostram o resultado; assim a lógica
//! (inclusive as mensagens de erro) pode ser testada sem janela.

use std::sync::Arc;

use cofre_core::api::{ApiError, Client, SecondFactor, TwoFactorProvider};
use cofre_core::crypto::{CryptoError, MasterKey};
use cofre_core::sync::SyncResponse;
use cofre_core::vault::{Keys, Vault, VaultError};
use cofre_core::Kdf;
use zeroize::Zeroizing;

use crate::io;

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
}

impl PendingLogin {
    pub fn email(&self) -> &str {
        &self.email
    }
}

pub enum LoginOutcome {
    Done(Box<Unlocked>),
    NeedsTwoFactor {
        pending: PendingLogin,
        providers: Vec<TwoFactorProvider>,
    },
    NeedsNewDeviceCode(PendingLogin),
}

/// Primeira etapa: prelogin, KDF, login.
pub async fn start_login(
    client: Arc<Client>,
    email: String,
    password: Zeroizing<String>,
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

    let pending = PendingLogin {
        client,
        email,
        kdf,
        master_key: Arc::new(master_key),
        server_hash,
    };
    continue_login(pending, None, None).await
}

/// Reenvia o login com o segundo fator e/ou o código de novo dispositivo.
pub async fn continue_login(
    pending: PendingLogin,
    second_factor: Option<SecondFactor>,
    new_device_code: Option<String>,
) -> Result<LoginOutcome, String> {
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
        Err(ApiError::TwoFactorRequired(providers)) => {
            return Ok(LoginOutcome::NeedsTwoFactor { pending, providers })
        }
        Err(ApiError::NewDeviceVerificationRequired) => {
            return Ok(LoginOutcome::NeedsNewDeviceCode(pending))
        }
        Err(e) => return Err(describe_api_error(e)),
    };

    let sync = {
        let client = pending.client.clone();
        let token = tokens.access_token.clone();
        io::run(async move { client.sync(&token).await })
            .await
            .map_err(describe_api_error)?
    };
    let sync = Arc::new(sync);

    let (keys, vault) = {
        let mk = pending.master_key.clone();
        let sync = sync.clone();
        io::blocking(move || open_vault(&mk, &sync))
            .await
            .map_err(describe_vault_error)?
    };

    Ok(LoginOutcome::Done(Box::new(Unlocked {
        session: Session {
            client: pending.client,
            email: pending.email,
            kdf: pending.kdf,
            access_token: tokens.access_token,
            refresh_token: tokens.refresh_token,
            sync,
        },
        keys,
        vault,
    })))
}

/// Pede o código de 2FA por e-mail. O futuro não empresta `pending`, para a
/// tela poder continuar com ele enquanto o pedido roda.
pub fn send_email_code(
    pending: &PendingLogin,
) -> impl std::future::Future<Output = Result<(), String>> + 'static {
    let client = pending.client.clone();
    let email = pending.email.clone();
    let hash = pending.server_hash.clone();
    async move {
        io::run(async move { client.send_email_login_code(&email, &hash).await })
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
pub async fn resync(session: Session, keys: Arc<Keys>) -> Result<(Session, Vault), String> {
    let (access, refresh, sync) = {
        let client = session.client.clone();
        let refresh_token = session.refresh_token.clone();
        io::run(async move {
            let t = client.refresh(&refresh_token).await?;
            let sync = client.sync(&t.access_token).await?;
            Ok::<_, ApiError>((t.access_token, t.refresh_token, sync))
        })
        .await
        .map_err(describe_api_error)?
    };
    let sync = Arc::new(sync);
    let vault = {
        let sync = sync.clone();
        io::blocking(move || Vault::decrypt(&sync, &keys)).await
    };
    Ok((
        Session {
            access_token: access,
            refresh_token: refresh.unwrap_or(session.refresh_token.clone()),
            sync,
            ..session
        },
        vault,
    ))
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
        ApiError::Unauthorized => "A sessão expirou. Entre novamente.".into(),
        ApiError::TwoFactorRequired(_) => "O servidor pediu um segundo fator.".into(),
        ApiError::NewDeviceVerificationRequired => {
            "O servidor pediu o código de verificação enviado por e-mail.".into()
        }
        ApiError::Status { status, .. } => format!("O servidor recusou o pedido (HTTP {status})."),
        other => other.to_string(),
    }
}

pub fn describe_vault_error(e: VaultError) -> String {
    match e {
        VaultError::WrongPassword => "Senha mestra incorreta.".into(),
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
            )
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
        assert_eq!(
            got,
            [TwoFactorProvider::Authenticator, TwoFactorProvider::Email]
        );
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
