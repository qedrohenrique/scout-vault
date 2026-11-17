//! Website icons in the list (login items without a chosen icon or a known brand).
//!
//! Kept **in memory only**: a cache on disk would tell which sites are in the vault.
//! Downloaded from the server's icon service, a few at a time, after the vault opens
//! and after each sync. Off with the "Website icons" option.

use std::collections::HashMap;
use std::sync::Arc;

use gpui_kit::*;
use scout_vault_core::api::ServerUrls;
use scout_vault_core::icons::{self, IconFormat};
use scout_vault_core::vault::{Item, ItemKind, Vault};

/// Downloads at the same time.
const PARALLEL: usize = 6;
/// A domain that never exists (`.invalid` is reserved), to learn the service's "no icon" answer.
const NO_ICON_HOST: &str = "scout-vault-no-icon.invalid";

enum Entry {
    Loading,
    Ready(Arc<Image>),
    Missing,
}

#[derive(Default)]
struct SiteIcons {
    enabled: bool,
    service: String,
    cache: HashMap<String, Entry>,
}

impl Global for SiteIcons {}

fn state(cx: &mut App) -> &mut SiteIcons {
    if !cx.has_global::<SiteIcons>() {
        cx.set_global(SiteIcons {
            enabled: true,
            ..Default::default()
        });
    }
    cx.global_mut::<SiteIcons>()
}

/// Turns the icons on or off (off also forgets the downloaded ones).
pub fn set_enabled(enabled: bool, cx: &mut App) {
    let s = state(cx);
    s.enabled = enabled;
    if !enabled {
        s.cache.clear();
    }
    cx.refresh_windows();
}

pub fn enabled(cx: &App) -> bool {
    cx.try_global::<SiteIcons>().is_none_or(|s| s.enabled)
}

/// Domain whose icon the item would show, if it has one worth asking for.
pub fn host_of_item(item: &Item) -> Option<String> {
    let ItemKind::Login(login) = &item.kind else {
        return None;
    };
    login
        .uris
        .iter()
        .filter_map(|u| crate::browser::host_of(u))
        .find(|h| icons::is_fetchable_host(h))
}

/// Icon already downloaded for `host`.
pub fn image(host: &str, cx: &App) -> Option<Arc<Image>> {
    let s = cx.try_global::<SiteIcons>()?;
    if !s.enabled {
        return None;
    }
    match s.cache.get(host)? {
        Entry::Ready(image) => Some(image.clone()),
        _ => None,
    }
}

fn to_image(bytes: Vec<u8>) -> Option<Arc<Image>> {
    if !icons::is_usable(&bytes) {
        return None;
    }
    let format = match icons::sniff(&bytes)? {
        IconFormat::Png => ImageFormat::Png,
        IconFormat::Jpeg => ImageFormat::Jpeg,
        IconFormat::Gif => ImageFormat::Gif,
        IconFormat::Webp => ImageFormat::Webp,
        IconFormat::Ico => ImageFormat::Ico,
        IconFormat::Svg => ImageFormat::Svg,
    };
    Some(Arc::new(Image::from_bytes(format, bytes)))
}

/// Puts an icon in the cache (also used by the tests, which have no network).
pub fn insert(host: &str, bytes: Vec<u8>, cx: &mut App) {
    let entry = to_image(bytes).map_or(Entry::Missing, Entry::Ready);
    state(cx).cache.insert(host.to_owned(), entry);
}

/// Domains of the vault's logins that would show a website icon.
pub fn hosts_of(vault: &Vault) -> Vec<String> {
    let mut hosts: Vec<String> = Vec::new();
    for item in vault.items.iter().filter(|i| !i.deleted) {
        if crate::item_icons::site_brand(item).is_some() {
            continue;
        }
        if let Some(host) = host_of_item(item) {
            if !hosts.contains(&host) {
                hosts.push(host);
            }
        }
    }
    hosts
}

/// Downloads the icons of `hosts` that haven't been asked for yet.
pub fn prefetch(urls: &ServerUrls, hosts: Vec<String>, cx: &mut App) {
    let service = icons::service_url(urls);
    let hosts: Vec<String> = {
        let s = state(cx);
        if !s.enabled {
            return;
        }
        if s.service != service {
            s.service = service.clone();
            s.cache.clear();
        }
        let hosts: Vec<String> = hosts
            .into_iter()
            .filter(|h| !s.cache.contains_key(h))
            .collect();
        for host in &hosts {
            s.cache.insert(host.clone(), Entry::Loading);
        }
        hosts
    };
    if hosts.is_empty() {
        return;
    }
    cx.spawn(async move |cx| {
        // What the service answers for a site it doesn't know (Bitwarden sends a generic
        // globe with status 200): equal answers mean "no icon", and the type's icon stays.
        let probe = service.clone();
        let fallback =
            crate::io::run(async move { icons::fetch(&probe, NO_ICON_HOST).await }).await;
        for chunk in hosts.chunks(PARALLEL) {
            let svc = service.clone();
            let chunk = chunk.to_vec();
            let results = crate::io::run(async move {
                let mut set = tokio::task::JoinSet::new();
                for host in chunk {
                    let svc = svc.clone();
                    set.spawn(async move {
                        let bytes = icons::fetch(&svc, &host).await;
                        (host, bytes)
                    });
                }
                let mut out = Vec::new();
                while let Some(Ok(r)) = set.join_next().await {
                    out.push(r);
                }
                out
            })
            .await;
            let still_on = cx.update(|cx| {
                let s = state(cx);
                if !s.enabled || s.service != service {
                    return false;
                }
                for (host, bytes) in results {
                    let entry = bytes
                        .filter(|b| fallback.as_ref() != Some(b))
                        .and_then(to_image)
                        .map_or(Entry::Missing, Entry::Ready);
                    s.cache.insert(host, entry);
                }
                cx.refresh_windows();
                true
            });
            if !still_on {
                break;
            }
        }
    })
    .detach();
}
