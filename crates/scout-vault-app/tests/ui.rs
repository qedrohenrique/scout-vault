//! UI integration tests: real screens in a headless window (nothing shows up
//! on screen), with clicks and typing dispatched through the same path as the app.

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::component::ActiveTheme;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    px, size, AnyWindowHandle, AppContext, Bounds, Entity, TestAppContext, WindowBounds,
    WindowOptions,
};
use scout_vault_app::app::{ScoutVaultApp, ScreenKind};
use scout_vault_app::appearance::{Accent, Appearance, ThemeChoice};
use scout_vault_app::clipboard::DEFAULT_CLEAR_SECS;
use scout_vault_app::flow::{Session, Unlocked};
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

fn fixture_password_and_email() -> (String, String) {
    let v: serde_json::Value = serde_json::from_str(VECTORS).unwrap();
    (
        v["password"].as_str().unwrap().to_owned(),
        v["email"].as_str().unwrap().to_owned(),
    )
}

/// Synthetic vault from `scout-vault-core` (generated in Python), already open.
fn unlocked_fixture() -> Unlocked {
    let (password, email) = fixture_password_and_email();
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

fn open_app(
    cx: &mut TestAppContext,
    unlocked: Option<Unlocked>,
) -> (AnyWindowHandle, Entity<ScoutVaultApp>) {
    cx.update(gpui_kit::init);
    // The texts checked here are the Portuguese ones (English lives in `ui_en.rs`).
    scout_vault_app::i18n::set(scout_vault_app::i18n::Language::PtBr);
    cx.update(|cx| {
        gpui_kit::open_window(options(), cx, |window, cx| {
            cx.new(|cx| match unlocked {
                Some(u) => ScoutVaultApp::with_unlocked(SettingsStore::in_memory(), u, window, cx),
                None => ScoutVaultApp::new(SettingsStore::in_memory(), window, cx),
            })
        })
        .expect("abrir janela de teste")
    })
}

/// Waits for work that runs outside the GPUI executor (KDF on Tokio).
fn wait_until(cx: &mut TestAppContext, mut done: impl FnMut(&mut TestAppContext) -> bool) {
    for _ in 0..500 {
        cx.run_until_parked();
        if done(cx) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("a condição não foi atingida a tempo");
}

#[gpui_kit::test]
fn login_validates_fields_before_touching_the_network(cx: &mut TestAppContext) {
    let (handle, app) = open_app(cx, None);

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("login-status").label(), Some(""));

        window.click("login-submit", cx);
        assert_eq!(
            window.find("login-status").label(),
            Some("Digite o e-mail da conta.")
        );

        window.click("login-email", cx);
        window.input("pessoa@exemplo.com", cx);
        window.click("login-submit", cx);
        assert_eq!(
            window.find("login-status").label(),
            Some("Digite a senha do cofre.")
        );
    })
    .unwrap();

    cx.update(|cx| assert_eq!(app.read(cx).screen_kind(), ScreenKind::Login));
}

#[gpui_kit::test]
fn sso_option_swaps_email_for_organization(cx: &mut TestAppContext) {
    let (handle, _) = open_app(cx, None);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("login-email").is_some());
        assert!(window.try_find("login-sso-identifier").is_none());

        window.click("login-use-sso", cx);
        window.render_frame(cx);
        assert!(window.try_find("login-email").is_none());
        assert!(window.try_find("login-sso-identifier").is_some());

        // No identifier and no password: validates before opening the browser.
        window.click("login-submit", cx);
        assert_eq!(
            window.find("login-status").label(),
            Some("Digite o identificador SSO da sua empresa.")
        );
        window.click("login-sso-identifier", cx);
        window.input("minha-empresa", cx);
        window.click("login-submit", cx);
        assert_eq!(
            window.find("login-status").label(),
            Some("Digite a senha do cofre: ela abre o cofre depois do SSO.")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn self_hosted_needs_a_valid_url(cx: &mut TestAppContext) {
    let (handle, _) = open_app(cx, None);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // The URL field only shows up after choosing "Self-hosted server".
        assert!(window.try_find("login-server-url").is_none());
        window.within("login-server").click(2usize, cx);
        assert!(window.try_find("login-server-url").is_some());

        window.click("login-email", cx);
        window.input("pessoa@exemplo.com", cx);
        window.click("login-password", cx);
        window.input("uma senha", cx);
        window.click("login-submit", cx);
        assert_eq!(
            window.find("login-status").label(),
            Some("Informe o endereço do servidor, começando com https://")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn browse_reveal_copy_and_lock(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (handle, app) = open_app(cx, Some(unlocked_fixture()));
    let vault_view = cx.update(|cx| app.read(cx).vault_view().cloned().unwrap());
    let visible = |cx: &mut TestAppContext| -> Vec<String> {
        cx.update(|cx| {
            vault_view
                .read(cx)
                .list()
                .read(cx)
                .delegate()
                .visible_items()
                .map(|i| i.id.clone())
                .collect()
        })
    };

    // Sidebar filters.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("filter-card", cx);
    })
    .unwrap();
    assert_eq!(visible(cx), ["card-org"]);

    cx.update_window(handle, |_, window, cx| window.click("filter-trash", cx))
        .unwrap();
    assert_eq!(visible(cx), ["note-deleted"]);

    cx.update_window(handle, |_, window, cx| window.click("filter-all", cx))
        .unwrap();
    assert!(!visible(cx).contains(&"note-deleted".to_owned()));

    // Opens the login and checks that the password starts hidden.
    cx.update_window(handle, |_, window, cx| {
        vault_view.update(cx, |v, cx| v.select_item("login-1", window, cx));
        window.render_frame(cx);
        assert_eq!(window.find("detail-name").label(), Some("Meu Banco"));
        assert_eq!(window.find("value-username").label(), Some("pessoa"));
        assert_eq!(window.find("value-password").label(), Some("••••••••"));

        window.click("reveal-password", cx);
        assert_eq!(window.find("value-password").label(), Some("s3nh@-ção"));
        window.click("reveal-password", cx);
        assert_eq!(window.find("value-password").label(), Some("••••••••"));

        // Copying doesn't require revealing.
        window.click("copy-password", cx);
    })
    .unwrap();

    let clipboard =
        |cx: &mut TestAppContext| cx.update(|cx| cx.read_from_clipboard().and_then(|c| c.text()));
    assert_eq!(clipboard(cx).as_deref(), Some("s3nh@-ção"));

    // Before the timeout it is still there; afterwards it is gone.
    cx.executor()
        .advance_clock(Duration::from_secs(u64::from(DEFAULT_CLEAR_SECS) - 1));
    cx.run_until_parked();
    assert_eq!(clipboard(cx).as_deref(), Some("s3nh@-ção"));
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    assert!(clipboard(cx).unwrap_or_default().is_empty());

    // Locking discards the decrypted vault and asks for the password.
    cx.update_window(handle, |_, window, cx| window.click("vault-lock", cx))
        .unwrap();
    cx.update(|cx| {
        assert_eq!(app.read(cx).screen_kind(), ScreenKind::Locked);
        assert!(app.read(cx).vault_view().is_none());
    });

    // Wrong password.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("unlock-password", cx);
        window.input("senha errada", cx);
        window.click("unlock-submit", cx);
    })
    .unwrap();
    wait_until(cx, |cx| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.find("unlock-status").label() == Some("Senha do cofre incorreta.")
        })
        .unwrap()
    });

    // The right password reopens the vault, without network.
    let (password, _) = fixture_password_and_email();
    cx.update_window(handle, |_, window, cx| {
        window.click("unlock-password", cx);
        window.input(&password, cx);
        window.click("unlock-submit", cx);
    })
    .unwrap();
    wait_until(cx, |cx| {
        cx.update(|cx| app.read(cx).screen_kind() == ScreenKind::Vault)
    });
}

#[gpui_kit::test]
fn locks_after_inactivity(cx: &mut TestAppContext) {
    let (handle, app) = open_app(cx, Some(unlocked_fixture()));
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |app, cx| {
            app.set_auto_lock_after(Duration::from_secs(60));
            // Still within the timeout: stays open.
            app.lock_if_idle(Instant::now(), window, cx);
            assert_eq!(app.screen_kind(), ScreenKind::Vault);
            // Past the timeout: locks.
            app.lock_if_idle(Instant::now() + Duration::from_secs(61), window, cx);
            assert_eq!(app.screen_kind(), ScreenKind::Locked);
        });
    })
    .unwrap();
}

#[gpui_kit::test]
fn search_filters_the_list(cx: &mut TestAppContext) {
    let (handle, app) = open_app(cx, Some(unlocked_fixture()));
    let vault_view = cx.update(|cx| app.read(cx).vault_view().cloned().unwrap());
    let visible = |cx: &mut TestAppContext| -> Vec<String> {
        cx.update(|cx| {
            vault_view
                .read(cx)
                .list()
                .read(cx)
                .delegate()
                .visible_items()
                .map(|i| i.id.clone())
                .collect()
        })
    };

    // `set_query` writes into the list's search field, which triggers the real search.
    for (query, expected) in [
        ("banco.example", vec!["login-1"]),
        ("ana souza", vec!["card-org"]),
    ] {
        cx.update_window(handle, |_, window, cx| {
            vault_view.update(cx, |v, cx| {
                v.list()
                    .update(cx, |list, cx| list.set_query(query, window, cx));
            });
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(visible(cx), expected, "busca: {query}");
    }
}

/// Full login through the UI against a real server (local Vaultwarden).
/// Run with the variables from `dev/test-account.env`:
/// `cargo test -p scout-vault-app --test ui -- --ignored`
#[gpui_kit::test]
#[ignore = "precisa de servidor; veja dev/test-account.env"]
fn logs_in_through_the_ui_against_a_real_server(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let env = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("defina {k}"));
    let (url, email, password) = (
        env("SCOUT_VAULT_TEST_URL"),
        env("SCOUT_VAULT_TEST_EMAIL"),
        env("SCOUT_VAULT_TEST_PASSWORD"),
    );
    let (handle, app) = open_app(cx, None);

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.within("login-server").click(2usize, cx);
        window.click("login-server-url", cx);
        window.input(&url, cx);
        window.click("login-email", cx);
        window.input(&email, cx);
        window.click("login-password", cx);
        window.input(&password, cx);
        window.click("login-submit", cx);
    })
    .unwrap();

    wait_until(cx, |cx| {
        let status = cx
            .update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window
                    .try_find("login-status")
                    .and_then(|s| s.label().map(str::to_owned))
            })
            .unwrap();
        let screen = cx.update(|cx| app.read(cx).screen_kind());
        assert!(
            screen == ScreenKind::Vault || !status.as_deref().unwrap_or("").contains("incorret"),
            "login falhou: {status:?}"
        );
        screen == ScreenKind::Vault
    });

    let names: Vec<String> = cx.update(|cx| {
        let view = app.read(cx).vault_view().unwrap().read(cx);
        view.list()
            .read(cx)
            .delegate()
            .visible_items()
            .map(|i| i.name.clone())
            .collect()
    });
    assert!(!names.is_empty(), "o cofre de teste deveria ter itens");
}

/// The theme color matches the swatch (tolerance for HSL rounding).
fn same_color(a: gpui_kit::Hsla, b: gpui_kit::Hsla) -> bool {
    let (a, b) = (a.to_rgb(), b.to_rgb());
    (a.r - b.r).abs() < 0.01 && (a.g - b.g).abs() < 0.01 && (a.b - b.b).abs() < 0.01
}

#[gpui_kit::test]
fn settings_change_theme_and_accent_live(cx: &mut TestAppContext) {
    use scout_vault_app::theme_fx;
    let (handle, app) = open_app(cx, Some(unlocked_fixture()));
    let settle = |cx: &mut TestAppContext| {
        cx.executor().advance_clock(theme_fx::FLIP * 2);
        cx.run_until_parked();
    };
    let mut light_bg = gpui_kit::Hsla::default();

    // Starts with the default color (jade) applied.
    cx.update(|cx| {
        let dark = cx.theme().is_dark();
        assert!(same_color(cx.theme().primary, Accent::Jade.swatch(dark)));
    });

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("vault-settings", cx);
    })
    .unwrap();
    // The request goes out as an event, handled after the click.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("settings-close").is_some());

        light_bg = cx.theme().background;
        // Light → dark: the interface doesn't disappear — the mode is already the new one, but the
        // colors start from where they were…
        window.click("theme-dark", cx);
        assert!(cx.theme().is_dark());
        assert!(same_color(cx.theme().background, light_bg));
        assert!(theme_fx::running(cx));
    })
    .unwrap();
    // …in the middle, each color is a blend of both…
    cx.executor().advance_clock(theme_fx::FLIP / 2);
    cx.run_until_parked();
    let mid = cx.update(|cx| cx.theme().background);
    assert!(!same_color(mid, light_bg));
    assert!(cx.update(|cx| theme_fx::running(cx)));
    // …and at the end it reaches exactly the dark theme.
    settle(cx);
    cx.update(|cx| {
        assert!(cx.theme().background.l < mid.l);
        assert!(!theme_fx::running(cx));
    });

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // The accent color transforms too (doesn't jump).
        window.click("accent-coral", cx);
        assert!(theme_fx::running(cx));
        assert!(!same_color(cx.theme().primary, Accent::Coral.swatch(true)));
    })
    .unwrap();
    settle(cx);
    cx.update(|cx| assert!(same_color(cx.theme().primary, Accent::Coral.swatch(true))));

    // Another switch in the middle of a transition continues from there to the new target.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("theme-light", cx);
    })
    .unwrap();
    cx.executor().advance_clock(theme_fx::FLIP / 3);
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("text-large", cx);
        window.click("corners-round", cx);
    })
    .unwrap();
    settle(cx);
    cx.update(|cx| {
        assert!(!cx.theme().is_dark());
        // The chosen color survives the mode switch.
        assert!(same_color(cx.theme().primary, Accent::Coral.swatch(false)));
        assert_eq!(cx.theme().font_size, gpui_kit::px(17.));
        assert_eq!(cx.theme().radius, gpui_kit::px(10.));
    });

    cx.update(|cx| {
        let view = app.read(cx).settings_view().unwrap();
        assert_eq!(
            view.read(cx).appearance(),
            Appearance {
                theme: ThemeChoice::Light,
                accent: Accent::Coral,
                text_size: scout_vault_app::appearance::TextSize::Large,
                motion: scout_vault_app::appearance::Motion::On,
                corners: scout_vault_app::appearance::Corners::Round,
                custom_accent: None,
            }
        );
    });

    // Any color can be the accent; it stays readable, and a built-in one undoes it.
    let pink = scout_vault_app::colors::Rgb::parse("#ff00aa");
    cx.update_window(handle, |_, window, cx| {
        let view = app.read(cx).settings_view().cloned().unwrap();
        view.update(cx, |v, cx| v.set_custom_accent(pink, window, cx));
    })
    .unwrap();
    settle(cx);
    cx.update(|cx| {
        let view = app.read(cx).settings_view().unwrap();
        assert_eq!(view.read(cx).appearance().custom_accent, pink);
        assert!(!same_color(cx.theme().primary, Accent::Coral.swatch(false)));
        let white = gpui_kit::hsla(0., 0., 1., 1.);
        assert!(scout_vault_app::colors::contrast(cx.theme().primary, white) >= 4.4);
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("accent-custom").is_some());
        window.click("accent-coral", cx);
    })
    .unwrap();
    settle(cx);
    cx.update(|cx| {
        let view = app.read(cx).settings_view().unwrap();
        assert_eq!(view.read(cx).appearance().custom_accent, None);
        assert!(same_color(cx.theme().primary, Accent::Coral.swatch(false)));
    });

    // Closing goes back to the vault, which stayed open underneath.
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-close", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("settings-close").is_none());
        assert!(window.try_find("vault-lock").is_some());
    })
    .unwrap();
    cx.update(|cx| assert_eq!(app.read(cx).screen_kind(), ScreenKind::Vault));
}

#[gpui_kit::test]
fn appearance_is_saved_and_restored(cx: &mut TestAppContext) {
    let dir =
        std::env::temp_dir().join(format!("scout-vault-ui-appearance-{}", std::process::id()));
    let store = SettingsStore::at(dir.join("settings.json"));
    let mut settings = store.load();
    settings.appearance.theme = ThemeChoice::Dark;
    settings.appearance.accent = Accent::Indigo;
    store.save(&settings);

    cx.update(gpui_kit::init);
    cx.update(|cx| {
        gpui_kit::open_window(options(), cx, |window, cx| {
            cx.new(|cx| ScoutVaultApp::new(store.clone(), window, cx))
        })
        .unwrap();
        assert!(cx.theme().is_dark());
        assert!(same_color(cx.theme().primary, Accent::Indigo.swatch(true)));
    });
    let _ = std::fs::remove_dir_all(dir);
}

#[gpui_kit::test]
fn shows_live_totp_code_and_copies_it_without_spaces(cx: &mut TestAppContext) {
    let (handle, app) = open_app(cx, Some(unlocked_fixture()));
    let vault_view = cx.update(|cx| app.read(cx).vault_view().cloned().unwrap());
    let totp = scout_vault_core::totp::Totp::parse("otpauth://totp/Banco?secret=JBSWY3DPEHPK3PXP")
        .unwrap();

    cx.update_window(handle, |_, window, cx| {
        vault_view.update(cx, |v, cx| v.select_item("login-1", window, cx));
        window.render_frame(cx);
        let shown = window.find("value-totp-code").label().unwrap().to_owned();
        let now = totp.now();
        assert_eq!(shown.replace(' ', ""), now.code);
        assert_eq!(shown.len(), 7, "código em dois grupos: {shown}");
        assert!(window
            .find("hint-totp-code")
            .label()
            .unwrap()
            .ends_with(" s"));
        // The key stays hidden.
        assert_eq!(window.find("value-totp").label(), Some("••••••••"));
        window.click("copy-totp-code", cx);
    })
    .unwrap();
    let copied = cx.update(|cx| cx.read_from_clipboard().and_then(|c| c.text()));
    assert_eq!(copied, Some(totp.now().code));
}

#[gpui_kit::test]
fn keyboard_shortcuts_copy_search_and_lock(cx: &mut TestAppContext) {
    let (handle, app) = open_app(cx, Some(unlocked_fixture()));
    let vault_view = cx.update(|cx| app.read(cx).vault_view().cloned().unwrap());
    let clipboard =
        |cx: &mut TestAppContext| cx.update(|cx| cx.read_from_clipboard().and_then(|c| c.text()));

    cx.update_window(handle, |_, window, cx| {
        vault_view.update(cx, |v, cx| v.select_item("login-1", window, cx));
        window.render_frame(cx);
        window.press("ctrl-p", cx);
    })
    .unwrap();
    assert_eq!(clipboard(cx).as_deref(), Some("s3nh@-ção"));

    cx.update_window(handle, |_, window, cx| window.press("ctrl-u", cx))
        .unwrap();
    assert_eq!(clipboard(cx).as_deref(), Some("pessoa"));

    // Ctrl+F moves focus to the search: the typed text filters the list.
    cx.update_window(handle, |_, window, cx| {
        window.press("ctrl-f", cx);
        window.input("banco", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let visible: Vec<String> = cx.update(|cx| {
        vault_view
            .read(cx)
            .list()
            .read(cx)
            .delegate()
            .visible_items()
            .map(|i| i.id.clone())
            .collect()
    });
    assert_eq!(visible, ["login-1"]);

    cx.update_window(handle, |_, window, cx| window.press("ctrl-l", cx))
        .unwrap();
    cx.update(|cx| assert_eq!(app.read(cx).screen_kind(), ScreenKind::Locked));
}

#[gpui_kit::test]
fn website_icons_show_in_the_list_and_can_be_turned_off(cx: &mut TestAppContext) {
    let dir =
        std::env::temp_dir().join(format!("scout-vault-ui-site-icons-{}", std::process::id()));
    let store = SettingsStore::at(dir.join("settings.json"));
    cx.update(gpui_kit::init);
    let store2 = store.clone();
    let (handle, app) = cx.update(|cx| {
        gpui_kit::open_window(options(), cx, |window, cx| {
            cx.new(|cx| ScoutVaultApp::with_unlocked(store2, unlocked_fixture(), window, cx))
        })
        .unwrap()
    });
    // Logins with a known brand keep the brand; the others get the site's icon.
    let item = cx.update(|cx| {
        let view = app.read(cx).vault_view().cloned().expect("vault aberto");
        let list = view.read(cx).list().clone();
        let vault = list.read(cx).delegate().vault();
        let hosts = scout_vault_app::site_icons::hosts_of(vault);
        vault
            .items
            .iter()
            .find(|i| {
                scout_vault_app::site_icons::host_of_item(i).is_some_and(|h| hosts.contains(&h))
            })
            .expect("um login com endereço e sem marca conhecida")
            .clone()
    });
    let host = cx.update(|cx| scout_vault_app::item_icons::site_host(&item, cx).unwrap());

    let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    png.extend_from_slice(&16u32.to_be_bytes());
    png.extend_from_slice(&16u32.to_be_bytes());
    png.extend_from_slice(&[8, 6, 0, 0, 0]);
    cx.update(|cx| scout_vault_app::site_icons::insert(&host, png.clone(), cx));
    cx.update(|cx| assert!(scout_vault_app::site_icons::image(&host, cx).is_some()));
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();

    // Garbage isn't an icon.
    cx.update(|cx| {
        scout_vault_app::site_icons::insert("broken.example", b"<html>404</html>".to_vec(), cx);
        assert!(scout_vault_app::site_icons::image("broken.example", cx).is_none());
    });

    // Turning the option off forgets the icons and saves the choice.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("vault-settings", cx);
    })
    .unwrap();
    let settings = cx.update(|cx| app.read(cx).settings_view().cloned().unwrap());
    cx.update(|cx| settings.update(cx, |s, cx| s.set_site_icons(false, cx)));
    cx.update(|cx| {
        assert!(!scout_vault_app::site_icons::enabled(cx));
        assert!(scout_vault_app::site_icons::image(&host, cx).is_none());
    });
    assert!(!store.load().site_icons);
    let _ = std::fs::remove_dir_all(dir);
}

#[gpui_kit::test]
fn generator_makes_passphrases_and_usernames(cx: &mut TestAppContext) {
    use scout_vault_core::generator::{GeneratorKind, UsernameKind};
    let dir = std::env::temp_dir().join(format!("scout-vault-ui-phrase-{}", std::process::id()));
    let store = SettingsStore::at(dir.join("settings.json"));
    cx.update(gpui_kit::init);
    let store2 = store.clone();
    let (handle, app) = cx.update(|cx| {
        gpui_kit::open_window(options(), cx, |window, cx| {
            cx.new(|cx| ScoutVaultApp::with_unlocked(store2, unlocked_fixture(), window, cx))
        })
        .unwrap()
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("vault-generator", cx);
    })
    .unwrap();
    let generator = cx.update(|cx| app.read(cx).generator_view().cloned().unwrap());

    // Passphrase: 4 words separated by dots.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("gen-kind-passphrase", cx);
    })
    .unwrap();
    cx.update(|cx| generator.update(cx, |g, cx| g.edit_phrase(|o| o.words = 4, cx)));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("sep-1", cx);
    })
    .unwrap();
    let phrase = cx.update(|cx| generator.read(cx).password().to_owned());
    assert_eq!(phrase.split('.').count(), 4, "{phrase}");
    assert!(phrase
        .chars()
        .all(|c| c.is_ascii_lowercase() || c == '.' || c == '-'));
    let saved = store.load();
    assert_eq!(saved.generator_kind, GeneratorKind::Passphrase);
    assert_eq!(saved.passphrase.words, 4);
    assert_eq!(saved.passphrase.separator, ".");

    // Username: a word with four digits, then the account email with a tag.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("gen-kind-username", cx);
    })
    .unwrap();
    let user = cx.update(|cx| generator.read(cx).password().to_owned());
    assert!(user.ends_with(|c: char| c.is_ascii_digit()), "{user}");
    assert!(!user.contains('@'));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("user-kind-plus", cx);
    })
    .unwrap();
    let user = cx.update(|cx| generator.read(cx).password().to_owned());
    assert!(user.contains('+') && user.contains('@'), "{user}");
    assert_eq!(
        store.load().username_generator.kind,
        UsernameKind::PlusEmail
    );

    // Reopening remembers the kind.
    cx.update_window(handle, |_, window, cx| window.press("escape", cx))
        .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("vault-generator", cx);
    })
    .unwrap();
    cx.update(|cx| {
        let g = app.read(cx).generator_view().cloned().unwrap();
        assert_eq!(g.read(cx).kind(), GeneratorKind::Username);
    });
    let _ = std::fs::remove_dir_all(dir);
}

#[gpui_kit::test]
fn password_generator_follows_options_and_saves_them(cx: &mut TestAppContext) {
    let dir = std::env::temp_dir().join(format!("scout-vault-ui-generator-{}", std::process::id()));
    let store = SettingsStore::at(dir.join("settings.json"));
    cx.update(gpui_kit::init);
    let store2 = store.clone();
    let (handle, app) = cx.update(|cx| {
        gpui_kit::open_window(options(), cx, |window, cx| {
            cx.new(|cx| ScoutVaultApp::with_unlocked(store2, unlocked_fixture(), window, cx))
        })
        .unwrap()
    });

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("vault-generator", cx);
    })
    .unwrap();
    let generator = cx.update(|cx| app.read(cx).generator_view().cloned().unwrap());
    let first = cx.update(|cx| generator.read(cx).password().to_owned());
    assert_eq!(first.chars().count(), 20);

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("generator-output").label(),
            Some(first.as_str())
        );
    })
    .unwrap();
    cx.update(|cx| {
        generator.update(cx, |g, cx| {
            let mut o = g.options();
            o.length = 32;
            g.set_options(o, cx)
        })
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("gen-symbols", cx);
    })
    .unwrap();
    let (pw, opts) = cx.update(|cx| {
        let g = generator.read(cx);
        (g.password().to_owned(), g.options())
    });
    assert_eq!(pw.chars().count(), 32);
    assert!(!opts.symbols);
    assert!(pw.chars().all(|c| c.is_ascii_alphanumeric()), "{pw}");
    assert_eq!(store.load().generator, opts, "opções salvas");

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("generator-copy", cx);
    })
    .unwrap();
    assert_eq!(
        cx.update(|cx| cx.read_from_clipboard().and_then(|c| c.text())),
        Some(pw)
    );

    // Esc closes and goes back to the vault.
    cx.update_window(handle, |_, window, cx| window.press("escape", cx))
        .unwrap();
    cx.update(|cx| {
        assert!(app.read(cx).generator_view().is_none());
        assert_eq!(app.read(cx).screen_kind(), ScreenKind::Vault);
    });
    let _ = std::fs::remove_dir_all(dir);
}

/// Minimal HTTP server that answers 400 to everything (refresh token refused).
fn server_that_rejects_tokens() -> String {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let body = r#"{"error":"invalid_grant"}"#;
            let _ = write!(
                stream,
                "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    url
}

#[gpui_kit::test]
fn expired_session_returns_to_login(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let mut unlocked = unlocked_fixture();
    unlocked.session.client = Arc::new(
        Client::new(
            ServerUrls::self_hosted(&server_that_rejects_tokens()),
            DeviceInfo::new("teste"),
        )
        .unwrap(),
    );
    let (handle, app) = open_app(cx, Some(unlocked));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("vault-sync", cx);
    })
    .unwrap();
    wait_until(cx, |cx| {
        cx.update(|cx| app.read(cx).screen_kind() == ScreenKind::Login)
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("login-status").label(),
            Some("Sua sessão expirou. Entre de novo.")
        );
    })
    .unwrap();
}

type Typed = Arc<std::sync::Mutex<Vec<(isize, Vec<String>)>>>;

/// Fake typist: records what would be typed instead of touching the keyboard.
fn fake_typer(app: &Entity<ScoutVaultApp>, cx: &mut TestAppContext) -> Typed {
    use scout_vault_app::autotype::Key;
    let typed: Typed = Default::default();
    let sink = typed.clone();
    cx.update(|cx| {
        app.update(cx, |app, _| {
            app.set_typer(Arc::new(move |window, keys, _check| {
                let keys = keys
                    .iter()
                    .map(|k| match k {
                        Key::Text(t) | Key::Password(t) => t.to_string(),
                        Key::Tab => "⇥".into(),
                        Key::Enter => "↵".into(),
                    })
                    .collect();
                sink.lock().unwrap().push((window, keys));
                Ok(())
            }))
        })
    });
    typed
}

fn request(title: &str) -> scout_vault_app::autotype::Request {
    scout_vault_app::autotype::Request {
        window: 42,
        title: title.into(),
        own_window: false,
    }
}

#[gpui_kit::test]
fn autotype_types_the_only_matching_login(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (handle, app) = open_app(cx, Some(unlocked_fixture()));
    let typed = fake_typer(&app, cx);
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |app, cx| {
            app.handle_autotype(request("Entrar — banco.example.com — Firefox"), window, cx)
        })
    })
    .unwrap();
    wait_until(cx, |_| !typed.lock().unwrap().is_empty());
    assert_eq!(
        typed.lock().unwrap()[0],
        (
            42,
            vec!["pessoa".into(), "⇥".into(), "s3nh@-ção".into(), "↵".into()]
        )
    );
    cx.update(|cx| assert!(app.read(cx).quick_search().is_none()));
}

#[gpui_kit::test]
fn autotype_asks_when_the_title_does_not_decide(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (handle, app) = open_app(cx, Some(unlocked_fixture()));
    let typed = fake_typer(&app, cx);
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |app, cx| {
            app.handle_autotype(request("Sem título - Bloco de Notas"), window, cx)
        });
    })
    .unwrap();
    // A HUD apart from the main window: only typeable logins, no title.
    let (popup, quick) = cx
        .update(|cx| app.read(cx).quick_search())
        .expect("HUD aberto");
    popup
        .update(cx, |_, window, cx| {
            assert!(quick.read(cx).is_autotype());
            window.render_frame(cx);
            assert!(window.try_find("quick-item-login-1").is_some());
            assert!(window.try_find("quick-item-card-org").is_none());
            assert!(window.try_find("quick-search-footer").is_none());
            assert!(window.try_find("quick-search-close").is_some());
            quick.update(cx, |q, cx| q.set_query("chave", window, cx));
            window.render_frame(cx);
            assert!(window.try_find("quick-item-login-1").is_none());
            window.click("quick-item-login-item-key", cx);
        })
        .unwrap();
    cx.run_until_parked();
    assert!(popup.update(cx, |_, _, _| ()).is_err(), "o HUD fechou");
    // This item asks for the vault password: nothing is typed before confirming.
    let (password, _) = fixture_password_and_email();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(typed.lock().unwrap().is_empty());
        window.click("reprompt-password", cx);
        window.input(&password, cx);
        window.press("enter", cx);
    })
    .unwrap();
    wait_until(cx, |_| !typed.lock().unwrap().is_empty());
    assert_eq!(
        typed.lock().unwrap()[0].1,
        ["item-key-pw".to_owned(), "↵".into()]
    );
}

#[gpui_kit::test]
fn autotype_hud_lists_suggestions_first_and_closes(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (handle, app) = open_app(cx, Some(unlocked_fixture()));
    let _typed = fake_typer(&app, cx);
    let ids = |cx: &mut TestAppContext| {
        let (_, quick) = cx.update(|cx| app.read(cx).quick_search()).unwrap();
        cx.update(|cx| {
            quick
                .read(cx)
                .results()
                .iter()
                .map(|i| i.id.clone())
                .collect::<Vec<_>>()
        })
    };
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |app, cx| {
            app.handle_autotype(request("Sem título - Bloco de Notas"), window, cx)
        });
    })
    .unwrap();
    let all = ids(cx);
    assert!(all.len() > 1);
    // The close button closes the HUD without typing.
    let (popup, _) = cx.update(|cx| app.read(cx).quick_search()).unwrap();
    popup
        .update(cx, |_, window, cx| {
            window.render_frame(cx);
            window.click("quick-search-close", cx);
        })
        .unwrap();
    cx.run_until_parked();
    assert!(popup.update(cx, |_, _, _| ()).is_err(), "o HUD fechou");
}

#[gpui_kit::test]
fn autotype_waits_for_unlock_and_ignores_own_window(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (handle, app) = open_app(cx, Some(unlocked_fixture()));
    let typed = fake_typer(&app, cx);
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |app, cx| {
            // Scout Vault's own window: nothing to type.
            let mut own = request("Scout Vault");
            own.own_window = true;
            app.handle_autotype(own, window, cx);
            app.lock(window, cx);
            app.handle_autotype(request("banco.example.com"), window, cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
    assert!(typed.lock().unwrap().is_empty());

    let (password, _) = fixture_password_and_email();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("unlock-password", cx);
        window.input(&password, cx);
        window.click("unlock-submit", cx);
    })
    .unwrap();
    // On unlock, the stored request is handled.
    wait_until(cx, |_| !typed.lock().unwrap().is_empty());
    assert_eq!(typed.lock().unwrap().len(), 1);
    assert_eq!(typed.lock().unwrap()[0].1[0], "pessoa");
}

#[gpui_kit::test]
fn autotype_can_be_turned_off_and_skip_enter(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let dir = std::env::temp_dir().join(format!("scout-vault-ui-autotype-{}", std::process::id()));
    let store = SettingsStore::at(dir.join("settings.json"));
    cx.update(gpui_kit::init);
    let s2 = store.clone();
    let (handle, app) = cx.update(|cx| {
        gpui_kit::open_window(options(), cx, |window, cx| {
            cx.new(|cx| ScoutVaultApp::with_unlocked(s2, unlocked_fixture(), window, cx))
        })
        .unwrap()
    });
    let typed = fake_typer(&app, cx);

    // Settings: remove the Enter; then turn everything off.
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |app, cx| app.open_settings(window, cx));
        window.render_frame(cx);
        let view = app.read(cx).settings_view().unwrap().clone();
        view.update(cx, |v, cx| {
            let next = scout_vault_app::settings::AutoTypeSettings {
                press_enter: false,
                ..v.autotype()
            };
            v.set_autotype(next, cx)
        });
    })
    .unwrap();
    assert!(!store.load().autotype.press_enter);
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |app, cx| {
            app.close_settings(cx);
            app.handle_autotype(request("banco.example.com"), window, cx);
        })
    })
    .unwrap();
    wait_until(cx, |_| !typed.lock().unwrap().is_empty());
    assert_eq!(typed.lock().unwrap()[0].1.last().unwrap(), "s3nh@-ção");

    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |app, cx| app.open_settings(window, cx));
        window.render_frame(cx);
        let view = app.read(cx).settings_view().unwrap().clone();
        view.update(cx, |v, cx| {
            let next = scout_vault_app::settings::AutoTypeSettings {
                enabled: false,
                ..v.autotype()
            };
            v.set_autotype(next, cx)
        });
        app.update(cx, |app, cx| {
            app.close_settings(cx);
            app.handle_autotype(request("banco.example.com"), window, cx);
        });
    })
    .unwrap();
    assert!(!store.load().autotype.enabled);
    cx.run_until_parked();
    assert_eq!(typed.lock().unwrap().len(), 1, "desligado não digita");
    let _ = std::fs::remove_dir_all(dir);
}

#[gpui_kit::test]
fn entry_art_can_be_stilled(cx: &mut TestAppContext) {
    let (handle, app) = open_app(cx, None);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("entry-backdrop").is_some());
        app.update(cx, |app, cx| app.open_settings(window, cx));
        window.render_frame(cx);
        window.click("motion-off", cx);
        let view = app.read(cx).settings_view().unwrap().clone();
        assert_eq!(
            view.read(cx).appearance().motion,
            scout_vault_app::appearance::Motion::Off
        );
        app.update(cx, |app, cx| app.close_settings(cx));
        window.render_frame(cx);
        assert_eq!(window.find("entry-backdrop").label(), Some("parado"));
        // "Still" applies to every interface animation.
        assert!(cx.reduce_motion());
    })
    .unwrap();
}

fn open_with_store(
    cx: &mut TestAppContext,
    store: &SettingsStore,
    unlocked: Option<Unlocked>,
) -> (AnyWindowHandle, Entity<ScoutVaultApp>) {
    let store = store.clone();
    cx.update(|cx| {
        gpui_kit::open_window(options(), cx, |window, cx| {
            cx.new(|cx| match unlocked {
                Some(u) => ScoutVaultApp::with_unlocked(store, u, window, cx),
                None => ScoutVaultApp::new(store, window, cx),
            })
        })
        .unwrap()
    })
}

#[gpui_kit::test]
fn remembered_session_reopens_locked_and_unlocks_offline(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    cx.update(gpui_kit::init);
    let store = SettingsStore::in_memory();

    // First launch: vault open; locking stores the session.
    let (handle, app) = open_with_store(cx, &store, Some(unlocked_fixture()));
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |a, cx| a.lock(window, cx))
    })
    .unwrap();
    assert!(scout_vault_app::saved_session::load(&store).is_some());

    // "Reopen the app": starts at unlock, without login.
    let (handle, app) = open_with_store(cx, &store, None);
    cx.update(|cx| assert_eq!(app.read(cx).screen_kind(), ScreenKind::Locked));

    // The fixture's server doesn't exist (127.0.0.1:9): opens from the cache, offline.
    let (password, _) = fixture_password_and_email();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("unlock-password", cx);
        window.input(&password, cx);
        window.click("unlock-submit", cx);
    })
    .unwrap();
    wait_until(cx, |cx| {
        cx.update(|cx| app.read(cx).screen_kind() == ScreenKind::Vault)
    });
    let count = cx.update(|cx| {
        let v = app.read(cx).vault_view().unwrap().read(cx);
        v.list().read(cx).delegate().vault().items.len()
    });
    assert!(count >= 5, "itens do cache: {count}");
}

#[gpui_kit::test]
fn logging_out_or_turning_off_forgets_the_session(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    cx.update(gpui_kit::init);
    let store = SettingsStore::in_memory();
    let (handle, app) = open_with_store(cx, &store, Some(unlocked_fixture()));
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |a, cx| a.lock(window, cx))
    })
    .unwrap();
    assert!(scout_vault_app::saved_session::load(&store).is_some());

    // Signing out deletes it.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("unlock-logout", cx);
    })
    .unwrap();
    cx.update(|cx| assert_eq!(app.read(cx).screen_kind(), ScreenKind::Login));
    assert!(scout_vault_app::saved_session::load(&store).is_none());
    assert!(store.read_session_blob_for_tests().is_none());

    // Turning the option off deletes it too.
    let (handle, app) = open_with_store(cx, &store, Some(unlocked_fixture()));
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |a, cx| a.lock(window, cx))
    })
    .unwrap();
    assert!(scout_vault_app::saved_session::load(&store).is_some());
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |a, cx| a.open_settings(window, cx));
        window.render_frame(cx);
        let view = app.read(cx).settings_view().unwrap().clone();
        view.update(cx, |v, cx| v.set_remember_session(false, cx));
    })
    .unwrap();
    assert!(store.read_session_blob_for_tests().is_none());
}

#[gpui_kit::test]
fn editor_validates_switches_kind_and_keeps_data_on_failure(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (handle, app) = open_app(cx, Some(unlocked_fixture()));
    let vault = cx.update(|cx| app.read(cx).vault_view().cloned().unwrap());

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("vault-new", cx);
        window.render_frame(cx);
        // Doesn't save without a name.
        window.click("editor-save", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("editor-status").label(),
            Some("Dê um nome ao item.")
        );
        // Switch to note and back to login without losing the name.
        window.click("editor-name", cx);
        window.input("Meu novo", cx);
        window.click("editor-kind-note", cx);
        window.render_frame(cx);
        assert!(window.try_find("editor-password").is_none());
        window.click("editor-kind-login", cx);
        window.render_frame(cx);
        window.click("editor-generate", cx);
        window.render_frame(cx);
    })
    .unwrap();
    let draft = cx.update(|cx| {
        let editor = vault.read(cx).editor().unwrap().clone();
        editor.read(cx).to_item(cx)
    });
    assert_eq!(draft.name, "Meu novo");
    let scout_vault_core::vault::ItemKind::Login(l) = &draft.kind else {
        panic!()
    };
    assert_eq!(
        l.password.as_ref().unwrap().expose().chars().count(),
        20,
        "senha gerada"
    );

    // The fixture's server doesn't exist: the error shows up and the form stays there.
    cx.update_window(handle, |_, window, cx| window.click("editor-save", cx))
        .unwrap();
    wait_until(cx, |cx| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            !window
                .find("editor-status")
                .label()
                .unwrap_or("")
                .is_empty()
        })
        .unwrap()
    });
    cx.update(|cx| assert!(vault.read(cx).editor().is_some()));
}

#[gpui_kit::test]
fn ssh_keys_are_generated_checked_and_unlocked(cx: &mut TestAppContext) {
    use scout_vault_core::vault::ItemKind;
    use ssh_key::{getrandom::SysRng, rand_core::UnwrapErr, Algorithm, HashAlg, LineEnding};
    cx.executor().allow_parking();
    let (handle, app) = open_app(cx, Some(unlocked_fixture()));
    let vault = cx.update(|cx| app.read(cx).vault_view().cloned().unwrap());
    let ssh_of = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            let editor = vault.read(cx).editor().unwrap().clone();
            let ItemKind::SshKey(k) = editor.read(cx).to_item(cx).kind else {
                panic!("not an SSH key")
            };
            k
        })
    };

    // A new SSH key item comes with a fresh Ed25519 key.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("vault-new", cx);
        window.render_frame(cx);
        window.click("editor-kind-ssh", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("editor-ssh-status").label(),
            Some("Chave válida")
        );
        assert!(window
            .find("editor-ssh-public")
            .label()
            .unwrap()
            .starts_with("ssh-ed25519 AAAA"));
        assert!(window
            .find("editor-ssh-fingerprint")
            .label()
            .unwrap()
            .starts_with("SHA256:"));
    })
    .unwrap();
    let first = ssh_of(cx);
    assert!(first
        .private_key
        .as_ref()
        .unwrap()
        .expose()
        .starts_with("-----BEGIN OPENSSH PRIVATE KEY-----"));

    // "Generate new key" replaces it.
    cx.update_window(handle, |_, window, cx| {
        window.click("editor-ssh-generate", cx);
        window.render_frame(cx);
    })
    .unwrap();
    let second = ssh_of(cx);
    assert_ne!(second.public_key, first.public_key);

    // Something that isn't a key: flagged, and not saved.
    let editor = cx.update(|cx| vault.read(cx).editor().unwrap().clone());
    cx.update_window(handle, |_, window, cx| {
        editor.update(cx, |e, cx| e.set_ssh_private("hello", window, cx));
        window.render_frame(cx);
        assert_eq!(
            window.find("editor-ssh-status").label(),
            Some("Não é uma chave privada válida (OpenSSH, PuTTY .ppk ou RSA PEM).")
        );
        window.click("editor-name", cx);
        window.input("Servidor", cx);
        window.click("editor-save", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("editor-status").label(),
            Some("Não é uma chave privada válida (OpenSSH, PuTTY .ppk ou RSA PEM).")
        );
    })
    .unwrap();

    // A key with a passphrase: asks for it, and keeps the key without it.
    let plain = ssh_key::PrivateKey::random(&mut UnwrapErr(SysRng), Algorithm::Ed25519).unwrap();
    let locked = plain
        .encrypt(&mut UnwrapErr(SysRng), "segredo")
        .unwrap()
        .to_openssh(LineEnding::LF)
        .unwrap();
    cx.update_window(handle, |_, window, cx| {
        editor.update(cx, |e, cx| e.set_ssh_private(&locked, window, cx));
        window.render_frame(cx);
        assert_eq!(
            window.find("editor-ssh-status").label(),
            Some("Chave protegida por senha: digite a senha para importá-la.")
        );
        window.click("editor-ssh-passphrase", cx);
        window.input("segredo", cx);
        window.click("editor-ssh-unlock", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("editor-ssh-status").label(),
            Some("Chave válida")
        );
        assert!(window.try_find("editor-ssh-passphrase").is_none());
    })
    .unwrap();
    let unlocked = ssh_of(cx);
    assert_eq!(
        unlocked.fingerprint.as_deref(),
        Some(plain.fingerprint(HashAlg::Sha256).to_string().as_str())
    );
    let private = unlocked.private_key.unwrap();
    assert!(!ssh_key::PrivateKey::from_openssh(private.expose())
        .unwrap()
        .is_encrypted());
}

/// A key whose item asks for the master password: every signature asks for it,
/// and "Remember" isn't offered.
#[cfg(windows)]
#[gpui_kit::test]
fn ssh_agent_asks_the_master_password_for_reprompt_keys(cx: &mut TestAppContext) {
    use scout_vault_app::ssh_agent;
    use scout_vault_core::vault::{Item, ItemKind, SshKey};
    use std::io::{Read, Write};
    cx.executor().allow_parking();

    let (password, _) = fixture_password_and_email();
    let key = scout_vault_core::ssh::generate();
    let blob = ssh_key::PrivateKey::from_openssh(key.private_key.as_str())
        .unwrap()
        .public_key()
        .to_bytes()
        .unwrap();
    let mut unlocked = unlocked_fixture();
    unlocked.vault.items.push(Item {
        id: "ssh-reprompt".into(),
        organization_id: None,
        folder_id: None,
        name: "Produção".into(),
        notes: None,
        favorite: false,
        reprompt: true,
        deleted: false,
        revision_date: None,
        attachments: Vec::new(),
        collection_ids: Vec::new(),
        fields: Vec::new(),
        kind: ItemKind::SshKey(SshKey {
            private_key: Some(scout_vault_core::Secret::new(key.private_key.to_string())),
            public_key: Some(key.public_key.clone()),
            fingerprint: Some(key.fingerprint.clone()),
        }),
    });
    let (handle, app) = open_app(cx, Some(unlocked));
    let pipe = format!(
        r"\\.\pipe\scout-vault-test-agent-reprompt-{}",
        std::process::id()
    );
    cx.update(|cx| {
        ssh_agent::set_pipe(&pipe, cx);
        ssh_agent::set_enabled(true, cx);
    });
    let sign = {
        let pipe = pipe.clone();
        let mut body = vec![13];
        body.extend_from_slice(&(blob.len() as u32).to_be_bytes());
        body.extend_from_slice(&blob);
        body.extend_from_slice(&5u32.to_be_bytes());
        body.extend_from_slice(b"hello");
        body.extend_from_slice(&0u32.to_be_bytes());
        move || {
            let (pipe, body) = (pipe.clone(), body.clone());
            std::thread::spawn(move || {
                let mut conn = std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(&pipe)
                    .unwrap();
                conn.write_all(&(body.len() as u32).to_be_bytes()).unwrap();
                conn.write_all(&body).unwrap();
                let mut len = [0u8; 4];
                conn.read_exact(&mut len).unwrap();
                let mut answer = vec![0u8; u32::from_be_bytes(len) as usize];
                conn.read_exact(&mut answer).unwrap();
                answer[0]
            })
        }
    };

    let waiting = sign();
    wait_until(cx, |cx| {
        cx.update(|cx| app.read(cx).ssh_request().is_some())
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("ssh-password").is_some());
        assert!(
            window.try_find("ssh-remember").is_none(),
            "never remembered"
        );
        let path = window.find("ssh-request-path").label().unwrap().to_owned();
        assert!(path.contains('\\'), "full path shown: {path}");
        // "Allow" alone (or a wrong password) doesn't sign.
        window.click("ssh-password", cx);
        window.input("errada", cx);
        window.click("ssh-allow", cx);
    })
    .unwrap();
    wait_until(cx, |cx| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.try_find("ssh-password-error").is_some()
        })
        .unwrap()
    });
    assert!(
        !waiting.is_finished(),
        "still waiting after a wrong password"
    );
    cx.update_window(handle, |_, window, cx| {
        window.click("ssh-password", cx);
        window.input(&password, cx);
        window.press("enter", cx);
    })
    .unwrap();
    wait_until(cx, |_| waiting.is_finished());
    assert_eq!(
        waiting.join().unwrap(),
        14,
        "signed with the master password"
    );

    // The next signature asks again.
    let again = sign();
    wait_until(cx, |cx| {
        cx.update(|cx| app.read(cx).ssh_request().is_some())
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("ssh-password").is_some());
        window.click("ssh-deny", cx);
    })
    .unwrap();
    assert_eq!(again.join().unwrap(), 5);
    cx.update(|cx| ssh_agent::set_enabled(false, cx));
}

#[cfg(windows)]
#[gpui_kit::test]
fn ssh_agent_offers_vault_keys_and_signs_only_when_allowed(cx: &mut TestAppContext) {
    use scout_vault_app::ssh_agent::{self, Status};
    use scout_vault_core::vault::{Item, ItemKind, SshKey};
    use std::io::{Read, Write};
    cx.executor().allow_parking();

    let key = scout_vault_core::ssh::generate();
    let blob = ssh_key::PrivateKey::from_openssh(key.private_key.as_str())
        .unwrap()
        .public_key()
        .to_bytes()
        .unwrap();
    let mut unlocked = unlocked_fixture();
    unlocked.vault.items.push(Item {
        id: "ssh-agent-test".into(),
        organization_id: None,
        folder_id: None,
        name: "GitHub pessoal".into(),
        notes: None,
        favorite: false,
        reprompt: false,
        deleted: false,
        revision_date: None,
        attachments: Vec::new(),
        collection_ids: Vec::new(),
        fields: Vec::new(),
        kind: ItemKind::SshKey(SshKey {
            private_key: Some(scout_vault_core::Secret::new(key.private_key.to_string())),
            public_key: Some(key.public_key.clone()),
            fingerprint: Some(key.fingerprint.clone()),
        }),
    });
    let (handle, app) = open_app(cx, Some(unlocked));
    let pipe = format!(r"\\.\pipe\scout-vault-test-agent-{}", std::process::id());
    cx.update(|cx| {
        ssh_agent::set_pipe(&pipe, cx);
        ssh_agent::set_enabled(true, cx);
        assert_eq!(ssh_agent::status(cx), Status::Running);
    });

    // A client: one request, one answer.
    fn ask(pipe: &str, body: &[u8]) -> Vec<u8> {
        let mut conn = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(pipe)
            .unwrap();
        conn.write_all(&(body.len() as u32).to_be_bytes()).unwrap();
        conn.write_all(body).unwrap();
        let mut len = [0u8; 4];
        conn.read_exact(&mut len).unwrap();
        let mut answer = vec![0u8; u32::from_be_bytes(len) as usize];
        conn.read_exact(&mut answer).unwrap();
        answer
    }
    let sign_body = {
        let mut body = vec![13];
        body.extend_from_slice(&(blob.len() as u32).to_be_bytes());
        body.extend_from_slice(&blob);
        body.extend_from_slice(&5u32.to_be_bytes());
        body.extend_from_slice(b"hello");
        body.extend_from_slice(&0u32.to_be_bytes());
        body
    };
    let contains = |hay: &[u8], needle: &[u8]| hay.windows(needle.len()).any(|w| w == needle);

    // The list has the vault's key, named after the item.
    let list = ask(&pipe, &[11]);
    assert_eq!(list[0], 12);
    assert!(contains(&list, &blob));
    assert!(contains(&list, "GitHub pessoal".as_bytes()));

    // Signing waits for "Allow".
    let sign = |pipe: String, body: Vec<u8>| std::thread::spawn(move || ask(&pipe, &body));
    let waiting = sign(pipe.clone(), sign_body.clone());
    wait_until(cx, |cx| {
        cx.update(|cx| app.read(cx).ssh_request().is_some())
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let text = window.find("ssh-request-text").label().unwrap().to_owned();
        assert!(text.contains("“GitHub pessoal”"), "{text}");
        window.click("ssh-allow", cx);
    })
    .unwrap();
    let answer = waiting.join().unwrap();
    assert_eq!(answer[0], 14, "signed");
    let mut rest = &answer[1..];
    let len = u32::from_be_bytes(rest[..4].try_into().unwrap()) as usize;
    rest = &rest[4..];
    let signature = {
        use ssh_key::encoding::Decode;
        ssh_key::Signature::decode(&mut &rest[..len]).unwrap()
    };
    {
        use signature::Verifier;
        ssh_key::PublicKey::from_bytes(&blob)
            .unwrap()
            .key_data()
            .verify(b"hello", &signature)
            .unwrap();
    }

    // "Deny": the program gets a failure.
    let waiting = sign(pipe.clone(), sign_body.clone());
    wait_until(cx, |cx| {
        cx.update(|cx| app.read(cx).ssh_request().is_some())
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("ssh-deny", cx);
    })
    .unwrap();
    assert_eq!(waiting.join().unwrap(), [5]);
    cx.update(|cx| assert!(app.read(cx).ssh_request().is_none()));

    // Locked: no keys at all.
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |a, cx| a.lock(window, cx))
    })
    .unwrap();
    let list = ask(&pipe, &[11]);
    assert_eq!(list, [12, 0, 0, 0, 0]);
    assert_eq!(ask(&pipe, &sign_body), [5]);

    cx.update(|cx| ssh_agent::set_enabled(false, cx));
}

#[gpui_kit::test]
fn import_reads_an_export_skips_duplicates_and_sends_it(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (handle, app) = open_app(cx, Some(unlocked_fixture()));
    let vault = cx.update(|cx| app.read(cx).vault_view().cloned().unwrap());
    let dir = std::env::temp_dir().join(format!("scout-vault-ui-import-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // Settings → Data → "Import…" (the button sits below the fold, so the event is sent).
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("vault-settings", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let settings = app
            .read(cx)
            .settings_view()
            .cloned()
            .expect("settings open");
        settings.update(cx, |_, cx| {
            cx.emit(scout_vault_app::settings_view::SettingsEvent::OpenImport)
        });
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(app.read(cx).settings_view().is_none());
        assert!(window.try_find("import-zone").is_some());
    })
    .unwrap();
    let import = cx.update(|cx| app.read(cx).import_view().cloned().expect("import open"));

    // A password-protected export: explained, nothing to import.
    let locked = dir.join("locked.json");
    std::fs::write(&locked, r#"{"encrypted":true,"passwordProtected":true}"#).unwrap();
    cx.update(|cx| import.update(cx, |v, cx| v.load_path(locked.clone(), cx)));
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(import
            .read(cx)
            .error()
            .unwrap()
            .contains("protegido por senha"));
        assert!(import.read(cx).to_import().is_empty());
    });

    // A browser CSV: one login the vault already has, one new.
    let csv = dir.join("Chrome Passwords.csv");
    std::fs::write(
        &csv,
        "name,url,username,password,note\n\
         Meu Banco,https://banco.example.com,pessoa,s3nh@-ção,\n\
         Novo site,https://novo.example,ana,outra,\n",
    )
    .unwrap();
    cx.update(|cx| import.update(cx, |v, cx| v.load_path(csv.clone(), cx)));
    cx.run_until_parked();
    cx.update(|cx| {
        let v = import.read(cx);
        assert!(v.error().is_none());
        assert_eq!(v.duplicates(), 1);
        let names: Vec<_> = v.to_import().iter().map(|i| i.item.name.clone()).collect();
        assert_eq!(names, ["Novo site"]);
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("import-file").label(),
            Some("Chrome Passwords.csv")
        );
        // Keeping duplicates too: both go.
        window.click("import-skip-duplicates", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.update(|cx| assert_eq!(import.read(cx).to_import().len(), 2));

    // "Import": the screen closes and the vault sends it (the fixture's server
    // doesn't exist, so it ends in an error toast, but the request went out).
    cx.update_window(handle, |_, window, cx| window.click("import-run", cx))
        .unwrap();
    cx.update(|cx| {
        assert!(app.read(cx).import_view().is_none());
        assert!(vault.read(cx).is_syncing());
    });
    wait_until(cx, |cx| cx.update(|cx| !vault.read(cx).is_syncing()));
    let _ = std::fs::remove_dir_all(dir);
}

#[gpui_kit::test]
fn a_line_separates_the_title_bar_from_the_vault(cx: &mut TestAppContext) {
    let (handle, _app) = open_app(cx, Some(unlocked_fixture()));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let line = window.find("title-divider").bounds();
        let actions = window.find("vault-settings").bounds();
        // Right under the title bar, one pixel tall, across the window.
        assert!(f32::from(line.size.height) <= 1.5);
        assert!(line.top() >= actions.bottom());
        assert!(f32::from(line.size.width) > 1000.0);
        // Settings has no sidebar: no line.
        window.click("vault-settings", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("title-divider").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn sidebar_sections_collapse_and_entries_take_custom_icons(cx: &mut TestAppContext) {
    let (_dir, store) = file_store("sidebar");
    cx.update(gpui_kit::init);
    let (handle, app) = open_with_store(cx, &store, Some(unlocked_fixture()));
    let vault = cx.update(|cx| app.read(cx).vault_view().cloned().unwrap());

    // "Types" collapses: its entries leave; the choice is saved.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("filter-login").is_some());
        // Headings are one line tall (they once grew to fill the sidebar's empty space).
        for section in ["section-types", "section-folders"] {
            let h = f32::from(window.find(section).bounds().size.height);
            assert!(h < 40.0, "{section}: {h}");
        }
        let types = window.find("section-types").bounds();
        let login = window.find("filter-login").bounds();
        assert!(f32::from(login.top() - types.bottom()) < 12.0);
        // The arrow and the "+" sit on the heading's center line.
        let center = |b: gpui_kit::Bounds<gpui_kit::Pixels>| f32::from(b.center().y);
        let heading = window.find("section-folders").bounds();
        let arrow = window.find("section-folders-arrow").bounds();
        let add = window.find("folder-new").bounds();
        assert!(
            (center(arrow) - center(heading)).abs() < 1.5,
            "arrow off center"
        );
        assert!((center(add) - center(heading)).abs() < 1.5, "+ off center");
        // The "+" of Folders stays beside the heading, inside the sidebar.
        let plus = window.find("folder-new").bounds();
        let folders = window.find("section-folders").bounds();
        assert!(plus.left() >= folders.right() - gpui_kit::px(1.));
        assert!(f32::from(plus.right()) <= scout_vault_app::vault_view::SIDEBAR_WIDTH);
        window.click("section-types", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("filter-login").is_none());
        assert!(window.try_find("filter-all").is_some());
    })
    .unwrap();
    assert_eq!(store.load().sidebar_collapsed, ["types"]);

    // Folders too (the folders and "No folder").
    cx.update_window(handle, |_, window, cx| window.click("section-folders", cx))
        .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("filter-no-folder").is_none());
    })
    .unwrap();
    assert_eq!(store.load().sidebar_collapsed, ["folders", "types"]);

    // Next time the vault opens, it comes back the same way.
    cx.update(|cx| {
        vault.update(cx, |v, _| {
            v.set_sidebar(vec!["types".into()], Default::default())
        })
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("filter-login").is_none());
        assert!(window.try_find("filter-no-folder").is_some());
        // Expanding again.
        window.click("section-types", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("filter-login").is_some());
        // Right click on an entry: icon menu; pick an icon and a color.
        window.right_click("filter-login", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("sidebar-icon-menu-filter-login").is_some());
        window.click("sidebar-icon-rocket", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("sidebar-color-red", cx);
    })
    .unwrap();
    let saved = store.load().sidebar_icons;
    let chosen = saved.get("filter-login").expect("saved");
    assert_eq!(chosen.icon, "rocket");
    assert_eq!(chosen.color.as_deref(), Some("red"));
    cx.update(|cx| {
        assert_eq!(
            vault
                .read(cx)
                .sidebar_icon("filter-login")
                .map(|c| c.icon.as_str()),
            Some("rocket")
        )
    });

    // "Default icon": back to the globe, and forgotten.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("sidebar-icon-reset", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("sidebar-icon-menu-filter-login").is_none());
    })
    .unwrap();
    assert!(store.load().sidebar_icons.is_empty());
}

#[gpui_kit::test]
fn the_star_favorites_an_item_from_its_detail(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (handle, app) = open_app(cx, Some(unlocked_fixture()));
    let vault = cx.update(|cx| app.read(cx).vault_view().cloned().unwrap());
    // "Meu Banco" is a favorite in the fixture.
    cx.update_window(handle, |_, window, cx| {
        vault.update(cx, |v, cx| v.select_item("login-1", window, cx));
        window.render_frame(cx);
        assert_eq!(
            window.find("item-favorite").label(),
            Some("Tirar dos favoritos")
        );
        window.click("item-favorite", cx);
        window.render_frame(cx);
        // Right away on screen, while it goes to the server.
        assert_eq!(window.find("item-favorite").label(), Some("Favoritar"));
    })
    .unwrap();
    cx.update(|cx| {
        assert!(!vault.read(cx).selected().unwrap().favorite);
        assert!(vault.read(cx).is_syncing());
    });
    // The fixture's server doesn't exist: the star goes back.
    wait_until(cx, |cx| cx.update(|cx| !vault.read(cx).is_syncing()));
    cx.update(|cx| assert!(vault.read(cx).selected().unwrap().favorite));
}

#[gpui_kit::test]
fn a_new_version_shows_in_the_title_bar_and_in_about(cx: &mut TestAppContext) {
    use scout_vault_app::updater::{self, State};
    use scout_vault_core::update::{Release, Version};
    let (handle, app) = open_app(cx, Some(unlocked_fixture()));
    // Nothing found yet: no button in the title bar.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("vault-update").is_none());
    })
    .unwrap();

    // A newer release turns up.
    let release = Release {
        version: Version::parse("99.0.0").unwrap(),
        page: "https://github.com/qedrohenrique/scout-vault/releases/tag/v99.0.0".into(),
        notes: String::new(),
        installer: None,
        installer_sha256: None,
    };
    cx.update(|cx| updater::set_state(State::Available(Box::new(release)), cx));
    cx.update(|cx| assert_eq!(updater::available(cx).unwrap().to_string(), "99.0.0"));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // The title bar's button opens Settings.
        window.click("vault-update", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(app.read(cx).settings_view().is_some());
    })
    .unwrap();

    // "Check for updates" in the tests (offline): up to date, and the button goes away.
    cx.update(|cx| updater::check(false, cx));
    cx.update(|cx| assert_eq!(updater::state(cx), State::UpToDate));
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |a, cx| a.close_settings(cx));
        window.render_frame(cx);
        assert!(window.try_find("vault-update").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn edit_button_prefills_and_cancel_closes(cx: &mut TestAppContext) {
    let (handle, app) = open_app(cx, Some(unlocked_fixture()));
    let vault = cx.update(|cx| app.read(cx).vault_view().cloned().unwrap());
    cx.update_window(handle, |_, window, cx| {
        vault.update(cx, |v, cx| v.select_item("login-1", window, cx));
        window.render_frame(cx);
        window.click("item-edit", cx);
        window.render_frame(cx);
        assert!(window.try_find("editor-name").is_some());
    })
    .unwrap();
    let draft = cx.update(|cx| vault.read(cx).editor().unwrap().read(cx).to_item(cx));
    assert_eq!(draft.name, "Meu Banco");
    assert_eq!(draft.fields[0].value.as_ref().unwrap().expose(), "1234");
    let scout_vault_core::vault::ItemKind::Login(l) = &draft.kind else {
        panic!()
    };
    assert_eq!(l.password.as_ref().unwrap().expose(), "s3nh@-ção");
    assert_eq!(l.uris, ["https://banco.example.com"]);

    cx.update_window(handle, |_, window, cx| window.click("editor-cancel", cx))
        .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("item-edit").is_some());
        // Item in the trash: restore and delete permanently, without editing.
        vault.update(cx, |v, cx| {
            v.set_filter(scout_vault_app::item_list::Filter::Trash, window, cx)
        });
        vault.update(cx, |v, cx| v.select_item("note-deleted", window, cx));
        window.render_frame(cx);
        assert!(window.try_find("item-edit").is_none());
        assert!(window.try_find("item-restore").is_some());
        window.click("item-delete-forever", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window
            .find("item-delete-forever")
            .label()
            .is_some_and(|l| l.contains("Confirmar")));
    })
    .unwrap();
}

/// Creates, edits and deletes an item through the interface, against the local Vaultwarden.
#[gpui_kit::test]
#[ignore = "precisa de servidor; veja dev/test-account.env"]
fn creates_edits_and_deletes_through_the_ui(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let env = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("defina {k}"));
    let (url, email, password) = (
        env("SCOUT_VAULT_TEST_URL"),
        env("SCOUT_VAULT_TEST_EMAIL"),
        env("SCOUT_VAULT_TEST_PASSWORD"),
    );
    let (handle, app) = open_app(cx, None);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.within("login-server").click(2usize, cx);
        window.click("login-server-url", cx);
        window.input(&url, cx);
        window.click("login-email", cx);
        window.input(&email, cx);
        window.click("login-password", cx);
        window.input(&password, cx);
        window.click("login-submit", cx);
    })
    .unwrap();
    wait_until(cx, |cx| {
        cx.update(|cx| app.read(cx).screen_kind() == ScreenKind::Vault)
    });
    let vault = cx.update(|cx| app.read(cx).vault_view().cloned().unwrap());
    let name = format!("Pela UI {}", std::process::id());
    let find = |cx: &mut TestAppContext, name: &str| {
        cx.update(|cx| {
            vault
                .read(cx)
                .list()
                .read(cx)
                .delegate()
                .vault()
                .items
                .iter()
                .find(|i| i.name == name)
                .cloned()
        })
    };

    // Create.
    cx.update_window(handle, |_, window, cx| {
        window.click("vault-new", cx);
        window.render_frame(cx);
        window.click("editor-name", cx);
        window.input(&name, cx);
        window.click("editor-username", cx);
        window.input("ui-user", cx);
        window.click("editor-password", cx);
        window.input("ui-senha-1", cx);
        window.click("editor-uri-0", cx);
        window.input("https://ui.example", cx);
        window.click("editor-save", cx);
    })
    .unwrap();
    wait_until(cx, |cx| {
        find(cx, &name).is_some() && cx.update(|cx| vault.read(cx).editor().is_none())
    });
    let created = find(cx, &name).unwrap();
    cx.update(|cx| assert_eq!(vault.read(cx).selected().unwrap().id, created.id));

    // Edit: change the password.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("item-edit", cx);
        window.render_frame(cx);
        window.click("editor-password", cx);
        window.press("ctrl-a", cx);
        window.input("ui-senha-2", cx);
        window.click("editor-save", cx);
    })
    .unwrap();
    wait_until(cx, |cx| {
        find(cx, &name).is_some_and(|i| match &i.kind {
            scout_vault_core::vault::ItemKind::Login(l) => l
                .password
                .as_ref()
                .is_some_and(|p| p.expose() == "ui-senha-2"),
            _ => false,
        })
    });

    // Trash and delete permanently.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("item-trash", cx);
    })
    .unwrap();
    wait_until(cx, |cx| find(cx, &name).is_some_and(|i| i.deleted));
    cx.update_window(handle, |_, window, cx| {
        vault.update(cx, |v, cx| {
            v.set_filter(scout_vault_app::item_list::Filter::Trash, window, cx)
        });
        vault.update(cx, |v, cx| v.select_item(&created.id, window, cx));
        window.render_frame(cx);
        window.click("item-delete-forever", cx);
        window.render_frame(cx);
        window.click("item-delete-forever", cx);
    })
    .unwrap();
    wait_until(cx, |cx| find(cx, &name).is_none());
}

/// Attaches, opens and removes a file through the interface, against the local Vaultwarden.
#[gpui_kit::test]
#[ignore = "precisa de servidor; veja dev/test-account.env"]
fn attachments_through_the_ui(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let env = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("defina {k}"));
    let (url, email, password) = (
        env("SCOUT_VAULT_TEST_URL"),
        env("SCOUT_VAULT_TEST_EMAIL"),
        env("SCOUT_VAULT_TEST_PASSWORD"),
    );
    let (handle, app) = open_app(cx, None);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.within("login-server").click(2usize, cx);
        window.click("login-server-url", cx);
        window.input(&url, cx);
        window.click("login-email", cx);
        window.input(&email, cx);
        window.click("login-password", cx);
        window.input(&password, cx);
        window.click("login-submit", cx);
    })
    .unwrap();
    wait_until(cx, |cx| {
        cx.update(|cx| app.read(cx).screen_kind() == ScreenKind::Vault)
    });
    let vault = cx.update(|cx| app.read(cx).vault_view().cloned().unwrap());
    let name = format!("Anexo UI {}", std::process::id());
    cx.update_window(handle, |_, window, cx| {
        window.click("vault-new", cx);
        window.render_frame(cx);
        window.click("editor-kind-note", cx);
        window.render_frame(cx);
        window.click("editor-name", cx);
        window.input(&name, cx);
        window.click("editor-save", cx);
    })
    .unwrap();
    wait_until(cx, |cx| {
        cx.update(|cx| vault.read(cx).selected().is_some_and(|i| i.name == name))
    });

    let file =
        std::env::temp_dir().join(format!("scout-vault-ui-anexo-{}.txt", std::process::id()));
    std::fs::write(&file, "conteúdo secreto").unwrap();
    cx.update(|cx| vault.update(cx, |v, cx| v.attach_file_from_path(&file, cx)));
    wait_until(cx, |cx| {
        cx.update(|cx| {
            vault
                .read(cx)
                .selected()
                .is_some_and(|i| i.attachments.len() == 1)
        })
    });
    let att = cx.update(|cx| vault.read(cx).selected().unwrap().attachments[0].clone());
    assert_eq!(att.file_name, file.file_name().unwrap().to_string_lossy());
    // The server stores the size of the encrypted file (extra IV, MAC and padding).
    assert!(att
        .size
        .is_some_and(|s| s > "conteúdo secreto".len() as u64));

    // Opening downloads, decrypts and writes the temporary copy.
    cx.update(|cx| vault.update(cx, |v, cx| v.open_attachment(att.id.clone(), cx)));
    wait_until(cx, |cx| {
        cx.update(|cx| {
            app.read(cx)
                .toasts()
                .read(cx)
                .texts()
                .iter()
                .any(|t| t.contains("aberto"))
        })
    });

    // Remove (two clicks) and delete the item.
    let remove = gpui_kit::SharedString::from(format!("attach-remove-{}", att.id));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(remove.clone(), cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(remove.clone(), cx);
    })
    .unwrap();
    wait_until(cx, |cx| {
        cx.update(|cx| {
            vault
                .read(cx)
                .selected()
                .is_some_and(|i| i.attachments.is_empty())
        })
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("item-trash", cx);
    })
    .unwrap();
    wait_until(cx, |cx| {
        cx.update(|cx| vault.read(cx).selected().is_some_and(|i| i.deleted))
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("item-delete-forever", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("item-delete-forever", cx);
    })
    .unwrap();
    wait_until(cx, |cx| cx.update(|cx| vault.read(cx).selected().is_none()));
    let _ = std::fs::remove_file(file);
}

fn browser_ask(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    app: &Entity<ScoutVaultApp>,
    request: scout_vault_app::browser::Request,
) -> serde_json::Value {
    let (reply, rx) = std::sync::mpsc::channel();
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |a, cx| {
            a.handle_browser(
                scout_vault_app::browser::Incoming { request, reply },
                window,
                cx,
            )
        })
    })
    .unwrap();
    rx.recv().unwrap()
}

#[gpui_kit::test]
fn browser_requests_follow_setting_and_lock_state(cx: &mut TestAppContext) {
    use scout_vault_app::browser::Request;
    cx.update(gpui_kit::init);
    let dir = std::env::temp_dir().join(format!("scout-vault-ui-browser-{}", std::process::id()));
    let store = SettingsStore::at(dir.join("settings.json"));
    let (handle, app) = open_with_store(cx, &store, Some(unlocked_fixture()));
    let list = || Request::List {
        url: "https://banco.example.com/entrar".into(),
    };

    // Off by default.
    let off = browser_ask(cx, handle, &app, list());
    assert!(off["error"].as_str().unwrap().contains("desligado"));

    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |a, cx| a.open_settings(window, cx));
        window.render_frame(cx);
        // It sits lower on the page; the click is tested through the button's method.
        let view = app.read(cx).settings_view().unwrap().clone();
        view.update(cx, |v, cx| v.set_browser_fill(true, cx));
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find_all("browser-open-folder").len() == 1);
        app.update(cx, |a, cx| a.close_settings(cx));
    })
    .unwrap();

    let found = browser_ask(cx, handle, &app, list());
    assert_eq!(found["items"][0]["id"], "login-1");
    assert_eq!(found["items"][0]["username"], "pessoa");
    let fill = browser_ask(
        cx,
        handle,
        &app,
        Request::Fill {
            id: "login-1".into(),
            url: "https://banco.example.com/".into(),
        },
    );
    assert_eq!(fill["password"], "s3nh@-ção");
    // Another site doesn't get the password.
    let other = browser_ask(
        cx,
        handle,
        &app,
        Request::Fill {
            id: "login-1".into(),
            url: "https://phishing.example.net/".into(),
        },
    );
    assert!(other.get("password").is_none());

    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |a, cx| a.lock(window, cx))
    })
    .unwrap();
    assert_eq!(browser_ask(cx, handle, &app, list())["locked"], true);
    let _ = std::fs::remove_dir_all(dir);
}

#[gpui_kit::test]
fn progress_bar_shows_while_syncing(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (handle, _app) = open_app(cx, Some(unlocked_fixture()));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("vault-progress").is_none());
        window.click("vault-sync", cx);
        window.render_frame(cx);
        assert!(window.try_find("vault-progress").is_some());
    })
    .unwrap();
    // The fixture's server doesn't exist: the sync fails and the bar goes away.
    wait_until(cx, |cx| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.try_find("vault-progress").is_none()
        })
        .unwrap()
    });
}

/// The form uses the width: short fields share the row.
#[gpui_kit::test]
fn editor_layout_puts_short_fields_side_by_side(cx: &mut TestAppContext) {
    let (handle, _app) = open_app(cx, Some(unlocked_fixture()));
    let row = |window: &mut gpui_kit::Window, ids: &[&'static str]| {
        let b: Vec<_> = ids.iter().map(|id| window.find(*id).bounds()).collect();
        (
            b.iter().map(|b| f32::from(b.origin.y)).collect::<Vec<_>>(),
            b,
        )
    };
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("vault-new", cx);
        window.render_frame(cx);
        // Type picker in the bar, at the height of Save.
        let kind = f32::from(window.find("editor-kind-login").bounds().center().y);
        let save = f32::from(window.find("editor-save").bounds().center().y);
        assert!((kind - save).abs() < 2.0, "seletor {kind} x salvar {save}");
        // Centered in the free space: equal room on both sides.
        let first = window.find("editor-kind-login").bounds();
        let last = window.find("editor-kind-ssh").bounds();
        let cancel = window.find("editor-cancel").bounds();
        let detail = window.find("editor-name").bounds();
        let left_gap = f32::from(first.left() - detail.left());
        let right_gap = f32::from(cancel.left() - last.right());
        assert!(
            (left_gap - right_gap).abs() < 16.0,
            "esquerda {left_gap} x direita {right_gap}"
        );
        // Login: username and password side by side.
        let (ys, _) = row(window, &["editor-username", "editor-password"]);
        assert!((ys[0] - ys[1]).abs() < 1.0, "{ys:?}");
        window.click("editor-kind-card", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let (ys, b) = row(
            window,
            &["editor-card-month", "editor-card-year", "editor-card-code"],
        );
        assert!(
            ys.windows(2).all(|w| (w[0] - w[1]).abs() < 1.0),
            "mesma linha: {ys:?}"
        );
        assert!(
            b.iter().all(|b| f32::from(b.size.width) <= 90.0),
            "campos curtos estreitos"
        );
        let number = f32::from(window.find("editor-card-number").bounds().size.width);
        assert!(number > 300.0, "número do cartão largo: {number}");
        // Nothing goes past the window edge.
        let right = f32::from(window.find("editor-save").bounds().right());
        assert!(right <= 1100.0, "{right}");
        window.click("editor-kind-identity", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let (ys, _) = row(
            window,
            &["editor-id-city", "editor-id-state", "editor-id-postal"],
        );
        assert!(
            ys.windows(2).all(|w| (w[0] - w[1]).abs() < 1.0),
            "endereço em linha: {ys:?}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn sync_button_shows_progress_then_result_then_resets(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (handle, _app) = open_app(cx, Some(unlocked_fixture()));
    let label = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.find("vault-sync").label().map(str::to_owned)
        })
        .unwrap()
    };
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let (_, email) = fixture_password_and_email();
        assert_eq!(window.find("vault-email").label(), Some(email.as_str()));
        window.click("vault-sync", cx);
    })
    .unwrap();
    assert_eq!(label(cx).as_deref(), Some("Sincronizando…"));
    // The fixture's server doesn't exist: ends with an error and shows the ✗…
    wait_until(cx, |cx| {
        label(cx).as_deref() == Some("Não deu para sincronizar")
    });
    // …which goes back to the arrows after a moment.
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(1700));
    cx.run_until_parked();
    assert_eq!(label(cx).as_deref(), Some("Sincronizar"));
}

/// Notices become toasts: they appear, stay a bit and leave. Nothing stays fixed in the footer.
#[gpui_kit::test]
fn actions_show_a_toast_that_goes_away(cx: &mut TestAppContext) {
    let (handle, app) = open_app(cx, Some(unlocked_fixture()));
    let vault_view = cx.update(|cx| app.read(cx).vault_view().cloned().unwrap());
    let texts = |cx: &mut TestAppContext| cx.update(|cx| app.read(cx).toasts().read(cx).texts());
    cx.update_window(handle, |_, window, cx| {
        vault_view.update(cx, |v, cx| v.select_item("login-1", window, cx));
        window.render_frame(cx);
        assert!(
            window.try_find("vault-status").is_none(),
            "sem texto fixo no rodapé"
        );
        window.click("copy-password", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("toast").label(),
            Some("Copiado: Senha · some em 30 s")
        );
    })
    .unwrap();
    assert_eq!(texts(cx), ["Copiado: Senha · some em 30 s"]);
    // Copying again doesn't stack the same notice.
    cx.update_window(handle, |_, window, cx| window.click("copy-password", cx))
        .unwrap();
    assert_eq!(texts(cx).len(), 1);
    // After the timeout, it leaves (and disappears from the screen).
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(3000));
    cx.run_until_parked();
    assert!(texts(cx).is_empty());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("toast").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn folder_controls_show_up_and_validate(cx: &mut TestAppContext) {
    let (handle, app) = open_app(cx, Some(unlocked_fixture()));
    let vault = cx.update(|cx| app.read(cx).vault_view().cloned().unwrap());
    let toasts = |cx: &mut TestAppContext| cx.update(|cx| app.read(cx).toasts().read(cx).texts());

    // "+" next to Folders opens the field; empty doesn't create.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("folder-new-name").is_none());
        window.click("folder-new", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("folder-new-name").is_some());
        window.click("folder-new-save", cx);
    })
    .unwrap();
    assert_eq!(toasts(cx), ["Dê um nome à pasta."]);
    cx.update_window(handle, |_, window, cx| {
        window.click("folder-new-cancel", cx)
    })
    .unwrap();

    // With a folder open: rename and delete in the header.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("folder-new-name").is_none());
        assert!(
            window.try_find("folder-rename").is_none(),
            "só aparece numa pasta"
        );
        vault.update(cx, |v, cx| {
            v.set_filter(
                scout_vault_app::item_list::Filter::Folder(Some("folder-1".into())),
                window,
                cx,
            )
        });
        window.render_frame(cx);
        window.click("folder-delete", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window
            .find("folder-delete")
            .label()
            .is_some_and(|l| l.contains("Apagar?")));
        window.click("folder-rename", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("folder-rename-name").is_some());
        assert!(window.try_find("folder-rename").is_none());
    })
    .unwrap();
}

/// Creates, renames and deletes a folder through the interface, against the local Vaultwarden.
#[gpui_kit::test]
#[ignore = "precisa de servidor; veja dev/test-account.env"]
fn folders_through_the_ui(cx: &mut TestAppContext) {
    use scout_vault_app::item_list::Filter;
    cx.executor().allow_parking();
    let env = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("defina {k}"));
    let (url, email, password) = (
        env("SCOUT_VAULT_TEST_URL"),
        env("SCOUT_VAULT_TEST_EMAIL"),
        env("SCOUT_VAULT_TEST_PASSWORD"),
    );
    let (handle, app) = open_app(cx, None);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.within("login-server").click(2usize, cx);
        window.click("login-server-url", cx);
        window.input(&url, cx);
        window.click("login-email", cx);
        window.input(&email, cx);
        window.click("login-password", cx);
        window.input(&password, cx);
        window.click("login-submit", cx);
    })
    .unwrap();
    wait_until(cx, |cx| {
        cx.update(|cx| app.read(cx).screen_kind() == ScreenKind::Vault)
    });
    let vault = cx.update(|cx| app.read(cx).vault_view().cloned().unwrap());
    let folder = |cx: &mut TestAppContext, name: &str| {
        cx.update(|cx| {
            vault
                .read(cx)
                .list()
                .read(cx)
                .delegate()
                .vault()
                .folders
                .iter()
                .find(|f| f.name == name)
                .map(|f| f.id.clone())
        })
    };
    let name = format!("Pasta UI {}", std::process::id());
    let renamed = format!("{name} renomeada");

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("folder-new", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("folder-new-name", cx);
        window.input(&name, cx);
        window.press("enter", cx);
    })
    .unwrap();
    wait_until(cx, |cx| folder(cx, &name).is_some());
    let id = folder(cx, &name).unwrap();
    // Opens the freshly created folder.
    wait_until(cx, |cx| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.try_find("folder-rename").is_some()
        })
        .unwrap()
    });
    cx.update(|cx| {
        assert_eq!(
            *vault.read(cx).list().read(cx).delegate().filter(),
            Filter::Folder(Some(id.clone()))
        )
    });

    cx.update_window(handle, |_, window, cx| window.click("folder-rename", cx))
        .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("folder-rename-name", cx);
        window.press("ctrl-a", cx);
        window.input(&renamed, cx);
        window.press("enter", cx);
    })
    .unwrap();
    wait_until(cx, |cx| folder(cx, &renamed).is_some());

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("folder-delete", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("folder-delete", cx);
    })
    .unwrap();
    wait_until(cx, |cx| folder(cx, &renamed).is_none());
    wait_until(cx, |cx| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            true
        })
        .unwrap();
        cx.update(|cx| *vault.read(cx).list().read(cx).delegate().filter() == Filter::All)
    });
}

#[gpui_kit::test]
fn pinning_adds_quick_access_and_is_saved(cx: &mut TestAppContext) {
    use scout_vault_app::item_list::Filter;
    cx.update(gpui_kit::init);
    let dir = std::env::temp_dir().join(format!("scout-vault-ui-pins-{}", std::process::id()));
    let store = SettingsStore::at(dir.join("settings.json"));
    let (handle, app) = open_with_store(cx, &store, Some(unlocked_fixture()));
    let vault = cx.update(|cx| app.read(cx).vault_view().cloned().unwrap());

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("pinned-login-1").is_none());
        vault.update(cx, |v, cx| v.select_item("login-1", window, cx));
        window.render_frame(cx);
        window.click("item-pin", cx);
    })
    .unwrap();
    assert_eq!(store.load().pinned_items, ["login-1"], "fica salvo");

    // With another filter active, the shortcut goes back to "All" and opens the item.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("pinned-login-1").is_some());
        vault.update(cx, |v, cx| {
            v.set_filter(
                Filter::Kind(scout_vault_app::item_list::KindFilter::Card),
                window,
                cx,
            )
        });
        window.render_frame(cx);
        window.click("pinned-login-1", cx);
    })
    .unwrap();
    cx.update(|cx| {
        let v = vault.read(cx);
        assert_eq!(v.selected().unwrap().id, "login-1");
        assert_eq!(*v.list().read(cx).delegate().filter(), Filter::All);
    });

    // Unpinning removes it from the bar and from the file.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("item-pin", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("pinned-login-1").is_none());
    })
    .unwrap();
    assert!(store.load().pinned_items.is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

#[gpui_kit::test]
fn tray_commands_and_close_without_tray(cx: &mut TestAppContext) {
    let (handle, app) = open_app(cx, Some(unlocked_fixture()));
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |app, cx| {
            // Without a tray (tests), closing really closes.
            assert!(app.on_close_requested(window, cx));
            app.handle_tray(scout_vault_app::tray::Command::Show, window, cx);
            app.handle_tray(scout_vault_app::tray::Command::Lock, window, cx);
            assert_eq!(app.screen_kind(), ScreenKind::Locked);
        });
    })
    .unwrap();
    let texts = cx.update(|cx| app.read(cx).toasts().read(cx).texts());
    assert_eq!(texts, ["Cofre bloqueado"]);
}

fn temp_files(tag: &str, names: &[&str]) -> Vec<std::path::PathBuf> {
    let dir = std::env::temp_dir().join(format!("scout-vault-ui-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    names
        .iter()
        .map(|n| {
            let p = dir.join(n);
            std::fs::write(&p, format!("conteúdo de {n}")).unwrap();
            p
        })
        .collect()
}

#[gpui_kit::test]
fn editor_collects_files_and_keeps_them_across_kinds(cx: &mut TestAppContext) {
    let (handle, app) = open_app(cx, Some(unlocked_fixture()));
    let vault = cx.update(|cx| app.read(cx).vault_view().cloned().unwrap());
    let files = temp_files("editor", &["rg.pdf", "foto.png"]);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("vault-new", cx);
        window.render_frame(cx);
        assert!(
            window.try_find("editor-attach-zone").is_some(),
            "área de anexos visível"
        );
        let editor = vault.read(cx).editor().unwrap().clone();
        editor.update(cx, |e, cx| e.add_paths(&files, cx));
        window.render_frame(cx);
        assert_eq!(window.find("editor-file-1").label(), Some("foto.png"));
        window.click("editor-kind-note", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let editor = vault.read(cx).editor().unwrap().clone();
        assert_eq!(
            editor.read(cx).pending_files(),
            ["rg.pdf", "foto.png"],
            "trocar o tipo não perde"
        );
        window.click("editor-file-remove-0", cx);
    })
    .unwrap();
    cx.update(|cx| {
        let editor = vault.read(cx).editor().unwrap().clone();
        assert_eq!(editor.read(cx).pending_files(), ["foto.png"]);
    });
    let _ = std::fs::remove_dir_all(files[0].parent().unwrap());
}

/// Creates a note already with files and attaches more from the detail, against the local Vaultwarden.
#[gpui_kit::test]
#[ignore = "precisa de servidor; veja dev/test-account.env"]
fn note_with_files_through_the_ui(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let env = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("defina {k}"));
    let (url, email, password) = (
        env("SCOUT_VAULT_TEST_URL"),
        env("SCOUT_VAULT_TEST_EMAIL"),
        env("SCOUT_VAULT_TEST_PASSWORD"),
    );
    let (handle, app) = open_app(cx, None);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.within("login-server").click(2usize, cx);
        window.click("login-server-url", cx);
        window.input(&url, cx);
        window.click("login-email", cx);
        window.input(&email, cx);
        window.click("login-password", cx);
        window.input(&password, cx);
        window.click("login-submit", cx);
    })
    .unwrap();
    wait_until(cx, |cx| {
        cx.update(|cx| app.read(cx).screen_kind() == ScreenKind::Vault)
    });
    let vault = cx.update(|cx| app.read(cx).vault_view().cloned().unwrap());
    let name = format!("Anexo UI {}", std::process::id());
    let first = temp_files("nota", &["rg.pdf", "foto.png"]);
    let more = temp_files("nota-mais", &["contrato.txt", "recibo.txt"]);

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("vault-new", cx);
        window.render_frame(cx);
        window.click("editor-kind-note", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("editor-name", cx);
        window.input(&name, cx);
        let editor = vault.read(cx).editor().unwrap().clone();
        editor.update(cx, |e, cx| e.add_paths(&first, cx));
        window.click("editor-save", cx);
    })
    .unwrap();
    wait_until(cx, |cx| {
        cx.update(|cx| {
            vault
                .read(cx)
                .selected()
                .is_some_and(|i| i.name == name && i.attachments.len() == 2)
        })
    });

    // Two at once from the detail (like dragging several).
    cx.update(|cx| vault.update(cx, |v, cx| v.attach_paths(&more, cx)));
    wait_until(cx, |cx| {
        cx.update(|cx| {
            vault
                .read(cx)
                .selected()
                .is_some_and(|i| i.attachments.len() == 4)
        })
    });
    let names: Vec<String> = cx.update(|cx| {
        let mut n: Vec<String> = vault
            .read(cx)
            .selected()
            .unwrap()
            .attachments
            .iter()
            .map(|a| a.file_name.clone())
            .collect();
        n.sort();
        n
    });
    assert_eq!(names, ["contrato.txt", "foto.png", "recibo.txt", "rg.pdf"]);

    // Clean up: trash and delete permanently.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("item-trash", cx);
    })
    .unwrap();
    wait_until(cx, |cx| {
        cx.update(|cx| vault.read(cx).selected().is_some_and(|i| i.deleted))
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("item-delete-forever", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("item-delete-forever", cx);
    })
    .unwrap();
    wait_until(cx, |cx| cx.update(|cx| vault.read(cx).selected().is_none()));
    let _ = std::fs::remove_dir_all(first[0].parent().unwrap());
    let _ = std::fs::remove_dir_all(more[0].parent().unwrap());
}

#[gpui_kit::test]
fn org_items_choose_owner_and_collections(cx: &mut TestAppContext) {
    use scout_vault_app::item_list::Filter;
    let (handle, app) = open_app(cx, Some(unlocked_fixture()));
    let vault = cx.update(|cx| app.read(cx).vault_view().cloned().unwrap());
    let editor =
        |cx: &mut TestAppContext| cx.update(|cx| vault.read(cx).editor().cloned().unwrap());

    // Collections in the sidebar, below the organization; filter by one.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("filter-col-col-fin").is_some());
        assert!(window.try_find("filter-col-col-ro").is_some());
        window.click("filter-col-col-fin", cx);
    })
    .unwrap();
    let visible: Vec<String> = cx.update(|cx| {
        vault
            .read(cx)
            .list()
            .read(cx)
            .delegate()
            .visible_items()
            .map(|i| i.id.clone())
            .collect()
    });
    assert_eq!(visible, ["card-org"]);

    // New item from the collection: born in the organization and in it.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("vault-new", cx);
        window.render_frame(cx);
        assert!(
            window.try_find("editor-col-col-ro").is_none(),
            "só leitura não é opção"
        );
    })
    .unwrap();
    let e = editor(cx);
    cx.update(|cx| {
        assert_eq!(e.read(cx).owner(), Some("org-1"));
        assert_eq!(e.read(cx).collections(), ["col-fin"]);
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("editor-owner-personal", cx);
    })
    .unwrap();
    cx.update(|cx| assert_eq!(e.read(cx).owner(), None));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("editor-owner-org-1", cx);
    })
    .unwrap();
    cx.update(|cx| assert_eq!(e.read(cx).collections(), ["col-fin"]));
    // Doesn't save without a collection.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("editor-col-col-fin", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("editor-name", cx);
        window.input("Da empresa", cx);
        window.click("editor-save", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("editor-status").label(),
            Some("Escolha pelo menos uma coleção da organização.")
        );
        window.click("editor-cancel", cx);
    })
    .unwrap();

    // Existing organization item: fixed owner, editable collections.
    cx.update_window(handle, |_, window, cx| {
        vault.update(cx, |v, cx| {
            v.set_filter(Filter::All, window, cx);
            v.select_item("card-org", window, cx);
        });
        window.render_frame(cx);
        window.click("item-edit", cx);
        window.render_frame(cx);
        assert!(
            window.try_find("editor-owner-personal").is_none(),
            "dono não muda ao editar"
        );
        assert!(window.try_find("editor-col-col-fin").is_some());
    })
    .unwrap();
    let e = editor(cx);
    cx.update(|cx| assert_eq!(e.read(cx).owner(), Some("org-1")));
}

/// Creates an item in an organization through the interface, against the local Vaultwarden.
/// Needs the organization from `tools/dev_org.py`.
#[gpui_kit::test]
#[ignore = "precisa de servidor; veja dev/test-account.env"]
fn org_item_through_the_ui(cx: &mut TestAppContext) {
    use scout_vault_app::item_list::Filter;
    cx.executor().allow_parking();
    let env = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("defina {k}"));
    let (url, email, password) = (
        env("SCOUT_VAULT_TEST_URL"),
        env("SCOUT_VAULT_TEST_EMAIL"),
        env("SCOUT_VAULT_TEST_PASSWORD"),
    );
    let (handle, app) = open_app(cx, None);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.within("login-server").click(2usize, cx);
        window.click("login-server-url", cx);
        window.input(&url, cx);
        window.click("login-email", cx);
        window.input(&email, cx);
        window.click("login-password", cx);
        window.input(&password, cx);
        window.click("login-submit", cx);
    })
    .unwrap();
    wait_until(cx, |cx| {
        cx.update(|cx| app.read(cx).screen_kind() == ScreenKind::Vault)
    });
    let vault = cx.update(|cx| app.read(cx).vault_view().cloned().unwrap());
    let (org, ti) = cx.update(|cx| {
        let v = vault.read(cx).list().read(cx).delegate().vault();
        let org = v
            .organizations
            .iter()
            .find(|o| o.name == "Empresa de teste")
            .expect("rode tools/dev_org.py")
            .id
            .clone();
        let ti = v
            .collections
            .iter()
            .find(|c| c.organization_id == org && c.name == "TI")
            .unwrap()
            .id
            .clone();
        (org, ti)
    });
    let name = format!("Pela UI na empresa {}", std::process::id());

    cx.update_window(handle, |_, window, cx| {
        vault.update(cx, |v, cx| {
            v.set_filter(Filter::Collection(ti.clone()), window, cx)
        });
        window.render_frame(cx);
        window.click("vault-new", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("editor-name", cx);
        window.input(&name, cx);
        window.click("editor-password", cx);
        window.input("senha-da-empresa", cx);
        window.click("editor-save", cx);
    })
    .unwrap();
    wait_until(cx, |cx| {
        cx.update(|cx| vault.read(cx).selected().is_some_and(|i| i.name == name))
    });
    cx.update(|cx| {
        let item = vault.read(cx).selected().unwrap();
        assert_eq!(item.organization_id.as_deref(), Some(org.as_str()));
        assert_eq!(item.collection_ids, std::slice::from_ref(&ti));
    });

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("item-trash", cx);
    })
    .unwrap();
    wait_until(cx, |cx| {
        cx.update(|cx| vault.read(cx).selected().is_some_and(|i| i.deleted))
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("item-delete-forever", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("item-delete-forever", cx);
    })
    .unwrap();
    wait_until(cx, |cx| cx.update(|cx| vault.read(cx).selected().is_none()));
}

fn file_store(tag: &str) -> (std::path::PathBuf, SettingsStore) {
    let dir = std::env::temp_dir().join(format!("scout-vault-ui-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    (dir.clone(), SettingsStore::at(dir.join("settings.json")))
}

#[gpui_kit::test]
fn opens_without_password_for_n_days_until_password_changes(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    cx.update(gpui_kit::init);
    let (dir, store) = file_store("quick");
    let mut settings = store.load();
    settings.unlock_memory_days = 7;
    store.save(&settings);

    // First time: lock and unlock with the password (that stores the key).
    let (handle, app) = open_with_store(cx, &store, Some(unlocked_fixture()));
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |a, cx| a.lock(window, cx))
    })
    .unwrap();
    let (password, _) = fixture_password_and_email();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("unlock-password", cx);
        window.input(&password, cx);
        window.click("unlock-submit", cx);
    })
    .unwrap();
    wait_until(cx, |cx| {
        cx.update(|cx| app.read(cx).screen_kind() == ScreenKind::Vault)
    });

    let saved = scout_vault_app::saved_session::load(&store).expect("sessão salva");
    assert!(
        scout_vault_app::quick_unlock::open(&store, &saved).is_some(),
        "chave guardada não abriu"
    );
    // "Reopen the app": opens already unlocked, without a password.
    let (_, app2) = open_with_store(cx, &store, None);
    cx.update(|cx| assert_eq!(app2.read(cx).screen_kind(), ScreenKind::Vault));

    // Master password changed elsewhere: the profile's encrypted user key changes on sync.
    let mut session = scout_vault_app::saved_session::load(&store).unwrap();
    let mut sync = (*session.sync).clone();
    sync.profile.key = Some("2.outra|chave|cifrada".into());
    session.sync = Arc::new(sync);
    assert!(scout_vault_app::quick_unlock::password_changed(
        &store, &session
    ));
    let (_, app3) = open_with_store(cx, &store, None);
    cx.update(|cx| assert_eq!(app3.read(cx).screen_kind(), ScreenKind::Locked));
    let _ = std::fs::remove_dir_all(dir);
}

#[gpui_kit::test]
fn full_login_is_required_after_n_days(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let (dir, store) = file_store("relogin");
    let mut session = unlocked_fixture().session;
    session.logged_in_at = scout_vault_app::flow::now_secs() - 31 * 86_400;
    scout_vault_app::saved_session::save(&store, &session);
    // Default: 30 days. Past 30 → asks for login.
    let (_, app) = open_with_store(cx, &store, None);
    cx.update(|cx| assert_eq!(app.read(cx).screen_kind(), ScreenKind::Login));
    assert!(
        scout_vault_app::saved_session::load(&store).is_none(),
        "sessão esquecida"
    );

    // With "Never", the same old session still works.
    let mut settings = store.load();
    settings.relogin_days = 0;
    store.save(&settings);
    scout_vault_app::saved_session::save(&store, &session);
    let (_, app) = open_with_store(cx, &store, None);
    cx.update(|cx| assert_eq!(app.read(cx).screen_kind(), ScreenKind::Locked));
    let _ = std::fs::remove_dir_all(dir);
}

#[gpui_kit::test]
fn auto_lock_follows_the_setting(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let (dir, store) = file_store("autolock");
    let mut settings = store.load();
    settings.auto_lock_minutes = 0;
    store.save(&settings);
    let (handle, app) = open_with_store(cx, &store, Some(unlocked_fixture()));
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |app, cx| {
            // "Never": not even after a day.
            app.lock_if_idle(Instant::now() + Duration::from_secs(86_400), window, cx);
            assert_eq!(app.screen_kind(), ScreenKind::Vault);
        })
    })
    .unwrap();
    let mut settings = store.load();
    settings.auto_lock_minutes = 1;
    store.save(&settings);
    let (handle, app) = open_with_store(cx, &store, Some(unlocked_fixture()));
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |app, cx| {
            app.lock_if_idle(Instant::now() + Duration::from_secs(30), window, cx);
            assert_eq!(app.screen_kind(), ScreenKind::Vault);
            app.lock_if_idle(Instant::now() + Duration::from_secs(61), window, cx);
            assert_eq!(app.screen_kind(), ScreenKind::Locked);
        })
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(dir);
}

/// Clicking an item in the list shows it, even with the form open.
#[gpui_kit::test]
fn clicking_an_item_leaves_the_form(cx: &mut TestAppContext) {
    let (handle, app) = open_app(cx, Some(unlocked_fixture()));
    let vault = cx.update(|cx| app.read(cx).vault_view().cloned().unwrap());
    let selected =
        |cx: &mut TestAppContext| cx.update(|cx| vault.read(cx).selected().map(|i| i.id.clone()));
    // New item form open → click an item.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("vault-new", cx);
        window.render_frame(cx);
        assert!(window.try_find("editor-name").is_some());
        window.click("item-drag-login-1", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("editor-name").is_none());
        assert!(window.try_find("item-edit").is_some());
    })
    .unwrap();
    assert!(cx.update(|cx| vault.read(cx).editor().is_none()));
    assert_eq!(selected(cx).as_deref(), Some("login-1"));

    // Editing an item → click another one: shows the other one.
    cx.update_window(handle, |_, window, cx| {
        window.click("item-edit", cx);
        window.render_frame(cx);
        assert!(window.try_find("editor-name").is_some());
        window.click("item-drag-identity-1", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("editor-name").is_none());
    })
    .unwrap();
    assert_eq!(selected(cx).as_deref(), Some("identity-1"));
}

/// Dragging to a folder with the form open doesn't lose the form.
#[gpui_kit::test]
fn dragging_into_a_folder_waits_for_the_form(cx: &mut TestAppContext) {
    let (handle, app) = open_app(cx, Some(unlocked_fixture()));
    let vault = cx.update(|cx| app.read(cx).vault_view().cloned().unwrap());
    let texts = |cx: &mut TestAppContext| cx.update(|cx| app.read(cx).toasts().read(cx).texts());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("vault-new", cx);
        window.render_frame(cx);
        window.drag_to("item-drag-identity-1", "filter-folder-folder-1", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    assert!(cx.update(|cx| vault.read(cx).editor().is_some()));
    assert_eq!(
        texts(cx),
        ["Salve ou cancele a edição antes de mover itens."]
    );
}

/// Drags an item into a folder and a folder into another one, against the
/// local Vaultwarden.
#[gpui_kit::test]
#[ignore = "precisa de servidor; veja dev/test-account.env"]
fn drag_and_drop_through_the_ui(cx: &mut TestAppContext) {
    use scout_vault_app::item_list::Filter;
    cx.executor().allow_parking();
    let env = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("defina {k}"));
    let (url, email, password) = (
        env("SCOUT_VAULT_TEST_URL"),
        env("SCOUT_VAULT_TEST_EMAIL"),
        env("SCOUT_VAULT_TEST_PASSWORD"),
    );
    let (handle, app) = open_app(cx, None);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.within("login-server").click(2usize, cx);
        window.click("login-server-url", cx);
        window.input(&url, cx);
        window.click("login-email", cx);
        window.input(&email, cx);
        window.click("login-password", cx);
        window.input(&password, cx);
        window.click("login-submit", cx);
    })
    .unwrap();
    wait_until(cx, |cx| {
        cx.update(|cx| app.read(cx).screen_kind() == ScreenKind::Vault)
    });
    let vault = cx.update(|cx| app.read(cx).vault_view().cloned().unwrap());
    let folders = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            vault
                .read(cx)
                .list()
                .read(cx)
                .delegate()
                .vault()
                .folders
                .iter()
                .map(|f| (f.id.clone(), f.name.clone()))
                .collect::<Vec<_>>()
        })
    };
    let item_state = |cx: &mut TestAppContext, name: &str| {
        cx.update(|cx| {
            vault
                .read(cx)
                .list()
                .read(cx)
                .delegate()
                .vault()
                .items
                .iter()
                .find(|i| i.name == name)
                .map(|i| (i.id.clone(), i.folder_id.clone(), i.deleted))
        })
    };
    let idle = |cx: &mut TestAppContext| {
        wait_until(cx, |cx| {
            cx.update_window(handle, |_, window, cx| window.render_frame(cx))
                .unwrap();
            cx.update(|cx| !vault.read(cx).is_busy())
        })
    };
    let folder_id = |cx: &mut TestAppContext, name: &str| {
        folders(cx)
            .into_iter()
            .find(|(_, n)| n == name)
            .map(|(id, _)| id)
    };
    let pid = std::process::id();
    let (outer, inner, item_name) = (
        format!("Arrasto A {pid}"),
        format!("Arrasto B {pid}"),
        format!("Arrastado {pid}"),
    );
    for name in [&outer, &inner] {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("folder-new", cx);
        })
        .unwrap();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("folder-new-name", cx);
            window.input(name, cx);
            window.press("enter", cx);
        })
        .unwrap();
        wait_until(cx, |cx| folder_id(cx, name).is_some());
        idle(cx);
    }
    let (outer_id, inner_id) = (
        folder_id(cx, &outer).unwrap(),
        folder_id(cx, &inner).unwrap(),
    );

    // An item with no folder, to drag.
    cx.update_window(handle, |_, window, cx| {
        vault.update(cx, |v, cx| v.set_filter(Filter::Folder(None), window, cx));
        window.render_frame(cx);
        window.click("vault-new", cx);
        window.render_frame(cx);
        window.click("editor-name", cx);
        window.input(&item_name, cx);
        window.click("editor-save", cx);
    })
    .unwrap();
    wait_until(cx, |cx| {
        item_state(cx, &item_name).is_some() && cx.update(|cx| vault.read(cx).editor().is_none())
    });
    idle(cx);
    let item_id = item_state(cx, &item_name).unwrap().0;

    // Item → folder B.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.drag_to(
            gpui_kit::SharedString::from(format!("item-drag-{item_id}")),
            gpui_kit::SharedString::from(format!("filter-folder-{inner_id}")),
            cx,
        );
    })
    .unwrap();
    wait_until(cx, |cx| {
        item_state(cx, &item_name).and_then(|s| s.1) == Some(inner_id.clone())
    });
    idle(cx);

    // Folder B → inside A: becomes "A/B" and shows up indented, with just the name.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.drag_to(
            gpui_kit::SharedString::from(format!("filter-folder-{inner_id}")),
            gpui_kit::SharedString::from(format!("filter-folder-{outer_id}")),
            cx,
        );
    })
    .unwrap();
    let nested = format!("{outer}/{inner}");
    wait_until(cx, |cx| folder_id(cx, &nested) == Some(inner_id.clone()));
    idle(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window
                .find(gpui_kit::SharedString::from(format!(
                    "filter-folder-{inner_id}"
                )))
                .label(),
            Some(inner.as_str())
        );
        // And back to the top level, dropping on the "Folders" heading.
        window.drag_to(
            gpui_kit::SharedString::from(format!("filter-folder-{inner_id}")),
            "folders-header",
            cx,
        );
    })
    .unwrap();
    wait_until(cx, |cx| folder_id(cx, &inner) == Some(inner_id.clone()));
    idle(cx);

    // Item → No folder.
    cx.update_window(handle, |_, window, cx| {
        vault.update(cx, |v, cx| {
            v.set_filter(Filter::Folder(Some(inner_id.clone())), window, cx)
        });
        window.render_frame(cx);
        window.drag_to(
            gpui_kit::SharedString::from(format!("item-drag-{item_id}")),
            "filter-no-folder",
            cx,
        );
    })
    .unwrap();
    wait_until(cx, |cx| {
        item_state(cx, &item_name).is_some_and(|s| s.1.is_none())
    });
    idle(cx);

    // Clean up: deletes the folders and the item.
    for id in [inner_id, outer_id] {
        cx.update_window(handle, |_, window, cx| {
            vault.update(cx, |v, cx| {
                v.set_filter(Filter::Folder(Some(id.clone())), window, cx);
                v.delete_folder(cx);
                v.delete_folder(cx);
            });
        })
        .unwrap();
        wait_until(cx, |cx| folders(cx).iter().all(|(f, _)| *f != id));
        idle(cx);
    }
    cx.update_window(handle, |_, window, cx| {
        vault.update(cx, |v, cx| {
            v.set_filter(Filter::All, window, cx);
            v.select_item(&item_id, window, cx);
        });
        window.render_frame(cx);
        window.click("item-trash", cx);
    })
    .unwrap();
    wait_until(cx, |cx| item_state(cx, &item_name).is_some_and(|s| s.2));
    idle(cx);
    cx.update_window(handle, |_, window, cx| {
        vault.update(cx, |v, cx| {
            v.set_filter(Filter::Trash, window, cx);
            v.select_item(&item_id, window, cx);
        });
        window.render_frame(cx);
        window.click("item-delete-forever", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("item-delete-forever", cx);
    })
    .unwrap();
    wait_until(cx, |cx| item_state(cx, &item_name).is_none());
}

/// Tray menu: reflects locking and quick access; copy, generate and
/// sync work without opening the item.
#[gpui_kit::test]
fn tray_menu_copies_generates_and_follows_lock(cx: &mut TestAppContext) {
    use scout_vault_app::tray::{Command, PinnedEntry};
    use scout_vault_app::vault_view::CopyShortcut;
    cx.executor().allow_parking();
    let (handle, app) = open_app(cx, Some(unlocked_fixture()));
    let vault = cx.update(|cx| app.read(cx).vault_view().cloned().unwrap());
    let texts = |cx: &mut TestAppContext| cx.update(|cx| app.read(cx).toasts().read(cx).texts());
    let menu = |cx: &mut TestAppContext| cx.update(|cx| app.read(cx).tray_menu_state(cx));

    // Unlocked, nothing pinned.
    let state = menu(cx);
    assert!(state.unlocked);
    assert!(state.pinned.is_empty());

    // Pin a login: shows up with username and password.
    cx.update(|cx| vault.update(cx, |v, _| v.set_pinned(vec!["login-1".into()])));
    assert_eq!(
        menu(cx).pinned,
        [PinnedEntry {
            id: "login-1".into(),
            name: "Meu Banco".into(),
            has_username: true,
            has_password: true,
            has_totp: true,
            icon: Some(gpui_kit::assets::IconName::Globe.into()),
        }]
    );

    // Copy the password from the menu, without opening the item.
    let copy = |what| Command::Copy {
        item_id: "login-1".into(),
        what,
    };
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |a, cx| {
            a.handle_tray(copy(CopyShortcut::Password), window, cx)
        });
    })
    .unwrap();
    assert_eq!(texts(cx), ["Copiado: Senha · some em 30 s"]);
    assert!(cx.update(|cx| vault.read(cx).selected().is_none()));
    // A field the item doesn't have.
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |a, cx| {
            let missing = Command::Copy {
                item_id: "identity-1".into(),
                what: CopyShortcut::Password,
            };
            a.handle_tray(missing, window, cx)
        });
    })
    .unwrap();
    assert!(texts(cx).contains(&"Este item não tem senha.".to_owned()));

    // Generate and copy.
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |a, cx| {
            a.handle_tray(Command::GeneratePassword, window, cx)
        });
    })
    .unwrap();
    assert!(texts(cx).contains(&"Senha gerada e copiada · some em 30 s".to_owned()));

    // Sync starts right away.
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |a, cx| a.handle_tray(Command::Sync, window, cx));
    })
    .unwrap();
    assert!(cx.update(|cx| vault.read(cx).is_busy()));
    // Wait for the attempt to finish (no server, it fails) before going on.
    wait_until(cx, |cx| cx.update(|cx| !vault.read(cx).is_busy()));

    // Locked: menu without quick access, and copying asks to unlock.
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |a, cx| a.handle_tray(Command::Lock, window, cx));
    })
    .unwrap();
    let state = menu(cx);
    assert!(!state.unlocked);
    assert!(state.pinned.is_empty());
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |a, cx| {
            a.handle_tray(copy(CopyShortcut::Password), window, cx)
        });
    })
    .unwrap();
    assert!(texts(cx).contains(&"Desbloqueie o cofre para copiar.".to_owned()));
}

/// The tray menu is an app window: opens at the cursor, each row does the
/// action and closes the menu; `Esc` closes; when locked, it only offers unlock.
#[gpui_kit::test]
fn tray_popup_menu_runs_actions_and_closes(cx: &mut TestAppContext) {
    use scout_vault_app::tray::Command;
    cx.executor().allow_parking();
    let (handle, app) = open_app(cx, Some(unlocked_fixture()));
    let vault = cx.update(|cx| app.read(cx).vault_view().cloned().unwrap());
    let texts = |cx: &mut TestAppContext| cx.update(|cx| app.read(cx).toasts().read(cx).texts());
    cx.update(|cx| vault.update(cx, |v, _| v.set_pinned(vec!["login-1".into()])));
    let open = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |a, cx| {
                a.handle_tray(Command::OpenMenu { x: 1800, y: 1060 }, window, cx)
            })
        })
        .unwrap();
        cx.run_until_parked();
        cx.update(|cx| app.read(cx).tray_menu())
            .expect("menu aberto")
    };
    let is_open = |cx: &mut TestAppContext, menu: AnyWindowHandle| {
        cx.update_window(menu, |_, _, _| ()).is_ok()
    };

    // Unlocked: quick access with buttons for each of the item's fields.
    let (menu, view) = open(cx);
    assert!(cx.update(|cx| view.read(cx).state().unlocked));
    cx.update_window(menu, |_, window, cx| {
        window.render_frame(cx);
        for id in [
            "tray-pin-login-1",
            "tray-pin-login-1-user",
            "tray-pin-login-1-pass",
            "tray-pin-login-1-code",
            "tray-menu-open",
            "tray-menu-generate",
            "tray-menu-sync",
            "tray-menu-lock",
            "tray-menu-quit",
        ] {
            assert!(window.try_find(id).is_some(), "falta {id}");
        }
        assert!(window.try_find("tray-menu-unlock").is_none());
        window.click("tray-pin-login-1-pass", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(texts(cx), ["Copiado: Senha · some em 30 s"]);
    assert!(!is_open(cx, menu), "escolher fecha o menu");

    // Opening again replaces the menu; Esc closes.
    let (menu, _) = open(cx);
    cx.update_window(menu, |_, window, cx| {
        window.render_frame(cx);
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(!is_open(cx, menu));

    // Generate a password from the menu.
    let (menu, _) = open(cx);
    cx.update_window(menu, |_, window, cx| {
        window.render_frame(cx);
        window.click("tray-menu-generate", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(texts(cx).contains(&"Senha gerada e copiada · some em 30 s".to_owned()));

    // Lock from the menu; the next menu only offers unlock and generate.
    let (menu, _) = open(cx);
    cx.update_window(menu, |_, window, cx| {
        window.render_frame(cx);
        window.click("tray-menu-lock", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        cx.update(|cx| app.read(cx).screen_kind()),
        ScreenKind::Locked
    );
    let (menu, view) = open(cx);
    assert!(!cx.update(|cx| view.read(cx).state().unlocked));
    cx.update_window(menu, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("tray-menu-unlock").is_some());
        assert!(window.try_find("tray-menu-generate").is_some());
        assert!(window.try_find("tray-pin-login-1").is_none());
        assert!(window.try_find("tray-menu-lock").is_none());
    })
    .unwrap();
}

/// The menu window has the exact height of the rows (no extra space and no clipping).
#[test]
fn tray_menu_height_follows_its_rows() {
    use scout_vault_app::tray::{MenuState, PinnedEntry};
    use scout_vault_app::tray_menu::window_size;
    let locked = window_size(&MenuState::default());
    let empty = window_size(&MenuState {
        unlocked: true,
        pinned: Vec::new(),
    });
    let pin = |n: usize| MenuState {
        unlocked: true,
        pinned: (0..n)
            .map(|i| PinnedEntry {
                id: i.to_string(),
                name: format!("Item {i}"),
                ..Default::default()
            })
            .collect(),
    };
    assert!(locked.height < empty.height);
    assert!(window_size(&pin(2)).height > window_size(&pin(1)).height);
    // Many pinned items scroll inside the menu instead of growing forever.
    assert_eq!(window_size(&pin(6)).height, window_size(&pin(10)).height);
    assert_eq!(locked.width, empty.width);
}

/// Locking Windows (or sleeping) locks the vault — unless the
/// option is off.
#[gpui_kit::test]
fn locks_with_windows_unless_turned_off(cx: &mut TestAppContext) {
    use scout_vault_app::windows_events::OsEvent;
    cx.update(gpui_kit::init);
    let (dir, store) = file_store("lockwin");
    let mut settings = store.load();
    assert!(settings.lock_with_windows, "ligado por padrão");
    // Off: nothing changes.
    settings.lock_with_windows = false;
    store.save(&settings);
    let (handle, app) = open_with_store(cx, &store, Some(unlocked_fixture()));
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |app, cx| {
            app.handle_os_event(OsEvent::SessionLocked, window, cx);
            assert_eq!(app.screen_kind(), ScreenKind::Vault);
        })
    })
    .unwrap();
    // On: locks when Windows locks…
    settings.lock_with_windows = true;
    store.save(&settings);
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |app, cx| {
            app.handle_os_event(OsEvent::SessionLocked, window, cx);
            assert_eq!(app.screen_kind(), ScreenKind::Locked);
        })
    })
    .unwrap();
    // …and on sleep.
    let (handle, app) = open_with_store(cx, &store, Some(unlocked_fixture()));
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |app, cx| {
            app.handle_os_event(OsEvent::Suspending, window, cx);
            assert_eq!(app.screen_kind(), ScreenKind::Locked);
        })
    })
    .unwrap();
    // The option shows up in settings.
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |app, cx| app.open_settings(window, cx));
        window.render_frame(cx);
        assert!(window.try_find("lock-with-windows").is_some());
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(dir);
}

/// Item with "ask for the master password": show, copy and edit wait for the password;
/// a wrong password doesn't release it; switching items asks again.
#[gpui_kit::test]
fn reprompt_guards_reveal_copy_and_edit(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (handle, app) = open_app(cx, Some(unlocked_fixture()));
    let vault = cx.update(|cx| app.read(cx).vault_view().cloned().unwrap());
    let texts = |cx: &mut TestAppContext| cx.update(|cx| app.read(cx).toasts().read(cx).texts());
    let (password, _) = fixture_password_and_email();
    cx.update_window(handle, |_, window, cx| {
        vault.update(cx, |v, cx| v.select_item("login-item-key", window, cx));
        window.render_frame(cx);
        assert!(window.try_find("reprompt-card").is_some());
        // Show the password: opens the prompt instead of revealing.
        window.click("reveal-password", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("reprompt-password").is_some());
        assert_eq!(window.find("value-password").label(), Some("••••••••"));
        // Wrong password.
        window.click("reprompt-password", cx);
        window.input("errada", cx);
        window.press("enter", cx);
    })
    .unwrap();
    wait_until(cx, |cx| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.try_find("reprompt-error").is_some()
        })
        .unwrap()
    });
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(
            window.find("reprompt-error").label(),
            Some("Senha do cofre incorreta.")
        );
        assert_eq!(window.find("value-password").label(), Some("••••••••"));
        // Now the right one: reveals what had been requested.
        window.click("reprompt-password", cx);
        window.input(&password, cx);
        window.click("reprompt-confirm", cx);
    })
    .unwrap();
    wait_until(cx, |cx| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.find("value-password").label() == Some("item-key-pw")
        })
        .unwrap()
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("reprompt-ok").is_some());
        // Released: copying goes straight through.
        window.click("copy-password", cx);
    })
    .unwrap();
    assert!(texts(cx).contains(&"Copiado: Senha · some em 30 s".to_owned()));

    // Another item and back: asks again; editing waits for the password.
    cx.update_window(handle, |_, window, cx| {
        vault.update(cx, |v, cx| {
            v.select_item("login-1", window, cx);
            v.select_item("login-item-key", window, cx);
        });
        window.render_frame(cx);
        assert!(window.try_find("reprompt-card").is_some());
        window.click("item-edit", cx);
    })
    .unwrap();
    assert!(cx.update(|cx| vault.read(cx).editor().is_none()));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("reprompt-password", cx);
        window.input(&password, cx);
        window.press("enter", cx);
    })
    .unwrap();
    wait_until(cx, |cx| cx.update(|cx| vault.read(cx).editor().is_some()));
    // The form shows the option checked.
    let draft = cx.update(|cx| vault.read(cx).editor().unwrap().read(cx).to_item(cx));
    assert!(draft.reprompt);

    // Items without the option don't ask for anything.
    cx.update_window(handle, |_, window, cx| {
        window.click("editor-cancel", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        vault.update(cx, |v, cx| v.select_item("login-1", window, cx));
        window.render_frame(cx);
        assert!(window.try_find("reprompt-card").is_none());
        window.click("reveal-password", cx);
        window.render_frame(cx);
        assert_eq!(window.find("value-password").label(), Some("s3nh@-ção"));
    })
    .unwrap();
}

/// Through the tray and the browser, the item that asks for the master password doesn't go out without it.
#[gpui_kit::test]
fn reprompt_guards_tray_copy_and_browser_fill(cx: &mut TestAppContext) {
    use scout_vault_app::tray::Command;
    use scout_vault_app::vault_view::CopyShortcut;
    cx.executor().allow_parking();
    let (handle, app) = open_app(cx, Some(unlocked_fixture()));
    let vault = cx.update(|cx| app.read(cx).vault_view().cloned().unwrap());
    let texts = |cx: &mut TestAppContext| cx.update(|cx| app.read(cx).toasts().read(cx).texts());
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |a, cx| {
            a.handle_tray(
                Command::Copy {
                    item_id: "login-item-key".into(),
                    what: CopyShortcut::Password,
                },
                window,
                cx,
            )
        });
    })
    .unwrap();
    // Didn't copy: opened the item with the prompt.
    assert!(texts(cx).is_empty());
    cx.update(|cx| {
        assert_eq!(
            vault.read(cx).selected().map(|i| i.id.as_str()),
            Some("login-item-key")
        );
        assert!(vault.read(cx).reprompt_open());
    });
    let (password, _) = fixture_password_and_email();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("reprompt-password", cx);
        window.input(&password, cx);
        window.press("enter", cx);
    })
    .unwrap();
    // Confirmed: the requested copy happens.
    wait_until(cx, |cx| {
        texts(cx).contains(&"Copiado: Senha · some em 30 s".to_owned())
    });

    // Browser: refuses to fill.
    let items = cx.update(|cx| {
        vault
            .read(cx)
            .list()
            .read(cx)
            .delegate()
            .vault()
            .items
            .clone()
    });
    let reply = scout_vault_app::browser::handle(
        &scout_vault_app::browser::Request::Fill {
            id: "login-item-key".into(),
            url: "https://qualquer.example".into(),
        },
        Some(&items),
    );
    assert!(reply["error"]
        .as_str()
        .is_some_and(|e| e.contains("pede a senha do cofre")));
    assert!(reply.get("password").is_none());
}

/// Windows Hello (with a fake signer in place of Windows): turning it on with the
/// vault open stores the encrypted key; the button opens the vault; another Hello
/// key or canceling doesn't open it; signing out deletes it.
#[gpui_kit::test]
fn windows_hello_unlocks_and_is_forgotten(cx: &mut TestAppContext) {
    use scout_vault_app::hello;
    cx.executor().allow_parking();
    cx.update(gpui_kit::init);
    cx.update(|cx| hello::install(cx, hello::fake_signer(b"tpm-1")));
    let (dir, store) = file_store("hello");
    let (handle, app) = open_with_store(cx, &store, Some(unlocked_fixture()));
    let texts = |cx: &mut TestAppContext| cx.update(|cx| app.read(cx).toasts().read(cx).texts());
    let session = cx.update(|cx| {
        app.read(cx)
            .vault_view()
            .unwrap()
            .read(cx)
            .session()
            .clone()
    });

    // Turn it on in settings (with the vault open, it asks for Hello right away).
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |a, cx| a.open_settings(window, cx));
        window.render_frame(cx);
        assert!(window.try_find("windows-hello").is_some());
        let view = app.read(cx).settings_view().cloned().unwrap();
        view.update(cx, |v, cx| v.set_windows_hello(true, cx));
    })
    .unwrap();
    wait_until(cx, |cx| {
        texts(cx).contains(&"Windows Hello ligado".to_owned())
    });
    assert!(store.load().windows_hello);
    assert!(hello::is_enrolled(&store, &session));

    // Lock: "Use Windows Hello" shows up, and it opens the vault.
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |a, cx| {
            a.close_settings(cx);
            a.lock(window, cx)
        });
        window.render_frame(cx);
        window.click("unlock-hello", cx);
    })
    .unwrap();
    wait_until(cx, |cx| {
        cx.update(|cx| app.read(cx).screen_kind() == ScreenKind::Vault)
    });

    // Another Hello key (e.g. another Windows user): doesn't open.
    cx.update(|cx| hello::install(cx, hello::fake_signer(b"tpm-2")));
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |a, cx| a.lock(window, cx));
        window.render_frame(cx);
        window.click("unlock-hello", cx);
    })
    .unwrap();
    let unlock = |cx: &mut TestAppContext| {
        cx.update(|cx| match app.read(cx).unlock_view() {
            Some(v) => v.read(cx).error().map(|e| e.to_string()),
            None => None,
        })
    };
    wait_until(cx, |cx| unlock(cx).is_some());
    assert!(unlock(cx).unwrap().contains("ligue a opção de novo"));
    assert_eq!(
        cx.update(|cx| app.read(cx).screen_kind()),
        ScreenKind::Locked
    );

    // Cancel in Hello: explains and stays locked.
    cx.update(|cx| hello::install(cx, hello::canceling_signer()));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("unlock-hello", cx);
    })
    .unwrap();
    wait_until(cx, |cx| unlock(cx).is_some_and(|e| e.contains("cancelado")));

    // The master password changed on the server: the Hello key is discarded.
    let mut changed = session.clone();
    let mut sync = (*changed.sync).clone();
    sync.profile.key = Some("2.outra|chave|qualquer".into());
    changed.sync = std::sync::Arc::new(sync);
    assert!(hello::password_changed(&store, &changed));
    assert!(!hello::is_enrolled(&store, &session));

    // Master password typed with the option still on: sets it up again on its own.
    cx.update(|cx| hello::install(cx, hello::fake_signer(b"tpm-1")));
    let (password, _) = fixture_password_and_email();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("unlock-password", cx);
        window.input(&password, cx);
        window.click("unlock-submit", cx);
    })
    .unwrap();
    wait_until(cx, |_| hello::is_enrolled(&store, &session));
    // Signing out deletes it.
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |a, cx| a.lock(window, cx));
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("unlock-logout", cx);
    })
    .unwrap();
    assert!(!hello::is_enrolled(&store, &session));
    let _ = std::fs::remove_dir_all(dir);
}

/// No Windows Hello on the computer: turning the option on explains and turns it back off.
#[gpui_kit::test]
fn windows_hello_without_support_turns_back_off(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let (dir, store) = file_store("hello-off");
    let (handle, app) = open_with_store(cx, &store, Some(unlocked_fixture()));
    let texts = |cx: &mut TestAppContext| cx.update(|cx| app.read(cx).toasts().read(cx).texts());
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |a, cx| a.open_settings(window, cx));
        let view = app.read(cx).settings_view().cloned().unwrap();
        view.update(cx, |v, cx| v.set_windows_hello(true, cx));
    })
    .unwrap();
    cx.run_until_parked();
    assert!(!store.load().windows_hello);
    assert!(texts(cx).iter().any(|t| t.contains("não está configurado")));
    let _ = std::fs::remove_dir_all(dir);
}

/// Background sync: only syncs when the server revision changes, and
/// doesn't disturb an open form nor runs with the option off.
#[gpui_kit::test]
fn background_sync_only_when_the_vault_changed(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (handle, app) = open_app(cx, Some(unlocked_fixture()));
    let vault = cx.update(|cx| app.read(cx).vault_view().cloned().unwrap());
    let busy = |cx: &mut TestAppContext| cx.update(|cx| vault.read(cx).is_busy());
    let revision = |cx: &mut TestAppContext, date: i64| {
        cx.update(|cx| vault.update(cx, |v, cx| v.on_revision(date, cx)))
    };
    // First answer: becomes the baseline, no sync.
    revision(cx, 100);
    assert!(!busy(cx));
    assert_eq!(cx.update(|cx| vault.read(cx).last_revision()), Some(100));
    // Same revision: nothing.
    revision(cx, 100);
    assert!(!busy(cx));
    // Form open: waits (doesn't sync underneath the edit).
    cx.update_window(handle, |_, window, cx| {
        vault.update(cx, |v, cx| v.new_item(window, cx));
    })
    .unwrap();
    revision(cx, 200);
    assert!(!busy(cx));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("editor-cancel", cx);
    })
    .unwrap();
    // Option off: nothing.
    cx.update(|cx| vault.update(cx, |v, _| v.set_background_sync(false)));
    revision(cx, 200);
    assert!(!busy(cx));
    // On and changed: syncs.
    cx.update(|cx| vault.update(cx, |v, _| v.set_background_sync(true)));
    revision(cx, 200);
    assert!(busy(cx));
    // (the fixture's server doesn't exist: the sync fails and the baseline stays the old one)
    wait_until(cx, |cx| !busy(cx));
    assert_eq!(cx.update(|cx| vault.read(cx).last_revision()), Some(100));
    // The option shows up in settings and starts on.
    assert!(cx.update(|cx| app.read(cx).store().load().background_sync));
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |a, cx| a.open_settings(window, cx));
        window.render_frame(cx);
        assert!(window.try_find("background-sync").is_some());
    })
    .unwrap();
}

/// Against the local Vaultwarden: a folder created "on another device" shows up
/// on its own after the revision check.
#[gpui_kit::test]
#[ignore = "precisa de servidor; veja dev/test-account.env"]
fn background_sync_picks_up_changes_from_elsewhere(cx: &mut TestAppContext) {
    use scout_vault_core::api::{Client, DeviceInfo, ServerUrls};
    use scout_vault_core::crypto::MasterKey;
    use scout_vault_core::vault::Keys;
    cx.executor().allow_parking();
    let env = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("defina {k}"));
    let (url, email, password) = (
        env("SCOUT_VAULT_TEST_URL"),
        env("SCOUT_VAULT_TEST_EMAIL"),
        env("SCOUT_VAULT_TEST_PASSWORD"),
    );
    let (handle, app) = open_app(cx, None);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.within("login-server").click(2usize, cx);
        window.click("login-server-url", cx);
        window.input(&url, cx);
        window.click("login-email", cx);
        window.input(&email, cx);
        window.click("login-password", cx);
        window.input(&password, cx);
        window.click("login-submit", cx);
    })
    .unwrap();
    wait_until(cx, |cx| {
        cx.update(|cx| app.read(cx).screen_kind() == ScreenKind::Vault)
    });
    let vault = cx.update(|cx| app.read(cx).vault_view().cloned().unwrap());
    let texts = |cx: &mut TestAppContext| cx.update(|cx| app.read(cx).toasts().read(cx).texts());
    // First check: baseline.
    cx.update(|cx| vault.update(cx, |v, cx| v.check_now(cx)));
    wait_until(cx, |cx| {
        cx.update(|cx| vault.read(cx).last_revision().is_some())
    });

    // "Another device" creates a folder directly on the server.
    let name = format!("Pasta UI {} remota", std::process::id());
    let (client_name, folder_id) = {
        let name = name.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let client = Client::new(
                    ServerUrls::self_hosted(&url),
                    DeviceInfo::new("Scout Vault (teste, outro aparelho)"),
                )
                .unwrap();
                let kdf = client.prelogin(&email).await.unwrap();
                let mk = MasterKey::derive(&password, &email, kdf).unwrap();
                let token = client
                    .login_password(&email, &mk.server_hash(&password), None, None)
                    .await
                    .unwrap()
                    .access_token;
                let profile = client.sync(&token).await.unwrap().profile;
                let keys = Keys::unlock(&mk, &profile).unwrap();
                let enc = scout_vault_core::edit::encrypt_folder_name(&name, &keys).unwrap();
                let folder = client.create_folder(&token, &enc).await.unwrap();
                (name, folder.id)
            })
        })
        .join()
        .unwrap()
    };

    // The check notices and syncs on its own.
    cx.update(|cx| vault.update(cx, |v, cx| v.check_now(cx)));
    let has_folder = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            vault
                .read(cx)
                .list()
                .read(cx)
                .delegate()
                .vault()
                .folders
                .iter()
                .any(|f| f.id == folder_id && f.name == client_name)
        })
    };
    wait_until(cx, has_folder);
    assert!(texts(cx).contains(&"Cofre atualizado com mudanças de outro aparelho".to_owned()));

    // Clean up.
    wait_until(cx, |cx| cx.update(|cx| !vault.read(cx).is_busy()));
    cx.update_window(handle, |_, window, cx| {
        vault.update(cx, |v, cx| {
            v.set_filter(
                scout_vault_app::item_list::Filter::Folder(Some(folder_id.clone())),
                window,
                cx,
            );
            v.delete_folder(cx);
            v.delete_folder(cx);
        });
    })
    .unwrap();
    wait_until(cx, |cx| !has_folder(cx));
}

/// Saving logins from the browser, without a server: `check` answers without a password and without
/// bothering when locked; `save` refuses with a form open and, with the
/// vault locked, waits for unlock.
#[gpui_kit::test]
fn browser_save_checks_waits_for_unlock_and_spares_the_form(cx: &mut TestAppContext) {
    use scout_vault_app::browser::Request;
    use serde_json::json;
    cx.executor().allow_parking();
    cx.update(gpui_kit::init);
    let (dir, store) = file_store("browser-save");
    let mut settings = store.load();
    settings.browser_fill = true;
    store.save(&settings);
    let (handle, app) = open_with_store(cx, &store, Some(unlocked_fixture()));
    let vault = cx.update(|cx| app.read(cx).vault_view().cloned().unwrap());
    let texts = |cx: &mut TestAppContext| cx.update(|cx| app.read(cx).toasts().read(cx).texts());
    let check = |url: &str, user: &str, pw: &str| Request::Check {
        url: url.into(),
        username: user.into(),
        password: pw.into(),
    };
    let save = |url: &str, user: &str, pw: &str| Request::Save {
        url: url.into(),
        username: user.into(),
        password: pw.into(),
    };
    let banco = "https://banco.example.com/entrar";
    assert_eq!(
        browser_ask(cx, handle, &app, check(banco, "pessoa", "s3nh@-ção")),
        json!({ "action": "none" })
    );
    assert_eq!(
        browser_ask(cx, handle, &app, check(banco, "pessoa", "nova")),
        json!({ "action": "update", "name": "Meu Banco" })
    );
    assert_eq!(
        browser_ask(cx, handle, &app, check("https://novo.example", "eu", "x")),
        json!({ "action": "save" })
    );

    // Form open: doesn't save underneath it.
    cx.update_window(handle, |_, window, cx| {
        vault.update(cx, |v, cx| v.new_item(window, cx));
    })
    .unwrap();
    let r = browser_ask(cx, handle, &app, save("https://novo.example", "eu", "x"));
    assert!(r["error"].as_str().is_some_and(|e| e.contains("editando")));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("editor-cancel", cx);
    })
    .unwrap();

    // Locked: `check` doesn't draw attention; `save` waits for unlock.
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |a, cx| a.lock(window, cx))
    })
    .unwrap();
    let before = texts(cx).len();
    assert_eq!(
        browser_ask(cx, handle, &app, check("https://novo.example", "eu", "x")),
        json!({ "locked": true })
    );
    assert_eq!(texts(cx).len(), before, "sem aviso a cada login nos sites");
    assert_eq!(
        browser_ask(cx, handle, &app, save("https://novo.example", "eu", "x")),
        json!({ "queued": true })
    );
    assert!(texts(cx)
        .iter()
        .any(|t| t.contains("salvar o login de novo.example")));
    let (password, _) = fixture_password_and_email();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("unlock-password", cx);
        window.input(&password, cx);
        window.click("unlock-submit", cx);
    })
    .unwrap();
    wait_until(cx, |cx| {
        cx.update(|cx| app.read(cx).screen_kind() == ScreenKind::Vault)
    });
    // Unlocked: the stored login goes to the server (which doesn't exist in the fixture).
    let vault = cx.update(|cx| app.read(cx).vault_view().cloned().unwrap());
    assert!(cx.update(|cx| vault.read(cx).is_busy()));
    wait_until(cx, |cx| cx.update(|cx| !vault.read(cx).is_busy()));
    let _ = std::fs::remove_dir_all(dir);
}

/// Against the local Vaultwarden: saves a new login from the browser, then
/// updates its password, and checks the password history.
#[gpui_kit::test]
#[ignore = "precisa de servidor; veja dev/test-account.env"]
fn browser_save_and_update_through_the_app(cx: &mut TestAppContext) {
    use scout_vault_app::browser::Request;
    use scout_vault_core::vault::ItemKind;
    cx.executor().allow_parking();
    cx.update(gpui_kit::init);
    let env = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("defina {k}"));
    let (url, email, password) = (
        env("SCOUT_VAULT_TEST_URL"),
        env("SCOUT_VAULT_TEST_EMAIL"),
        env("SCOUT_VAULT_TEST_PASSWORD"),
    );
    let (dir, store) = file_store("browser-save-live");
    let mut settings = store.load();
    settings.browser_fill = true;
    store.save(&settings);
    let (handle, app) = open_with_store(cx, &store, None);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.within("login-server").click(2usize, cx);
        window.click("login-server-url", cx);
        window.input(&url, cx);
        window.click("login-email", cx);
        window.input(&email, cx);
        window.click("login-password", cx);
        window.input(&password, cx);
        window.click("login-submit", cx);
    })
    .unwrap();
    wait_until(cx, |cx| {
        cx.update(|cx| app.read(cx).screen_kind() == ScreenKind::Vault)
    });
    let vault = cx.update(|cx| app.read(cx).vault_view().cloned().unwrap());
    let site = format!("teste-salvar-{}.example", std::process::id());
    let page = format!("https://{site}/login?x=1");
    let find = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            vault
                .read(cx)
                .list()
                .read(cx)
                .delegate()
                .vault()
                .items
                .iter()
                .find(|i| i.name == site && !i.deleted)
                .cloned()
        })
    };
    let idle =
        |cx: &mut TestAppContext| wait_until(cx, |cx| cx.update(|cx| !vault.read(cx).is_busy()));

    let r = browser_ask(
        cx,
        handle,
        &app,
        Request::Save {
            url: page.clone(),
            username: "eu@site".into(),
            password: "primeira".into(),
        },
    );
    assert_eq!(r, serde_json::json!({ "ok": true }));
    wait_until(cx, |cx| find(cx).is_some());
    idle(cx);
    let item = find(cx).unwrap();
    let ItemKind::Login(l) = &item.kind else {
        panic!()
    };
    assert_eq!(l.username.as_deref(), Some("eu@site"));
    assert_eq!(l.uris, [format!("https://{site}")]);

    // Same password: nothing to do. New password: updates the same item.
    assert_eq!(
        browser_ask(
            cx,
            handle,
            &app,
            Request::Check {
                url: page.clone(),
                username: "eu@site".into(),
                password: "primeira".into(),
            },
        ),
        serde_json::json!({ "action": "none" })
    );
    browser_ask(
        cx,
        handle,
        &app,
        Request::Save {
            url: page.clone(),
            username: "eu@site".into(),
            password: "segunda".into(),
        },
    );
    wait_until(cx, |cx| {
        find(cx).is_some_and(|i| match &i.kind {
            ItemKind::Login(l) => l.password.as_ref().is_some_and(|p| p.expose() == "segunda"),
            _ => false,
        })
    });
    idle(cx);
    assert_eq!(find(cx).unwrap().id, item.id, "atualizou, não duplicou");

    // Clean up.
    cx.update_window(handle, |_, window, cx| {
        vault.update(cx, |v, cx| v.select_item(&item.id, window, cx));
        window.render_frame(cx);
        window.click("item-trash", cx);
    })
    .unwrap();
    wait_until(cx, |cx| find(cx).is_none());
    idle(cx);
    cx.update_window(handle, |_, window, cx| {
        vault.update(cx, |v, cx| {
            v.set_filter(scout_vault_app::item_list::Filter::Trash, window, cx);
            v.select_item(&item.id, window, cx);
        });
        window.render_frame(cx);
        window.click("item-delete-forever", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("item-delete-forever", cx);
    })
    .unwrap();
    wait_until(cx, |cx| {
        cx.update(|cx| {
            vault
                .read(cx)
                .list()
                .read(cx)
                .delegate()
                .vault()
                .items
                .iter()
                .all(|i| i.id != item.id)
        })
    });
    let _ = std::fs::remove_dir_all(dir);
}

/// With "Motion: Still", switching the theme is direct: no circle and no veil.
#[gpui_kit::test]
fn theme_change_without_motion_is_instant(cx: &mut TestAppContext) {
    use scout_vault_app::theme_fx;
    let (handle, _app) = open_app(cx, Some(unlocked_fixture()));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("vault-settings", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("motion-off", cx);
        window.render_frame(cx);
        window.click("theme-dark", cx);
        assert!(cx.theme().is_dark());
        assert!(!theme_fx::running(cx));
    })
    .unwrap();
}

/// Quick search: finds, copies, fills in the target app, respects the master password
/// prompt and, with the vault locked, asks to unlock.
#[gpui_kit::test]
fn quick_search_finds_copies_and_fills(cx: &mut TestAppContext) {
    use scout_vault_app::vault_view::CopyShortcut;
    cx.executor().allow_parking();
    let (handle, app) = open_app(cx, Some(unlocked_fixture()));
    let typed = fake_typer(&app, cx);
    let vault = cx.update(|cx| app.read(cx).vault_view().cloned().unwrap());
    let texts = |cx: &mut TestAppContext| cx.update(|cx| app.read(cx).toasts().read(cx).texts());
    let open = |cx: &mut TestAppContext, title: &str, own: bool| {
        let mut r = request(title);
        r.own_window = own;
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |a, cx| a.open_quick_search(r, window, cx))
        })
        .unwrap();
        cx.update(|cx| app.read(cx).quick_search())
            .expect("busca rápida aberta")
    };

    // Find and copy the password (without opening the item in the app).
    let (popup, quick) = open(cx, "Entrar - Banco", false);
    popup
        .update(cx, |_, window, cx| {
            quick.update(cx, |q, cx| q.set_query("banco", window, cx));
            let first = quick.read(cx).selected().unwrap().clone();
            assert_eq!(first.name, "Meu Banco");
            assert_eq!(first.detail, "pessoa");
            quick.update(cx, |q, cx| q.copy(CopyShortcut::Password, window, cx));
        })
        .unwrap();
    cx.run_until_parked();
    assert!(texts(cx).contains(&"Copiado: Senha · some em 30 s".to_owned()));
    assert!(cx.update(|cx| vault.read(cx).selected().is_none()));
    // Shows "Password copied" and closes on its own.
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
    assert!(popup.update(cx, |_, _, _| ()).is_err(), "a janela fechou");

    // Enter: types into the app that had focus.
    let (popup, quick) = open(cx, "Entrar - Banco", false);
    popup
        .update(cx, |_, window, cx| {
            quick.update(cx, |q, cx| {
                q.set_query("meu banco", window, cx);
                q.activate(window, cx);
            });
        })
        .unwrap();
    wait_until(cx, |_| !typed.lock().unwrap().is_empty());
    let (target, keys) = typed.lock().unwrap()[0].clone();
    assert_eq!(target, 42, "digita na janela que estava em foco");
    assert_eq!(keys[0], "pessoa");
    assert_eq!(keys[2], "s3nh@-ção");
    typed.lock().unwrap().clear();

    // Item that asks for the master password: nothing is typed before confirming.
    let (popup, quick) = open(cx, "Outro app", false);
    popup
        .update(cx, |_, window, cx| {
            quick.update(cx, |q, cx| {
                q.set_query("chave", window, cx);
                q.activate(window, cx);
            });
        })
        .unwrap();
    cx.run_until_parked();
    assert!(typed.lock().unwrap().is_empty());
    assert!(cx.update(|cx| vault.read(cx).reprompt_open()));

    // Used inside Scout Vault itself: Enter opens the item in the app.
    let (popup, quick) = open(cx, "Scout Vault", true);
    popup
        .update(cx, |_, window, cx| {
            quick.update(cx, |q, cx| {
                q.set_query("identidade", window, cx);
                assert_eq!(q.selected().unwrap().id, "identity-1");
                q.activate(window, cx);
            });
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        cx.update(|cx| vault.read(cx).selected().map(|i| i.id.clone())),
        Some("identity-1".into())
    );

    // Locked: the HUD itself asks for the vault password (not the main window).
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |a, cx| {
            a.lock(window, cx);
            a.open_quick_search(request("Qualquer"), window, cx)
        })
    })
    .unwrap();
    let (popup, quick) = cx
        .update(|cx| app.read(cx).quick_search())
        .expect("HUD pedindo a senha");
    assert!(cx.update(|cx| quick.read(cx).is_locked()));
    let _ = texts;

    // Wrong password: says so and stays open.
    popup
        .update(cx, |_, window, cx| {
            window.render_frame(cx);
            window.click("quick-search-password", cx);
            window.input("senha-errada", cx);
            window.press("enter", cx);
        })
        .unwrap();
    wait_until(cx, |cx| {
        cx.update(|cx| quick.read(cx).unlock_error().is_some())
    });
    assert!(cx.update(|cx| app.read(cx).vault_view().is_none()));
    assert!(cx.update(|cx| quick.read(cx).is_locked()));

    // Right password: the vault opens and the HUD shows the list.
    let (password, _) = fixture_password_and_email();
    popup
        .update(cx, |_, window, cx| {
            window.render_frame(cx);
            window.click("quick-search-password", cx);
            window.input(&password, cx);
            window.press("enter", cx);
        })
        .unwrap();
    wait_until(cx, |cx| {
        cx.update(|cx| {
            app.read(cx).vault_view().is_some()
                && app
                    .read(cx)
                    .quick_search()
                    .is_some_and(|(_, q)| !q.read(cx).is_locked())
        })
    });
}

/// With "ask for the password every N days" still within the period, a
/// shortcut opens the locked vault without asking anything.
#[gpui_kit::test]
fn shortcut_skips_the_password_within_the_remembered_period(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    cx.update(gpui_kit::init);
    let (_dir, store) = file_store("hud-lembra");
    let mut s = store.load();
    s.remember_session = true;
    s.unlock_memory_days = 30;
    store.save(&s);
    let (handle, app) = open_with_store(cx, &store, Some(unlocked_fixture()));
    let _typed = fake_typer(&app, cx);
    // Typing the password once (main window) remembers it for the period.
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |a, cx| a.lock(window, cx));
    })
    .unwrap();
    let (password, _) = fixture_password_and_email();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("unlock-password", cx);
        window.input(&password, cx);
        window.click("unlock-submit", cx);
    })
    .unwrap();
    wait_until(cx, |cx| cx.update(|cx| app.read(cx).vault_view().is_some()));

    // Auto-lock, then the shortcut: the list opens, no password.
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |a, cx| {
            a.lock(window, cx);
            a.open_quick_search(request("Qualquer"), window, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
    assert!(cx.update(|cx| app.read(cx).vault_view().is_some()));
    let (_, quick) = cx.update(|cx| app.read(cx).quick_search()).expect("HUD");
    assert!(!cx.update(|cx| quick.read(cx).is_locked()));
}

/// The clipboard clearing time comes from the settings (and
/// shows up in the notice); "Never" doesn't clear nor promise to clear.
#[gpui_kit::test]
fn clipboard_clear_time_follows_the_setting(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let (_dir, store) = file_store("clip");
    let mut s = store.load();
    s.clipboard_clear_secs = 10;
    store.save(&s);
    let (handle, app) = open_with_store(cx, &store, Some(unlocked_fixture()));
    let vault = cx.update(|cx| app.read(cx).vault_view().cloned().unwrap());
    let texts = |cx: &mut TestAppContext| cx.update(|cx| app.read(cx).toasts().read(cx).texts());
    cx.update_window(handle, |_, window, cx| {
        vault.update(cx, |v, cx| v.select_item("login-1", window, cx));
        window.render_frame(cx);
        window.click("copy-password", cx);
    })
    .unwrap();
    assert_eq!(texts(cx), ["Copiado: Senha · some em 10 s"]);
    // 10 s later, it's gone.
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(11));
    cx.run_until_parked();
    assert!(cx
        .update(|cx| cx.read_from_clipboard().and_then(|c| c.text()))
        .unwrap_or_default()
        .is_empty());

    // "Never": copies without a deadline and the value stays.
    cx.update(|cx| scout_vault_app::clipboard::set_clear_after_secs(0, cx));
    cx.update_window(handle, |_, window, cx| window.click("copy-password", cx))
        .unwrap();
    assert!(texts(cx).contains(&"Copiado: Senha".to_owned()));
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(40));
    cx.run_until_parked();
    assert_eq!(
        cx.update(|cx| cx.read_from_clipboard().and_then(|c| c.text())),
        Some("s3nh@-ção".to_owned())
    );
}

/// "Lock on minimize": minimizing (or going to the tray) locks only with
/// the option on.
#[gpui_kit::test]
fn minimizing_locks_only_when_asked(cx: &mut TestAppContext) {
    let (handle, app) = open_app(cx, Some(unlocked_fixture()));
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |a, cx| a.on_minimized(window, cx))
    })
    .unwrap();
    assert_eq!(
        cx.update(|cx| app.read(cx).screen_kind()),
        ScreenKind::Vault
    );

    let (_dir, store) = file_store("minimize");
    let mut s = store.load();
    s.lock_on_minimize = true;
    store.save(&s);
    let (handle, app) = open_with_store(cx, &store, Some(unlocked_fixture()));
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |a, cx| a.on_minimized(window, cx))
    })
    .unwrap();
    assert_eq!(
        cx.update(|cx| app.read(cx).screen_kind()),
        ScreenKind::Locked
    );
}

/// Fake leak API server: answers `login-1` (the fixture's password)
/// as leaked 7 times, and returns the requested paths.
fn fake_pwned_server() -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let paths = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = paths.clone();
    let (_, leaked) = scout_vault_core::health::pwned_hash("s3nh@-ção");
    std::thread::spawn(move || {
        for sock in listener.incoming() {
            let Ok(mut sock) = sock else { break };
            let mut buf = vec![0u8; 4096];
            let n = sock.read(&mut buf).unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]).to_string();
            if let Some(path) = request.split_whitespace().nth(1) {
                seen.lock().unwrap().push(path.to_owned());
            }
            let body = format!("{leaked}:7\r\n0000000000000000000000000000000000A:0");
            let _ = sock.write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            );
        }
    });
    (url, paths)
}

/// "Password health": opens from the vault, shows the score and the tabs, checks for
/// leaks (hash prefixes only), opens the clicked item and goes away on lock.
#[gpui_kit::test]
fn health_report_checks_and_opens_items(cx: &mut TestAppContext) {
    use scout_vault_app::health_view::{Pwned, Tab};
    cx.executor().allow_parking();
    let (handle, app) = open_app(cx, Some(unlocked_fixture()));
    let vault = cx.update(|cx| app.read(cx).vault_view().cloned().unwrap());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("vault-health", cx);
    })
    .unwrap();
    let health = cx.update(|cx| app.read(cx).health_view().cloned().expect("tela aberta"));
    let checked = cx.update(|cx| health.read(cx).report().checked);
    assert!(checked > 0, "o fixture tem logins com senha");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("health-score").is_some());
        for tab in Tab::ALL {
            assert!(window.try_find(tab.id()).is_some());
        }
        // Leaks not checked yet: "?" and the check button.
        assert!(window
            .find(Tab::Exposed.id())
            .label()
            .is_some_and(|l| l.ends_with('?')));
        window.click(Tab::Exposed.id(), cx);
    })
    .unwrap();
    assert_eq!(cx.update(|cx| health.read(cx).tab()), Tab::Exposed);

    // Checks against the fake server.
    let (url, paths) = fake_pwned_server();
    cx.update(|cx| health.update(cx, |h, _| h.set_pwned_url(url)));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("health-check-pwned", cx);
    })
    .unwrap();
    wait_until(cx, |cx| {
        cx.update(|cx| matches!(health.read(cx).pwned(), Pwned::Done(_)))
    });
    let pwned = cx.update(|cx| health.read(cx).pwned().clone());
    assert_eq!(pwned, Pwned::Done(vec![("login-1".to_owned(), 7)]));
    for p in paths.lock().unwrap().iter() {
        assert_eq!(p.len(), "/range/".len() + 5, "só o prefixo sai: {p}");
    }

    // Clicking the leaked item opens it in the vault.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("health-item-login-1", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(cx.update(|cx| app.read(cx).health_view().is_none()));
    assert_eq!(
        cx.update(|cx| vault.read(cx).selected().map(|i| i.id.clone())),
        Some("login-1".to_owned())
    );

    // Locking discards the screen (it had decrypted copies).
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |a, cx| {
            a.open_health(window, cx);
            assert!(a.health_view().is_some());
            a.lock(window, cx);
            assert!(a.health_view().is_none());
        })
    })
    .unwrap();
}

/// Item icon: clicking the icon in the detail opens the picker; the choice applies
/// right away, is stored on this computer and goes back to the default with "Default".
#[gpui_kit::test]
fn item_icon_can_be_chosen_saved_and_reset(cx: &mut TestAppContext) {
    use gpui_kit::assets::IconName;
    use scout_vault_app::item_icons::{icon_for, CustomIcon, ItemIcon};
    cx.update(gpui_kit::init);
    let (_dir, store) = file_store("icons");
    let (handle, app) = open_with_store(cx, &store, Some(unlocked_fixture()));
    let vault = cx.update(|cx| app.read(cx).vault_view().cloned().unwrap());
    let icon_of = |cx: &mut TestAppContext, id: &str| {
        cx.update(|cx| {
            let item = vault
                .read(cx)
                .list()
                .read(cx)
                .delegate()
                .vault()
                .items
                .iter()
                .find(|i| i.id == id)
                .cloned()
                .unwrap();
            icon_for(&item, cx)
        })
    };
    assert_eq!(
        icon_of(cx, "login-1"),
        ItemIcon::Lucide(IconName::Globe),
        "padrão: ícone do tipo"
    );

    cx.update_window(handle, |_, window, cx| {
        vault.update(cx, |v, cx| v.select_item("login-1", window, cx));
        window.render_frame(cx);
        window.click("item-icon", cx);
    })
    .unwrap();
    assert!(cx.update(|cx| vault.read(cx).icon_picker_open()));
    // The picker expands: wait for the animation before clicking inside it.
    std::thread::sleep(std::time::Duration::from_millis(300));
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(300));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("icon-choice-gamepad", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("icon-color-violet", cx);
    })
    .unwrap();
    assert_eq!(icon_of(cx, "login-1"), ItemIcon::Lucide(IconName::Gamepad2));
    assert_eq!(
        store.load().item_icons.get("login-1"),
        Some(&CustomIcon {
            icon: "gamepad".into(),
            color: Some("violet".into()),
        }),
        "guardado nas configurações"
    );

    // Switching items closes the picker.
    cx.update_window(handle, |_, window, cx| {
        vault.update(cx, |v, cx| v.select_item("identity-1", window, cx));
    })
    .unwrap();
    assert!(!cx.update(|cx| vault.read(cx).icon_picker_open()));

    // Open the app again: the choice is still there.
    let (handle2, app2) = open_with_store(cx, &store, Some(unlocked_fixture()));
    let vault2 = cx.update(|cx| app2.read(cx).vault_view().cloned().unwrap());
    let _ = handle2;
    let again = cx.update(|cx| {
        let item = vault2
            .read(cx)
            .list()
            .read(cx)
            .delegate()
            .vault()
            .items
            .iter()
            .find(|i| i.id == "login-1")
            .cloned()
            .unwrap();
        icon_for(&item, cx)
    });
    assert_eq!(again, ItemIcon::Lucide(IconName::Gamepad2));

    // "Default" goes back to the type icon and deletes the choice.
    cx.update_window(handle, |_, window, cx| {
        vault.update(cx, |v, cx| {
            v.select_item("login-1", window, cx);
            v.toggle_icon_picker(cx);
        });
        window.render_frame(cx);
    })
    .unwrap();
    // The picker expands: wait for the animation before clicking inside it.
    std::thread::sleep(std::time::Duration::from_millis(300));
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(300));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("icon-reset", cx);
    })
    .unwrap();
    assert_eq!(icon_of(cx, "login-1"), ItemIcon::Lucide(IconName::Globe));
    assert!(store.load().item_icons.is_empty());

    // Brands: choose Discord in the picker.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("icon-choice-brand-discord", cx);
    })
    .unwrap();
    assert_eq!(icon_of(cx, "login-1"), ItemIcon::Brand("discord"));
    assert_eq!(
        store
            .load()
            .item_icons
            .get("login-1")
            .map(|c| c.icon.clone()),
        Some("brand:discord".to_owned())
    );

    // A click outside the picker closes it; clicking the icon opens it again.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("detail-name", cx);
    })
    .unwrap();
    assert!(!cx.update(|cx| vault.read(cx).icon_picker_open()));
    // (a later click: the one right after closing it belongs to the same gesture)
    std::thread::sleep(std::time::Duration::from_millis(600));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("item-icon", cx);
    })
    .unwrap();
    assert!(cx.update(|cx| vault.read(cx).icon_picker_open()));
}

/// Signed out: the shortcut shows the login inside the HUD (not the main
/// window), and the HUD doesn't close when it loses focus (2FA by email, SSO).
#[gpui_kit::test]
fn signed_out_shortcut_shows_the_login_in_the_hud(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (handle, app) = open_app(cx, None);
    let _typed = fake_typer(&app, cx);
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |a, cx| {
            a.handle_autotype(request("Riot Client"), window, cx)
        })
    })
    .unwrap();
    let (popup, quick) = cx
        .update(|cx| app.read(cx).quick_search())
        .expect("HUD com login");
    assert!(cx.update(|cx| quick.read(cx).login_view().is_some()));
    popup
        .update(cx, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("login-email").is_some());
            assert!(window.try_find("login-password").is_some());
            assert!(window.try_find("login-submit").is_some());
            // Only the form: no settings button nor entry screen.
            assert!(window.try_find("login-settings").is_none());
            window.click("quick-search-close", cx);
        })
        .unwrap();
    cx.run_until_parked();
    assert!(popup.update(cx, |_, _, _| ()).is_err(), "o HUD fechou");
}

/// An app that doesn't tell its fields: nothing is typed until the person
/// confirms in the HUD; a field that isn't a password field stops typing and
/// the HUD says so.
#[gpui_kit::test]
fn autotype_asks_before_typing_where_it_cant_check_the_field(cx: &mut TestAppContext) {
    use scout_vault_app::autotype::{FieldCheck, Key, TypeError};
    cx.executor().allow_parking();
    let (handle, app) = open_app(cx, Some(unlocked_fixture()));
    let checks: Arc<std::sync::Mutex<Vec<FieldCheck>>> = Default::default();
    let seen = checks.clone();
    cx.update(|cx| {
        app.update(cx, |app, _| {
            app.set_typer(Arc::new(move |_, keys: Vec<Key>, check| {
                seen.lock().unwrap().push(check);
                assert!(keys.iter().any(|k| matches!(k, Key::Password(_))));
                match check {
                    FieldCheck::Verify => Err(TypeError::CantVerify),
                    FieldCheck::Skip => Ok(()),
                }
            }))
        })
    });
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |app, cx| {
            app.handle_autotype(request("Entrar — banco.example.com — Firefox"), window, cx)
        })
    })
    .unwrap();
    wait_until(cx, |cx| {
        cx.update(|cx| {
            app.read(cx)
                .quick_search()
                .is_some_and(|(_, q)| q.read(cx).notice_text().is_some())
        })
    });
    let (popup, _) = cx.update(|cx| app.read(cx).quick_search()).unwrap();
    popup
        .update(cx, |_, window, cx| {
            window.render_frame(cx);
            // The message wraps inside the card instead of being cut at its edge.
            let notice = window.find("quick-search-notice").bounds();
            let button = window.find("quick-search-type-anyway").bounds();
            assert!(
                notice.right() <= button.right() + gpui_kit::px(1.),
                "notice {:?} past the card (button {:?})",
                notice,
                button
            );
            // Enter confirms, like the button.
            window.press("enter", cx);
        })
        .unwrap();
    wait_until(cx, |_| checks.lock().unwrap().len() == 2);
    assert_eq!(
        *checks.lock().unwrap(),
        [FieldCheck::Verify, FieldCheck::Skip]
    );

    // Not a password field: stops, and the HUD explains (no "type anyway").
    cx.update(|cx| {
        app.update(cx, |app, _| {
            app.set_typer(Arc::new(|_, _, _| {
                Err(TypeError::Failed("não é campo de senha".into()))
            }))
        })
    });
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |app, cx| {
            app.handle_autotype(request("Entrar — banco.example.com — Firefox"), window, cx)
        })
    })
    .unwrap();
    wait_until(cx, |cx| {
        cx.update(|cx| {
            app.read(cx).quick_search().is_some_and(|(_, q)| {
                q.read(cx).notice_text().map(|t| t.to_string())
                    == Some("não é campo de senha".into())
            })
        })
    });
    let (popup, _) = cx.update(|cx| app.read(cx).quick_search()).unwrap();
    popup
        .update(cx, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("quick-search-type-anyway").is_none());
        })
        .unwrap();
}

/// The floating windows' opacity is a setting: it applies to the next HUD and is saved.
#[gpui_kit::test]
fn hud_opacity_is_a_setting(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let (_dir, store) = file_store("hud-opacidade");
    let (handle, app) = open_with_store(cx, &store, Some(unlocked_fixture()));
    assert_eq!(
        cx.update(|cx| scout_vault_app::quick_search::opacity(cx)),
        0.92,
        "padrão"
    );
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |a, cx| a.open_settings(window, cx));
        window.render_frame(cx);
        // A slider from 1% to 100%.
        assert!(window.try_find("settings-hud-opacity").is_some());
        let view = app.read(cx).settings_view().cloned().unwrap();
        view.update(cx, |v, cx| v.set_hud_opacity(1, cx));
        window.render_frame(cx);
    })
    .unwrap();
    assert_eq!(
        cx.update(|cx| scout_vault_app::quick_search::opacity(cx)),
        0.01
    );
    assert_eq!(store.load().hud_opacity, 1);
}

/// Settings → Shortcuts: click, press the new combination, it's saved; Esc
/// cancels; Shift alone and the other shortcut's keys are refused; "Default" undoes.
#[gpui_kit::test]
fn shortcuts_can_be_recorded_in_settings(cx: &mut TestAppContext) {
    use scout_vault_app::settings_view::HotkeyKind;
    cx.update(gpui_kit::init);
    let (_dir, store) = file_store("atalhos");
    let (handle, app) = open_with_store(cx, &store, Some(unlocked_fixture()));
    let settings = |cx: &gpui_kit::App| app.read(cx).settings_view().cloned().unwrap();
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |a, cx| a.open_settings(window, cx));
        window.render_frame(cx);
        // (the section is at the bottom of the page: start recording directly)
        settings(cx).update(cx, |v, cx| {
            v.record_shortcut(HotkeyKind::QuickSearch, window, cx)
        });
        window.render_frame(cx);
    })
    .unwrap();
    assert!(cx.update(|cx| settings(cx).read(cx).is_recording()));

    // Shift alone isn't enough; the auto-type keys are taken.
    cx.update_window(handle, |_, window, cx| {
        window.press("shift-k", cx);
        window.render_frame(cx);
        assert!(settings(cx).read(cx).is_recording());
        window.press("ctrl-alt-a", cx);
    })
    .unwrap();
    assert!(cx.update(|cx| settings(cx).read(cx).is_recording()));
    assert_eq!(store.load().hotkeys.quick_search, "Ctrl+Shift+Space");

    // A valid one is saved and recording ends.
    cx.update_window(handle, |_, window, cx| window.press("ctrl-alt-k", cx))
        .unwrap();
    assert!(!cx.update(|cx| settings(cx).read(cx).is_recording()));
    assert_eq!(store.load().hotkeys.quick_search, "Ctrl+Alt+K");
    assert_eq!(
        cx.update(|cx| scout_vault_app::shortcut::quick_search_label(cx)),
        "Ctrl+Alt+K"
    );

    // Esc cancels without closing settings; "Default" goes back.
    cx.update_window(handle, |_, window, cx| {
        settings(cx).update(cx, |v, cx| {
            v.record_shortcut(HotkeyKind::AutoType, window, cx)
        });
        window.render_frame(cx);
        window.press("escape", cx);
    })
    .unwrap();
    assert!(!cx.update(|cx| settings(cx).read(cx).is_recording()));
    assert!(cx.update(|cx| app.read(cx).settings_view().is_some()));
    assert_eq!(store.load().hotkeys.autotype, "Ctrl+Alt+A");
    cx.update(|cx| {
        settings(cx).update(cx, |v, cx| v.set_hotkey(HotkeyKind::QuickSearch, None, cx))
    });
    assert_eq!(store.load().hotkeys.quick_search, "Ctrl+Shift+Space");
}

/// Profile: clicking the email in the sidebar opens it; shows the account and
/// the vault in numbers; Esc goes back; "Sign out" asks twice and goes to login.
#[gpui_kit::test]
fn start_with_windows_is_a_setting(cx: &mut TestAppContext) {
    // Outside the real app nothing touches the registry: the choice stays in memory.
    let (handle, app) = open_app(cx, Some(unlocked_fixture()));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("vault-settings", cx);
    })
    .unwrap();
    let settings = cx.update(|cx| app.read(cx).settings_view().cloned().unwrap());
    assert!(!scout_vault_app::autostart::is_enabled());
    cx.update(|cx| settings.update(cx, |s, cx| s.set_start_with_windows(true, cx)));
    assert!(scout_vault_app::autostart::is_enabled());
    cx.update(|cx| settings.update(cx, |s, cx| s.set_start_with_windows(false, cx)));
    assert!(!scout_vault_app::autostart::is_enabled());
    let line = scout_vault_app::autostart::command_line().unwrap();
    assert!(
        line.starts_with('"') && line.ends_with("\" --tray"),
        "{line}"
    );
}

#[gpui_kit::test]
fn profile_photo_is_chosen_kept_in_the_app_folder_and_removed(cx: &mut TestAppContext) {
    let dir = std::env::temp_dir().join(format!("scout-vault-ui-photo-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let store = SettingsStore::at(dir.join("settings.json"));
    cx.update(gpui_kit::init);
    let (handle, app) = open_with_store(cx, &store, Some(unlocked_fixture()));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("vault-email", cx);
    })
    .unwrap();
    let profile = cx.update(|cx| app.read(cx).profile_view().cloned().unwrap());
    let email = cx.update(|cx| profile.read(cx).info().email.clone());

    // Not an image: refused, still the initials.
    let text = dir.join("notes.txt");
    std::fs::write(&text, b"hello, not a picture").unwrap();
    cx.update(|cx| profile.update(cx, |p, cx| p.set_photo(&text, cx)));
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(scout_vault_app::profile_photo::get(&email, cx).is_none());
        assert!(!profile.read(cx).cropping());
    });

    // A wide PNG: opens the crop editor, framed in the middle, without zoom.
    let wide = image::RgbaImage::from_fn(300, 200, |x, y| image::Rgba([x as u8, y as u8, 0, 255]));
    let picture = dir.join("me.png");
    wide.save(&picture).unwrap();
    cx.update(|cx| profile.update(cx, |p, cx| p.set_photo(&picture, cx)));
    cx.run_until_parked();
    let area = cx.update(|cx| profile.read(cx).crop_area().expect("the editor opens"));
    assert_eq!((area.x, area.y, area.size), (50., 0., 200.));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("photo-crop-area").is_some());
    })
    .unwrap();

    // Cancel: nothing saved.
    cx.update(|cx| profile.update(cx, |p, cx| p.cancel_crop(cx)));
    cx.update(|cx| assert!(scout_vault_app::profile_photo::get(&email, cx).is_none()));

    // Again, zoomed in and dragged to the left edge (it can't leave the picture).
    cx.update(|cx| profile.update(cx, |p, cx| p.set_photo(&picture, cx)));
    cx.run_until_parked();
    cx.update(|cx| {
        profile.update(cx, |p, cx| {
            p.set_crop_zoom(2., cx);
            p.pan_crop(1000., 0., cx);
        })
    });
    let area = cx.update(|cx| profile.read(cx).crop_area().unwrap());
    assert_eq!((area.x, area.size), (0., 100.));
    cx.update_window(handle, |_, window, cx| window.click("photo-crop-save", cx))
        .unwrap();

    // Saved square in the app folder and shown.
    let stored = cx.update(|cx| {
        assert!(!profile.read(cx).cropping());
        assert!(scout_vault_app::profile_photo::get(&email, cx).is_some());
        scout_vault_app::profile_photo::stored_path(&email, cx).unwrap()
    });
    assert!(stored.starts_with(dir.join("photos")));
    let saved = image::ImageReader::open(&stored)
        .unwrap()
        .with_guessed_format()
        .unwrap()
        .decode()
        .unwrap();
    assert_eq!((saved.width(), saved.height()), (100, 100));
    std::fs::remove_file(&picture).unwrap(); // the copy doesn't depend on the original
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("profile-photo-remove").is_some());
    })
    .unwrap();

    // Remove: back to the initials, and the copy is deleted.
    cx.update_window(handle, |_, window, cx| {
        window.click("profile-photo-remove", cx)
    })
    .unwrap();
    cx.update(|cx| assert!(scout_vault_app::profile_photo::get(&email, cx).is_none()));
    assert!(!stored.exists());
    let _ = std::fs::remove_dir_all(dir);
}

#[gpui_kit::test]
fn profile_shows_the_account_and_signs_out(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (handle, app) = open_app(cx, Some(unlocked_fixture()));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("vault-email", cx);
    })
    .unwrap();
    assert!(
        cx.update(|cx| app.read(cx).profile_view().is_some()),
        "o clique no e-mail abre o perfil"
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("profile-name").is_some());
        assert!(window.try_find("profile-premium").is_some());
        assert!(window.try_find("profile-count-0").is_some());
    })
    .unwrap();
    let info = cx.update(|cx| app.read(cx).profile_view().unwrap().read(cx).info().clone());
    assert!(info.email.contains('@'));
    assert!(info.counts[0] > 0, "o fixture tem logins");

    cx.update_window(handle, |_, window, cx| window.press("escape", cx))
        .unwrap();
    assert!(cx.update(|cx| app.read(cx).profile_view().is_none()));

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("vault-email", cx);
    })
    .unwrap();
    // (the button is at the bottom of the page: call it directly)
    let sign_out = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            let view = app.read(cx).profile_view().cloned().unwrap();
            view.update(cx, |v, cx| v.sign_out(cx));
        });
        cx.run_until_parked();
    };
    sign_out(cx);
    assert!(
        cx.update(|cx| app.read(cx).vault_view().is_some()),
        "pede confirmação"
    );
    sign_out(cx);
    assert!(cx.update(|cx| app.read(cx).vault_view().is_none()));
    assert!(cx.update(|cx| app.read(cx).profile_view().is_none()));
}
