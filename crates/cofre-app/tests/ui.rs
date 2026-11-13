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
                motion: cofre_app::appearance::Motion::On,
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

#[gpui_kit::test]
fn shows_live_totp_code_and_copies_it_without_spaces(cx: &mut TestAppContext) {
    let (handle, app) = open_app(cx, Some(unlocked_fixture()));
    let vault_view = cx.update(|cx| app.read(cx).vault_view().cloned().unwrap());
    let totp =
        cofre_core::totp::Totp::parse("otpauth://totp/Banco?secret=JBSWY3DPEHPK3PXP").unwrap();

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
    let dir = std::env::temp_dir().join(format!("cofre-ui-generator-{}", std::process::id()));
    let store = SettingsStore::at(dir.join("settings.json"));
    cx.update(gpui_kit::init);
    let store2 = store.clone();
    let (handle, app) = cx.update(|cx| {
        gpui_kit::open_window(options(), cx, |window, cx| {
            cx.new(|cx| CofreApp::with_unlocked(store2, unlocked_fixture(), window, cx))
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
fn fake_typer(app: &Entity<CofreApp>, cx: &mut TestAppContext) -> Typed {
    use cofre_app::autotype::Key;
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

fn request(title: &str) -> cofre_app::autotype::Request {
    cofre_app::autotype::Request {
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
            // A própria janela do Cofre: nada a digitar.
            let mut own = request("Cofre");
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
    let dir = std::env::temp_dir().join(format!("cofre-ui-autotype-{}", std::process::id()));
    let store = SettingsStore::at(dir.join("settings.json"));
    cx.update(gpui_kit::init);
    let s2 = store.clone();
    let (handle, app) = cx.update(|cx| {
        gpui_kit::open_window(options(), cx, |window, cx| {
            cx.new(|cx| CofreApp::with_unlocked(s2, unlocked_fixture(), window, cx))
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
            let next = cofre_app::settings::AutoTypeSettings {
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
            let next = cofre_app::settings::AutoTypeSettings {
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
            cofre_app::appearance::Motion::Off
        );
        app.update(cx, |app, cx| app.close_settings(cx));
        window.render_frame(cx);
        assert_eq!(window.find("entry-backdrop").label(), Some("parado"));
    })
    .unwrap();
}

fn open_with_store(
    cx: &mut TestAppContext,
    store: &SettingsStore,
    unlocked: Option<Unlocked>,
) -> (AnyWindowHandle, Entity<CofreApp>) {
    let store = store.clone();
    cx.update(|cx| {
        gpui_kit::open_window(options(), cx, |window, cx| {
            cx.new(|cx| match unlocked {
                Some(u) => CofreApp::with_unlocked(store, u, window, cx),
                None => CofreApp::new(store, window, cx),
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
    assert!(cofre_app::saved_session::load(&store).is_some());

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
    assert!(cofre_app::saved_session::load(&store).is_some());

    // Sair da conta apaga.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("unlock-logout", cx);
    })
    .unwrap();
    cx.update(|cx| assert_eq!(app.read(cx).screen_kind(), ScreenKind::Login));
    assert!(cofre_app::saved_session::load(&store).is_none());
    assert!(store.read_session_blob_for_tests().is_none());

    // Desligar a opção também apaga.
    let (handle, app) = open_with_store(cx, &store, Some(unlocked_fixture()));
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |a, cx| a.lock(window, cx))
    })
    .unwrap();
    assert!(cofre_app::saved_session::load(&store).is_some());
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |a, cx| a.open_settings(window, cx));
        window.render_frame(cx);
        window.click("remember-session", cx);
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
    let cofre_core::vault::ItemKind::Login(l) = &draft.kind else {
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
    let cofre_core::vault::ItemKind::Login(l) = &draft.kind else {
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
            v.set_filter(cofre_app::item_list::Filter::Trash, window, cx)
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
            cofre_core::vault::ItemKind::Login(l) => l
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
            v.set_filter(cofre_app::item_list::Filter::Trash, window, cx)
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

    let file = std::env::temp_dir().join(format!("cofre-ui-anexo-{}.txt", std::process::id()));
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
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window
                .find("vault-status")
                .label()
                .is_some_and(|l| l.contains("aberto"))
        })
        .unwrap()
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
    app: &Entity<CofreApp>,
    request: cofre_app::browser::Request,
) -> serde_json::Value {
    let (reply, rx) = std::sync::mpsc::channel();
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |a, cx| {
            a.handle_browser(cofre_app::browser::Incoming { request, reply }, window, cx)
        })
    })
    .unwrap();
    rx.recv().unwrap()
}

#[gpui_kit::test]
fn browser_requests_follow_setting_and_lock_state(cx: &mut TestAppContext) {
    use cofre_app::browser::Request;
    cx.update(gpui_kit::init);
    let dir = std::env::temp_dir().join(format!("cofre-ui-browser-{}", std::process::id()));
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
