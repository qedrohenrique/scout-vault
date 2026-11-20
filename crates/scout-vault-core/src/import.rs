//! Import from other password managers: reads the file, turns it into vault items
//! (still in plain text, in memory) and builds the encrypted body for the server's
//! bulk import (`POST /api/ciphers/import`).
//!
//! Formats:
//! - **Bitwarden JSON** (unencrypted export): every item type, folders, custom fields.
//! - **Bitwarden CSV**: logins and notes, with folder, favorite and fields.
//! - **Browser CSV** (Chrome, Edge, Brave, Opera, Firefox, Safari) and other CSVs with
//!   the usual columns (`name`/`title`, `url`, `username`, `password`, `notes`, `otpauth`).
//!
//! Password-protected exports aren't read yet ([`ImportError::Encrypted`]).

use std::collections::HashMap;

use serde_json::{json, Value};

use crate::edit::{encrypt_folder_name, encrypt_item, request_body};
use crate::vault::{
    Card, Field, FieldKind, Folder, Identity, Item, ItemKind, Keys, Login, SshKey, Vault,
    VaultError,
};
use crate::Secret;

/// What the file turned out to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    BitwardenJson,
    BitwardenCsv,
    Chrome,
    Firefox,
    Safari,
    /// A CSV with the usual columns, from another program.
    Csv,
}

impl Format {
    pub fn name(self) -> &'static str {
        match self {
            Self::BitwardenJson => "Bitwarden (JSON)",
            Self::BitwardenCsv => "Bitwarden (CSV)",
            Self::Chrome => "Chrome / Edge / Brave (CSV)",
            Self::Firefox => "Firefox (CSV)",
            Self::Safari => "Safari / 1Password (CSV)",
            Self::Csv => "CSV",
        }
    }
}

/// An item read from the file, and the folder it was in.
#[derive(Debug, Clone)]
pub struct Imported {
    pub item: Item,
    pub folder: Option<String>,
}

/// Why an entry of the file was left out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Skip {
    /// Nothing worth saving (no name, address, username, password or notes).
    Empty,
    /// An item type this app doesn't import.
    UnknownType(String),
}

#[derive(Debug, Clone)]
pub struct Parsed {
    pub format: Format,
    pub items: Vec<Imported>,
    /// (line or position in the file, why).
    pub skipped: Vec<(usize, Skip)>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ImportError {
    #[error("the export is protected by a password")]
    Encrypted,
    #[error("unknown file format")]
    Unknown,
    #[error("the file is damaged: {0}")]
    Invalid(String),
}

/// Reads an export file.
pub fn parse(text: &str) -> Result<Parsed, ImportError> {
    let text = text.trim_start_matches('\u{feff}');
    if text.trim_start().starts_with('{') {
        return parse_bitwarden_json(text);
    }
    parse_csv_export(text)
}

fn blank(name: String, kind: ItemKind) -> Item {
    Item {
        id: String::new(),
        organization_id: None,
        folder_id: None,
        name,
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

fn some(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_owned())
}

fn secret(s: &str) -> Option<Secret> {
    (!s.is_empty()).then(|| Secret::new(s.to_owned()))
}

// ---------------------------------------------------------------------------
// Bitwarden JSON

fn parse_bitwarden_json(text: &str) -> Result<Parsed, ImportError> {
    let v: Value = serde_json::from_str(text).map_err(|e| ImportError::Invalid(e.to_string()))?;
    if v["encrypted"].as_bool() == Some(true) {
        return Err(ImportError::Encrypted);
    }
    let Some(list) = v["items"].as_array() else {
        return Err(ImportError::Unknown);
    };
    let folders: HashMap<&str, &str> = v["folders"]
        .as_array()
        .map(|f| {
            f.iter()
                .filter_map(|f| Some((f["id"].as_str()?, f["name"].as_str()?)))
                .collect()
        })
        .unwrap_or_default();
    let s = |v: &Value| v.as_str().unwrap_or_default().to_owned();
    let opt = |v: &Value| v.as_str().and_then(some);
    let sec = |v: &Value| v.as_str().and_then(secret);

    let mut items = Vec::new();
    let mut skipped = Vec::new();
    for (ix, e) in list.iter().enumerate() {
        let kind = match e["type"].as_u64() {
            Some(1) => {
                let l = &e["login"];
                ItemKind::Login(Login {
                    username: opt(&l["username"]),
                    password: sec(&l["password"]),
                    totp: sec(&l["totp"]),
                    uris: l["uris"]
                        .as_array()
                        .map(|u| u.iter().filter_map(|u| opt(&u["uri"])).collect())
                        .unwrap_or_default(),
                })
            }
            Some(2) => ItemKind::SecureNote,
            Some(3) => {
                let c = &e["card"];
                ItemKind::Card(Card {
                    cardholder_name: opt(&c["cardholderName"]),
                    brand: opt(&c["brand"]),
                    number: sec(&c["number"]),
                    exp_month: opt(&c["expMonth"]),
                    exp_year: opt(&c["expYear"]),
                    code: sec(&c["code"]),
                })
            }
            Some(4) => {
                let i = &e["identity"];
                ItemKind::Identity(Box::new(Identity {
                    title: opt(&i["title"]),
                    first_name: opt(&i["firstName"]),
                    middle_name: opt(&i["middleName"]),
                    last_name: opt(&i["lastName"]),
                    address1: opt(&i["address1"]),
                    address2: opt(&i["address2"]),
                    address3: opt(&i["address3"]),
                    city: opt(&i["city"]),
                    state: opt(&i["state"]),
                    postal_code: opt(&i["postalCode"]),
                    country: opt(&i["country"]),
                    company: opt(&i["company"]),
                    email: opt(&i["email"]),
                    phone: opt(&i["phone"]),
                    username: opt(&i["username"]),
                    ssn: sec(&i["ssn"]),
                    passport_number: sec(&i["passportNumber"]),
                    license_number: sec(&i["licenseNumber"]),
                }))
            }
            Some(5) => {
                let k = &e["sshKey"];
                ItemKind::SshKey(SshKey {
                    private_key: sec(&k["privateKey"]),
                    public_key: opt(&k["publicKey"]),
                    fingerprint: opt(&k["keyFingerprint"]),
                })
            }
            other => {
                let label = other
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| s(&e["type"]));
                skipped.push((ix + 1, Skip::UnknownType(label)));
                continue;
            }
        };
        let mut item = blank(s(&e["name"]).trim().to_owned(), kind);
        item.notes = e["notes"].as_str().and_then(secret);
        item.favorite = e["favorite"].as_bool().unwrap_or(false);
        item.reprompt = e["reprompt"].as_u64().unwrap_or(0) == 1;
        item.fields = e["fields"]
            .as_array()
            .map(|f| {
                f.iter()
                    .map(|f| Field {
                        kind: match f["type"].as_u64().unwrap_or(0) {
                            1 => FieldKind::Hidden,
                            2 => FieldKind::Boolean,
                            3 => FieldKind::Linked,
                            _ => FieldKind::Text,
                        },
                        name: opt(&f["name"]),
                        value: sec(&f["value"]),
                    })
                    .collect()
            })
            .unwrap_or_default();
        if item.name.is_empty() {
            match fallback_name(&item) {
                Some(name) => item.name = name,
                None => {
                    skipped.push((ix + 1, Skip::Empty));
                    continue;
                }
            }
        }
        let folder = e["folderId"]
            .as_str()
            .and_then(|id| folders.get(id))
            .and_then(|n| some(n));
        items.push(Imported { item, folder });
    }
    Ok(Parsed {
        format: Format::BitwardenJson,
        items,
        skipped,
    })
}

/// A name for an item that came without one: the site, else the username.
fn fallback_name(item: &Item) -> Option<String> {
    match &item.kind {
        ItemKind::Login(l) => l
            .uris
            .first()
            .map(|u| host_of(u))
            .or_else(|| l.username.clone()),
        _ => None,
    }
}

fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let host = host.rsplit('@').next().unwrap_or(host);
    host.trim_start_matches("www.").to_owned()
}

// ---------------------------------------------------------------------------
// CSV

/// RFC 4180: commas, quotes (`""` inside quotes) and line breaks inside quotes.
pub fn read_csv(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    field.push('"');
                    chars.next();
                }
                '"' => quoted = false,
                _ => field.push(c),
            }
            continue;
        }
        match c {
            '"' => quoted = true,
            ',' => row.push(std::mem::take(&mut field)),
            '\r' => {}
            '\n' => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            _ => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows.retain(|r| r.iter().any(|f| !f.trim().is_empty()));
    rows
}

fn parse_csv_export(text: &str) -> Result<Parsed, ImportError> {
    let rows = read_csv(text);
    let Some((header, rows)) = rows.split_first() else {
        return Err(ImportError::Unknown);
    };
    let header: Vec<String> = header.iter().map(|h| h.trim().to_lowercase()).collect();
    let col = |names: &[&str]| {
        names
            .iter()
            .find_map(|n| header.iter().position(|h| h == n))
    };
    let has = |n: &str| header.iter().any(|h| h == n);

    let format = if has("login_uri") && has("type") {
        Format::BitwardenCsv
    } else if has("formactionorigin") || has("httprealm") {
        Format::Firefox
    } else if has("otpauth") && has("title") {
        Format::Safari
    } else if has("name") && has("url") && has("username") && has("password") {
        Format::Chrome
    } else {
        Format::Csv
    };
    let name = col(&["name", "title"]);
    let url = col(&["login_uri", "url", "website", "uri", "login url"]);
    let username = col(&["login_username", "username", "login", "user name", "email"]);
    let password = col(&["login_password", "password"]);
    let notes = col(&["notes", "note", "extra", "comments"]);
    let totp = col(&["login_totp", "otpauth", "totp", "one-time password"]);
    let folder = col(&["folder", "group", "grouping"]);
    let favorite = col(&["favorite", "fav"]);
    let kind = col(&["type"]);
    let fields = col(&["fields"]);
    let reprompt = col(&["reprompt"]);
    if url.is_none() && password.is_none() && username.is_none() {
        return Err(ImportError::Unknown);
    }

    let mut items = Vec::new();
    let mut skipped = Vec::new();
    for (ix, row) in rows.iter().enumerate() {
        // Line in the file (the header is line 1).
        let line = ix + 2;
        let get = |c: Option<usize>| c.and_then(|c| row.get(c)).map(String::as_str).unwrap_or("");
        let kind_name = get(kind).trim().to_lowercase();
        let item_kind = match kind_name.as_str() {
            "" | "login" => ItemKind::Login(Login {
                username: some(get(username)),
                password: secret(get(password)),
                totp: some(get(totp)).map(Secret::new),
                uris: get(url).split([',', '\n']).filter_map(some).collect(),
            }),
            "note" => ItemKind::SecureNote,
            other => {
                skipped.push((line, Skip::UnknownType(other.to_owned())));
                continue;
            }
        };
        let mut item = blank(get(name).trim().to_owned(), item_kind);
        item.notes = secret(get(notes).trim_end());
        item.favorite = matches!(get(favorite).trim(), "1" | "true" | "yes");
        item.reprompt = get(reprompt).trim() == "1";
        item.fields = get(fields)
            .lines()
            .filter_map(|l| {
                let (n, v) = l.split_once(": ").or_else(|| l.split_once(':'))?;
                Some(Field {
                    kind: FieldKind::Text,
                    name: some(n),
                    value: secret(v),
                })
            })
            .collect();
        if item.name.is_empty() {
            match fallback_name(&item) {
                Some(n) => item.name = n,
                None if item.notes.is_some() => item.name = format!("#{line}"),
                None => {
                    skipped.push((line, Skip::Empty));
                    continue;
                }
            }
        }
        let empty_login = matches!(&item.kind, ItemKind::Login(l)
            if l.username.is_none() && l.password.is_none() && l.uris.is_empty() && l.totp.is_none())
            && item.notes.is_none();
        if empty_login {
            skipped.push((line, Skip::Empty));
            continue;
        }
        items.push(Imported {
            item,
            folder: some(get(folder)),
        });
    }
    Ok(Parsed {
        format,
        items,
        skipped,
    })
}

// ---------------------------------------------------------------------------
// Duplicates and the request

fn same(a: Option<&str>, b: Option<&str>) -> bool {
    a.map(str::trim).unwrap_or_default() == b.map(str::trim).unwrap_or_default()
}

/// Is this item already in the vault (same type, name and main data)?
pub fn already_in(vault: &Vault, item: &Item) -> bool {
    vault
        .items
        .iter()
        .filter(|v| !v.deleted && v.name.trim().eq_ignore_ascii_case(item.name.trim()))
        .any(|v| match (&v.kind, &item.kind) {
            (ItemKind::Login(a), ItemKind::Login(b)) => {
                same(a.username.as_deref(), b.username.as_deref())
                    && same(
                        a.password.as_ref().map(|s| s.expose()),
                        b.password.as_ref().map(|s| s.expose()),
                    )
            }
            (ItemKind::Card(a), ItemKind::Card(b)) => same(
                a.number.as_ref().map(|s| s.expose()),
                b.number.as_ref().map(|s| s.expose()),
            ),
            (ItemKind::SshKey(a), ItemKind::SshKey(b)) => {
                same(a.public_key.as_deref(), b.public_key.as_deref())
            }
            (ItemKind::SecureNote, ItemKind::SecureNote) => same(
                v.notes.as_ref().map(|s| s.expose()),
                item.notes.as_ref().map(|s| s.expose()),
            ),
            (ItemKind::Identity(_), ItemKind::Identity(_)) => true,
            _ => false,
        })
}

/// The encrypted body of `POST /api/ciphers/import`: the items, the folders (an
/// existing folder with the same name is reused) and which item goes in which folder.
pub fn request(
    items: &[Imported],
    existing: &[Folder],
    keys: &Keys,
    user_id: &str,
) -> Result<Value, VaultError> {
    let mut folders: Vec<Value> = Vec::new();
    let mut folder_ix: HashMap<String, usize> = HashMap::new();
    let mut relations = Vec::new();
    let mut ciphers = Vec::with_capacity(items.len());
    for (ix, imported) in items.iter().enumerate() {
        let mut item = imported.item.clone();
        item.id.clear();
        item.organization_id = None;
        item.folder_id = None;
        item.collection_ids.clear();
        let raw = encrypt_item(&item, None, keys)?;
        ciphers.push(request_body(&raw, user_id));
        if let Some(name) = &imported.folder {
            let f = match folder_ix.get(name) {
                Some(f) => *f,
                None => {
                    let mut folder = json!({ "name": encrypt_folder_name(name, keys)? });
                    if let Some(old) = existing.iter().find(|f| f.name.trim() == name.trim()) {
                        folder["id"] = json!(old.id);
                    }
                    folders.push(folder);
                    folder_ix.insert(name.clone(), folders.len() - 1);
                    folders.len() - 1
                }
            };
            relations.push(json!({ "key": ix, "value": f }));
        }
    }
    Ok(json!({
        "ciphers": ciphers,
        "folders": folders,
        "folderRelationships": relations,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::MasterKey;
    use crate::sync::{CipherRaw, SyncResponse};
    use crate::Kdf;

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

    fn login(item: &Item) -> &Login {
        match &item.kind {
            ItemKind::Login(l) => l,
            _ => panic!("not a login"),
        }
    }

    #[test]
    fn csv_reader_handles_quotes_commas_and_line_breaks() {
        let rows = read_csv("a,b,c\r\n\"x, y\",\"he said \"\"hi\"\"\",\"two\nlines\"\n\n1,2,3");
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1], ["x, y", "he said \"hi\"", "two\nlines"]);
        assert_eq!(rows[2], ["1", "2", "3"]);
    }

    #[test]
    fn chrome_csv() {
        let csv = "\u{feff}name,url,username,password,note\n\
                   github.com,https://github.com/login,ana,s3nha,\n\
                   ,https://www.example.com/x,bob,pw,lembrete\n\
                   ,,,,\n";
        let p = parse(csv).unwrap();
        assert_eq!(p.format, Format::Chrome);
        assert_eq!(p.items.len(), 2);
        let a = &p.items[0].item;
        assert_eq!(a.name, "github.com");
        assert_eq!(login(a).username.as_deref(), Some("ana"));
        assert_eq!(login(a).password.as_ref().unwrap().expose(), "s3nha");
        assert_eq!(login(a).uris, ["https://github.com/login"]);
        // No name: the site's.
        let b = &p.items[1].item;
        assert_eq!(b.name, "example.com");
        assert_eq!(b.notes.as_ref().unwrap().expose(), "lembrete");
    }

    #[test]
    fn firefox_csv_has_no_names() {
        let csv = "\"url\",\"username\",\"password\",\"httpRealm\",\"formActionOrigin\",\"guid\"\n\
                   \"https://mail.example.org\",\"eu\",\"x\",,\"\",\"{1}\"\n";
        let p = parse(csv).unwrap();
        assert_eq!(p.format, Format::Firefox);
        assert_eq!(p.items[0].item.name, "mail.example.org");
    }

    #[test]
    fn bitwarden_csv_with_folders_notes_and_fields() {
        let csv = "folder,favorite,type,name,notes,fields,reprompt,login_uri,login_username,login_password,login_totp\n\
                   Trabalho,1,login,Jira,,\"team: core\nid: 7\",1,\"https://a.example,https://b.example\",ana,pw,JBSWY3DPEHPK3PXP\n\
                   ,,note,Wi-Fi,\"rede: casa\",,,,,,\n\
                   ,,card,Cartão,,,,,,,\n";
        let p = parse(csv).unwrap();
        assert_eq!(p.format, Format::BitwardenCsv);
        assert_eq!(p.items.len(), 2);
        assert_eq!(p.skipped, [(4, Skip::UnknownType("card".into()))]);
        let jira = &p.items[0];
        assert_eq!(jira.folder.as_deref(), Some("Trabalho"));
        assert!(jira.item.favorite && jira.item.reprompt);
        assert_eq!(jira.item.fields.len(), 2);
        assert_eq!(jira.item.fields[1].name.as_deref(), Some("id"));
        assert_eq!(login(&jira.item).uris.len(), 2);
        assert!(login(&jira.item).totp.is_some());
        assert!(matches!(p.items[1].item.kind, ItemKind::SecureNote));
    }

    #[test]
    fn bitwarden_json_every_type() {
        let json = r#"{"encrypted":false,
          "folders":[{"id":"f1","name":"Pessoal"}],
          "items":[
            {"type":1,"name":"Banco","folderId":"f1","favorite":true,"notes":"n",
             "login":{"username":"u","password":"p","totp":null,"uris":[{"uri":"https://bank.example"}]},
             "fields":[{"name":"pin","value":"1234","type":1}]},
            {"type":2,"name":"Nota","secureNote":{"type":0},"notes":"texto"},
            {"type":3,"name":"Visa","card":{"cardholderName":"Ana","number":"4111111111111111","expMonth":"1","expYear":"2030","code":"123","brand":"Visa"}},
            {"type":4,"name":"Eu","identity":{"firstName":"Ana","email":"a@example.com","ssn":"000"}},
            {"type":5,"name":"Servidor","sshKey":{"privateKey":"-----BEGIN","publicKey":"ssh-ed25519 AAAA","keyFingerprint":"SHA256:x"}},
            {"type":9,"name":"Futuro"}
          ]}"#;
        let p = parse(json).unwrap();
        assert_eq!(p.format, Format::BitwardenJson);
        assert_eq!(p.items.len(), 5);
        assert_eq!(p.skipped, [(6, Skip::UnknownType("9".into()))]);
        assert_eq!(p.items[0].folder.as_deref(), Some("Pessoal"));
        assert_eq!(p.items[0].item.fields[0].kind, FieldKind::Hidden);
        assert!(
            matches!(&p.items[2].item.kind, ItemKind::Card(c) if c.brand.as_deref() == Some("Visa"))
        );
        assert!(matches!(&p.items[3].item.kind, ItemKind::Identity(i) if i.ssn.is_some()));
        assert!(matches!(&p.items[4].item.kind, ItemKind::SshKey(k) if k.public_key.is_some()));
    }

    #[test]
    fn protected_and_unknown_files_are_refused() {
        assert_eq!(
            parse(r#"{"encrypted":true,"passwordProtected":true,"data":"2.x"}"#).err(),
            Some(ImportError::Encrypted)
        );
        assert_eq!(parse("hello world").err(), Some(ImportError::Unknown));
        assert_eq!(parse("a,b\n1,2").err(), Some(ImportError::Unknown));
        assert!(matches!(
            parse("{ broken").err(),
            Some(ImportError::Invalid(_))
        ));
    }

    #[test]
    fn duplicates_are_recognized() {
        let (_, _, vault) = open();
        let existing = vault
            .items
            .iter()
            .find(|i| matches!(i.kind, ItemKind::Login(_)) && !i.deleted)
            .unwrap()
            .clone();
        assert!(already_in(&vault, &existing));
        let mut other = existing.clone();
        if let ItemKind::Login(l) = &mut other.kind {
            l.password = Some(Secret::new("different".into()));
        }
        assert!(!already_in(&vault, &other));
    }

    #[test]
    fn the_request_encrypts_items_and_links_folders() {
        let (sync, keys, vault) = open();
        let csv = "folder,favorite,type,name,notes,fields,reprompt,login_uri,login_username,login_password,login_totp\n\
                   Novos,,login,A,,,,https://a.example,ana,pw-a,\n\
                   Novos,,login,B,,,,https://b.example,bob,pw-b,\n\
                   ,,note,C,segredo,,,,,,\n";
        let p = parse(csv).unwrap();
        let existing = vec![Folder {
            id: "folder-x".into(),
            name: "Novos".into(),
        }];
        let body = request(&p.items, &existing, &keys, "user-1").unwrap();
        let ciphers = body["ciphers"].as_array().unwrap();
        assert_eq!(ciphers.len(), 3);
        // One folder, the existing one, for the first two items.
        assert_eq!(body["folders"].as_array().unwrap().len(), 1);
        assert_eq!(body["folders"][0]["id"], "folder-x");
        assert_eq!(
            body["folderRelationships"],
            json!([{ "key": 0, "value": 0 }, { "key": 1, "value": 0 }])
        );
        // Nothing in plain text, and any client can open it.
        let text = body.to_string();
        assert!(!text.contains("pw-a") && !text.contains("segredo") && !text.contains("Novos"));
        // What the server stores: the body plus an id.
        let mut stored = ciphers[1].clone();
        stored["id"] = json!("new-1");
        let raw: CipherRaw = serde_json::from_value(stored).unwrap();
        let mut s = sync.clone();
        s.ciphers = vec![raw];
        let back = Vault::decrypt(&s, &keys).items.remove(0);
        assert_eq!(back.name, "B");
        assert_eq!(login(&back).password.as_ref().unwrap().expose(), "pw-b");
        assert!(!vault.items.is_empty());
    }
}
