//! Password, passphrase and username generator using system randomness
//! (`getrandom`), without modulo bias.
//!
//! Passphrases use the EFF large wordlist (7,776 words, CC BY 3.0,
//! <https://www.eff.org/dice>): each word adds log2(7776) ≈ 12.9 bits.

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

pub const MIN_LENGTH: usize = 5;
pub const MAX_LENGTH: usize = 128;

const LOWER: &str = "abcdefghijklmnopqrstuvwxyz";
const UPPER: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const DIGITS: &str = "0123456789";
/// Same symbol set as Bitwarden's generator.
const SYMBOLS: &str = "!@#$%^&*";
/// Characters that are easy to confuse when reading: `I l 1 O 0`.
const AMBIGUOUS: &str = "Il1O0";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PasswordOptions {
    pub length: usize,
    pub lowercase: bool,
    pub uppercase: bool,
    pub digits: bool,
    pub symbols: bool,
    pub avoid_ambiguous: bool,
}

impl Default for PasswordOptions {
    fn default() -> Self {
        Self {
            length: 20,
            lowercase: true,
            uppercase: true,
            digits: true,
            symbols: true,
            avoid_ambiguous: false,
        }
    }
}

impl PasswordOptions {
    /// Active sets, already without the ambiguous ones if asked. If none is
    /// checked, uses lowercase (never generates an empty password).
    fn sets(&self) -> Vec<Vec<char>> {
        let mut sets: Vec<&str> = Vec::new();
        if self.lowercase {
            sets.push(LOWER);
        }
        if self.uppercase {
            sets.push(UPPER);
        }
        if self.digits {
            sets.push(DIGITS);
        }
        if self.symbols {
            sets.push(SYMBOLS);
        }
        if sets.is_empty() {
            sets.push(LOWER);
        }
        sets.into_iter()
            .map(|s| {
                s.chars()
                    .filter(|c| !(self.avoid_ambiguous && AMBIGUOUS.contains(*c)))
                    .collect()
            })
            .collect()
    }

    /// Approximate entropy in bits (`length × log2(alphabet)`).
    pub fn entropy_bits(&self) -> f64 {
        let alphabet: usize = self.sets().iter().map(Vec::len).sum();
        self.clamped_length() as f64 * (alphabet as f64).log2()
    }

    fn clamped_length(&self) -> usize {
        self.length.clamp(MIN_LENGTH, MAX_LENGTH)
    }
}

/// Uniform number in `0..n` (rejection sampling: no modulo bias).
fn uniform(n: usize) -> usize {
    assert!(n > 0 && n <= u32::MAX as usize);
    let n = n as u32;
    let zone = u32::MAX - (u32::MAX % n);
    loop {
        let mut b = [0u8; 4];
        getrandom::fill(&mut b).expect("o sistema não forneceu aleatoriedade");
        let v = u32::from_le_bytes(b);
        if v < zone {
            return (v % n) as usize;
        }
    }
}

/// Generates a password with **at least one character from each checked set**.
pub fn password(opts: &PasswordOptions) -> Zeroizing<String> {
    let sets = opts.sets();
    let length = opts.clamped_length().max(sets.len());
    let all: Vec<char> = sets.iter().flatten().copied().collect();

    let mut chars: Zeroizing<Vec<char>> = Zeroizing::new(Vec::with_capacity(length));
    for set in &sets {
        chars.push(set[uniform(set.len())]);
    }
    while chars.len() < length {
        chars.push(all[uniform(all.len())]);
    }
    // Fisher–Yates: the required ones don't always end up at the start.
    for i in (1..chars.len()).rev() {
        chars.swap(i, uniform(i + 1));
    }
    Zeroizing::new(chars.iter().collect())
}

/// The EFF large wordlist, one `ddddd\tword` per line.
const EFF_LIST: &str = include_str!("../assets/eff_large_wordlist.txt");

/// The 7,776 words (parsed once).
pub fn words() -> &'static [&'static str] {
    static WORDS: std::sync::OnceLock<Vec<&'static str>> = std::sync::OnceLock::new();
    WORDS.get_or_init(|| {
        EFF_LIST
            .lines()
            .filter_map(|l| l.split_once('\t').map(|(_, w)| w.trim()))
            .filter(|w| !w.is_empty())
            .collect()
    })
}

pub const MIN_WORDS: usize = 3;
pub const MAX_WORDS: usize = 20;

/// What the generator makes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GeneratorKind {
    #[default]
    Password,
    Passphrase,
    Username,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PassphraseOptions {
    pub words: usize,
    pub separator: String,
    pub capitalize: bool,
    /// One digit appended to one of the words.
    pub include_number: bool,
}

impl Default for PassphraseOptions {
    fn default() -> Self {
        Self {
            words: 6,
            separator: "-".into(),
            capitalize: false,
            include_number: false,
        }
    }
}

impl PassphraseOptions {
    fn clamped_words(&self) -> usize {
        self.words.clamp(MIN_WORDS, MAX_WORDS)
    }

    /// Entropy in bits; the separator and capitals are known, so they add nothing.
    pub fn entropy_bits(&self) -> f64 {
        let n = self.clamped_words() as f64;
        let mut bits = n * (words().len() as f64).log2();
        if self.include_number {
            bits += (n * 10.0).log2();
        }
        bits
    }
}

/// Generates a passphrase (`correct-horse-battery-staple`).
pub fn passphrase(opts: &PassphraseOptions) -> Zeroizing<String> {
    let list = words();
    let n = opts.clamped_words();
    let mut parts: Zeroizing<Vec<String>> = Zeroizing::new(
        (0..n)
            .map(|_| {
                let w = list[uniform(list.len())];
                if opts.capitalize {
                    capitalize(w)
                } else {
                    w.to_string()
                }
            })
            .collect(),
    );
    if opts.include_number {
        let ix = uniform(n);
        parts[ix].push(char::from(b'0' + uniform(10) as u8));
    }
    // The separator is a single character (or none).
    let separator: String = opts.separator.chars().take(1).collect();
    Zeroizing::new(parts.join(&separator))
}

fn capitalize(w: &str) -> String {
    let mut c = w.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// Which kind of username.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsernameKind {
    /// A random word (`dolphin4821`).
    #[default]
    Word,
    /// The account email with a `+word` tag (`name+dolphin@example.com`).
    PlusEmail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UsernameOptions {
    pub kind: UsernameKind,
    pub capitalize: bool,
    /// Four digits at the end of the word.
    pub include_number: bool,
}

impl Default for UsernameOptions {
    fn default() -> Self {
        Self {
            kind: UsernameKind::Word,
            capitalize: false,
            include_number: true,
        }
    }
}

/// Generates a username. `email` is the account's, used by [`UsernameKind::PlusEmail`];
/// if it isn't a usable address, falls back to a word.
pub fn username(opts: &UsernameOptions, email: &str) -> String {
    let list = words();
    let word = list[uniform(list.len())];
    match opts.kind {
        UsernameKind::PlusEmail => {
            if let Some((local, domain)) = email.trim().split_once('@') {
                let local = local.split('+').next().unwrap_or(local);
                if !local.is_empty() && domain.contains('.') {
                    return format!("{local}+{word}@{domain}");
                }
            }
            username(
                &UsernameOptions {
                    kind: UsernameKind::Word,
                    ..*opts
                },
                "",
            )
        }
        UsernameKind::Word => {
            let mut s = if opts.capitalize {
                capitalize(word)
            } else {
                word.to_string()
            };
            if opts.include_number {
                s.push_str(&format!("{:04}", uniform(10_000)));
            }
            s
        }
    }
}

/// Strength in words, from the entropy.
pub fn strength_label(bits: f64) -> &'static str {
    match bits {
        b if b < 40.0 => "Fraca",
        b if b < 64.0 => "Razoável",
        b if b < 100.0 => "Forte",
        _ => "Muito forte",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn respects_length_and_required_sets() {
        let opts = PasswordOptions::default();
        for _ in 0..200 {
            let p = password(&opts);
            assert_eq!(p.chars().count(), 20);
            assert!(p.chars().any(|c| c.is_ascii_lowercase()));
            assert!(p.chars().any(|c| c.is_ascii_uppercase()));
            assert!(p.chars().any(|c| c.is_ascii_digit()));
            assert!(p.chars().any(|c| SYMBOLS.contains(c)));
        }
    }

    #[test]
    fn only_selected_sets_and_no_ambiguous() {
        let opts = PasswordOptions {
            length: 64,
            lowercase: false,
            uppercase: true,
            digits: true,
            symbols: false,
            avoid_ambiguous: true,
        };
        for _ in 0..100 {
            let p = password(&opts);
            assert!(p
                .chars()
                .all(|c| (c.is_ascii_uppercase() || c.is_ascii_digit()) && !AMBIGUOUS.contains(c)));
        }
    }

    #[test]
    fn clamps_length_and_never_empty() {
        let none = PasswordOptions {
            length: 1,
            lowercase: false,
            uppercase: false,
            digits: false,
            symbols: false,
            avoid_ambiguous: false,
        };
        let p = password(&none);
        assert_eq!(p.len(), MIN_LENGTH);
        assert!(p.chars().all(|c| c.is_ascii_lowercase()));
        let long = PasswordOptions {
            length: 10_000,
            ..Default::default()
        };
        assert_eq!(password(&long).len(), MAX_LENGTH);
    }

    #[test]
    fn distribution_is_roughly_uniform() {
        // 10 digits × 20,000 draws: each should come up ~2000 times.
        let mut counts = [0usize; 10];
        for _ in 0..20_000 {
            counts[uniform(10)] += 1;
        }
        assert!(
            counts.iter().all(|&c| (1700..2300).contains(&c)),
            "{counts:?}"
        );
    }

    #[test]
    fn entropy_and_label() {
        let opts = PasswordOptions::default();
        // 70 symbols (26+26+10+8) × 20 ≈ 122 bits.
        assert!((opts.entropy_bits() - 20.0 * 70f64.log2()).abs() < 1e-9);
        assert_eq!(strength_label(opts.entropy_bits()), "Muito forte");
        assert_eq!(strength_label(30.0), "Fraca");
    }

    #[test]
    fn wordlist_is_the_eff_large_list() {
        let w = words();
        assert_eq!(w.len(), 7776);
        assert_eq!(w[0], "abacus");
        assert_eq!(w[7775], "zoom");
        assert!(w
            .iter()
            .all(|w| w.chars().all(|c| c.is_ascii_lowercase() || c == '-')));
    }

    #[test]
    fn passphrases_follow_the_options() {
        let opts = PassphraseOptions::default();
        let p = passphrase(&opts);
        assert!(p.split('-').count() >= 6, "{:?}", p.as_str()); // a few EFF words contain '-'
        assert!((opts.entropy_bits() - 6.0 * 7776f64.log2()).abs() < 1e-9);

        let opts = PassphraseOptions {
            words: 4,
            separator: " ".into(),
            capitalize: true,
            include_number: true,
        };
        for _ in 0..50 {
            let p = passphrase(&opts);
            let parts: Vec<&str> = p.split(' ').collect();
            assert_eq!(parts.len(), 4);
            assert!(parts
                .iter()
                .all(|w| w.chars().next().unwrap().is_ascii_uppercase()));
            assert_eq!(p.chars().filter(char::is_ascii_digit).count(), 1);
        }

        let few = PassphraseOptions {
            words: 1,
            separator: String::new(),
            ..Default::default()
        };
        assert!(!passphrase(&few).is_empty());
        assert!((few.entropy_bits() - 3.0 * 7776f64.log2()).abs() < 1e-9);
    }

    #[test]
    fn usernames_follow_the_options() {
        let word = username(&UsernameOptions::default(), "");
        assert!(word.ends_with(|c: char| c.is_ascii_digit()));
        assert!(word[..word.len() - 4]
            .chars()
            .all(|c| c.is_ascii_lowercase() || c == '-'));

        let plus = UsernameOptions {
            kind: UsernameKind::PlusEmail,
            ..Default::default()
        };
        let u = username(&plus, "ana+old@example.com");
        assert!(u.starts_with("ana+") && u.ends_with("@example.com"), "{u}");
        let fallback = username(&plus, "not-an-email");
        assert!(!fallback.contains('@'));

        let caps = UsernameOptions {
            capitalize: true,
            include_number: false,
            ..Default::default()
        };
        let u = username(&caps, "");
        assert!(u.chars().next().unwrap().is_ascii_uppercase());
        assert!(!u.chars().any(|c| c.is_ascii_digit()));
    }
}
