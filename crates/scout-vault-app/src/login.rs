//! Login screen: server, email, master password, second factor and new
//! device code.

use std::sync::Arc;

use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::label::Label;
use gpui_kit::component::radio::RadioGroup;
use gpui_kit::component::{button::*, *};
use gpui_kit::prelude::*;
use gpui_kit::*;
use scout_vault_core::api::{Client, SecondFactor, TwoFactorProvider};
use zeroize::Zeroizing;

use crate::flow::{self, Handoff, LoginOutcome, PendingLogin};
use crate::settings::{ServerChoice, Settings, SettingsStore};

pub enum LoginEvent {
    Unlocked(Handoff),
    OpenSettings,
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
    sso_identifier: Entity<InputState>,
    /// Sign in with company SSO instead of email + password.
    use_sso: bool,
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
                .placeholder(t!("login.server_placeholder"))
                .default_value(settings.self_hosted_url.clone())
        });
        let email = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("login.email_placeholder"))
                .default_value(settings.last_email.clone())
        });
        let password = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder(t!("common.vault_password"))
        });
        let code =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("login.code_placeholder")));
        let sso_identifier = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("login.sso_placeholder"))
                .default_value(settings.last_sso_identifier.clone())
        });

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
            submit_on_enter(&sso_identifier, window, cx),
        ];

        Self {
            store,
            settings,
            server_url,
            email,
            password,
            code,
            sso_identifier,
            use_sso: false,
            remember_device: false,
            step: Step::Credentials,
            busy: None,
            error: None,
            info: None,
            _subscriptions: subscriptions,
        }
    }

    /// Debug builds only: fills in and submits the login from
    /// `SCOUT_VAULT_TEST_URL/EMAIL/PASSWORD` (see `dev/test-account.env`) when
    /// `SCOUT_VAULT_DEV_AUTOLOGIN` is set. Refuses servers outside localhost.
    #[cfg(debug_assertions)]
    pub fn dev_autologin(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Ok(url), Ok(email), Ok(password)) = (
            std::env::var("SCOUT_VAULT_TEST_URL"),
            std::env::var("SCOUT_VAULT_TEST_EMAIL"),
            std::env::var("SCOUT_VAULT_TEST_PASSWORD"),
        ) else {
            return;
        };
        let local = ["http://localhost", "http://127.0.0.1"]
            .iter()
            .any(|prefix| url.starts_with(prefix));
        if !local {
            return;
        }
        self.settings.server = ServerChoice::SelfHosted;
        self.server_url
            .update(cx, |i, cx| i.set_value(url, window, cx));
        self.email
            .update(cx, |i, cx| i.set_value(email, window, cx));
        self.password
            .update(cx, |i, cx| i.set_value(password, window, cx));
        self.submit(window, cx);
    }

    /// Informational message shown above the form (e.g. session expired).
    /// Rebuilds the field placeholders in the current language.
    pub fn relabel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let hints = [
            (&self.server_url, t!("login.server_placeholder")),
            (&self.email, t!("login.email_placeholder")),
            (&self.password, t!("common.vault_password")),
            (&self.code, t!("login.code_placeholder")),
            (&self.sso_identifier, t!("login.sso_placeholder")),
        ];
        for (input, hint) in hints {
            input.update(cx, |s, cx| s.set_placeholder(hint, window, cx));
        }
        cx.notify();
    }

    pub fn set_notice(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.info = Some(message.into());
        cx.notify();
    }

    pub fn info(&self) -> Option<&SharedString> {
        self.info.as_ref()
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
                if selected == TwoFactorProvider::WebAuthn {
                    self.run(
                        t!("login.webauthn_busy"),
                        flow::continue_with_webauthn(pending, self.remember_device),
                        cx,
                    );
                    return;
                }
                let mut token = self.code.read(cx).value().trim().to_owned();
                if matches!(
                    selected,
                    TwoFactorProvider::Duo | TwoFactorProvider::OrganizationDuo
                ) && !token.is_empty()
                {
                    match flow::duo_token(&token) {
                        Some(t) => token = t,
                        None => {
                            self.error = Some(t!("login.duo_paste_url").into());
                            self.step = Step::TwoFactor {
                                pending,
                                providers,
                                selected,
                            };
                            cx.notify();
                            return;
                        }
                    }
                }
                if token.is_empty() {
                    self.error = Some(t!("login.enter_code").into());
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
                    t!("login.checking_code"),
                    flow::continue_login(pending, Some(second), None),
                    cx,
                );
            }
            Step::NewDevice { pending } => {
                let code = self.code.read(cx).value().trim().to_owned();
                if code.is_empty() {
                    self.error = Some(t!("login.enter_email_code").into());
                    self.step = Step::NewDevice { pending };
                    cx.notify();
                    return;
                }
                self.run(
                    t!("login.checking_device"),
                    flow::continue_login(pending, None, Some(code)),
                    cx,
                );
            }
        }
    }

    fn submit_credentials(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.use_sso {
            self.submit_sso(window, cx);
            return;
        }
        let email = self.email.read(cx).value().trim().to_owned();
        let password = Zeroizing::new(self.password.read(cx).value().to_string());
        self.settings.self_hosted_url = self.server_url.read(cx).value().trim().to_owned();

        let validation = if email.is_empty() || !email.contains('@') {
            Err(t!("login.email_required").to_owned())
        } else if password.is_empty() {
            Err(t!("login.password_required").to_owned())
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
            t!("login.signing_in"),
            flow::start_login(client, email, password, self.store.secrets().clone()),
            cx,
        );
    }

    fn submit_sso(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let identifier = self.sso_identifier.read(cx).value().trim().to_owned();
        let password = Zeroizing::new(self.password.read(cx).value().to_string());
        self.settings.self_hosted_url = self.server_url.read(cx).value().trim().to_owned();

        let validation = if identifier.is_empty() {
            Err(t!("login.sso_identifier_required").to_owned())
        } else if password.is_empty() {
            Err(t!("login.sso_password_required").to_owned())
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
        self.settings.last_sso_identifier = identifier.clone();
        self.store.save(&self.settings);

        let client = match Client::for_sso(urls, self.settings.device.clone()) {
            Ok(c) => Arc::new(c),
            Err(e) => {
                self.error = Some(flow::describe_api_error(e).into());
                cx.notify();
                return;
            }
        };
        self.run(
            t!("login.sso_busy"),
            flow::start_sso_login(client, identifier, password),
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
                        self.info = Some(flow::provider_prompt(selected).into());
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
                            t!("login.unsupported_2fa", names = names.join(t!("login.or"))).into(),
                        );
                    }
                }
            }
            Ok(LoginOutcome::WrongCode { pending, message }) => {
                self.error = Some(message.into());
                if pending.is_new_device() {
                    self.step = Step::NewDevice { pending };
                } else {
                    let providers = flow::supported_providers(pending.providers());
                    let selected = pending
                        .last_provider()
                        .filter(|p| providers.contains(p))
                        .or_else(|| providers.first().copied());
                    match selected {
                        Some(selected) => {
                            self.step = Step::TwoFactor {
                                pending,
                                providers,
                                selected,
                            }
                        }
                        None => self.step = Step::Credentials,
                    }
                }
            }
            Ok(LoginOutcome::NeedsNewDeviceCode(pending)) => {
                self.info = Some(t!("login.new_device").into());
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
            self.info = Some(flow::provider_prompt(provider).into());
            cx.notify();
        }
    }

    fn send_email_code(&mut self, cx: &mut Context<Self>) {
        let Step::TwoFactor { pending, .. } = &self.step else {
            return;
        };
        // `PendingLogin` stays on the current step; only what is needed goes to the task.
        let request = flow::send_email_code(pending);
        self.busy = Some(t!("login.sending_code").into());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |this, cx| {
                this.busy = None;
                match result {
                    Ok(()) => this.info = Some(t!("login.code_sent").into()),
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
                    .child(Label::new(t!("login.server")).text_sm())
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
            .when(!self.use_sso, |el| {
                el.child(
                    v_flex()
                        .gap_1()
                        .child(Label::new(t!("login.email")).text_sm())
                        .child(Input::new(&self.email).id("login-email").disabled(busy)),
                )
            })
            .when(self.use_sso, |el| {
                el.child(
                    v_flex()
                        .gap_1()
                        .child(Label::new(t!("login.sso_identifier")).text_sm())
                        .child(
                            Input::new(&self.sso_identifier)
                                .id("login-sso-identifier")
                                .disabled(busy),
                        ),
                )
            })
            .child(
                v_flex()
                    .gap_1()
                    .child(Label::new(t!("common.vault_password")).text_sm())
                    .child(
                        Input::new(&self.password)
                            .id("login-password")
                            .mask_toggle()
                            .disabled(busy),
                    ),
            )
            .child(
                Checkbox::new("login-use-sso")
                    .label(t!("login.use_sso"))
                    .checked(self.use_sso)
                    .disabled(busy)
                    .on_change(cx.listener(|this, value: &bool, _, cx| {
                        this.use_sso = *value;
                        this.error = None;
                        cx.notify();
                    })),
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
                        .label(t!("login.send_email_code"))
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| this.send_email_code(cx))),
                );
            }
        }

        let uses_key = matches!(
            &self.step,
            Step::TwoFactor { selected, .. } if *selected == TwoFactorProvider::WebAuthn
        );
        let duo = match &self.step {
            Step::TwoFactor {
                pending, selected, ..
            } if matches!(
                selected,
                TwoFactorProvider::Duo | TwoFactorProvider::OrganizationDuo
            ) =>
            {
                Some(flow::duo_auth_url(pending, *selected))
            }
            _ => None,
        };
        if let Some(url) = duo {
            col = col.child(
                Button::new("login-duo-open")
                    .outline()
                    .label(t!("login.open_duo"))
                    .disabled(busy || url.is_none())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(url) = &url {
                            if let Err(e) = crate::sso::open_browser(url) {
                                this.error = Some(e.into());
                                cx.notify();
                            }
                        }
                    })),
            );
        }
        if uses_key {
            col = col.child(
                Button::new("login-webauthn")
                    .outline()
                    .label(t!("login.use_webauthn"))
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
            );
        } else {
            col = col.child(Input::new(&self.code).id("login-code").disabled(busy));
        }

        if matches!(self.step, Step::TwoFactor { .. }) {
            col = col.child(
                Checkbox::new("login-remember-device")
                    .label(t!("login.remember_device"))
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
                .label(t!("common.back"))
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
            Step::Credentials => (t!("login.title"), self.render_credentials(cx)),
            Step::TwoFactor { .. } => (t!("login.title_2fa"), self.render_code_step(cx)),
            Step::NewDevice { .. } => (t!("login.title_new_device"), self.render_code_step(cx)),
        };
        let status: SharedString = self
            .error
            .clone()
            .or_else(|| self.busy.clone())
            .or_else(|| self.info.clone())
            .unwrap_or_default();
        let subtitle = match self.step {
            Step::Credentials => t!("login.subtitle"),
            Step::TwoFactor { .. } => t!("login.subtitle_2fa"),
            Step::NewDevice { .. } => t!("login.subtitle_new_device"),
        };
        let body = v_flex()
            .gap_4()
            .child(form)
            .child(crate::ui::status_line(
                "login-status",
                status,
                self.error.is_some(),
                cx,
            ))
            .child(
                Button::new("login-submit")
                    .primary()
                    .w_full()
                    .label(t!("common.continue"))
                    .loading(busy)
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
            );
        crate::ui::entry_screen(
            "login-settings",
            title,
            subtitle,
            px(420.),
            body,
            cx.listener(|_, _, _, cx| cx.emit(LoginEvent::OpenSettings)),
            cx,
        )
    }
}
