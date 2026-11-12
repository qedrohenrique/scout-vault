//! Tela de login: servidor, e-mail, senha mestra, segundo fator e código de
//! novo dispositivo.

use std::sync::Arc;

use cofre_core::api::{Client, SecondFactor, TwoFactorProvider};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::label::Label;
use gpui_kit::component::radio::RadioGroup;
use gpui_kit::component::{button::*, *};
use gpui_kit::prelude::*;
use gpui_kit::*;
use zeroize::Zeroizing;

use crate::flow::{self, Handoff, LoginOutcome, PendingLogin};
use crate::settings::{ServerChoice, Settings, SettingsStore};

pub enum LoginEvent {
    Unlocked(Handoff),
}

enum Step {
    Credentials,
    TwoFactor {
        pending: PendingLogin,
        providers: Vec<TwoFactorProvider>,
        selected: TwoFactorProvider,
    },
    NewDevice {
        pending: PendingLogin,
    },
}

pub struct LoginView {
    store: SettingsStore,
    settings: Settings,
    server_url: Entity<InputState>,
    email: Entity<InputState>,
    password: Entity<InputState>,
    code: Entity<InputState>,
    remember_device: bool,
    step: Step,
    busy: Option<SharedString>,
    error: Option<SharedString>,
    info: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<LoginEvent> for LoginView {}

impl LoginView {
    pub fn new(store: SettingsStore, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = store.load();
        let server_url = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("https://vault.exemplo.com")
                .default_value(settings.self_hosted_url.clone())
        });
        let email = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("voce@exemplo.com")
                .default_value(settings.last_email.clone())
        });
        let password = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("Senha mestra")
        });
        let code = cx.new(|cx| InputState::new(window, cx).placeholder("Código"));

        let submit_on_enter =
            |input: &Entity<InputState>, window: &mut Window, cx: &mut Context<Self>| {
                cx.subscribe_in(input, window, |this, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        this.submit(window, cx);
                    }
                })
            };
        let subscriptions = vec![
            submit_on_enter(&email, window, cx),
            submit_on_enter(&password, window, cx),
            submit_on_enter(&code, window, cx),
        ];

        Self {
            store,
            settings,
            server_url,
            email,
            password,
            code,
            remember_device: false,
            step: Step::Credentials,
            busy: None,
            error: None,
            info: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn error(&self) -> Option<&SharedString> {
        self.error.as_ref()
    }

    pub fn is_busy(&self) -> bool {
        self.busy.is_some()
    }

    fn set_server(&mut self, choice: ServerChoice, cx: &mut Context<Self>) {
        self.settings.server = choice;
        cx.notify();
    }

    pub fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        self.error = None;
        self.info = None;

        match std::mem::replace(&mut self.step, Step::Credentials) {
            Step::Credentials => self.submit_credentials(window, cx),
            Step::TwoFactor {
                pending,
                providers,
                selected,
            } => {
                let token = self.code.read(cx).value().trim().to_owned();
                if token.is_empty() {
                    self.error = Some("Digite o código.".into());
                    self.step = Step::TwoFactor {
                        pending,
                        providers,
                        selected,
                    };
                    cx.notify();
                    return;
                }
                let second = SecondFactor {
                    provider: selected,
                    token,
                    remember: self.remember_device,
                };
                self.run(
                    "Verificando o código…",
                    flow::continue_login(pending, Some(second), None),
                    cx,
                );
            }
            Step::NewDevice { pending } => {
                let code = self.code.read(cx).value().trim().to_owned();
                if code.is_empty() {
                    self.error = Some("Digite o código enviado por e-mail.".into());
                    self.step = Step::NewDevice { pending };
                    cx.notify();
                    return;
                }
                self.run(
                    "Verificando o dispositivo…",
                    flow::continue_login(pending, None, Some(code)),
                    cx,
                );
            }
        }
    }

    fn submit_credentials(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let email = self.email.read(cx).value().trim().to_owned();
        let password = Zeroizing::new(self.password.read(cx).value().to_string());
        self.settings.self_hosted_url = self.server_url.read(cx).value().trim().to_owned();

        let validation = if email.is_empty() || !email.contains('@') {
            Err("Informe o e-mail da conta.".to_owned())
        } else if password.is_empty() {
            Err("Informe a senha mestra.".to_owned())
        } else {
            self.settings.server_urls()
        };
        let urls = match validation {
            Ok(urls) => urls,
            Err(message) => {
                self.error = Some(message.into());
                cx.notify();
                return;
            }
        };

        self.settings.last_email = email.clone();
        self.store.save(&self.settings);

        let client = match Client::new(urls, self.settings.device.clone()) {
            Ok(c) => Arc::new(c),
            Err(e) => {
                self.error = Some(flow::describe_api_error(e).into());
                cx.notify();
                return;
            }
        };
        self.run(
            "Entrando… (derivar a chave leva alguns segundos)",
            flow::start_login(client, email, password),
            cx,
        );
    }

    fn run(
        &mut self,
        message: &'static str,
        work: impl std::future::Future<Output = Result<LoginOutcome, String>> + 'static,
        cx: &mut Context<Self>,
    ) {
        self.busy = Some(message.into());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = work.await;
            this.update(cx, |this, cx| this.finish(result, cx)).ok();
        })
        .detach();
    }

    fn finish(&mut self, result: Result<LoginOutcome, String>, cx: &mut Context<Self>) {
        self.busy = None;
        match result {
            Ok(LoginOutcome::Done(unlocked)) => {
                self.step = Step::Credentials;
                cx.emit(LoginEvent::Unlocked(Handoff::new(unlocked)));
            }
            Ok(LoginOutcome::NeedsTwoFactor { pending, providers }) => {
                let supported = flow::supported_providers(&providers);
                match supported.first().copied() {
                    Some(selected) => {
                        self.info = Some(
                            format!("Digite o código do seu {}.", flow::provider_name(selected))
                                .into(),
                        );
                        self.step = Step::TwoFactor {
                            pending,
                            providers: supported,
                            selected,
                        };
                    }
                    None => {
                        let names: Vec<_> =
                            providers.iter().map(|p| flow::provider_name(*p)).collect();
                        self.error = Some(
                            format!(
                                "Esta conta exige {}, que o Cofre ainda não suporta.",
                                names.join(" ou ")
                            )
                            .into(),
                        );
                    }
                }
            }
            Ok(LoginOutcome::NeedsNewDeviceCode(pending)) => {
                self.info = Some(
                    "Primeiro acesso deste dispositivo: digite o código enviado para o seu e-mail."
                        .into(),
                );
                self.step = Step::NewDevice { pending };
            }
            Err(message) => {
                self.step = Step::Credentials;
                self.error = Some(message.into());
            }
        }
        cx.notify();
    }

    fn select_provider(&mut self, provider: TwoFactorProvider, cx: &mut Context<Self>) {
        if let Step::TwoFactor { selected, .. } = &mut self.step {
            *selected = provider;
            self.info =
                Some(format!("Digite o código do seu {}.", flow::provider_name(provider)).into());
            cx.notify();
        }
    }

    fn send_email_code(&mut self, cx: &mut Context<Self>) {
        let Step::TwoFactor { pending, .. } = &self.step else {
            return;
        };
        // `PendingLogin` fica no passo atual; só o necessário vai para a tarefa.
        let request = flow::send_email_code(pending);
        self.busy = Some("Enviando código…".into());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |this, cx| {
                this.busy = None;
                match result {
                    Ok(()) => this.info = Some("Código enviado. Confira o seu e-mail.".into()),
                    Err(e) => this.error = Some(e.into()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        self.step = Step::Credentials;
        self.error = None;
        self.info = None;
        cx.notify();
    }

    fn render_credentials(&self, cx: &mut Context<Self>) -> AnyElement {
        let busy = self.busy.is_some();
        v_flex()
            .gap_3()
            .child(
                v_flex()
                    .gap_1()
                    .child(Label::new("Servidor").text_sm())
                    .child(
                        RadioGroup::horizontal("login-server")
                            .children(ServerChoice::ALL.map(|c| c.label()))
                            .selected_index(Some(self.settings.server.index()))
                            .on_change(cx.listener(|this, ix: &usize, _, cx| {
                                this.set_server(ServerChoice::ALL[*ix], cx);
                            })),
                    ),
            )
            .when(self.settings.server == ServerChoice::SelfHosted, |el| {
                el.child(
                    Input::new(&self.server_url)
                        .id("login-server-url")
                        .disabled(busy),
                )
            })
            .child(
                v_flex()
                    .gap_1()
                    .child(Label::new("E-mail").text_sm())
                    .child(Input::new(&self.email).id("login-email").disabled(busy)),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(Label::new("Senha mestra").text_sm())
                    .child(
                        Input::new(&self.password)
                            .id("login-password")
                            .mask_toggle()
                            .disabled(busy),
                    ),
            )
            .into_any_element()
    }

    fn render_code_step(&self, cx: &mut Context<Self>) -> AnyElement {
        let busy = self.busy.is_some();
        let mut col = v_flex().gap_3();

        if let Step::TwoFactor {
            providers,
            selected,
            ..
        } = &self.step
        {
            if providers.len() > 1 {
                let options = providers.clone();
                let selected_ix = providers.iter().position(|p| p == selected);
                col = col.child(
                    RadioGroup::horizontal("login-2fa-provider")
                        .children(providers.iter().map(|p| flow::provider_name(*p)))
                        .selected_index(selected_ix)
                        .on_change(cx.listener(move |this, ix: &usize, _, cx| {
                            this.select_provider(options[*ix], cx);
                        })),
                );
            }
            if *selected == TwoFactorProvider::Email {
                col = col.child(
                    Button::new("login-send-email-code")
                        .outline()
                        .label("Enviar código por e-mail")
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| this.send_email_code(cx))),
                );
            }
        }

        col = col.child(Input::new(&self.code).id("login-code").disabled(busy));

        if matches!(self.step, Step::TwoFactor { .. }) {
            col = col.child(
                Checkbox::new("login-remember-device")
                    .label("Lembrar este dispositivo")
                    .checked(self.remember_device)
                    .on_change(cx.listener(|this, value: &bool, _, cx| {
                        this.remember_device = *value;
                        cx.notify();
                    })),
            );
        }

        col.child(
            Button::new("login-cancel")
                .ghost()
                .label("Voltar")
                .disabled(busy)
                .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
        )
        .into_any_element()
    }
}

impl Render for LoginView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.busy.is_some();
        let (title, form) = match self.step {
            Step::Credentials => ("Entrar no Cofre", self.render_credentials(cx)),
            Step::TwoFactor { .. } => ("Verificação em duas etapas", self.render_code_step(cx)),
            Step::NewDevice { .. } => ("Verificar este dispositivo", self.render_code_step(cx)),
        };
        let status: SharedString = self
            .error
            .clone()
            .or_else(|| self.busy.clone())
            .or_else(|| self.info.clone())
            .unwrap_or_default();
        let status_color = if self.error.is_some() {
            cx.theme().danger
        } else {
            cx.theme().muted_foreground
        };

        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                v_flex()
                    .w(px(420.))
                    .gap_4()
                    .p_6()
                    .rounded(cx.theme().radius_lg)
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().secondary)
                    .child(
                        h_flex()
                            .gap_2()
                            .child(Icon::new(gpui_kit::assets::IconName::LockKeyhole))
                            .child(div().text_lg().font_semibold().child(title)),
                    )
                    .child(form)
                    .child(
                        div()
                            .id("login-status")
                            .role(Role::Status)
                            .test_support()
                            .aria_label(status.clone())
                            .min_h(px(20.))
                            .text_sm()
                            .text_color(status_color)
                            .child(status),
                    )
                    .child(
                        Button::new("login-submit")
                            .primary()
                            .label("Continuar")
                            .loading(busy)
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                    ),
            )
    }
}
