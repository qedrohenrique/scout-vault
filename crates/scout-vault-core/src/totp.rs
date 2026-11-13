//! Códigos TOTP (RFC 6238) a partir da chave guardada no item.
//!
//! Aceita os mesmos formatos que o Bitwarden guarda no campo `totp`:
//! - a chave em base32 pura (`JBSW Y3DP …`, com ou sem espaços/padding);
//! - uma URI `otpauth://totp/…?secret=…&algorithm=…&digits=…&period=…`;
//! - `steam://<chave>` — códigos de 5 caracteres do Steam Guard.

use hmac::{Hmac, KeyInit, Mac};
use sha1::Sha1;
use sha2::{Sha256, Sha512};
use zeroize::Zeroizing;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Algorithm {
    Sha1,
    Sha256,
    Sha512,
}

/// Parâmetros de um gerador TOTP. A chave fica zerada ao sair da memória.
pub struct Totp {
    key: Zeroizing<Vec<u8>>,
    pub algorithm: Algorithm,
    pub digits: u32,
    pub period: u64,
    pub steam: bool,
}

impl std::fmt::Debug for Totp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Totp")
            .field("algorithm", &self.algorithm)
            .field("digits", &self.digits)
            .field("period", &self.period)
            .field("steam", &self.steam)
            .finish_non_exhaustive()
    }
}

/// Um código e quantos segundos ele ainda vale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Code {
    pub code: String,
    pub remaining: u64,
    pub period: u64,
}

const STEAM_CHARS: &[u8] = b"23456789BCDFGHJKMNPQRTVWXY";

impl Totp {
    /// Lê o campo `totp` do item. `None` se não for uma chave válida.
    pub fn parse(input: &str) -> Option<Self> {
        let input = input.trim();
        let lower = input.to_ascii_lowercase();
        if lower.starts_with("steam://") {
            return Some(Self {
                key: Zeroizing::new(base32_decode(&input[8..])?),
                algorithm: Algorithm::Sha1,
                digits: 5,
                period: 30,
                steam: true,
            });
        }
        if lower.starts_with("otpauth://") {
            return Self::parse_uri(input);
        }
        Some(Self {
            key: Zeroizing::new(base32_decode(input)?),
            algorithm: Algorithm::Sha1,
            digits: 6,
            period: 30,
            steam: false,
        })
    }

    fn parse_uri(uri: &str) -> Option<Self> {
        let (_, query) = uri.split_once('?')?;
        let mut secret = None;
        let mut algorithm = Algorithm::Sha1;
        let mut digits = 6;
        let mut period = 30;
        let mut steam = false;
        for pair in query.split('&') {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            let v = percent_decode(v);
            match k.to_ascii_lowercase().as_str() {
                "secret" => secret = Some(Zeroizing::new(v)),
                "algorithm" => {
                    algorithm = match v.to_ascii_uppercase().as_str() {
                        "SHA256" => Algorithm::Sha256,
                        "SHA512" => Algorithm::Sha512,
                        _ => Algorithm::Sha1,
                    }
                }
                "digits" => digits = v.parse().ok().filter(|d| (1..=10).contains(d))?,
                "period" => period = v.parse().ok().filter(|p| *p > 0)?,
                // Variante usada por alguns exportadores para Steam Guard.
                "encoder" if v.eq_ignore_ascii_case("steam") => steam = true,
                _ => {}
            }
        }
        if steam {
            digits = 5;
        }
        Some(Self {
            key: Zeroizing::new(base32_decode(&secret?)?),
            algorithm,
            digits,
            period,
            steam,
        })
    }

    /// Código para o instante `unix_secs`.
    pub fn at(&self, unix_secs: u64) -> Code {
        let counter = unix_secs / self.period;
        let hash = self.hmac(&counter.to_be_bytes());
        let offset = (hash[hash.len() - 1] & 0x0f) as usize;
        let value = u32::from_be_bytes([
            hash[offset] & 0x7f,
            hash[offset + 1],
            hash[offset + 2],
            hash[offset + 3],
        ]);
        let code = if self.steam {
            let mut v = value;
            (0..self.digits)
                .map(|_| {
                    let c = STEAM_CHARS[(v % STEAM_CHARS.len() as u32) as usize] as char;
                    v /= STEAM_CHARS.len() as u32;
                    c
                })
                .collect()
        } else {
            let modulo = 10u64.pow(self.digits);
            format!(
                "{:0width$}",
                u64::from(value) % modulo,
                width = self.digits as usize
            )
        };
        Code {
            code,
            remaining: self.period - unix_secs % self.period,
            period: self.period,
        }
    }

    /// Código para agora.
    pub fn now(&self) -> Code {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.at(secs)
    }

    fn hmac(&self, msg: &[u8]) -> Vec<u8> {
        macro_rules! run {
            ($h:ty) => {{
                let mut m =
                    <Hmac<$h>>::new_from_slice(&self.key).expect("HMAC aceita qualquer chave");
                m.update(msg);
                m.finalize().into_bytes().to_vec()
            }};
        }
        match self.algorithm {
            Algorithm::Sha1 => run!(Sha1),
            Algorithm::Sha256 => run!(Sha256),
            Algorithm::Sha512 => run!(Sha512),
        }
    }
}

/// Mostra o código em dois grupos (`123 456`), como os apps autenticadores.
pub fn group(code: &str) -> String {
    if code.len() >= 6 && code.chars().all(|c| c.is_ascii_digit()) {
        let mid = code.len() / 2;
        format!("{} {}", &code[..mid], &code[mid..])
    } else {
        code.to_owned()
    }
}

/// Base32 (RFC 4648), tolerante a espaços, hífens, minúsculas e padding.
fn base32_decode(input: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() * 5 / 8);
    let (mut buffer, mut bits) = (0u32, 0u32);
    let mut any = false;
    for c in input.chars() {
        let v = match c.to_ascii_uppercase() {
            c @ 'A'..='Z' => c as u32 - 'A' as u32,
            c @ '2'..='7' => c as u32 - '2' as u32 + 26,
            ' ' | '-' | '=' => continue,
            _ => return None,
        };
        any = true;
        buffer = (buffer << 5) | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    (any && !out.is_empty()).then_some(out)
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(b) => {
                        out.push(b);
                        i += 3;
                        continue;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            b'+' => out.push(b' '),
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b32(bytes: &[u8]) -> String {
        const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
        let (mut out, mut buf, mut bits) = (String::new(), 0u32, 0u32);
        for &b in bytes {
            buf = (buf << 8) | b as u32;
            bits += 8;
            while bits >= 5 {
                bits -= 5;
                out.push(A[((buf >> bits) & 31) as usize] as char);
            }
            buf &= (1 << bits) - 1;
        }
        if bits > 0 {
            out.push(A[((buf << (5 - bits)) & 31) as usize] as char);
        }
        out
    }

    /// Vetores do apêndice B da RFC 6238 (8 dígitos).
    #[test]
    fn rfc6238_vectors() {
        let cases = [
            ("SHA1", &b"12345678901234567890"[..], 59, "94287082"),
            ("SHA1", &b"12345678901234567890"[..], 1111111109, "07081804"),
            (
                "SHA1",
                &b"12345678901234567890"[..],
                20000000000,
                "65353130",
            ),
            (
                "SHA256",
                &b"12345678901234567890123456789012"[..],
                59,
                "46119246",
            ),
            (
                "SHA256",
                &b"12345678901234567890123456789012"[..],
                1234567890,
                "91819424",
            ),
            (
                "SHA512",
                &b"1234567890123456789012345678901234567890123456789012345678901234"[..],
                59,
                "90693936",
            ),
            (
                "SHA512",
                &b"1234567890123456789012345678901234567890123456789012345678901234"[..],
                2000000000,
                "38618901",
            ),
        ];
        for (alg, key, t, want) in cases {
            let uri = format!(
                "otpauth://totp/Teste:a%40b.com?secret={}&algorithm={alg}&digits=8&period=30",
                b32(key)
            );
            let totp = Totp::parse(&uri).unwrap();
            assert_eq!(totp.at(t).code, want, "{alg} t={t}");
        }
    }

    #[test]
    fn plain_secret_with_spaces_and_lowercase() {
        let secret = b32(b"12345678901234567890").to_lowercase();
        let spaced: String = secret
            .chars()
            .collect::<Vec<_>>()
            .chunks(4)
            .map(|c| c.iter().collect::<String>())
            .collect::<Vec<_>>()
            .join(" ");
        let totp = Totp::parse(&spaced).unwrap();
        assert_eq!(totp.digits, 6);
        // Os 6 últimos dígitos do vetor de 8 da RFC.
        assert_eq!(totp.at(59).code, "287082");
        assert_eq!(totp.at(59).remaining, 1);
        assert_eq!(totp.at(60).remaining, 30);
    }

    #[test]
    fn steam_codes_use_the_steam_alphabet() {
        let totp = Totp::parse(&format!("steam://{}", b32(b"12345678901234567890"))).unwrap();
        let code = totp.at(59).code;
        assert_eq!(code.len(), 5);
        assert!(code.bytes().all(|b| STEAM_CHARS.contains(&b)), "{code}");
    }

    #[test]
    fn rejects_garbage() {
        assert!(Totp::parse("não é uma chave!").is_none());
        assert!(Totp::parse("").is_none());
        assert!(Totp::parse("otpauth://totp/x?issuer=y").is_none());
    }

    #[test]
    fn groups_codes() {
        assert_eq!(group("123456"), "123 456");
        assert_eq!(group("12345678"), "1234 5678");
        assert_eq!(group("BCD2F"), "BCD2F");
    }
}
