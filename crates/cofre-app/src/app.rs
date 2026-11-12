//! Raiz da janela: decide qual tela aparece e bloqueia por inatividade.

use std::time::{Duration, Instant};

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::appearance::{self, ThemeChoice};
use crate::flow::{Handoff, Session, Unlocked};
use crate::login::{LoginEvent, LoginView};
use crate::settings::SettingsStore;
use crate::settings_view::{SettingsEvent, SettingsView};
use crate::unlock::{UnlockEvent, UnlockView};
use crate::vault_view::{VaultEvent, VaultView};

pub const DEFAULT_AUTO_LOCK: Duration = Duration::from_secs(5 * 60);
const IDLE_CHECK_EVERY: Duration = Duration::from_secs(10);

enum Screen {
    Login(Entity<LoginView>),
    Locked(Entity<UnlockView>),
    Vault(Entity<VaultView>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenKind {
    Login,
    Locked,
    Vault,
}

pub struct CofreApp {
    store: SettingsStore,
    screen: Screen,
    last_activity: Instant,
    auto_lock_after: Duration,
    /// Configurações abertas por cima da tela atual (que segue viva por baixo).
    settings: Option<(Entity<SettingsView>, Subscription)>,
    _screen_subscription: Subscription,
    _appearance_subscription: Subscription,
    _idle_task: Task<()>,
}

impl CofreApp {
    pub fn new(store: SettingsStore, window: &mut Window, cx: &mut Context<Self>) -> Self {
        appearance::apply(&store.load().appearance, Some(window), cx);
        let (screen, sub) = Self::login_screen(&store, window, cx);
        Self::build(store, screen, sub, window, cx)
    }

    /// Começa já com o cofre aberto (usado pelos testes de UI).
    pub fn with_unlocked(
        store: SettingsStore,
        unlocked: Unlocked,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        appearance::apply(&store.load().appearance, Some(window), cx);
        let (screen, sub) = Self::vault_screen(unlocked, window, cx);
        Self::build(store, screen, sub, window, cx)
    }

    fn build(
        store: SettingsStore,
        screen: Screen,
        sub: Subscription,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let idle_task = cx.spawn_in(window, async move |this, cx| loop {
            cx.background_executor().timer(IDLE_CHECK_EVERY).await;
            let alive = this.update_in(cx, |this, window, cx| {
                this.lock_if_idle(Instant::now(), window, cx)
            });
            if alive.is_err() {
                break;
            }
        });
        // Tema "Sistema": acompanha a troca de claro/escuro do Windows ao vivo.
        let appearance_sub = cx.observe_window_appearance(window, |this, window, cx| {
            let current = this.store.load().appearance;
            if current.theme == ThemeChoice::System {
                appearance::apply(&current, Some(window), cx);
            }
        });
        Self {
            store,
            screen,
            last_activity: Instant::now(),
            auto_lock_after: DEFAULT_AUTO_LOCK,
            settings: None,
            _screen_subscription: sub,
            _appearance_subscription: appearance_sub,
            _idle_task: idle_task,
        }
    }

    pub fn settings_view(&self) -> Option<&Entity<SettingsView>> {
        self.settings.as_ref().map(|(v, _)| v)
    }

    pub fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings.is_some() {
            return;
        }
        let view = cx.new(|_| SettingsView::new(self.store.clone()));
        let sub = cx.subscribe_in(
            &view,
            window,
            |this, _, event: &SettingsEvent, _, cx| match event {
                SettingsEvent::Close => this.close_settings(cx),
            },
        );
        self.settings = Some((view, sub));
        cx.notify();
    }

    pub fn close_settings(&mut self, cx: &mut Context<Self>) {
        if self.settings.take().is_some() {
            cx.notify();
        }
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.record_activity();
        let k = &event.keystroke;
        if k.key == "," && k.modifiers.secondary(cx) {
            if self.settings.is_some() {
                self.close_settings(cx);
            } else {
                self.open_settings(window, cx);
            }
        } else if k.key == "escape" && self.settings.is_some() {
            self.close_settings(cx);
        }
    }

    pub fn screen_kind(&self) -> ScreenKind {
        match self.screen {
            Screen::Login(_) => ScreenKind::Login,
            Screen::Locked(_) => ScreenKind::Locked,
            Screen::Vault(_) => ScreenKind::Vault,
        }
    }

    pub fn vault_view(&self) -> Option<&Entity<VaultView>> {
        match &self.screen {
            Screen::Vault(v) => Some(v),
            _ => None,
        }
    }

    pub fn set_auto_lock_after(&mut self, after: Duration) {
        self.auto_lock_after = after;
    }

    pub fn record_activity(&mut self) {
        self.last_activity = Instant::now();
    }

    /// Bloqueia se o cofre estiver aberto e ninguém mexeu desde `auto_lock_after`.
    pub fn lock_if_idle(&mut self, now: Instant, window: &mut Window, cx: &mut Context<Self>) {
        if now.saturating_duration_since(self.last_activity) >= self.auto_lock_after {
            self.lock(window, cx);
        }
    }

    pub fn lock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Screen::Vault(view) = &self.screen {
            let session = view.read(cx).session().clone();
            self.show_locked(session, window, cx);
        }
    }

    fn set_screen(&mut self, (screen, sub): (Screen, Subscription), cx: &mut Context<Self>) {
        // Trocar a tela solta a anterior: o cofre decifrado sai da memória junto.
        self.screen = screen;
        self._screen_subscription = sub;
        self.record_activity();
        cx.notify();
    }

    fn show_login(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let next = Self::login_screen(&self.store, window, cx);
        self.set_screen(next, cx);
    }

    fn show_locked(&mut self, session: Session, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.new(|cx| UnlockView::new(session, window, cx));
        let sub =
            cx.subscribe_in(
                &view,
                window,
                |this, _, event: &UnlockEvent, window, cx| match event {
                    UnlockEvent::Unlocked(unlocked) => this.show_vault_from(unlocked, window, cx),
                    UnlockEvent::LogOut => this.show_login(window, cx),
                    UnlockEvent::OpenSettings => this.open_settings(window, cx),
                },
            );
        self.set_screen((Screen::Locked(view), sub), cx);
    }

    fn show_vault_from(&mut self, handoff: &Handoff, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(unlocked) = handoff.take() {
            let next = Self::vault_screen(*unlocked, window, cx);
            self.set_screen(next, cx);
        }
    }

    fn login_screen(
        store: &SettingsStore,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Screen, Subscription) {
        let view = cx.new(|cx| LoginView::new(store.clone(), window, cx));
        #[cfg(debug_assertions)]
        if std::env::var_os("COFRE_DEV_AUTOLOGIN").is_some() {
            view.update(cx, |v, cx| v.dev_autologin(window, cx));
        }
        let sub =
            cx.subscribe_in(
                &view,
                window,
                |this, _, event: &LoginEvent, window, cx| match event {
                    LoginEvent::Unlocked(unlocked) => this.show_vault_from(unlocked, window, cx),
                    LoginEvent::OpenSettings => this.open_settings(window, cx),
                },
            );
        (Screen::Login(view), sub)
    }

    fn vault_screen(
        unlocked: Unlocked,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Screen, Subscription) {
        let view = cx.new(|cx| VaultView::new(unlocked, window, cx));
        let sub =
            cx.subscribe_in(
                &view,
                window,
                |this, _, event: &VaultEvent, window, cx| match event {
                    VaultEvent::Lock(session) => this.show_locked(session.clone(), window, cx),
                    VaultEvent::OpenSettings => this.open_settings(window, cx),
                },
            );
        (Screen::Vault(view), sub)
    }
}

impl Render for CofreApp {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content: AnyView = match (&self.settings, &self.screen) {
            (Some((v, _)), _) => v.clone().into(),
            (None, Screen::Login(v)) => v.clone().into(),
            (None, Screen::Locked(v)) => v.clone().into(),
            (None, Screen::Vault(v)) => v.clone().into(),
        };
        div()
            .size_full()
            // Qualquer tecla ou movimento do mouse adia o bloqueio automático.
            // Ctrl+, abre/fecha as configurações; Esc fecha.
            .capture_key_down(cx.listener(Self::on_key))
            .on_mouse_move(cx.listener(|this, _: &MouseMoveEvent, _, _| this.record_activity()))
            .child(content)
    }
}
