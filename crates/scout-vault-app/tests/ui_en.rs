//! English interface. Lives in a separate binary because the language is global to
//! the process: it changes here without disturbing the `ui.rs` tests (in pt-BR).

use std::sync::Arc;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{px, size, AppContext, Bounds, TestAppContext, WindowBounds, WindowOptions};
use scout_vault_app::app::ScoutVaultApp;
use scout_vault_app::flow::{Session, Unlocked};
use scout_vault_app::i18n::{self, Language};
use scout_vault_app::settings::SettingsStore;
use scout_vault_core::api::{Client, DeviceInfo, ServerUrls};
use scout_vault_core::crypto::MasterKey;
use scout_vault_core::sync::SyncResponse;
use scout_vault_core::vault::{Keys, Vault};
use scout_vault_core::Kdf;

const SYNC: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../scout-vault-core/tests/fixtures/sync.json"
));
const VECTORS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../scout-vault-core/tests/fixtures/vectors.json"
));

fn unlocked_fixture() -> Unlocked {
    let v: serde_json::Value = serde_json::from_str(VECTORS).unwrap();
    let (password, email) = (
        v["password"].as_str().unwrap().to_owned(),
        v["email"].as_str().unwrap().to_owned(),
    );
    let kdf = Kdf::Pbkdf2Sha256 { iterations: 5000 };
    let sync = Arc::new(SyncResponse::from_json(SYNC).unwrap());
    let mk = MasterKey::derive(&password, &email, kdf).unwrap();
    let keys = Keys::unlock(&mk, &sync.profile).unwrap();
    let vault = Vault::decrypt(&sync, &keys);
    let client = Client::new(
        ServerUrls::self_hosted("http://127.0.0.1:9"),
        DeviceInfo::new("teste"),
    )
    .unwrap();
    Unlocked {
        session: Session {
            client: Arc::new(client),
            email,
            kdf,
            access_token: "token".into(),
            refresh_token: "refresh".into(),
            sync,
            logged_in_at: scout_vault_app::flow::now_secs(),
        },
        keys,
        vault,
    }
}

fn options() -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            Default::default(),
            size(px(1100.), px(720.)),
        ))),
        ..Default::default()
    }
}

/// A single test: the steps depend on the global language, which can't change in
/// the middle of another test running in parallel.
#[gpui_kit::test]
fn switching_to_english_redraws_and_new_screens_follow(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);

    // Starts in Portuguese; switching in Settings changes the screen right away.
    i18n::set(Language::PtBr);
    let unlocked = unlocked_fixture();
    let (handle, app) = cx.update(|cx| {
        gpui_kit::open_window(options(), cx, |window, cx| {
            cx.new(|cx| {
                ScoutVaultApp::with_unlocked(SettingsStore::in_memory(), unlocked, window, cx)
            })
        })
        .expect("abrir janela de teste")
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("vault-sync").label(), Some("Sincronizar"));

        app.update(cx, |a, cx| a.open_settings(window, cx));
        window.render_frame(cx);
        assert_eq!(window.find("language-system").label(), Some("Sistema"));
        window.click("language-en", cx);
        window.render_frame(cx);
        assert_eq!(window.find("language-system").label(), Some("System"));

        app.update(cx, |a, cx| a.close_settings(cx));
        window.render_frame(cx);
        assert_eq!(window.find("vault-sync").label(), Some("Sync"));
    })
    .unwrap();
    assert_eq!(i18n::current(), Language::En);

    // A new screen is born in English.
    let (login, _) = cx.update(|cx| {
        gpui_kit::open_window(options(), cx, |window, cx| {
            cx.new(|cx| ScoutVaultApp::new(SettingsStore::in_memory(), window, cx))
        })
        .expect("abrir janela de teste")
    });
    cx.update_window(login, |_, window, cx| {
        window.render_frame(cx);
        window.click("login-submit", cx);
        assert_eq!(
            window.find("login-status").label(),
            Some("Enter your account email.")
        );
        window.click("login-email", cx);
        window.input("person@example.com", cx);
        window.click("login-submit", cx);
        assert_eq!(
            window.find("login-status").label(),
            Some("Enter your vault password.")
        );
    })
    .unwrap();
}
