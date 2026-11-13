//! Login com 2FA por autenticador e "lembrar este dispositivo" contra o
//! Vaultwarden local. Crie a conta com `python tools/dev_seed.py --totp` e rode:
//!
//! ```bash
//! set -a; . dev/test-account-2fa.env; set +a
//! cargo test -p cofre-app --test live_2fa -- --ignored
//! ```

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use cofre_app::flow::{self, LoginOutcome, PendingLogin};
use cofre_app::secrets::{remember_key, SecretStore};
use cofre_core::api::{Client, DeviceInfo, SecondFactor, ServerUrls, TwoFactorProvider};
use hmac::{Hmac, KeyInit, Mac};
use zeroize::Zeroizing;

fn env(k: &str) -> String {
    std::env::var(k).unwrap_or_else(|_| panic!("defina {k} (veja dev/test-account-2fa.env)"))
}

fn base32(s: &str) -> Vec<u8> {
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let (mut bits, mut acc, mut out) = (0u32, 0u64, Vec::new());
    for c in s.trim_end_matches('=').bytes() {
        let v = alphabet
            .iter()
            .position(|a| *a == c.to_ascii_uppercase())
            .unwrap() as u64;
        acc = (acc << 5) | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

/// Código TOTP (RFC 6238, SHA-1, 6 dígitos) para `offset` passos de 30 s a partir de agora.
fn totp(secret: &str, offset: i64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let counter = (now / 30 + offset) as u64;
    let mut mac = Hmac::<sha1::Sha1>::new_from_slice(&base32(secret)).unwrap();
    mac.update(&counter.to_be_bytes());
    let d = mac.finalize().into_bytes();
    let o = (d[19] & 0x0f) as usize;
    let code = (u32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]) & 0x7fff_ffff) % 1_000_000;
    format!("{code:06}")
}

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(f)
}

fn client(url: &str, device: &DeviceInfo) -> Arc<Client> {
    Arc::new(Client::new(ServerUrls::self_hosted(url), device.clone()).unwrap())
}

fn second(provider: TwoFactorProvider, token: String, remember: bool) -> SecondFactor {
    SecondFactor {
        provider,
        token,
        remember,
    }
}

fn expect_two_factor(outcome: Result<LoginOutcome, String>) -> PendingLogin {
    match outcome {
        Ok(LoginOutcome::NeedsTwoFactor { pending, providers }) => {
            assert!(providers.contains(&TwoFactorProvider::Authenticator));
            pending
        }
        Ok(LoginOutcome::Done(_)) => panic!("entrou sem pedir o 2FA"),
        Ok(LoginOutcome::WrongCode { message, .. }) => panic!("código recusado: {message}"),
        Ok(LoginOutcome::NeedsNewDeviceCode(_)) => panic!("pediu código de novo dispositivo"),
        Err(e) => panic!("erro: {e}"),
    }
}

#[test]
#[ignore = "precisa do Vaultwarden local; veja o comentário do arquivo"]
fn remember_device_skips_two_factor_next_time() {
    let (url, email, password, secret) = (
        env("COFRE_TEST_2FA_URL"),
        env("COFRE_TEST_2FA_EMAIL"),
        env("COFRE_TEST_2FA_PASSWORD"),
        env("COFRE_TEST_2FA_TOTP_SECRET"),
    );
    let device = DeviceInfo::new("Cofre (teste 2FA)");
    let secrets = SecretStore::default();
    let key = remember_key(&ServerUrls::self_hosted(&url).identity, &email);
    let pw = || Zeroizing::new(password.clone());

    block_on(async {
        // 1. Primeiro login: o servidor pede o autenticador.
        let pending = expect_two_factor(
            flow::start_login(client(&url, &device), email.clone(), pw(), secrets.clone()).await,
        );

        // 2. Código errado: continua no mesmo passo, sem refazer a KDF.
        let pending = match flow::continue_login(
            pending,
            Some(second(
                TwoFactorProvider::Authenticator,
                "000000".into(),
                true,
            )),
            None,
        )
        .await
        {
            Ok(LoginOutcome::WrongCode { pending, message }) => {
                assert!(message.contains("Código incorreto"), "{message}");
                pending
            }
            _ => panic!("esperava código incorreto"),
        };

        // 3. Código certo com "lembrar": entra e guarda o token.
        // (Usa o próximo passo de 30 s: o atual pode já ter sido usado pelo dev_seed.)
        let outcome = flow::continue_login(
            pending,
            Some(second(
                TwoFactorProvider::Authenticator,
                totp(&secret, 1),
                true,
            )),
            None,
        )
        .await;
        assert!(matches!(outcome, Ok(LoginOutcome::Done(_))), "não entrou");
        assert!(
            secrets.get(&key).is_some(),
            "o token de lembrar não foi guardado"
        );

        // 4. Segundo login (outra sessão, mesmo dispositivo): entra sem pedir código.
        let outcome =
            flow::start_login(client(&url, &device), email.clone(), pw(), secrets.clone()).await;
        assert!(
            matches!(outcome, Ok(LoginOutcome::Done(_))),
            "não usou o dispositivo lembrado"
        );

        // 5. Token inválido: volta a pedir o 2FA e descarta o token ruim.
        secrets.set(&key, "token-que-nao-vale");
        expect_two_factor(
            flow::start_login(client(&url, &device), email.clone(), pw(), secrets.clone()).await,
        );
        assert_eq!(secrets.get(&key), None, "token inválido não foi descartado");
    });
}
