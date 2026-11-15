//! Tela de configurações. Por enquanto só aparência; novas seções entram como
//! mais um `group` em [`SettingsView::render`].

use gpui_kit::assets::IconName;
use gpui_kit::component::{button::*, *};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::appearance::{Accent, Appearance, Corners, Motion, TextSize, ThemeChoice};
use crate::autotype;
use crate::settings::{AutoTypeSettings, SettingsStore};
use crate::ui::{self, Segment};
use gpui_kit::component::switch::Switch;

pub enum SettingsEvent {
    Close,
    /// Ligou/desligou o Windows Hello (o app cria ou apaga a chave).
    WindowsHello(bool),
}

pub struct SettingsView {
    store: SettingsStore,
    appearance: Appearance,
    autotype: AutoTypeSettings,
    remember_session: bool,
    close_to_tray: bool,
    lock_with_windows: bool,
    windows_hello: bool,
    background_sync: bool,
    quick_search: bool,
    unlock_memory_days: u32,
    relogin_days: u32,
    auto_lock_minutes: u32,
    browser_fill: bool,
    browser_error: Option<SharedString>,
}

impl EventEmitter<SettingsEvent> for SettingsView {}

impl SettingsView {
    pub fn new(store: SettingsStore) -> Self {
        let loaded = store.load();
        Self {
            store,
            appearance: loaded.appearance,
            autotype: loaded.autotype,
            remember_session: loaded.remember_session,
            close_to_tray: loaded.close_to_tray,
            lock_with_windows: loaded.lock_with_windows,
            windows_hello: loaded.windows_hello,
            background_sync: loaded.background_sync,
            quick_search: loaded.quick_search,
            unlock_memory_days: loaded.unlock_memory_days,
            relogin_days: loaded.relogin_days,
            auto_lock_minutes: loaded.auto_lock_minutes,
            browser_fill: loaded.browser_fill,
            browser_error: None,
        }
    }

    pub fn appearance(&self) -> Appearance {
        self.appearance
    }

    /// Aplica na hora e grava.
    pub fn set_appearance(
        &mut self,
        next: Appearance,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if next == self.appearance {
            return;
        }
        let prev = self.appearance;
        self.appearance = next;
        let mut settings = self.store.load();
        settings.appearance = next;
        self.store.save(&settings);
        // Com transição: claro↔escuro "pinta" a janela a partir do clique; o resto, fade.
        let origin = window.mouse_position();
        crate::theme_fx::change(&prev, &next, origin, window, cx);
        cx.notify();
    }

    /// Abrir sem senha por N dias (0 = pedir sempre). Desligar apaga a chave guardada.
    pub fn set_unlock_memory_days(&mut self, days: u32, cx: &mut Context<Self>) {
        self.unlock_memory_days = days;
        let mut settings = self.store.load();
        settings.unlock_memory_days = days;
        self.store.save(&settings);
        if days == 0 {
            crate::saved_session::forget_quick_unlock(&self.store);
        }
        cx.notify();
    }

    pub fn set_relogin_days(&mut self, days: u32, cx: &mut Context<Self>) {
        self.relogin_days = days;
        let mut settings = self.store.load();
        settings.relogin_days = days;
        self.store.save(&settings);
        cx.notify();
    }

    pub fn set_auto_lock_minutes(&mut self, minutes: u32, cx: &mut Context<Self>) {
        self.auto_lock_minutes = minutes;
        let mut settings = self.store.load();
        settings.auto_lock_minutes = minutes;
        self.store.save(&settings);
        cx.notify();
    }

    /// Controle segmentado de opções numéricas (dias, minutos).
    fn number_choice(
        &self,
        id: &'static str,
        prefix: &'static str,
        options: &'static [(u32, &'static str)],
        current: u32,
        set: fn(&mut Self, u32, &mut Context<Self>),
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let this = cx.entity().downgrade();
        ui::segmented_compact(
            id,
            options
                .iter()
                .map(|(v, label)| Segment {
                    id: format!("{prefix}-{v}").into(),
                    label: (*label).into(),
                    icon: None,
                    selected: *v == current,
                })
                .collect(),
            move |ix, _, cx| {
                let value = options[ix].0;
                this.update(cx, |v, cx| set(v, value, cx)).ok();
            },
            cx,
        )
    }

    pub fn set_quick_search(&mut self, on: bool, cx: &mut Context<Self>) {
        self.quick_search = on;
        let mut settings = self.store.load();
        settings.quick_search = on;
        self.store.save(&settings);
        cx.notify();
    }

    pub fn set_background_sync(&mut self, on: bool, cx: &mut Context<Self>) {
        self.background_sync = on;
        let mut settings = self.store.load();
        settings.background_sync = on;
        self.store.save(&settings);
        cx.notify();
    }

    /// Liga/desliga o desbloqueio com Windows Hello (o app faz o resto).
    pub fn set_windows_hello(&mut self, on: bool, cx: &mut Context<Self>) {
        self.show_windows_hello(on, cx);
        let mut settings = self.store.load();
        settings.windows_hello = on;
        self.store.save(&settings);
        cx.emit(SettingsEvent::WindowsHello(on));
    }

    /// Só atualiza a chave na tela (o app desfez a opção, por exemplo).
    pub fn show_windows_hello(&mut self, on: bool, cx: &mut Context<Self>) {
        self.windows_hello = on;
        cx.notify();
    }

    pub fn set_lock_with_windows(&mut self, on: bool, cx: &mut Context<Self>) {
        self.lock_with_windows = on;
        let mut settings = self.store.load();
        settings.lock_with_windows = on;
        self.store.save(&settings);
        cx.notify();
    }

    pub fn set_close_to_tray(&mut self, on: bool, cx: &mut Context<Self>) {
        self.close_to_tray = on;
        let mut settings = self.store.load();
        settings.close_to_tray = on;
        self.store.save(&settings);
        cx.notify();
    }

    /// Liga/desliga a sessão lembrada; desligar apaga o que estava guardado.
    pub fn set_remember_session(&mut self, on: bool, cx: &mut Context<Self>) {
        self.remember_session = on;
        let mut settings = self.store.load();
        settings.remember_session = on;
        self.store.save(&settings);
        if !on {
            crate::saved_session::clear(&self.store);
        }
        cx.notify();
    }

    /// Liga/desliga a extensão do navegador. No app real, registra o Scout Vault
    /// no Chrome/Edge/Brave (só no usuário atual) ou desfaz o registro.
    pub fn set_browser_fill(&mut self, on: bool, cx: &mut Context<Self>) {
        self.browser_error = None;
        if crate::browser::system_enabled() {
            if let Some(dir) = self.store.dir() {
                if on {
                    if let Err(e) = crate::browser::register(&dir) {
                        self.browser_error = Some(e.into());
                        cx.notify();
                        return;
                    }
                } else {
                    crate::browser::unregister(&dir);
                }
            }
        }
        self.browser_fill = on;
        let mut settings = self.store.load();
        settings.browser_fill = on;
        self.store.save(&settings);
        cx.notify();
    }

    pub fn autotype(&self) -> AutoTypeSettings {
        self.autotype
    }

    pub fn set_autotype(&mut self, next: AutoTypeSettings, cx: &mut Context<Self>) {
        self.autotype = next;
        let mut settings = self.store.load();
        settings.autotype = next;
        self.store.save(&settings);
        cx.notify();
    }

    fn update(
        &mut self,
        edit: impl FnOnce(&mut Appearance),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut next = self.appearance;
        edit(&mut next);
        self.set_appearance(next, window, cx);
    }

    fn row(
        title: &'static str,
        description: &'static str,
        control: impl IntoElement,
        cx: &App,
    ) -> Div {
        h_flex()
            .gap_6()
            .px_4()
            .py_3p5()
            .justify_between()
            .child(
                v_flex()
                    .min_w_0()
                    .gap_0p5()
                    .child(div().text_sm().font_medium().child(title))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(description),
                    ),
            )
            .child(div().flex_shrink_0().child(control))
    }

    fn group(title: &'static str, rows: Vec<Div>, cx: &App) -> Div {
        let count = rows.len();
        let body = rows.into_iter().enumerate().map(|(ix, row)| {
            row.when(ix + 1 < count, |r| {
                r.border_b_1().border_color(cx.theme().border)
            })
        });
        v_flex()
            .gap_2()
            .child(ui::section_label(title, cx).px_0())
            .child(
                v_flex()
                    .rounded(cx.theme().radius_lg)
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().popover)
                    .children(body),
            )
    }

    fn render_accents(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dark = cx.theme().is_dark();
        let current = self.appearance.accent;
        let _ = window;
        h_flex().gap_2().children(Accent::ALL.map(|accent| {
            let selected = accent == current;
            div()
                .id(SharedString::from(format!("accent-{}", accent.id())))
                .test_support()
                .role(Role::Button)
                .aria_label(SharedString::from(accent.label()))
                .tooltip(move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(accent.label()).build(window, cx)
                })
                .size(px(26.))
                .p(px(3.))
                .rounded_full()
                .border_2()
                .border_color(if selected {
                    accent.swatch(dark)
                } else {
                    transparent_black()
                })
                .cursor_pointer()
                .active(|el| el.opacity(0.7))
                .child(
                    div()
                        .size_full()
                        .rounded_full()
                        .bg(accent.swatch(dark))
                        .flex()
                        .items_center()
                        .justify_center(),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.update(|a| a.accent = accent, window, cx)
                }))
                // O anel da cor escolhida aparece crescendo de dentro.
                .with_animation(
                    crate::motion::key("cor", format!("{}:{selected}", accent.id())),
                    Animation::new(crate::motion::NORMAL).with_easing(ease_out_quint()),
                    move |el, t| {
                        if selected {
                            el.p(px(3. + 3. * (1.0 - t)))
                        } else {
                            el
                        }
                    },
                )
        }))
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let a = self.appearance;
        let this = cx.entity().downgrade();

        let theme_control = {
            let this = this.clone();
            ui::segmented(
                "settings-theme",
                ThemeChoice::ALL
                    .iter()
                    .map(|t| Segment {
                        id: format!("theme-{}", t.id()).into(),
                        label: t.label().into(),
                        icon: Some(match t {
                            ThemeChoice::System => IconName::Monitor,
                            ThemeChoice::Light => IconName::Sun,
                            ThemeChoice::Dark => IconName::Moon,
                        }),
                        selected: *t == a.theme,
                    })
                    .collect(),
                move |ix, window, cx| {
                    this.update(cx, |v, cx| {
                        v.update(|a| a.theme = ThemeChoice::ALL[ix], window, cx)
                    })
                    .ok();
                },
                cx,
            )
        };
        let text_control = {
            let this = this.clone();
            ui::segmented(
                "settings-text-size",
                TextSize::ALL
                    .iter()
                    .map(|t| Segment {
                        id: format!("text-{}", t.id()).into(),
                        label: t.label().into(),
                        icon: None,
                        selected: *t == a.text_size,
                    })
                    .collect(),
                move |ix, window, cx| {
                    this.update(cx, |v, cx| {
                        v.update(|a| a.text_size = TextSize::ALL[ix], window, cx)
                    })
                    .ok();
                },
                cx,
            )
        };
        let corners_control = {
            let this = this.clone();
            ui::segmented(
                "settings-corners",
                Corners::ALL
                    .iter()
                    .map(|c| Segment {
                        id: format!("corners-{}", c.id()).into(),
                        label: c.label().into(),
                        icon: None,
                        selected: *c == a.corners,
                    })
                    .collect(),
                move |ix, window, cx| {
                    this.update(cx, |v, cx| {
                        v.update(|a| a.corners = Corners::ALL[ix], window, cx)
                    })
                    .ok();
                },
                cx,
            )
        };

        let motion_control = {
            let this = this.clone();
            ui::segmented(
                "settings-motion",
                Motion::ALL
                    .iter()
                    .map(|m| Segment {
                        id: format!("motion-{}", m.id()).into(),
                        label: m.label().into(),
                        icon: None,
                        selected: *m == a.motion,
                    })
                    .collect(),
                move |ix, window, cx| {
                    this.update(cx, |v, cx| {
                        v.update(|a| a.motion = Motion::ALL[ix], window, cx)
                    })
                    .ok();
                },
                cx,
            )
        };

        let appearance_group = Self::group(
            "APARÊNCIA",
            vec![
                Self::row(
                    "Tema",
                    "Claro, escuro ou igual ao do sistema.",
                    theme_control,
                    cx,
                ),
                Self::row(
                    "Cor de destaque",
                    "Usada na seleção, no foco e nos botões principais.",
                    self.render_accents(window, cx),
                    cx,
                ),
                Self::row(
                    "Tamanho do texto",
                    "Vale para todas as telas.",
                    text_control,
                    cx,
                ),
                Self::row(
                    "Cantos",
                    "Arredondamento de botões e cartões.",
                    corners_control,
                    cx,
                ),
                Self::row(
                    "Movimento",
                    "Animações da interface e a arte de fundo do login. \"Parado\" desliga tudo.",
                    motion_control,
                    cx,
                ),
            ],
            cx,
        );

        let at = self.autotype;
        let autotype_group = Self::group(
            "PREENCHER EM OUTROS APPS",
            vec![
                Self::row(
                    "Atalho global",
                    "Com o cofre aberto, Ctrl+Alt+A em qualquer janela digita usuário ⇥ senha.",
                    Switch::new("autotype-enabled")
                        .checked(at.enabled)
                        .on_click(cx.listener(move |this, v: &bool, _, cx| {
                            this.set_autotype(AutoTypeSettings { enabled: *v, ..at }, cx)
                        })),
                    cx,
                ),
                Self::row(
                    "Apertar Enter no final",
                    "Envia o formulário depois de digitar a senha.",
                    Switch::new("autotype-enter")
                        .checked(at.press_enter)
                        .disabled(!at.enabled)
                        .on_click(cx.listener(move |this, v: &bool, _, cx| {
                            this.set_autotype(
                                AutoTypeSettings {
                                    press_enter: *v,
                                    ..at
                                },
                                cx,
                            )
                        })),
                    cx,
                ),
            ],
            cx,
        );
        let _ = autotype::HOTKEY_LABEL;
        let browser_group = {
            let mut rows = vec![Self::row(
                "Preencher no navegador",
                "Deixa a extensão do Scout Vault (Chrome, Edge, Brave) pedir logins a este app. Só funciona com o cofre desbloqueado.",
                Switch::new("browser-enabled")
                    .checked(self.browser_fill)
                    .on_click(cx.listener(|this, v: &bool, _, cx| this.set_browser_fill(*v, cx))),
                cx,
            )];
            if self.browser_fill {
                let dir = crate::browser::extension_dir();
                rows.push(
                    v_flex()
                        .gap_2()
                        .px_4()
                        .py_3p5()
                        .text_sm()
                        .child(div().font_medium().child("Instalar a extensão"))
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(
                                    "No navegador, abra chrome://extensions (ou edge://extensions), ligue o \
                                     Modo do desenvolvedor, clique em \"Carregar sem compactação\" e escolha a \
                                     pasta abaixo. Depois, num site, clique no ícone do Scout Vault ou use Ctrl+Shift+L.",
                                ),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_xs()
                                        .font_family(cx.theme().mono_font_family.clone())
                                        .child(dir.to_string_lossy().into_owned()),
                                )
                                .child(
                                    Button::new("browser-open-folder")
                                        .outline()
                                        .small()
                                        .label("Abrir pasta")
                                        .on_click(move |_, _, cx| cx.reveal_path(&dir.join("manifest.json"))),
                                ),
                        ),
                );
            }
            if let Some(e) = &self.browser_error {
                rows.push(
                    div()
                        .px_4()
                        .py_3()
                        .text_sm()
                        .text_color(cx.theme().danger)
                        .child(e.clone()),
                );
            }
            Self::group("NAVEGADOR", rows, cx)
        };
        let unlock_control = self.number_choice(
            "settings-unlock-memory",
            "unlock",
            &[(0, "Sempre"), (1, "1 dia"), (7, "7 dias"), (30, "30 dias")],
            self.unlock_memory_days,
            Self::set_unlock_memory_days,
            cx,
        );
        let relogin_control = self.number_choice(
            "settings-relogin",
            "relogin",
            &[
                (7, "7 dias"),
                (30, "30 dias"),
                (90, "90 dias"),
                (0, "Nunca"),
            ],
            self.relogin_days,
            Self::set_relogin_days,
            cx,
        );
        let lock_control = self.number_choice(
            "settings-auto-lock",
            "lock",
            &[
                (1, "1 min"),
                (5, "5 min"),
                (15, "15 min"),
                (60, "1 h"),
                (0, "Nunca"),
            ],
            self.auto_lock_minutes,
            Self::set_auto_lock_minutes,
            cx,
        );
        let session_group = Self::group(
            "SESSÃO",
            vec![
                Self::row(
                    "Pedir a senha mestra ao abrir",
                    "Depois de digitar a senha, o app abre direto por esse tempo. Quem usar este computador com o seu usuário do Windows abre o cofre sem senha nesse período. Se a senha mudar, ela volta a ser pedida.",
                    unlock_control,
                    cx,
                ),
                Self::row(
                    "Pedir login completo a cada",
                    "E-mail, senha mestra e 2FA de novo depois desse tempo. Trocar a senha ou encerrar a sessão em outro lugar também pede login.",
                    relogin_control,
                    cx,
                ),
                Self::row(
                    "Bloquear sem uso após",
                    "Tempo sem mexer no teclado ou no mouse até o cofre bloquear.",
                    lock_control,
                    cx,
                ),
                Self::row(
                    "Busca rápida (Ctrl+Shift+Espaço)",
                    "Em qualquer app, abre uma janelinha para achar um item e preencher, copiar o usuário, a senha ou o código, sem abrir o Scout Vault.",
                    Switch::new("quick-search-setting")
                        .checked(self.quick_search)
                        .on_click(cx.listener(|this, v: &bool, _, cx| {
                            this.set_quick_search(*v, cx)
                        })),
                    cx,
                ),
                Self::row(
                    "Sincronizar em segundo plano",
                    "A cada minuto o app pergunta ao servidor se o cofre mudou (no celular, no cofre web…) e, se mudou, atualiza sozinho. A pergunta não baixa o cofre.",
                    Switch::new("background-sync")
                        .checked(self.background_sync)
                        .on_click(cx.listener(|this, v: &bool, _, cx| {
                            this.set_background_sync(*v, cx)
                        })),
                    cx,
                ),
                Self::row(
                    "Desbloquear com Windows Hello",
                    if crate::hello::available(cx) {
                        "Rosto, digital ou PIN do Windows no lugar da senha mestra. A chave fica cifrada pelo Windows Hello; se a senha mestra mudar, ligue de novo."
                    } else {
                        "O Windows Hello não está configurado neste computador (Configurações do Windows › Contas › Opções de entrada)."
                    },
                    Switch::new("windows-hello")
                        .checked(self.windows_hello)
                        .disabled(!crate::hello::available(cx) && !self.windows_hello)
                        .on_click(cx.listener(|this, v: &bool, _, cx| {
                            this.set_windows_hello(*v, cx)
                        })),
                    cx,
                ),
                Self::row(
                    "Bloquear junto com o Windows",
                    "Ao bloquear o computador (Win+L) ou ao suspender/hibernar, o cofre também bloqueia.",
                    Switch::new("lock-with-windows")
                        .checked(self.lock_with_windows)
                        .on_click(cx.listener(|this, v: &bool, _, cx| {
                            this.set_lock_with_windows(*v, cx)
                        })),
                    cx,
                ),
                Self::row(
                    "Ao fechar, continuar na bandeja",
                    "O app segue rodando perto do relógio: Ctrl+Alt+A e o navegador continuam funcionando. Para sair de vez, use Sair no ícone.",
                    Switch::new("tray-on-close")
                        .checked(self.close_to_tray)
                        .on_click(cx.listener(|this, v: &bool, _, cx| this.set_close_to_tray(*v, cx))),
                    cx,
                ),
                Self::row(
                "Lembrar sessão",
                "Ao abrir o app, pede só a senha mestra. O cofre fica guardado cifrado neste computador e abre mesmo sem internet.",
                Switch::new("remember-session")
                    .checked(self.remember_session)
                    .on_click(cx.listener(|this, v: &bool, _, cx| {
                        this.set_remember_session(*v, cx)
                    })),
                cx,
            )],
            cx,
        );

        let header = h_flex()
            .justify_between()
            .child(
                v_flex()
                    .gap_1()
                    .child(div().text_2xl().font_semibold().child("Configurações"))
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("As mudanças valem na hora e ficam salvas neste computador."),
                    ),
            )
            .child(
                Button::new("settings-close")
                    .ghost()
                    .small()
                    .icon(IconName::Close)
                    .tooltip("Fechar (Esc)")
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::Close))),
            );

        div()
            .id("settings")
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .overflow_y_scroll()
            .child(
                v_flex()
                    .w_full()
                    .max_w(px(680.))
                    .mx_auto()
                    .px_8()
                    .py_10()
                    .gap_8()
                    .child(header)
                    .child(appearance_group)
                    .child(session_group)
                    .child(browser_group)
                    .child(autotype_group),
            )
    }
}
