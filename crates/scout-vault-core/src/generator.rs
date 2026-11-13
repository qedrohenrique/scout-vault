//! Gerador de senhas com aleatoriedade do sistema (`getrandom`), sem viés de módulo.

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

pub const MIN_LENGTH: usize = 5;
pub const MAX_LENGTH: usize = 128;

const LOWER: &str = "abcdefghijklmnopqrstuvwxyz";
const UPPER: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const DIGITS: &str = "0123456789";
/// Mesmo conjunto de símbolos do gerador do Bitwarden.
const SYMBOLS: &str = "!@#$%^&*";
/// Caracteres que se confundem na leitura: `I l 1 O 0`.
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
    /// Conjuntos ativos, já sem os ambíguos se pedido. Se nenhum estiver
    /// marcado, usa minúsculas (nunca gera senha vazia).
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

    /// Entropia aproximada em bits (`comprimento × log2(alfabeto)`).
    pub fn entropy_bits(&self) -> f64 {
        let alphabet: usize = self.sets().iter().map(Vec::len).sum();
        self.clamped_length() as f64 * (alphabet as f64).log2()
    }

    fn clamped_length(&self) -> usize {
        self.length.clamp(MIN_LENGTH, MAX_LENGTH)
    }
}

/// Número uniforme em `0..n` (rejeição: sem viés de módulo).
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

/// Gera uma senha com **pelo menos um caractere de cada conjunto marcado**.
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
    // Fisher–Yates: os obrigatórios não ficam sempre no começo.
    for i in (1..chars.len()).rev() {
        chars.swap(i, uniform(i + 1));
    }
    Zeroizing::new(chars.iter().collect())
}

/// Força em palavras, a partir da entropia.
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
        // 10 dígitos × 20 000 sorteios: cada um deve sair ~2000 vezes.
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
        // 70 símbolos (26+26+10+8) × 20 ≈ 122 bits.
        assert!((opts.entropy_bits() - 20.0 * 70f64.log2()).abs() < 1e-9);
        assert_eq!(strength_label(opts.entropy_bits()), "Muito forte");
        assert_eq!(strength_label(30.0), "Fraca");
    }
}
