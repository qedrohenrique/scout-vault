//! Testes de integração de UI: telas reais numa janela headless (nada aparece
//! na tela), com cliques e digitação despachados pelo mesmo caminho do app.

use std::sync::Arc;
use std::time::{Duration, Instant};

use cofre_app::app::{CofreApp, ScreenKind};
use cofre_app::appearance::{Accent, Appearance, ThemeChoice};
use cofre_app::clipboard::CLEAR_AFTER;
use cofre_app::flow::{Session, Unlocked};
use cofre_app::settings::SettingsStore;
use cofre_core::api::{Client, DeviceInfo, ServerUrls};
use cofre_core::crypto::MasterKey;
use cofre_core::sync::SyncResponse;
use cofre_core::vault::{Keys, Vault};
use cofre_core::Kdf;
use gpui_kit::component::ActiveTheme;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    px, size, AnyWindowHandle, AppContext, Bounds, Entity, TestAppContext, WindowBounds,
    WindowOptions,
};

const SYNC: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../cofre-core/tests/fixtures/sync.json"
));
const VECTORS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../cofre-core/tests/fixtures/vectors.json"
));

fn fixture_password_and_email() -> (String, String) {
    let v: serde_json::Value = serde_json::from_str(VECTORS).unwrap();
    (
        v["password"].as_str().unwrap().to_owned(),
        v["email"].as_str().unwrap().to_owned(),
    )
}

/// Cofre sintético do `cofre-core` (gerado em Python), já aberto.
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
) -> (AnyWindowHandle, Entity<CofreApp>) {
    cx.update(gpui_kit::init);
    cx.update(|cx| {
        gpui_kit::open_window(options(), cx, |window, cx| {
            cx.new(|cx| match unlocked {
                Some(u) => CofreApp::with_unlocked(SettingsStore::in_memory(), u, window, cx),
                None => CofreApp::new(SettingsStore::in_memory(), window, cx),
            })
        })
        .expect("abrir janela de teste")
    })
}

/// Espera um trabalho que roda fora do executor do GPUI (KDF no Tokio).
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
            Some("Informe o e-mail da conta.")
        );

        window.click("login-email", cx);
        window.input("pessoa@exemplo.com", cx);
        window.click("login-submit", cx);
        assert_eq!(
            window.find("login-status").label(),
            Some("Informe a senha mestra.")
        );
    })
    .unwrap();

    cx.update(|cx| assert_eq!(app.read(cx).screen_kind(), ScreenKind::Login));
}

#[gpui_kit::test]
fn self_hosted_needs_a_valid_url(cx: &mut TestAppContext) {
    let (handle, _) = open_app(cx, None);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // O campo de URL só aparece ao escolher "Servidor próprio".
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

    // Filtros da barra lateral.
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

    // Abre o login e confere que a senha começa escondida.
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

        // Copiar não precisa revelar.
        window.click("copy-password", cx);
    })
    .unwrap();

    let clipboard =
        |cx: &mut TestAppContext| cx.update(|cx| cx.read_from_clipboard().and_then(|c| c.text()));
    assert_eq!(clipboard(cx).as_deref(), Some("s3nh@-ção"));

    // Antes do prazo continua lá; depois some.
    cx.executor()
        .advance_clock(CLEAR_AFTER - Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(clipboard(cx).as_deref(), Some("s3nh@-ção"));
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    assert!(clipboard(cx).unwrap_or_default().is_empty());

    // Bloquear descarta o cofre decifrado e pede a senha.
    cx.update_window(handle, |_, window, cx| window.click("vault-lock", cx))
        .unwrap();
    cx.update(|cx| {
        assert_eq!(app.read(cx).screen_kind(), ScreenKind::Locked);
        assert!(app.read(cx).vault_view().is_none());
    });

    // Senha errada.
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
            window.find("unlock-status").label() == Some("Senha mestra incorreta.")
        })
        .unwrap()
    });

    // Senha certa reabre o cofre, sem rede.
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
            // Ainda dentro do prazo: continua aberto.
            app.lock_if_idle(Instant::now(), window, cx);
            assert_eq!(app.screen_kind(), ScreenKind::Vault);
            // Passou do prazo: bloqueia.
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

    // `set_query` escreve no campo de busca da lista, que dispara a busca real.
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

/// Login completo pela UI contra um servidor de verdade (Vaultwarden local).
/// Rode com as variáveis de `dev/test-account.env`:
/// `cargo test -p cofre-app --test ui -- --ignored`
#[gpui_kit::test]
#[ignore = "precisa de servidor; veja dev/test-account.env"]
fn logs_in_through_the_ui_against_a_real_server(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let env = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("defina {k}"));
    let (url, email, password) = (
        env("COFRE_TEST_URL"),
        env("COFRE_TEST_EMAIL"),
        env("COFRE_TEST_PASSWORD"),
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

/// A cor do tema bate com a amostra (tolerância para arredondamento HSL).
fn same_color(a: gpui_kit::Hsla, b: gpui_kit::Hsla) -> bool {
    let (a, b) = (a.to_rgb(), b.to_rgb());
    (a.r - b.r).abs() < 0.01 && (a.g - b.g).abs() < 0.01 && (a.b - b.b).abs() < 0.01
}

#[gpui_kit::test]
fn settings_change_theme_and_accent_live(cx: &mut TestAppContext) {
    let (handle, app) = open_app(cx, Some(unlocked_fixture()));

    // Começa com a cor padrão (jade) aplicada.
    cx.update(|cx| {
        let dark = cx.theme().is_dark();
        assert!(same_color(cx.theme().primary, Accent::Jade.swatch(dark)));
    });

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("vault-settings", cx);
    })
    .unwrap();
    // O pedido sai por evento, tratado depois do clique.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("settings-close").is_some());

        window.click("theme-dark", cx);
        assert!(cx.theme().is_dark());
        window.click("accent-coral", cx);
        assert!(same_color(cx.theme().primary, Accent::Coral.swatch(true)));

        window.click("theme-light", cx);
        assert!(!cx.theme().is_dark());
        // A cor escolhida sobrevive à troca de modo.
        assert!(same_color(cx.theme().primary, Accent::Coral.swatch(false)));

        window.click("text-large", cx);
        assert_eq!(cx.theme().font_size, gpui_kit::px(17.));
        window.click("corners-round", cx);
        assert_eq!(cx.theme().radius, gpui_kit::px(10.));
    })
    .unwrap();

    cx.update(|cx| {
        let view = app.read(cx).settings_view().unwrap();
        assert_eq!(
            view.read(cx).appearance(),
            Appearance {
                theme: ThemeChoice::Light,
                accent: Accent::Coral,
                text_size: cofre_app::appearance::TextSize::Large,
                corners: cofre_app::appearance::Corners::Round,
            }
        );
    });

    // Fechar volta para o cofre, que continuou aberto por baixo.
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
    let dir = std::env::temp_dir().join(format!("cofre-ui-appearance-{}", std::process::id()));
    let store = SettingsStore::at(dir.join("settings.json"));
    let mut settings = store.load();
    settings.appearance.theme = ThemeChoice::Dark;
    settings.appearance.accent = Accent::Indigo;
    store.save(&settings);

    cx.update(gpui_kit::init);
    cx.update(|cx| {
        gpui_kit::open_window(options(), cx, |window, cx| {
            cx.new(|cx| CofreApp::new(store.clone(), window, cx))
        })
        .unwrap();
        assert!(cx.theme().is_dark());
        assert!(same_color(cx.theme().primary, Accent::Indigo.swatch(true)));
    });
    let _ = std::fs::remove_dir_all(dir);
}
