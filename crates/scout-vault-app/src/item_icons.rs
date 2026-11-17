//! Icon and color chosen for an item.
//!
//! The Bitwarden server has no field for this, so the choice stays
//! **on this computer** (in the settings, by item id) — nothing goes into the
//! item, which stays the same in other apps. Without a choice, the site's brand
//! applies (if the address belongs to a known service) or the type's icon.
//!
//! Brand icons come from Simple Icons (simpleicons.org, CC0); the brands
//! belong to their owners and are used only to identify the item.

use std::collections::HashMap;

use gpui_kit::assets::IconName as I;
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use scout_vault_core::vault::Item;
use serde::{Deserialize, Serialize};

use crate::appearance;

/// An item's icon: one from the Lucide collection or a service's brand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemIcon {
    Lucide(I),
    /// Simple Icons slug (file `assets/brands/<slug>.svg`).
    Brand(&'static str),
}

impl From<I> for ItemIcon {
    fn from(icon: I) -> Self {
        Self::Lucide(icon)
    }
}

impl gpui_kit::assets::IconNamed for ItemIcon {
    fn path(self) -> SharedString {
        match self {
            Self::Lucide(icon) => icon.path(),
            Self::Brand(slug) => format!("{BRAND_DIR}{slug}.svg").into(),
        }
    }
}

/// Virtual folder of the brand icons (served by `assets::AppAssets`).
pub const BRAND_DIR: &str = "brands/";

/// A brand: slug, name and the site's domains (to pick it automatically).
pub struct Brand {
    pub slug: &'static str,
    pub name: &'static str,
    pub domains: &'static [&'static str],
    pub svg: &'static [u8],
}

macro_rules! brand {
    ($slug:literal, $name:literal, [$($d:literal),*]) => {
        Brand {
            slug: $slug,
            name: $name,
            domains: &[$($d),*],
            svg: include_bytes!(concat!("../assets/brands/", $slug, ".svg")),
        }
    };
}

/// Offered brands, in picker order.
pub const BRANDS: &[Brand] = &[
    brand!("google", "Google", ["google.com", "accounts.google.com"]),
    brand!("gmail", "Gmail", ["gmail.com", "mail.google.com"]),
    brand!("googledrive", "Google Drive", ["drive.google.com"]),
    brand!("youtube", "YouTube", ["youtube.com"]),
    brand!("claude", "Claude", ["claude.ai"]),
    brand!(
        "anthropic",
        "Anthropic",
        ["anthropic.com", "console.anthropic.com"]
    ),
    brand!("openai", "OpenAI", ["openai.com", "chatgpt.com"]),
    brand!(
        "riotgames",
        "Riot Games",
        ["riotgames.com", "account.riotgames.com"]
    ),
    brand!("valorant", "Valorant", ["playvalorant.com"]),
    brand!(
        "leagueoflegends",
        "League of Legends",
        ["leagueoflegends.com"]
    ),
    brand!("discord", "Discord", ["discord.com", "discordapp.com"]),
    brand!("instagram", "Instagram", ["instagram.com"]),
    brand!("facebook", "Facebook", ["facebook.com"]),
    brand!("whatsapp", "WhatsApp", ["whatsapp.com"]),
    brand!("telegram", "Telegram", ["telegram.org"]),
    brand!("x", "X", ["x.com", "twitter.com"]),
    brand!("tiktok", "TikTok", ["tiktok.com"]),
    brand!("reddit", "Reddit", ["reddit.com"]),
    brand!("linkedin", "LinkedIn", ["linkedin.com"]),
    brand!("twitch", "Twitch", ["twitch.tv"]),
    brand!("spotify", "Spotify", ["spotify.com"]),
    brand!("netflix", "Netflix", ["netflix.com"]),
    brand!("steam", "Steam", ["steampowered.com", "steamcommunity.com"]),
    brand!("epicgames", "Epic Games", ["epicgames.com"]),
    brand!("battledotnet", "Battle.net", ["battle.net", "blizzard.com"]),
    brand!("ea", "EA", ["ea.com"]),
    brand!("ubisoft", "Ubisoft", ["ubisoft.com", "ubi.com"]),
    brand!(
        "playstation",
        "PlayStation",
        ["playstation.com", "sonyentertainmentnetwork.com"]
    ),
    brand!("xbox", "Xbox", ["xbox.com"]),
    brand!("nintendoswitch", "Nintendo", ["nintendo.com"]),
    brand!(
        "microsoft",
        "Microsoft",
        ["microsoft.com", "live.com", "outlook.com", "office.com"]
    ),
    brand!("apple", "Apple", ["apple.com", "icloud.com"]),
    brand!("amazon", "Amazon", ["amazon.com", "amazon.com.br"]),
    brand!("github", "GitHub", ["github.com"]),
    brand!("gitlab", "GitLab", ["gitlab.com"]),
    brand!("cloudflare", "Cloudflare", ["cloudflare.com"]),
    brand!("notion", "Notion", ["notion.so", "notion.com"]),
    brand!("slack", "Slack", ["slack.com"]),
    brand!("figma", "Figma", ["figma.com"]),
    brand!("dropbox", "Dropbox", ["dropbox.com"]),
    brand!("protonmail", "Proton", ["proton.me", "protonmail.com"]),
    brand!("paypal", "PayPal", ["paypal.com"]),
    brand!("nubank", "Nubank", ["nubank.com.br"]),
    brand!(
        "mercadopago",
        "Mercado Pago",
        ["mercadopago.com.br", "mercadolivre.com.br"]
    ),
    brand!("ifood", "iFood", ["ifood.com.br"]),
    brand!("uber", "Uber", ["uber.com"]),
];

/// A brand's SVG by asset path (`brands/<slug>.svg`).
pub fn brand_svg(path: &str) -> Option<&'static [u8]> {
    let slug = path.strip_prefix(BRAND_DIR)?.strip_suffix(".svg")?;
    BRANDS.iter().find(|b| b.slug == slug).map(|b| b.svg)
}

/// Brand of a login's site, by domain (`accounts.google.com` → Google).
pub fn brand_for_url(url: &str) -> Option<&'static Brand> {
    let host = crate::browser::host_of(url)?;
    // The most specific domain wins (mail.google.com → Gmail, not Google).
    BRANDS
        .iter()
        .flat_map(|b| b.domains.iter().map(move |d| (b, *d)))
        .filter(|(_, d)| host == *d || host.ends_with(&format!(".{d}")))
        .max_by_key(|(_, d)| d.len())
        .map(|(b, _)| b)
}

/// Chosen icon (name from [`CHOICES`]) and color (id from [`COLORS`] or a free
/// `#rrggbb`; no color = theme accent).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CustomIcon {
    pub icon: String,
    #[serde(default)]
    pub color: Option<String>,
}

/// Offered (Lucide) icons, with the name stored in the settings. The
/// brands live in [`BRANDS`] and are stored as `brand:<slug>`.
pub const CHOICES: &[(&str, I)] = &[
    ("globe", I::Globe),
    ("gamepad", I::Gamepad2),
    ("swords", I::Swords),
    ("trophy", I::Trophy),
    ("landmark", I::Landmark),
    ("wallet", I::Wallet),
    ("credit-card", I::CreditCard),
    ("piggy-bank", I::PiggyBank),
    ("bitcoin", I::Bitcoin),
    ("shopping-cart", I::ShoppingCart),
    ("shopping-bag", I::ShoppingBag),
    ("gift", I::Gift),
    ("mail", I::Mail),
    ("message", I::MessageCircle),
    ("phone", I::Phone),
    ("smartphone", I::Smartphone),
    ("laptop", I::Laptop),
    ("monitor", I::Monitor),
    ("server", I::Server),
    ("database", I::Database),
    ("wifi", I::Wifi),
    ("cloud", I::Cloud),
    ("code", I::Code),
    ("terminal", I::Terminal),
    ("briefcase", I::Briefcase),
    ("building", I::Building2),
    ("graduation", I::GraduationCap),
    ("health", I::HeartPulse),
    ("dumbbell", I::Dumbbell),
    ("plane", I::Plane),
    ("car", I::Car),
    ("house", I::House),
    ("utensils", I::Utensils),
    ("coffee", I::Coffee),
    ("music", I::Music),
    ("film", I::Film),
    ("tv", I::Tv),
    ("camera", I::Camera),
    ("palette", I::Palette),
    ("book", I::BookOpen),
    ("file", I::FileText),
    ("note", I::StickyNote),
    ("id-card", I::IdCard),
    ("key", I::KeyRound),
    ("lock", I::Lock),
    ("shield", I::Shield),
    ("user", I::User),
    ("users", I::Users),
    ("star", I::Star),
    ("sparkles", I::Sparkles),
    ("zap", I::Zap),
    ("rocket", I::Rocket),
];

/// Offered colors: (stored id, name, hue 0–1).
pub const COLORS: &[(&str, &str, f32)] = &[
    ("red", "Vermelho", 0.0),
    ("orange", "Laranja", 0.07),
    ("amber", "Âmbar", 0.12),
    ("green", "Verde", 0.38),
    ("teal", "Turquesa", 0.48),
    ("blue", "Azul", 0.6),
    ("violet", "Violeta", 0.74),
    ("pink", "Rosa", 0.9),
];

/// Stored name for a brand.
pub fn brand_choice_name(slug: &str) -> String {
    format!("brand:{slug}")
}

pub fn icon_by_name(name: &str) -> Option<ItemIcon> {
    if let Some(slug) = name.strip_prefix("brand:") {
        return BRANDS
            .iter()
            .find(|b| b.slug == slug)
            .map(|b| ItemIcon::Brand(b.slug));
    }
    CHOICES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, i)| ItemIcon::Lucide(*i))
}

/// Color of an id from [`COLORS`] or a free `#rrggbb`, adjusted to the light/dark theme.
pub fn color_by_id(id: &str, cx: &App) -> Option<Hsla> {
    let dark = cx.theme().mode.is_dark();
    if let Some(rgb) = crate::colors::Rgb::parse(id) {
        return Some(crate::colors::readable(rgb.to_hsla(), dark));
    }
    let (_, _, hue) = COLORS.iter().find(|(c, _, _)| *c == id)?;
    Some(hsla(*hue, 0.72, if dark { 0.64 } else { 0.48 }, 1.))
}

/// Icon choices by item id (valid for the app session).
#[derive(Default)]
pub struct ItemIcons(pub HashMap<String, CustomIcon>);

impl Global for ItemIcons {}

/// Replaces all choices (on app start, from the settings).
pub fn set_all(icons: HashMap<String, CustomIcon>, cx: &mut App) {
    cx.set_global(ItemIcons(icons));
}

/// An item's choice (`None` goes back to the type icon). Returns all of them, for saving.
pub fn set(item_id: &str, choice: Option<CustomIcon>, cx: &mut App) -> HashMap<String, CustomIcon> {
    let mut all = cx
        .try_global::<ItemIcons>()
        .map(|g| g.0.clone())
        .unwrap_or_default();
    match choice {
        Some(c) => {
            all.insert(item_id.to_owned(), c);
        }
        None => {
            all.remove(item_id);
        }
    }
    cx.set_global(ItemIcons(all.clone()));
    all
}

pub fn choice(item_id: &str, cx: &App) -> Option<CustomIcon> {
    cx.try_global::<ItemIcons>()
        .and_then(|g| g.0.get(item_id).cloned())
}

/// The item's icon: the chosen one, the site's brand or the type's.
pub fn icon_for(item: &Item, cx: &App) -> ItemIcon {
    choice(&item.id, cx)
        .and_then(|c| icon_by_name(&c.icon))
        .or_else(|| site_brand(item).map(|b| ItemIcon::Brand(b.slug)))
        .unwrap_or_else(|| crate::item_list::kind_icon(&item.kind).into())
}

/// Domain whose website icon the item shows: only without a chosen icon or a known brand.
pub fn site_host(item: &Item, cx: &App) -> Option<String> {
    if choice(&item.id, cx).is_some() || site_brand(item).is_some() {
        return None;
    }
    crate::site_icons::host_of_item(item)
}

/// Square with a website's icon (downloaded image).
pub fn site_chip(image: std::sync::Arc<Image>, size: Pixels, highlighted: bool, cx: &App) -> Div {
    div()
        .size(size)
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded(cx.theme().radius)
        .bg(if highlighted {
            cx.theme().secondary_hover
        } else {
            cx.theme().secondary
        })
        .child(img(image).size(size * 0.58))
}

/// Brand of a login's first known address.
pub fn site_brand(item: &Item) -> Option<&'static Brand> {
    match &item.kind {
        scout_vault_core::vault::ItemKind::Login(l) => l.uris.iter().find_map(|u| brand_for_url(u)),
        _ => None,
    }
}

/// Color chosen for the item, if any.
pub fn color_for(item: &Item, cx: &App) -> Option<Hsla> {
    choice(&item.id, cx)
        .and_then(|c| c.color)
        .and_then(|id| color_by_id(&id, cx))
}

/// Square with the item's icon, in the chosen color (or in the default style).
pub fn chip(item: &Item, size: Pixels, highlighted: bool, cx: &App) -> Div {
    if let Some(image) = site_host(item, cx).and_then(|h| crate::site_icons::image(&h, cx)) {
        return site_chip(image, size, highlighted, cx);
    }
    chip_with(
        icon_for(item, cx),
        color_for(item, cx),
        size,
        highlighted,
        cx,
    )
}

/// Square with an icon in the given color (or in the default style, without color).
pub fn chip_with(
    icon: ItemIcon,
    color: Option<Hsla>,
    size: Pixels,
    highlighted: bool,
    cx: &App,
) -> Div {
    match color {
        Some(color) => div()
            .size(size)
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .rounded(cx.theme().radius)
            .bg(color.opacity(if highlighted { 0.24 } else { 0.16 }))
            .text_color(color)
            .child(Icon::new(icon).size(size * 0.5)),
        None => crate::ui::icon_chip(icon, size, highlighted, cx),
    }
}

/// Accent color, so the picker can mark the "theme" color.
pub fn theme_color(cx: &App) -> Hsla {
    appearance::accent(cx)
}

#[cfg(test)]
mod tests {
    use gpui_kit::AssetSource;

    #[test]
    fn every_choice_is_bundled_and_unique() {
        let assets = crate::assets::AppAssets;
        let mut names = Vec::new();
        for (name, icon) in super::CHOICES {
            let path = icon.path();
            assert!(
                assets.load(&path).unwrap().is_some(),
                "ícone {name} ({path}) não está no binário"
            );
            assert!(!names.contains(name), "nome repetido: {name}");
            names.push(name);
        }
        assert_eq!(
            super::icon_by_name("gamepad"),
            Some(super::ItemIcon::Lucide(super::I::Gamepad2))
        );
        assert_eq!(super::icon_by_name("não existe"), None);
        assert_eq!(
            super::icon_by_name("brand:discord"),
            Some(super::ItemIcon::Brand("discord"))
        );
        assert_eq!(super::icon_by_name("brand:não-existe"), None);
    }

    #[test]
    fn every_brand_is_an_svg_and_unique() {
        use super::{brand_svg, BRANDS, BRAND_DIR};
        let mut slugs = Vec::new();
        for b in BRANDS {
            let svg = std::str::from_utf8(b.svg).unwrap();
            assert!(svg.starts_with("<svg"), "{} não é SVG", b.slug);
            assert!(!slugs.contains(&b.slug), "marca repetida: {}", b.slug);
            assert!(!b.domains.is_empty(), "{} sem domínio", b.slug);
            slugs.push(b.slug);
            let path = format!("{BRAND_DIR}{}.svg", b.slug);
            assert!(brand_svg(&path).is_some());
            assert!(crate::assets::AppAssets.load(&path).unwrap().is_some());
        }
    }

    #[test]
    fn brands_follow_the_site_address() {
        use super::brand_for_url;
        let slug = |u: &str| brand_for_url(u).map(|b| b.slug);
        assert_eq!(slug("https://accounts.google.com/signin"), Some("google"));
        assert_eq!(slug("https://mail.google.com/"), Some("gmail"));
        assert_eq!(slug("https://www.discord.com/login"), Some("discord"));
        assert_eq!(slug("twitter.com"), Some("x"));
        assert_eq!(
            slug("https://authenticate.riotgames.com"),
            Some("riotgames")
        );
        assert_eq!(slug("https://notgoogle.com"), None);
        assert_eq!(slug("https://exemplo.com.br"), None);
    }
}
