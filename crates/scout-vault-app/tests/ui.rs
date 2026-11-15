//! Testes de integração de UI: telas reais numa janela headless (nada aparece
//! na tela), com cliques e digitação despachados pelo mesmo caminho do app.

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
use scout_vault_app::clipboard::CLEAR_AFTER;
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

/// Scout Vault sintético do `scout-vault-core` (gerado em Python), já aberto.
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

        // Sem identificador e sem senha: valida antes de abrir o navegador.
        window.click("login-submit", cx);
        assert_eq!(
            window.find("login-status").label(),
            Some("Informe o identificador SSO da organização.")
        );
        window.click("login-sso-identifier", cx);
        window.input("minha-empresa", cx);
        window.click("login-submit", cx);
        assert_eq!(
            window.find("login-status").label(),
            Some("Informe a senha mestra: ela abre o cofre depois do SSO.")
        );
    })
    .unwrap();
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

/// A cor do tema bate com a amostra (tolerância para arredondamento HSL).
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

        light_bg = cx.theme().background;
        // Claro → escuro: a interface não some — o modo já é o novo, mas as
        // cores começam de onde estavam…
        window.click("theme-dark", cx);
        assert!(cx.theme().is_dark());
        assert!(same_color(cx.theme().background, light_bg));
        assert!(theme_fx::running(cx));
    })
    .unwrap();
    // …no meio, cada cor é uma mistura das duas…
    cx.executor().advance_clock(theme_fx::FLIP / 2);
    cx.run_until_parked();
    let mid = cx.update(|cx| cx.theme().background);
    assert!(!same_color(mid, light_bg));
    assert!(cx.update(|cx| theme_fx::running(cx)));
    // …e no fim chega exatamente ao tema escuro.
    settle(cx);
    cx.update(|cx| {
        assert!(cx.theme().background.l < mid.l);
        assert!(!theme_fx::running(cx));
    });

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // A cor de destaque também se transforma (não pula).
        window.click("accent-coral", cx);
        assert!(theme_fx::running(cx));
        assert!(!same_color(cx.theme().primary, Accent::Coral.swatch(true)));
    })
    .unwrap();
    settle(cx);
    cx.update(|cx| assert!(same_color(cx.theme().primary, Accent::Coral.swatch(true))));

    // Outra troca no meio de uma transição continua dali para o novo alvo.
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
        // A cor escolhida sobrevive à troca de modo.
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
        // A chave continua escondida.
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

    // Ctrl+F leva o foco para a busca: o texto digitado filtra a lista.
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
        window.click("length-32", cx);
    })
    .unwrap();
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

    // Esc fecha e volta ao cofre.
    cx.update_window(handle, |_, window, cx| window.press("escape", cx))
        .unwrap();
    cx.update(|cx| {
        assert!(app.read(cx).generator_view().is_none());
        assert_eq!(app.read(cx).screen_kind(), ScreenKind::Vault);
    });
    let _ = std::fs::remove_dir_all(dir);
}

/// Servidor HTTP mínimo que responde 400 a tudo (refresh token recusado).
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
            Some("A sessão expirou no servidor. Entre novamente.")
        );
    })
    .unwrap();
}

type Typed = Arc<std::sync::Mutex<Vec<(isize, Vec<String>)>>>;

/// Digitador falso: registra o que seria digitado em vez de mexer no teclado.
fn fake_typer(app: &Entity<ScoutVaultApp>, cx: &mut TestAppContext) -> Typed {
    use scout_vault_app::autotype::Key;
    let typed: Typed = Default::default();
    let sink = typed.clone();
    cx.update(|cx| {
        app.update(cx, |app, _| {
            app.set_typer(Arc::new(move |window, keys| {
                let keys = keys
                    .iter()
                    .map(|k| match k {
                        Key::Text(t) => t.to_string(),
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
    cx.update(|cx| assert!(app.read(cx).autotype_view().is_none()));
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
        window.render_frame(cx);
        // Sem sugestão: lista todos os logins digitáveis.
        assert!(window.try_find("autotype-pick-login-1").is_some());
        assert!(window.try_find("autotype-pick-card-org").is_none());
        window.click("autotype-search", cx);
        window.input("chave", cx);
        window.render_frame(cx);
        assert!(window.try_find("autotype-pick-login-1").is_none());
        window.click("autotype-pick-login-item-key", cx);
    })
    .unwrap();
    // Este item pede a senha mestra: nada é digitado antes de confirmar.
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
    cx.update(|cx| assert!(app.read(cx).autotype_view().is_none()));
}

#[gpui_kit::test]
fn autotype_waits_for_unlock_and_ignores_own_window(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (handle, app) = open_app(cx, Some(unlocked_fixture()));
    let typed = fake_typer(&app, cx);
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |app, cx| {
            // A própria janela do Scout Vault: nada a digitar.
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
    // Ao desbloquear, o pedido guardado é atendido.
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

    // Configurações: tira o Enter; depois desliga tudo.
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
        // "Parado" vale para todas as animações da interface.
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

    // Primeira abertura: cofre aberto; bloquear guarda a sessão.
    let (handle, app) = open_with_store(cx, &store, Some(unlocked_fixture()));
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |a, cx| a.lock(window, cx))
    })
    .unwrap();
    assert!(scout_vault_app::saved_session::load(&store).is_some());

    // "Reabrir o app": começa no desbloqueio, sem login.
    let (handle, app) = open_with_store(cx, &store, None);
    cx.update(|cx| assert_eq!(app.read(cx).screen_kind(), ScreenKind::Locked));

    // O servidor do fixture não existe (127.0.0.1:9): abre do cache, offline.
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

    // Sair da conta apaga.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("unlock-logout", cx);
    })
    .unwrap();
    cx.update(|cx| assert_eq!(app.read(cx).screen_kind(), ScreenKind::Login));
    assert!(scout_vault_app::saved_session::load(&store).is_none());
    assert!(store.read_session_blob_for_tests().is_none());

    // Desligar a opção também apaga.
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
        // Sem nome não salva.
        window.click("editor-save", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("editor-status").label(),
            Some("Dê um nome ao item.")
        );
        // Troca para nota e volta para login sem perder o nome.
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

    // O servidor do fixture não existe: o erro aparece e o formulário continua lá.
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
        // Item na lixeira: restaurar e apagar de vez, sem editar.
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

/// Cria, edita e apaga um item pela interface, contra o Vaultwarden local.
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

    // Criar.
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

    // Editar: troca a senha.
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

    // Lixeira e apagar de vez.
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

/// Anexa, abre e remove um arquivo pela interface, contra o Vaultwarden local.
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
    // O servidor guarda o tamanho do arquivo cifrado (IV, MAC e padding a mais).
    assert!(att
        .size
        .is_some_and(|s| s > "conteúdo secreto".len() as u64));

    // Abrir baixa, decifra e grava a cópia temporária.
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

    // Remover (dois cliques) e apagar o item.
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

    // Desligado por padrão.
    let off = browser_ask(cx, handle, &app, list());
    assert!(off["error"].as_str().unwrap().contains("desligada"));

    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |a, cx| a.open_settings(window, cx));
        window.render_frame(cx);
        // Fica mais abaixo na página; o clique é testado pelo método do botão.
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
    // Outro site não recebe a senha.
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
    // O servidor do fixture não existe: a sincronização falha e a barra some.
    wait_until(cx, |cx| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.try_find("vault-progress").is_none()
        })
        .unwrap()
    });
}

/// O formulário aproveita a largura: campos curtos dividem a linha.
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
        // Seletor de tipo na barra, na altura do Salvar.
        let kind = f32::from(window.find("editor-kind-login").bounds().center().y);
        let save = f32::from(window.find("editor-save").bounds().center().y);
        assert!((kind - save).abs() < 2.0, "seletor {kind} x salvar {save}");
        // Centralizado no espaço livre: sobra igual dos dois lados.
        let first = window.find("editor-kind-login").bounds();
        let last = window.find("editor-kind-identity").bounds();
        let cancel = window.find("editor-cancel").bounds();
        let detail = window.find("editor-name").bounds();
        let left_gap = f32::from(first.left() - detail.left());
        let right_gap = f32::from(cancel.left() - last.right());
        assert!(
            (left_gap - right_gap).abs() < 16.0,
            "esquerda {left_gap} x direita {right_gap}"
        );
        // Login: usuário e senha lado a lado.
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
        // Nada passa da borda da janela.
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
    // O servidor do fixture não existe: termina com erro e mostra o ✗…
    wait_until(cx, |cx| {
        label(cx).as_deref() == Some("Não foi possível sincronizar")
    });
    // …que volta às setas depois de um instante.
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(1700));
    cx.run_until_parked();
    assert_eq!(label(cx).as_deref(), Some("Sincronizar"));
}

/// Avisos viram toasts: aparecem, ficam um pouco e saem. Nada fica fixo no rodapé.
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
    // Copiar de novo não empilha o mesmo aviso.
    cx.update_window(handle, |_, window, cx| window.click("copy-password", cx))
        .unwrap();
    assert_eq!(texts(cx).len(), 1);
    // Depois do tempo, sai (e some da tela).
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

    // "+" ao lado de Pastas abre o campo; vazio não cria.
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

    // Com uma pasta aberta: renomear e apagar no cabeçalho.
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

/// Cria, renomeia e apaga uma pasta pela interface, contra o Vaultwarden local.
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
    // Abre a pasta recém-criada.
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

    // Com outro filtro ativo, o atalho volta para "Todos" e abre o item.
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

    // Desafixar tira da barra e do arquivo.
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
            // Sem bandeja (testes), fechar fecha de verdade.
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

/// Cria uma nota já com arquivos e anexa mais pelo detalhe, contra o Vaultwarden local.
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

    // Dois de uma vez pelo detalhe (como arrastar vários).
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

    // Limpa: lixeira e apagar de vez.
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

    // Coleções na barra lateral, embaixo da organização; filtrar por uma.
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

    // Novo item a partir da coleção: já nasce na organização e nela.
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
    // Sem coleção não salva.
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

    // Item existente da organização: dono fixo, coleções editáveis.
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

/// Cria um item numa organização pela interface, contra o Vaultwarden local.
/// Precisa da organização de `tools/dev_org.py`.
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

    // Primeira vez: bloqueia e desbloqueia com a senha (isso guarda a chave).
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
    // "Reabrir o app": já abre desbloqueado, sem senha.
    let (_, app2) = open_with_store(cx, &store, None);
    cx.update(|cx| assert_eq!(app2.read(cx).screen_kind(), ScreenKind::Vault));

    // Senha mestra trocada noutro lugar: a user key cifrada do perfil muda no sync.
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
    // Padrão: 30 dias. Passou de 30 → pede login.
    let (_, app) = open_with_store(cx, &store, None);
    cx.update(|cx| assert_eq!(app.read(cx).screen_kind(), ScreenKind::Login));
    assert!(
        scout_vault_app::saved_session::load(&store).is_none(),
        "sessão esquecida"
    );

    // Com "Nunca", a mesma sessão antiga continua valendo.
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
            // "Nunca": nem depois de um dia.
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

/// Clicar num item da lista mostra ele, mesmo com o formulário aberto.
#[gpui_kit::test]
fn clicking_an_item_leaves_the_form(cx: &mut TestAppContext) {
    let (handle, app) = open_app(cx, Some(unlocked_fixture()));
    let vault = cx.update(|cx| app.read(cx).vault_view().cloned().unwrap());
    let selected =
        |cx: &mut TestAppContext| cx.update(|cx| vault.read(cx).selected().map(|i| i.id.clone()));
    // Formulário de item novo aberto → clica num item.
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

    // Editando um item → clica em outro: mostra o outro.
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

/// Arrastar para uma pasta com o formulário aberto não perde o formulário.
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
        ["Salve ou cancele o formulário antes de mover itens."]
    );
}

/// Arrasta um item para uma pasta e uma pasta para dentro de outra, contra o
/// Vaultwarden local.
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

    // Um item sem pasta, para arrastar.
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

    // Item → pasta B.
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

    // Pasta B → dentro de A: vira "A/B" e aparece recuada, só com o nome.
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
        // E de volta para o topo, soltando no título "Pastas".
        window.drag_to(
            gpui_kit::SharedString::from(format!("filter-folder-{inner_id}")),
            "folders-header",
            cx,
        );
    })
    .unwrap();
    wait_until(cx, |cx| folder_id(cx, &inner) == Some(inner_id.clone()));
    idle(cx);

    // Item → Sem pasta.
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

    // Limpeza: apaga as pastas e o item.
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

/// Menu da bandeja: reflete bloqueio e acesso rápido; copiar, gerar e
/// sincronizar funcionam sem abrir o item.
#[gpui_kit::test]
fn tray_menu_copies_generates_and_follows_lock(cx: &mut TestAppContext) {
    use scout_vault_app::tray::{Command, PinnedEntry};
    use scout_vault_app::vault_view::CopyShortcut;
    cx.executor().allow_parking();
    let (handle, app) = open_app(cx, Some(unlocked_fixture()));
    let vault = cx.update(|cx| app.read(cx).vault_view().cloned().unwrap());
    let texts = |cx: &mut TestAppContext| cx.update(|cx| app.read(cx).toasts().read(cx).texts());
    let menu = |cx: &mut TestAppContext| cx.update(|cx| app.read(cx).tray_menu_state(cx));

    // Aberto, sem fixados.
    let state = menu(cx);
    assert!(state.unlocked);
    assert!(state.pinned.is_empty());

    // Fixa um login: aparece com usuário e senha.
    cx.update(|cx| vault.update(cx, |v, _| v.set_pinned(vec!["login-1".into()])));
    assert_eq!(
        menu(cx).pinned,
        [PinnedEntry {
            id: "login-1".into(),
            name: "Meu Banco".into(),
            has_username: true,
            has_password: true,
            has_totp: true,
            icon: Some(gpui_kit::assets::IconName::Globe),
        }]
    );

    // Copiar a senha pelo menu, sem abrir o item.
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
    // Campo que o item não tem.
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

    // Gerar e copiar.
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |a, cx| {
            a.handle_tray(Command::GeneratePassword, window, cx)
        });
    })
    .unwrap();
    assert!(texts(cx).contains(&"Senha gerada e copiada · some em 30 s".to_owned()));

    // Sincronizar começa na hora.
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |a, cx| a.handle_tray(Command::Sync, window, cx));
    })
    .unwrap();
    assert!(cx.update(|cx| vault.read(cx).is_busy()));
    // Espera a tentativa terminar (sem servidor, falha) antes de seguir.
    wait_until(cx, |cx| cx.update(|cx| !vault.read(cx).is_busy()));

    // Bloqueado: menu sem acesso rápido, e copiar pede o desbloqueio.
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

/// O menu da bandeja é uma janela do app: abre no cursor, cada linha faz a
/// ação e fecha o menu; `Esc` fecha; bloqueado, só oferece desbloquear.
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

    // Aberto: acesso rápido com os botões de cada campo do item.
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

    // Abrir de novo troca o menu; Esc fecha.
    let (menu, _) = open(cx);
    cx.update_window(menu, |_, window, cx| {
        window.render_frame(cx);
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(!is_open(cx, menu));

    // Gerar senha pelo menu.
    let (menu, _) = open(cx);
    cx.update_window(menu, |_, window, cx| {
        window.render_frame(cx);
        window.click("tray-menu-generate", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(texts(cx).contains(&"Senha gerada e copiada · some em 30 s".to_owned()));

    // Bloquear pelo menu; o próximo menu só oferece desbloquear e gerar.
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

/// A janela do menu tem a altura exata das linhas (sem sobra nem corte).
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
    // Muitos fixados rolam dentro do menu em vez de crescer sem fim.
    assert_eq!(window_size(&pin(6)).height, window_size(&pin(10)).height);
    assert_eq!(locked.width, empty.width);
}

/// Bloquear o Windows (ou suspender) bloqueia o cofre — a não ser que a
/// opção esteja desligada.
#[gpui_kit::test]
fn locks_with_windows_unless_turned_off(cx: &mut TestAppContext) {
    use scout_vault_app::windows_events::OsEvent;
    cx.update(gpui_kit::init);
    let (dir, store) = file_store("lockwin");
    let mut settings = store.load();
    assert!(settings.lock_with_windows, "ligado por padrão");
    // Desligado: nada muda.
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
    // Ligado: bloqueia ao bloquear o Windows…
    settings.lock_with_windows = true;
    store.save(&settings);
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |app, cx| {
            app.handle_os_event(OsEvent::SessionLocked, window, cx);
            assert_eq!(app.screen_kind(), ScreenKind::Locked);
        })
    })
    .unwrap();
    // …e ao suspender.
    let (handle, app) = open_with_store(cx, &store, Some(unlocked_fixture()));
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |app, cx| {
            app.handle_os_event(OsEvent::Suspending, window, cx);
            assert_eq!(app.screen_kind(), ScreenKind::Locked);
        })
    })
    .unwrap();
    // A opção aparece nas configurações.
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |app, cx| app.open_settings(window, cx));
        window.render_frame(cx);
        assert!(window.try_find("lock-with-windows").is_some());
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(dir);
}

/// Item com "pedir a senha mestra": mostrar, copiar e editar esperam a senha;
/// senha errada não libera; trocar de item volta a pedir.
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
        // Mostrar a senha: abre o pedido em vez de revelar.
        window.click("reveal-password", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("reprompt-password").is_some());
        assert_eq!(window.find("value-password").label(), Some("••••••••"));
        // Senha errada.
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
            Some("Senha mestra incorreta.")
        );
        assert_eq!(window.find("value-password").label(), Some("••••••••"));
        // Agora a certa: revela o que tinha sido pedido.
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
        // Liberado: copiar vai direto.
        window.click("copy-password", cx);
    })
    .unwrap();
    assert!(texts(cx).contains(&"Copiado: Senha · some em 30 s".to_owned()));

    // Outro item e de volta: pede de novo; editar espera a senha.
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
    // O formulário mostra a opção marcada.
    let draft = cx.update(|cx| vault.read(cx).editor().unwrap().read(cx).to_item(cx));
    assert!(draft.reprompt);

    // Itens sem a opção não pedem nada.
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

/// Pela bandeja e pelo navegador, o item que pede a senha mestra não sai sem ela.
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
    // Não copiou: abriu o item com o pedido.
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
    // Confirmou: a cópia pedida acontece.
    wait_until(cx, |cx| {
        texts(cx).contains(&"Copiado: Senha · some em 30 s".to_owned())
    });

    // Navegador: recusa preencher.
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
        .is_some_and(|e| e.contains("pede a senha mestra")));
    assert!(reply.get("password").is_none());
}

/// Windows Hello (com um assinador falso no lugar do Windows): ligar com o
/// cofre aberto guarda a chave cifrada; o botão abre o cofre; outra chave do
/// Hello ou cancelar não abrem; sair da conta apaga.
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

    // Liga nas configurações (com o cofre aberto, pede o Hello na hora).
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |a, cx| a.open_settings(window, cx));
        window.render_frame(cx);
        assert!(window.try_find("windows-hello").is_some());
        let view = app.read(cx).settings_view().cloned().unwrap();
        view.update(cx, |v, cx| v.set_windows_hello(true, cx));
    })
    .unwrap();
    wait_until(cx, |cx| {
        texts(cx).contains(&"Windows Hello ligado para desbloquear".to_owned())
    });
    assert!(store.load().windows_hello);
    assert!(hello::is_enrolled(&store, &session));

    // Bloqueia: aparece "Usar Windows Hello", e ele abre o cofre.
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

    // Outra chave do Hello (ex.: outro usuário do Windows): não abre.
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

    // Cancelar no Hello: explica e continua bloqueado.
    cx.update(|cx| hello::install(cx, hello::canceling_signer()));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("unlock-hello", cx);
    })
    .unwrap();
    wait_until(cx, |cx| unlock(cx).is_some_and(|e| e.contains("cancelado")));

    // A senha mestra mudou no servidor: a chave do Hello é descartada.
    let mut changed = session.clone();
    let mut sync = (*changed.sync).clone();
    sync.profile.key = Some("2.outra|chave|qualquer".into());
    changed.sync = std::sync::Arc::new(sync);
    assert!(hello::password_changed(&store, &changed));
    assert!(!hello::is_enrolled(&store, &session));

    // Senha mestra digitada com a opção ainda ligada: configura de novo sozinho.
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
    // Sair da conta apaga.
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

/// Sem Windows Hello no computador: ligar a opção explica e volta a desligar.
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

/// Sync em segundo plano: só sincroniza quando a revisão do servidor muda, e
/// não atrapalha um formulário aberto nem roda com a opção desligada.
#[gpui_kit::test]
fn background_sync_only_when_the_vault_changed(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (handle, app) = open_app(cx, Some(unlocked_fixture()));
    let vault = cx.update(|cx| app.read(cx).vault_view().cloned().unwrap());
    let busy = |cx: &mut TestAppContext| cx.update(|cx| vault.read(cx).is_busy());
    let revision = |cx: &mut TestAppContext, date: i64| {
        cx.update(|cx| vault.update(cx, |v, cx| v.on_revision(date, cx)))
    };
    // Primeira resposta: vira a base, sem sync.
    revision(cx, 100);
    assert!(!busy(cx));
    assert_eq!(cx.update(|cx| vault.read(cx).last_revision()), Some(100));
    // Mesma revisão: nada.
    revision(cx, 100);
    assert!(!busy(cx));
    // Formulário aberto: espera (não sincroniza por baixo da edição).
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
    // Opção desligada: nada.
    cx.update(|cx| vault.update(cx, |v, _| v.set_background_sync(false)));
    revision(cx, 200);
    assert!(!busy(cx));
    // Ligada e mudou: sincroniza.
    cx.update(|cx| vault.update(cx, |v, _| v.set_background_sync(true)));
    revision(cx, 200);
    assert!(busy(cx));
    // (o servidor do fixture não existe: o sync falha e a base continua a antiga)
    wait_until(cx, |cx| !busy(cx));
    assert_eq!(cx.update(|cx| vault.read(cx).last_revision()), Some(100));
    // A opção aparece nas configurações e começa ligada.
    assert!(cx.update(|cx| app.read(cx).store().load().background_sync));
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |a, cx| a.open_settings(window, cx));
        window.render_frame(cx);
        assert!(window.try_find("background-sync").is_some());
    })
    .unwrap();
}

/// Contra o Vaultwarden local: uma pasta criada "em outro aparelho" aparece
/// sozinha depois da checagem de revisão.
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
    // Primeira checagem: base.
    cx.update(|cx| vault.update(cx, |v, cx| v.check_now(cx)));
    wait_until(cx, |cx| {
        cx.update(|cx| vault.read(cx).last_revision().is_some())
    });

    // "Outro aparelho" cria uma pasta direto no servidor.
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

    // A checagem percebe e sincroniza sozinha.
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

    // Limpeza.
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

/// Salvar logins do navegador, sem servidor: o `check` responde sem senha e sem
/// incomodar quando bloqueado; o `save` recusa com formulário aberto e, com o
/// cofre bloqueado, espera o desbloqueio.
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

    // Formulário aberto: não salva por baixo dele.
    cx.update_window(handle, |_, window, cx| {
        vault.update(cx, |v, cx| v.new_item(window, cx));
    })
    .unwrap();
    let r = browser_ask(cx, handle, &app, save("https://novo.example", "eu", "x"));
    assert!(r["error"]
        .as_str()
        .is_some_and(|e| e.contains("formulário")));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("editor-cancel", cx);
    })
    .unwrap();

    // Bloqueado: o `check` não chama atenção; o `save` espera o desbloqueio.
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
    // Desbloqueou: o login guardado vai para o servidor (que no fixture não existe).
    let vault = cx.update(|cx| app.read(cx).vault_view().cloned().unwrap());
    assert!(cx.update(|cx| vault.read(cx).is_busy()));
    wait_until(cx, |cx| cx.update(|cx| !vault.read(cx).is_busy()));
    let _ = std::fs::remove_dir_all(dir);
}

/// Contra o Vaultwarden local: salva um login novo pelo navegador, depois
/// atualiza a senha dele, e confere o histórico de senhas.
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

    // Mesma senha: nada a fazer. Senha nova: atualiza o mesmo item.
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

    // Limpeza.
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

/// Com "Movimento: Parado", trocar o tema é direto: sem círculo nem véu.
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

/// Busca rápida: acha, copia, preenche no app de destino, respeita o pedido de
/// senha mestra e, com o cofre bloqueado, manda desbloquear.
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

    // Achar e copiar a senha (sem abrir o item no app).
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
    // Mostra "Senha copiada" e fecha sozinha.
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
    assert!(popup.update(cx, |_, _, _| ()).is_err(), "a janela fechou");

    // Enter: digita no app que estava em foco.
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

    // Item que pede a senha mestra: nada é digitado antes de confirmar.
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

    // Usada dentro do próprio Scout Vault: Enter abre o item no app.
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

    // Bloqueado: não abre a janelinha; pede para desbloquear.
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |a, cx| {
            a.lock(window, cx);
            a.open_quick_search(request("Qualquer"), window, cx)
        })
    })
    .unwrap();
    assert!(cx.update(|cx| app.read(cx).quick_search().is_none()));
    assert!(texts(cx).contains(&"Desbloqueie o cofre para usar a busca rápida.".to_owned()));
}
