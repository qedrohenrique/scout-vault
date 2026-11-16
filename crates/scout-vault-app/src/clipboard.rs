//! Copying with automatic clipboard clearing.
//!
//! On Windows, the real app writes directly to the system API and marks the
//! content to **stay out of the history (Win+V) and the cloud
//! clipboard** — the same markers password managers use
//! (`ExcludeClipboardContentFromMonitorProcessing`, `CanIncludeInClipboardHistory = 0`,
//! `CanUploadToCloudClipboard = 0`). In tests (GPUI's simulated platform)
//! and outside Windows, it uses GPUI's clipboard.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gpui_kit::*;
use zeroize::Zeroizing;

/// Seconds until the clipboard is cleared, if the person didn't choose another value.
pub const DEFAULT_CLEAR_SECS: u32 = 30;

/// Time chosen in settings (0 = don't clear).
struct ClearAfter(u32);

impl Global for ClearAfter {}

/// Changes the clearing time (applies to the next copies).
pub fn set_clear_after_secs(secs: u32, cx: &mut App) {
    cx.set_global(ClearAfter(secs));
}

/// How long what was copied stays on the clipboard (`None` = not cleared).
pub fn clear_after(cx: &App) -> Option<Duration> {
    let secs = cx
        .try_global::<ClearAfter>()
        .map_or(DEFAULT_CLEAR_SECS, |c| c.0);
    (secs > 0).then(|| Duration::from_secs(u64::from(secs)))
}

/// Ending of the copy messages: " · clears in 30 s" (or nothing, if not cleared).
pub fn clear_note(cx: &App) -> String {
    match clear_after(cx) {
        Some(d) => format!(" · {}", t!("tray.clears_in", n = d.as_secs())),
        None => String::new(),
    }
}

static NATIVE: AtomicBool = AtomicBool::new(false);

/// Turns on native writing (with history exclusion). Called only by the real app.
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

/// Copies `text` and schedules the clearing. Only clears if the clipboard still
/// holds the same value — doesn't destroy something the user copied afterwards.
///
/// The task is detached from the screen: locking the vault doesn't cancel the clearing.
pub fn copy_and_clear_later(text: &str, cx: &mut App) {
    write(text, cx);
    let Some(after) = clear_after(cx) else {
        return;
    };
    let expected = Zeroizing::new(text.to_owned());
    cx.spawn(async move |cx| {
        cx.background_executor().timer(after).await;
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

    /// Opens the clipboard; another program may have it open,
    /// so it tries a few times.
    fn open() -> windows::core::Result<()> {
        let mut last = Ok(());
        for _ in 0..10 {
            // SAFETY: simple API call; we close it in `close`.
            last = unsafe { OpenClipboard(None) };
            if last.is_ok() {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        last
    }

    /// Copies `bytes` into a global block and hands it to the clipboard.
    unsafe fn set(format: u32, bytes: &[u8]) -> windows::core::Result<()> {
        let mem: HGLOBAL = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1))?;
        let ptr = GlobalLock(mem) as *mut u8;
        if ptr.is_null() {
            let _ = GlobalFree(Some(mem));
            return Err(windows::core::Error::from_thread());
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
        let _ = GlobalUnlock(mem);
        // If it succeeds, the system becomes the owner of the memory.
        if let Err(e) = SetClipboardData(format, Some(HANDLE(mem.0))) {
            let _ = GlobalFree(Some(mem));
            return Err(e);
        }
        Ok(())
    }

    pub fn write_private(text: &str) -> windows::core::Result<()> {
        let mut wide: Vec<u16> = text.encode_utf16().collect();
        wide.push(0);
        // SAFETY: clipboard opened by us until the CloseClipboard below.
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
                // The markers are optional: if one fails, the text has already been copied.
                for format in [exclude, history, cloud] {
                    if format != 0 {
                        let _ = set(format, &zero);
                    }
                }
                Ok(())
            })();
            let _ = CloseClipboard();
            // The text moved to system memory; wipe our copy.
            zeroize::Zeroize::zeroize(&mut wide);
            result
        }
    }

    pub fn clear() -> windows::core::Result<()> {
        // SAFETY: same as above.
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

    /// Touches the real clipboard (saves and restores the text that
    /// was on it). Run with `--ignored`.
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
