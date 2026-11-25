//! Website icons from the server's icon service.
//!
//! Bitwarden's cloud serves them at `icons.bitwarden.net` / `icons.bitwarden.eu`;
//! Vaultwarden and self-hosted Bitwarden at `<server>/icons`. The request carries only the
//! site's domain (`GET <service>/<domain>/icon.png`), no account data.

use std::sync::OnceLock;
use std::time::Duration;

use crate::api::ServerUrls;

/// Biggest icon accepted (anything larger isn't an icon).
pub const MAX_BYTES: usize = 256 * 1024;

/// Base address of the icon service for this server.
pub fn service_url(urls: &ServerUrls) -> String {
    if urls.api == ServerUrls::bitwarden_us().api {
        "https://icons.bitwarden.net".into()
    } else if urls.api == ServerUrls::bitwarden_eu().api {
        "https://icons.bitwarden.eu".into()
    } else {
        let base = urls.vault.trim_end_matches('/');
        let base = if base.is_empty() {
            urls.api.trim_end_matches('/').trim_end_matches("/api")
        } else {
            base
        };
        format!("{base}/icons")
    }
}

/// The site whose icon a saved address shows, or `None`.
///
/// Only addresses that **say** `http://` or `https://` and parse as a real URL count.
/// The server can move encrypted fields around inside an item (the format doesn't bind
/// a value to its field), so a "URI" may really be the password: guessing a scheme for
/// text like `Verao.2024` would send it to the icon service. With the scheme required,
/// such text is never asked for.
pub fn site_host(uri: &str) -> Option<String> {
    let url = reqwest::Url::parse(uri.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    // IPs (and IPv6's `[…]`) are turned down by `is_fetchable_host`.
    let host = url.host_str()?.to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_owned();
    is_fetchable_host(&host).then_some(host)
}

/// A domain that is worth asking for: `example.com`, not `localhost`, an IP or garbage.
pub fn is_fetchable_host(host: &str) -> bool {
    host.len() <= 253
        && host.contains('.')
        && !host.starts_with('.')
        && !host.ends_with('.')
        && host.parse::<std::net::IpAddr>().is_err()
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
}

fn http() -> &'static reqwest::Client {
    static HTTP: OnceLock<reqwest::Client> = OnceLock::new();
    HTTP.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(concat!("scout-vault/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_default()
    })
}

/// Downloads the icon of `host`. `None` when the service has none (or answered garbage).
/// Needs a Tokio runtime.
pub async fn fetch(service: &str, host: &str) -> Option<Vec<u8>> {
    if !is_fetchable_host(host) {
        return None;
    }
    let url = format!("{}/{host}/icon.png", service.trim_end_matches('/'));
    let mut response = http().get(url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        bytes.extend_from_slice(&chunk);
        if bytes.len() > MAX_BYTES {
            return None;
        }
    }
    (bytes.len() >= 32).then_some(bytes)
}

/// Image format, from the first bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconFormat {
    Png,
    Jpeg,
    Gif,
    Webp,
    Ico,
    Svg,
}

pub fn sniff(bytes: &[u8]) -> Option<IconFormat> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(IconFormat::Png)
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(IconFormat::Jpeg)
    } else if bytes.starts_with(b"GIF8") {
        Some(IconFormat::Gif)
    } else if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(IconFormat::Webp)
    } else if bytes.starts_with(&[0, 0, 1, 0]) {
        Some(IconFormat::Ico)
    } else {
        let head = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).to_ascii_lowercase();
        head.contains("<svg").then_some(IconFormat::Svg)
    }
}

/// Width of a PNG (tiny placeholders, like a 1×1 "no icon", are discarded).
pub fn png_width(bytes: &[u8]) -> Option<u32> {
    (sniff(bytes) == Some(IconFormat::Png) && bytes.len() >= 24)
        .then(|| u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]))
}

/// The bytes are a usable icon.
pub fn is_usable(bytes: &[u8]) -> bool {
    match sniff(bytes) {
        Some(IconFormat::Png) => png_width(bytes).is_some_and(|w| w >= 8),
        Some(_) => true,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_addresses_with_a_scheme_get_an_icon() {
        assert_eq!(
            site_host("https://www.GitHub.com/login").as_deref(),
            Some("github.com")
        );
        assert_eq!(
            site_host(" http://banco.example.com:8443/x ").as_deref(),
            Some("banco.example.com")
        );
        // A password moved into the URI field by a malicious server: never asked for.
        for text in [
            "Verao.2024",
            "minha.senha.secreta",
            "github.com",
            "user@banco.example.com",
            "androidapp://com.example",
            "ftp://files.example.com",
            "https://192.168.0.1/",
            "https://[::1]/",
            "https://localhost/",
        ] {
            assert_eq!(site_host(text), None, "{text}");
        }
    }

    #[test]
    fn service_follows_the_server() {
        assert_eq!(
            service_url(&ServerUrls::bitwarden_us()),
            "https://icons.bitwarden.net"
        );
        assert_eq!(
            service_url(&ServerUrls::bitwarden_eu()),
            "https://icons.bitwarden.eu"
        );
        assert_eq!(
            service_url(&ServerUrls::self_hosted("http://localhost:8000/")),
            "http://localhost:8000/icons"
        );
    }

    #[test]
    fn only_real_domains_are_asked_for() {
        assert!(is_fetchable_host("github.com"));
        assert!(is_fetchable_host("mail.google.com"));
        assert!(!is_fetchable_host("localhost"));
        assert!(!is_fetchable_host("192.168.0.1"));
        assert!(!is_fetchable_host("::1"));
        assert!(!is_fetchable_host("evil.com/../x"));
        assert!(!is_fetchable_host(".com"));
    }

    #[test]
    fn formats_are_recognized() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&16u32.to_be_bytes());
        png.extend_from_slice(&16u32.to_be_bytes());
        assert_eq!(sniff(&png), Some(IconFormat::Png));
        assert_eq!(png_width(&png), Some(16));
        assert!(is_usable(&png));

        let mut tiny = png.clone();
        tiny[16..20].copy_from_slice(&1u32.to_be_bytes());
        assert!(!is_usable(&tiny), "1×1 placeholder");

        assert_eq!(sniff(&[0, 0, 1, 0, 1, 0]), Some(IconFormat::Ico));
        assert_eq!(
            sniff(b"<?xml version=\"1.0\"?><svg xmlns=\"\"></svg>"),
            Some(IconFormat::Svg)
        );
        assert_eq!(sniff(b"<html>not found</html>"), None);
    }
}
