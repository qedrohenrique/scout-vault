//! Profile photo, chosen on this computer.
//!
//! A copy goes to the app folder (`%APPDATA%\Scout Vault\photos\<hash of the
//! email>`), so the original can move or be deleted. It never goes to the server:
//! Bitwarden has no profile photo, and other apps keep showing the initials.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::*;
use scout_vault_core::icons::{self, IconFormat};
use sha2::{Digest, Sha256};

/// Biggest photo accepted.
pub const MAX_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Default)]
struct Photos {
    dir: Option<PathBuf>,
    /// By email (lowercase): the loaded photo, or `None` when there is none.
    cache: HashMap<String, Option<Arc<Image>>>,
}

impl Global for Photos {}

fn state(cx: &mut App) -> &mut Photos {
    if !cx.has_global::<Photos>() {
        cx.set_global(Photos::default());
    }
    cx.global_mut::<Photos>()
}

/// Where the photos live (the preferences folder). `None`: nowhere (in memory).
pub fn init(settings_dir: Option<PathBuf>, cx: &mut App) {
    let s = state(cx);
    let dir = settings_dir.map(|d| d.join("photos"));
    if s.dir != dir {
        s.dir = dir;
        s.cache.clear();
    }
}

fn key(email: &str) -> String {
    email.trim().to_lowercase()
}

fn file_for(dir: &Path, email: &str) -> PathBuf {
    let hash = Sha256::digest(key(email).as_bytes());
    let name: String = hash[..12].iter().map(|b| format!("{b:02x}")).collect();
    dir.join(name)
}

fn to_image(bytes: Vec<u8>) -> Option<Arc<Image>> {
    let format = match icons::sniff(&bytes)? {
        IconFormat::Png => ImageFormat::Png,
        IconFormat::Jpeg => ImageFormat::Jpeg,
        IconFormat::Gif => ImageFormat::Gif,
        IconFormat::Webp => ImageFormat::Webp,
        IconFormat::Ico => ImageFormat::Ico,
        IconFormat::Svg => return None,
    };
    Some(Arc::new(Image::from_bytes(format, bytes)))
}

/// Loads the account's photo from disk (once), so [`get`] can show it.
pub fn load(email: &str, cx: &mut App) {
    let s = state(cx);
    if s.cache.contains_key(&key(email)) {
        return;
    }
    let photo = s
        .dir
        .as_ref()
        .and_then(|dir| std::fs::read(file_for(dir, email)).ok())
        .and_then(to_image);
    s.cache.insert(key(email), photo);
}

/// The account's photo, if it has one (already loaded).
pub fn get(email: &str, cx: &App) -> Option<Arc<Image>> {
    cx.try_global::<Photos>()?.cache.get(&key(email))?.clone()
}

/// Why a file can't be the photo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PhotoError {
    TooBig,
    NotAnImage,
    Unreadable,
}

impl PhotoError {
    pub fn message(&self) -> &'static str {
        match self {
            Self::TooBig => t!("profile.photo_too_big"),
            Self::NotAnImage => t!("profile.photo_not_image"),
            Self::Unreadable => t!("profile.photo_unreadable"),
        }
    }
}

/// Makes the image at `path` the account's photo (copied into the app folder).
pub fn set_from_file(email: &str, path: &Path, cx: &mut App) -> Result<(), PhotoError> {
    let size = std::fs::metadata(path)
        .map_err(|_| PhotoError::Unreadable)?
        .len();
    if size > MAX_BYTES {
        return Err(PhotoError::TooBig);
    }
    let bytes = std::fs::read(path).map_err(|_| PhotoError::Unreadable)?;
    let image = to_image(bytes.clone()).ok_or(PhotoError::NotAnImage)?;
    let s = state(cx);
    if let Some(dir) = &s.dir {
        std::fs::create_dir_all(dir).map_err(|_| PhotoError::Unreadable)?;
        std::fs::write(file_for(dir, email), &bytes).map_err(|_| PhotoError::Unreadable)?;
    }
    s.cache.insert(key(email), Some(image));
    cx.refresh_windows();
    Ok(())
}

/// Goes back to the initials (and deletes the copy).
pub fn remove(email: &str, cx: &mut App) {
    let s = state(cx);
    if let Some(dir) = &s.dir {
        let _ = std::fs::remove_file(file_for(dir, email));
    }
    s.cache.insert(key(email), None);
    cx.refresh_windows();
}

/// Where the account's photo is stored (for tests).
pub fn stored_path(email: &str, cx: &App) -> Option<PathBuf> {
    let dir = cx.try_global::<Photos>()?.dir.as_ref()?;
    Some(file_for(dir, email))
}
