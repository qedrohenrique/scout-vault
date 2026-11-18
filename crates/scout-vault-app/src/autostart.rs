//! "Start with Windows": the value `Scout Vault` in
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, which opens the app hidden in
//! the tray (`--tray`). The installer's checkbox writes the same value, so the
//! setting and the installer always agree: the registry is the only truth.

use std::sync::atomic::{AtomicBool, Ordering};

/// Name of the value under `Run`.
pub const VALUE_NAME: &str = "Scout Vault";
/// Argument that opens the app straight into the tray.
pub const TRAY_ARG: &str = "--tray";

static SYSTEM: AtomicBool = AtomicBool::new(false);
/// Outside the real app (tests), the choice only lives here.
static FAKE: AtomicBool = AtomicBool::new(false);

/// In the real app: read and write the Windows registry.
pub fn enable_system() {
    SYSTEM.store(true, Ordering::Relaxed);
}

/// What goes in the registry: this executable, hidden in the tray.
pub fn command_line() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    Some(format!("\"{}\" {TRAY_ARG}", exe.display()))
}

pub fn is_enabled() -> bool {
    if !SYSTEM.load(Ordering::Relaxed) {
        return FAKE.load(Ordering::Relaxed);
    }
    platform::read().is_some()
}

/// Turns it on (pointing to this executable) or off.
pub fn set(on: bool) -> Result<(), String> {
    if !SYSTEM.load(Ordering::Relaxed) {
        FAKE.store(on, Ordering::Relaxed);
        return Ok(());
    }
    if on {
        let command = command_line().ok_or_else(|| t!("autostart.failed").to_string())?;
        platform::write(&command)
    } else {
        platform::delete();
        Ok(())
    }
}

/// On start: if it's on but points to another place (the app moved or was
/// updated elsewhere), points it to this executable.
pub fn refresh() {
    if !SYSTEM.load(Ordering::Relaxed) {
        return;
    }
    if let (Some(current), Some(wanted)) = (platform::read(), command_line()) {
        if current != wanted {
            let _ = platform::write(&wanted);
        }
    }
}

#[cfg(windows)]
mod platform {
    use windows::core::HSTRING;
    use windows::Win32::System::Registry::{
        RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ,
    };

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

    pub fn read() -> Option<String> {
        let key = HSTRING::from(RUN_KEY);
        let name = HSTRING::from(super::VALUE_NAME);
        let mut size = 0u32;
        // SAFETY: asks only for the size of a REG_SZ value of the current user.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                &key,
                &name,
                RRF_RT_REG_SZ,
                None,
                None,
                Some(&mut size),
            )
        };
        if status.is_err() || size == 0 {
            return None;
        }
        let mut buf = vec![0u16; (size as usize).div_ceil(2)];
        // SAFETY: `buf` holds `size` bytes, as the call above reported.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                &key,
                &name,
                RRF_RT_REG_SZ,
                None,
                Some(buf.as_mut_ptr().cast()),
                Some(&mut size),
            )
        };
        if status.is_err() {
            return None;
        }
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..end]))
    }

    pub fn write(command: &str) -> Result<(), String> {
        let value: Vec<u16> = command.encode_utf16().chain(Some(0)).collect();
        // SAFETY: valid strings; the value is a zero-terminated REG_SZ.
        let status = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                &HSTRING::from(RUN_KEY),
                &HSTRING::from(super::VALUE_NAME),
                REG_SZ.0,
                Some(value.as_ptr().cast()),
                (value.len() * 2) as u32,
            )
        };
        if status.is_err() {
            return Err(t!("autostart.failed").to_string());
        }
        Ok(())
    }

    pub fn delete() {
        // SAFETY: only deletes Scout Vault's own value.
        let _ = unsafe {
            RegDeleteKeyValueW(
                HKEY_CURRENT_USER,
                &HSTRING::from(RUN_KEY),
                &HSTRING::from(super::VALUE_NAME),
            )
        };
    }
}

#[cfg(not(windows))]
mod platform {
    pub fn read() -> Option<String> {
        None
    }
    pub fn write(_: &str) -> Result<(), String> {
        Err("só no Windows por enquanto".into())
    }
    pub fn delete() {}
}
