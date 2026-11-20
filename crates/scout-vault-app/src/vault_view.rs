//! Open vault: filters on the left, list in the middle, item detail on the right.

use std::borrow::Cow;
use std::collections::HashSet;
use std::sync::Arc;

use gpui_kit::component::list::{List, ListEvent, ListState};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{button::*, *};
use gpui_kit::prelude::*;
use gpui_kit::*;
use scout_vault_core::totp;
use scout_vault_core::vault::{FieldKind, Item, ItemKind, Keys};

use crate::flow::{self, Session, Unlocked, WriteOp};
use crate::item_editor::{EditorEvent, ItemEditor, NewKind, Places};
use crate::item_list::{Filter, ItemList, KindFilter};
use crate::{appearance, clipboard, motion, ui};
use gpui_kit::assets::IconName;
use gpui_kit::component::color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use scout_vault_core::generator::PasswordOptions;

/// Width of the sidebar (the title bar continues its column).
pub const SIDEBAR_WIDTH: f32 = 240.;

pub enum VaultEvent {
    /// Asks to lock; carries the session (with possibly renewed tokens).
    Lock(Session),
    OpenSettings,
    /// The session expired on the server: back to login.
    SessionExpired,
    /// Synced: new tokens and cache to store.
    Synced(Session),
    /// Quick access changed: ids to store in the settings.
    PinsChanged(Vec<String>),
    OpenGenerator,
    /// Open the "Password health" screen.
    OpenHealth,
    /// Open the profile screen (who is signed in).
    OpenProfile,
    /// A sidebar section was collapsed/expanded or an entry's icon changed: to store.
    SidebarChanged {
        collapsed: Vec<String>,
        icons: std::collections::HashMap<String, crate::item_icons::CustomIcon>,
    },
    /// Some item's icon changed: all choices, to store.
    IconsChanged(std::collections::HashMap<String, crate::item_icons::CustomIcon>),
    /// The master password asked for by an item was confirmed for an app action
    /// (e.g. typing with `Ctrl+Alt+A`); carries the number the app passed.
    RepromptPassed(u64),
}

/// Action on an item that asks for the master password, kept until the password is confirmed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Guarded {
    Reveal(String),
    Copy(String),
    Edit,
    /// App action (type, fill); the number comes back in [`VaultEvent::RepromptPassed`].
    External(u64),
}

/// The master password prompt open in the item detail.
struct RepromptPrompt {
    item_id: String,
    input: Entity<InputState>,
    _sub: Subscription,
    pending: Option<Guarded>,
    error: Option<SharedString>,
    checking: bool,
}

/// How often the app asks the server whether the vault changed.
pub const BACKGROUND_SYNC_EVERY: std::time::Duration = std::time::Duration::from_secs(60);

/// What the copy shortcuts copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyShortcut {
    Username,
    Password,
    TotpCode,
}

impl CopyShortcut {
    fn key(self) -> &'static str {
        match self {
            Self::Username => "username",
            Self::Password => "password",
            Self::TotpCode => TOTP_CODE_KEY,
        }
    }

    fn missing(self) -> &'static str {
        match self {
            Self::Username => t!("vault.no_username"),
            Self::Password => t!("vault.no_password"),
            Self::TotpCode => t!("vault.no_totp"),
        }
    }
}

/// A displayable field of the selected item.
struct FieldRow<'a> {
    key: String,
    label: String,
    value: Cow<'a, str>,
    secret: bool,
    multiline: bool,
    /// Short text next to the label (e.g. seconds left on the TOTP code).
    hint: Option<String>,
}

fn add<'a>(
    rows: &mut Vec<FieldRow<'a>>,
    key: &str,
    label: &str,
    value: Option<&'a str>,
    secret: bool,
) {
    if let Some(v) = value.filter(|v| !v.is_empty()) {
        rows.push(FieldRow {
            key: key.to_owned(),
            label: label.to_owned(),
            value: Cow::Borrowed(v),
            secret,
            multiline: v.contains('\n'),
            hint: None,
        });
    }
}

const TOTP_CODE_KEY: &str = "totp-code";

/// Current TOTP period (changes every 30 s), to animate the code change.
fn totp_step(key: &str) -> u64 {
    if key != TOTP_CODE_KEY {
        return 0;
    }
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() / 30)
        .unwrap_or(0)
}

/// The item's current TOTP code, if it has a valid key.
fn live_totp(item: &Item) -> Option<totp::Code> {
    let ItemKind::Login(l) = &item.kind else {
        return None;
    };
    Some(totp::Totp::parse(l.totp.as_ref()?.expose())?.now())
}

fn field_rows(item: &Item) -> Vec<FieldRow<'_>> {
    let mut rows = Vec::new();

    match &item.kind {
        ItemKind::Login(l) => {
            add(
                &mut rows,
                "username",
                t!("field.username"),
                l.username.as_deref(),
                false,
            );
            add(
                &mut rows,
                "password",
                t!("field.password"),
                l.password.as_ref().map(|s| s.expose()),
                true,
            );
            if let Some(code) = live_totp(item) {
                rows.push(FieldRow {
                    key: TOTP_CODE_KEY.into(),
                    label: t!("field.totp_code").into(),
                    value: Cow::Owned(totp::group(&code.code)),
                    secret: false,
                    multiline: false,
                    hint: Some(t!("field.seconds_left", n = code.remaining)),
                });
            }
            add(
                &mut rows,
                "totp",
                t!("field.totp_key"),
                l.totp.as_ref().map(|s| s.expose()),
                true,
            );
            for (i, uri) in l.uris.iter().enumerate() {
                add(
                    &mut rows,
                    &format!("uri-{i}"),
                    t!("field.website"),
                    Some(uri),
                    false,
                );
            }
        }
        ItemKind::Card(c) => {
            add(
                &mut rows,
                "card-holder",
                t!("field.cardholder"),
                c.cardholder_name.as_deref(),
                false,
            );
            add(
                &mut rows,
                "card-brand",
                t!("field.brand"),
                c.brand.as_deref(),
                false,
            );
            add(
                &mut rows,
                "card-number",
                t!("field.number"),
                c.number.as_ref().map(|s| s.expose()),
                true,
            );
            let exp = match (&c.exp_month, &c.exp_year) {
                (Some(m), Some(y)) => Some(format!("{m:0>2}/{y}")),
                (Some(m), None) => Some(m.clone()),
                (None, Some(y)) => Some(y.clone()),
                (None, None) => None,
            };
            if let Some(exp) = exp {
                rows.push(FieldRow {
                    key: "card-exp".into(),
                    label: t!("field.expiration").into(),
                    value: Cow::Owned(exp),
                    secret: false,
                    multiline: false,
                    hint: None,
                });
            }
            add(
                &mut rows,
                "card-code",
                t!("field.security_code"),
                c.code.as_ref().map(|s| s.expose()),
                true,
            );
        }
        ItemKind::Identity(i) => {
            add(
                &mut rows,
                "id-title",
                t!("field.title"),
                i.title.as_deref(),
                false,
            );
            add(
                &mut rows,
                "id-first",
                t!("field.first_name"),
                i.first_name.as_deref(),
                false,
            );
            add(
                &mut rows,
                "id-middle",
                t!("field.middle_name"),
                i.middle_name.as_deref(),
                false,
            );
            add(
                &mut rows,
                "id-last",
                t!("field.last_name"),
                i.last_name.as_deref(),
                false,
            );
            add(
                &mut rows,
                "id-company",
                t!("field.company"),
                i.company.as_deref(),
                false,
            );
            add(
                &mut rows,
                "id-email",
                t!("login.email"),
                i.email.as_deref(),
                false,
            );
            add(
                &mut rows,
                "id-phone",
                t!("field.phone"),
                i.phone.as_deref(),
                false,
            );
            add(
                &mut rows,
                "id-username",
                t!("field.username"),
                i.username.as_deref(),
                false,
            );
            add(
                &mut rows,
                "id-address1",
                t!("field.address"),
                i.address1.as_deref(),
                false,
            );
            add(
                &mut rows,
                "id-address2",
                t!("field.address2"),
                i.address2.as_deref(),
                false,
            );
            add(
                &mut rows,
                "id-address3",
                t!("field.address3"),
                i.address3.as_deref(),
                false,
            );
            add(
                &mut rows,
                "id-city",
                t!("field.city"),
                i.city.as_deref(),
                false,
            );
            add(
                &mut rows,
                "id-state",
                t!("field.state"),
                i.state.as_deref(),
                false,
            );
            add(
                &mut rows,
                "id-postal",
                t!("field.postal_code"),
                i.postal_code.as_deref(),
                false,
            );
            add(
                &mut rows,
                "id-country",
                t!("field.country"),
                i.country.as_deref(),
                false,
            );
            add(
                &mut rows,
                "id-ssn",
                t!("field.ssn"),
                i.ssn.as_ref().map(|s| s.expose()),
                true,
            );
            add(
                &mut rows,
                "id-passport",
                t!("field.passport"),
                i.passport_number.as_ref().map(|s| s.expose()),
                true,
            );
            add(
                &mut rows,
                "id-license",
                t!("field.license"),
                i.license_number.as_ref().map(|s| s.expose()),
                true,
            );
        }
        ItemKind::SshKey(k) => {
            add(
                &mut rows,
                "ssh-fingerprint",
                t!("field.fingerprint"),
                k.fingerprint.as_deref(),
                false,
            );
            add(
                &mut rows,
                "ssh-public",
                t!("field.public_key"),
                k.public_key.as_deref(),
                false,
            );
            add(
                &mut rows,
                "ssh-private",
                t!("field.private_key"),
                k.private_key.as_ref().map(|s| s.expose()),
                true,
            );
        }
        ItemKind::SecureNote | ItemKind::Unknown(_) => {}
    }

    for (i, f) in item.fields.iter().enumerate() {
        let label = f.name.clone().unwrap_or_else(|| t!("field.custom").into());
        let secret = f.kind == FieldKind::Hidden;
        if let Some(v) = f
            .value
            .as_ref()
            .map(|s| s.expose())
            .filter(|v| !v.is_empty())
        {
            rows.push(FieldRow {
                key: format!("field-{i}"),
                label,
                value: Cow::Borrowed(v),
                secret,
                multiline: v.contains('\n'),
                hint: None,
            });
        }
    }

    add(
        &mut rows,
        "notes",
        t!("field.notes"),
        item.notes.as_ref().map(|s| s.expose()),
        true,
    );
    rows
}

pub struct VaultView {
    session: Session,
    keys: Arc<Keys>,
    list: Entity<ListState<ItemList>>,
    selected: Option<Item>,
    revealed: HashSet<String>,
    syncing: bool,
    _subscriptions: Vec<Subscription>,
    /// Redraws every second while the open item has a TOTP code.
    _ticker: Task<()>,
    /// Asks the server, from time to time, whether the vault changed.
    _poller: Task<()>,
    /// Sync on its own when the vault changes on another device.
    background_sync: bool,
    /// Vault revision date at the last sync (ms), for comparison.
    last_revision: Option<i64>,
    /// Editor open in place of the detail (new item or edit).
    editor: Option<(Entity<ItemEditor>, Subscription)>,
    /// "Delete permanently" asks for a second click.
    confirm_delete: bool,
    /// Icon picker open in the item detail.
    icon_picker: bool,
    /// When a click outside closed the picker: the click on the item's icon
    /// that follows it must not open it again.
    icon_picker_closed_at: Option<std::time::Instant>,
    /// The picker is collapsing (still drawn until the animation ends).
    icon_picker_closing: bool,
    /// Counts openings/closings: restarts the animation and ignores stale timers.
    icon_picker_anim: u32,
    /// Last measured height of the picker (for the expand/collapse animation).
    icon_picker_height: std::rc::Rc<std::cell::Cell<f32>>,
    /// "Any color" of the icon picker.
    icon_color: Entity<ColorPickerState>,
    _icon_color_sub: Subscription,
    /// Generator options, for the generate password button in the editor.
    password_options: PasswordOptions,
    /// "Remove attachment" asks for a second click.
    confirm_remove_attachment: Option<String>,
    /// Field copied a moment ago: the button shows ✓ for an instant.
    copied: Option<(String, u32)>,
    /// Items pinned to quick access, in the order they were pinned.
    pinned: Vec<String>,
    /// "New folder" field open in the sidebar.
    new_folder: Option<(Entity<InputState>, Subscription)>,
    /// Folder being renamed (id + field) in the list header.
    renaming_folder: Option<(String, Entity<InputState>, Subscription)>,
    /// "Delete folder" asks for a second click.
    confirm_delete_folder: bool,
    /// Filter to apply on the next frame (after creating/deleting a folder).
    pending_filter: Option<Filter>,
    /// Result of the last sync (ok?, counter), shown on the icon for an instant.
    sync_feedback: Option<(bool, u32)>,
    /// How many syncs have finished (restarts the icon animations).
    sync_count: u32,
    /// Position of each sidebar row in the current frame (for the cascade).
    sidebar_order: std::cell::Cell<usize>,
    /// Collapsed sidebar sections (`quick`, `types`, `orgs`, `folders`).
    collapsed: HashSet<String>,
    /// Icon (and color) chosen per sidebar entry, by the entry's id.
    sidebar_icons: std::collections::HashMap<String, crate::item_icons::CustomIcon>,
    /// The sidebar entry whose icon menu is open.
    sidebar_icon_menu: Option<String>,
    /// Attachment being downloaded right now.
    downloading: Option<String>,
    /// Temporary copies opened with another program; deleted on lock.
    temp_files: Vec<std::path::PathBuf>,
    /// Master password prompt of an item with "ask for the master password".
    reprompt: Option<RepromptPrompt>,
    /// Items released with the master password (until switching items or locking).
    reprompt_ok: HashSet<String>,
}

impl Drop for VaultView {
    fn drop(&mut self) {
        // Locking/signing out drops this screen: the decrypted copies go with it.
        for path in &self.temp_files {
            crate::attachments::remove_temp(path);
        }
    }
}

impl EventEmitter<VaultEvent> for VaultView {}

/// Text color, for the ring around the chosen color.
fn theme_fg(cx: &App) -> Hsla {
    cx.theme().foreground
}

impl VaultView {
    pub fn new(unlocked: Unlocked, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let Unlocked {
            session,
            keys,
            vault,
        } = unlocked;
        let errors = vault.errors.len();
        let list = cx.new(|cx| ListState::new(ItemList::new(vault), window, cx).searchable(true));
        let sub = cx.subscribe(&list, |this, list, event: &ListEvent, cx| match event {
            ListEvent::Select(ix) | ListEvent::Confirm(ix) => {
                this.selected = list.read(cx).delegate().item_at(*ix).cloned();
                this.revealed.clear();
                this.leave_editor(cx);
                cx.notify();
            }
            ListEvent::Cancel => {}
        });
        // Background sync: a cheap question to the server ("changed?") and,
        // if it changed, a full sync.
        let poller = cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(BACKGROUND_SYNC_EVERY).await;
            let session = match this.update(cx, |this, _| this.poll_session()) {
                Ok(session) => session,
                Err(_) => break,
            };
            let Some(session) = session else { continue };
            let result = flow::check_revision(&session).await;
            if this
                .update(cx, |this, cx| this.revision_checked(result, cx))
                .is_err()
            {
                break;
            }
        });
        let ticker = cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(1))
                .await;
            let alive = this.update(cx, |this, cx| {
                if this
                    .selected
                    .as_ref()
                    .is_some_and(|i| live_totp(i).is_some())
                {
                    cx.notify();
                }
            });
            if alive.is_err() {
                break;
            }
        });
        let icon_color = cx.new(|cx| ColorPickerState::new(window, cx));
        let _icon_color_sub = cx.subscribe(&icon_color, |this, _, event: &ColorPickerEvent, cx| {
            let ColorPickerEvent::Change(Some(color)) = event else {
                return;
            };
            let hex = crate::colors::Rgb::from_hsla(*color).to_hex();
            this.choose_icon_color(Some(hex), cx);
        });
        let mut this = Self {
            editor: None,
            confirm_delete: false,
            icon_picker: false,
            icon_picker_closed_at: None,
            icon_picker_closing: false,
            icon_picker_anim: 0,
            icon_picker_height: Default::default(),
            icon_color,
            _icon_color_sub,
            confirm_remove_attachment: None,
            copied: None,
            sidebar_order: std::cell::Cell::new(0),
            collapsed: HashSet::new(),
            sidebar_icons: Default::default(),
            sidebar_icon_menu: None,
            sync_feedback: None,
            new_folder: None,
            pinned: Vec::new(),
            renaming_folder: None,
            confirm_delete_folder: false,
            pending_filter: None,
            sync_count: 0,
            downloading: None,
            temp_files: Vec::new(),
            reprompt: None,
            reprompt_ok: HashSet::new(),
            password_options: PasswordOptions::default(),
            _ticker: ticker,
            _poller: poller,
            background_sync: true,
            last_revision: None,
            session,
            keys: Arc::new(keys),
            list,
            selected: None,
            revealed: HashSet::new(),
            syncing: false,
            _subscriptions: vec![sub],
        };
        if errors > 0 {
            crate::toast::error(t!("vault.decrypt_errors", n = errors), cx);
        }
        // Opened from the remembered session (no access token): fetch what
        // changed on the server. Without network, the vault stays open with the cache.
        if this.session.access_token.is_empty() {
            this.sync(cx);
        }
        this
    }

    /// Items the person can change (outside the trash; org only if allowed).
    pub fn editable_items(&self, cx: &App) -> Vec<Item> {
        self.list
            .read(cx)
            .delegate()
            .vault()
            .items
            .iter()
            .filter(|i| !i.deleted && self.can_edit(i))
            .cloned()
            .collect()
    }

    /// Saves an item that came from outside (the browser). Doesn't touch an open
    /// form: with one open (or while syncing), refuses and explains.
    pub fn save_external(
        &mut self,
        item: Item,
        message: String,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if self.editor.is_some() {
            return Err(t!("vault.form_open").into());
        }
        if self.syncing {
            return Err(t!("vault.busy_syncing").into());
        }
        self.run_write_with(WriteOp::Save(Box::new(item), Vec::new()), Some(message), cx);
        Ok(())
    }

    pub fn set_background_sync(&mut self, on: bool) {
        self.background_sync = on;
    }

    pub fn last_revision(&self) -> Option<i64> {
        self.last_revision
    }

    /// Session for the background check — or `None` if now isn't the time
    /// (option off, already syncing, form open).
    fn poll_session(&self) -> Option<Session> {
        (self.background_sync && !self.syncing && self.editor.is_none())
            .then(|| self.session.clone())
    }

    fn revision_checked(
        &mut self,
        result: Result<flow::Revision, flow::SyncError>,
        cx: &mut Context<Self>,
    ) {
        // No network or server down: try again next time, without a notice.
        let Ok(flow::Revision { date, tokens }) = result else {
            return;
        };
        if let Some((access, refresh)) = tokens {
            self.session.access_token = access;
            if let Some(refresh) = refresh {
                self.session.refresh_token = refresh;
            }
            cx.emit(VaultEvent::Synced(self.session.clone()));
        }
        self.on_revision(date, cx);
    }

    /// Asks now (without waiting for the next minute) whether the vault changed.
    pub fn check_now(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.poll_session() else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let result = flow::check_revision(&session).await;
            this.update(cx, |this, cx| this.revision_checked(result, cx))
                .ok();
        })
        .detach();
    }

    /// The server said the vault is at revision `date`: if it changed since the
    /// last sync (on another device, in the web vault…), sync now.
    pub fn on_revision(&mut self, date: i64, cx: &mut Context<Self>) {
        match self.last_revision {
            None => self.last_revision = Some(date),
            Some(last) if last != date => {
                if self.poll_session().is_some() {
                    self.run_write_with(
                        WriteOp::Nothing,
                        Some(t!("vault.updated_elsewhere").into()),
                        cx,
                    );
                }
            }
            Some(_) => {}
        }
    }

    /// Keys of the open vault (to turn on Windows Hello).
    pub fn keys(&self) -> Arc<Keys> {
        self.keys.clone()
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    pub fn list(&self) -> &Entity<ListState<ItemList>> {
        &self.list
    }

    /// Syncing or saving something to the server right now.
    pub fn is_busy(&self) -> bool {
        self.syncing
    }

    pub fn selected(&self) -> Option<&Item> {
        self.selected.as_ref()
    }

    pub fn is_revealed(&self, key: &str) -> bool {
        self.revealed.contains(key)
    }

    pub fn select_item(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let ix = self.list.read(cx).delegate().position_of(id);
        self.list
            .update(cx, |list, cx| list.set_selected_index(ix, window, cx));
        self.selected = ix.and_then(|ix| self.list.read(cx).delegate().item_at(ix).cloned());
        self.revealed.clear();
        self.leave_editor(cx);
        cx.notify();
    }

    /// Choosing another item shows it: closes the open form (new item or
    /// edit) and the "click again to confirm" prompts. While the form is
    /// saving, it stays — the result will still show up in it.
    fn leave_editor(&mut self, cx: &mut Context<Self>) {
        self.forget_reprompt();
        self.confirm_delete = false;
        self.icon_picker = false;
        self.icon_picker_closing = false;
        self.confirm_remove_attachment = None;
        if let Some((editor, _)) = &self.editor {
            if !editor.read(cx).is_saving() {
                self.editor = None;
            }
        }
    }

    pub fn set_filter(&mut self, filter: Filter, window: &mut Window, cx: &mut Context<Self>) {
        self.list.update(cx, |list, cx| {
            list.delegate_mut().set_filter(filter);
            list.set_selected_index(None, window, cx);
            cx.notify();
        });
        self.selected = None;
        self.revealed.clear();
        self.forget_reprompt();
        cx.notify();
    }

    /// Another item (or filter): the item asks for the master password again.
    fn forget_reprompt(&mut self) {
        let keep = self.selected.as_ref().map(|i| i.id.clone());
        if self.reprompt.as_ref().map(|r| Some(&r.item_id)) != Some(keep.as_ref()) {
            self.reprompt = None;
        }
        self.reprompt_ok.retain(|id| Some(id) == keep.as_ref());
    }

    /// Does the open item ask for the master password and it hasn't been confirmed yet?
    pub fn needs_reprompt(&self, item: &Item) -> bool {
        item.reprompt && !self.reprompt_ok.contains(&item.id)
    }

    pub fn reprompt_open(&self) -> bool {
        self.reprompt.is_some()
    }

    /// Allows `action` on the open item, or opens the master password prompt and
    /// keeps the action for later. `true` = can go ahead now.
    fn guard(&mut self, action: Guarded, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(item) = self.selected.clone() else {
            return false;
        };
        if !self.needs_reprompt(&item) {
            return true;
        }
        self.start_reprompt(window, cx);
        if let Some(prompt) = &mut self.reprompt {
            prompt.pending = Some(action);
        }
        false
    }

    /// Opens (or focuses) the master password field of the open item.
    pub fn start_reprompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = &self.selected else { return };
        let item_id = item.id.clone();
        if let Some(prompt) = self.reprompt.as_ref().filter(|p| p.item_id == item_id) {
            prompt.input.update(cx, |i, cx| i.focus(window, cx));
            return;
        }
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder(t!("common.vault_password"))
        });
        let sub = cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.confirm_reprompt(window, cx);
            }
        });
        input.update(cx, |i, cx| i.focus(window, cx));
        self.reprompt = Some(RepromptPrompt {
            item_id,
            input,
            _sub: sub,
            pending: None,
            error: None,
            checking: false,
        });
        cx.notify();
    }

    fn cancel_reprompt(&mut self, cx: &mut Context<Self>) {
        self.reprompt = None;
        cx.notify();
    }

    /// Checks the typed password; if right, releases the item and runs the kept action.
    pub fn confirm_reprompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(prompt) = &mut self.reprompt else {
            return;
        };
        if prompt.checking {
            return;
        }
        let password = zeroize::Zeroizing::new(prompt.input.read(cx).value().to_string());
        if password.is_empty() {
            prompt.error = Some(t!("vault.type_password").into());
            cx.notify();
            return;
        }
        prompt.checking = true;
        prompt.error = None;
        let item_id = prompt.item_id.clone();
        cx.notify();
        let session = self.session.clone();
        cx.spawn_in(window, async move |this, cx| {
            let ok = flow::check_master_password(&session, password).await;
            this.update_in(cx, |this, window, cx| {
                // Switched items while checking: release nothing.
                if this.reprompt.as_ref().map(|p| &p.item_id) != Some(&item_id) {
                    return;
                }
                if !ok {
                    if let Some(prompt) = &mut this.reprompt {
                        prompt.checking = false;
                        prompt.error = Some(t!("vault.wrong_password").into());
                        prompt.input.update(cx, |i, cx| i.set_value("", window, cx));
                    }
                    cx.notify();
                    return;
                }
                let pending = this.reprompt.take().and_then(|p| p.pending);
                this.reprompt_ok.insert(item_id);
                match pending {
                    Some(Guarded::Reveal(key)) => {
                        this.revealed.insert(key);
                    }
                    Some(Guarded::Copy(key)) => {
                        if let Some(item) = this.selected.clone() {
                            this.copy_from(&item, &key, cx);
                        }
                    }
                    Some(Guarded::Edit) => this.edit_selected(window, cx),
                    Some(Guarded::External(token)) => cx.emit(VaultEvent::RepromptPassed(token)),
                    None => {}
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// For an app action (typing, for example) on an item that asks for the master
    /// password: opens the item and the prompt. `true` = not needed, can go ahead.
    pub fn request_reprompt(
        &mut self,
        item_id: &str,
        token: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.open_pinned(item_id, window, cx);
        self.guard(Guarded::External(token), window, cx)
    }

    fn toggle_reveal(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.revealed.remove(key) {
            cx.notify();
            return;
        }
        if self.guard(Guarded::Reveal(key.to_owned()), window, cx) {
            self.revealed.insert(key.to_owned());
        }
        cx.notify();
    }

    /// Copy shortcut (`Ctrl+U`, `Ctrl+P`, `Ctrl+T`) on the open item.
    pub fn copy_shortcut(
        &mut self,
        what: CopyShortcut,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = &self.selected else {
            let _ = window;
            crate::toast::info(t!("vault.open_item_first"), cx);
            return;
        };
        let key = what.key();
        if field_rows(item).iter().any(|r| r.key == key) {
            self.copy_field(key, window, cx);
        } else {
            crate::toast::info(what.missing(), cx);
        }
    }

    /// Focuses the list search (`Ctrl+F`).
    pub fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.list.update(cx, |list, cx| list.focus(window, cx));
    }

    fn copy_field(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.selected.clone() else {
            return;
        };
        let secret = field_rows(&item).iter().any(|r| r.key == key && r.secret);
        if secret && !self.guard(Guarded::Copy(key.to_owned()), window, cx) {
            return;
        }
        self.copy_from(&item, key, cx);
    }

    /// Copies a field of any item (from the tray menu), without opening it.
    /// If the item asks for the master password, opens it with the prompt and returns `true`:
    /// the app needs to show the window.
    pub fn copy_item_field(
        &mut self,
        id: &str,
        what: CopyShortcut,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let item = self
            .list
            .read(cx)
            .delegate()
            .vault()
            .items
            .iter()
            .find(|i| i.id == id && !i.deleted)
            .cloned();
        let Some(item) = item else {
            crate::toast::error(t!("vault.item_gone"), cx);
            return false;
        };
        let Some(row) = field_rows(&item).into_iter().find(|r| r.key == what.key()) else {
            crate::toast::info(what.missing(), cx);
            return false;
        };
        if row.secret && item.reprompt {
            self.open_pinned(id, window, cx);
            self.guard(Guarded::Copy(what.key().to_owned()), window, cx);
            return true;
        }
        self.copy_from(&item, what.key(), cx);
        false
    }

    /// What can be copied from the item: (username, password, TOTP code).
    pub fn copyable(item: &Item) -> (bool, bool, bool) {
        let rows = field_rows(item);
        let has = |what: CopyShortcut| rows.iter().any(|r| r.key == what.key());
        (
            has(CopyShortcut::Username),
            has(CopyShortcut::Password),
            has(CopyShortcut::TotpCode),
        )
    }

    fn copy_from(&mut self, item: &Item, key: &str, cx: &mut Context<Self>) {
        let Some(row) = field_rows(item).into_iter().find(|r| r.key == key) else {
            return;
        };
        let value = if row.key == TOTP_CODE_KEY {
            Cow::Owned(row.value.replace(' ', ""))
        } else {
            row.value.clone()
        };
        clipboard::copy_and_clear_later(&value, cx);
        // ✓ on the button for an instant (the counter restarts the animation on each copy).
        let n = self.copied.as_ref().map_or(0, |(_, n)| n + 1);
        self.copied = Some((key.to_owned(), n));
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(1300))
                .await;
            this.update(cx, |this, cx| {
                if this.copied.as_ref().is_some_and(|(_, m)| *m == n) {
                    this.copied = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
        let message = if row.secret {
            t!(
                "vault.copied_secret",
                label = row.label,
                note = clipboard::clear_note(cx)
            )
        } else {
            t!("vault.copied", label = row.label)
        };
        crate::toast::success(message, cx);
    }

    fn request_lock(&mut self, cx: &mut Context<Self>) {
        cx.emit(VaultEvent::Lock(self.session.clone()));
    }

    fn sync(&mut self, cx: &mut Context<Self>) {
        self.run_write(WriteOp::Nothing, cx);
    }

    /// Syncs now (sidebar button or tray menu).
    pub fn sync_now(&mut self, cx: &mut Context<Self>) {
        self.sync(cx);
    }

    pub fn set_password_options(&mut self, options: PasswordOptions) {
        self.password_options = options;
    }

    pub fn editor(&self) -> Option<&Entity<ItemEditor>> {
        self.editor.as_ref().map(|(e, _)| e)
    }

    /// Folders, organizations and collections, for the editor.
    fn places(&self, cx: &App) -> Places {
        let vault = self.list.read(cx).delegate().vault();
        Places {
            folders: vault
                .folders
                .iter()
                .map(|f| (f.id.clone(), f.name.clone()))
                .collect(),
            orgs: vault
                .organizations
                .iter()
                .map(|o| (o.id.clone(), o.name.clone()))
                .collect(),
            collections: vault.collections.clone(),
        }
    }

    /// Does the server allow editing this item? (organization items may be read-only)
    pub fn can_edit(&self, item: &Item) -> bool {
        !matches!(item.kind, ItemKind::Unknown(_))
            && self
                .session
                .sync
                .ciphers
                .iter()
                .find(|c| c.id == item.id)
                .is_none_or(|c| c.extra.get("edit").and_then(|v| v.as_bool()) != Some(false))
    }

    fn open_editor(
        &mut self,
        editor: Entity<ItemEditor>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let sub = cx.subscribe_in(
            &editor,
            window,
            |this, _, event: &EditorEvent, _, cx| match event {
                EditorEvent::Cancel => {
                    this.editor = None;
                    cx.notify();
                }
                EditorEvent::Save(item, files) => {
                    this.run_write(WriteOp::Save(item.clone(), files.clone()), cx)
                }
            },
        );
        self.editor = Some((editor, sub));
        self.confirm_delete = false;
        cx.notify();
    }

    /// How many items fit in quick access.
    pub const MAX_PINNED: usize = 10;

    /// The sidebar as the person left it (from the settings).
    pub fn set_sidebar(
        &mut self,
        collapsed: Vec<String>,
        icons: std::collections::HashMap<String, crate::item_icons::CustomIcon>,
    ) {
        self.collapsed = collapsed.into_iter().collect();
        self.sidebar_icons = icons;
    }

    pub fn is_collapsed(&self, section: &str) -> bool {
        self.collapsed.contains(section)
    }

    fn emit_sidebar(&self, cx: &mut Context<Self>) {
        let mut collapsed: Vec<String> = self.collapsed.iter().cloned().collect();
        collapsed.sort();
        cx.emit(VaultEvent::SidebarChanged {
            collapsed,
            icons: self.sidebar_icons.clone(),
        });
    }

    /// Collapses or expands a sidebar section (remembered).
    pub fn toggle_section(&mut self, section: &str, cx: &mut Context<Self>) {
        if !self.collapsed.remove(section) {
            self.collapsed.insert(section.to_owned());
        }
        self.emit_sidebar(cx);
        cx.notify();
    }

    /// Opens (or closes) the icon menu of a sidebar entry.
    pub fn toggle_sidebar_icon_menu(&mut self, entry: &str, cx: &mut Context<Self>) {
        self.sidebar_icon_menu = match &self.sidebar_icon_menu {
            Some(open) if open == entry => None,
            _ => Some(entry.to_owned()),
        };
        cx.notify();
    }

    /// Chooses the icon of a sidebar entry (`None` = back to the default).
    pub fn set_sidebar_icon(&mut self, entry: &str, icon: Option<&str>, cx: &mut Context<Self>) {
        let color = self.sidebar_icons.get(entry).and_then(|c| c.color.clone());
        match (icon, color) {
            (None, None) => {
                self.sidebar_icons.remove(entry);
            }
            (icon, color) => {
                self.sidebar_icons.insert(
                    entry.to_owned(),
                    crate::item_icons::CustomIcon {
                        icon: icon.unwrap_or_default().to_owned(),
                        color,
                    },
                );
            }
        }
        self.emit_sidebar(cx);
        cx.notify();
    }

    /// Chooses the icon color of a sidebar entry (`None` = the theme's).
    pub fn set_sidebar_color(&mut self, entry: &str, color: Option<&str>, cx: &mut Context<Self>) {
        let icon = self
            .sidebar_icons
            .get(entry)
            .map(|c| c.icon.clone())
            .unwrap_or_default();
        if icon.is_empty() && color.is_none() {
            self.sidebar_icons.remove(entry);
        } else {
            self.sidebar_icons.insert(
                entry.to_owned(),
                crate::item_icons::CustomIcon {
                    icon,
                    color: color.map(str::to_owned),
                },
            );
        }
        self.emit_sidebar(cx);
        cx.notify();
    }

    /// Back to the entry's own icon and color.
    pub fn reset_sidebar_icon(&mut self, entry: &str, cx: &mut Context<Self>) {
        self.sidebar_icons.remove(entry);
        self.sidebar_icon_menu = None;
        self.emit_sidebar(cx);
        cx.notify();
    }

    pub fn sidebar_icon(&self, entry: &str) -> Option<&crate::item_icons::CustomIcon> {
        self.sidebar_icons.get(entry)
    }

    /// A section heading: click to collapse/expand; the arrow turns.
    fn section_header(
        &self,
        key: &'static str,
        label: &'static str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        use std::f32::consts::TAU;
        let open = !self.collapsed.contains(key);
        let muted = cx.theme().muted_foreground.opacity(0.8);
        let hover = cx.theme().foreground.opacity(0.9);
        h_flex()
            .id(SharedString::from(format!("section-{key}")))
            .test_support()
            .role(Role::Button)
            .aria_label(SharedString::from(label))
            .flex_1()
            .min_w_0()
            .gap_1()
            .items_center()
            .cursor_pointer()
            .text_color(muted)
            .hover(move |s| s.text_color(hover))
            .child(div().flex_1().min_w_0().child(ui::section_label(label, cx)))
            .child(div().pt_2().pr_1().child(
                Icon::new(IconName::ChevronRight).xsmall().with_animation(
                    motion::key("secao", format!("{key}:{open}")),
                    Animation::new(motion::NORMAL).with_easing(ease_out_quint()),
                    move |icon, t| {
                        let (from, to) = if open { (0., TAU / 4.) } else { (TAU / 4., 0.) };
                        icon.rotate(radians(from + (to - from) * t))
                    },
                ),
            ))
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_section(key, cx)))
    }

    /// The icon menu of a sidebar entry: icons and colors.
    fn render_sidebar_icon_menu(&self, entry: &str, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (border, bg, hover, radius) = (
            theme.border,
            theme.popover,
            theme.secondary_hover,
            theme.radius,
        );
        let accent = appearance::accent(cx);
        let current = self.sidebar_icons.get(entry).cloned().unwrap_or_default();
        let mut grid = h_flex().flex_wrap().gap_0p5();
        for (name, icon) in crate::item_icons::CHOICES {
            let chosen = current.icon == *name;
            let entry_id = entry.to_owned();
            grid = grid.child(
                div()
                    .id(SharedString::from(format!("sidebar-icon-{name}")))
                    .test_support()
                    .role(Role::Button)
                    .aria_label(SharedString::from(*name))
                    .size(px(26.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(radius)
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .when(chosen, |el| el.bg(accent.opacity(0.18)).text_color(accent))
                    .child(Icon::new(*icon).small())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_sidebar_icon(&entry_id, Some(name), cx)
                    })),
            );
        }
        let mut colors = h_flex().gap_1p5().items_center();
        for (id, _, _) in crate::item_icons::COLORS {
            let color = crate::item_icons::color_by_id(id, cx).unwrap_or(accent);
            let chosen = current.color.as_deref() == Some(*id);
            let entry_id = entry.to_owned();
            colors = colors.child(
                div()
                    .id(SharedString::from(format!("sidebar-color-{id}")))
                    .test_support()
                    .role(Role::Button)
                    .aria_label(SharedString::from(*id))
                    .size(px(16.))
                    .rounded_full()
                    .cursor_pointer()
                    .bg(color)
                    .when(chosen, |el| el.border_2().border_color(theme_fg(cx)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_sidebar_color(&entry_id, Some(id), cx)
                    })),
            );
        }
        let entry_id = entry.to_owned();
        let closing = entry.to_owned();
        motion::rise(
            v_flex()
                .id(SharedString::from(format!("sidebar-icon-menu-{entry}")))
                .test_support()
                .mx_1()
                .my_1()
                .p_2()
                .gap_2()
                .rounded(radius)
                .border_1()
                .border_color(border)
                .bg(bg)
                .shadow_md()
                .child(grid)
                .child(colors)
                .child(
                    h_flex().justify_end().child(
                        Button::new("sidebar-icon-reset")
                            .ghost()
                            .xsmall()
                            .label(t!("vault.sidebar_icon_reset"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.reset_sidebar_icon(&entry_id, cx)
                            })),
                    ),
                )
                .on_mouse_down_out(cx.listener(move |this, _, _, cx| {
                    if this.sidebar_icon_menu.as_deref() == Some(closing.as_str()) {
                        this.sidebar_icon_menu = None;
                        cx.notify();
                    }
                })),
            motion::key("menu-icone-lateral", entry),
            4.,
        )
    }

    pub fn set_pinned(&mut self, ids: Vec<String>) {
        self.pinned = ids;
    }

    pub fn pinned(&self) -> &[String] {
        &self.pinned
    }

    /// Pins or unpins the open item in quick access.
    /// Marks or unmarks the open item as favorite (right away on screen, then on the server).
    pub fn toggle_favorite(&mut self, cx: &mut Context<Self>) {
        let Some(item) = &mut self.selected else {
            return;
        };
        if item.deleted || self.syncing {
            return;
        }
        item.favorite = !item.favorite;
        let op = WriteOp::SetFavorite {
            item_id: item.id.clone(),
            favorite: item.favorite,
        };
        self.run_write(op, cx);
    }

    pub fn toggle_pin(&mut self, cx: &mut Context<Self>) {
        let Some(item) = &self.selected else { return };
        let id = item.id.clone();
        if let Some(pos) = self.pinned.iter().position(|p| *p == id) {
            self.pinned.remove(pos);
            crate::toast::info(t!("vault.unpinned"), cx);
        } else if self.pinned.len() >= Self::MAX_PINNED {
            crate::toast::error(t!("vault.pins_full", n = Self::MAX_PINNED), cx);
            return;
        } else {
            self.pinned.push(id);
            crate::toast::success(t!("vault.pinned"), cx);
        }
        cx.emit(VaultEvent::PinsChanged(self.pinned.clone()));
        cx.notify();
    }

    /// Opens a quick access item (showing "All" if the current filter hides it).
    pub fn open_pinned(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let visible = self
            .list
            .read(cx)
            .delegate()
            .visible_items()
            .any(|i| i.id == id);
        if !visible {
            self.set_filter(Filter::All, window, cx);
        }
        self.select_item(id, window, cx);
    }

    /// Pinned items that still exist (outside the trash), in pinning order.
    pub fn pinned_items(&self, cx: &App) -> Vec<Item> {
        let vault = self.list.read(cx).delegate().vault();
        self.pinned
            .iter()
            .filter_map(|id| vault.items.iter().find(|i| &i.id == id && !i.deleted))
            .cloned()
            .collect()
    }

    fn pinned_row(&self, item: &Item, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.selected.as_ref().is_some_and(|s| s.id == item.id);
        let accent = appearance::accent(cx);
        let theme = cx.theme();
        let (fg, muted, hover) = (
            theme.foreground.opacity(0.85),
            theme.muted_foreground,
            theme.secondary_hover,
        );
        let order = self.sidebar_order.get();
        self.sidebar_order.set(order + 1);
        let id = item.id.clone();
        let row_id = SharedString::from(format!("pinned-{}", item.id));
        let row = h_flex()
            .id(row_id.clone())
            .test_support()
            .role(Role::Button)
            .aria_label(SharedString::from(item.name.clone()))
            .relative()
            .w_full()
            .gap_2p5()
            .px_2()
            .py_1()
            .rounded(theme.radius)
            .text_sm()
            .cursor_pointer()
            .when(selected, |el| el.text_color(accent).font_semibold())
            .when(!selected, |el| el.text_color(fg).hover(|el| el.bg(hover)))
            .active(|el| el.opacity(0.7))
            .when(selected, |el| {
                el.child(motion::accent_bar(
                    motion::key("fixado-barra", &row_id),
                    accent,
                ))
            })
            .child(
                Icon::new(crate::item_icons::icon_for(item, cx))
                    .small()
                    .when(!selected, |i| {
                        i.text_color(crate::item_icons::color_for(item, cx).unwrap_or(muted))
                    }),
            )
            .child(div().flex_1().min_w_0().truncate().child(item.name.clone()))
            .on_click(cx.listener(move |this, _, window, cx| this.open_pinned(&id, window, cx)))
            .with_animation(
                motion::key("fixado", format!("{row_id}:{selected}")),
                Animation::new(motion::NORMAL).with_easing(ease_out_quint()),
                move |el, t| {
                    if selected {
                        el.bg(accent.opacity(0.13 * t))
                    } else {
                        el
                    }
                },
            );
        motion::cascade(
            div().w_full().child(row),
            motion::key("fixado-entra", &row_id),
            order,
        )
    }

    /// Opens the new folder field in the sidebar.
    pub fn start_new_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.new_folder.is_some() {
            return;
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(t!("vault.folder_name")));
        let sub = cx.subscribe_in(&input, window, |this, _, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.save_new_folder(cx);
            }
        });
        input.update(cx, |i, cx| i.focus(window, cx));
        self.new_folder = Some((input, sub));
        cx.notify();
    }

    pub fn save_new_folder(&mut self, cx: &mut Context<Self>) {
        let Some((input, _)) = &self.new_folder else {
            return;
        };
        let name = input.read(cx).value().trim().to_owned();
        if name.is_empty() {
            crate::toast::error(t!("vault.folder_name_required"), cx);
            return;
        }
        self.run_write(WriteOp::CreateFolder(name), cx);
    }

    fn cancel_folder_edit(&mut self, cx: &mut Context<Self>) {
        self.new_folder = None;
        self.renaming_folder = None;
        self.confirm_delete_folder = false;
        cx.notify();
    }

    /// Rename the open folder: the list title becomes a field.
    pub fn start_rename_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Filter::Folder(Some(id)) = self.list.read(cx).delegate().filter().clone() else {
            return;
        };
        let current = self.filter_title(&Filter::Folder(Some(id.clone())), cx);
        let input = cx.new(|cx| InputState::new(window, cx).default_value(current));
        let sub = cx.subscribe_in(&input, window, |this, _, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.save_rename_folder(cx);
            }
        });
        input.update(cx, |i, cx| i.focus(window, cx));
        self.renaming_folder = Some((id, input, sub));
        self.confirm_delete_folder = false;
        cx.notify();
    }

    pub fn save_rename_folder(&mut self, cx: &mut Context<Self>) {
        let Some((id, input, _)) = &self.renaming_folder else {
            return;
        };
        let name = input.read(cx).value().trim().to_owned();
        if name.is_empty() {
            crate::toast::error(t!("vault.folder_name_required"), cx);
            return;
        }
        let id = id.clone();
        self.run_write(WriteOp::RenameFolder { id, name }, cx);
    }

    /// Delete the open folder (a second click confirms).
    pub fn delete_folder(&mut self, cx: &mut Context<Self>) {
        let Filter::Folder(Some(id)) = self.list.read(cx).delegate().filter().clone() else {
            return;
        };
        if !self.confirm_delete_folder {
            self.confirm_delete_folder = true;
            cx.notify();
            return;
        }
        self.confirm_delete_folder = false;
        self.run_write(WriteOp::DeleteFolder(id), cx);
    }

    pub fn toggle_icon_picker(&mut self, cx: &mut Context<Self>) {
        // The mouse down on the item's icon already closed it (click outside).
        let just_closed = self
            .icon_picker_closed_at
            .take()
            .is_some_and(|t| t.elapsed() < std::time::Duration::from_millis(500));
        if just_closed {
            return;
        }
        if self.icon_picker_open() {
            self.close_icon_picker(cx);
        } else {
            self.icon_picker = true;
            self.icon_picker_closing = false;
            self.icon_picker_anim += 1;
            cx.notify();
        }
    }

    /// Collapses the picker; it leaves the screen when the animation ends.
    fn close_icon_picker(&mut self, cx: &mut Context<Self>) {
        if !self.icon_picker_open() {
            return;
        }
        self.icon_picker_closing = true;
        self.icon_picker_anim += 1;
        let anim = self.icon_picker_anim;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(motion::NORMAL).await;
            let _ = this.update(cx, |v, cx| {
                if v.icon_picker_closing && v.icon_picker_anim == anim {
                    v.icon_picker = false;
                    v.icon_picker_closing = false;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    /// A click outside the picker closes it (but not one inside the color popup,
    /// which is drawn outside the picker).
    fn close_icon_picker_outside(&mut self, cx: &mut Context<Self>) {
        if self.icon_picker_open() && !self.icon_color.read(cx).is_open() {
            self.close_icon_picker(cx);
            self.icon_picker_closed_at = Some(std::time::Instant::now());
        }
    }

    pub fn icon_picker_open(&self) -> bool {
        self.icon_picker && !self.icon_picker_closing
    }

    /// Chooses the icon (`Some(name)`), the color (`color`) or goes back to the default
    /// (`reset`) for the open item. Applies right away on every screen.
    fn apply_icon(
        &mut self,
        choice: Option<crate::item_icons::CustomIcon>,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = &self.selected else { return };
        let all = crate::item_icons::set(&item.id, choice, cx);
        cx.emit(VaultEvent::IconsChanged(all));
        // The list draws the icons: redraw with the new choice.
        self.list.update(cx, |_, cx| cx.notify());
        cx.notify();
    }

    pub fn choose_icon(&mut self, name: &str, cx: &mut Context<Self>) {
        let Some(item) = &self.selected else { return };
        let color = crate::item_icons::choice(&item.id, cx).and_then(|c| c.color);
        let choice = crate::item_icons::CustomIcon {
            icon: name.to_owned(),
            color,
        };
        self.apply_icon(Some(choice), cx);
    }

    /// `None` = theme color.
    pub fn choose_icon_color(&mut self, color: Option<String>, cx: &mut Context<Self>) {
        let Some(item) = &self.selected else { return };
        let icon = crate::item_icons::choice(&item.id, cx)
            .map(|c| c.icon)
            .unwrap_or_default();
        let choice = (!icon.is_empty() || color.is_some())
            .then_some(crate::item_icons::CustomIcon { icon, color });
        self.apply_icon(choice, cx);
    }

    pub fn reset_icon(&mut self, cx: &mut Context<Self>) {
        self.apply_icon(None, cx);
    }

    /// Grid of icons and colors for the open item.
    fn render_icon_picker(&self, item: &Item, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let accent = appearance::accent(cx);
        let (border, popover, muted, hover) = (
            theme.border,
            theme.popover,
            theme.muted_foreground,
            theme.secondary_hover,
        );
        let current = crate::item_icons::choice(&item.id, cx);
        let current_icon = crate::item_icons::icon_for(item, cx);
        let current_color = current.as_ref().and_then(|c| c.color.clone());
        let tint = crate::item_icons::color_for(item, cx).unwrap_or(accent);

        // (stored name, element id, icon, label)
        let lucide: Vec<(String, String, crate::item_icons::ItemIcon, String)> =
            crate::item_icons::CHOICES
                .iter()
                .map(|(name, icon)| {
                    (
                        (*name).to_owned(),
                        format!("icon-choice-{name}"),
                        (*icon).into(),
                        (*name).to_owned(),
                    )
                })
                .collect();
        let brands: Vec<(String, String, crate::item_icons::ItemIcon, String)> =
            crate::item_icons::BRANDS
                .iter()
                .map(|b| {
                    (
                        crate::item_icons::brand_choice_name(b.slug),
                        format!("icon-choice-brand-{}", b.slug),
                        crate::item_icons::ItemIcon::Brand(b.slug),
                        b.name.to_owned(),
                    )
                })
                .collect();
        let mut grids = Vec::new();
        for entries in [lucide, brands] {
            let mut grid = h_flex().flex_wrap().gap_1p5();
            for (order, (name, id, icon, label)) in entries.into_iter().enumerate() {
                let selected = icon == current_icon;
                let key = id.clone();
                let button = div()
                    .id(SharedString::from(id))
                    .test_support()
                    .role(Role::Button)
                    .aria_label(SharedString::from(label.clone()))
                    .tooltip(move |window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(label.clone()).build(window, cx)
                    })
                    .size(px(36.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(9.))
                    .border_1()
                    .border_color(if selected { tint } else { border.opacity(0.) })
                    .bg(if selected {
                        tint.opacity(0.16)
                    } else {
                        popover
                    })
                    .text_color(if selected { tint } else { muted })
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover).text_color(tint))
                    .active(|s| s.opacity(0.7))
                    .child(Icon::new(icon).small())
                    .on_click(cx.listener(move |this, _, _, cx| this.choose_icon(&name, cx)));
                grid = grid.child(motion::cascade(
                    div().child(button),
                    motion::key("icone-opcao", &key),
                    order / 4,
                ));
            }
            grids.push(grid);
        }
        let brands_grid = grids.pop().unwrap();
        let grid = grids.pop().unwrap();

        let swatch = |id: SharedString, color: Hsla, selected: bool, label: SharedString| {
            div()
                .id(id)
                .test_support()
                .role(Role::Button)
                .aria_label(label)
                .size(px(24.))
                .rounded_full()
                .bg(color)
                .cursor_pointer()
                .border_2()
                .border_color(if selected { theme.foreground } else { color })
                .hover(|s| s.opacity(0.8))
        };
        let mut colors = h_flex()
            .gap_2()
            .items_center()
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .w(px(36.))
                    .child(t!("icons.color")),
            )
            .child(
                swatch(
                    "icon-color-theme".into(),
                    accent,
                    current_color.is_none(),
                    t!("icons.theme_color").into(),
                )
                .on_click(cx.listener(|this, _, _, cx| this.choose_icon_color(None, cx))),
            );
        let mut presets = vec![accent];
        for (id, label, _) in crate::item_icons::COLORS {
            let color = crate::item_icons::color_by_id(id, cx).unwrap_or(accent);
            presets.push(color);
            let selected = current_color.as_deref() == Some(*id);
            let id = *id;
            colors = colors.child(
                swatch(
                    format!("icon-color-{id}").into(),
                    color,
                    selected,
                    (*label).into(),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.choose_icon_color(Some(id.to_owned()), cx)
                })),
            );
        }

        // A free color already chosen shows as its own (selected) swatch.
        if let Some(custom) = current_color
            .as_deref()
            .filter(|c| c.starts_with('#'))
            .and_then(|c| crate::item_icons::color_by_id(c, cx))
        {
            colors = colors.child(swatch(
                "icon-color-custom".into(),
                custom,
                true,
                t!("icons.custom_color").into(),
            ));
        }
        colors = colors.child(
            ColorPicker::new(&self.icon_color)
                .featured_colors(presets)
                .icon(IconName::Plus)
                .small()
                .accessibility_label(t!("icons.custom_color")),
        );

        let measured = self.icon_picker_height.clone();
        let panel = v_flex()
            .id("icon-picker")
            .relative()
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.close_icon_picker_outside(cx)))
            .gap_3()
            .p_4()
            .rounded(cx.theme().radius_lg)
            .border_1()
            .border_color(border)
            .bg(popover)
            .child(
                h_flex()
                    .justify_between()
                    .child(
                        v_flex()
                            .child(div().text_sm().font_semibold().child(t!("icons.title")))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(muted)
                                    .child(t!("icons.local_only")),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new("icon-reset")
                                    .ghost()
                                    .small()
                                    .label(t!("icons.reset"))
                                    .disabled(current.is_none())
                                    .on_click(cx.listener(|this, _, _, cx| this.reset_icon(cx))),
                            )
                            .child(
                                Button::new("icon-picker-close")
                                    .ghost()
                                    .small()
                                    .icon(IconName::Close)
                                    .tooltip(t!("common.close"))
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.toggle_icon_picker(cx)),
                                    ),
                            ),
                    ),
            )
            .child(colors)
            .child(grid)
            .child(
                div()
                    .text_xs()
                    .font_semibold()
                    .text_color(muted)
                    .child(t!("icons.brands")),
            )
            .child(brands_grid)
            // Measures the panel, so the next animation knows how tall it is.
            .child(
                canvas(
                    move |bounds, _, _| measured.set(f32::from(bounds.size.height)),
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            );

        // Expands when opening and collapses when closing.
        let closing = self.icon_picker_closing;
        let full = match self.icon_picker_height.get() {
            h if h > 0. => h,
            _ => 520.,
        };
        div().child(panel).with_animation(
            motion::key(
                "icone-painel",
                format!("{}:{}", item.id, self.icon_picker_anim),
            ),
            Animation::new(motion::NORMAL).with_easing(ease_out_quint()),
            move |el, t| {
                let shown = if closing { 1. - t } else { t };
                if !closing && t >= 1. {
                    // Open: no clipping, so the color popup and new rows fit.
                    return el;
                }
                el.overflow_hidden().max_h(px(full * shown)).opacity(shown)
            },
        )
    }

    /// Moves an item (dragged from the list) to the `folder` folder (`None` = no folder).
    pub fn move_item_to_folder(
        &mut self,
        item_id: &str,
        folder: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if self.syncing {
            crate::toast::info(t!("vault.wait_sync"), cx);
            return;
        }
        if self.editor.is_some() {
            crate::toast::info(t!("vault.save_before_move"), cx);
            return;
        }
        let vault = self.list.read(cx).delegate().vault();
        let Some(item) = vault.items.iter().find(|i| i.id == item_id) else {
            return;
        };
        if item.deleted || item.folder_id == folder {
            return;
        }
        let message = match &folder {
            Some(id) => {
                let name = vault
                    .folders
                    .iter()
                    .find(|f| &f.id == id)
                    .map(|f| f.name.rsplit('/').next().unwrap_or(&f.name).to_owned())
                    .unwrap_or_default();
                t!("vault.moved_to", item = item.name, folder = name)
            }
            None => t!("vault.moved_to_no_folder", item = item.name),
        };
        let op = WriteOp::MoveItem {
            item_id: item_id.to_owned(),
            folder_id: folder,
        };
        self.run_write_with(op, Some(message), cx);
    }

    /// Puts the `folder_id` folder (with its subfolders) inside `target` (`None` = top level).
    pub fn move_folder_into(
        &mut self,
        folder_id: &str,
        target: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if self.syncing {
            crate::toast::info(t!("vault.wait_sync"), cx);
            return;
        }
        let folders = &self.list.read(cx).delegate().vault().folders;
        match crate::dnd::move_folder(folders, folder_id, target.as_deref()) {
            Ok(plan) if plan.is_empty() => {}
            Ok(plan) => {
                let message = match plan[0].1.rsplit_once('/') {
                    Some((parent, _)) => t!("vault.folder_moved_to", parent = parent),
                    None => t!("vault.folder_moved_top").to_owned(),
                };
                self.renaming_folder = None;
                self.confirm_delete_folder = false;
                self.run_write_with(WriteOp::RenameFolders(plan), Some(message), cx);
            }
            Err(e) => crate::toast::error(e.message(), cx),
        }
    }

    /// New item (`Ctrl+N`). Starts in the current filter's folder, if any.
    pub fn new_item(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Doesn't discard a form that is already open.
        if self.syncing || self.editor.is_some() {
            return;
        }
        let folder = match self.list.read(cx).delegate().filter() {
            Filter::Folder(Some(id)) => Some(id.clone()),
            _ => None,
        };
        let kind = match self.list.read(cx).delegate().filter() {
            Filter::Kind(KindFilter::Note) => NewKind::Note,
            Filter::Kind(KindFilter::Card) => NewKind::Card,
            Filter::Kind(KindFilter::Identity) => NewKind::Identity,
            Filter::Kind(KindFilter::SshKey) => NewKind::SshKey,
            _ => NewKind::Login,
        };
        // Filter on an organization/collection: the new item is born there.
        let owner = match self.list.read(cx).delegate().filter() {
            Filter::Organization(id) => Some((id.clone(), None)),
            Filter::Collection(id) => self
                .list
                .read(cx)
                .delegate()
                .vault()
                .collections
                .iter()
                .find(|c| &c.id == id && !c.read_only)
                .map(|c| (c.organization_id.clone(), Some(c.id.clone()))),
            _ => None,
        };
        let places = self.places(cx);
        let options = self.password_options;
        let editor =
            cx.new(|cx| ItemEditor::create(kind, folder, owner, places, options, window, cx));
        self.open_editor(editor, window, cx);
    }

    /// Edits the open item (`Ctrl+E`).
    pub fn edit_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.selected.clone() else {
            return;
        };
        if self.syncing || self.editor.is_some() || item.deleted || !self.can_edit(&item) {
            return;
        }
        if !self.guard(Guarded::Edit, window, cx) {
            return;
        }
        let places = self.places(cx);
        let options = self.password_options;
        let editor = cx.new(|cx| ItemEditor::edit(&item, places, options, window, cx));
        self.open_editor(editor, window, cx);
    }

    fn trash_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(item) = &self.selected {
            self.run_write(WriteOp::Trash(item.id.clone()), cx);
        }
    }

    fn restore_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(item) = &self.selected {
            self.run_write(WriteOp::Restore(item.id.clone()), cx);
        }
    }

    fn delete_selected(&mut self, cx: &mut Context<Self>) {
        if !self.confirm_delete {
            self.confirm_delete = true;
            cx.notify();
            return;
        }
        self.confirm_delete = false;
        if let Some(item) = &self.selected {
            self.run_write(WriteOp::Delete(item.id.clone()), cx);
        }
    }

    /// Picks a file in Windows and attaches it to the open item.
    fn pick_attachment(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some(t!("vault.attach_prompt").into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await {
                this.update(cx, |this, cx| this.attach_paths(&paths, cx))
                    .ok();
            }
        })
        .detach();
    }

    /// Attaches the file at `path` to the open item.
    pub fn attach_file_from_path(&mut self, path: &std::path::Path, cx: &mut Context<Self>) {
        self.attach_paths(&[path.to_path_buf()], cx);
    }

    /// Attaches several files to the open item (chosen or dragged).
    pub fn attach_paths(&mut self, paths: &[std::path::PathBuf], cx: &mut Context<Self>) {
        let Some(item) = &self.selected else { return };
        let item_id = item.id.clone();
        let mut files = Vec::new();
        for path in paths.iter().filter(|p| !p.is_dir()) {
            match flow::NewFile::read(path) {
                Ok(file) => files.push(file),
                Err(e) => crate::toast::error(e, cx),
            }
        }
        if !files.is_empty() {
            self.run_write(WriteOp::AddAttachments { item_id, files }, cx);
        }
    }

    fn remove_attachment(&mut self, attachment_id: String, cx: &mut Context<Self>) {
        if self.confirm_remove_attachment.as_deref() != Some(attachment_id.as_str()) {
            self.confirm_remove_attachment = Some(attachment_id);
            cx.notify();
            return;
        }
        self.confirm_remove_attachment = None;
        if let Some(item) = &self.selected {
            self.run_write(
                WriteOp::RemoveAttachment {
                    item_id: item.id.clone(),
                    attachment_id,
                },
                cx,
            );
        }
    }

    /// Downloads and decrypts; then `then` decides what to do with the contents.
    fn fetch_attachment(
        &mut self,
        attachment_id: String,
        then: impl FnOnce(&mut Self, String, zeroize::Zeroizing<Vec<u8>>, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = &self.selected else { return };
        let Some(att) = item
            .attachments
            .iter()
            .find(|a| a.id == attachment_id)
            .cloned()
        else {
            return;
        };
        if self.downloading.is_some() {
            return;
        }
        self.downloading = Some(attachment_id.clone());
        cx.notify();
        let work = flow::download_attachment(
            self.session.clone(),
            self.keys.clone(),
            item.id.clone(),
            attachment_id,
        );
        cx.spawn(async move |this, cx| {
            let result = work.await;
            this.update(cx, |this, cx| {
                this.downloading = None;
                match result {
                    Ok(bytes) => then(this, att.file_name, bytes, cx),
                    Err(flow::SyncError::Expired) => cx.emit(VaultEvent::SessionExpired),
                    Err(flow::SyncError::Other(e)) => crate::toast::error(e, cx),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Opens the attachment with the default program (temporary copy, deleted on lock).
    pub fn open_attachment(&mut self, attachment_id: String, cx: &mut Context<Self>) {
        self.fetch_attachment(
            attachment_id,
            |this, name, bytes, cx| match crate::attachments::write_temp(&name, &bytes) {
                Ok(path) => {
                    if crate::attachments::system_open_enabled() {
                        cx.open_with_system(&path);
                    }
                    this.temp_files.push(path);
                    crate::toast::success(t!("vault.file_opened", name = name), cx);
                }
                Err(e) => crate::toast::error(t!("vault.open_failed", e = e), cx),
            },
            cx,
        );
    }

    /// Saves the decrypted attachment wherever the person chooses.
    fn save_attachment(&mut self, attachment_id: String, cx: &mut Context<Self>) {
        self.fetch_attachment(
            attachment_id,
            |_, name, bytes, cx| {
                let dir = crate::attachments::downloads_dir();
                let target = cx.prompt_for_new_path(&dir, Some(&name));
                cx.spawn(async move |this, cx| {
                    let path = match target.await {
                        Ok(Ok(Some(path))) => path,
                        _ => return,
                    };
                    let result = std::fs::write(&path, &bytes[..]);
                    this.update(cx, |_, cx| match result {
                        Ok(()) => {
                            crate::toast::success(t!("vault.saved_to", path = path.display()), cx)
                        }
                        Err(e) => crate::toast::error(t!("vault.save_failed", e = e), cx),
                    })
                    .ok();
                })
                .detach();
            },
            cx,
        );
    }

    fn render_attachments(&self, item: &Item, cx: &mut Context<Self>) -> Option<AnyElement> {
        let can_add = !item.deleted && self.can_edit(item);
        if item.attachments.is_empty() && !can_add {
            return None;
        }
        let busy = self.syncing || self.downloading.is_some();
        let count = item.attachments.len();
        let rows = item.attachments.iter().enumerate().map(|(ix, a)| {
            let (open_id, save_id, remove_id) = (a.id.clone(), a.id.clone(), a.id.clone());
            let confirming = self.confirm_remove_attachment.as_deref() == Some(a.id.as_str());
            h_flex()
                .gap_3()
                .px_4()
                .py_2p5()
                .when(ix + 1 < count, |el| {
                    el.border_b_1().border_color(cx.theme().border)
                })
                .child(
                    Icon::new(IconName::File)
                        .small()
                        .text_color(cx.theme().muted_foreground),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .id(SharedString::from(format!("attach-name-{}", a.id)))
                                .test_support()
                                .aria_label(SharedString::from(a.file_name.clone()))
                                .text_sm()
                                .truncate()
                                .child(a.file_name.clone()),
                        )
                        .children(a.size.map(|s| {
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(scout_vault_core::attachment::human_size(s))
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("attach-open-{}", a.id)))
                        .ghost()
                        .small()
                        .icon(IconName::ExternalLink)
                        .tooltip(t!("common.open"))
                        .disabled(busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_attachment(open_id.clone(), cx)
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("attach-save-{}", a.id)))
                        .ghost()
                        .small()
                        .icon(IconName::Download)
                        .tooltip(t!("vault.save_as"))
                        .disabled(busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.save_attachment(save_id.clone(), cx)
                        })),
                )
                .when(can_add, |row| {
                    row.child(
                        Button::new(SharedString::from(format!("attach-remove-{}", a.id)))
                            .when(confirming, |b| b.danger().label(t!("vault.remove_confirm")))
                            .when(!confirming, |b| {
                                b.ghost()
                                    .icon(IconName::Trash)
                                    .tooltip(t!("vault.remove_attachment"))
                            })
                            .small()
                            .disabled(busy)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.remove_attachment(remove_id.clone(), cx)
                            })),
                    )
                })
        });
        Some(
            v_flex()
                .gap_2()
                .child(ui::section_label(t!("vault.attachments"), cx).px_0())
                .when(count > 0, |v| {
                    v.child(
                        v_flex()
                            .rounded(cx.theme().radius_lg)
                            .border_1()
                            .border_color(cx.theme().border)
                            .bg(cx.theme().popover)
                            .children(rows),
                    )
                })
                // Always visible on editable items: click or drag files.
                .when(can_add, |v| {
                    v.child(ui::drop_zone(
                        "attach-add",
                        busy,
                        cx.listener(|this, _, _, cx| {
                            if !this.syncing {
                                this.pick_attachment(cx)
                            }
                        }),
                        cx.listener(|this, paths: &ExternalPaths, _, cx| {
                            this.attach_paths(paths.paths(), cx)
                        }),
                        cx,
                    ))
                })
                .into_any_element(),
        )
    }

    /// Adds the imported items to the vault (encrypted here, sent together).
    pub fn import_items(
        &mut self,
        items: Vec<scout_vault_core::import::Imported>,
        existing: Vec<scout_vault_core::vault::Folder>,
        cx: &mut Context<Self>,
    ) {
        let message = t!("vault.imported", n = items.len());
        self.run_write_with(WriteOp::Import { items, existing }, Some(message), cx);
    }

    /// Is a change (or sync) being sent right now?
    pub fn is_syncing(&self) -> bool {
        self.syncing
    }

    /// Sends a change (or just syncs) and updates the screen with the result.
    fn run_write(&mut self, op: WriteOp, cx: &mut Context<Self>) {
        self.run_write_with(op, None, cx);
    }

    /// Like [`Self::run_write`], with the success message chosen by the caller.
    fn run_write_with(&mut self, op: WriteOp, message: Option<String>, cx: &mut Context<Self>) {
        if self.syncing {
            return;
        }
        let attach_only = message.is_none() && matches!(op, WriteOp::AddAttachments { .. });
        let done_message: Option<String> = message.or_else(|| {
            match &op {
                WriteOp::Nothing => None,
                WriteOp::Save(item, _) if item.id.is_empty() => Some(t!("vault.item_created")),
                WriteOp::Save(..) => Some(t!("vault.changes_saved")),
                WriteOp::Trash(_) => Some(t!("vault.item_trashed")),
                WriteOp::Restore(_) => Some(t!("vault.item_restored")),
                WriteOp::Delete(_) => Some(t!("vault.item_deleted")),
                WriteOp::AddAttachments { files, .. } if files.len() > 1 => {
                    Some(t!("vault.files_attached"))
                }
                WriteOp::AddAttachments { .. } => Some(t!("vault.file_attached")),
                WriteOp::RemoveAttachment { .. } => Some(t!("vault.attachment_removed")),
                WriteOp::CreateFolder(_) => Some(t!("vault.folder_created")),
                WriteOp::RenameFolder { .. } => Some(t!("vault.folder_renamed")),
                WriteOp::DeleteFolder(_) => Some(t!("vault.folder_deleted")),
                WriteOp::MoveItem { .. } => Some(t!("vault.item_moved")),
                WriteOp::RenameFolders(_) => Some(t!("vault.folder_moved")),
                // The message comes from `import_items`.
                WriteOp::Import { .. } => None,
                WriteOp::SetFavorite { favorite: true, .. } => Some(t!("vault.favorited")),
                WriteOp::SetFavorite { .. } => Some(t!("vault.unfavorited")),
            }
            .map(String::from)
        });
        let closes_detail = matches!(op, WriteOp::Delete(_));
        // Folders: after saving, opens the folder; after deleting, goes back to all.
        let folder_saved = matches!(op, WriteOp::CreateFolder(_) | WriteOp::RenameFolder { .. });
        let folder_deleted = matches!(op, WriteOp::DeleteFolder(_));
        let folder_moved = matches!(op, WriteOp::RenameFolders(_));
        self.syncing = true;
        if let Some((editor, _)) = &self.editor {
            editor.update(cx, |e, cx| e.set_saving(true, None, cx));
        }
        cx.notify();
        let work = flow::write(self.session.clone(), self.keys.clone(), op);
        cx.spawn(async move |this, cx| {
            let result = work.await;
            this.update(cx, |this, cx| {
                this.syncing = false;
                this.sync_count += 1;
                let n = this.sync_count;
                this.sync_feedback = Some((result.is_ok(), n));
                cx.spawn(async move |this, cx| {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(1600))
                        .await;
                    this.update(cx, |this, cx| {
                        if this.sync_feedback.is_some_and(|(_, m)| m == n) {
                            this.sync_feedback = None;
                            cx.notify();
                        }
                    })
                    .ok();
                })
                .detach();
                match result {
                    Ok(flow::Written {
                        session,
                        vault,
                        saved_id,
                        warnings,
                        revision,
                    }) => {
                        if revision.is_some() {
                            this.last_revision = revision;
                        }
                        for w in &warnings {
                            crate::toast::error(t!("vault.attachment_not_sent", w = w), cx);
                        }
                        if folder_saved || folder_deleted {
                            this.new_folder = None;
                            this.renaming_folder = None;
                            this.confirm_delete_folder = false;
                            this.pending_filter = if folder_deleted {
                                Some(Filter::All)
                            } else {
                                saved_id.clone().map(|id| Filter::Folder(Some(id)))
                            };
                        }
                        let saved_id = if folder_saved || folder_deleted || folder_moved {
                            None
                        } else {
                            saved_id
                        };
                        if done_message.is_some() && !(folder_saved || folder_deleted) {
                            this.editor = None;
                        }
                        let keep = saved_id.or_else(|| {
                            (!closes_detail)
                                .then(|| this.selected.as_ref().map(|i| i.id.clone()))
                                .flatten()
                        });
                        let errors = vault.errors.len();
                        cx.emit(VaultEvent::Synced(session.clone()));
                        this.session = session;
                        let selected_id = keep;
                        this.list.update(cx, |list, cx| {
                            list.delegate_mut().set_vault(vault);
                            cx.notify();
                        });
                        // Keeps the item open, now with the new data.
                        this.selected = selected_id.and_then(|id| {
                            this.list
                                .read(cx)
                                .delegate()
                                .vault()
                                .items
                                .iter()
                                .find(|i| i.id == id)
                                .cloned()
                        });
                        // Syncing on its own already shows ✓ on the icon; the rest becomes a toast.
                        if let Some(m) =
                            done_message.filter(|_| !(attach_only && !warnings.is_empty()))
                        {
                            crate::toast::success(m.trim_end_matches('.').to_owned(), cx);
                        }
                        if errors > 0 {
                            crate::toast::error(t!("vault.decrypt_errors", n = errors), cx);
                        }
                    }
                    Err(flow::SyncError::Expired) => cx.emit(VaultEvent::SessionExpired),
                    Err(flow::SyncError::Other(e)) => {
                        // Undo what was shown ahead of the server (e.g. the favorite star).
                        if let Some(id) = this.selected.as_ref().map(|i| i.id.clone()) {
                            if let Some(item) = this
                                .list
                                .read(cx)
                                .delegate()
                                .vault()
                                .items
                                .iter()
                                .find(|i| i.id == id)
                            {
                                this.selected = Some(item.clone());
                            }
                        }
                        // With the editor open, the error shows up in it and nothing is lost.
                        if let Some((editor, _)) = &this.editor {
                            let msg = e.clone();
                            editor.update(cx, |ed, cx| ed.set_saving(false, Some(msg), cx));
                        } else {
                            crate::toast::error(e, cx);
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn filter_title(&self, filter: &Filter, cx: &App) -> String {
        let vault = self.list.read(cx).delegate().vault();
        match filter {
            Filter::All => t!("vault.all_items").into(),
            Filter::Favorites => t!("vault.favorites").into(),
            Filter::Kind(k) => k.label().into(),
            Filter::Organization(id) => vault
                .organizations
                .iter()
                .find(|o| &o.id == id)
                .map(|o| o.name.clone())
                .unwrap_or_default(),
            Filter::Folder(Some(id)) => vault
                .folders
                .iter()
                .find(|f| &f.id == id)
                .map(|f| f.name.clone())
                .unwrap_or_default(),
            Filter::Folder(None) => t!("vault.no_folder").into(),
            Filter::Collection(id) => vault
                .collections
                .iter()
                .find(|c| &c.id == id)
                .map(|c| c.name.clone())
                .unwrap_or_default(),
            Filter::Trash => t!("vault.trash").into(),
        }
    }

    /// Highlight of a place where what is being dragged can be dropped.
    fn drop_style<T: 'static>(
        cx: &App,
    ) -> impl Fn(StyleRefinement, &T, &mut Window, &mut App) -> StyleRefinement + 'static {
        let accent = appearance::accent(cx);
        move |style, _, _, _| style.bg(accent.opacity(0.18))
    }

    /// Folder in the sidebar: indented by hierarchy, draggable and a target for
    /// items and other folders.
    fn folder_row(&self, row: crate::dnd::FolderRow, cx: &mut Context<Self>) -> impl IntoElement {
        let dragged = crate::dnd::DraggedFolder {
            id: row.id.clone(),
            name: row.label.clone(),
        };
        let (item_target, folder_target) = (row.id.clone(), row.id.clone());
        let id = row.id.clone();
        div()
            .id(SharedString::from(format!("drop-folder-{id}")))
            .w_full()
            .pl(px(14. * row.depth as f32))
            .rounded(cx.theme().radius)
            .on_drag(dragged, |f, _, _, cx| {
                crate::dnd::DragPreview::folder(f, cx)
            })
            .drag_over::<crate::dnd::DraggedItem>(Self::drop_style(cx))
            .drag_over::<crate::dnd::DraggedFolder>(Self::drop_style(cx))
            .on_drop(
                cx.listener(move |this, item: &crate::dnd::DraggedItem, _, cx| {
                    this.move_item_to_folder(&item.id, Some(item_target.clone()), cx)
                }),
            )
            .on_drop(
                cx.listener(move |this, f: &crate::dnd::DraggedFolder, _, cx| {
                    this.move_folder_into(&f.id, Some(folder_target.clone()), cx)
                }),
            )
            .child(self.filter_row(
                format!("filter-folder-{id}").into(),
                Filter::Folder(Some(row.id)),
                IconName::Folder,
                row.label,
                cx,
            ))
    }

    fn filter_row(
        &self,
        id: SharedString,
        filter: Filter,
        icon: IconName,
        label: String,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let list = self.list.read(cx).delegate();
        let count = list.count(&filter);
        let selected = *list.filter() == filter;
        let accent = appearance::accent(cx);
        let order = self.sidebar_order.get();
        self.sidebar_order.set(order + 1);
        let theme = cx.theme();
        let (fg, muted, hover) = (
            theme.foreground.opacity(0.85),
            theme.muted_foreground,
            theme.secondary_hover,
        );
        let id_text = id.clone();
        let custom = self.sidebar_icons.get(id.as_ref()).cloned();
        let icon = custom
            .as_ref()
            .and_then(|c| {
                crate::item_icons::CHOICES
                    .iter()
                    .find(|(n, _)| *n == c.icon)
            })
            .map(|(_, i)| *i)
            .unwrap_or(icon);
        let icon_color = custom
            .as_ref()
            .and_then(|c| c.color.as_deref())
            .and_then(|c| crate::item_icons::color_by_id(c, cx));
        let menu_open = self.sidebar_icon_menu.as_deref() == Some(id.as_ref());
        let menu_entry = id.to_string();
        let row = h_flex()
            .id(id)
            .test_support()
            .role(Role::Button)
            .aria_label(SharedString::from(label.clone()))
            .relative()
            .w_full()
            .gap_2p5()
            .px_2()
            .py_1()
            .rounded(theme.radius)
            .text_sm()
            .cursor_pointer()
            .when(selected, |el| el.text_color(accent).font_semibold())
            .when(!selected, |el| el.text_color(fg).hover(|el| el.bg(hover)))
            .active(|el| el.opacity(0.7))
            .when(selected, |el| {
                el.child(motion::accent_bar(
                    motion::key("filtro-barra", &id_text),
                    accent,
                ))
            })
            .child({
                let icon = div().flex().child(
                    Icon::new(icon)
                        .small()
                        .when(!selected, |i| i.text_color(muted))
                        .when_some(icon_color, |i, c| i.text_color(c)),
                );
                if selected {
                    motion::nudge(icon, motion::key("filtro-icone", &id_text), 2.)
                        .into_any_element()
                } else {
                    icon.into_any_element()
                }
            })
            .child(div().flex_1().min_w_0().truncate().child(label))
            .child({
                let badge = div()
                    .text_xs()
                    .when(!selected, |d| d.text_color(muted))
                    .child(count.to_string());
                if selected {
                    motion::pop(badge, motion::key("filtro-conta", &id_text)).into_any_element()
                } else {
                    badge.into_any_element()
                }
            })
            .on_click(
                cx.listener(move |this, _, window, cx| this.set_filter(filter.clone(), window, cx)),
            )
            // Right click: choose the entry's icon.
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _, _, cx| {
                    this.toggle_sidebar_icon_menu(&menu_entry, cx);
                    cx.stop_propagation();
                }),
            )
            // The chosen filter's highlight lights up softly.
            .with_animation(
                motion::key("filtro", format!("{id_text}:{selected}")),
                Animation::new(motion::NORMAL).with_easing(ease_out_quint()),
                move |el, t| {
                    if selected {
                        el.bg(accent.opacity(0.13 * t))
                    } else {
                        el
                    }
                },
            );
        // When the vault opens, the rows cascade in (once per screen).
        let menu = menu_open.then(|| self.render_sidebar_icon_menu(&id_text, cx));
        motion::cascade(
            div().w_full().child(row).children(menu),
            motion::key("filtro-entra", &id_text),
            order,
        )
    }

    /// Sync button: spins while working; when done turns into ✓ (or ✗)
    /// for an instant, with a transition, and goes back to the arrows.
    fn render_sync_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        use std::f32::consts::TAU;
        let accent = appearance::accent(cx);
        let danger = cx.theme().danger;
        let muted = cx.theme().foreground.opacity(0.8);
        let n = self.sync_count;
        let (icon, label): (AnyElement, &str) = if self.syncing {
            (
                Icon::new(IconName::RefreshCw)
                    .small()
                    .text_color(accent)
                    .with_animation(
                        "sync-girando",
                        Animation::new(std::time::Duration::from_millis(900)).repeat(),
                        |icon, t| icon.rotate(radians(t * TAU)),
                    )
                    .into_any_element(),
                t!("vault.syncing"),
            )
        } else if let Some((ok, m)) = self.sync_feedback {
            let (name, color, label) = if ok {
                (IconName::Check, accent, t!("vault.synced"))
            } else {
                (IconName::Close, danger, t!("vault.sync_failed"))
            };
            (
                // Enters spinning a quarter turn while fading in.
                Icon::new(name)
                    .small()
                    .with_animation(
                        motion::key("sync-fim", m),
                        Animation::new(std::time::Duration::from_millis(340))
                            .with_easing(ease_out_quint()),
                        move |icon, t| {
                            icon.text_color(color.opacity(t))
                                .rotate(radians(-TAU / 4. * (1. - t)))
                        },
                    )
                    .into_any_element(),
                label,
            )
        } else {
            (
                // Back to the arrows with a soft half turn (doesn't animate the first time).
                Icon::new(IconName::RefreshCw)
                    .small()
                    .text_color(muted)
                    .with_animation(
                        motion::key("sync-volta", n),
                        Animation::new(if n == 0 {
                            std::time::Duration::from_millis(1)
                        } else {
                            motion::NORMAL
                        })
                        .with_easing(ease_out_quint()),
                        move |icon, t| {
                            if n == 0 {
                                icon
                            } else {
                                icon.rotate(radians(-TAU / 2. * (1. - t)))
                            }
                        },
                    )
                    .into_any_element(),
                t!("vault.sync"),
            )
        };
        let busy = self.syncing;
        let hover = cx.theme().secondary_hover;
        div()
            .id("vault-sync")
            .test_support()
            .role(Role::Button)
            .aria_label(SharedString::from(label))
            .size(px(24.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(cx.theme().radius)
            .when(!busy, |el| {
                el.cursor_pointer()
                    .hover(|el| el.bg(hover))
                    .active(|el| el.opacity(0.7))
            })
            .tooltip(move |window, cx| {
                gpui_kit::component::tooltip::Tooltip::new(label).build(window, cx)
            })
            .child(icon)
            .on_click(cx.listener(|this, _, _, cx| this.sync(cx)))
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        self.sidebar_order.set(0);
        let vault = self.list.read(cx).delegate().vault();
        let orgs: Vec<_> = vault
            .organizations
            .iter()
            .map(|o| (o.id.clone(), o.name.clone()))
            .collect();
        let collections: Vec<_> = vault
            .collections
            .iter()
            .map(|c| (c.id.clone(), c.organization_id.clone(), c.name.clone()))
            .collect();
        let folders = crate::dnd::folder_rows(&vault.folders);

        let mut nav = v_flex()
            .id("vault-filters")
            .flex_1()
            .min_h_0()
            .px_2()
            .gap_px()
            .overflow_y_scroll()
            .child(self.filter_row(
                "filter-all".into(),
                Filter::All,
                IconName::LayoutGrid,
                t!("vault.all_items").into(),
                cx,
            ))
            .child(self.filter_row(
                "filter-favorites".into(),
                Filter::Favorites,
                IconName::Star,
                t!("vault.favorites").into(),
                cx,
            ));
        // Quick access: items pinned by the person, right at the top.
        let pinned = self.pinned_items(cx);
        if !pinned.is_empty() {
            nav = nav.child(self.section_header("quick", t!("vault.quick_access"), cx));
            if !self.is_collapsed("quick") {
                for item in &pinned {
                    nav = nav.child(self.pinned_row(item, cx));
                }
            }
        }
        nav = nav.child(self.section_header("types", t!("vault.types"), cx));
        for k in KindFilter::ALL
            .into_iter()
            .filter(|_| !self.is_collapsed("types"))
        {
            let icon = match k {
                KindFilter::Login => IconName::Globe,
                KindFilter::Card => IconName::CreditCard,
                KindFilter::Identity => IconName::IdCard,
                KindFilter::Note => IconName::StickyNote,
                KindFilter::SshKey => IconName::KeyRound,
            };
            nav = nav.child(self.filter_row(
                format!("filter-{}", k.id()).into(),
                Filter::Kind(k),
                icon,
                k.label().into(),
                cx,
            ));
        }
        if !orgs.is_empty() {
            nav = nav.child(self.section_header("orgs", t!("vault.organizations"), cx));
            for (id, name) in orgs.into_iter().filter(|_| !self.is_collapsed("orgs")) {
                nav = nav.child(self.filter_row(
                    format!("filter-org-{id}").into(),
                    Filter::Organization(id.clone()),
                    IconName::Building2,
                    name,
                    cx,
                ));
                // The organization's collections, indented below it.
                for (cid, _, cname) in collections.iter().filter(|(_, org, _)| *org == id) {
                    nav = nav.child(div().pl_4().child(self.filter_row(
                        format!("filter-col-{cid}").into(),
                        Filter::Collection(cid.clone()),
                        IconName::FolderOpen,
                        cname.clone(),
                        cx,
                    )));
                }
            }
        }
        nav = nav.child(
            h_flex()
                .id("folders-header")
                .justify_between()
                .pr_1()
                .rounded(cx.theme().radius)
                // Dropping a folder on the "Folders" heading moves it to the top level.
                .drag_over::<crate::dnd::DraggedFolder>(Self::drop_style(cx))
                .on_drop(cx.listener(|this, f: &crate::dnd::DraggedFolder, _, cx| {
                    this.move_folder_into(&f.id, None, cx)
                }))
                .child(self.section_header("folders", t!("vault.folders"), cx))
                .child(
                    Button::new("folder-new")
                        .ghost()
                        .xsmall()
                        .icon(IconName::Plus)
                        .tooltip(t!("vault.new_folder"))
                        .disabled(self.syncing)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.start_new_folder(window, cx)),
                        ),
                )
                .test_support(),
        );
        if let Some((input, _)) = &self.new_folder {
            nav = nav.child(motion::rise(
                h_flex()
                    .gap_1()
                    .px_1()
                    .pb_1()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(input).id("folder-new-name").small()),
                    )
                    .child(
                        Button::new("folder-new-save")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Check)
                            .tooltip(t!("vault.create_enter"))
                            .disabled(self.syncing)
                            .on_click(cx.listener(|this, _, _, cx| this.save_new_folder(cx))),
                    )
                    .child(
                        Button::new("folder-new-cancel")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Close)
                            .tooltip(t!("common.cancel"))
                            .on_click(cx.listener(|this, _, _, cx| this.cancel_folder_edit(cx))),
                    ),
                motion::key("pasta-nova", "campo"),
                4.,
            ));
        }
        let folders_open = !self.is_collapsed("folders");
        for row in folders.into_iter().filter(|_| folders_open) {
            nav = nav.child(self.folder_row(row, cx));
        }
        nav = nav
            .when(folders_open, |nav| {
                nav.child(
                    div()
                        .id("drop-no-folder")
                        .w_full()
                        .rounded(cx.theme().radius)
                        .drag_over::<crate::dnd::DraggedItem>(Self::drop_style(cx))
                        .on_drop(cx.listener(|this, item: &crate::dnd::DraggedItem, _, cx| {
                            this.move_item_to_folder(&item.id, None, cx)
                        }))
                        .child(self.filter_row(
                            "filter-no-folder".into(),
                            Filter::Folder(None),
                            IconName::FolderOpen,
                            t!("vault.no_folder").into(),
                            cx,
                        )),
                )
            })
            .child(div().h_3())
            .child(self.filter_row(
                "filter-trash".into(),
                Filter::Trash,
                IconName::Trash,
                t!("vault.trash").into(),
                cx,
            ))
            .child(div().h_3());

        // The account: avatar, name and email — click for the profile screen.
        let footer = {
            let profile = &self.session.sync.profile;
            let email = SharedString::from(self.session.email.clone());
            let name = profile.name.clone().filter(|n| !n.trim().is_empty());
            let initials = crate::profile_view::initials_of(name.as_deref(), &self.session.email);
            let color = crate::profile_view::avatar_color(profile.avatar_color.as_deref(), cx);
            let title: SharedString = name
                .clone()
                .unwrap_or_else(|| {
                    self.session
                        .email
                        .split('@')
                        .next()
                        .unwrap_or_default()
                        .to_owned()
                })
                .into();
            let (muted, hover) = (cx.theme().muted_foreground, cx.theme().secondary_hover);
            div()
                .px_2()
                .py_2()
                .border_t_1()
                .border_color(cx.theme().border)
                .child(
                    h_flex()
                        .id("vault-email")
                        .test_support()
                        .role(Role::Button)
                        .aria_label(email.clone())
                        .w_full()
                        .gap_2p5()
                        .items_center()
                        .px_2()
                        .py_2()
                        .rounded(cx.theme().radius_lg)
                        .cursor_pointer()
                        .hover(move |s| s.bg(hover))
                        .active(|s| s.opacity(0.8))
                        .tooltip(move |window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(t!(
                                "vault.profile_tooltip",
                                email = email.clone()
                            ))
                            .build(window, cx)
                        })
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(VaultEvent::OpenProfile)))
                        .child(crate::profile_view::avatar(
                            initials,
                            color,
                            px(32.),
                            crate::profile_photo::get(&self.session.email, cx),
                        ))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .child(div().text_sm().font_semibold().truncate().child(title))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(muted)
                                        .truncate()
                                        .child(self.session.email.clone()),
                                ),
                        )
                        .child(
                            div()
                                .text_color(muted)
                                .child(Icon::new(IconName::ChevronRight).small()),
                        ),
                )
        };

        v_flex()
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .flex_shrink_0()
            .border_r_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().sidebar)
            .child(div().h_2())
            .child(nav)
            .child(footer)
    }

    /// The vault's actions (sync, health, generator, settings, lock), drawn in the
    /// window's title bar by the app.
    pub fn render_title_actions(&mut self, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .id("vault-actions")
            // Blocks the title bar's drag area behind it: otherwise Windows takes
            // the click as "move the window" and the buttons never get it.
            .occlude()
            .gap_0p5()
            .pr_2()
            .child(self.render_sync_button(cx))
            .child(
                Button::new("vault-health")
                    .ghost()
                    .small()
                    .icon(IconName::ShieldCheck)
                    .tooltip(t!("vault.health_tooltip"))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(VaultEvent::OpenHealth))),
            )
            .child(
                Button::new("vault-generator")
                    .ghost()
                    .small()
                    .icon(IconName::Dices)
                    .tooltip(t!("vault.generator_tooltip"))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(VaultEvent::OpenGenerator))),
            )
            .child(
                Button::new("vault-settings")
                    .ghost()
                    .small()
                    .icon(IconName::Settings)
                    .tooltip(t!("vault.settings_tooltip"))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(VaultEvent::OpenSettings))),
            )
            .child(
                Button::new("vault-lock")
                    .ghost()
                    .small()
                    .icon(IconName::Lock)
                    .tooltip(t!("vault.lock_tooltip"))
                    .on_click(cx.listener(|this, _, _, cx| this.request_lock(cx))),
            )
            .into_any_element()
    }

    fn render_list_column(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let list = self.list.read(cx).delegate();
        let filter = list.filter().clone();
        let count = list.count(&filter);
        let title = self.filter_title(&filter, cx);
        let list_key = format!("{filter:?}");
        v_flex()
            .w(px(330.))
            .h_full()
            .flex_shrink_0()
            .border_r_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .px_4()
                    .pt_4()
                    .pb_1()
                    .gap_2()
                    .child(match &self.renaming_folder {
                        Some((_, input, _)) => h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(Input::new(input).id("folder-rename-name").small()),
                            )
                            .child(
                                Button::new("folder-rename-save")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Check)
                                    .tooltip(t!("vault.save_enter"))
                                    .disabled(self.syncing)
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.save_rename_folder(cx)),
                                    ),
                            )
                            .child(
                                Button::new("folder-rename-cancel")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Close)
                                    .tooltip(t!("common.cancel"))
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.cancel_folder_edit(cx)),
                                    ),
                            )
                            .into_any_element(),
                        None => div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_lg()
                            .font_semibold()
                            .child(title)
                            .into_any_element(),
                    })
                    .when(
                        matches!(filter, Filter::Folder(Some(_))) && self.renaming_folder.is_none(),
                        |row| {
                            let confirming = self.confirm_delete_folder;
                            row.child(
                                Button::new("folder-rename")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Pencil)
                                    .tooltip(t!("vault.rename_folder"))
                                    .disabled(self.syncing)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.start_rename_folder(window, cx)
                                    })),
                            )
                            .child(
                                Button::new("folder-delete")
                                    .when(confirming, |b| {
                                        b.danger().label(t!("vault.delete_confirm"))
                                    })
                                    .when(!confirming, |b| {
                                        b.ghost()
                                            .icon(IconName::Trash)
                                            .tooltip(t!("vault.delete_folder"))
                                    })
                                    .xsmall()
                                    .disabled(self.syncing)
                                    .on_click(cx.listener(|this, _, _, cx| this.delete_folder(cx))),
                            )
                        },
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(if count == 1 {
                                t!("vault.item_count_one").to_owned()
                            } else {
                                t!("vault.item_count", n = count)
                            }),
                    )
                    .child(
                        Button::new("vault-new")
                            .ghost()
                            .small()
                            .icon(IconName::Plus)
                            .tooltip(t!("vault.new_item_tooltip"))
                            .disabled(self.syncing)
                            .on_click(cx.listener(|this, _, window, cx| this.new_item(window, cx))),
                    ),
            )
            .child(if self.syncing || self.downloading.is_some() {
                motion::progress_bar("vault-progress", appearance::accent(cx)).into_any_element()
            } else {
                div().h(px(2.)).into_any_element()
            })
            .child(motion::fade(
                div().flex_1().min_h_0().child(List::new(&self.list)),
                motion::key("lista", &list_key),
            ))
    }

    /// "This item asks for the master password" card, at the top of the detail.
    fn render_reprompt(&self, item: &Item, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !item.reprompt {
            return None;
        }
        let accent = appearance::accent(cx);
        let theme = cx.theme();
        if !self.needs_reprompt(item) {
            return Some(
                h_flex()
                    .id("reprompt-ok")
                    .test_support()
                    .gap_2()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(Icon::new(IconName::LockOpen).xsmall())
                    .child(t!("vault.reprompt_unlocked"))
                    .into_any_element(),
            );
        }
        let prompt = self.reprompt.as_ref().filter(|p| p.item_id == item.id);
        let card = v_flex()
            .id("reprompt-card")
            .test_support()
            .gap_3()
            .p_4()
            .rounded(theme.radius_lg)
            .border_1()
            .border_color(accent.opacity(0.45))
            .bg(accent.opacity(0.06))
            .child(
                h_flex()
                    .gap_2()
                    .text_sm()
                    .font_semibold()
                    .child(Icon::new(IconName::LockKeyhole).small().text_color(accent))
                    .child(t!("vault.reprompt_title")),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(t!("vault.reprompt_body")),
            );
        let card = match prompt {
            None => card.child(
                h_flex().child(
                    Button::new("reprompt-start")
                        .outline()
                        .small()
                        .icon(IconName::KeyRound)
                        .label(t!("vault.reprompt_start"))
                        .on_click(
                            cx.listener(|this, _, window, cx| this.start_reprompt(window, cx)),
                        ),
                ),
            ),
            Some(prompt) => card
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            div().flex_1().min_w_0().child(
                                Input::new(&prompt.input)
                                    .id("reprompt-password")
                                    .small()
                                    .disabled(prompt.checking),
                            ),
                        )
                        .child(
                            Button::new("reprompt-confirm")
                                .primary()
                                .small()
                                .label(if prompt.checking {
                                    t!("vault.checking")
                                } else {
                                    t!("common.confirm")
                                })
                                .loading(prompt.checking)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.confirm_reprompt(window, cx)
                                })),
                        )
                        .child(
                            Button::new("reprompt-cancel")
                                .ghost()
                                .small()
                                .label(t!("common.cancel"))
                                .on_click(cx.listener(|this, _, _, cx| this.cancel_reprompt(cx))),
                        ),
                )
                .children(prompt.error.clone().map(|e| {
                    motion::shake(
                        div()
                            .id("reprompt-error")
                            .test_support()
                            .aria_label(e.clone())
                            .text_xs()
                            .text_color(cx.theme().danger)
                            .child(e.clone()),
                        motion::key("reprompt-erro", &e),
                    )
                })),
        };
        Some(motion::rise(card, motion::key("reprompt", &item.id), 6.).into_any_element())
    }

    fn render_detail(&self, cx: &mut Context<Self>) -> AnyElement {
        if let Some((editor, _)) = &self.editor {
            return motion::slide(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .child(editor.clone()),
                motion::key("editor", editor.entity_id()),
                14.,
            )
            .into_any_element();
        }
        let Some(item) = &self.selected else {
            return v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .gap_3()
                .text_color(cx.theme().muted_foreground)
                .child(ui::icon_chip(IconName::LockKeyhole, px(48.), false, cx))
                .child(div().text_sm().child(t!("vault.select_item")))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground.opacity(0.75))
                        .child(t!("vault.select_item_hint")),
                )
                .with_animation("detalhe-vazio", Animation::new(motion::NORMAL), |el, t| {
                    el.opacity(t)
                })
                .into_any_element();
        };

        let vault = self.list.read(cx).delegate().vault();
        let place = match (&item.organization_id, &item.folder_id) {
            (Some(org), _) => vault.organizations.iter().find(|o| &o.id == org).map(|o| {
                // "Company · Finance, IT"
                let cols: Vec<&str> = vault
                    .collections
                    .iter()
                    .filter(|c| item.collection_ids.contains(&c.id))
                    .map(|c| c.name.as_str())
                    .collect();
                let text = if cols.is_empty() {
                    o.name.clone()
                } else {
                    format!("{} · {}", o.name, cols.join(", "))
                };
                (IconName::Building2, text)
            }),
            (None, Some(folder)) => vault
                .folders
                .iter()
                .find(|f| &f.id == folder)
                .map(|f| (IconName::Folder, f.name.clone())),
            (None, None) => None,
        };

        let busy = self.syncing;
        let mut actions = h_flex().gap_1().flex_shrink_0();
        if item.deleted {
            actions = actions
                .child(
                    Button::new("item-restore")
                        .outline()
                        .small()
                        .icon(IconName::Undo2)
                        .label(t!("vault.restore"))
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| this.restore_selected(cx))),
                )
                .child(
                    Button::new("item-delete-forever")
                        .when(self.confirm_delete, |b| b.danger())
                        .when(!self.confirm_delete, |b| b.ghost())
                        .small()
                        .icon(IconName::Trash)
                        .label(if self.confirm_delete {
                            t!("vault.delete_forever_confirm")
                        } else {
                            t!("vault.delete_forever")
                        })
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| this.delete_selected(cx))),
                );
        } else if self.can_edit(item) {
            let pinned = self.pinned.contains(&item.id);
            actions = actions
                .child(
                    Button::new("item-pin")
                        .ghost()
                        .small()
                        .icon(if pinned {
                            IconName::PinOff
                        } else {
                            IconName::Pin
                        })
                        .tooltip(if pinned {
                            t!("vault.unpin")
                        } else {
                            t!("vault.pin")
                        })
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_pin(cx))),
                )
                .child(
                    Button::new("item-edit")
                        .outline()
                        .small()
                        .icon(IconName::Pencil)
                        .label(t!("common.edit"))
                        .tooltip(t!("vault.edit_tooltip"))
                        .disabled(busy)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.edit_selected(window, cx)),
                        ),
                )
                .child(
                    Button::new("item-trash")
                        .ghost()
                        .small()
                        .icon(IconName::Trash)
                        .tooltip(t!("vault.move_to_trash"))
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| this.trash_selected(cx))),
                );
        }

        let picker_open = self.icon_picker_open();
        let header = h_flex()
            .gap_4()
            .child(
                // Clicking the icon opens the icon and color picker.
                div()
                    .id("item-icon")
                    .test_support()
                    .role(Role::Button)
                    .aria_label(SharedString::from(t!("icons.choose")))
                    .relative()
                    .cursor_pointer()
                    .rounded(cx.theme().radius)
                    .hover(|s| s.opacity(0.85))
                    .active(|s| s.opacity(0.7))
                    .tooltip(|window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(t!("icons.choose"))
                            .build(window, cx)
                    })
                    .child(crate::item_icons::chip(item, px(48.), true, cx))
                    .child(
                        div()
                            .absolute()
                            .right(px(-4.))
                            .bottom(px(-4.))
                            .size(px(18.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .bg(cx.theme().background)
                            .border_1()
                            .border_color(cx.theme().border)
                            .text_color(cx.theme().muted_foreground)
                            .child(
                                Icon::new(if picker_open {
                                    IconName::Close
                                } else {
                                    IconName::Pencil
                                })
                                .xsmall(),
                            ),
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_icon_picker(cx))),
            )
            .child(
                v_flex()
                    .min_w_0()
                    .flex_1()
                    .gap_0p5()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                div()
                                    .id("detail-name")
                                    .test_support()
                                    .aria_label(SharedString::from(item.name.clone()))
                                    .text_2xl()
                                    .font_semibold()
                                    .truncate()
                                    .child(item.name.clone()),
                            )
                            .when(!item.deleted, |el| {
                                let favorite = item.favorite;
                                let (accent, muted, busy) = (
                                    appearance::accent(cx),
                                    cx.theme().muted_foreground,
                                    self.syncing,
                                );
                                el.child(
                                    div()
                                        .id("item-favorite")
                                        .test_support()
                                        .role(Role::Button)
                                        .aria_label(SharedString::from(if favorite {
                                            t!("vault.unfavorite")
                                        } else {
                                            t!("vault.favorite")
                                        }))
                                        .flex_shrink_0()
                                        .p_1()
                                        .rounded(cx.theme().radius)
                                        .when(!busy, |el| {
                                            el.cursor_pointer()
                                                .hover(|s| s.bg(cx.theme().secondary_hover))
                                                .active(|s| s.opacity(0.7))
                                        })
                                        .tooltip(move |window, cx| {
                                            gpui_kit::component::tooltip::Tooltip::new(
                                                if favorite {
                                                    t!("vault.unfavorite")
                                                } else {
                                                    t!("vault.favorite")
                                                },
                                            )
                                            .build(window, cx)
                                        })
                                        .child(motion::pop(
                                            Icon::new(if favorite {
                                                IconName::StarFill
                                            } else {
                                                IconName::Star
                                            })
                                            .small()
                                            .text_color(if favorite { accent } else { muted }),
                                            motion::key(
                                                "favorito",
                                                format!("{}:{favorite}", item.id),
                                            ),
                                        ))
                                        .on_click(
                                            cx.listener(|this, _, _, cx| this.toggle_favorite(cx)),
                                        ),
                                )
                            }),
                    )
                    .children(place.map(|(icon, name)| {
                        h_flex()
                            .gap_1p5()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(Icon::new(icon).xsmall())
                            .child(name)
                    })),
            )
            .child(actions);

        let has_badges = item.deleted || matches!(item.kind, ItemKind::Unknown(_));
        let mut badges = h_flex().gap_2();
        if item.deleted {
            badges = badges.child(Tag::danger().small().child(t!("vault.in_trash")));
        }
        if let ItemKind::Unknown(code) = item.kind {
            badges = badges.child(
                Tag::secondary()
                    .small()
                    .child(t!("vault.unsupported_type", code = code)),
            );
        }

        let attachments = self.render_attachments(item, cx);
        let rows = field_rows(item);
        let total = rows.len();
        let rows = rows.into_iter().enumerate().map(|(ix, row)| {
            let revealed = !row.secret || self.revealed.contains(&row.key);
            let shown: SharedString = if revealed {
                row.value.to_string().into()
            } else {
                "••••••••".into()
            };
            let copy_key = row.key.clone();
            let reveal_key = row.key.clone();
            h_flex()
                .gap_3()
                .px_4()
                .py_3()
                .when(ix + 1 < total, |el| {
                    el.border_b_1().border_color(cx.theme().border)
                })
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_0p5()
                        .child(
                            h_flex()
                                .gap_2()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(row.label.clone())
                                .children(row.hint.clone().map(|hint| {
                                    div()
                                        .id(SharedString::from(format!("hint-{}", row.key)))
                                        .test_support()
                                        .aria_label(SharedString::from(hint.clone()))
                                        .text_color(appearance::accent(cx))
                                        .child(hint)
                                })),
                        )
                        .child(
                            div()
                                .id(SharedString::from(format!("value-{}", row.key)))
                                .test_support()
                                .aria_label(shown.clone())
                                .min_w_0()
                                .when(row.secret, |d| {
                                    d.font_family(cx.theme().mono_font_family.clone())
                                })
                                .when(!row.multiline, |d| d.truncate())
                                .when(row.multiline && revealed, |d| d.whitespace_normal())
                                .child(motion::fade(
                                    div().child(shown.clone()),
                                    // Never the value in the key (it may be a password):
                                    // only the field, whether it is revealed and, for TOTP, the period.
                                    motion::key(
                                        "valor",
                                        format!("{}:{revealed}:{}", row.key, totp_step(&row.key)),
                                    ),
                                )),
                        ),
                )
                .when(row.secret, |el| {
                    el.child(
                        Button::new(SharedString::from(format!("reveal-{}", row.key)))
                            .ghost()
                            .small()
                            .icon(if revealed {
                                IconName::EyeOff
                            } else {
                                IconName::Eye
                            })
                            .tooltip(if revealed {
                                t!("vault.hide")
                            } else {
                                t!("vault.show")
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.toggle_reveal(&reveal_key, window, cx)
                            })),
                    )
                })
                .child({
                    let just_copied = self
                        .copied
                        .as_ref()
                        .filter(|(k, _)| *k == row.key)
                        .map(|(_, n)| *n);
                    let button = Button::new(SharedString::from(format!("copy-{}", row.key)))
                        .ghost()
                        .small()
                        .icon(if just_copied.is_some() {
                            IconName::Check
                        } else {
                            IconName::Copy
                        })
                        .tooltip(t!("common.copy"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.copy_field(&copy_key, window, cx)
                        }));
                    match just_copied {
                        Some(n) => motion::pop(
                            div().text_color(appearance::accent(cx)).child(button),
                            motion::key("copiado", format!("{}:{n}", row.key)),
                        )
                        .into_any_element(),
                        None => button.into_any_element(),
                    }
                })
        });

        let fields = v_flex()
            .rounded(cx.theme().radius_lg)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .children(rows);

        let detail = v_flex()
            .id("vault-detail")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .child(
                v_flex()
                    .w_full()
                    .max_w(px(720.))
                    .px_8()
                    .py_8()
                    .gap_6()
                    .child(header)
                    .when(self.icon_picker, |el| {
                        el.child(self.render_icon_picker(item, cx))
                    })
                    .when(has_badges, |el| el.child(badges))
                    .children(self.render_reprompt(item, cx))
                    .when(total > 0, |el| el.child(fields))
                    .children(attachments),
            );
        motion::slide(detail, motion::key("detalhe", &item.id), 12.).into_any_element()
    }
}

impl VaultView {
    /// The language changed: the texts come with the next draw (transient fields stay as they are).
    pub fn relabel(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        cx.notify();
    }
}

impl Render for VaultView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(filter) = self.pending_filter.take() {
            self.set_filter(filter, window, cx);
        }
        h_flex()
            .items_stretch()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.render_sidebar(cx))
            .child(self.render_list_column(cx))
            .child(self.render_detail(cx))
    }
}
