//! Escrita: transforma um [`Item`] editado no corpo que o servidor espera,
//! **cifrando cada campo** na máquina.
//!
//! Princípio: partir do item **como o servidor mandou** ([`CipherRaw`]) e só
//! trocar o que a pessoa editou. Tudo o que o Scout Vault não conhece (passkeys,
//! histórico de senhas, regras de URI, campos vinculados, anexos…) volta
//! intacto — editar no Scout Vault nunca apaga dados criados por outro cliente.

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::crypto::SymmetricKey;
use crate::sync::{CardRaw, CipherRaw, FieldRaw, IdentityRaw, LoginRaw, SshKeyRaw, UriRaw};
use crate::vault::{FieldKind, Item, ItemKind, Keys, VaultError};

/// Quantas senhas antigas o histórico guarda (igual aos clientes oficiais).
const PASSWORD_HISTORY_MAX: usize = 5;

/// Chaves que só existem na **resposta** do servidor. Mandá-las de volta pode
/// ser rejeitado (ex.: `attachments` tem outro formato no pedido).
const RESPONSE_ONLY: &[&str] = &[
    "id",
    "object",
    "attachments",
    "edit",
    "viewPassword",
    "permissions",
    "organizationUseTotp",
    "collectionIds",
    "creationDate",
    "revisionDate",
    "deletedDate",
    "data",
];

fn now_iso() -> String {
    // Formato do servidor: 2026-11-13T18:40:00.000Z (UTC).
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Conversão de dias desde 1970 para data civil (algoritmo de H. Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.000Z",
        rem / 3600,
        (rem / 60) % 60,
        rem % 60
    )
}

struct Enc<'a>(&'a SymmetricKey);

impl Enc<'_> {
    /// Cifra; vazio vira `None` (campo ausente, como nos clientes oficiais).
    fn opt(&self, value: Option<&str>) -> Result<Option<String>, VaultError> {
        match value.filter(|v| !v.is_empty()) {
            None => Ok(None),
            Some(v) => Ok(Some(self.0.encrypt_str(v)?.to_string())),
        }
    }
}

fn decrypt_opt(key: &SymmetricKey, field: &Option<String>) -> Option<String> {
    let enc = field.as_deref().filter(|s| !s.is_empty())?;
    key.decrypt_str(&enc.parse().ok()?)
        .ok()
        .map(|s| s.expose().to_owned())
}

fn kind_code(kind: &ItemKind) -> u8 {
    match kind {
        ItemKind::Login(_) => 1,
        ItemKind::SecureNote => 2,
        ItemKind::Card(_) => 3,
        ItemKind::Identity(_) => 4,
        ItemKind::SshKey(_) => 5,
        ItemKind::Unknown(n) => *n,
    }
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

/// Monta o item cifrado a partir do editado. `base` é o item atual do
/// servidor (edição) ou `None` (item novo).
pub fn encrypt_item(
    item: &Item,
    base: Option<&CipherRaw>,
    keys: &Keys,
) -> Result<CipherRaw, VaultError> {
    if let ItemKind::Unknown(n) = item.kind {
        return Err(VaultError::Unsupported(format!(
            "o Scout Vault ainda não edita itens do tipo {n}"
        )));
    }
    let mut raw = match base {
        Some(b) => b.clone(),
        None => CipherRaw {
            id: String::new(),
            organization_id: item.organization_id.clone(),
            folder_id: None,
            kind: kind_code(&item.kind),
            name: None,
            notes: None,
            key: None,
            favorite: false,
            reprompt: Some(0),
            deleted_date: None,
            revision_date: None,
            login: None,
            card: None,
            identity: None,
            ssh_key: None,
            fields: None,
            extra: Map::new(),
        },
    };

    // A chave do item: a própria (se tiver) ou a do dono.
    let owner_key = keys.for_owner(raw.organization_id.as_deref())?;
    let item_key;
    let key: &SymmetricKey = match &raw.key {
        Some(k) => {
            item_key = SymmetricKey::from_bytes(&owner_key.decrypt(&k.parse()?)?)?;
            &item_key
        }
        None => owner_key,
    };
    let enc = Enc(key);

    raw.name = Some(
        key.encrypt_str(if item.name.trim().is_empty() {
            "Sem nome"
        } else {
            item.name.trim()
        })?
        .to_string(),
    );
    raw.notes = enc.opt(item.notes.as_ref().map(|s| s.expose()))?;
    raw.favorite = item.favorite;
    raw.folder_id = item.folder_id.clone().filter(|f| !f.is_empty());
    raw.reprompt = Some(u8::from(item.reprompt));

    match &item.kind {
        ItemKind::Login(l) => {
            let mut login = raw.login.take().unwrap_or(LoginRaw {
                username: None,
                password: None,
                totp: None,
                uris: None,
                extra: Map::new(),
            });
            let old_password = decrypt_opt(key, &login.password);
            let new_password = l.password.as_ref().map(|p| p.expose().to_owned());
            let changed = old_password.as_deref().filter(|p| !p.is_empty())
                != new_password.as_deref().filter(|p| !p.is_empty());
            if changed && base.is_some() {
                // A senha antiga vai para o histórico (já cifrada: reaproveita o texto do servidor).
                if let Some(old_enc) = login.password.clone().filter(|_| old_password.is_some()) {
                    let history = raw
                        .extra
                        .entry("passwordHistory")
                        .or_insert_with(|| Value::Array(Vec::new()));
                    if !history.is_array() {
                        *history = Value::Array(Vec::new());
                    }
                    let list = history.as_array_mut().expect("é array");
                    list.insert(0, json!({ "password": old_enc, "lastUsedDate": now_iso() }));
                    list.truncate(PASSWORD_HISTORY_MAX);
                }
                login
                    .extra
                    .insert("passwordRevisionDate".into(), Value::String(now_iso()));
            }
            login.username = enc.opt(l.username.as_deref())?;
            login.password = enc.opt(new_password.as_deref())?;
            login.totp = enc.opt(l.totp.as_ref().map(|s| s.expose()))?;

            let old_uris = login.uris.take().unwrap_or_default();
            let mut uris = Vec::new();
            for (i, uri) in l
                .uris
                .iter()
                .map(|u| u.trim())
                .filter(|u| !u.is_empty())
                .enumerate()
            {
                // Mantém a regra de correspondência e o resto da URI que já existia nessa posição.
                let mut u = old_uris.get(i).cloned().unwrap_or(UriRaw {
                    uri: None,
                    match_type: None,
                    extra: Map::new(),
                });
                u.uri = Some(key.encrypt_str(uri)?.to_string());
                // Os clientes oficiais conferem este checksum para detectar URI adulterada.
                let checksum = base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    Sha256::digest(uri.as_bytes()),
                );
                u.extra.insert(
                    "uriChecksum".into(),
                    Value::String(key.encrypt_str(&checksum)?.to_string()),
                );
                uris.push(u);
            }
            login.uris = (!uris.is_empty()).then_some(uris);
            raw.login = Some(login);
        }
        ItemKind::SecureNote => {
            raw.extra
                .entry("secureNote")
                .or_insert_with(|| json!({ "type": 0 }));
        }
        ItemKind::Card(c) => {
            let mut card = raw.card.take().unwrap_or(CardRaw {
                cardholder_name: None,
                brand: None,
                number: None,
                exp_month: None,
                exp_year: None,
                code: None,
                extra: Map::new(),
            });
            card.cardholder_name = enc.opt(c.cardholder_name.as_deref())?;
            card.brand = enc.opt(c.brand.as_deref())?;
            card.number = enc.opt(c.number.as_ref().map(|s| s.expose()))?;
            card.exp_month = enc.opt(c.exp_month.as_deref())?;
            card.exp_year = enc.opt(c.exp_year.as_deref())?;
            card.code = enc.opt(c.code.as_ref().map(|s| s.expose()))?;
            raw.card = Some(card);
        }
        ItemKind::Identity(i) => {
            let mut id = raw.identity.take().unwrap_or_default();
            id.title = enc.opt(i.title.as_deref())?;
            id.first_name = enc.opt(i.first_name.as_deref())?;
            id.middle_name = enc.opt(i.middle_name.as_deref())?;
            id.last_name = enc.opt(i.last_name.as_deref())?;
            id.address1 = enc.opt(i.address1.as_deref())?;
            id.address2 = enc.opt(i.address2.as_deref())?;
            id.address3 = enc.opt(i.address3.as_deref())?;
            id.city = enc.opt(i.city.as_deref())?;
            id.state = enc.opt(i.state.as_deref())?;
            id.postal_code = enc.opt(i.postal_code.as_deref())?;
            id.country = enc.opt(i.country.as_deref())?;
            id.company = enc.opt(i.company.as_deref())?;
            id.email = enc.opt(i.email.as_deref())?;
            id.phone = enc.opt(i.phone.as_deref())?;
            id.username = enc.opt(i.username.as_deref())?;
            id.ssn = enc.opt(i.ssn.as_ref().map(|s| s.expose()))?;
            id.passport_number = enc.opt(i.passport_number.as_ref().map(|s| s.expose()))?;
            id.license_number = enc.opt(i.license_number.as_ref().map(|s| s.expose()))?;
            raw.identity = Some(IdentityRaw { ..id });
        }
        ItemKind::SshKey(k) => {
            let mut ssh = raw.ssh_key.take().unwrap_or(SshKeyRaw {
                private_key: None,
                public_key: None,
                key_fingerprint: None,
                extra: Map::new(),
            });
            ssh.private_key = enc.opt(k.private_key.as_ref().map(|s| s.expose()))?;
            ssh.public_key = enc.opt(k.public_key.as_deref())?;
            ssh.key_fingerprint = enc.opt(k.fingerprint.as_deref())?;
            raw.ssh_key = Some(ssh);
        }
        ItemKind::Unknown(_) => unreachable!("tratado no começo"),
    }

    let old_fields = raw.fields.take().unwrap_or_default();
    let mut fields = Vec::new();
    for (i, f) in item.fields.iter().enumerate() {
        let mut out = old_fields.get(i).cloned().unwrap_or(FieldRaw {
            kind: None,
            name: None,
            value: None,
            extra: Map::new(),
        });
        out.kind = Some(field_code(f.kind));
        out.name = enc.opt(f.name.as_deref())?;
        out.value = enc.opt(f.value.as_ref().map(|s| s.expose()))?;
        fields.push(out);
    }
    raw.fields = (!fields.is_empty()).then_some(fields);
    Ok(raw)
}

/// Nome de pasta cifrado com a user key (pastas são sempre pessoais).
pub fn encrypt_folder_name(name: &str, keys: &Keys) -> Result<String, VaultError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(VaultError::Unsupported("a pasta precisa de um nome".into()));
    }
    Ok(keys.user_key().encrypt_str(name)?.to_string())
}

/// Corpo JSON do `POST`/`PUT /api/ciphers`: o item sem as chaves que só
/// existem na resposta, mais `lastKnownRevisionDate` (o servidor recusa se
/// outro cliente mudou o item nesse meio-tempo) e `encryptedFor` (o id de
/// quem cifrou — `profile.id`; servidores recentes exigem).
pub fn request_body(raw: &CipherRaw, user_id: &str) -> Value {
    let mut v = serde_json::to_value(raw).unwrap_or(Value::Null);
    if let Value::Object(map) = &mut v {
        for k in RESPONSE_ONLY {
            map.remove(*k);
        }
        map.insert("encryptedFor".into(), Value::String(user_id.to_owned()));
        if let Some(rev) = &raw.revision_date {
            map.insert("lastKnownRevisionDate".into(), Value::String(rev.clone()));
        }
        // `null` explícito nos tipos que não são deste item.
        map.retain(|_, v| !v.is_null());
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::MasterKey;
    use crate::sync::SyncResponse;
    use crate::vault::{Login, Vault};
    use crate::{Kdf, Secret};

    const SYNC: &[u8] = include_bytes!("../tests/fixtures/sync.json");
    const VECTORS: &str = include_str!("../tests/fixtures/vectors.json");

    fn open() -> (SyncResponse, Keys, Vault) {
        let v: Value = serde_json::from_str(VECTORS).unwrap();
        let mk = MasterKey::derive(
            v["password"].as_str().unwrap(),
            v["email"].as_str().unwrap(),
            Kdf::Pbkdf2Sha256 { iterations: 5000 },
        )
        .unwrap();
        let sync = SyncResponse::from_json(SYNC).unwrap();
        let keys = Keys::unlock(&mk, &sync.profile).unwrap();
        let vault = Vault::decrypt(&sync, &keys);
        (sync, keys, vault)
    }

    /// Decifra o item cifrado de volta, como faria qualquer cliente.
    fn reopen(raw: CipherRaw, sync: &SyncResponse, keys: &Keys) -> Item {
        let mut s = sync.clone();
        s.ciphers = vec![raw];
        Vault::decrypt(&s, keys).items.remove(0)
    }

    #[test]
    fn editing_keeps_unknown_data_and_records_password_history() {
        let (mut sync, keys, vault) = open();
        let base_ix = sync.ciphers.iter().position(|c| c.id == "login-1").unwrap();
        // Simula dados de outro cliente que o Scout Vault não entende.
        sync.ciphers[base_ix]
            .extra
            .insert("fido2Credentials".into(), json!([{ "credentialId": "x" }]));
        sync.ciphers[base_ix]
            .extra
            .insert("collectionIds".into(), json!(["c1"]));
        let base = sync.ciphers[base_ix].clone();

        let mut item = vault
            .items
            .iter()
            .find(|i| i.id == "login-1")
            .unwrap()
            .clone();
        item.name = "Banco novo".into();
        if let ItemKind::Login(l) = &mut item.kind {
            l.password = Some(Secret::new("outra-senha".into()));
            l.uris = vec!["https://novo.example.com".into()];
        }
        let raw = encrypt_item(&item, Some(&base), &keys).unwrap();

        // O que não foi editado continua lá.
        assert_eq!(
            raw.extra["fido2Credentials"],
            json!([{ "credentialId": "x" }])
        );
        let history = raw.extra["passwordHistory"].as_array().unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(
            history[0]["password"],
            json!(base.login.as_ref().unwrap().password)
        );

        let back = reopen(raw.clone(), &sync, &keys);
        assert_eq!(back.name, "Banco novo");
        let ItemKind::Login(l) = back.kind else {
            panic!()
        };
        assert_eq!(l.password.unwrap().expose(), "outra-senha");
        assert_eq!(l.username.as_deref(), Some("pessoa"));
        assert_eq!(l.uris, ["https://novo.example.com"]);
        // Campo customizado preservado e ainda legível.
        assert_eq!(back.fields[0].value.as_ref().unwrap().expose(), "1234");

        let body = request_body(&raw, "user-1");
        assert!(body.get("id").is_none());
        assert!(body.get("collectionIds").is_none());
        assert!(body.get("lastKnownRevisionDate").is_some() || base.revision_date.is_none());
        assert_eq!(body["type"], 1);
        assert_eq!(body["encryptedFor"], "user-1");
    }

    #[test]
    fn item_with_its_own_key_is_reencrypted_with_it() {
        let (sync, keys, vault) = open();
        let base = sync
            .ciphers
            .iter()
            .find(|c| c.id == "login-item-key")
            .unwrap();
        let mut item = vault
            .items
            .iter()
            .find(|i| i.id == "login-item-key")
            .unwrap()
            .clone();
        item.notes = Some(Secret::new("nota nova".into()));
        let raw = encrypt_item(&item, Some(base), &keys).unwrap();
        assert_eq!(raw.key, base.key, "a chave do item é mantida");
        let back = reopen(raw, &sync, &keys);
        assert_eq!(back.notes.unwrap().expose(), "nota nova");
    }

    #[test]
    fn new_items_of_each_kind_round_trip() {
        let (sync, keys, _) = open();
        let mut item = Item {
            id: String::new(),
            organization_id: None,
            folder_id: None,
            name: "Novo login".into(),
            notes: None,
            favorite: true,
            reprompt: false,
            deleted: false,
            revision_date: None,
            attachments: Vec::new(),
            collection_ids: Vec::new(),
            fields: vec![crate::vault::Field {
                kind: FieldKind::Hidden,
                name: Some("PIN".into()),
                value: Some(Secret::new("9999".into())),
            }],
            kind: ItemKind::Login(Login {
                username: Some("eu".into()),
                password: Some(Secret::new("s3nh@".into())),
                totp: None,
                uris: vec!["https://a.example".into(), "  ".into()],
            }),
        };
        let raw = encrypt_item(&item, None, &keys).unwrap();
        assert!(raw.extra.get("passwordHistory").is_none());
        let back = reopen(raw, &sync, &keys);
        assert!(back.favorite);
        assert_eq!(back.fields[0].kind, FieldKind::Hidden);
        let ItemKind::Login(l) = back.kind else {
            panic!()
        };
        assert_eq!(l.uris.len(), 1, "URI em branco é descartada");

        item.kind = ItemKind::SecureNote;
        item.notes = Some(Secret::new("só uma nota".into()));
        let raw = encrypt_item(&item, None, &keys).unwrap();
        assert_eq!(raw.extra["secureNote"], json!({ "type": 0 }));
        assert_eq!(request_body(&raw, "user-1")["type"], 2);
        let back = reopen(raw, &sync, &keys);
        assert!(matches!(back.kind, ItemKind::SecureNote));
        assert_eq!(back.notes.unwrap().expose(), "só uma nota");
    }

    #[test]
    fn folder_names_round_trip() {
        let (mut sync, keys, _) = open();
        let enc = encrypt_folder_name("  Bancos ção ", &keys).unwrap();
        sync.folders = vec![crate::sync::FolderRaw {
            id: "f".into(),
            name: enc,
        }];
        let vault = Vault::decrypt(&sync, &keys);
        assert_eq!(vault.folders[0].name, "Bancos ção");
        assert!(encrypt_folder_name("   ", &keys).is_err());
    }

    #[test]
    fn iso_dates_look_right() {
        let d = now_iso();
        assert_eq!(d.len(), 24, "{d}");
        assert!(d.ends_with('Z') && d.as_bytes()[10] == b'T');
        assert!(d.starts_with("20"));
    }
}
