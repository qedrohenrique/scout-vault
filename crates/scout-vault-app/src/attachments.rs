//! Arquivos temporários de anexos abertos com outro programa.
//!
//! Para abrir um PDF ou uma imagem no programa padrão, o conteúdo decifrado
//! precisa existir num arquivo. Ele vai para `%TEMP%\ScoutVault-anexos\<aleatório>\`
//! e é apagado ao bloquear o cofre (ou na próxima abertura, se o app caiu).
//! "Salvar como…" grava onde a pessoa escolher e não é apagado.

use std::path::{Path, PathBuf};

static SYSTEM_OPEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Permite abrir arquivos com o programa padrão (só o app real; nos testes não há sistema).
pub fn enable_system_open() {
    SYSTEM_OPEN.store(true, std::sync::atomic::Ordering::Relaxed);
}

pub fn system_open_enabled() -> bool {
    SYSTEM_OPEN.load(std::sync::atomic::Ordering::Relaxed)
}

fn temp_root() -> PathBuf {
    std::env::temp_dir().join("ScoutVault-anexos")
}

/// Nome seguro para o sistema de arquivos (o nome vem do cofre).
fn safe_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').to_owned();
    if cleaned.is_empty() {
        "anexo".into()
    } else {
        cleaned
    }
}

pub fn write_temp(name: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
    let mut random = [0u8; 8];
    getrandom::fill(&mut random).map_err(|e| std::io::Error::other(e.to_string()))?;
    let dir = temp_root().join(
        random
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>(),
    );
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(safe_name(name));
    std::fs::write(&path, bytes)?;
    Ok(path)
}

/// Apaga a cópia e a pasta aleatória dela. Se o arquivo ainda estiver aberto
/// noutro programa, o Windows não deixa; sobra para a próxima limpeza.
pub fn remove_temp(path: &Path) {
    let _ = std::fs::remove_file(path);
    if let Some(dir) = path.parent() {
        let _ = std::fs::remove_dir(dir);
    }
}

pub fn cleanup_temp() {
    let _ = std::fs::remove_dir_all(temp_root());
}

/// Pasta sugerida no "Salvar como…".
pub fn downloads_dir() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(|h| PathBuf::from(h).join("Downloads"))
        .filter(|d| d.is_dir())
        .unwrap_or_else(std::env::temp_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_made_safe() {
        assert_eq!(safe_name("a/b\\c:d*e?.pdf"), "a_b_c_d_e_.pdf");
        assert_eq!(safe_name("..\\..\\x"), "_.._x");
        assert_eq!(safe_name("  "), "anexo");
    }

    #[test]
    fn temp_copy_is_written_and_removed() {
        let path = write_temp("teste ção.txt", b"oi").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"oi");
        assert!(path.starts_with(temp_root()));
        remove_temp(&path);
        assert!(!path.exists());
        assert!(!path.parent().unwrap().exists());
    }
}
