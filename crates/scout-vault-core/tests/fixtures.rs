//! Abre um cofre sintético gerado por `tools/gen_fixtures.py`, que usa a
//! biblioteca `cryptography` do Python — uma implementação independente da nossa.
//! Se os dois lados discordarem em qualquer detalhe (KDF, HKDF, MAC, padding,
//! RSA-OAEP, chave de item), este teste quebra.

use scout_vault_core::crypto::MasterKey;
use scout_vault_core::sync::SyncResponse;
use scout_vault_core::vault::{FieldKind, ItemKind, Keys, Vault, VaultError};
use scout_vault_core::Kdf;
use serde_json::Value;

fn vectors() -> Value {
    serde_json::from_str(include_str!("fixtures/vectors.json")).unwrap()
}

fn sync() -> SyncResponse {
    SyncResponse::from_json(include_bytes!("fixtures/sync.json")).unwrap()
}

fn pbkdf2_master_key(v: &Value) -> MasterKey {
    let kdf = Kdf::Pbkdf2Sha256 {
        iterations: v["pbkdf2"]["iterations"].as_u64().unwrap() as u32,
    };
    MasterKey::derive(
        v["password"].as_str().unwrap(),
        v["email"].as_str().unwrap(),
        kdf,
    )
    .unwrap()
}

#[test]
fn pbkdf2_server_hash_matches_python() {
    let v = vectors();
    let mk = pbkdf2_master_key(&v);
    assert_eq!(
        mk.server_hash(v["password"].as_str().unwrap()),
        v["pbkdf2"]["server_hash"].as_str().unwrap()
    );
}

#[test]
fn argon2id_server_hash_matches_python() {
    let v = vectors();
    let a = &v["argon2id"];
    let kdf = Kdf::Argon2id {
        iterations: a["iterations"].as_u64().unwrap() as u32,
        memory_mib: a["memory_mib"].as_u64().unwrap() as u32,
        parallelism: a["parallelism"].as_u64().unwrap() as u32,
    };
    let mk = MasterKey::derive(
        v["password"].as_str().unwrap(),
        v["email"].as_str().unwrap(),
        kdf,
    )
    .unwrap();
    assert_eq!(
        mk.server_hash(v["password"].as_str().unwrap()),
        a["server_hash"].as_str().unwrap()
    );
}

#[test]
fn wrong_password_is_reported() {
    let v = vectors();
    let mk = MasterKey::derive(
        "senha errada",
        v["email"].as_str().unwrap(),
        Kdf::Pbkdf2Sha256 { iterations: 5000 },
    )
    .unwrap();
    assert_eq!(
        Keys::unlock(&mk, &sync().profile).unwrap_err(),
        VaultError::WrongPassword
    );
}

#[test]
fn opens_the_whole_vault() {
    let v = vectors();
    let sync = sync();
    let keys = Keys::unlock(&pbkdf2_master_key(&v), &sync.profile).unwrap();

    // A organização com chave lixo falha sozinha, sem derrubar as outras.
    assert_eq!(keys.failed_orgs.len(), 1);
    assert_eq!(keys.failed_orgs[0].0, "org-quebrada");

    let vault = Vault::decrypt(&sync, &keys);
    let item = |id: &str| vault.items.iter().find(|i| i.id == id).unwrap();

    assert_eq!(vault.folders.len(), 1);
    assert_eq!(vault.folders[0].name, "Bancos");

    // Login pessoal, vindo em PascalCase.
    let banco = item("login-1");
    assert_eq!(banco.name, "Meu Banco");
    assert_eq!(banco.folder_id.as_deref(), Some("folder-1"));
    assert!(banco.favorite && banco.reprompt && !banco.deleted);
    assert_eq!(
        banco.notes.as_ref().unwrap().expose(),
        "nota com\nduas linhas"
    );
    let ItemKind::Login(login) = &banco.kind else {
        panic!("esperava login")
    };
    assert_eq!(login.username.as_deref(), Some("pessoa"));
    assert_eq!(login.password.as_ref().unwrap().expose(), "s3nh@-ção");
    assert!(login
        .totp
        .as_ref()
        .unwrap()
        .expose()
        .starts_with("otpauth://"));
    assert_eq!(login.uris, ["https://banco.example.com"]);
    assert_eq!(banco.fields.len(), 1);
    assert_eq!(banco.fields[0].kind, FieldKind::Hidden);
    assert_eq!(banco.fields[0].name.as_deref(), Some("PIN"));
    assert_eq!(banco.fields[0].value.as_ref().unwrap().expose(), "1234");

    // Item com chave própria.
    let ik = item("login-item-key");
    assert_eq!(ik.name, "Com chave própria");
    let ItemKind::Login(l) = &ik.kind else {
        panic!()
    };
    assert_eq!(l.password.as_ref().unwrap().expose(), "item-key-pw");

    // Cartão de organização (chave via RSA-OAEP-SHA1).
    let card = item("card-org");
    assert_eq!(card.name, "Cartão da empresa");
    let ItemKind::Card(c) = &card.kind else {
        panic!()
    };
    assert_eq!(c.cardholder_name.as_deref(), Some("Ana Souza"));
    assert_eq!(c.number.as_ref().unwrap().expose(), "4111111111111111");
    assert_eq!(c.code.as_ref().unwrap().expose(), "123");
    assert_eq!(
        (c.exp_month.as_deref(), c.exp_year.as_deref()),
        (Some("12"), Some("2030"))
    );

    // Identidade: "SSN" em sigla precisa virar "ssn".
    let ItemKind::Identity(id) = &item("identity-1").kind else {
        panic!()
    };
    assert_eq!(id.first_name.as_deref(), Some("Ana"));
    assert_eq!(id.ssn.as_ref().unwrap().expose(), "123-45-6789");

    // Chave SSH.
    let ItemKind::SshKey(ssh) = &item("ssh-1").kind else {
        panic!()
    };
    assert_eq!(ssh.fingerprint.as_deref(), Some("SHA256:fake"));
    assert!(ssh
        .private_key
        .as_ref()
        .unwrap()
        .expose()
        .contains("OPENSSH"));

    // Tipo desconhecido continua listado.
    assert!(matches!(item("future-type").kind, ItemKind::Unknown(99)));

    // Lixeira: decifra, mas fica fora da busca.
    assert!(item("note-deleted").deleted);
    assert!(vault.search("").all(|i| !i.deleted));

    // Busca por nome, usuário e URI.
    let ids = |q: &str| vault.search(q).map(|i| i.id.as_str()).collect::<Vec<_>>();
    assert_eq!(ids("BANCO.example"), ["login-1"]);
    assert_eq!(ids("pessoa"), ["login-1"]);
    assert_eq!(ids("ana souza"), ["card-org"]);

    // Falhas isoladas: MAC adulterado e organização sem chave.
    let mut errors: Vec<_> = vault
        .errors
        .iter()
        .map(|(id, e)| (id.as_str(), e.clone()))
        .collect();
    errors.sort_by_key(|(id, _)| *id);
    assert_eq!(errors.len(), 2, "{errors:?}");
    assert_eq!(errors[0].0, "corrupted");
    assert_eq!(
        errors[1],
        ("orphan", VaultError::MissingOrgKey("org-quebrada".into()))
    );
}
