//! Profile photo, chosen on this computer.
//!
//! A copy goes to the app folder (`%APPDATA%\Scout Vault\photos\<hash of the
//! email>`), so the original can move or be deleted. It never goes to the server:
//! Bitwarden has no profile photo, and other apps keep showing the initials.
//!
//! The photo is always saved square (cropped in the editor, at most
//! [`OUTPUT`] pixels, as PNG): GPUI rounds a stretched image before clipping it,
//! so a rectangular picture wouldn't come out as a circle.

use std::collections::HashMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::*;
use image::codecs::png::{CompressionType, FilterType as PngFilter, PngEncoder};
use image::imageops::{self, FilterType};
use image::{DynamicImage, ExtendedColorType, ImageDecoder, ImageEncoder, ImageReader, RgbaImage};
use scout_vault_core::icons::{self, IconFormat};
use sha2::{Digest, Sha256};

/// Biggest photo accepted.
pub const MAX_BYTES: u64 = 8 * 1024 * 1024;
/// Side of the saved photo, at most (it is never enlarged).
pub const OUTPUT: u32 = 512;
/// The editor works on a copy at most this big (on the longest side).
const WORKING_MAX: u32 = 1600;

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

/// Decodes a picture (turned as the camera meant it, by its EXIF orientation).
fn decode(bytes: &[u8]) -> Option<RgbaImage> {
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut decoder = reader.into_decoder().ok()?;
    let orientation = decoder.orientation().ok();
    let mut image = DynamicImage::from_decoder(decoder).ok()?;
    if let Some(orientation) = orientation {
        image.apply_orientation(orientation);
    }
    Some(image.to_rgba8())
}

fn encode_png(image: &RgbaImage) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    PngEncoder::new_with_quality(&mut out, CompressionType::Fast, PngFilter::Adaptive)
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            ExtendedColorType::Rgba8,
        )
        .ok()?;
    Some(out)
}

/// The square in the middle of a picture.
fn center_square(image: &RgbaImage) -> RgbaImage {
    let side = image.width().min(image.height());
    let x = (image.width() - side) / 2;
    let y = (image.height() - side) / 2;
    imageops::crop_imm(image, x, y, side, side).to_image()
}

/// A saved photo, ready to show. Older photos (saved before the editor) may not be
/// square: those show their middle.
fn shown(bytes: Vec<u8>) -> Option<Arc<Image>> {
    let (w, h) = ImageReader::new(Cursor::new(&bytes))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()?;
    if w == h {
        return to_image(bytes);
    }
    to_image(encode_png(&center_square(&decode(&bytes)?))?)
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
        .and_then(shown);
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

/// A picture chosen for the photo, opened for the crop editor.
#[derive(Clone)]
pub struct Picture {
    /// The pixels the crop is taken from.
    pub pixels: Arc<RgbaImage>,
    /// The same picture, to show in the editor.
    pub preview: Arc<Image>,
}

impl Picture {
    pub fn width(&self) -> u32 {
        self.pixels.width()
    }

    pub fn height(&self) -> u32 {
        self.pixels.height()
    }

    /// From pixels already decoded (bigger pictures are shrunk to work on).
    pub fn from_pixels(pixels: RgbaImage) -> Option<Self> {
        let (w, h) = pixels.dimensions();
        if w == 0 || h == 0 {
            return None;
        }
        let pixels = if w.max(h) > WORKING_MAX {
            let k = WORKING_MAX as f32 / w.max(h) as f32;
            let (nw, nh) = (
                ((w as f32 * k).round() as u32).max(1),
                ((h as f32 * k).round() as u32).max(1),
            );
            imageops::resize(&pixels, nw, nh, FilterType::Triangle)
        } else {
            pixels
        };
        let preview = to_image(encode_png(&pixels)?)?;
        Some(Self {
            pixels: Arc::new(pixels),
            preview,
        })
    }
}

/// The part of the picture that becomes the photo: a square, in the picture's
/// pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Crop {
    pub x: f32,
    pub y: f32,
    pub size: f32,
}

/// Opens the image at `path` for the editor (slow for big photos: run it in the
/// background).
pub fn open_file(path: &Path) -> Result<Picture, PhotoError> {
    let size = std::fs::metadata(path)
        .map_err(|_| PhotoError::Unreadable)?
        .len();
    if size > MAX_BYTES {
        return Err(PhotoError::TooBig);
    }
    let bytes = std::fs::read(path).map_err(|_| PhotoError::Unreadable)?;
    match icons::sniff(&bytes) {
        None | Some(IconFormat::Svg) => return Err(PhotoError::NotAnImage),
        Some(_) => {}
    }
    let pixels = decode(&bytes).ok_or(PhotoError::NotAnImage)?;
    Picture::from_pixels(pixels).ok_or(PhotoError::NotAnImage)
}

/// Cuts `crop` out of the picture: a square PNG of at most [`OUTPUT`] pixels.
pub fn cropped_png(picture: &Picture, crop: Crop) -> Option<Vec<u8>> {
    let (w, h) = (picture.width(), picture.height());
    let side = (crop.size.round() as u32).clamp(1, w.min(h));
    let x = (crop.x.round().max(0.) as u32).min(w - side);
    let y = (crop.y.round().max(0.) as u32).min(h - side);
    let part = imageops::crop_imm(&*picture.pixels, x, y, side, side).to_image();
    let part = if side > OUTPUT {
        imageops::resize(&part, OUTPUT, OUTPUT, FilterType::Lanczos3)
    } else {
        part
    };
    encode_png(&part)
}

/// Makes the cropped picture the account's photo (saved in the app folder).
pub fn save(email: &str, picture: &Picture, crop: Crop, cx: &mut App) -> Result<(), PhotoError> {
    let bytes = cropped_png(picture, crop).ok_or(PhotoError::Unreadable)?;
    let image = to_image(bytes.clone()).ok_or(PhotoError::Unreadable)?;
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

#[cfg(test)]
mod tests {
    use super::{cropped_png, decode, encode_png, shown, Crop, Picture, RgbaImage, OUTPUT};

    fn picture(w: u32, h: u32) -> Picture {
        let pixels = RgbaImage::from_fn(w, h, |x, y| image::Rgba([x as u8, y as u8, 0, 255]));
        Picture::from_pixels(pixels).unwrap()
    }

    fn size_of(png: &[u8]) -> (u32, u32) {
        decode(png).unwrap().dimensions()
    }

    #[test]
    fn the_crop_is_a_square_from_the_chosen_spot() {
        let p = picture(300, 200);
        let png = cropped_png(
            &p,
            Crop {
                x: 50.,
                y: 20.,
                size: 100.,
            },
        )
        .unwrap();
        let out = decode(&png).unwrap();
        assert_eq!(out.dimensions(), (100, 100));
        // The top-left pixel is the picture's (50, 20).
        assert_eq!(out.get_pixel(0, 0).0, [50, 20, 0, 255]);
    }

    #[test]
    fn a_crop_outside_the_picture_is_pulled_back_in() {
        let p = picture(300, 200);
        let png = cropped_png(
            &p,
            Crop {
                x: 280.,
                y: -40.,
                size: 500.,
            },
        )
        .unwrap();
        // At most the short side, inside the picture.
        assert_eq!(size_of(&png), (200, 200));
    }

    #[test]
    fn big_pictures_are_shrunk_and_saved_small() {
        let p = picture(4000, 3000);
        assert_eq!((p.width(), p.height()), (1600, 1200));
        let png = cropped_png(
            &p,
            Crop {
                x: 0.,
                y: 0.,
                size: 1200.,
            },
        )
        .unwrap();
        assert_eq!(size_of(&png), (OUTPUT, OUTPUT));
    }

    #[test]
    fn old_rectangular_photos_show_their_middle_as_a_square() {
        let pixels = RgbaImage::from_fn(60, 20, |x, _| image::Rgba([x as u8, 0, 0, 255]));
        let png = encode_png(&pixels).unwrap();
        let shown = shown(png).unwrap();
        let out = decode(shown.bytes()).unwrap();
        assert_eq!(out.dimensions(), (20, 20));
        assert_eq!(out.get_pixel(0, 0).0[0], 20);
    }
}
