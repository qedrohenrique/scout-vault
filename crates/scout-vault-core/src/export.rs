//! Export the personal vault in Bitwarden's formats, so it opens in Bitwarden (and in
//! Scout Vault's own import):
//!
//! - **Password-protected JSON**: the JSON below, encrypted with a key derived from a
//!   password chosen for the file (PBKDF2-SHA256 + HKDF, AES-256-CBC + HMAC, the
//!   format Bitwarden calls "password protected").
//! - **JSON**: every item type, folders and custom fields, in plain text.
//! - **CSV**: logins and notes only (the CSV format has no columns for the rest).
//!
//! Like Bitwarden's "individual vault" export: organization items and the trash stay out.

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use serde_json::{json, Map, Value};

use crate::crypto::{CryptoError, MasterKey};
use crate::vault::{FieldKind, Item, ItemKind, Vault};
use crate::Kdf;

/// Iterations for password-protected exports (Bitwarden's default for accounts).
pub const PROTECT_ITERATIONS: u32 = 600_000;

/// The items that go in an export: personal and not in the trash.
pub fn personal_items(vault: &Vault) -> Vec<&Item> {
    vault
        .items
        .iter()
        .filter(|i| i.organization_id.is_none() && !i.deleted)
        .filter(|i| !matches!(i.kind, ItemKind::Unknown(_)))
        .collect()
}

fn field_code(kind: FieldKind) -> u8 {
    match kind {
        FieldKind::Text => 0,
        FieldKind::Hidden => 1,
        FieldKind::Boolean => 2,
        FieldKind::Linked => 3,
        FieldKind::Other(n) => n,
    }
}

fn item_json(item: &Item) -> Value {
    let s = |v: &Option<String>| v.clone().map(Value::String).unwrap_or(Value::Null);
    let sec = |v: &Option<crate::Secret>| {
        v.as_ref()
            .map(|s| Value::String(s.expose().to_owned()))
            .unwrap_or(Value::Null)
    };
    let mut out = Map::new();
    out.insert("id".into(), json!(item.id));
    out.insert("organizationId".into(), Value::Null);
    out.insert("folderId".into(), s(&item.folder_id));
    out.insert("reprompt".into(), json!(u8::from(item.reprompt)));
    out.insert("name".into(), json!(item.name));
    out.insert("notes".into(), sec(&item.notes));
    out.insert("favorite".into(), json!(item.favorite));
    out.insert(
        "fields".into(),
        Value::Array(
            item.fields
                .iter()
                .map(|f| {
                    json!({
                        "name": s(&f.name),
                        "value": sec(&f.value),
                        "type": field_code(f.kind),
                        "linkedId": null,
                    })
                })
                .collect(),
        ),
    );
    out.insert("collectionIds".into(), Value::Null);
    let (code, key, body) = match &item.kind {
        ItemKind::Login(l) => (
            1,
            "login",
            json!({
                "uris": l.uris.iter().map(|u| json!({ "match": null, "uri": u })).collect::<Vec<_>>(),
                "username": s(&l.username),
                "password": sec(&l.password),
                "totp": sec(&l.totp),
                "fido2Credentials": [],
            }),
        ),
        ItemKind::SecureNote => (2, "secureNote", json!({ "type": 0 })),
        ItemKind::Card(c) => (
            3,
            "card",
            json!({
                "cardholderName": s(&c.cardholder_name),
                "brand": s(&c.brand),
                "number": sec(&c.number),
                "expMonth": s(&c.exp_month),
                "expYear": s(&c.exp_year),
                "code": sec(&c.code),
            }),
        ),
        ItemKind::Identity(i) => (
            4,
            "identity",
            json!({
                "title": s(&i.title),
                "firstName": s(&i.first_name),
                "middleName": s(&i.middle_name),
                "lastName": s(&i.last_name),
                "address1": s(&i.address1),
                "address2": s(&i.address2),
                "address3": s(&i.address3),
                "city": s(&i.city),
                "state": s(&i.state),
                "postalCode": s(&i.postal_code),
                "country": s(&i.country),
                "company": s(&i.company),
                "email": s(&i.email),
                "phone": s(&i.phone),
                "ssn": sec(&i.ssn),
                "username": s(&i.username),
                "passportNumber": sec(&i.passport_number),
                "licenseNumber": sec(&i.license_number),
            }),
        ),
        ItemKind::SshKey(k) => (
            5,
            "sshKey",
            json!({
                "privateKey": sec(&k.private_key),
                "publicKey": s(&k.public_key),
                "keyFingerprint": s(&k.fingerprint),
            }),
        ),
        ItemKind::Unknown(n) => (*n, "unknown", Value::Null),
    };
    out.insert("type".into(), json!(code));
    out.insert(key.into(), body);
    Value::Object(out)
}

/// The vault as Bitwarden's unencrypted JSON export.
pub fn to_json(vault: &Vault) -> String {
    let items = personal_items(vault);
    let folders: Vec<Value> = vault
        .folders
        .iter()
        .map(|f| json!({ "id": f.id, "name": f.name }))
        .collect();
    let doc = json!({
        "encrypted": false,
        "folders": folders,
        "items": items.iter().map(|i| item_json(i)).collect::<Vec<_>>(),
    });
    serde_json::to_string_pretty(&doc).unwrap_or_default()
}

fn csv_field(text: &str) -> String {
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_owned()
    }
}

/// The vault as Bitwarden's CSV: logins and notes. Returns the text and how many items
/// were left out (cards, identities, SSH keys).
pub fn to_csv(vault: &Vault) -> (String, usize) {
    let folder_name = |id: &Option<String>| {
        id.as_ref()
            .and_then(|id| vault.folders.iter().find(|f| &f.id == id))
            .map(|f| f.name.clone())
            .unwrap_or_default()
    };
    let mut out = String::from(
        "folder,favorite,type,name,notes,fields,reprompt,login_uri,login_username,login_password,login_totp\n",
    );
    let mut skipped = 0;
    for item in personal_items(vault) {
        let (kind, login) = match &item.kind {
            ItemKind::Login(l) => ("login", Some(l)),
            ItemKind::SecureNote => ("note", None),
            _ => {
                skipped += 1;
                continue;
            }
        };
        let fields = item
            .fields
            .iter()
            .map(|f| {
                format!(
                    "{}: {}",
                    f.name.as_deref().unwrap_or_default(),
                    f.value.as_ref().map(|v| v.expose()).unwrap_or_default()
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let cells = [
            folder_name(&item.folder_id),
            if item.favorite { "1" } else { "" }.to_owned(),
            kind.to_owned(),
            item.name.clone(),
            item.notes
                .as_ref()
                .map(|n| n.expose().to_owned())
                .unwrap_or_default(),
            fields,
            if item.reprompt { "1" } else { "0" }.to_owned(),
            login.map(|l| l.uris.join(",")).unwrap_or_default(),
            login.and_then(|l| l.username.clone()).unwrap_or_default(),
            login
                .and_then(|l| l.password.as_ref().map(|p| p.expose().to_owned()))
                .unwrap_or_default(),
            login
                .and_then(|l| l.totp.as_ref().map(|p| p.expose().to_owned()))
                .unwrap_or_default(),
        ];
        let line: Vec<String> = cells.iter().map(|c| csv_field(c)).collect();
        out.push_str(&line.join(","));
        out.push('\n');
    }
    (out, skipped)
}

/// The key of a password-protected export: PBKDF2-SHA256(password, salt) stretched
/// with HKDF (the same steps as Bitwarden's "PIN key").
pub fn file_key(
    password: &str,
    salt: &str,
    kdf: Kdf,
) -> Result<crate::crypto::SymmetricKey, CryptoError> {
    Ok(MasterKey::derive_with_salt(password, salt, kdf)?.stretch())
}

/// Encrypts an export (the JSON above) with a password, in Bitwarden's
/// password-protected format.
pub fn protect(json_text: &str, password: &str, iterations: u32) -> Result<String, CryptoError> {
    let mut salt = [0u8; 16];
    getrandom::fill(&mut salt).map_err(|e| CryptoError::Kdf(e.to_string()))?;
    let salt = STANDARD.encode(salt);
    let key = file_key(password, &salt, Kdf::Pbkdf2Sha256 { iterations })?;
    let check = uuid::Uuid::new_v4().to_string();
    let doc = json!({
        "encrypted": true,
        "passwordProtected": true,
        "salt": salt,
        "kdfType": 0,
        "kdfIterations": iterations,
        "kdfMemory": null,
        "kdfParallelism": null,
        "encKeyValidation_DO_NOT_EDIT": key.encrypt_str(&check)?.to_string(),
        "data": key.encrypt_str(json_text)?.to_string(),
    });
    Ok(serde_json::to_string_pretty(&doc).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::{self, ImportError};
    use crate::vault::{Folder, Login};
    use crate::Secret;

    fn item(id: &str, name: &str, kind: ItemKind) -> Item {
        Item {
            id: id.into(),
            organization_id: None,
            folder_id: None,
            name: name.into(),
            notes: None,
            favorite: false,
            reprompt: false,
            deleted: false,
            revision_date: None,
            fields: Vec::new(),
            kind,
            attachments: Vec::new(),
            collection_ids: Vec::new(),
        }
    }

    fn vault() -> Vault {
        let mut login = item(
            "a",
            "Banco, \"principal\"",
            ItemKind::Login(Login {
                username: Some("ana".into()),
                password: Some(Secret::new("s3nh@,x".into())),
                totp: None,
                uris: vec!["https://bank.example".into()],
            }),
        );
        login.folder_id = Some("f1".into());
        login.favorite = true;
        login.fields.push(crate::vault::Field {
            kind: FieldKind::Hidden,
            name: Some("pin".into()),
            value: Some(Secret::new("1234".into())),
        });
        let mut note = item("b", "Wi-Fi", ItemKind::SecureNote);
        note.notes = Some(Secret::new("linha 1\nlinha 2".into()));
        let card = item("c", "Visa", ItemKind::Card(Default::default()));
        let mut org = item("d", "Da empresa", ItemKind::SecureNote);
        org.organization_id = Some("org".into());
        let mut trashed = item("e", "Lixo", ItemKind::SecureNote);
        trashed.deleted = true;
        Vault {
            folders: vec![Folder {
                id: "f1".into(),
                name: "Pessoal".into(),
            }],
            items: vec![login, note, card, org, trashed],
            ..Default::default()
        }
    }

    #[test]
    fn json_export_reads_back_with_everything() {
        let json = to_json(&vault());
        let parsed = import::parse(&json).unwrap();
        assert_eq!(parsed.format, import::Format::BitwardenJson);
        // Organization items and the trash stay out.
        let names: Vec<_> = parsed.items.iter().map(|i| i.item.name.as_str()).collect();
        assert_eq!(names, ["Banco, \"principal\"", "Wi-Fi", "Visa"]);
        let bank = &parsed.items[0];
        assert_eq!(bank.folder.as_deref(), Some("Pessoal"));
        assert!(bank.item.favorite);
        assert_eq!(bank.item.fields[0].kind, FieldKind::Hidden);
        let ItemKind::Login(l) = &bank.item.kind else {
            panic!()
        };
        assert_eq!(l.password.as_ref().unwrap().expose(), "s3nh@,x");
        assert_eq!(
            parsed.items[1].item.notes.as_ref().unwrap().expose(),
            "linha 1\nlinha 2"
        );
    }

    #[test]
    fn csv_export_quotes_and_reads_back() {
        let (csv, skipped) = to_csv(&vault());
        assert_eq!(skipped, 1, "the card has no CSV columns");
        let parsed = import::parse(&csv).unwrap();
        assert_eq!(parsed.format, import::Format::BitwardenCsv);
        assert_eq!(parsed.items.len(), 2);
        assert_eq!(parsed.items[0].item.name, "Banco, \"principal\"");
        assert_eq!(parsed.items[0].folder.as_deref(), Some("Pessoal"));
        let ItemKind::Login(l) = &parsed.items[0].item.kind else {
            panic!()
        };
        assert_eq!(l.password.as_ref().unwrap().expose(), "s3nh@,x");
        assert_eq!(
            parsed.items[1].item.notes.as_ref().unwrap().expose(),
            "linha 1\nlinha 2"
        );
    }

    #[test]
    fn protected_export_needs_its_password() {
        // Few iterations: the test only checks the format, not the strength.
        let file = protect(&to_json(&vault()), "senha do arquivo", 5_000).unwrap();
        assert!(!file.contains("s3nh@") && !file.contains("Banco"));
        let doc: Value = serde_json::from_str(&file).unwrap();
        assert_eq!(doc["passwordProtected"], true);
        assert_eq!(doc["kdfType"], 0);

        assert_eq!(import::parse(&file).err(), Some(ImportError::NeedsPassword));
        assert_eq!(
            import::parse_with_password(&file, "errada").err(),
            Some(ImportError::WrongPassword)
        );
        let parsed = import::parse_with_password(&file, "senha do arquivo").unwrap();
        assert_eq!(parsed.items.len(), 3);
        assert_eq!(parsed.items[0].item.name, "Banco, \"principal\"");
    }
}
