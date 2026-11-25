//! Temporary files for attachments opened with another program.
//!
//! To open a PDF or an image in the default program, the decrypted contents
//! must exist in a file. It goes to `%TEMP%\ScoutVault-anexos\<random>\`
//! and is deleted when the vault locks (or on the next launch, if the app crashed).
//! "Save as…" writes wherever the person chooses and is not deleted.

use std::path::{Path, PathBuf};

static SYSTEM_OPEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Allows opening files with the default program (real app only; tests have no system).
pub fn enable_system_open() {
    SYSTEM_OPEN.store(true, std::sync::atomic::Ordering::Relaxed);
}

pub fn system_open_enabled() -> bool {
    SYSTEM_OPEN.load(std::sync::atomic::Ordering::Relaxed)
}

fn temp_root() -> PathBuf {
    std::env::temp_dir().join("ScoutVault-anexos")
}

/// Invisible formatting characters: direction overrides and isolates (`contrato`,
/// U+202E, `fdp.exe` shows as `contratoexe.pdf`), zero-width ones, the BOM.
fn is_format_char(c: char) -> bool {
    matches!(
        c,
        '\u{061C}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2069}'
            | '\u{FEFF}'
    )
}

/// File-system-safe name (the name comes from the vault, i.e. from whoever attached it).
pub fn safe_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| !is_format_char(*c))
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

/// Extensions Windows runs (or that run something) when "opened". The app never opens
/// these by itself: in a shared organization, someone else chooses the attachment's name.
const DANGEROUS: &[&str] = &[
    "exe",
    "com",
    "scr",
    "pif",
    "cpl",
    "msi",
    "msix",
    "msixbundle",
    "msp",
    "appx",
    "appxbundle",
    "appinstaller",
    "bat",
    "cmd",
    "ps1",
    "psm1",
    "psd1",
    "vbs",
    "vbe",
    "js",
    "jse",
    "wsf",
    "wsh",
    "ws",
    "hta",
    "lnk",
    "url",
    "scf",
    "reg",
    "inf",
    "chm",
    "jar",
    "dll",
    "sys",
    "ocx",
    "gadget",
    "application",
    "library-ms",
    "searchconnector-ms",
    "settingcontent-ms",
    "website",
    "xll",
    "xlam",
    "docm",
    "dotm",
    "xlsm",
    "xltm",
    "pptm",
    "potm",
    "ppam",
    "sldm",
    "iso",
    "img",
    "vhd",
    "vhdx",
    "diagcab",
    "msc",
];

/// Would opening this file run something? (By its extension, after [`safe_name`].)
pub fn is_dangerous(name: &str) -> bool {
    let name = safe_name(name);
    let name = name.trim_end_matches([' ', '.']);
    name.rsplit_once('.')
        .is_some_and(|(_, ext)| DANGEROUS.iter().any(|d| ext.eq_ignore_ascii_case(d)))
}

/// Marks a file as "from the internet" (Mark-of-the-Web: the `Zone.Identifier` stream,
/// zone 3), so SmartScreen and Office's Protected View treat it as untrusted.
pub fn mark_of_the_web(path: &Path) {
    #[cfg(windows)]
    {
        let mut stream = path.as_os_str().to_owned();
        stream.push(":Zone.Identifier");
        let _ = std::fs::write(stream, "[ZoneTransfer]\r\nZoneId=3\r\n");
    }
    #[cfg(not(windows))]
    let _ = path;
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
    mark_of_the_web(&path);
    Ok(path)
}

/// Deletes the copy and its random folder. If the file is still open in
/// another program, Windows doesn't allow it; it is left for the next cleanup.
pub fn remove_temp(path: &Path) {
    let _ = std::fs::remove_file(path);
    if let Some(dir) = path.parent() {
        let _ = std::fs::remove_dir(dir);
    }
}

pub fn cleanup_temp() {
    let _ = std::fs::remove_dir_all(temp_root());
}

/// Folder suggested in "Save as…".
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
    fn invisible_direction_tricks_are_removed_and_executables_spotted() {
        // Shows as "contratoexe.pdf"; really "contratofdp.exe".
        let tricked = "contrato\u{202E}fdp.exe";
        assert_eq!(safe_name(tricked), "contratofdp.exe");
        assert!(is_dangerous(tricked));
        for name in [
            "setup.EXE",
            "fatura.pdf.lnk",
            "x.hta",
            "a.js",
            "atalho.url",
            "m.docm",
            "y.exe. ",
        ] {
            assert!(is_dangerous(name), "{name}");
        }
        for name in [
            "contrato.pdf",
            "foto.png",
            "notas.txt",
            "planilha.xlsx",
            "sem-extensao",
        ] {
            assert!(!is_dangerous(name), "{name}");
        }
    }

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
        #[cfg(windows)]
        {
            let mut stream = path.as_os_str().to_owned();
            stream.push(":Zone.Identifier");
            let motw = std::fs::read_to_string(stream).expect("Mark-of-the-Web");
            assert!(motw.contains("ZoneId=3"), "{motw}");
        }
        assert!(path.starts_with(temp_root()));
        remove_temp(&path);
        assert!(!path.exists());
        assert!(!path.parent().unwrap().exists());
    }
}
