//! Test against a real server (preferably the Vaultwarden from `dev/`).
//! Ignored by default. To run it:
//!
//! ```text
//! SCOUT_VAULT_TEST_URL=http://localhost:8000 \
//! SCOUT_VAULT_TEST_EMAIL=test@example.com \
//! SCOUT_VAULT_TEST_PASSWORD='test account password' \
//! cargo test -p scout-vault-core --test live_server -- --ignored --nocapture
//! ```
//!
//! Use a TEST account without 2FA. Never run this against your real vault.

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

/// Creates, edits, trashes, restores and deletes a real item.
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
        collection_ids: Vec::new(),
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

    // Edit the password: the history has to show up on the server.
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

/// Attaches a file, downloads it, checks the contents, and deletes it.
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
        collection_ids: Vec::new(),
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

/// Deletes items that interrupted tests left in the test account.
#[tokio::test]
#[ignore = "precisa de um servidor; veja o comentário do arquivo"]
async fn cleanup_test_leftovers() {
    const PREFIXES: &[&str] = &[
        "Teste escrita ",
        "Com anexo ",
        "Pela UI ",
        "Anexo UI ",
        "Arrastado ",
        "teste-salvar-",
    ];
    const FOLDER_PREFIXES: &[&str] = &["Arrasto ", "Pasta UI ", "Pasta teste "];
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
    let vault = Vault::decrypt(&sync, &keys);
    for folder in &vault.folders {
        if FOLDER_PREFIXES.iter().any(|p| folder.name.starts_with(p)) {
            client.delete_folder(&token, &folder.id).await.unwrap();
            println!("pasta apagada: {}", folder.name);
        }
    }
    for item in vault.items {
        if PREFIXES.iter().any(|p| item.name.starts_with(p)) {
            if !item.deleted {
                client.trash_cipher(&token, &item.id).await.unwrap();
            }
            client.delete_cipher(&token, &item.id).await.unwrap();
            println!("apagado: {}", item.name);
        }
    }
}

/// Creates, renames and deletes a folder; an item inside it ends up with "no folder".
#[tokio::test]
#[ignore = "precisa de um servidor; veja o comentário do arquivo"]
async fn folder_create_rename_delete() {
    use scout_vault_core::edit::{encrypt_folder_name, encrypt_item, request_body};
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

    let name = format!("Pasta teste {}", std::process::id());
    let before = client.revision_date(&token).await.unwrap();
    let folder = client
        .create_folder(&token, &encrypt_folder_name(&name, &keys).unwrap())
        .await
        .unwrap();
    // The revision date moves when the vault changes (basis of background sync).
    let after = client.revision_date(&token).await.unwrap();
    assert!(after > before, "revisão {before} → {after}");
    let note = Item {
        id: String::new(),
        organization_id: None,
        folder_id: Some(folder.id.clone()),
        name: format!("Na pasta {}", std::process::id()),
        notes: None,
        favorite: false,
        reprompt: false,
        deleted: false,
        revision_date: None,
        attachments: Vec::new(),
        collection_ids: Vec::new(),
        fields: Vec::new(),
        kind: ItemKind::SecureNote,
    };
    let item = client
        .create_cipher(
            &token,
            &request_body(&encrypt_item(&note, None, &keys).unwrap(), &profile.id),
        )
        .await
        .unwrap();
    assert_eq!(item.folder_id.as_deref(), Some(folder.id.as_str()));

    // Moving without re-encrypting (drag to "No folder" and back), keeping the favorite.
    client
        .move_cipher_to_folder(&token, &item.id, None, true)
        .await
        .unwrap();
    let sync = client.sync(&token).await.unwrap();
    let c = sync.ciphers.iter().find(|c| c.id == item.id).unwrap();
    assert_eq!((c.folder_id.as_deref(), c.favorite), (None, true));
    client
        .move_cipher_to_folder(&token, &item.id, Some(&folder.id), false)
        .await
        .unwrap();
    let sync = client.sync(&token).await.unwrap();
    let c = sync.ciphers.iter().find(|c| c.id == item.id).unwrap();
    assert_eq!(c.folder_id.as_deref(), Some(folder.id.as_str()));

    client
        .rename_folder(
            &token,
            &folder.id,
            &encrypt_folder_name("Renomeada", &keys).unwrap(),
        )
        .await
        .unwrap();
    let vault = Vault::decrypt(&client.sync(&token).await.unwrap(), &keys);
    assert!(vault
        .folders
        .iter()
        .any(|f| f.id == folder.id && f.name == "Renomeada"));

    client.delete_folder(&token, &folder.id).await.unwrap();
    let sync = client.sync(&token).await.unwrap();
    assert!(sync.folders.iter().all(|f| f.id != folder.id));
    let moved = sync.ciphers.iter().find(|c| c.id == item.id).unwrap();
    assert_eq!(moved.folder_id, None, "o item fica sem pasta");

    client.trash_cipher(&token, &item.id).await.unwrap();
    client.delete_cipher(&token, &item.id).await.unwrap();
}

/// Organization item: creates it in a collection, changes collection, edits and deletes.
/// Needs the organization from `tools/dev_org.py`.
#[tokio::test]
#[ignore = "precisa de um servidor; veja o comentário do arquivo"]
async fn org_item_with_collections() {
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
    let token = client
        .login_password(&email, &mk.server_hash(&password), None, None)
        .await
        .unwrap()
        .access_token;
    let sync = client.sync(&token).await.unwrap();
    let keys = Keys::unlock(&mk, &sync.profile).unwrap();
    let vault = Vault::decrypt(&sync, &keys);
    let org = vault
        .organizations
        .iter()
        .find(|o| o.name == "Empresa de teste")
        .expect("rode tools/dev_org.py");
    let col = |name: &str| {
        vault
            .collections
            .iter()
            .find(|c| c.organization_id == org.id && c.name == name)
            .unwrap_or_else(|| panic!("coleção {name}"))
            .id
            .clone()
    };
    let (fin, ti) = (col("Financeiro"), col("TI"));

    let item = Item {
        id: String::new(),
        organization_id: Some(org.id.clone()),
        folder_id: None,
        name: format!("Da empresa {}", std::process::id()),
        notes: None,
        favorite: false,
        reprompt: false,
        deleted: false,
        revision_date: None,
        attachments: Vec::new(),
        collection_ids: vec![fin.clone()],
        fields: Vec::new(),
        kind: ItemKind::Login(Login {
            username: Some("financeiro".into()),
            password: Some(Secret::new("da-empresa".into())),
            totp: None,
            uris: vec![],
        }),
    };
    let body = request_body(&encrypt_item(&item, None, &keys).unwrap(), &sync.profile.id);
    let created = client
        .create_cipher_in_org(&token, &body, &item.collection_ids)
        .await
        .unwrap();

    let find = |s: &scout_vault_core::sync::SyncResponse| {
        Vault::decrypt(s, &keys)
            .items
            .into_iter()
            .find(|i| i.id == created.id)
            .unwrap()
    };
    let s = client.sync(&token).await.unwrap();
    let got = find(&s);
    assert_eq!(got.organization_id.as_deref(), Some(org.id.as_str()));
    assert_eq!(got.collection_ids, std::slice::from_ref(&fin));
    let ItemKind::Login(l) = &got.kind else {
        panic!()
    };
    assert_eq!(l.password.as_ref().unwrap().expose(), "da-empresa");

    // Switch to TI and edit the name.
    client
        .update_collections(&token, &created.id, std::slice::from_ref(&ti))
        .await
        .unwrap();
    let s = client.sync(&token).await.unwrap();
    let base = s
        .ciphers
        .iter()
        .find(|c| c.id == created.id)
        .unwrap()
        .clone();
    let mut edited = find(&s);
    assert_eq!(edited.collection_ids, std::slice::from_ref(&ti));
    edited.name.push_str(" (editado)");
    client
        .update_cipher(
            &token,
            &created.id,
            &request_body(
                &encrypt_item(&edited, Some(&base), &keys).unwrap(),
                &s.profile.id,
            ),
        )
        .await
        .unwrap();
    let s = client.sync(&token).await.unwrap();
    let after = find(&s);
    assert!(after.name.ends_with("(editado)"));
    assert_eq!(after.collection_ids, [ti], "editar não tira da coleção");

    client.trash_cipher(&token, &created.id).await.unwrap();
    client.delete_cipher(&token, &created.id).await.unwrap();
}
