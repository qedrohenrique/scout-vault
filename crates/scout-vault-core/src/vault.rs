//! Cofre decifrado, pronto para a UI. Vive só em memória.

use std::collections::HashMap;

use crate::crypto::{CryptoError, MasterKey, RsaPrivateKey, SymmetricKey};
use crate::enc_string::{EncString, ParseError};
use crate::secret::Secret;
use crate::sync::{CipherRaw, Profile, SyncResponse};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VaultError {
    #[error("o perfil não tem a user key cifrada")]
    MissingUserKey,
    #[error("senha mestra incorreta (não foi possível abrir a user key)")]
    WrongPassword,
    #[error("campo cifrado malformado: {0}")]
    Parse(#[from] ParseError),
    #[error(transparent)]
    Crypto(#[from] CryptoError),
    #[error("sem chave para a organização {0}")]
    MissingOrgKey(String),
    #[error("{0}")]
    Unsupported(String),
}

/// Chaves da sessão desbloqueada.
#[derive(Debug)]
pub struct Keys {
    user: SymmetricKey,
    orgs: HashMap<String, SymmetricKey>,
    /// Organizações cuja chave não abriu (o resto do cofre continua acessível).
    pub failed_orgs: Vec<(String, VaultError)>,
}

impl Keys {
    /// Abre a user key com a master key e, a partir dela, as chaves de organização.
    pub fn unlock(master_key: &MasterKey, profile: &Profile) -> Result<Self, VaultError> {
        let protected = profile.key.as_deref().ok_or(VaultError::MissingUserKey)?;
        let protected: EncString = protected.parse()?;
        let user_bytes = master_key
            .stretch()
            .decrypt(&protected)
            .map_err(|e| match e {
                CryptoError::InvalidMac => VaultError::WrongPassword,
                other => other.into(),
            })?;
        let user = SymmetricKey::from_bytes(&user_bytes)?;

        let mut keys = Keys {
            user,
            orgs: HashMap::new(),
            failed_orgs: Vec::new(),
        };

        let orgs_with_keys: Vec<_> = profile
            .organizations
            .iter()
            .filter_map(|o| o.key.as_deref().map(|k| (o.id.clone(), k)))
            .collect();
        if orgs_with_keys.is_empty() {
            return Ok(keys);
        }

        let wrapped_private_key = profile.private_key.as_deref().or_else(|| {
            profile
                .account_keys
                .as_ref()
                .and_then(|a| a.public_key_encryption_key_pair.as_ref())
                .and_then(|p| p.wrapped_private_key.as_deref())
        });
        let private_key = match wrapped_private_key {
            Some(pk) => keys.open_private_key(pk),
            None => Err(VaultError::Crypto(CryptoError::RsaKey)),
        };
        for (id, enc) in orgs_with_keys {
            let result = private_key.as_ref().map_err(Clone::clone).and_then(|rsa| {
                let bytes = rsa.decrypt(&enc.parse()?)?;
                Ok(SymmetricKey::from_bytes(&bytes)?)
            });
            match result {
                Ok(k) => {
                    keys.orgs.insert(id, k);
                }
                Err(e) => keys.failed_orgs.push((id, e)),
            }
        }
        Ok(keys)
    }

    fn open_private_key(&self, enc: &str) -> Result<RsaPrivateKey, VaultError> {
        let der = self.user.decrypt(&enc.parse()?)?;
        Ok(RsaPrivateKey::from_pkcs8_der(&der)?)
    }

    pub fn user_key(&self) -> &SymmetricKey {
        &self.user
    }

    /// Chave que cifra itens do dono indicado (`None` = cofre pessoal).
    pub fn for_owner(&self, organization_id: Option<&str>) -> Result<&SymmetricKey, VaultError> {
        match organization_id {
            None => Ok(&self.user),
            Some(id) => self
                .orgs
                .get(id)
                .ok_or_else(|| VaultError::MissingOrgKey(id.to_owned())),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Folder {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct Organization {
    pub id: String,
    pub name: String,
}

/// Arquivo anexado ao item (o conteúdo fica no servidor, cifrado).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    pub id: String,
    pub file_name: String,
    pub size: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct Item {
    pub id: String,
    pub organization_id: Option<String>,
    pub folder_id: Option<String>,
    pub name: String,
    pub notes: Option<Secret>,
    pub favorite: bool,
    /// Exige digitar a senha mestra de novo antes de revelar.
    pub reprompt: bool,
    /// Está na lixeira.
    pub deleted: bool,
    pub revision_date: Option<String>,
    pub fields: Vec<Field>,
    pub kind: ItemKind,
    pub attachments: Vec<Attachment>,
}

#[derive(Debug, Clone)]
pub enum ItemKind {
    Login(Login),
    SecureNote,
    Card(Card),
    Identity(Box<Identity>),
    SshKey(SshKey),
    /// Tipo que este cliente ainda não conhece — mantido para não sumir da lista.
    Unknown(u8),
}

#[derive(Debug, Clone, Default)]
pub struct Login {
    pub username: Option<String>,
    pub password: Option<Secret>,
    /// Chave/URI `otpauth://` do autenticador.
    pub totp: Option<Secret>,
    pub uris: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Card {
    pub cardholder_name: Option<String>,
    pub brand: Option<String>,
    pub number: Option<Secret>,
    pub exp_month: Option<String>,
    pub exp_year: Option<String>,
    pub code: Option<Secret>,
}

#[derive(Debug, Clone, Default)]
pub struct Identity {
    pub title: Option<String>,
    pub first_name: Option<String>,
    pub middle_name: Option<String>,
    pub last_name: Option<String>,
    pub address1: Option<String>,
    pub address2: Option<String>,
    pub address3: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub postal_code: Option<String>,
    pub country: Option<String>,
    pub company: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub username: Option<String>,
    pub ssn: Option<Secret>,
    pub passport_number: Option<Secret>,
    pub license_number: Option<Secret>,
}

#[derive(Debug, Clone, Default)]
pub struct SshKey {
    pub private_key: Option<Secret>,
    pub public_key: Option<String>,
    pub fingerprint: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    Text,
    Hidden,
    Boolean,
    Linked,
    Other(u8),
}

#[derive(Debug, Clone)]
pub struct Field {
    pub kind: FieldKind,
    pub name: Option<String>,
    pub value: Option<Secret>,
}

impl Item {
    /// Busca simples, sem diferenciar maiúsculas: nome, usuário e URIs.
    pub fn matches(&self, query: &str) -> bool {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return true;
        }
        let hit = |s: &str| s.to_lowercase().contains(&q);
        if hit(&self.name) {
            return true;
        }
        match &self.kind {
            ItemKind::Login(l) => {
                l.username.as_deref().is_some_and(hit) || l.uris.iter().any(|u| hit(u))
            }
            ItemKind::Card(c) => c.cardholder_name.as_deref().is_some_and(hit),
            ItemKind::Identity(i) => i.email.as_deref().is_some_and(hit),
            _ => false,
        }
    }
}

#[derive(Debug, Default)]
pub struct Vault {
    pub organizations: Vec<Organization>,
    pub folders: Vec<Folder>,
    pub items: Vec<Item>,
    /// Itens/pastas que não abriram, com o motivo. Um item corrompido não derruba o cofre.
    pub errors: Vec<(String, VaultError)>,
}

impl Vault {
    pub fn decrypt(sync: &SyncResponse, keys: &Keys) -> Self {
        let mut vault = Vault {
            organizations: sync
                .profile
                .organizations
                .iter()
                .map(|o| Organization {
                    id: o.id.clone(),
                    name: o.name.clone().unwrap_or_default(),
                })
                .collect(),
            ..Default::default()
        };

        for f in &sync.folders {
            match plain(keys.user_key(), &Some(f.name.clone())) {
                Ok(name) => vault.folders.push(Folder {
                    id: f.id.clone(),
                    name: name.unwrap_or_default(),
                }),
                Err(e) => vault.errors.push((f.id.clone(), e)),
            }
        }

        for c in &sync.ciphers {
            match decrypt_item(c, keys) {
                Ok(item) => vault.items.push(item),
                Err(e) => vault.errors.push((c.id.clone(), e)),
            }
        }

        vault
            .items
            .sort_by_cached_key(|i| (i.deleted, i.name.to_lowercase()));
        vault
    }

    /// Itens ativos (fora da lixeira) que batem com a busca.
    pub fn search(&self, query: &str) -> impl Iterator<Item = &Item> + '_ {
        let query = query.to_owned();
        self.items
            .iter()
            .filter(move |i| !i.deleted && i.matches(&query))
    }
}

fn decrypt_item(c: &CipherRaw, keys: &Keys) -> Result<Item, VaultError> {
    let owner_key = keys.for_owner(c.organization_id.as_deref())?;
    // Itens novos têm chave própria, cifrada com a chave do dono.
    let item_key;
    let key = match &c.key {
        Some(k) => {
            let bytes = owner_key.decrypt(&k.parse()?)?;
            item_key = SymmetricKey::from_bytes(&bytes)?;
            &item_key
        }
        None => owner_key,
    };

    let kind = match c.kind {
        1 => {
            let l = c.login.clone().unwrap_or(crate::sync::LoginRaw {
                username: None,
                password: None,
                totp: None,
                uris: None,
                extra: Default::default(),
            });
            let mut uris = Vec::new();
            for u in l.uris.iter().flatten() {
                if let Some(uri) = plain(key, &u.uri)? {
                    uris.push(uri);
                }
            }
            ItemKind::Login(Login {
                username: plain(key, &l.username)?,
                password: secret(key, &l.password)?,
                totp: secret(key, &l.totp)?,
                uris,
            })
        }
        2 => ItemKind::SecureNote,
        3 => {
            let card = c.card.as_ref();
            let f = |sel: fn(&crate::sync::CardRaw) -> &Option<String>| {
                card.map(sel).cloned().unwrap_or(None)
            };
            ItemKind::Card(Card {
                cardholder_name: plain(key, &f(|c| &c.cardholder_name))?,
                brand: plain(key, &f(|c| &c.brand))?,
                number: secret(key, &f(|c| &c.number))?,
                exp_month: plain(key, &f(|c| &c.exp_month))?,
                exp_year: plain(key, &f(|c| &c.exp_year))?,
                code: secret(key, &f(|c| &c.code))?,
            })
        }
        4 => {
            let i = c.identity.clone().unwrap_or_default();
            ItemKind::Identity(Box::new(Identity {
                title: plain(key, &i.title)?,
                first_name: plain(key, &i.first_name)?,
                middle_name: plain(key, &i.middle_name)?,
                last_name: plain(key, &i.last_name)?,
                address1: plain(key, &i.address1)?,
                address2: plain(key, &i.address2)?,
                address3: plain(key, &i.address3)?,
                city: plain(key, &i.city)?,
                state: plain(key, &i.state)?,
                postal_code: plain(key, &i.postal_code)?,
                country: plain(key, &i.country)?,
                company: plain(key, &i.company)?,
                email: plain(key, &i.email)?,
                phone: plain(key, &i.phone)?,
                username: plain(key, &i.username)?,
                ssn: secret(key, &i.ssn)?,
                passport_number: secret(key, &i.passport_number)?,
                license_number: secret(key, &i.license_number)?,
            }))
        }
        5 => {
            let s = c.ssh_key.as_ref();
            ItemKind::SshKey(SshKey {
                private_key: secret(key, &s.and_then(|s| s.private_key.clone()))?,
                public_key: plain(key, &s.and_then(|s| s.public_key.clone()))?,
                fingerprint: plain(key, &s.and_then(|s| s.key_fingerprint.clone()))?,
            })
        }
        other => ItemKind::Unknown(other),
    };

    let mut fields = Vec::new();
    for f in c.fields.iter().flatten() {
        fields.push(Field {
            kind: match f.kind.unwrap_or(0) {
                0 => FieldKind::Text,
                1 => FieldKind::Hidden,
                2 => FieldKind::Boolean,
                3 => FieldKind::Linked,
                n => FieldKind::Other(n),
            },
            name: plain(key, &f.name)?,
            value: secret(key, &f.value)?,
        });
    }

    let attachments = crate::attachment::raw_attachments(c)
        .into_iter()
        .map(|a| Attachment {
            file_name: plain(key, &a.file_name)
                .ok()
                .flatten()
                .unwrap_or_else(|| "(nome ilegível)".into()),
            size: a.size_bytes(),
            id: a.id,
        })
        .collect();

    Ok(Item {
        attachments,
        id: c.id.clone(),
        organization_id: c.organization_id.clone(),
        folder_id: c.folder_id.clone(),
        name: plain(key, &c.name)?.unwrap_or_default(),
        notes: secret(key, &c.notes)?,
        favorite: c.favorite,
        reprompt: c.reprompt == Some(1),
        deleted: c.deleted_date.is_some(),
        revision_date: c.revision_date.clone(),
        fields,
        kind,
    })
}

fn secret(key: &SymmetricKey, field: &Option<String>) -> Result<Option<Secret>, VaultError> {
    match field.as_deref() {
        None | Some("") => Ok(None),
        Some(enc) => Ok(Some(key.decrypt_str(&enc.parse()?)?)),
    }
}

/// Campo não tão sensível (nome, usuário, URI): decifrado para `String` comum,
/// para a UI poder buscar e ordenar.
fn plain(key: &SymmetricKey, field: &Option<String>) -> Result<Option<String>, VaultError> {
    Ok(secret(key, field)?.map(|s| s.expose().to_owned()))
}
