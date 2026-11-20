//! Updates: is there a newer Scout Vault on the project's GitHub releases, and the
//! installer for it, checked against its published SHA-256.
//!
//! Only the project's own release URLs are accepted
//! (`https://github.com/qedrohenrique/scout-vault/releases/download/…`), and nothing
//! about the vault is sent: GitHub sees an ordinary request for the release list.
//!
//! The checksum protects against a broken or swapped download on the way; it is not a
//! signature (the files aren't signed yet), so it doesn't protect against someone who
//! controls the releases themselves.

use std::cmp::Ordering;
use std::time::Duration;

use serde_json::Value;
use sha2::{Digest, Sha256};

/// The project's repository on GitHub.
pub const REPOSITORY: &str = "qedrohenrique/scout-vault";
/// Biggest installer accepted.
pub const MAX_INSTALLER: usize = 200 * 1024 * 1024;
/// GitHub asks automated clients for a User-Agent; a generic one says nothing about the user.
const USER_AGENT: &str = "scout-vault-updater";

/// A version `X.Y.Z` (an optional `-suffix`, like `-rc.1`, sorts before the plain one).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub parts: [u64; 3],
    pub pre: Option<String>,
}

impl Version {
    /// `0.11.4`, `v0.11.4` or `1.0.0-rc.1`.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim().trim_start_matches('v');
        let (main, pre) = match text.split_once('-') {
            Some((m, p)) => (m, Some(p.to_owned())),
            None => (text, None),
        };
        let mut parts = [0u64; 3];
        let mut n = 0;
        for (ix, p) in main.split('.').enumerate() {
            if ix >= 3 {
                return None;
            }
            parts[ix] = p.parse().ok()?;
            n += 1;
        }
        (n == 3).then_some(Self { parts, pre })
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let [a, b, c] = self.parts;
        write!(f, "{a}.{b}.{c}")?;
        if let Some(pre) = &self.pre {
            write!(f, "-{pre}")?;
        }
        Ok(())
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.parts
            .cmp(&other.parts)
            .then_with(|| match (&self.pre, &other.pre) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(a), Some(b)) => a.cmp(b),
            })
    }
}

/// A file of a release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
}

/// A published release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    /// The release's page (notes, all the files).
    pub page: String,
    /// The release notes (Markdown).
    pub notes: String,
    pub installer: Option<Asset>,
    /// `<sha256> *<file name>`, next to the installer.
    pub installer_sha256: Option<Asset>,
}

fn download_prefix() -> String {
    format!("https://github.com/{REPOSITORY}/releases/download/")
}

/// Reads GitHub's release list (`GET /repos/{repo}/releases`). Drafts, releases with
/// a tag that isn't a version, and files from anywhere else are left out.
pub fn parse_releases(json: &str) -> Vec<Release> {
    let Ok(Value::Array(list)) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    let prefix = download_prefix();
    list.iter()
        .filter(|r| !r["draft"].as_bool().unwrap_or(false))
        .filter_map(|r| {
            let version = Version::parse(r["tag_name"].as_str()?)?;
            let assets: Vec<Asset> = r["assets"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|a| {
                            let url = a["browser_download_url"].as_str()?;
                            if !url.starts_with(&prefix) {
                                return None;
                            }
                            Some(Asset {
                                name: a["name"].as_str()?.to_owned(),
                                url: url.to_owned(),
                                size: a["size"].as_u64().unwrap_or(0),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            let installer_name = format!("scout-vault-{version}-setup.exe");
            let find = |name: &str| assets.iter().find(|a| a.name == name).cloned();
            Some(Release {
                page: r["html_url"].as_str().unwrap_or_default().to_owned(),
                notes: r["body"].as_str().unwrap_or_default().to_owned(),
                installer: find(&installer_name),
                installer_sha256: find(&format!("{installer_name}.sha256")),
                version,
            })
        })
        .collect()
}

/// The newest release above `current` (none if this is already the newest).
pub fn newer_than(releases: Vec<Release>, current: &Version) -> Option<Release> {
    releases
        .into_iter()
        .filter(|r| r.version > *current)
        .max_by(|a, b| a.version.cmp(&b.version))
}

/// The digest in a `.sha256` file (`<hex> *<name>` or `<hex>  <name>`), if it is for `name`.
pub fn read_checksum(text: &str, name: &str) -> Option<[u8; 32]> {
    let line = text.lines().find(|l| !l.trim().is_empty())?;
    let mut fields = line.split_whitespace();
    let hex = fields.next()?;
    if let Some(file) = fields.next() {
        if file.trim_start_matches('*') != name {
            return None;
        }
    }
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (ix, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(hex.get(ix * 2..ix * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

/// Does `bytes` match the published digest?
pub fn matches(bytes: &[u8], expected: &[u8; 32]) -> bool {
    Sha256::digest(bytes).as_slice() == expected
}

#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("network: {0}")]
    Network(String),
    #[error("the release has no installer")]
    NoInstaller,
    #[error("the downloaded file doesn't match its published checksum")]
    Checksum,
    #[error("the download is too big")]
    TooBig,
}

fn client(timeout: Duration) -> Result<reqwest::Client, UpdateError> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(timeout)
        .https_only(true)
        .build()
        .map_err(|e| UpdateError::Network(e.to_string()))
}

/// The newest release above `current`, asking GitHub.
pub async fn check(current: &Version) -> Result<Option<Release>, UpdateError> {
    let url = format!("https://api.github.com/repos/{REPOSITORY}/releases?per_page=20");
    let text = client(Duration::from_secs(20))?
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| UpdateError::Network(e.to_string()))?
        .text()
        .await
        .map_err(|e| UpdateError::Network(e.to_string()))?;
    Ok(newer_than(parse_releases(&text), current))
}

async fn fetch(url: &str, max: usize) -> Result<Vec<u8>, UpdateError> {
    let mut res = client(Duration::from_secs(600))?
        .get(url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| UpdateError::Network(e.to_string()))?;
    if res.content_length().is_some_and(|n| n as usize > max) {
        return Err(UpdateError::TooBig);
    }
    let mut out = Vec::new();
    while let Some(chunk) = res
        .chunk()
        .await
        .map_err(|e| UpdateError::Network(e.to_string()))?
    {
        if out.len() + chunk.len() > max {
            return Err(UpdateError::TooBig);
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

/// Downloads the release's installer and checks it against its `.sha256`.
pub async fn download_installer(release: &Release) -> Result<(String, Vec<u8>), UpdateError> {
    let (Some(installer), Some(sum)) = (&release.installer, &release.installer_sha256) else {
        return Err(UpdateError::NoInstaller);
    };
    let sum_text = fetch(&sum.url, 4096).await?;
    let expected = read_checksum(&String::from_utf8_lossy(&sum_text), &installer.name)
        .ok_or(UpdateError::Checksum)?;
    let bytes = fetch(&installer.url, MAX_INSTALLER).await?;
    if !matches(&bytes, &expected) {
        return Err(UpdateError::Checksum);
    }
    Ok((installer.name.clone(), bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    #[test]
    fn versions_compare_like_numbers() {
        assert!(v("0.11.4") > v("0.11.3"));
        assert!(v("0.11.10") > v("0.11.9"));
        assert!(v("1.0.0") > v("0.99.99"));
        assert!(v("v0.12.0") > v("0.11.4"));
        assert!(v("1.0.0") > v("1.0.0-rc.1"));
        assert_eq!(v("v0.9.0").to_string(), "0.9.0");
        assert!(Version::parse("0.9").is_none());
        assert!(Version::parse("1.2.3.4").is_none());
        assert!(Version::parse("latest").is_none());
    }

    const RELEASES: &str = r#"[
      {"tag_name":"v0.12.0","draft":true,"html_url":"x","assets":[]},
      {"tag_name":"v0.11.4","draft":false,"prerelease":true,
       "html_url":"https://github.com/qedrohenrique/scout-vault/releases/tag/v0.11.4","body":"notes",
       "assets":[
         {"name":"scout-vault-0.11.4-setup.exe","size":11663003,
          "browser_download_url":"https://github.com/qedrohenrique/scout-vault/releases/download/v0.11.4/scout-vault-0.11.4-setup.exe"},
         {"name":"scout-vault-0.11.4-setup.exe.sha256","size":94,
          "browser_download_url":"https://github.com/qedrohenrique/scout-vault/releases/download/v0.11.4/scout-vault-0.11.4-setup.exe.sha256"}]},
      {"tag_name":"v0.11.3","draft":false,"html_url":"y",
       "assets":[{"name":"scout-vault-0.11.3-setup.exe","size":1,
          "browser_download_url":"https://evil.example/scout-vault-0.11.3-setup.exe"}]},
      {"tag_name":"nightly","draft":false,"assets":[]}
    ]"#;

    #[test]
    fn the_newest_published_release_is_picked() {
        let list = parse_releases(RELEASES);
        // The draft and the tag that isn't a version are left out.
        assert_eq!(list.len(), 2);
        let newest = newer_than(list.clone(), &v("0.11.2")).unwrap();
        assert_eq!(newest.version, v("0.11.4"));
        assert_eq!(newest.notes, "notes");
        assert_eq!(newest.installer.as_ref().unwrap().size, 11663003);
        assert!(newest.installer_sha256.is_some());
        // Already on it (or newer): nothing.
        assert!(newer_than(list.clone(), &v("0.11.4")).is_none());
        assert!(newer_than(list, &v("0.12.0")).is_none());
    }

    #[test]
    fn files_from_elsewhere_are_ignored() {
        let list = parse_releases(RELEASES);
        let old = list.iter().find(|r| r.version == v("0.11.3")).unwrap();
        assert!(old.installer.is_none());
        assert!(parse_releases("{ not a list }").is_empty());
    }

    #[test]
    fn checksums_are_read_and_checked() {
        let data = b"installer bytes";
        let hex: String = Sha256::digest(data)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let file = format!("{hex} *scout-vault-0.11.4-setup.exe");
        let sum = read_checksum(&file, "scout-vault-0.11.4-setup.exe").unwrap();
        assert!(matches(data, &sum));
        assert!(!matches(b"something else", &sum));
        // Two spaces (sha256sum's text mode) and no name are fine too.
        assert!(read_checksum(
            &format!("{hex}  scout-vault-0.11.4-setup.exe\n"),
            "scout-vault-0.11.4-setup.exe"
        )
        .is_some());
        assert!(read_checksum(&hex, "any").is_some());
        // For another file, or not a digest: no.
        assert!(read_checksum(&file, "other.exe").is_none());
        assert!(read_checksum(
            "abc *scout-vault-0.11.4-setup.exe",
            "scout-vault-0.11.4-setup.exe"
        )
        .is_none());
    }
}
