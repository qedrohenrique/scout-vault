//! Teste contra um servidor de verdade (de preferência o Vaultwarden de `dev/`).
//! Ignorado por padrão. Para rodar:
//!
//! ```text
//! COFRE_TEST_URL=http://localhost:8000 \
//! COFRE_TEST_EMAIL=teste@exemplo.com \
//! COFRE_TEST_PASSWORD='senha da conta de teste' \
//! cargo test -p cofre-core --test live_server -- --ignored --nocapture
//! ```
//!
//! Use uma conta de TESTE sem 2FA. Nunca rode isso com o seu cofre real.

use cofre_core::api::{Client, DeviceInfo, ServerUrls};
use cofre_core::crypto::MasterKey;
use cofre_core::vault::{Keys, Vault};

#[tokio::test]
#[ignore = "precisa de um servidor; veja o comentário do arquivo"]
async fn login_sync_and_decrypt() {
    let env = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("defina {k}"));
    let (url, email, password) = (
        env("COFRE_TEST_URL"),
        env("COFRE_TEST_EMAIL"),
        env("COFRE_TEST_PASSWORD"),
    );

    let client = Client::new(
        ServerUrls::self_hosted(&url),
        DeviceInfo::new("Cofre (teste)"),
    )
    .unwrap();

    let kdf = client.prelogin(&email).await.unwrap();
    println!("KDF: {kdf:?}");

    let master_key = MasterKey::derive(&password, &email, kdf).unwrap();
    let tokens = client
        .login_password(&email, &master_key.server_hash(&password), None, None)
        .await
        .unwrap();

    let refreshed = client.refresh(&tokens.refresh_token).await.unwrap();
    let sync = client.sync(&refreshed.access_token).await.unwrap();

    let keys = Keys::unlock(&master_key, &sync.profile).unwrap();
    let vault = Vault::decrypt(&sync, &keys);

    println!(
        "{} itens, {} pastas, {} erros",
        vault.items.len(),
        vault.folders.len(),
        vault.errors.len()
    );
    for item in &vault.items {
        println!("  - {}", item.name);
    }
    assert!(vault.errors.is_empty(), "{:?}", vault.errors);
}
