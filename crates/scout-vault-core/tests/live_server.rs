//! Teste contra um servidor de verdade (de preferência o Vaultwarden de `dev/`).
//! Ignorado por padrão. Para rodar:
//!
//! ```text
//! SCOUT_VAULT_TEST_URL=http://localhost:8000 \
//! SCOUT_VAULT_TEST_EMAIL=teste@exemplo.com \
//! SCOUT_VAULT_TEST_PASSWORD='senha da conta de teste' \
//! cargo test -p scout-vault-core --test live_server -- --ignored --nocapture
//! ```
//!
//! Use uma conta de TESTE sem 2FA. Nunca rode isso com o seu cofre real.

use scout_vault_core::api::{Client, DeviceInfo, ServerUrls};
use scout_vault_core::crypto::MasterKey;
use scout_vault_core::vault::{Keys, Vault};

#[tokio::test]
#[ignore = "precisa de um servidor; veja o comentário do arquivo"]
async fn login_sync_and_decrypt() {
    let env = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("defina {k}"));
    let (url, email, password) = (
        env("SCOUT_VAULT_TEST_URL"),
        env("SCOUT_VAULT_TEST_EMAIL"),
        env("SCOUT_VAULT_TEST_PASSWORD"),
    );

    let client = Client::new(
        ServerUrls::self_hosted(&url),
        DeviceInfo::new("Scout Vault (teste)"),
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

/// Cria, edita, manda para a lixeira, restaura e apaga um item de verdade.
#[tokio::test]
#[ignore = "precisa de um servidor; veja o comentário do arquivo"]
async fn create_edit_trash_restore_delete() {
    use scout_vault_core::edit::{encrypt_item, request_body};
    use scout_vault_core::vault::{Item, ItemKind, Login};
    use scout_vault_core::Secret;

    let env = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("defina {k}"));
    let (url, email, password) = (
        env("SCOUT_VAULT_TEST_URL"),
        env("SCOUT_VAULT_TEST_EMAIL"),
        env("SCOUT_VAULT_TEST_PASSWORD"),
    );
    let client = Client::new(
        ServerUrls::self_hosted(&url),
        DeviceInfo::new("Scout Vault (teste)"),
    )
    .unwrap();
    let kdf = client.prelogin(&email).await.unwrap();
    let mk = MasterKey::derive(&password, &email, kdf).unwrap();
    let t = client
        .login_password(&email, &mk.server_hash(&password), None, None)
        .await
        .unwrap();
    let token = t.access_token;
    let profile = client.sync(&token).await.unwrap().profile;
    let keys = Keys::unlock(&mk, &profile).unwrap();
    let me = profile.id.clone();

    let name = format!("Teste escrita {}", std::process::id());
    let item = Item {
        id: String::new(),
        organization_id: None,
        folder_id: None,
        name: name.clone(),
        notes: Some(Secret::new("nota ✓".into())),
        favorite: false,
        reprompt: false,
        deleted: false,
        revision_date: None,
        attachments: Vec::new(),
        fields: Vec::new(),
        kind: ItemKind::Login(Login {
            username: Some("quem".into()),
            password: Some(Secret::new("primeira".into())),
            totp: None,
            uris: vec!["https://escrita.example".into()],
        }),
    };
    let created = client
        .create_cipher(
            &token,
            &request_body(&encrypt_item(&item, None, &keys).unwrap(), &me),
        )
        .await
        .unwrap();
    println!("criado: {}", created.id);

    // Edita a senha: o histórico tem que aparecer no servidor.
    let find = |sync: &scout_vault_core::sync::SyncResponse| {
        sync.ciphers
            .iter()
            .find(|c| c.id == created.id)
            .cloned()
            .unwrap()
    };
    let sync = client.sync(&token).await.unwrap();
    let base = find(&sync);
    let mut edited = Vault::decrypt(&sync, &keys)
        .items
        .into_iter()
        .find(|i| i.id == created.id)
        .unwrap();
    if let ItemKind::Login(l) = &mut edited.kind {
        l.password = Some(Secret::new("segunda".into()));
    }
    client
        .update_cipher(
            &token,
            &created.id,
            &request_body(&encrypt_item(&edited, Some(&base), &keys).unwrap(), &me),
        )
        .await
        .unwrap();
    let sync = client.sync(&token).await.unwrap();
    let after = find(&sync);
    assert!(
        after
            .extra
            .get("passwordHistory")
            .and_then(|h| h.as_array())
            .is_some_and(|h| h.len() == 1),
        "histórico: {:?}",
        after.extra.get("passwordHistory")
    );
    let item = Vault::decrypt(&sync, &keys)
        .items
        .into_iter()
        .find(|i| i.id == created.id)
        .unwrap();
    let ItemKind::Login(l) = &item.kind else {
        panic!()
    };
    assert_eq!(l.password.as_ref().unwrap().expose(), "segunda");
    assert_eq!(item.notes.as_ref().unwrap().expose(), "nota ✓");

    client.trash_cipher(&token, &created.id).await.unwrap();
    assert!(find(&client.sync(&token).await.unwrap())
        .deleted_date
        .is_some());
    client.restore_cipher(&token, &created.id).await.unwrap();
    assert!(find(&client.sync(&token).await.unwrap())
        .deleted_date
        .is_none());
    client.trash_cipher(&token, &created.id).await.unwrap();
    client.delete_cipher(&token, &created.id).await.unwrap();
    let gone = client.sync(&token).await.unwrap();
    assert!(gone.ciphers.iter().all(|c| c.id != created.id));
}

/// Anexa um arquivo, baixa e confere o conteúdo, e apaga.
#[tokio::test]
#[ignore = "precisa de um servidor; veja o comentário do arquivo"]
async fn attachment_upload_download_delete() {
    use scout_vault_core::attachment::{attachment_key, decrypt_file, prepare, raw_attachments};
    use scout_vault_core::edit::{encrypt_item, request_body};
    use scout_vault_core::vault::{Item, ItemKind};

    let env = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("defina {k}"));
    let (url, email, password) = (
        env("SCOUT_VAULT_TEST_URL"),
        env("SCOUT_VAULT_TEST_EMAIL"),
        env("SCOUT_VAULT_TEST_PASSWORD"),
    );
    let client = Client::new(
        ServerUrls::self_hosted(&url),
        DeviceInfo::new("Scout Vault (teste)"),
    )
    .unwrap();
    let kdf = client.prelogin(&email).await.unwrap();
    let mk = MasterKey::derive(&password, &email, kdf).unwrap();
    let token = client
        .login_password(&email, &mk.server_hash(&password), None, None)
        .await
        .unwrap()
        .access_token;
    let profile = client.sync(&token).await.unwrap().profile;
    let keys = Keys::unlock(&mk, &profile).unwrap();

    let note = Item {
        id: String::new(),
        organization_id: None,
        folder_id: None,
        name: format!("Com anexo {}", std::process::id()),
        notes: None,
        favorite: false,
        reprompt: false,
        deleted: false,
        revision_date: None,
        attachments: Vec::new(),
        fields: Vec::new(),
        kind: ItemKind::SecureNote,
    };
    let created = client
        .create_cipher(
            &token,
            &request_body(&encrypt_item(&note, None, &keys).unwrap(), &profile.id),
        )
        .await
        .unwrap();

    let contents: Vec<u8> = (0..70_000u32).map(|i| (i * 7 % 251) as u8).collect();
    let new = prepare(&created, &keys, "relatório ção.pdf", &contents).unwrap();
    let up = client
        .create_attachment(
            &token,
            &created.id,
            &new.key,
            &new.file_name,
            new.data.len(),
        )
        .await
        .unwrap();
    println!(
        "envio: tipo {} id {}",
        up.file_upload_type, up.attachment_id
    );
    client
        .upload_attachment(&token, &created.id, &up, &new.file_name, new.data)
        .await
        .unwrap();

    let sync = client.sync(&token).await.unwrap();
    let cipher = sync.ciphers.iter().find(|c| c.id == created.id).unwrap();
    let item = Vault::decrypt(&sync, &keys)
        .items
        .into_iter()
        .find(|i| i.id == created.id)
        .unwrap();
    assert_eq!(item.attachments.len(), 1);
    assert_eq!(item.attachments[0].file_name, "relatório ção.pdf");
    let raw = raw_attachments(cipher).remove(0);
    let url = client
        .attachment_url(&token, &created.id, &raw.id)
        .await
        .unwrap();
    let bytes = client.download(&url).await.unwrap();
    let key = attachment_key(cipher, &raw, &keys).unwrap();
    assert_eq!(&decrypt_file(&bytes, &key).unwrap()[..], &contents[..]);

    client
        .delete_attachment(&token, &created.id, &raw.id)
        .await
        .unwrap();
    let sync = client.sync(&token).await.unwrap();
    let cipher = sync.ciphers.iter().find(|c| c.id == created.id).unwrap();
    assert!(raw_attachments(cipher).is_empty());
    client.trash_cipher(&token, &created.id).await.unwrap();
    client.delete_cipher(&token, &created.id).await.unwrap();
}

/// Apaga itens que testes interrompidos deixaram na conta de teste.
#[tokio::test]
#[ignore = "precisa de um servidor; veja o comentário do arquivo"]
async fn cleanup_test_leftovers() {
    const PREFIXES: &[&str] = &["Teste escrita ", "Com anexo ", "Pela UI ", "Anexo UI "];
    let env = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("defina {k}"));
    let (url, email, password) = (
        env("SCOUT_VAULT_TEST_URL"),
        env("SCOUT_VAULT_TEST_EMAIL"),
        env("SCOUT_VAULT_TEST_PASSWORD"),
    );
    let client = Client::new(
        ServerUrls::self_hosted(&url),
        DeviceInfo::new("Scout Vault (teste)"),
    )
    .unwrap();
    let kdf = client.prelogin(&email).await.unwrap();
    let mk = MasterKey::derive(&password, &email, kdf).unwrap();
    let token = client
        .login_password(&email, &mk.server_hash(&password), None, None)
        .await
        .unwrap()
        .access_token;
    let sync = client.sync(&token).await.unwrap();
    let keys = Keys::unlock(&mk, &sync.profile).unwrap();
    for item in Vault::decrypt(&sync, &keys).items {
        if PREFIXES.iter().any(|p| item.name.starts_with(p)) {
            if !item.deleted {
                client.trash_cipher(&token, &item.id).await.unwrap();
            }
            client.delete_cipher(&token, &item.id).await.unwrap();
            println!("apagado: {}", item.name);
        }
    }
}
