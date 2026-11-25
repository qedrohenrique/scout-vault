//! Window root: decides which screen shows up and locks on inactivity.

use std::time::{Duration, Instant};

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::appearance::{self, ThemeChoice};
use crate::autotype::{self, Request, Typer};
use crate::export_view::{ExportEvent, ExportView};
use crate::flow::{Handoff, Session, Unlocked};
use crate::generator_view::{GeneratorEvent, GeneratorView};
use crate::health_view::{HealthEvent, HealthView};
use crate::import_view::{ImportEvent, ImportView};
use crate::login::{LoginEvent, LoginView};
use crate::profile_view::{ProfileEvent, ProfileInfo, ProfileView};
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
    /// "Lock on minimize" (read from settings; updated when they close).
    lock_on_minimize: bool,
    _minimize_task: Task<()>,
    /// Settings open on top of the current screen (which stays alive underneath).
    settings: Option<(Entity<SettingsView>, Subscription)>,
    /// Password generator open on top of the current screen.
    generator: Option<(Entity<GeneratorView>, Subscription)>,
    /// "Password health" open (holds decrypted copies: goes away on lock).
    health: Option<(Entity<HealthView>, Subscription)>,
    /// "Import" open (holds a copy of the vault's items: goes away on lock).
    import: Option<(Entity<ImportView>, Subscription)>,
    /// "Export" open (also holds a copy of the vault's items).
    export: Option<(Entity<ExportView>, Subscription)>,
    _screen_subscription: Subscription,
    _appearance_subscription: Subscription,
    _idle_task: Task<()>,
    /// Root focus: makes sure shortcuts work even with no field focused.
    focus: FocusHandle,
    /// Short notices on top of everything.
    toasts: Entity<crate::toast::Toasts>,
    /// Request that arrived with the vault locked; handled on unlock.
    pending_autotype: Option<Request>,
    /// Quick search asked for while locked: opens after the HUD unlocks.
    pending_quick_search: Option<Request>,
    /// The login shown inside the HUD (signed out).
    _hud_login: Option<Subscription>,
    /// Redraws the title bar when the vault screen changes.
    _screen_observer: Option<Subscription>,
    /// "Profile" screen open, over the vault.
    profile: Option<(Entity<ProfileView>, Subscription)>,
    /// SSH signatures waiting for "Allow" (the first one is on screen).
    ssh_requests: std::collections::VecDeque<crate::ssh_agent::Approval>,
    /// "Remember for 10 minutes" in the SSH prompt.
    ssh_remember: bool,
    /// Windows Hello (or the master password) is being checked for the SSH request on screen.
    ssh_confirming: bool,
    /// Master password field, when the key on screen asks for it ("reprompt").
    ssh_password: Option<Entity<gpui_kit::component::input::InputState>>,
    ssh_password_error: Option<SharedString>,
    /// The window was in the tray when the SSH request came: goes back after.
    ssh_was_hidden: bool,
    _ssh_task: Task<()>,
    /// Looks for new versions now and then.
    _update_task: Task<()>,
    /// Typing waiting for "type anyway" (the app doesn't tell its fields).
    pending_type_anyway: Option<(Item, Request)>,
    /// Browser login waiting for unlock to be saved (url, username, password).
    pending_browser_save: Option<(String, String, zeroize::Zeroizing<String>)>,
    /// Rate limits on the browser pipe (see `browser::Limits`).
    browser_limits: crate::browser::Limits,
    /// Typing waiting for the master password of an item that asks for it (request number).
    reprompt_autotype: Option<(u64, Item, Request)>,
    reprompt_counter: u64,
    /// Registered global shortcut (dropping it turns it off).
    hotkey: Option<(autotype::Listener, crate::shortcut::Shortcut)>,
    /// Quick search shortcut (`Ctrl+Shift+Space`).
    quick_hotkey: Option<(autotype::Listener, crate::shortcut::Shortcut)>,
    /// Global shortcuts off while a new one is being recorded in settings
    /// (otherwise Windows would take the current combination).
    hotkeys_paused: bool,
    quick_tx: std::sync::mpsc::Sender<Request>,
    /// Open quick search window and the app to type into.
    quick_search: Option<(AnyWindowHandle, Entity<crate::quick_search::QuickSearch>)>,
    quick_target: Option<Request>,
    autotype_tx: std::sync::mpsc::Sender<Request>,
    /// Browser pipe server (dropping it turns it off).
    browser_server: Option<crate::io::Background>,
    browser_tx: std::sync::mpsc::Sender<crate::browser::Incoming>,
    /// Windows tray icon (dropping it removes it).
    tray: Option<crate::tray::Tray>,
    tray_tx: tokio::sync::mpsc::UnboundedSender<crate::tray::Command>,
    /// Tray right-click menu, if open.
    tray_menu: Option<(AnyWindowHandle, Entity<crate::tray_menu::TrayMenu>)>,
    typer: Typer,
    _autotype_task: Task<()>,
    /// Listener for Windows lock/sleep (real app only).
    _os_watcher: Option<crate::windows_events::Watcher>,
    _tray_task: Task<()>,
}

impl ScoutVaultApp {
    pub fn new(store: SettingsStore, window: &mut Window, cx: &mut Context<Self>) -> Self {
        appearance::apply(&store.load().appearance, Some(window), cx);
        // Remembered session: opens straight to unlock (master password only) — or
        // already unlocked, if the password was typed less than N days ago.
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
        this.refresh_hello_button(cx);
        this
    }

    /// Generator options and pinned items for a freshly opened vault screen.
    fn configure_vault(&self, view: &Entity<VaultView>, cx: &mut App) {
        let settings = self.store.load();
        view.update(cx, |v, _| {
            v.set_password_options(settings.generator);
            v.set_pinned(settings.pinned_items.clone());
            v.set_sidebar(
                settings.sidebar_collapsed.clone(),
                settings.sidebar_icons.clone(),
            );
            v.set_background_sync(settings.background_sync);
        });
        crate::site_icons::set_enabled(settings.site_icons, cx);
        Self::prefetch_site_icons(view, cx);
        let email = view.read(cx).session().email.clone();
        crate::profile_photo::load(&email, cx);
    }

    /// Downloads the website icons the open vault needs (only the missing ones).
    fn prefetch_site_icons(view: &Entity<VaultView>, cx: &mut App) {
        if !crate::site_icons::enabled(cx) {
            return;
        }
        let (urls, hosts) = {
            let v = view.read(cx);
            (
                v.session().client.urls().clone(),
                crate::site_icons::hosts_of(v.list().read(cx).delegate().vault()),
            )
        };
        crate::site_icons::prefetch(&urls, hosts, cx);
    }

    /// Idle lock timeout, from the settings (0 = never).
    fn auto_lock_from_settings(store: &SettingsStore) -> Duration {
        match store.load().auto_lock_minutes {
            0 => Duration::from_secs(u64::MAX / 4),
            m => Duration::from_secs(u64::from(m) * 60),
        }
    }

    /// Starts with the vault already open (used by the UI tests).
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
        crate::profile_photo::init(store.dir(), cx);
        if let Screen::Vault(view) = &screen {
            let email = view.read(cx).session().email.clone();
            crate::profile_photo::load(&email, cx);
        }
        let idle_task = cx.spawn_in(window, async move |this, cx| loop {
            cx.background_executor().timer(IDLE_CHECK_EVERY).await;
            let alive = this.update_in(cx, |this, window, cx| {
                this.lock_if_idle(Instant::now(), window, cx)
            });
            if alive.is_err() {
                break;
            }
        });
        // "Lock on minimize": GPUI doesn't report minimization, so check
        // twice a second (cheap: one call into Windows).
        let minimize_task = cx.spawn_in(window, async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(500))
                .await;
            let alive = this.update_in(cx, |this, window, cx| {
                if this.lock_on_minimize && crate::tray::is_minimized(window) {
                    this.on_minimized(window, cx);
                }
            });
            if alive.is_err() {
                break;
            }
        });
        crate::clipboard::set_clear_after_secs(store.load().clipboard_clear_secs, cx);
        crate::quick_search::set_opacity(store.load().hud_opacity, cx);
        crate::item_icons::set_all(store.load().item_icons, cx);
        // Global shortcut requests arrive through a channel (the shortcut thread isn't the UI thread).
        let (autotype_tx, autotype_rx) = std::sync::mpsc::channel::<Request>();
        let (quick_tx, quick_rx) = std::sync::mpsc::channel::<Request>();
        let (browser_tx, browser_rx) = std::sync::mpsc::channel::<crate::browser::Incoming>();
        // Windows lock/sleep (real app only, like the tray).
        let (os_tx, os_rx) = std::sync::mpsc::channel::<crate::windows_events::OsEvent>();
        let os_watcher = if crate::tray::system_enabled() {
            crate::windows_events::watch(os_tx).ok()
        } else {
            None
        };
        let (tray_tx, mut tray_rx) = tokio::sync::mpsc::unbounded_channel::<crate::tray::Command>();
        let autotype_task = cx.spawn_in(window, async move |this, cx| loop {
            cx.background_executor().timer(AUTOTYPE_POLL_EVERY).await;
            let alive = this.update_in(cx, |this, window, cx| {
                while let Ok(request) = autotype_rx.try_recv() {
                    this.handle_autotype(request, window, cx);
                }
                while let Ok(request) = quick_rx.try_recv() {
                    this.open_quick_search(request, window, cx);
                }
                while let Ok(incoming) = browser_rx.try_recv() {
                    this.handle_browser(incoming, window, cx);
                }
                if let Some(error) = crate::browser::take_server_error() {
                    crate::toast::error(error, cx);
                }
                while let Ok(event) = os_rx.try_recv() {
                    this.handle_os_event(event, window, cx);
                }
                this.drop_stale_ssh_requests(window, cx);
            });
            if alive.is_err() {
                break;
            }
        });
        // Bringing focus back to the window resumes the entry screens' animated art.
        cx.observe_window_activation(window, |_, _, cx| cx.notify())
            .detach();
        // "System" theme: follows Windows light/dark switching live.
        let appearance_sub = cx.observe_window_appearance(window, |this, window, cx| {
            let current = this.store.load().appearance;
            if current.theme == ThemeChoice::System {
                appearance::apply(&current, Some(window), cx);
            }
        });
        // The tray (and its menu) responds right away, without waiting for the next cycle.
        let tray_task = cx.spawn_in(window, async move |this, cx| {
            while let Some(command) = tray_rx.recv().await {
                if this
                    .update_in(cx, |this, window, cx| this.handle_tray(command, window, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        // SSH agent: signature requests come from the pipe threads.
        let (ssh_tx, mut ssh_rx) =
            tokio::sync::mpsc::unbounded_channel::<crate::ssh_agent::Approval>();
        crate::ssh_agent::install(crate::ssh_agent::PIPE, ssh_tx, cx);
        if store.load().ssh_agent && crate::tray::system_enabled() {
            crate::ssh_agent::set_enabled(true, cx);
        }
        let ssh_task = cx.spawn_in(window, async move |this, cx| {
            while let Some(request) = ssh_rx.recv().await {
                if this
                    .update_in(cx, |this, window, cx| {
                        this.handle_ssh_request(request, window, cx)
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        // Updates: on start (if the last check is old) and then twice a day.
        let update_store = store.clone();
        let update_task = cx.spawn(async move |_, cx| loop {
            let due = {
                let s = update_store.load();
                crate::updater::network_enabled()
                    && s.check_updates
                    && crate::flow::now_secs().saturating_sub(s.last_update_check)
                        >= crate::updater::CHECK_EVERY.as_secs()
            };
            if due {
                let mut s = update_store.load();
                s.last_update_check = crate::flow::now_secs();
                update_store.save(&s);
                cx.update(|cx| crate::updater::check(true, cx));
            }
            cx.background_executor()
                .timer(std::time::Duration::from_secs(30 * 60))
                .await;
        });
        let auto_lock_after = Self::auto_lock_from_settings(&store);
        let lock_on_minimize = store.load().lock_on_minimize;
        let mut this = Self {
            pending_autotype: None,
            pending_quick_search: None,
            _hud_login: None,
            _screen_observer: None,
            profile: None,
            pending_type_anyway: None,
            ssh_requests: Default::default(),
            ssh_remember: false,
            ssh_confirming: false,
            ssh_password: None,
            ssh_password_error: None,
            ssh_was_hidden: false,
            _ssh_task: ssh_task,
            _update_task: update_task,
            pending_browser_save: None,
            browser_limits: Default::default(),
            reprompt_autotype: None,
            reprompt_counter: 0,
            hotkey: None,
            autotype_tx,
            quick_hotkey: None,
            hotkeys_paused: false,
            quick_tx,
            quick_search: None,
            quick_target: None,
            browser_server: None,
            browser_tx,
            tray: None,
            tray_tx,
            tray_menu: None,
            _tray_task: tray_task,
            typer: autotype::system_typer(),
            _autotype_task: autotype_task,
            _os_watcher: os_watcher,
            store,
            screen,
            last_activity: Instant::now(),
            auto_lock_after,
            lock_on_minimize,
            _minimize_task: minimize_task,
            settings: None,
            generator: None,
            health: None,
            import: None,
            export: None,
            _screen_subscription: sub,
            _appearance_subscription: appearance_sub,
            _idle_task: idle_task,
            focus: cx.focus_handle(),
            toasts: crate::toast::init(cx),
        };
        this.refresh_ssh_keys(cx);
        // An update closes the app: not while that would lose an edit or a change in flight.
        let weak = cx.entity().downgrade();
        crate::updater::set_quit_check(
            move |cx| {
                weak.upgrade()
                    .and_then(|app| app.read(cx).unsafe_to_quit(cx))
            },
            cx,
        );
        this.refresh_hotkey(window, cx);
        this.refresh_browser();
        this.refresh_tray(window, cx);
        // Closing the window (X, Alt+F4) only hides it, if the tray is on.
        let app = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            app.update(cx, |this, cx| this.on_close_requested(window, cx))
                .unwrap_or(true)
        });
        this
    }

    /// Why closing the app now would lose something (`None` = safe to close).
    pub fn unsafe_to_quit(&self, cx: &App) -> Option<SharedString> {
        let Screen::Vault(view) = &self.screen else {
            return None;
        };
        let v = view.read(cx);
        if v.editor().is_some() {
            Some(t!("update.blocked_editing").into())
        } else if v.is_syncing() {
            Some(t!("update.blocked_syncing").into())
        } else {
            None
        }
    }

    /// Gives the SSH agent the open vault's keys (none when locked).
    fn refresh_ssh_keys(&self, cx: &mut App) {
        match &self.screen {
            Screen::Vault(view) => {
                let list = view.read(cx).list().clone();
                let vault = list.read(cx).delegate().vault();
                // `set_keys` needs `&mut App`: work on a copy of the SSH items only.
                let ssh = scout_vault_core::vault::Vault {
                    items: vault
                        .items
                        .iter()
                        .filter(|i| matches!(i.kind, scout_vault_core::vault::ItemKind::SshKey(_)))
                        .cloned()
                        .collect(),
                    ..Default::default()
                };
                crate::ssh_agent::set_keys(Some(&ssh), cx);
            }
            _ => crate::ssh_agent::set_keys(None, cx),
        }
    }

    /// A program wants to sign with a vault SSH key: ask (or allow, if remembered).
    pub fn handle_ssh_request(
        &mut self,
        request: crate::ssh_agent::Approval,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut request = request;
        let Screen::Vault(view) = &self.screen else {
            request.answer(false);
            return;
        };
        // A key whose item asks for the master password needs it for every
        // signature, and is never remembered.
        let reprompt = {
            let list = view.read(cx).list().clone();
            let vault = list.read(cx).delegate().vault();
            vault
                .items
                .iter()
                .find(|i| i.id == request.item_id)
                .map(|i| i.reprompt)
        };
        let Some(reprompt) = reprompt else {
            request.answer(false);
            return;
        };
        request.reprompt = reprompt;
        if !reprompt && crate::ssh_agent::is_remembered(&request, cx) {
            request.answer(true);
            return;
        }
        let first = self.ssh_requests.is_empty();
        if first {
            self.ssh_was_hidden = crate::tray::is_hidden(window);
            self.ssh_remember = false;
        }
        self.ssh_requests.push_back(request);
        if first {
            self.prepare_ssh_front(window, cx);
        }
        crate::tray::show_window(window);
        cx.notify();
    }

    /// Gets the prompt ready for the request now on screen (the password field, if needed).
    fn prepare_ssh_front(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use gpui_kit::component::input::{InputEvent, InputState};
        self.ssh_password_error = None;
        self.ssh_password = None;
        if !self.ssh_requests.front().is_some_and(|r| r.reprompt) {
            return;
        }
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder(t!("common.vault_password"))
        });
        cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.allow_ssh(window, cx);
            }
        })
        .detach();
        input.update(cx, |i, cx| i.focus(window, cx));
        self.ssh_password = Some(input);
    }

    /// The SSH request on screen asks for the master password: checks it, then signs.
    fn allow_ssh_with_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(input), Screen::Vault(view)) = (&self.ssh_password, &self.screen) else {
            return;
        };
        let password = zeroize::Zeroizing::new(input.read(cx).value().to_string());
        if password.is_empty() {
            self.ssh_password_error = Some(t!("vault.type_password").into());
            cx.notify();
            return;
        }
        let session = view.read(cx).session().clone();
        self.ssh_confirming = true;
        self.ssh_password_error = None;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let ok = crate::flow::check_master_password(&session, password).await;
            this.update_in(cx, |this, window, cx| {
                this.ssh_confirming = false;
                if ok {
                    this.finish_ssh(true, window, cx);
                } else {
                    this.ssh_password_error = Some(t!("vault.wrong_password").into());
                    if let Some(input) = &this.ssh_password {
                        input.update(cx, |i, cx| i.set_value("", window, cx));
                    }
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// The SSH request on screen, if any.
    pub fn ssh_request(&self) -> Option<&crate::ssh_agent::Approval> {
        self.ssh_requests.front()
    }

    pub fn set_ssh_remember(&mut self, on: bool, cx: &mut Context<Self>) {
        self.ssh_remember = on;
        cx.notify();
    }

    /// "Allow": with Windows Hello first, if the setting asks for it.
    pub fn allow_ssh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ssh_confirming || self.ssh_requests.is_empty() {
            return;
        }
        if self.ssh_requests.front().is_some_and(|r| r.reprompt) {
            self.allow_ssh_with_password(window, cx);
            return;
        }
        let hello = self.store.load().ssh_agent_hello && crate::hello::available(cx);
        let signer = hello.then(|| crate::hello::signer(cx)).flatten();
        let Some(signer) = signer else {
            self.finish_ssh(true, window, cx);
            return;
        };
        self.ssh_confirming = true;
        cx.notify();
        let challenge = crate::hello::new_challenge();
        let task = cx.background_spawn(async move { signer(&challenge) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                this.ssh_confirming = false;
                match result {
                    Ok(_) => this.finish_ssh(true, window, cx),
                    Err(e) => {
                        crate::toast::error(e.message(), cx);
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    pub fn deny_ssh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.ssh_confirming {
            self.finish_ssh(false, window, cx);
        }
    }

    fn finish_ssh(&mut self, allow: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(request) = self.ssh_requests.pop_front() else {
            return;
        };
        // Keys that ask for the master password are never remembered.
        let remember = allow && self.ssh_remember && !request.reprompt;
        if remember {
            crate::ssh_agent::remember(&request, cx);
        }
        request.answer(allow);
        // Others waiting for the same key and program get the same answer.
        if remember {
            let (same, rest): (Vec<_>, Vec<_>) = self
                .ssh_requests
                .drain(..)
                .partition(|r| !r.reprompt && crate::ssh_agent::is_remembered(r, cx));
            self.ssh_requests = rest.into();
            for r in same {
                r.answer(true);
            }
        }
        self.ssh_remember = false;
        self.prepare_ssh_front(window, cx);
        if self.ssh_requests.is_empty() && self.ssh_was_hidden {
            crate::tray::hide_window(window);
        }
        cx.notify();
    }

    /// Requests the program stopped waiting for leave the screen.
    fn drop_stale_ssh_requests(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ssh_confirming || !self.ssh_requests.iter().any(|r| r.is_stale()) {
            return;
        }
        let front_stale = self.ssh_requests.front().is_some_and(|r| r.is_stale());
        self.ssh_requests.retain(|r| !r.is_stale());
        if front_stale {
            self.prepare_ssh_front(window, cx);
        }
        if self.ssh_requests.is_empty() && self.ssh_was_hidden {
            crate::tray::hide_window(window);
        }
        cx.notify();
    }

    fn render_ssh_request(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        use gpui_kit::assets::IconName;
        use gpui_kit::component::button::*;
        use gpui_kit::component::checkbox::Checkbox;
        use gpui_kit::component::input::Input;
        use gpui_kit::component::*;
        let request = self.ssh_requests.front()?;
        let theme = cx.theme();
        let (bg, border, muted, radius) = (
            theme.background,
            theme.border,
            theme.muted_foreground,
            theme.radius_lg,
        );
        let accent = crate::appearance::accent(cx);
        let waiting = self.ssh_requests.len() - 1;
        let busy = self.ssh_confirming;
        let card = v_flex()
            .id("ssh-request")
            .w(px(420.))
            .p_6()
            .gap_4()
            .rounded(radius)
            .border_1()
            .border_color(border)
            .bg(bg)
            .shadow_lg()
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(
                        div()
                            .size(px(40.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .bg(accent.opacity(0.15))
                            .text_color(accent)
                            .child(Icon::new(IconName::KeyRound)),
                    )
                    .child(
                        div()
                            .text_lg()
                            .font_semibold()
                            .child(t!("ssh_agent.request_title")),
                    ),
            )
            .child(
                div()
                    .id("ssh-request-text")
                    .test_support()
                    .aria_label(SharedString::from(t!(
                        "ssh_agent.request_text",
                        program = request.client.describe(),
                        key = request.key_name.clone()
                    )))
                    .text_sm()
                    .child(t!(
                        "ssh_agent.request_text",
                        program = request.client.describe(),
                        key = request.key_name.clone()
                    )),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .font_family("monospace")
                    .child(request.fingerprint.clone()),
            )
            .child(
                div()
                    .id("ssh-request-path")
                    .test_support()
                    .aria_label(SharedString::from(request.client.path_line()))
                    .text_xs()
                    .text_color(muted)
                    .font_family("monospace")
                    .child(request.client.path_line()),
            )
            .when_some(self.ssh_password.clone(), |el, input| {
                el.child(
                    v_flex()
                        .gap_1()
                        .child(div().text_sm().child(t!("ssh_agent.reprompt_hint")))
                        .child(Input::new(&input).id("ssh-password").small())
                        .when_some(self.ssh_password_error.clone(), |el, error| {
                            el.child(
                                div()
                                    .id("ssh-password-error")
                                    .test_support()
                                    .aria_label(error.clone())
                                    .text_xs()
                                    .text_color(cx.theme().danger)
                                    .child(error),
                            )
                        }),
                )
            })
            .when(!request.reprompt, |el| {
                el.child(
                    Checkbox::new("ssh-remember")
                        .label(t!("ssh_agent.remember"))
                        .checked(self.ssh_remember)
                        .disabled(busy)
                        .on_change(
                            cx.listener(|this, v: &bool, _, cx| this.set_ssh_remember(*v, cx)),
                        ),
                )
            })
            .when(waiting > 0, |el| {
                el.child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(t!("ssh_agent.more_waiting", n = waiting)),
                )
            })
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("ssh-deny")
                            .outline()
                            .small()
                            .label(t!("ssh_agent.deny"))
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, window, cx| this.deny_ssh(window, cx))),
                    )
                    .child(
                        Button::new("ssh-allow")
                            .primary()
                            .small()
                            .icon(IconName::Check)
                            .label(t!("ssh_agent.allow"))
                            .loading(busy)
                            .disabled(busy)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.allow_ssh(window, cx)),
                            ),
                    ),
            );
        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui_kit::black().opacity(0.55))
                .occlude()
                .child(card)
                .into_any_element(),
        )
    }

    /// Windows locked the session or is about to sleep: lock the vault too
    /// (if the option is on).
    pub fn handle_os_event(
        &mut self,
        event: crate::windows_events::OsEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.store.load().lock_with_windows {
            return;
        }
        let _ = event;
        self.lock(window, cx);
    }

    /// Creates or removes the tray icon according to the setting.
    fn refresh_tray(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let wanted = self.store.load().close_to_tray && crate::tray::system_enabled();
        if !wanted {
            self.tray = None;
        } else if self.tray.is_none() {
            match crate::tray::start(self.tray_tx.clone()) {
                Ok(tray) => self.tray = Some(tray),
                Err(e) => cx.defer_in(window, move |_, _, cx| {
                    crate::toast::error(t!("app.tray_icon_failed", e = e), cx)
                }),
            }
        }
    }

    /// `true` lets the window close (and the app quit); `false` only hides it in the tray.
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
            tray.notify(t!("app.tray_hint_title"), t!("app.tray_hint_body"));
            settings.tray_hint_shown = true;
            self.store.save(&settings);
        }
        if settings.lock_on_minimize {
            self.on_minimized(window, cx);
        }
        false
    }

    /// The window was minimized (or went to the tray) with "Lock on
    /// minimize" on: locks if the vault is open.
    pub fn on_minimized(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.lock_on_minimize && matches!(self.screen, Screen::Vault(_)) {
            self.lock(window, cx);
        }
    }

    /// Commands from the tray icon.
    pub fn handle_tray(
        &mut self,
        command: crate::tray::Command,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::tray::Command;
        match command {
            Command::Show => crate::tray::show_window(window),
            Command::Lock => {
                self.lock(window, cx);
                crate::toast::info(t!("app.vault_locked"), cx);
            }
            Command::Quit => {
                // Quit for good: drop the icon before exiting.
                self.tray = None;
                cx.quit();
            }
            Command::GeneratePassword => {
                let options = self.store.load().generator;
                let password = scout_vault_core::generator::password(&options);
                crate::clipboard::copy_and_clear_later(&password, cx);
                crate::toast::success(
                    t!(
                        "app.password_generated",
                        note = crate::clipboard::clear_note(cx)
                    ),
                    cx,
                );
            }
            Command::Sync => match &self.screen {
                Screen::Vault(view) => view.update(cx, |v, cx| v.sync_now(cx)),
                _ => {
                    crate::tray::show_window(window);
                    crate::toast::info(t!("app.unlock_to_sync"), cx);
                }
            },
            Command::OpenMenu { x, y } => self.open_tray_menu((x, y), cx),
            Command::Unlock(password) => self.unlock_from_hud(password, window, cx),
            Command::TypeAnyway => {
                self.close_quick_search(cx);
                if let Some((item, request)) = self.pending_type_anyway.take() {
                    self.type_now(&item, request, autotype::FieldCheck::Skip, window, cx);
                }
            }
            Command::Fill { item_id } => {
                self.close_quick_search(cx);
                let Some(request) = self.quick_target.take() else {
                    return;
                };
                let item = match &self.screen {
                    Screen::Vault(view) => view
                        .read(cx)
                        .list()
                        .read(cx)
                        .delegate()
                        .vault()
                        .items
                        .iter()
                        .find(|i| i.id == item_id && !i.deleted)
                        .cloned(),
                    _ => None,
                };
                match item {
                    Some(item) => self.type_item(&item, request, window, cx),
                    None => {
                        crate::tray::show_window(window);
                        crate::toast::info(t!("app.unlock_to_fill"), cx);
                    }
                }
            }
            Command::OpenItem { item_id } => {
                self.close_quick_search(cx);
                crate::tray::show_window(window);
                if let Screen::Vault(view) = &self.screen {
                    view.update(cx, |v, cx| v.open_pinned(&item_id, window, cx));
                }
            }
            Command::Copy { item_id, what } => match &self.screen {
                Screen::Vault(view) => {
                    // Item that asks for the master password: show the window with the prompt.
                    let needs_window =
                        view.update(cx, |v, cx| v.copy_item_field(&item_id, what, window, cx));
                    if needs_window {
                        crate::tray::show_window(window);
                    }
                    self.record_activity();
                }
                _ => {
                    crate::tray::show_window(window);
                    crate::toast::info(t!("app.unlock_to_copy"), cx);
                }
            },
        }
    }

    /// What the tray menu should show now.
    pub fn tray_menu_state(&self, cx: &App) -> crate::tray::MenuState {
        let Screen::Vault(view) = &self.screen else {
            return crate::tray::MenuState::default();
        };
        let pinned = view
            .read(cx)
            .pinned_items(cx)
            .iter()
            .map(|item| {
                let (has_username, has_password, has_totp) = VaultView::copyable(item);
                crate::tray::PinnedEntry {
                    id: item.id.clone(),
                    name: item.name.clone(),
                    has_username,
                    has_password,
                    has_totp,
                    icon: Some(crate::item_icons::icon_for(item, cx)),
                }
            })
            .collect();
        crate::tray::MenuState {
            unlocked: true,
            pinned,
        }
    }

    /// Opens the tray menu at the cursor (closes the previous one, if any).
    pub fn open_tray_menu(&mut self, cursor: (i32, i32), cx: &mut Context<Self>) {
        self.close_tray_menu(cx);
        let state = self.tray_menu_state(cx);
        let size = crate::tray_menu::window_size(&state);
        let (work, scale) = crate::tray::work_area_and_scale();
        let physical = (
            (f32::from(size.width) * scale).round() as i32,
            (f32::from(size.height) * scale).round() as i32,
        );
        // The card has a transparent margin around it (shadow): the visible corner
        // sits right at the cursor.
        let margin = (crate::tray_menu::MARGIN * scale).round() as i32;
        let (x, y) = crate::tray::place_menu(
            (cursor.0 + margin, cursor.1 + margin),
            physical,
            (
                work.0 - margin,
                work.1 - margin,
                work.2 + margin,
                work.3 + margin,
            ),
        );
        let options = WindowOptions {
            titlebar: None,
            kind: WindowKind::PopUp,
            focus: true,
            show: true,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            // Windows blurs what is behind (acrylic); the card's opacity is a setting.
            window_background: WindowBackgroundAppearance::Blurred,
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                point(px(x as f32 / scale), px(y as f32 / scale)),
                size,
            ))),
            ..Default::default()
        };
        let tx = self.tray_tx.clone();
        let opened = gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| crate::tray_menu::TrayMenu::new(state, tx, window, cx))
        });
        match opened {
            Ok((handle, view)) => {
                handle
                    .update(cx, |_, window, cx| {
                        crate::tray::frameless(window);
                        transparent_root(window, cx);
                        window.activate_window()
                    })
                    .ok();
                self.tray_menu = Some((handle, view));
            }
            Err(e) => crate::toast::error(t!("app.menu_failed", e = e), cx),
        }
    }

    fn close_tray_menu(&mut self, cx: &mut App) {
        if let Some((handle, _)) = self.tray_menu.take() {
            handle
                .update(cx, |_, window, _| window.remove_window())
                .ok();
        }
    }

    /// The tray menu open right now (for tests).
    pub fn tray_menu(&self) -> Option<(AnyWindowHandle, Entity<crate::tray_menu::TrayMenu>)> {
        self.tray_menu.clone()
    }

    /// Turns the browser pipe on or off according to the setting.
    fn refresh_browser(&mut self) {
        let wanted = self.store.load().browser_fill && crate::browser::system_enabled();
        if !wanted {
            self.browser_server = None;
        } else if self.browser_server.is_none() {
            self.browser_server = crate::browser::start_server(self.browser_tx.clone());
        }
    }

    /// Answers a request from the browser extension.
    pub fn handle_browser(
        &mut self,
        incoming: crate::browser::Incoming,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.store.load().browser_fill {
            let _ = incoming
                .reply
                .send(serde_json::json!({ "error": t!("app.browser_disabled") }));
            return;
        }
        use crate::browser::Request as BrowserRequest;
        // Anything running as this Windows user can talk to the pipe: too many requests
        // for secrets in a minute are refused, and the person is told.
        if let crate::browser::Verdict::Refuse { warn } = self
            .browser_limits
            .check(&incoming.request, std::time::Instant::now())
        {
            if warn {
                crate::toast::error(t!("app.browser_too_many"), cx);
            }
            let _ = incoming
                .reply
                .send(serde_json::json!({ "error": t!("app.browser_too_many") }));
            return;
        }
        // Saving a login: the app decides and saves (or keeps it for after unlock).
        if let BrowserRequest::Save {
            url,
            username,
            password,
        } = &incoming.request
        {
            let response = self.save_from_browser(
                url.clone(),
                username.clone(),
                zeroize::Zeroizing::new(password.clone()),
                window,
                cx,
            );
            let _ = incoming.reply.send(response);
            return;
        }
        let response = match &self.screen {
            Screen::Vault(view) => {
                // "Is it new?" only looks at what can be changed.
                if matches!(incoming.request, BrowserRequest::Check { .. }) {
                    let items = view.read(cx).editable_items(cx);
                    crate::browser::handle(&incoming.request, Some(&items))
                } else {
                    let items = &view.read(cx).list().read(cx).delegate().vault().items;
                    crate::browser::handle(&incoming.request, Some(items))
                }
            }
            _ => {
                // Status and "is it new?" don't draw attention (they happen on every site login).
                if !matches!(
                    incoming.request,
                    BrowserRequest::Status | BrowserRequest::Check { .. }
                ) {
                    crate::tray::show_window(window);
                    crate::toast::info(t!("app.unlock_to_fill_browser"), cx);
                }
                crate::browser::handle(&incoming.request, None)
            }
        };
        // A fill doesn't count as activity: a program asking through the pipe must not
        // keep the vault from locking when the person walks away.
        let _ = incoming.reply.send(response);
    }

    /// Saves (or updates) a login submitted on a site, after "Save" on the page.
    /// With the vault locked, shows the app and saves right after unlock.
    fn save_from_browser(
        &mut self,
        url: String,
        username: String,
        password: zeroize::Zeroizing<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> serde_json::Value {
        use crate::browser::{decide_save, new_login, with_password, SaveDecision};
        let Screen::Vault(view) = &self.screen else {
            let host = crate::browser::host_of(&url).unwrap_or_default();
            self.pending_browser_save = Some((url, username, password));
            crate::tray::show_window(window);
            crate::toast::info(t!("app.unlock_to_save_login", host = host), cx);
            return serde_json::json!({ "queued": true });
        };
        let view = view.clone();
        let items = view.read(cx).editable_items(cx);
        let (item, message) = match decide_save(&items, &url, &username, &password) {
            SaveDecision::Nothing => return serde_json::json!({ "ok": true, "unchanged": true }),
            SaveDecision::New => {
                let item = new_login(&url, &username, &password);
                let message = t!("app.login_saved_from_browser", name = item.name);
                (item, message)
            }
            SaveDecision::Update { id, name } => {
                let Some(base) = items.iter().find(|i| i.id == id) else {
                    return serde_json::json!({ "error": t!("app.login_not_found") });
                };
                (
                    with_password(base, &password),
                    t!("app.password_updated", name = name),
                )
            }
        };
        // Not activity either: it comes through the pipe (see `handle_browser`).
        match view.update(cx, |v, cx| v.save_external(item, message, cx)) {
            Ok(()) => serde_json::json!({ "ok": true }),
            Err(e) => {
                crate::toast::error(e.clone(), cx);
                serde_json::json!({ "error": e })
            }
        }
    }

    /// After unlock: saves the login the browser sent while the vault was locked.
    fn finish_pending_browser_save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((url, username, password)) = self.pending_browser_save.take() {
            self.save_from_browser(url, username, password, window, cx);
        }
    }

    /// Registers, re-registers (another combination) or drops the global
    /// shortcuts according to the settings.
    fn refresh_hotkey(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let settings = self.store.load();
        crate::shortcut::set_current(settings.hotkeys.clone(), cx);
        let system = autotype::system_hotkey_enabled() && !self.hotkeys_paused;

        let wanted = (settings.autotype.enabled && system).then(|| settings.hotkeys.autotype());
        if self.hotkey.as_ref().map(|(_, s)| s) != wanted.as_ref() {
            // Drop the old one first: Windows refuses the same combination twice.
            self.hotkey = None;
            if let Some(shortcut) = wanted {
                match autotype::listen(self.autotype_tx.clone(), &shortcut) {
                    Ok(listener) => self.hotkey = Some((listener, shortcut)),
                    // Report it later: while the window is being created the
                    // notification layer doesn't exist yet.
                    Err(e) => cx.defer_in(window, move |_, _, cx| crate::toast::error(e, cx)),
                }
            }
        }

        let wanted = (settings.quick_search && system).then(|| settings.hotkeys.quick_search());
        if self.quick_hotkey.as_ref().map(|(_, s)| s) != wanted.as_ref() {
            self.quick_hotkey = None;
            if let Some(shortcut) = wanted {
                match autotype::listen_quick_search(self.quick_tx.clone(), &shortcut) {
                    Ok(listener) => self.quick_hotkey = Some((listener, shortcut)),
                    Err(e) => cx.defer_in(window, move |_, _, cx| crate::toast::error(e, cx)),
                }
            }
        }
    }

    /// `Ctrl+Shift+Space`: opens quick search on top of everything (or, with the
    /// vault locked, the app asking for the password).
    pub fn open_quick_search(
        &mut self,
        request: Request,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.store.load().quick_search {
            return;
        }
        self.close_quick_search(cx);
        if matches!(self.screen, Screen::Locked(_)) {
            if self.unlock_silently(window, cx) {
                return self.open_quick_search(request, window, cx);
            }
            let target = (!request.own_window).then(|| request.title.clone());
            self.pending_quick_search = Some(request);
            self.open_locked_hud(target, false, cx);
            return;
        }
        if matches!(self.screen, Screen::Login(_)) {
            let target = (!request.own_window).then(|| request.title.clone());
            self.pending_quick_search = Some(request);
            self.open_login_hud(target, false, window, cx);
            return;
        }
        let Some(items) = self.quick_items(cx) else {
            return;
        };
        // Used in Scout Vault itself: there is no other app to type into.
        let target = (!request.own_window).then(|| request.title.clone());
        self.quick_target = (!request.own_window).then_some(request);
        self.open_quick_window(crate::quick_search::HEIGHT, cx, move |tx, window, cx| {
            crate::quick_search::QuickSearch::new(items, target, tx, window, cx)
        });
    }

    /// Ctrl+Alt+A when the window title doesn't pick one login: a HUD on top of
    /// everything (apart from the main window) to search and choose.
    fn open_autotype_hud(
        &mut self,
        request: Request,
        suggested: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        self.close_quick_search(cx);
        let Some(items) = self.quick_items(cx) else {
            return;
        };
        let target = request.title.clone();
        let program = request.program.clone();
        self.quick_target = Some(request);
        self.open_quick_window(crate::quick_search::HEIGHT, cx, move |tx, window, cx| {
            crate::quick_search::QuickSearch::autotype(
                items, target, program, suggested, tx, window, cx,
            )
        });
    }

    /// "Ask for the vault password on open: every N days" still within the period:
    /// a shortcut opens the locked vault without the password (as opening the app
    /// would). Doesn't extend the period.
    fn unlock_silently(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Screen::Locked(view) = &self.screen else {
            return false;
        };
        let session = view.read(cx).session().clone();
        let Some(keys) = crate::quick_unlock::open(&self.store, &session) else {
            return false;
        };
        let vault = scout_vault_core::vault::Vault::decrypt(&session.sync, &keys);
        // Without the pending requests: whoever called handles its own.
        let (autotype, quick) = (
            self.pending_autotype.take(),
            self.pending_quick_search.take(),
        );
        self.open_unlocked(
            Unlocked {
                session,
                keys,
                vault,
            },
            window,
            cx,
        );
        self.pending_autotype = autotype;
        self.pending_quick_search = quick;
        true
    }

    /// The HUD in its password step (vault locked).
    fn open_locked_hud(&mut self, target: Option<String>, autotype: bool, cx: &mut Context<Self>) {
        self.close_quick_search(cx);
        self.open_quick_window(
            crate::quick_search::LOCKED_HEIGHT,
            cx,
            move |tx, window, cx| {
                crate::quick_search::QuickSearch::locked(target, autotype, tx, window, cx)
            },
        );
    }

    /// Signed out: the HUD shows the login; signing in carries on with what was asked.
    fn open_login_hud(
        &mut self,
        target: Option<String>,
        autotype: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_quick_search(cx);
        let store = self.store.clone();
        self.open_quick_window(
            crate::quick_search::LOGIN_HEIGHT,
            cx,
            move |tx, window, cx| {
                crate::quick_search::QuickSearch::signed_out(
                    target, autotype, store, tx, window, cx,
                )
            },
        );
        let login = self
            .quick_search
            .as_ref()
            .and_then(|(_, q)| q.read(cx).login_view().cloned());
        self._hud_login = login.map(|login| {
            cx.subscribe_in(&login, window, |this, _, event: &LoginEvent, window, cx| {
                if let LoginEvent::Unlocked(unlocked) = event {
                    // Close first: focus goes back to the app that will be typed into.
                    this.close_quick_search(cx);
                    this._hud_login = None;
                    let quick = this.pending_quick_search.take();
                    this.show_vault_from(unlocked, window, cx);
                    if let Some(request) = quick {
                        this.open_quick_search(request, window, cx);
                    }
                }
            })
        });
    }

    /// Password typed in the HUD: opens the vault and carries on with what was asked.
    fn unlock_from_hud(
        &mut self,
        password: crate::tray::HudPassword,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Screen::Locked(view) = &self.screen else {
            self.close_quick_search(cx);
            return;
        };
        let session = view.read(cx).session().clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = crate::io::blocking(move || {
                crate::flow::unlock(&session, &password.0).map(|(keys, vault)| Unlocked {
                    session,
                    keys,
                    vault,
                })
            })
            .await;
            this.update_in(cx, |this, window, cx| match result {
                Ok(unlocked) => {
                    // Close first: focus goes back to the app that will be typed into.
                    this.close_quick_search(cx);
                    let quick = this.pending_quick_search.take();
                    this.show_vault_from(&Handoff::new(Box::new(unlocked)), window, cx);
                    if let Some(request) = quick {
                        this.open_quick_search(request, window, cx);
                    }
                }
                Err(e) => {
                    if let Some((handle, quick)) = this.quick_search.clone() {
                        let message: SharedString = e.into();
                        handle
                            .update(cx, |_, window, cx| {
                                quick.update(cx, |q, cx| q.unlock_failed(message, window, cx))
                            })
                            .ok();
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    /// The vault's items as quick search shows them (`None` with the vault locked).
    fn quick_items(&self, cx: &App) -> Option<Vec<crate::quick_search::QuickItem>> {
        let Screen::Vault(view) = &self.screen else {
            return None;
        };
        let items: Vec<crate::quick_search::QuickItem> = view
            .read(cx)
            .list()
            .read(cx)
            .delegate()
            .vault()
            .items
            .iter()
            .filter(|i| {
                !i.deleted && !matches!(i.kind, scout_vault_core::vault::ItemKind::Unknown(_))
            })
            .map(|i| {
                let mut q = crate::quick_search::QuickItem::from_item(i);
                q.icon = crate::item_icons::icon_for(i, cx);
                q.color = crate::item_icons::choice(&i.id, cx).and_then(|c| c.color);
                q.site = crate::item_icons::site_host(i, cx);
                q
            })
            .collect();
        Some(items)
    }

    /// Opens the quick search window (centered, near the top, on top of everything).
    fn open_quick_window(
        &mut self,
        height: f32,
        cx: &mut Context<Self>,
        build: impl FnOnce(
                tokio::sync::mpsc::UnboundedSender<crate::tray::Command>,
                &mut Window,
                &mut Context<crate::quick_search::QuickSearch>,
            ) -> crate::quick_search::QuickSearch
            + 'static,
    ) {
        let size = size(px(crate::quick_search::WIDTH), px(height));
        // Centered, in the top third of the work area (like launchers).
        let (work, scale) = crate::tray::work_area_and_scale();
        let (wx, wy, ww, wh) = (
            work.0 as f32 / scale,
            work.1 as f32 / scale,
            (work.2 - work.0) as f32 / scale,
            (work.3 - work.1) as f32 / scale,
        );
        let origin = point(
            px(wx + (ww - f32::from(size.width)) / 2.),
            px(wy + (wh * 0.22).max(0.)),
        );
        let options = WindowOptions {
            titlebar: None,
            kind: WindowKind::PopUp,
            focus: true,
            show: true,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            // Windows blurs what is behind (acrylic); the card's opacity is a setting.
            window_background: WindowBackgroundAppearance::Blurred,
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(origin, size))),
            ..Default::default()
        };
        let tx = self.tray_tx.clone();
        let opened =
            gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| build(tx, window, cx)));
        match opened {
            Ok((handle, view)) => {
                handle
                    .update(cx, |_, window, cx| {
                        crate::tray::frameless(window);
                        transparent_root(window, cx);
                        window.activate_window()
                    })
                    .ok();
                self.quick_search = Some((handle, view));
            }
            Err(e) => crate::toast::error(t!("app.quick_search_failed", e = e), cx),
        }
    }

    fn close_quick_search(&mut self, cx: &mut App) {
        if let Some((handle, _)) = self.quick_search.take() {
            handle
                .update(cx, |_, window, _| window.remove_window())
                .ok();
        }
    }

    /// The quick search open right now (tests).
    pub fn quick_search(
        &self,
    ) -> Option<(AnyWindowHandle, Entity<crate::quick_search::QuickSearch>)> {
        self.quick_search.clone()
    }

    /// Swaps the typist (tests use a fake one that only records what it would type).
    pub fn set_typer(&mut self, typer: Typer) {
        self.typer = typer;
    }

    /// Handles a `Ctrl+Alt+A`: types directly if only one login matches the
    /// window title; otherwise opens the auto-type HUD. With the vault closed,
    /// keeps the request for after unlock.
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
                t!(
                    "app.autotype_own_window",
                    hotkey = crate::shortcut::autotype_label(cx)
                ),
                cx,
            );
            return;
        }
        if matches!(self.screen, Screen::Locked(_)) {
            if self.unlock_silently(window, cx) {
                return self.handle_autotype(request, window, cx);
            }
            let target = request.title.clone();
            self.pending_autotype = Some(request);
            self.open_locked_hud(Some(target), true, cx);
            return;
        }
        let Screen::Vault(view) = &self.screen else {
            // Signed out: the login happens in the HUD too.
            let target = request.title.clone();
            self.pending_autotype = Some(request);
            self.open_login_hud(Some(target), true, window, cx);
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
        // Always confirm in the HUD, even with a single candidate: the window title is
        // chosen by whoever made the window (a site's `<title>` in a browser), so it
        // only suggests — the person checks the target program and presses Enter.
        let found = autotype::candidates(&items, &request.title);
        let suggested = found.iter().map(|i| i.id.clone()).collect();
        self.open_autotype_hud(request, suggested, cx);
    }

    fn type_item(
        &mut self,
        item: &Item,
        request: Request,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Item that asks for the master password: confirm in the app before typing.
        if item.reprompt {
            if let Screen::Vault(view) = &self.screen {
                self.reprompt_counter += 1;
                let token = self.reprompt_counter;
                let id = item.id.clone();
                let free = view.update(cx, |v, cx| v.request_reprompt(&id, token, window, cx));
                if !free {
                    self.reprompt_autotype = Some((token, item.clone(), request));
                    crate::tray::show_window(window);
                    crate::toast::info(t!("app.autotype_reprompt"), cx);
                    return;
                }
            }
        }
        self.type_now(item, request, autotype::FieldCheck::Verify, window, cx);
    }

    fn type_now(
        &mut self,
        item: &Item,
        request: Request,
        check: autotype::FieldCheck,
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
        let item = item.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = crate::io::blocking(move || typer(target, keys, check)).await;
            this.update(cx, |this, cx| match result {
                Ok(()) => {}
                // The HUD says it (the main window may be hidden behind the app).
                Err(autotype::TypeError::CantVerify) => {
                    this.pending_type_anyway = Some((item, request));
                    this.open_notice_hud(t!("autotype.cant_verify").into(), true, cx);
                }
                Err(autotype::TypeError::Failed(message)) => {
                    this.open_notice_hud(message.into(), false, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// A message from auto-type in the HUD (with "type anyway", if `confirm`).
    fn open_notice_hud(&mut self, message: SharedString, confirm: bool, cx: &mut Context<Self>) {
        self.close_quick_search(cx);
        self.open_quick_window(
            crate::quick_search::NOTICE_HEIGHT,
            cx,
            move |tx, window, cx| {
                crate::quick_search::QuickSearch::notice(message, confirm, tx, window, cx)
            },
        );
    }

    /// Toast stack (for tests).
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
        self.health = None;
        let view = cx.new(|cx| SettingsView::new(self.store.clone(), window, cx));
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
                    this.lock_on_minimize = this.store.load().lock_on_minimize;
                    this.refresh_hello_button(cx);
                    if let Screen::Vault(view) = &this.screen {
                        let on = this.store.load().background_sync;
                        view.update(cx, |v, _| v.set_background_sync(on));
                        Self::prefetch_site_icons(view, cx);
                    }
                }
                SettingsEvent::WindowsHello(on) => this.windows_hello_changed(*on, window, cx),
                SettingsEvent::Hotkeys => this.refresh_hotkey(window, cx),
                SettingsEvent::RecordingShortcut(on) => {
                    this.hotkeys_paused = *on;
                    this.refresh_hotkey(window, cx);
                }
                SettingsEvent::Language => this.language_changed(window, cx),
                SettingsEvent::OpenImport => this.open_import(window, cx),
                SettingsEvent::OpenExport => this.open_export(window, cx),
            },
        );
        self.settings = Some((view, sub));
        cx.notify();
    }

    /// The language changed: rebuilds the texts the screens keep (field placeholders,
    /// for example). The rest comes with the next draw.
    fn language_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match &self.screen {
            Screen::Vault(view) => view.update(cx, |v, cx| v.relabel(window, cx)),
            Screen::Login(view) => view.update(cx, |v, cx| v.relabel(window, cx)),
            Screen::Locked(view) => view.update(cx, |v, cx| v.relabel(window, cx)),
        }
        cx.notify();
    }

    pub fn import_view(&self) -> Option<&Entity<ImportView>> {
        self.import.as_ref().map(|(v, _)| v)
    }

    /// Opens "Import" (needs the vault open: the items go into it).
    pub fn open_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Screen::Vault(vault) = &self.screen else {
            crate::toast::error(t!("import.needs_vault"), cx);
            return;
        };
        if self.import.is_some() {
            return;
        }
        self.settings = None;
        self.generator = None;
        self.health = None;
        self.profile = None;
        let copy = {
            let v = vault.read(cx);
            let open = v.list().read(cx).delegate().vault();
            scout_vault_core::vault::Vault {
                folders: open.folders.clone(),
                items: open.items.clone(),
                ..Default::default()
            }
        };
        let view = cx.new(|cx| ImportView::new(copy, window, cx));
        let sub = cx.subscribe_in(
            &view,
            window,
            |this, _, event: &ImportEvent, _, cx| match event {
                ImportEvent::Close => this.close_import(cx),
                ImportEvent::Import { items, existing } => {
                    let (items, existing) = (items.clone(), existing.clone());
                    this.close_import(cx);
                    if let Screen::Vault(v) = &this.screen {
                        v.update(cx, |v, cx| v.import_items(items, existing, cx));
                    }
                }
            },
        );
        self.import = Some((view, sub));
        cx.notify();
    }

    pub fn export_view(&self) -> Option<&Entity<ExportView>> {
        self.export.as_ref().map(|(v, _)| v)
    }

    /// Opens "Export" (needs the vault open; the master password is asked there).
    pub fn open_export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Screen::Vault(vault) = &self.screen else {
            crate::toast::error(t!("export.needs_vault"), cx);
            return;
        };
        if self.export.is_some() {
            return;
        }
        self.settings = None;
        self.generator = None;
        self.health = None;
        self.profile = None;
        self.import = None;
        let (copy, session) = {
            let v = vault.read(cx);
            let open = v.list().read(cx).delegate().vault();
            (
                scout_vault_core::vault::Vault {
                    folders: open.folders.clone(),
                    items: open.items.clone(),
                    ..Default::default()
                },
                v.session().clone(),
            )
        };
        let view = cx.new(|cx| ExportView::new(copy, session, window, cx));
        let sub = cx.subscribe_in(
            &view,
            window,
            |this, _, event: &ExportEvent, _, cx| match event {
                ExportEvent::Close => this.close_export(cx),
            },
        );
        self.export = Some((view, sub));
        cx.notify();
    }

    pub fn close_export(&mut self, cx: &mut Context<Self>) {
        if self.export.take().is_some() {
            cx.notify();
        }
    }

    pub fn close_import(&mut self, cx: &mut Context<Self>) {
        if self.import.take().is_some() {
            cx.notify();
        }
    }

    pub fn health_view(&self) -> Option<&Entity<HealthView>> {
        self.health.as_ref().map(|(v, _)| v)
    }

    /// Opens "Password health" with the open vault's items.
    /// The profile screen (who is signed in, the account, this computer).
    pub fn open_profile(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Screen::Vault(vault) = &self.screen else {
            return;
        };
        if self.profile.is_some() {
            return;
        }
        self.settings = None;
        self.generator = None;
        self.health = None;
        let info = {
            let v = vault.read(cx);
            ProfileInfo::new(
                v.session(),
                v.list().read(cx).delegate().vault(),
                &self.store.load(),
            )
        };
        let view = cx.new(|_| ProfileView::new(info));
        let sub = cx.subscribe_in(
            &view,
            window,
            |this, _, event: &ProfileEvent, window, cx| match event {
                ProfileEvent::Close => this.close_profile(cx),
                ProfileEvent::OpenSettings => {
                    this.close_profile(cx);
                    this.open_settings(window, cx);
                }
                ProfileEvent::SignOut => {
                    this.close_profile(cx);
                    if let Some(session) = this.current_session(cx) {
                        crate::hello::forget(&this.store, &session);
                    }
                    crate::saved_session::clear(&this.store);
                    this.show_login(window, cx);
                }
            },
        );
        self.profile = Some((view, sub));
        cx.notify();
    }

    pub fn close_profile(&mut self, cx: &mut Context<Self>) {
        if self.profile.take().is_some() {
            cx.notify();
        }
    }

    pub fn profile_view(&self) -> Option<&Entity<ProfileView>> {
        self.profile.as_ref().map(|(v, _)| v)
    }

    pub fn open_health(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Screen::Vault(vault) = &self.screen else {
            return;
        };
        if self.health.is_some() {
            return;
        }
        self.settings = None;
        self.generator = None;
        let (items, dates) = {
            let v = vault.read(cx);
            (
                v.list().read(cx).delegate().vault().items.clone(),
                scout_vault_core::health::password_dates(&v.session().sync),
            )
        };
        let view = cx.new(|_| HealthView::new(items, &dates));
        let sub =
            cx.subscribe_in(
                &view,
                window,
                |this, _, event: &HealthEvent, window, cx| match event {
                    HealthEvent::Close => this.close_health(cx),
                    HealthEvent::OpenItem(id) => {
                        let id = id.clone();
                        this.close_health(cx);
                        if let Screen::Vault(v) = &this.screen {
                            v.update(cx, |v, cx| v.open_pinned(&id, window, cx));
                        }
                    }
                },
            );
        self.health = Some((view, sub));
        cx.notify();
    }

    pub fn close_health(&mut self, cx: &mut Context<Self>) {
        if self.health.take().is_some() {
            cx.notify();
        }
    }

    pub fn generator_view(&self) -> Option<&Entity<GeneratorView>> {
        self.generator.as_ref().map(|(v, _)| v)
    }

    pub fn open_generator(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.generator.is_some() {
            return;
        }
        self.settings = None;
        let email = self
            .current_session(cx)
            .map(|s| s.email)
            .unwrap_or_default();
        let view = cx.new(|cx| GeneratorView::new(self.store.clone(), email, cx));
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
        // Recording a shortcut in settings: every key goes there (even Esc).
        if self
            .settings_view()
            .is_some_and(|v| v.read(cx).is_recording())
        {
            return;
        }
        let k = &event.keystroke;
        // The SSH prompt answers Esc (deny) and Enter (allow) before anything else.
        if !self.ssh_requests.is_empty() {
            match k.key.as_str() {
                "escape" => self.deny_ssh(window, cx),
                "enter" => self.allow_ssh(window, cx),
                _ => {}
            }
            return;
        }
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
        } else if k.key == "h" && k.modifiers.secondary(cx) && k.modifiers.shift {
            if self.health.is_some() {
                self.close_health(cx);
            } else {
                self.open_health(window, cx);
            }
        } else if k.key == "escape" && self.generator.is_some() {
            self.close_generator(cx);
        } else if k.key == "escape" && self.health.is_some() {
            self.close_health(cx);
        } else if k.key == "escape" && self.import.is_some() {
            self.close_import(cx);
        } else if k.key == "escape" && self.export.is_some() {
            self.close_export(cx);
        } else if k.key == "escape" && self.profile.is_some() {
            let profile = self.profile_view().unwrap().clone();
            if profile.read(cx).cropping() {
                profile.update(cx, |p, cx| p.cancel_crop(cx));
            } else {
                self.close_profile(cx);
            }
        } else if k.key == "enter" && self.profile_view().is_some_and(|p| p.read(cx).cropping()) {
            let profile = self.profile_view().unwrap().clone();
            profile.update(cx, |p, cx| p.save_crop(cx));
        } else if self.settings.is_none()
            && self.generator.is_none()
            && self.health.is_none()
            && self.profile.is_none()
            && k.modifiers.secondary(cx)
            && !k.modifiers.shift
        {
            self.vault_shortcut(&k.key, window, cx);
        }
    }

    /// Shortcuts of the open vault: `Ctrl+L` locks, `Ctrl+F` searches,
    /// `Ctrl+U`/`Ctrl+P`/`Ctrl+T` copy the open item's username, password and code.
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

    pub fn unlock_view(&self) -> Option<&Entity<UnlockView>> {
        match &self.screen {
            Screen::Locked(v) => Some(v),
            _ => None,
        }
    }

    pub fn set_auto_lock_after(&mut self, after: Duration) {
        self.auto_lock_after = after;
    }

    pub fn record_activity(&mut self) {
        self.last_activity = Instant::now();
    }

    /// Locks if the vault is open and nobody touched anything since `auto_lock_after`.
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
        // The title bar shows the vault's actions (the sync spinner…): redraw with it.
        self._screen_observer = match &screen {
            Screen::Vault(view) => Some(cx.observe(view, |this, _, cx| {
                this.refresh_ssh_keys(cx);
                cx.notify()
            })),
            _ => None,
        };
        // Switching the screen drops the previous one: the decrypted vault leaves memory with it
        // (including the copy the fill list had and the health screen's).
        self.health = None;
        self.import = None;
        self.export = None;
        self.profile = None;
        self.screen = screen;
        self._screen_subscription = sub;
        // The SSH agent only has keys while the vault is open.
        self.refresh_ssh_keys(cx);
        if !matches!(self.screen, Screen::Vault(_)) {
            for request in self.ssh_requests.drain(..) {
                request.answer(false);
            }
            self.ssh_confirming = false;
        }
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
        self.refresh_hello_button(cx);
    }

    /// Shows "Use Windows Hello" on unlock when it is ready.
    fn refresh_hello_button(&self, cx: &mut App) {
        if let Screen::Locked(view) = &self.screen {
            let session = view.read(cx).session().clone();
            let ready = self.store.load().windows_hello
                && crate::hello::available(cx)
                && crate::hello::is_enrolled(&self.store, &session);
            view.update(cx, |v, cx| v.set_hello(ready, cx));
        }
    }

    /// Session of the open or locked vault.
    fn current_session(&self, cx: &App) -> Option<Session> {
        match &self.screen {
            Screen::Vault(view) => Some(view.read(cx).session().clone()),
            Screen::Locked(view) => Some(view.read(cx).session().clone()),
            Screen::Login(_) => None,
        }
    }

    /// Turned the option on/off in settings.
    fn windows_hello_changed(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !on {
            if let Some(session) = self.current_session(cx) {
                crate::hello::forget(&self.store, &session);
            }
            self.refresh_hello_button(cx);
            return;
        }
        match &self.screen {
            Screen::Vault(view) => {
                let (session, keys) = {
                    let v = view.read(cx);
                    (v.session().clone(), v.keys())
                };
                self.enroll_hello(session, keys, window, cx);
            }
            _ => crate::toast::info(t!("app.hello_next_unlock"), cx),
        }
    }

    /// Asks for Windows Hello once and stores the user key encrypted by it.
    fn enroll_hello(
        &mut self,
        session: Session,
        keys: std::sync::Arc<scout_vault_core::vault::Keys>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(signer) = crate::hello::signer(cx) else {
            self.hello_failed(crate::hello::HelloError::Unavailable, cx);
            return;
        };
        let challenge = crate::hello::new_challenge();
        let to_sign = challenge.clone();
        cx.spawn_in(window, async move |this, cx| {
            let signature = crate::io::blocking(move || signer(&to_sign)).await;
            this.update(cx, |this, cx| {
                let result = signature.and_then(|sig| {
                    crate::hello::seal(&this.store, &session, &keys, &challenge, &sig)
                });
                match result {
                    Ok(()) => crate::toast::success(t!("app.hello_enabled"), cx),
                    Err(e) => this.hello_failed(e, cx),
                }
            })
            .ok();
        })
        .detach();
    }

    /// Couldn't turn it on: undoes the option and explains.
    fn hello_failed(&mut self, error: crate::hello::HelloError, cx: &mut Context<Self>) {
        let mut settings = self.store.load();
        settings.windows_hello = false;
        self.store.save(&settings);
        if let Some(view) = self.settings_view().cloned() {
            view.update(cx, |v, cx| v.show_windows_hello(false, cx));
        }
        crate::toast::error(error.message(), cx);
    }

    /// "Use Windows Hello" on the unlock screen.
    pub fn unlock_with_hello(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Screen::Locked(view) = &self.screen else {
            return;
        };
        let view = view.clone();
        let session = view.read(cx).session().clone();
        let (Some(signer), Some(sealed)) = (
            crate::hello::signer(cx),
            crate::hello::sealed(&self.store, &session),
        ) else {
            view.update(cx, |v, cx| {
                v.set_hello(false, cx);
                v.set_hello_progress(false, Some(t!("app.hello_not_ready").into()), cx)
            });
            return;
        };
        view.update(cx, |v, cx| v.set_hello_progress(true, None, cx));
        cx.spawn_in(window, async move |this, cx| {
            let result = crate::io::blocking(move || {
                let signature = signer(&sealed.challenge)?;
                let keys = crate::hello::unseal(&sealed, &signature, &session)?;
                let vault = scout_vault_core::vault::Vault::decrypt(&session.sync, &keys);
                Ok::<_, crate::hello::HelloError>(Unlocked {
                    session,
                    keys,
                    vault,
                })
            })
            .await;
            this.update_in(cx, |this, window, cx| match result {
                Ok(unlocked) => {
                    view.update(cx, |v, cx| v.set_hello_progress(false, None, cx));
                    this.open_unlocked(unlocked, window, cx);
                }
                Err(e) => view.update(cx, |v, cx| {
                    v.set_hello_progress(false, Some(e.message()), cx)
                }),
            })
            .ok();
        })
        .detach();
    }

    /// Opens the already decrypted vault (Windows Hello): without touching "open without password".
    fn open_unlocked(&mut self, unlocked: Unlocked, window: &mut Window, cx: &mut Context<Self>) {
        crate::saved_session::save(&self.store, &unlocked.session);
        let next = Self::vault_screen(unlocked, window, cx);
        if let Screen::Vault(view) = &next.0 {
            self.configure_vault(view, cx);
        }
        self.set_screen(next, cx);
        if let Some(request) = self.pending_autotype.take() {
            self.handle_autotype(request, window, cx);
        }
        self.finish_pending_browser_save(window, cx);
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
                        if let Some(session) = this.current_session(cx) {
                            crate::hello::forget(&this.store, &session);
                        }
                        crate::saved_session::clear(&this.store);
                        this.show_login(window, cx)
                    }
                    UnlockEvent::OpenSettings => this.open_settings(window, cx),
                    UnlockEvent::UseHello => this.unlock_with_hello(window, cx),
                },
            );
        (Screen::Locked(view), sub)
    }

    /// Current app state is "remembered vault" (used by tests).
    pub fn store(&self) -> &SettingsStore {
        &self.store
    }

    fn show_vault_from(&mut self, handoff: &Handoff, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(unlocked) = handoff.take() {
            crate::saved_session::save(&self.store, &unlocked.session);
            // Password typed now: valid for opening without a password for the next N days.
            if self.store.load().remember_session {
                crate::quick_unlock::remember(&self.store, &unlocked.session, &unlocked.keys);
            }
            let hello_pending = self.store.load().windows_hello
                && crate::hello::available(cx)
                && !crate::hello::is_enrolled(&self.store, &unlocked.session);
            let next = Self::vault_screen(*unlocked, window, cx);
            if let Screen::Vault(view) = &next.0 {
                self.configure_vault(view, cx);
                if hello_pending {
                    let (session, keys) = {
                        let v = view.read(cx);
                        (v.session().clone(), v.keys())
                    };
                    self.enroll_hello(session, keys, window, cx);
                }
            }
            self.set_screen(next, cx);
            if let Some(request) = self.pending_autotype.take() {
                self.handle_autotype(request, window, cx);
            }
            self.finish_pending_browser_save(window, cx);
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
                    VaultEvent::OpenHealth => this.open_health(window, cx),
                    VaultEvent::OpenProfile => this.open_profile(window, cx),
                    VaultEvent::IconsChanged(icons) => {
                        let mut settings = this.store.load();
                        settings.item_icons = icons.clone();
                        this.store.save(&settings);
                    }
                    VaultEvent::SidebarChanged { collapsed, icons } => {
                        let mut settings = this.store.load();
                        settings.sidebar_collapsed = collapsed.clone();
                        settings.sidebar_icons = icons.clone();
                        this.store.save(&settings);
                    }
                    VaultEvent::RepromptPassed(token) => {
                        if let Some((t, item, request)) = this.reprompt_autotype.take() {
                            if t == *token {
                                this.type_now(
                                    &item,
                                    request,
                                    autotype::FieldCheck::Verify,
                                    window,
                                    cx,
                                );
                            } else {
                                this.reprompt_autotype = Some((t, item, request));
                            }
                        }
                    }
                    VaultEvent::Synced(session) => {
                        crate::saved_session::save(&this.store, session);
                        if let Screen::Vault(view) = &this.screen {
                            Self::prefetch_site_icons(&view.clone(), cx);
                        }
                        if crate::hello::password_changed(&this.store, session) {
                            let mut settings = this.store.load();
                            settings.windows_hello = false;
                            this.store.save(&settings);
                            crate::toast::info(t!("app.hello_password_changed"), cx);
                        }
                        if crate::quick_unlock::password_changed(&this.store, session) {
                            crate::toast::info(t!("app.quick_unlock_password_changed"), cx);
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
                            login.update(cx, |l, cx| l.set_notice(t!("app.session_expired"), cx));
                        }
                    }
                },
            );
        (Screen::Vault(view), sub)
    }
}

impl Render for ScoutVaultApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Switching screens drops the field that had focus; with no focus at all, the
        // keys wouldn't reach the shortcuts.
        if window.focused(cx).is_none() {
            self.focus.focus(window, cx);
        }
        let content: AnyView = match (&self.settings, &self.screen) {
            _ if self.generator.is_some() => self.generator_view().unwrap().clone().into(),
            _ if self.health.is_some() => self.health_view().unwrap().clone().into(),
            _ if self.import.is_some() => self.import_view().unwrap().clone().into(),
            _ if self.export.is_some() => self.export_view().unwrap().clone().into(),
            _ if self.profile.is_some() => self.profile_view().unwrap().clone().into(),
            (Some((v, _)), _) => v.clone().into(),
            (None, Screen::Login(v)) => v.clone().into(),
            (None, Screen::Locked(v)) => v.clone().into(),
            (None, Screen::Vault(v)) => v.clone().into(),
        };
        // Title bar drawn by the app (minimize, maximize, close): the same colors as
        // the screen below, so window and app read as one piece.
        let theme = cx.theme();
        let with_sidebar = matches!(
            (
                &self.settings,
                &self.screen,
                self.generator.is_some()
                    || self.health.is_some()
                    || self.profile.is_some()
                    || self.import.is_some()
                    || self.export.is_some()
            ),
            (None, Screen::Vault(_), false)
        );
        let (background, sidebar, border) = (theme.background, theme.sidebar, theme.border);
        // On the vault: the brand over the sidebar's column, the actions on the right.
        let actions = match &self.screen {
            Screen::Vault(view) if with_sidebar => {
                Some(view.update(cx, |v, cx| v.render_title_actions(cx)))
            }
            _ => None,
        };
        let title_bar = gpui_kit::component::TitleBar::new()
            // A bit taller than the default (34 px): room around the logo.
            .h(px(44.))
            .pl_0()
            .border_0()
            .bg(background)
            .when(with_sidebar, |bar| {
                bar.child(
                    gpui_kit::component::h_flex()
                        .h_full()
                        .w(px(crate::vault_view::SIDEBAR_WIDTH))
                        .flex_shrink_0()
                        .px_4()
                        .items_center()
                        .bg(sidebar)
                        .border_r_1()
                        .border_color(border)
                        .child(crate::ui::wordmark(cx)),
                )
            })
            .children(actions);
        div()
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .child(title_bar)
            // On the vault: a line under the title bar, from the sidebar's edge to the
            // window's (the sidebar's column stays one piece).
            .when(with_sidebar, |el| {
                el.child(
                    gpui_kit::component::h_flex()
                        .id("title-divider")
                        .test_support()
                        .h(px(1.))
                        .w_full()
                        .flex_shrink_0()
                        .child(
                            div()
                                .h_full()
                                .w(px(crate::vault_view::SIDEBAR_WIDTH))
                                .flex_shrink_0()
                                .bg(sidebar)
                                .border_r_1()
                                .border_color(border),
                        )
                        .child(div().flex_1().h_full().bg(border)),
                )
            })
            // Any key or mouse movement postpones the automatic lock.
            // Ctrl+, opens/closes settings; Esc closes.
            .capture_key_down(cx.listener(Self::on_key))
            .on_mouse_move(cx.listener(|this, _: &MouseMoveEvent, _, _| this.record_activity()))
            // Each new screen (login → vault, opening settings…) eases in.
            .child(crate::motion::rise(
                div().flex_1().min_h_0().w_full().child(content.clone()),
                crate::motion::key("tela", content.entity_id()),
                10.,
            ))
            .children(self.render_ssh_request(cx))
            .child(self.toasts.clone())
            // Theme transition on top of everything.
            .children(crate::theme_fx::overlay(cx))
    }
}

/// gpui-kit's `Root` paints the theme background over the whole window. In the
/// floating windows (tray menu, quick search) only the card should show: the rest
/// stays transparent, without a darker "frame" around it.
fn transparent_root(window: &mut Window, cx: &mut App) {
    if let Some(Some(root)) = window.root::<gpui_kit::base::Root>() {
        root.update(cx, |root, cx| {
            root.style().background = Some(gpui_kit::transparent_black().into());
            cx.notify();
        });
    }
}
