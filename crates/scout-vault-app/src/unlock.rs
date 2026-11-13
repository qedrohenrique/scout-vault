//! Tela de desbloqueio: o cofre continua sincronizado (cifrado) em memória;
//! basta a senha mestra para abri-lo de novo, sem rede.

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
}

pub struct UnlockView {
    session: Session,
    password: Entity<InputState>,
    busy: bool,
    error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<UnlockEvent> for UnlockView {}

impl UnlockView {
    pub fn new(session: Session, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let password = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("Senha mestra")
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
            error: None,
            _subscriptions: vec![sub],
        }
    }

    pub fn error(&self) -> Option<&SharedString> {
        self.error.as_ref()
    }

    pub fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let password = Zeroizing::new(self.password.read(cx).value().to_string());
        if password.is_empty() {
            self.error = Some("Informe a senha mestra.".into());
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
            (None, true) => "Abrindo o cofre…".into(),
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
                            .label("Desbloquear")
                            .loading(self.busy)
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                    )
                    .child(
                        Button::new("unlock-logout")
                            .ghost()
                            .w_full()
                            .label("Sair da conta")
                            .disabled(self.busy)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(UnlockEvent::LogOut))),
                    ),
            );
        crate::ui::entry_screen(
            "unlock-settings",
            "Cofre bloqueado",
            self.session.email.clone(),
            px(380.),
            body,
            cx.listener(|_, _, _, cx| cx.emit(UnlockEvent::OpenSettings)),
            cx,
        )
    }
}
