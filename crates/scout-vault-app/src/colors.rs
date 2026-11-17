//! Free colors (picked with the color picker): stored as `#rrggbb`, adjusted so
//! they stay readable on the light and dark themes.

use gpui_kit::{hsla, Hsla, Rgba};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// An opaque RGB color, stored as `"#rrggbb"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u32);

impl Rgb {
    pub fn parse(text: &str) -> Option<Self> {
        let s = text.trim().strip_prefix('#')?;
        let s = match s.len() {
            6 => s.to_owned(),
            3 => s.chars().flat_map(|c| [c, c]).collect(),
            _ => return None,
        };
        u32::from_str_radix(&s, 16).ok().map(Rgb)
    }

    pub fn to_hex(self) -> String {
        format!("#{:06x}", self.0 & 0xff_ffff)
    }

    pub fn from_hsla(c: Hsla) -> Self {
        let r = Rgba::from(c);
        let byte = |v: f32| (v.clamp(0., 1.) * 255.).round() as u32;
        Rgb((byte(r.r) << 16) | (byte(r.g) << 8) | byte(r.b))
    }

    pub fn to_hsla(self) -> Hsla {
        Rgba {
            r: ((self.0 >> 16) & 0xff) as f32 / 255.,
            g: ((self.0 >> 8) & 0xff) as f32 / 255.,
            b: (self.0 & 0xff) as f32 / 255.,
            a: 1.,
        }
        .into()
    }
}

impl Serialize for Rgb {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Rgb {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        Rgb::parse(&text).ok_or_else(|| serde::de::Error::custom("cor inválida"))
    }
}

/// WCAG relative luminance.
pub fn luminance(c: Hsla) -> f32 {
    let r = Rgba::from(c);
    let lin = |v: f32| {
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(r.r) + 0.7152 * lin(r.g) + 0.0722 * lin(r.b)
}

/// WCAG contrast ratio (1–21).
pub fn contrast(a: Hsla, b: Hsla) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// The color with its lightness moved (the hue and saturation stay) until it
/// reaches `ratio` against `against`, stepping darker or lighter.
pub fn until_contrast(c: Hsla, against: Hsla, ratio: f32, darker: bool) -> Hsla {
    let mut out = c;
    for _ in 0..60 {
        if contrast(out, against) >= ratio {
            break;
        }
        out.l = if darker {
            (out.l - 0.015).max(0.)
        } else {
            (out.l + 0.015).min(1.)
        };
    }
    out
}

/// An icon color that stands out on the theme's background (3:1, like icons need).
pub fn readable(c: Hsla, dark: bool) -> Hsla {
    let background = if dark {
        hsla(0., 0., 0.08, 1.)
    } else {
        hsla(0., 0., 0.98, 1.)
    };
    until_contrast(c, background, 3.0, !dark)
}

/// Accent tones for a free color: (light, on light, dark, on dark), each pair
/// with 4.5:1 between the button and its text, as the built-in accents.
pub fn accent_tones(rgb: Rgb) -> (String, String, String, String) {
    let base = rgb.to_hsla();
    let white = hsla(0., 0., 1., 1.);
    let light = until_contrast(base, white, 4.5, true);
    let on_dark = hsla(base.h, base.s.min(0.6), 0.09, 1.);
    let dark = until_contrast(
        Hsla {
            l: base.l.max(0.55),
            ..base
        },
        on_dark,
        4.5,
        false,
    );
    (
        Rgb::from_hsla(light).to_hex(),
        "#ffffff".into(),
        Rgb::from_hsla(dark).to_hex(),
        Rgb::from_hsla(on_dark).to_hex(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips() {
        assert_eq!(Rgb::parse("#0B7D62").unwrap().to_hex(), "#0b7d62");
        assert_eq!(Rgb::parse("#fa0").unwrap().to_hex(), "#ffaa00");
        assert!(Rgb::parse("0b7d62").is_none());
        assert!(Rgb::parse("#zzzzzz").is_none());
        let c = Rgb::parse("#3366cc").unwrap();
        assert_eq!(Rgb::from_hsla(c.to_hsla()), c);
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(json, "\"#3366cc\"");
        assert_eq!(serde_json::from_str::<Rgb>(&json).unwrap(), c);
    }

    #[test]
    fn any_color_becomes_a_readable_accent() {
        for hex in [
            "#ffff00", "#00ff00", "#000000", "#ffffff", "#ff00ff", "#123456",
        ] {
            let (light, on_light, dark, on_dark) = accent_tones(Rgb::parse(hex).unwrap());
            let p = |s: &str| Rgb::parse(s).unwrap().to_hsla();
            assert!(contrast(p(&light), p(&on_light)) >= 4.4, "{hex} claro");
            assert!(contrast(p(&dark), p(&on_dark)) >= 4.4, "{hex} escuro");
        }
    }

    #[test]
    fn icon_colors_stand_out() {
        let yellow = Rgb::parse("#ffff00").unwrap().to_hsla();
        assert!(contrast(readable(yellow, false), hsla(0., 0., 0.98, 1.)) >= 2.9);
        let navy = Rgb::parse("#000033").unwrap().to_hsla();
        assert!(contrast(readable(navy, true), hsla(0., 0., 0.08, 1.)) >= 2.9);
    }
}
