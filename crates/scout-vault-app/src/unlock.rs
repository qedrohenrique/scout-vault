//! Unlock screen: the vault stays synced (encrypted) in memory;
//! the master password is enough to open it again, without network.

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{button::*, *};
use gpui_kit::prelude::*;
use gpui_kit::*;
use zeroize::Zeroizing;

use crate::flow::{self, Handoff, Session, Unlocked};
use crate::io;

pub enum UnlockEvent {
    Unlocked(Handoff),
    LogOut,
    OpenSettings,
    /// Asked to open with Windows Hello (the app drives it).
    UseHello,
}

pub struct UnlockView {
    session: Session,
    password: Entity<InputState>,
    busy: bool,
    /// The "Use Windows Hello" button shows up (option on and set up).
    hello: bool,
    error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<UnlockEvent> for UnlockView {}

impl UnlockView {
    pub fn new(session: Session, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let password = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder(t!("common.vault_password"))
        });
        let sub = cx.subscribe_in(
            &password,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.submit(window, cx);
                }
            },
        );
        Self {
            session,
            password,
            busy: false,
            hello: false,
            error: None,
            _subscriptions: vec![sub],
        }
    }

    /// Rebuilds the field placeholder in the current language.
    pub fn relabel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.password.update(cx, |s, cx| {
            s.set_placeholder(t!("common.vault_password"), window, cx)
        });
        cx.notify();
    }

    pub fn error(&self) -> Option<&SharedString> {
        self.error.as_ref()
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    pub fn set_hello(&mut self, on: bool, cx: &mut Context<Self>) {
        self.hello = on;
        cx.notify();
    }

    /// Windows Hello progress (the app reports when it starts and when it ends).
    pub fn set_hello_progress(
        &mut self,
        busy: bool,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.busy = busy;
        self.error = error.map(Into::into);
        cx.notify();
    }

    pub fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let password = Zeroizing::new(self.password.read(cx).value().to_string());
        if password.is_empty() {
            self.error = Some(t!("login.password_required").into());
            cx.notify();
            return;
        }
        self.password
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.busy = true;
        self.error = None;
        cx.notify();

        let session = self.session.clone();
        cx.spawn(async move |this, cx| {
            let result = io::blocking(move || {
                flow::unlock(&session, &password).map(|(keys, vault)| Unlocked {
                    session,
                    keys,
                    vault,
                })
            })
            .await;
            this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(unlocked) => {
                        cx.emit(UnlockEvent::Unlocked(Handoff::new(Box::new(unlocked))))
                    }
                    Err(e) => this.error = Some(e.into()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

impl Render for UnlockView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let status: SharedString = match (&self.error, self.busy) {
            (Some(e), _) => e.clone(),
            (None, true) if self.hello => t!("unlock.confirm_hello").into(),
            (None, true) => t!("unlock.opening").into(),
            (None, false) => "".into(),
        };
        let body = v_flex()
            .gap_4()
            .child(
                Input::new(&self.password)
                    .id("unlock-password")
                    .mask_toggle()
                    .disabled(self.busy),
            )
            .child(crate::ui::status_line(
                "unlock-status",
                status,
                self.error.is_some(),
                cx,
            ))
            .child(
                v_flex()
                    .gap_2()
                    .child(
                        Button::new("unlock-submit")
                            .primary()
                            .w_full()
                            .label(t!("unlock.submit"))
                            .loading(self.busy)
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                    )
                    .when(self.hello, |el| {
                        el.child(
                            Button::new("unlock-hello")
                                .outline()
                                .w_full()
                                .icon(gpui_kit::assets::IconName::ScanFace)
                                .label(t!("unlock.use_hello"))
                                .disabled(self.busy)
                                .on_click(
                                    cx.listener(|_, _, _, cx| cx.emit(UnlockEvent::UseHello)),
                                ),
                        )
                    })
                    .child(
                        Button::new("unlock-logout")
                            .ghost()
                            .w_full()
                            .label(t!("unlock.log_out"))
                            .disabled(self.busy)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(UnlockEvent::LogOut))),
                    ),
            );
        crate::ui::entry_screen(
            "unlock-settings",
            t!("app.vault_locked"),
            self.session.email.clone(),
            px(380.),
            body,
            cx.listener(|_, _, _, cx| cx.emit(UnlockEvent::OpenSettings)),
            cx,
        )
    }
}
