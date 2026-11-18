//! The SSH agent protocol (draft-miller-ssh-agent), the part OpenSSH needs to log
//! in with a key: list the keys and sign. Everything else is answered with
//! "failure", so the vault's keys can only be used, never added, removed or
//! exported through the agent.
//!
//! This module only reads and writes messages and signs; the app serves them on
//! Windows' OpenSSH pipe and asks before each signature.

use signature::Signer;
use ssh_key::encoding::Encode;
use ssh_key::private::KeypairData;
use ssh_key::{HashAlg, PrivateKey};

/// Biggest message accepted (OpenSSH's own limit is 256 KiB).
pub const MAX_MESSAGE: usize = 256 * 1024;

const FAILURE: u8 = 5;
const REQUEST_IDENTITIES: u8 = 11;
const IDENTITIES_ANSWER: u8 = 12;
const SIGN_REQUEST: u8 = 13;
const SIGN_RESPONSE: u8 = 14;
/// Sign request flags (RSA only): which hash to use.
const RSA_SHA2_256: u32 = 2;
const RSA_SHA2_512: u32 = 4;

/// What a client asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// "Which keys do you have?"
    Identities,
    /// "Sign `data` with the key whose public blob is `key`."
    Sign {
        key: Vec<u8>,
        data: Vec<u8>,
        flags: u32,
    },
    /// Anything else (add/remove keys, lock, extensions…): refused.
    Other(u8),
}

/// Reads a message body (without its length prefix).
pub fn parse(body: &[u8]) -> Option<Request> {
    let (&kind, mut rest) = body.split_first()?;
    match kind {
        REQUEST_IDENTITIES => Some(Request::Identities),
        SIGN_REQUEST => {
            let key = read_string(&mut rest)?.to_vec();
            let data = read_string(&mut rest)?.to_vec();
            // Older clients may leave the flags out.
            let flags = read_u32(&mut rest).unwrap_or(0);
            Some(Request::Sign { key, data, flags })
        }
        other => Some(Request::Other(other)),
    }
}

fn read_u32(input: &mut &[u8]) -> Option<u32> {
    let (head, rest) = input.split_first_chunk::<4>()?;
    *input = rest;
    Some(u32::from_be_bytes(*head))
}

fn read_string<'a>(input: &mut &'a [u8]) -> Option<&'a [u8]> {
    let len = read_u32(input)? as usize;
    if len > input.len() {
        return None;
    }
    let (s, rest) = input.split_at(len);
    *input = rest;
    Some(s)
}

fn put_string(out: &mut Vec<u8>, s: &[u8]) {
    out.extend_from_slice(&(s.len() as u32).to_be_bytes());
    out.extend_from_slice(s);
}

/// Adds the length prefix, ready to write.
pub fn frame(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + 4);
    put_string(&mut out, body);
    out
}

/// "No" (or "I can't").
pub fn failure() -> Vec<u8> {
    vec![FAILURE]
}

/// The list of keys: public blob and comment (the item's name).
pub fn identities<'a>(keys: impl IntoIterator<Item = (&'a [u8], &'a str)>) -> Vec<u8> {
    let keys: Vec<_> = keys.into_iter().collect();
    let mut out = vec![IDENTITIES_ANSWER];
    out.extend_from_slice(&(keys.len() as u32).to_be_bytes());
    for (blob, comment) in keys {
        put_string(&mut out, blob);
        put_string(&mut out, comment.as_bytes());
    }
    out
}

/// A signature, already encoded (`string algorithm, string blob`).
pub fn signed(signature: &[u8]) -> Vec<u8> {
    let mut out = vec![SIGN_RESPONSE];
    put_string(&mut out, signature);
    out
}

/// A vault key the agent can use.
pub struct AgentKey {
    /// The vault item it comes from.
    pub item_id: String,
    /// The item's name (the comment `ssh-add -l` shows).
    pub name: String,
    /// Public key, in the wire format clients ask for.
    pub blob: Vec<u8>,
    /// `SHA256:…`
    pub fingerprint: String,
    key: PrivateKey,
}

impl AgentKey {
    /// From an item's OpenSSH private key. `None` when it can't be used (not a
    /// key, or still protected by a passphrase).
    pub fn new(item_id: &str, name: &str, private_key: &str) -> Option<Self> {
        let key = PrivateKey::from_openssh(private_key.trim()).ok()?;
        if key.is_encrypted() {
            return None;
        }
        let blob = key.public_key().to_bytes().ok()?;
        Some(Self {
            item_id: item_id.to_owned(),
            name: name.to_owned(),
            blob,
            fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
            key,
        })
    }

    /// Signs `data` as the client asked (`flags` picks the hash for RSA keys).
    /// Returns the encoded signature, or `None` if this key can't do it.
    pub fn sign(&self, data: &[u8], flags: u32) -> Option<Vec<u8>> {
        let signature = match self.key.key_data() {
            KeypairData::Rsa(keypair) => {
                // SHA-1 (`ssh-rsa`) signatures are refused: OpenSSH 8.8+ doesn't ask for them.
                let hash = if flags & RSA_SHA2_512 != 0 {
                    HashAlg::Sha512
                } else if flags & RSA_SHA2_256 != 0 {
                    HashAlg::Sha256
                } else {
                    return None;
                };
                (keypair, Some(hash)).try_sign(data).ok()?
            }
            _ => self.key.try_sign(data).ok()?,
        };
        signature.encode_vec().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use signature::Verifier;
    use ssh_key::encoding::Decode;
    use ssh_key::getrandom::SysRng;
    use ssh_key::rand_core::UnwrapErr;
    use ssh_key::{Algorithm, LineEnding, Signature};

    fn key(algorithm: Algorithm) -> (AgentKey, PrivateKey) {
        let key = PrivateKey::random(&mut UnwrapErr(SysRng), algorithm).unwrap();
        let text = key.to_openssh(LineEnding::LF).unwrap();
        (AgentKey::new("item-1", "GitHub", &text).unwrap(), key)
    }

    fn sign_request(blob: &[u8], data: &[u8], flags: u32) -> Vec<u8> {
        let mut body = vec![SIGN_REQUEST];
        put_string(&mut body, blob);
        put_string(&mut body, data);
        body.extend_from_slice(&flags.to_be_bytes());
        body
    }

    #[test]
    fn requests_are_read() {
        assert_eq!(parse(&[11]), Some(Request::Identities));
        assert_eq!(
            parse(&sign_request(b"blob", b"data", 2)),
            Some(Request::Sign {
                key: b"blob".to_vec(),
                data: b"data".to_vec(),
                flags: 2
            })
        );
        // Adding keys, extensions…: refused, not misread.
        assert_eq!(parse(&[17, 0, 0]), Some(Request::Other(17)));
        assert_eq!(parse(&[27]), Some(Request::Other(27)));
        // Broken messages.
        assert_eq!(parse(&[]), None);
        assert_eq!(parse(&[13, 0, 0, 0, 9, 1]), None);
        assert_eq!(frame(&[11]), [0, 0, 0, 1, 11]);
    }

    #[test]
    fn identities_list_the_keys() {
        let (a, _) = key(Algorithm::Ed25519);
        let body = identities([(a.blob.as_slice(), "GitHub")]);
        let mut rest = &body[1..];
        assert_eq!(body[0], IDENTITIES_ANSWER);
        assert_eq!(read_u32(&mut rest), Some(1));
        assert_eq!(read_string(&mut rest), Some(a.blob.as_slice()));
        assert_eq!(read_string(&mut rest), Some(&b"GitHub"[..]));
        assert!(rest.is_empty());
        assert_eq!(identities([]), [IDENTITIES_ANSWER, 0, 0, 0, 0]);
    }

    #[test]
    fn ed25519_signatures_verify() {
        let (agent, plain) = key(Algorithm::Ed25519);
        assert!(agent.fingerprint.starts_with("SHA256:"));
        let sig = agent.sign(b"session data", 0).unwrap();
        let sig = Signature::decode(&mut sig.as_slice()).unwrap();
        assert_eq!(sig.algorithm(), Algorithm::Ed25519);
        plain
            .public_key()
            .key_data()
            .verify(b"session data", &sig)
            .unwrap();
        assert!(plain
            .public_key()
            .key_data()
            .verify(b"other data", &sig)
            .is_err());
    }

    #[test]
    fn rsa_uses_the_hash_the_client_asks_for() {
        let rsa = rsa::PrivateKey::new(&mut UnwrapErr(SysRng), 1024).unwrap();
        let keypair = ssh_key::private::RsaKeypair::try_from(rsa).unwrap();
        let plain = PrivateKey::new(KeypairData::from(keypair), "").unwrap();
        let text = plain.to_openssh(LineEnding::LF).unwrap();
        let agent = AgentKey::new("item-2", "Server", &text).unwrap();
        for (flags, hash) in [
            (RSA_SHA2_256, HashAlg::Sha256),
            (RSA_SHA2_512, HashAlg::Sha512),
        ] {
            let sig = agent.sign(b"data", flags).unwrap();
            let sig = Signature::decode(&mut sig.as_slice()).unwrap();
            assert_eq!(sig.algorithm(), Algorithm::Rsa { hash: Some(hash) });
            plain.public_key().key_data().verify(b"data", &sig).unwrap();
        }
        // Old SHA-1 signatures: no.
        assert!(agent.sign(b"data", 0).is_none());
    }

    #[test]
    fn protected_or_broken_keys_are_left_out() {
        let plain = PrivateKey::random(&mut UnwrapErr(SysRng), Algorithm::Ed25519).unwrap();
        let locked = plain.encrypt(&mut UnwrapErr(SysRng), "pass").unwrap();
        let text = locked.to_openssh(LineEnding::LF).unwrap();
        assert!(AgentKey::new("x", "x", &text).is_none());
        assert!(AgentKey::new("x", "x", "hello").is_none());
    }

    #[test]
    fn the_answer_is_a_sign_response() {
        let (agent, _) = key(Algorithm::Ed25519);
        let sig = agent.sign(b"x", 0).unwrap();
        let body = signed(&sig);
        assert_eq!(body[0], SIGN_RESPONSE);
        let mut rest = &body[1..];
        assert_eq!(read_string(&mut rest), Some(sig.as_slice()));
    }
}
