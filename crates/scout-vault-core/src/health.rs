//! Password health: weak, reused, old and leaked.
//!
//! Everything runs on the machine, except the leak check, which uses the
//! "Pwned Passwords" API (Have I Been Pwned) with **k-anonymity**: only the first 5
//! characters of each password's SHA-1 leave the machine; the response brings hundreds
//! of possible suffixes and the comparison is done here. With `Add-Padding`, the
//! response size doesn't give anything away either.

use std::collections::HashMap;

use sha1::{Digest, Sha1};

use crate::vault::{Item, ItemKind};

/// A password below this (in estimated bits) counts as weak.
pub const WEAK_BELOW_BITS: f64 = 50.;
/// A password unchanged for longer than this counts as old.
pub const OLD_AFTER_DAYS: u64 = 365;

/// Common passwords and fragments (lowercase). Containing one of them drags the strength down.
const COMMON: &[&str] = &[
    "password",
    "passw0rd",
    "senha",
    "123456",
    "12345",
    "1234",
    "qwerty",
    "asdf",
    "zxcv",
    "abc123",
    "admin",
    "letmein",
    "welcome",
    "iloveyou",
    "teamo",
    "monkey",
    "dragon",
    "football",
    "futebol",
    "baseball",
    "master",
    "login",
    "princess",
    "princesa",
    "sunshine",
    "shadow",
    "trustno1",
    "batman",
    "superman",
    "brasil",
    "flamengo",
    "corinthians",
    "palmeiras",
    "saopaulo",
    "gremio",
    "mudar",
    "teste",
    "test",
    "amor",
    "1q2w3e",
    "qazwsx",
    "0000",
    "1111",
    "2222",
    "9999",
    "abcd",
    "default",
    "changeme",
    "secret",
    "segredo",
    "minhasenha",
    "mudar123",
];

/// Estimated strength of a password, in bits. Counts the length and the variety of
/// characters, and discounts repetitions, sequences (`abc`, `123`), years and
/// common words. It isn't zxcvbn, but it catches what matters in a report.
pub fn estimate_bits(password: &str) -> f64 {
    let chars: Vec<char> = password.chars().collect();
    if chars.is_empty() {
        return 0.;
    }
    let has = |f: fn(&char) -> bool| chars.iter().any(f);
    let mut pool: f64 = 0.;
    if has(char::is_ascii_lowercase) {
        pool += 26.;
    }
    if has(char::is_ascii_uppercase) {
        pool += 26.;
    }
    if has(char::is_ascii_digit) {
        pool += 10.;
    }
    if has(|c| c.is_ascii_punctuation() || *c == ' ') {
        pool += 33.;
    }
    if has(|c| !c.is_ascii()) {
        pool += 60.;
    }
    let per_char = f64::log2(pool.max(2.));

    // Characters are "worth" less when they repeat or follow a sequence.
    let mut weight = vec![1.; chars.len()];
    for i in 1..chars.len() {
        let (a, b) = (chars[i - 1] as i64, chars[i] as i64);
        if a == b {
            weight[i] = 0.2;
        } else if (b - a).abs() == 1 {
            weight[i] = 0.4;
        }
    }
    // Common words and years (19xx/20xx) add almost nothing.
    let lower = password.to_lowercase();
    let lower_chars: Vec<char> = lower.chars().collect();
    let mut hits: Vec<(usize, usize)> = Vec::new();
    for word in COMMON {
        let w: Vec<char> = word.chars().collect();
        for start in 0..lower_chars.len().saturating_sub(w.len() - 1) {
            if lower_chars[start..start + w.len()] == w[..] {
                hits.push((start, w.len()));
            }
        }
    }
    for start in 0..lower_chars.len().saturating_sub(3) {
        let y = &lower_chars[start..start + 4];
        if y.iter().all(char::is_ascii_digit) && matches!((y[0], y[1]), ('1', '9') | ('2', '0')) {
            hits.push((start, 4));
        }
    }
    let mut extra = 0.;
    for (start, len) in &hits {
        for w in &mut weight[*start..start + len] {
            *w = 0.;
        }
        extra = f64::max(extra, 8.);
    }
    let bits = weight.iter().sum::<f64>() * per_char + extra;
    if COMMON.contains(&lower.as_str()) {
        return bits.min(8.);
    }
    bits
}

/// What the report found. Each list holds item ids.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Report {
    /// Logins with a password that were analyzed.
    pub checked: usize,
    /// Weak passwords, with the estimated strength (bits), weakest first.
    pub weak: Vec<(String, f64)>,
    /// Groups of items with the same password (each group has 2 or more).
    pub reused: Vec<Vec<String>>,
    /// Passwords unchanged for more than [`OLD_AFTER_DAYS`], with the age in days.
    pub old: Vec<(String, u64)>,
}

impl Report {
    /// Items with some problem (no repeats).
    pub fn affected(&self) -> usize {
        let mut ids: Vec<&String> = self.weak.iter().map(|(id, _)| id).collect();
        ids.extend(self.reused.iter().flatten());
        ids.extend(self.old.iter().map(|(id, _)| id));
        ids.sort();
        ids.dedup();
        ids.len()
    }
}

/// The item's password, if it is an active login (not in the trash) with a password.
pub fn login_password(item: &Item) -> Option<&str> {
    if item.deleted {
        return None;
    }
    match &item.kind {
        ItemKind::Login(l) => l
            .password
            .as_ref()
            .map(|p| p.expose())
            .filter(|p| !p.is_empty()),
        _ => None,
    }
}

/// Analyzes the logins. `changed_at`: id → when the password last changed
/// (Unix seconds); `now`: now, in Unix seconds.
pub fn analyze(items: &[Item], changed_at: &HashMap<String, u64>, now: u64) -> Report {
    let mut report = Report::default();
    let mut by_password: HashMap<&str, Vec<String>> = HashMap::new();
    for item in items {
        let Some(password) = login_password(item) else {
            continue;
        };
        report.checked += 1;
        let bits = estimate_bits(password);
        if bits < WEAK_BELOW_BITS {
            report.weak.push((item.id.clone(), bits));
        }
        by_password
            .entry(password)
            .or_default()
            .push(item.id.clone());
        if let Some(at) = changed_at.get(&item.id) {
            let days = now.saturating_sub(*at) / 86_400;
            if days > OLD_AFTER_DAYS {
                report.old.push((item.id.clone(), days));
            }
        }
    }
    report
        .weak
        .sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    report.reused = by_password
        .into_values()
        .filter(|ids| ids.len() > 1)
        .collect();
    report
        .reused
        .sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    report
        .old
        .sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    report
}

/// When each item's password last changed, from the raw sync
/// (`login.passwordRevisionDate`; without it, the item's creation date).
pub fn password_dates(sync: &crate::sync::SyncResponse) -> HashMap<String, u64> {
    let mut out = HashMap::new();
    for c in &sync.ciphers {
        let changed = c
            .login
            .as_ref()
            .and_then(|l| l.extra.get("passwordRevisionDate"))
            .and_then(|v| v.as_str())
            .or_else(|| c.extra.get("creationDate").and_then(|v| v.as_str()));
        if let Some(secs) = changed.and_then(parse_rfc3339) {
            out.insert(c.id.clone(), secs);
        }
    }
    out
}

/// `2024-03-05T12:34:56.789Z` → Unix seconds (only the date and time part, in UTC).
pub fn parse_rfc3339(s: &str) -> Option<u64> {
    let num = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, m, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (hh, mm, ss) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if s.as_bytes().get(4) != Some(&b'-') || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    // Days since 1970-01-01 (Howard Hinnant's algorithm, Gregorian calendar).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days * 86_400 + hh * 3600 + mm * 60 + ss).ok()
}

/// SHA-1 of the password in uppercase hex, split into (5-char prefix, rest).
/// Only the prefix goes over the network.
pub fn pwned_hash(password: &str) -> (String, String) {
    let digest = Sha1::digest(password.as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02X}")).collect();
    let (prefix, suffix) = hex.split_at(5);
    (prefix.to_owned(), suffix.to_owned())
}

/// How many times the suffix shows up in an API response (`SUFFIX:COUNT` per
/// line). Padding lines have count 0.
pub fn count_in_range(body: &str, suffix: &str) -> u64 {
    body.lines()
        .filter_map(|line| line.trim().split_once(':'))
        .find(|(s, _)| s.eq_ignore_ascii_case(suffix))
        .and_then(|(_, n)| n.trim().parse().ok())
        .unwrap_or(0)
}

/// Address of the "Pwned Passwords" API.
pub const PWNED_API: &str = "https://api.pwnedpasswords.com";

/// Error while checking for leaks.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct PwnedError(pub String);

/// Checks which passwords show up in leaks. Takes (id, password) and returns
/// (id, times leaked) only for the leaked ones. Each prefix is queried once,
/// even if several passwords share it.
pub async fn check_pwned(
    base_url: &str,
    passwords: &[(String, &str)],
) -> Result<Vec<(String, u64)>, PwnedError> {
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| PwnedError(format!("Não foi possível preparar a consulta: {e}")))?;
    let mut by_prefix: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for (id, password) in passwords {
        let (prefix, suffix) = pwned_hash(password);
        by_prefix
            .entry(prefix)
            .or_default()
            .push((id.clone(), suffix));
    }
    let mut prefixes: Vec<_> = by_prefix.into_iter().collect();
    prefixes.sort_by(|a, b| a.0.cmp(&b.0));
    let mut found = Vec::new();
    for (prefix, entries) in prefixes {
        let res = http
            .get(format!("{}/range/{prefix}", base_url.trim_end_matches('/')))
            .header("Add-Padding", "true")
            .header("User-Agent", "Scout-Vault")
            .send()
            .await
            .map_err(|e| PwnedError(format!("Não foi possível consultar vazamentos: {e}")))?;
        if !res.status().is_success() {
            return Err(PwnedError(format!(
                "O serviço de vazamentos respondeu {}.",
                res.status()
            )));
        }
        let body = res
            .text()
            .await
            .map_err(|e| PwnedError(format!("Resposta inválida do serviço de vazamentos: {e}")))?;
        for (id, suffix) in entries {
            let n = count_in_range(&body, &suffix);
            if n > 0 {
                found.push((id, n));
            }
        }
    }
    found.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secret::Secret;
    use crate::vault::Login;

    fn login(id: &str, password: &str) -> Item {
        Item {
            id: id.into(),
            organization_id: None,
            folder_id: None,
            name: id.into(),
            notes: None,
            favorite: false,
            reprompt: false,
            deleted: false,
            revision_date: None,
            fields: Vec::new(),
            kind: ItemKind::Login(Login {
                password: Some(Secret::new(password.to_owned())),
                ..Default::default()
            }),
            attachments: Vec::new(),
            collection_ids: Vec::new(),
        }
    }

    #[test]
    fn strength_catches_common_and_short_passwords() {
        for weak in [
            "123456",
            "password",
            "Senha123",
            "Password1!",
            "abc123",
            "qwerty2024",
            "aaaaaaaaaaaa",
            "abcdefgh",
            "Flamengo2019",
        ] {
            assert!(
                estimate_bits(weak) < WEAK_BELOW_BITS,
                "{weak} deveria ser fraca ({})",
                estimate_bits(weak)
            );
        }
        for strong in [
            "k#9Lq2!vZp8@xR",
            "correto-cavalo-bateria-grampo",
            "Tq7&mW2#pL9z",
            "s3nh@-ção-forte-87",
        ] {
            assert!(
                estimate_bits(strong) >= WEAK_BELOW_BITS,
                "{strong} deveria ser forte ({})",
                estimate_bits(strong)
            );
        }
        assert_eq!(estimate_bits(""), 0.);
    }

    #[test]
    fn report_finds_weak_reused_and_old() {
        let mut deleted = login("lixo", "123456");
        deleted.deleted = true;
        let items = vec![
            login("a", "k#9Lq2!vZp8@xR"),
            login("b", "k#9Lq2!vZp8@xR"),
            login("c", "123456"),
            login("d", "Tq7&mW2#pL9z"),
            deleted,
        ];
        let now = 2_000_000_000;
        let changed = HashMap::from([
            ("a".to_owned(), now - 400 * 86_400),
            ("d".to_owned(), now - 30 * 86_400),
        ]);
        let r = analyze(&items, &changed, now);
        assert_eq!(r.checked, 4, "a lixeira fica de fora");
        assert_eq!(
            r.weak.iter().map(|w| w.0.as_str()).collect::<Vec<_>>(),
            ["c"]
        );
        assert_eq!(r.reused, [vec!["a".to_owned(), "b".to_owned()]]);
        assert_eq!(r.old, [("a".to_owned(), 400)]);
        assert_eq!(r.affected(), 3);
    }

    #[test]
    fn dates_parse_as_utc() {
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            parse_rfc3339("2024-03-05T12:34:56.789Z"),
            Some(1_709_642_096)
        );
        assert_eq!(parse_rfc3339("2000-02-29T00:00:00"), Some(951_782_400));
        assert_eq!(parse_rfc3339("ontem"), None);
    }

    #[test]
    fn pwned_hash_and_range_parsing() {
        // SHA-1("password") = 5BAA61E4C9B93F3F0682250B6CF8331B7EE68FD8
        let (prefix, suffix) = pwned_hash("password");
        assert_eq!(prefix, "5BAA6");
        assert_eq!(suffix, "1E4C9B93F3F0682250B6CF8331B7EE68FD8");
        let body = "0018A45C4D1DEF81644B54AB7F969B88D65:1\r\n\
                    1E4C9B93F3F0682250B6CF8331B7EE68FD8:9659365\r\n\
                    00D4F6E8FA6EECAD2A3AA415EEC418D38EC:0";
        assert_eq!(count_in_range(body, &suffix), 9_659_365);
        assert_eq!(count_in_range(body, "FFFFF"), 0);
        // Padding (count 0) doesn't count as leaked.
        assert_eq!(
            count_in_range(body, "00D4F6E8FA6EECAD2A3AA415EEC418D38EC"),
            0
        );
    }
}
