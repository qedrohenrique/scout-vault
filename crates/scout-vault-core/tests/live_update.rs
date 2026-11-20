//! Against the real GitHub releases (network; `cargo test --test live_update -- --ignored`).

use scout_vault_core::update::{self, Version};

#[tokio::test]
#[ignore]
async fn finds_downloads_and_checks_the_newest_installer() {
    let old = Version::parse("0.0.1").unwrap();
    let release = update::check(&old)
        .await
        .expect("GitHub answers")
        .expect("there is a newer release");
    assert!(release.version > old);
    assert!(release.page.contains("/releases/tag/v"));
    let (name, bytes) = update::download_installer(&release)
        .await
        .expect("the installer downloads and matches its SHA-256");
    assert_eq!(name, format!("scout-vault-{}-setup.exe", release.version));
    assert!(bytes.starts_with(b"MZ"), "a Windows executable");
    // Already on the newest: nothing to do.
    assert!(update::check(&release.version).await.unwrap().is_none());
}
