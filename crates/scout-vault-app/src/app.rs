//! Raiz da janela: decide qual tela aparece e bloqueia por inatividade.

use std::time::{Duration, Instant};

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::appearance::{self, ThemeChoice};
use crate::autotype::{self, Request, Typer};
use crate::autotype_view::{AutoTypeEvent, AutoTypeView};
use crate::flow::{Handoff, Session, Unlocked};
use crate::generator_view::{GeneratorEvent, GeneratorView};
use crate::login::{LoginEvent, LoginView};
use crate::settings::SettingsStore;
use crate::settings_view::{SettingsEvent, SettingsView};
use crate::unlock::{UnlockEvent, UnlockView};
use crate::vault_view::{CopyShortcut, VaultEvent, VaultView};
use scout_vault_core::vault::Item;

pub const DEFAULT_AUTO_LOCK: Duration = Duration::from_secs(5 * 60);
const IDLE_CHECK_EVERY: Duration = Duration::from_secs(10);
const AUTOTYPE_POLL_EVERY: Duration = Duration::from_millis(120);

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

pub struct ScoutVaultApp {
    store: SettingsStore,
    screen: Screen,
    last_activity: Instant,
    auto_lock_after: Duration,
    /// Configurações abertas por cima da tela atual (que segue viva por baixo).
    settings: Option<(Entity<SettingsView>, Subscription)>,
    /// Gerador de senhas aberto por cima da tela atual.
    generator: Option<(Entity<GeneratorView>, Subscription)>,
    _screen_subscription: Subscription,
    _appearance_subscription: Subscription,
    _idle_task: Task<()>,
    /// Foco da raiz: garante que os atalhos funcionem mesmo sem campo focado.
    focus: FocusHandle,
    /// Avisos curtos por cima de tudo.
    toasts: Entity<crate::toast::Toasts>,
    /// Escolha do login para preencher em outro app.
    autotype_picker: Option<(Entity<AutoTypeView>, Subscription)>,
    /// Pedido que chegou com o cofre bloqueado; atendido ao desbloquear.
    pending_autotype: Option<Request>,
    /// Atalho global registrado (soltar desliga).
    hotkey: Option<autotype::Listener>,
    autotype_tx: std::sync::mpsc::Sender<Request>,
    /// Servidor do pipe do navegador (soltar desliga).
    browser_server: Option<crate::io::Background>,
    browser_tx: std::sync::mpsc::Sender<crate::browser::Incoming>,
    /// Ícone na bandeja do Windows (soltar remove).
    tray: Option<crate::tray::Tray>,
    tray_tx: std::sync::mpsc::Sender<crate::tray::Command>,
    typer: Typer,
    _autotype_task: Task<()>,
}

impl ScoutVaultApp {
    pub fn new(store: SettingsStore, window: &mut Window, cx: &mut Context<Self>) -> Self {
        appearance::apply(&store.load().appearance, Some(window), cx);
        // Sessão lembrada: abre direto no desbloqueio (só a senha mestra) — ou
        // já desbloqueado, se a senha foi digitada há menos de N dias.
        let (screen, sub) = match crate::saved_session::load(&store) {
            Some(session) => match crate::quick_unlock::open(&store, &session) {
                Some(keys) => {
                    let vault = scout_vault_core::vault::Vault::decrypt(&session.sync, &keys);
                    Self::vault_screen(
                        Unlocked {
                            session,
                            keys,
                            vault,
                        },
                        window,
                        cx,
                    )
                }
                None => Self::locked_screen(session, window, cx),
            },
            None => Self::login_screen(&store, window, cx),
        };
        let this = Self::build(store, screen, sub, window, cx);
        if let Screen::Vault(view) = &this.screen {
            this.configure_vault(view, cx);
        }
        this
    }

    /// Opções do gerador e itens fixados para uma tela do cofre recém-aberta.
    fn configure_vault(&self, view: &Entity<VaultView>, cx: &mut App) {
        let settings = self.store.load();
        view.update(cx, |v, _| {
            v.set_password_options(settings.generator);
            v.set_pinned(settings.pinned_items.clone());
        });
    }

    /// Prazo do bloqueio por inatividade, das configurações (0 = nunca).
    fn auto_lock_from_settings(store: &SettingsStore) -> Duration {
        match store.load().auto_lock_minutes {
            0 => Duration::from_secs(u64::MAX / 4),
            m => Duration::from_secs(u64::from(m) * 60),
        }
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
        // Pedidos do atalho global chegam por um canal (a thread do atalho não é a da UI).
        let (autotype_tx, autotype_rx) = std::sync::mpsc::channel::<Request>();
        let (browser_tx, browser_rx) = std::sync::mpsc::channel::<crate::browser::Incoming>();
        let (tray_tx, tray_rx) = std::sync::mpsc::channel::<crate::tray::Command>();
        let autotype_task = cx.spawn_in(window, async move |this, cx| loop {
            cx.background_executor().timer(AUTOTYPE_POLL_EVERY).await;
            let alive = this.update_in(cx, |this, window, cx| {
                while let Ok(request) = autotype_rx.try_recv() {
                    this.handle_autotype(request, window, cx);
                }
                while let Ok(incoming) = browser_rx.try_recv() {
                    this.handle_browser(incoming, window, cx);
                }
                while let Ok(command) = tray_rx.try_recv() {
                    this.handle_tray(command, window, cx);
                }
            });
            if alive.is_err() {
                break;
            }
        });
        // Voltar o foco para a janela retoma a arte animada das telas de entrada.
        cx.observe_window_activation(window, |_, _, cx| cx.notify())
            .detach();
        // Tema "Sistema": acompanha a troca de claro/escuro do Windows ao vivo.
        let appearance_sub = cx.observe_window_appearance(window, |this, window, cx| {
            let current = this.store.load().appearance;
            if current.theme == ThemeChoice::System {
                appearance::apply(&current, Some(window), cx);
            }
        });
        let auto_lock_after = Self::auto_lock_from_settings(&store);
        let mut this = Self {
            autotype_picker: None,
            pending_autotype: None,
            hotkey: None,
            autotype_tx,
            browser_server: None,
            browser_tx,
            tray: None,
            tray_tx,
            typer: autotype::system_typer(),
            _autotype_task: autotype_task,
            store,
            screen,
            last_activity: Instant::now(),
            auto_lock_after,
            settings: None,
            generator: None,
            _screen_subscription: sub,
            _appearance_subscription: appearance_sub,
            _idle_task: idle_task,
            focus: cx.focus_handle(),
            toasts: crate::toast::init(cx),
        };
        this.refresh_hotkey(window, cx);
        this.refresh_browser();
        this.refresh_tray(window, cx);
        // Fechar a janela (X, Alt+F4) só esconde, se a bandeja estiver ligada.
        let app = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            app.update(cx, |this, cx| this.on_close_requested(window, cx))
                .unwrap_or(true)
        });
        this
    }

    /// Cria ou remove o ícone da bandeja conforme a configuração.
    fn refresh_tray(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let wanted = self.store.load().close_to_tray && crate::tray::system_enabled();
        if !wanted {
            self.tray = None;
        } else if self.tray.is_none() {
            match crate::tray::start(self.tray_tx.clone()) {
                Ok(tray) => self.tray = Some(tray),
                Err(e) => cx.defer_in(window, move |_, _, cx| {
                    crate::toast::error(
                        format!("Não foi possível criar o ícone na bandeja: {e}"),
                        cx,
                    )
                }),
            }
        }
    }

    /// `true` deixa a janela fechar (e o app sair); `false` só a esconde na bandeja.
    pub fn on_close_requested(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(tray) = &self.tray else {
            return true;
        };
        let mut settings = self.store.load();
        if !settings.close_to_tray {
            return true;
        }
        crate::tray::hide_window(window);
        if !settings.tray_hint_shown {
            tray.notify(
                "O Scout Vault continua aberto",
                "Ele fica aqui na bandeja. Clique para abrir; botão direito para sair.",
            );
            settings.tray_hint_shown = true;
            self.store.save(&settings);
        }
        let _ = cx;
        false
    }

    /// Comandos do ícone da bandeja.
    pub fn handle_tray(
        &mut self,
        command: crate::tray::Command,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match command {
            crate::tray::Command::Show => crate::tray::show_window(window),
            crate::tray::Command::Lock => {
                self.lock(window, cx);
                crate::toast::info("Cofre bloqueado", cx);
            }
            crate::tray::Command::Quit => {
                // Sai de vez: solta o ícone antes de encerrar.
                self.tray = None;
                cx.quit();
            }
        }
    }

    /// Liga ou desliga o pipe do navegador conforme a configuração.
    fn refresh_browser(&mut self) {
        let wanted = self.store.load().browser_fill && crate::browser::system_enabled();
        if !wanted {
            self.browser_server = None;
        } else if self.browser_server.is_none() {
            self.browser_server = crate::browser::start_server(self.browser_tx.clone());
        }
    }

    /// Responde um pedido da extensão do navegador.
    pub fn handle_browser(
        &mut self,
        incoming: crate::browser::Incoming,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.store.load().browser_fill {
            let _ = incoming
                .reply
                .send(serde_json::json!({ "error": "A integração com o navegador está desligada no Scout Vault." }));
            return;
        }
        let response = match &self.screen {
            Screen::Vault(view) => {
                let items = &view.read(cx).list().read(cx).delegate().vault().items;
                crate::browser::handle(&incoming.request, Some(items))
            }
            _ => {
                if !matches!(incoming.request, crate::browser::Request::Status) {
                    crate::tray::show_window(window);
                    crate::toast::info("Desbloqueie o cofre para preencher no navegador.", cx);
                }
                crate::browser::handle(&incoming.request, None)
            }
        };
        if matches!(incoming.request, crate::browser::Request::Fill { .. }) {
            self.record_activity();
        }
        let _ = incoming.reply.send(response);
    }

    /// Registra ou solta o atalho global conforme a configuração.
    fn refresh_hotkey(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let wanted = self.store.load().autotype.enabled && autotype::system_hotkey_enabled();
        if !wanted {
            self.hotkey = None;
        } else if self.hotkey.is_none() {
            match autotype::listen(self.autotype_tx.clone()) {
                Ok(listener) => self.hotkey = Some(listener),
                // Avisa depois: durante a criação da janela a camada de
                // notificações ainda não existe.
                Err(e) => cx.defer_in(window, move |_, _, cx| crate::toast::error(e, cx)),
            }
        }
    }

    /// Troca o digitador (os testes usam um falso que só registra o que digitaria).
    pub fn set_typer(&mut self, typer: Typer) {
        self.typer = typer;
    }

    pub fn autotype_view(&self) -> Option<&Entity<AutoTypeView>> {
        self.autotype_picker.as_ref().map(|(v, _)| v)
    }

    /// Atende um `Ctrl+Alt+A`: digita direto se só um login combina com o
    /// título da janela; senão mostra a lista. Com o cofre fechado, guarda o
    /// pedido para depois do desbloqueio.
    pub fn handle_autotype(
        &mut self,
        request: Request,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.store.load().autotype.enabled {
            return;
        }
        if request.own_window {
            crate::toast::info(
                format!(
                    "Abra o app onde quer entrar e use {} lá.",
                    autotype::HOTKEY_LABEL
                ),
                cx,
            );
            return;
        }
        let Screen::Vault(view) = &self.screen else {
            let message = format!(
                "Desbloqueie o cofre para preencher em “{}”.",
                request.title.trim()
            );
            self.pending_autotype = Some(request);
            crate::tray::show_window(window);
            crate::toast::info(message, cx);
            return;
        };
        let items = view
            .read(cx)
            .list()
            .read(cx)
            .delegate()
            .vault()
            .items
            .clone();
        let found = autotype::candidates(&items, &request.title);
        if let [only] = found.as_slice() {
            let item = (*only).clone();
            self.type_item(&item, request, window, cx);
            return;
        }
        self.open_autotype_picker(request, &items, window, cx);
        crate::tray::show_window(window);
    }

    fn open_autotype_picker(
        &mut self,
        request: Request,
        items: &[Item],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings = None;
        self.generator = None;
        let view = cx.new(|cx| AutoTypeView::new(request, items, window, cx));
        let sub = cx.subscribe_in(
            &view,
            window,
            |this, view, event: &AutoTypeEvent, window, cx| match event {
                AutoTypeEvent::Close => this.close_autotype(cx),
                AutoTypeEvent::Chosen(id) => {
                    let picked = {
                        let v = view.read(cx);
                        v.shown(cx)
                            .into_iter()
                            .find(|i| &i.id == id)
                            .cloned()
                            .map(|item| (item, v.request().clone()))
                    };
                    this.close_autotype(cx);
                    if let Some((item, request)) = picked {
                        this.type_item(&item, request, window, cx);
                    }
                }
            },
        );
        self.autotype_picker = Some((view, sub));
        cx.notify();
    }

    pub fn close_autotype(&mut self, cx: &mut Context<Self>) {
        if self.autotype_picker.take().is_some() {
            cx.notify();
        }
    }

    fn type_item(
        &mut self,
        item: &Item,
        request: Request,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let keys = autotype::sequence(item, self.store.load().autotype.press_enter);
        if keys.is_empty() {
            return;
        }
        self.record_activity();
        let typer = self.typer.clone();
        let target = request.window;
        cx.spawn_in(window, async move |this, cx| {
            let result = crate::io::blocking(move || typer(target, keys)).await;
            if let Err(message) = result {
                this.update(cx, |_, cx| crate::toast::error(message, cx))
                    .ok();
            }
        })
        .detach();
    }

    /// Pilha de toasts (para testes).
    pub fn toasts(&self) -> &Entity<crate::toast::Toasts> {
        &self.toasts
    }

    pub fn settings_view(&self) -> Option<&Entity<SettingsView>> {
        self.settings.as_ref().map(|(v, _)| v)
    }

    pub fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings.is_some() {
            return;
        }
        self.generator = None;
        let view = cx.new(|_| SettingsView::new(self.store.clone()));
        let sub = cx.subscribe_in(
            &view,
            window,
            |this, _, event: &SettingsEvent, window, cx| match event {
                SettingsEvent::Close => {
                    this.close_settings(cx);
                    this.refresh_hotkey(window, cx);
                    this.refresh_browser();
                    this.refresh_tray(window, cx);
                    this.auto_lock_after = Self::auto_lock_from_settings(&this.store);
                }
            },
        );
        self.settings = Some((view, sub));
        cx.notify();
    }

    pub fn generator_view(&self) -> Option<&Entity<GeneratorView>> {
        self.generator.as_ref().map(|(v, _)| v)
    }

    pub fn open_generator(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.generator.is_some() {
            return;
        }
        self.settings = None;
        let view = cx.new(|_| GeneratorView::new(self.store.clone()));
        let sub =
            cx.subscribe_in(
                &view,
                window,
                |this, _, event: &GeneratorEvent, _, cx| match event {
                    GeneratorEvent::Close => this.close_generator(cx),
                },
            );
        self.generator = Some((view, sub));
        cx.notify();
    }

    pub fn close_generator(&mut self, cx: &mut Context<Self>) {
        if self.generator.take().is_some() {
            cx.notify();
        }
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
        } else if k.key == "g" && k.modifiers.secondary(cx) && !k.modifiers.shift {
            if self.generator.is_some() {
                self.close_generator(cx);
            } else {
                self.open_generator(window, cx);
            }
        } else if k.key == "escape" && self.settings.is_some() {
            self.close_settings(cx);
        } else if k.key == "escape" && self.generator.is_some() {
            self.close_generator(cx);
        } else if k.key == "escape" && self.autotype_picker.is_some() {
            self.close_autotype(cx);
        } else if self.settings.is_none()
            && self.generator.is_none()
            && self.autotype_picker.is_none()
            && k.modifiers.secondary(cx)
            && !k.modifiers.shift
        {
            self.vault_shortcut(&k.key, window, cx);
        }
    }

    /// Atalhos do cofre aberto: `Ctrl+L` bloqueia, `Ctrl+F` busca,
    /// `Ctrl+U`/`Ctrl+P`/`Ctrl+T` copiam usuário, senha e código do item aberto.
    fn vault_shortcut(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Screen::Vault(view) = &self.screen else {
            return;
        };
        let view = view.clone();
        let copy = |what| {
            move |v: &mut VaultView, window: &mut Window, cx: &mut Context<VaultView>| {
                v.copy_shortcut(what, window, cx)
            }
        };
        match key {
            "l" => self.lock(window, cx),
            "f" => view.update(cx, |v, cx| v.focus_search(window, cx)),
            "u" => view.update(cx, |v, cx| copy(CopyShortcut::Username)(v, window, cx)),
            "p" => view.update(cx, |v, cx| copy(CopyShortcut::Password)(v, window, cx)),
            "t" => view.update(cx, |v, cx| copy(CopyShortcut::TotpCode)(v, window, cx)),
            "n" => view.update(cx, |v, cx| v.new_item(window, cx)),
            "e" => view.update(cx, |v, cx| v.edit_selected(window, cx)),
            _ => return,
        }
        cx.stop_propagation();
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
        // Trocar a tela solta a anterior: o cofre decifrado sai da memória junto
        // (inclusive a cópia que a lista de preenchimento tinha).
        self.autotype_picker = None;
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
        crate::saved_session::save(&self.store, &session);
        let next = Self::locked_screen(session, window, cx);
        self.set_screen(next, cx);
    }

    fn locked_screen(
        session: Session,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Screen, Subscription) {
        let view = cx.new(|cx| UnlockView::new(session, window, cx));
        let sub =
            cx.subscribe_in(
                &view,
                window,
                |this, _, event: &UnlockEvent, window, cx| match event {
                    UnlockEvent::Unlocked(unlocked) => this.show_vault_from(unlocked, window, cx),
                    UnlockEvent::LogOut => {
                        crate::saved_session::clear(&this.store);
                        this.show_login(window, cx)
                    }
                    UnlockEvent::OpenSettings => this.open_settings(window, cx),
                },
            );
        (Screen::Locked(view), sub)
    }

    /// Estado atual do app é "cofre lembrado" (usado pelos testes).
    pub fn store(&self) -> &SettingsStore {
        &self.store
    }

    fn show_vault_from(&mut self, handoff: &Handoff, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(unlocked) = handoff.take() {
            crate::saved_session::save(&self.store, &unlocked.session);
            // Senha digitada agora: vale para abrir sem senha pelos próximos N dias.
            if self.store.load().remember_session {
                crate::quick_unlock::remember(&self.store, &unlocked.session, &unlocked.keys);
            }
            let next = Self::vault_screen(*unlocked, window, cx);
            if let Screen::Vault(view) = &next.0 {
                self.configure_vault(view, cx);
            }
            self.set_screen(next, cx);
            if let Some(request) = self.pending_autotype.take() {
                self.handle_autotype(request, window, cx);
            }
        }
    }

    fn login_screen(
        store: &SettingsStore,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Screen, Subscription) {
        let view = cx.new(|cx| LoginView::new(store.clone(), window, cx));
        #[cfg(debug_assertions)]
        if std::env::var_os("SCOUT_VAULT_DEV_AUTOLOGIN").is_some() {
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
                    VaultEvent::OpenGenerator => this.open_generator(window, cx),
                    VaultEvent::Synced(session) => {
                        crate::saved_session::save(&this.store, session);
                        if crate::quick_unlock::password_changed(&this.store, session) {
                            crate::toast::info(
                                "A senha mestra mudou. Na próxima vez, o app vai pedir a nova.",
                                cx,
                            );
                        }
                    }
                    VaultEvent::PinsChanged(ids) => {
                        let mut settings = this.store.load();
                        settings.pinned_items = ids.clone();
                        this.store.save(&settings);
                    }
                    VaultEvent::SessionExpired => {
                        crate::saved_session::clear(&this.store);
                        this.show_login(window, cx);
                        if let Screen::Login(login) = &this.screen {
                            login.update(cx, |l, cx| {
                                l.set_notice("A sessão expirou no servidor. Entre novamente.", cx)
                            });
                        }
                    }
                },
            );
        (Screen::Vault(view), sub)
    }
}

impl Render for ScoutVaultApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Trocar de tela solta o campo que tinha foco; sem foco nenhum, as
        // teclas não chegariam aos atalhos.
        if window.focused(cx).is_none() {
            self.focus.focus(window, cx);
        }
        let content: AnyView = match (&self.settings, &self.screen) {
            _ if self.autotype_picker.is_some() => self.autotype_view().unwrap().clone().into(),
            _ if self.generator.is_some() => self.generator_view().unwrap().clone().into(),
            (Some((v, _)), _) => v.clone().into(),
            (None, Screen::Login(v)) => v.clone().into(),
            (None, Screen::Locked(v)) => v.clone().into(),
            (None, Screen::Vault(v)) => v.clone().into(),
        };
        div()
            .track_focus(&self.focus)
            .relative()
            .size_full()
            // Qualquer tecla ou movimento do mouse adia o bloqueio automático.
            // Ctrl+, abre/fecha as configurações; Esc fecha.
            .capture_key_down(cx.listener(Self::on_key))
            .on_mouse_move(cx.listener(|this, _: &MouseMoveEvent, _, _| this.record_activity()))
            // Cada tela nova (login → cofre, abrir configurações…) entra suave.
            .child(crate::motion::rise(
                div().size_full().child(content.clone()),
                crate::motion::key("tela", content.entity_id()),
                10.,
            ))
            .child(self.toasts.clone())
    }
}
