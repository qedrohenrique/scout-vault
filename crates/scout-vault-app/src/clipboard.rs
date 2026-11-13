//! Copiar com limpeza automática da área de transferência.
//!
//! No Windows, o app de verdade escreve direto na API do sistema e marca o
//! conteúdo para **não entrar no histórico (Win+V) nem na área de transferência
//! na nuvem** — os mesmos marcadores que gerenciadores de senha usam
//! (`ExcludeClipboardContentFromMonitorProcessing`, `CanIncludeInClipboardHistory = 0`,
//! `CanUploadToCloudClipboard = 0`). Nos testes (plataforma simulada do GPUI)
//! e fora do Windows, usa a área de transferência do GPUI.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gpui_kit::*;
use zeroize::Zeroizing;

pub const CLEAR_AFTER: Duration = Duration::from_secs(30);

static NATIVE: AtomicBool = AtomicBool::new(false);

/// Liga a escrita nativa (com exclusão do histórico). Chamado só pelo app real.
pub fn use_native_clipboard() {
    NATIVE.store(cfg!(windows), Ordering::Relaxed);
}

fn native() -> bool {
    NATIVE.load(Ordering::Relaxed)
}

fn write(text: &str, cx: &mut App) {
    if native() && platform::write_private(text).is_ok() {
        return;
    }
    cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned()));
}

/// Copia `text` e agenda a limpeza. Só apaga se a área de transferência ainda
/// tiver o mesmo valor — não destrói algo que o usuário copiou depois.
///
/// A tarefa é desacoplada da tela: bloquear o cofre não cancela a limpeza.
pub fn copy_and_clear_later(text: &str, cx: &mut App) {
    write(text, cx);
    let expected = Zeroizing::new(text.to_owned());
    cx.spawn(async move |cx| {
        cx.background_executor().timer(CLEAR_AFTER).await;
        cx.update(|cx| clear_if_unchanged(&expected, cx));
    })
    .detach();
}

pub fn clear_if_unchanged(expected: &str, cx: &mut App) {
    let current = cx.read_from_clipboard().and_then(|item| item.text());
    if current.as_deref() == Some(expected) {
        if native() && platform::clear().is_ok() {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(String::new()));
    }
}

#[cfg(windows)]
mod platform {
    use windows::core::w;
    use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
    };
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};

    const CF_UNICODETEXT: u32 = 13;

    /// Abre a área de transferência; outro programa pode estar com ela aberta,
    /// então tenta algumas vezes.
    fn open() -> windows::core::Result<()> {
        let mut last = Ok(());
        for _ in 0..10 {
            // SAFETY: chamada simples da API; fechamos em `close`.
            last = unsafe { OpenClipboard(None) };
            if last.is_ok() {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        last
    }

    /// Copia `bytes` para um bloco global e entrega à área de transferência.
    unsafe fn set(format: u32, bytes: &[u8]) -> windows::core::Result<()> {
        let mem: HGLOBAL = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1))?;
        let ptr = GlobalLock(mem) as *mut u8;
        if ptr.is_null() {
            let _ = GlobalFree(Some(mem));
            return Err(windows::core::Error::from_thread());
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
        let _ = GlobalUnlock(mem);
        // Se der certo, o sistema passa a ser dono da memória.
        if let Err(e) = SetClipboardData(format, Some(HANDLE(mem.0))) {
            let _ = GlobalFree(Some(mem));
            return Err(e);
        }
        Ok(())
    }

    pub fn write_private(text: &str) -> windows::core::Result<()> {
        let mut wide: Vec<u16> = text.encode_utf16().collect();
        wide.push(0);
        // SAFETY: área de transferência aberta por nós até o CloseClipboard abaixo.
        unsafe {
            open()?;
            let result = (|| {
                EmptyClipboard()?;
                let text_bytes =
                    std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2);
                set(CF_UNICODETEXT, text_bytes)?;
                let zero = 0u32.to_le_bytes();
                let exclude =
                    RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing"));
                let history = RegisterClipboardFormatW(w!("CanIncludeInClipboardHistory"));
                let cloud = RegisterClipboardFormatW(w!("CanUploadToCloudClipboard"));
                // Os marcadores são opcionais: se algum falhar, o texto já foi copiado.
                for format in [exclude, history, cloud] {
                    if format != 0 {
                        let _ = set(format, &zero);
                    }
                }
                Ok(())
            })();
            let _ = CloseClipboard();
            // O texto passou para a memória do sistema; apaga a nossa cópia.
            zeroize::Zeroize::zeroize(&mut wide);
            result
        }
    }

    pub fn clear() -> windows::core::Result<()> {
        // SAFETY: idem.
        unsafe {
            open()?;
            let result = EmptyClipboard();
            let _ = CloseClipboard();
            result
        }
    }
}

#[cfg(not(windows))]
mod platform {
    pub fn write_private(_: &str) -> Result<(), ()> {
        Err(())
    }
    pub fn clear() -> Result<(), ()> {
        Err(())
    }
}

#[cfg(all(test, windows))]
mod tests {
    use windows::core::w;
    use windows::Win32::Foundation::HGLOBAL;
    use windows::Win32::System::DataExchange::{
        CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
        RegisterClipboardFormatW,
    };
    use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};

    fn read() -> (Option<String>, bool, bool) {
        unsafe {
            OpenClipboard(None).unwrap();
            let text = GetClipboardData(13).ok().map(|h| {
                let mem = HGLOBAL(h.0);
                let mut p = GlobalLock(mem) as *const u16;
                let mut v = Vec::new();
                while *p != 0 {
                    v.push(*p);
                    p = p.add(1);
                }
                let _ = GlobalUnlock(mem);
                String::from_utf16_lossy(&v)
            });
            let history = IsClipboardFormatAvailable(RegisterClipboardFormatW(w!(
                "CanIncludeInClipboardHistory"
            )))
            .is_ok();
            let exclude = IsClipboardFormatAvailable(RegisterClipboardFormatW(w!(
                "ExcludeClipboardContentFromMonitorProcessing"
            )))
            .is_ok();
            let _ = CloseClipboard();
            (text, history, exclude)
        }
    }

    /// Mexe na área de transferência de verdade (guarda e devolve o texto que
    /// havia nela). Rode com `--ignored`.
    #[test]
    #[ignore]
    fn private_copy_is_marked_out_of_history() {
        let before = read().0;
        super::platform::write_private("segredo-de-teste-çã").unwrap();
        let (text, history, exclude) = read();
        assert_eq!(text.as_deref(), Some("segredo-de-teste-çã"));
        assert!(history && exclude, "marcadores ausentes");
        super::platform::clear().unwrap();
        assert_eq!(read().0, None);
        if let Some(before) = before {
            super::platform::write_private(&before).unwrap();
        }
    }
}
