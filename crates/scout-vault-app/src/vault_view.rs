//! Cofre aberto: filtros à esquerda, lista no meio, detalhe do item à direita.

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
use crate::item_editor::{EditorEvent, ItemEditor, NewKind};
use crate::item_list::{kind_icon, Filter, ItemList, KindFilter};
use crate::{appearance, clipboard, motion, ui};
use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use scout_vault_core::generator::PasswordOptions;

pub enum VaultEvent {
    /// Pede para bloquear; leva a sessão (com tokens possivelmente renovados).
    Lock(Session),
    OpenSettings,
    /// A sessão expirou no servidor: volta para o login.
    SessionExpired,
    /// Sincronizou: tokens e cache novos para guardar.
    Synced(Session),
    /// O "Acesso rápido" mudou: ids para guardar nas configurações.
    PinsChanged(Vec<String>),
    OpenGenerator,
}

/// O que os atalhos de cópia copiam.
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
            Self::Username => "Este item não tem usuário.",
            Self::Password => "Este item não tem senha.",
            Self::TotpCode => "Este item não tem código de autenticador.",
        }
    }
}

/// Um campo exibível do item selecionado.
struct FieldRow<'a> {
    key: String,
    label: String,
    value: Cow<'a, str>,
    secret: bool,
    multiline: bool,
    /// Texto curto ao lado do rótulo (ex.: segundos restantes do código TOTP).
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

/// Período atual do TOTP (muda a cada 30 s), para animar a troca do código.
fn totp_step(key: &str) -> u64 {
    if key != TOTP_CODE_KEY {
        return 0;
    }
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() / 30)
        .unwrap_or(0)
}

/// Código TOTP atual do item, se ele tiver uma chave válida.
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
                "Usuário",
                l.username.as_deref(),
                false,
            );
            add(
                &mut rows,
                "password",
                "Senha",
                l.password.as_ref().map(|s| s.expose()),
                true,
            );
            if let Some(code) = live_totp(item) {
                rows.push(FieldRow {
                    key: TOTP_CODE_KEY.into(),
                    label: "Código do autenticador".into(),
                    value: Cow::Owned(totp::group(&code.code)),
                    secret: false,
                    multiline: false,
                    hint: Some(format!("{} s", code.remaining)),
                });
            }
            add(
                &mut rows,
                "totp",
                "Chave do autenticador (TOTP)",
                l.totp.as_ref().map(|s| s.expose()),
                true,
            );
            for (i, uri) in l.uris.iter().enumerate() {
                add(&mut rows, &format!("uri-{i}"), "Site", Some(uri), false);
            }
        }
        ItemKind::Card(c) => {
            add(
                &mut rows,
                "card-holder",
                "Titular",
                c.cardholder_name.as_deref(),
                false,
            );
            add(
                &mut rows,
                "card-brand",
                "Bandeira",
                c.brand.as_deref(),
                false,
            );
            add(
                &mut rows,
                "card-number",
                "Número",
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
                    label: "Validade".into(),
                    value: Cow::Owned(exp),
                    secret: false,
                    multiline: false,
                    hint: None,
                });
            }
            add(
                &mut rows,
                "card-code",
                "Código de segurança",
                c.code.as_ref().map(|s| s.expose()),
                true,
            );
        }
        ItemKind::Identity(i) => {
            add(
                &mut rows,
                "id-title",
                "Tratamento",
                i.title.as_deref(),
                false,
            );
            add(
                &mut rows,
                "id-first",
                "Nome",
                i.first_name.as_deref(),
                false,
            );
            add(
                &mut rows,
                "id-middle",
                "Nome do meio",
                i.middle_name.as_deref(),
                false,
            );
            add(
                &mut rows,
                "id-last",
                "Sobrenome",
                i.last_name.as_deref(),
                false,
            );
            add(
                &mut rows,
                "id-company",
                "Empresa",
                i.company.as_deref(),
                false,
            );
            add(&mut rows, "id-email", "E-mail", i.email.as_deref(), false);
            add(&mut rows, "id-phone", "Telefone", i.phone.as_deref(), false);
            add(
                &mut rows,
                "id-username",
                "Usuário",
                i.username.as_deref(),
                false,
            );
            add(
                &mut rows,
                "id-address1",
                "Endereço",
                i.address1.as_deref(),
                false,
            );
            add(
                &mut rows,
                "id-address2",
                "Complemento",
                i.address2.as_deref(),
                false,
            );
            add(
                &mut rows,
                "id-address3",
                "Complemento 2",
                i.address3.as_deref(),
                false,
            );
            add(&mut rows, "id-city", "Cidade", i.city.as_deref(), false);
            add(&mut rows, "id-state", "Estado", i.state.as_deref(), false);
            add(
                &mut rows,
                "id-postal",
                "CEP",
                i.postal_code.as_deref(),
                false,
            );
            add(&mut rows, "id-country", "País", i.country.as_deref(), false);
            add(
                &mut rows,
                "id-ssn",
                "CPF / SSN",
                i.ssn.as_ref().map(|s| s.expose()),
                true,
            );
            add(
                &mut rows,
                "id-passport",
                "Passaporte",
                i.passport_number.as_ref().map(|s| s.expose()),
                true,
            );
            add(
                &mut rows,
                "id-license",
                "CNH",
                i.license_number.as_ref().map(|s| s.expose()),
                true,
            );
        }
        ItemKind::SshKey(k) => {
            add(
                &mut rows,
                "ssh-fingerprint",
                "Impressão digital",
                k.fingerprint.as_deref(),
                false,
            );
            add(
                &mut rows,
                "ssh-public",
                "Chave pública",
                k.public_key.as_deref(),
                false,
            );
            add(
                &mut rows,
                "ssh-private",
                "Chave privada",
                k.private_key.as_ref().map(|s| s.expose()),
                true,
            );
        }
        ItemKind::SecureNote | ItemKind::Unknown(_) => {}
    }

    for (i, f) in item.fields.iter().enumerate() {
        let label = f.name.clone().unwrap_or_else(|| "Campo".into());
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
        "Notas",
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
    /// Redesenha a cada segundo enquanto o item aberto tem código TOTP.
    _ticker: Task<()>,
    /// Editor aberto no lugar do detalhe (novo item ou edição).
    editor: Option<(Entity<ItemEditor>, Subscription)>,
    /// "Apagar de vez" pede um segundo clique.
    confirm_delete: bool,
    /// Opções do gerador, para o botão de gerar senha no editor.
    password_options: PasswordOptions,
    /// "Remover anexo" pede um segundo clique.
    confirm_remove_attachment: Option<String>,
    /// Campo copiado há pouco: o botão mostra ✓ por um instante.
    copied: Option<(String, u32)>,
    /// Itens fixados no "Acesso rápido", na ordem em que foram fixados.
    pinned: Vec<String>,
    /// Campo de "nova pasta" aberto na barra lateral.
    new_folder: Option<(Entity<InputState>, Subscription)>,
    /// Pasta sendo renomeada (id + campo) no cabeçalho da lista.
    renaming_folder: Option<(String, Entity<InputState>, Subscription)>,
    /// "Apagar pasta" pede um segundo clique.
    confirm_delete_folder: bool,
    /// Filtro a aplicar no próximo quadro (depois de criar/apagar uma pasta).
    pending_filter: Option<Filter>,
    /// Resultado da última sincronização (ok?, contador), mostrado no ícone por um instante.
    sync_feedback: Option<(bool, u32)>,
    /// Quantas sincronizações já terminaram (reinicia as animações do ícone).
    sync_count: u32,
    /// Posição de cada linha da barra lateral no quadro atual (para a cascata).
    sidebar_order: std::cell::Cell<usize>,
    /// Anexo sendo baixado agora.
    downloading: Option<String>,
    /// Cópias temporárias abertas com outro programa; apagadas ao bloquear.
    temp_files: Vec<std::path::PathBuf>,
}

impl Drop for VaultView {
    fn drop(&mut self) {
        // Bloquear/sair solta esta tela: some com as cópias decifradas.
        for path in &self.temp_files {
            crate::attachments::remove_temp(path);
        }
    }
}

impl EventEmitter<VaultEvent> for VaultView {}

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
                cx.notify();
            }
            ListEvent::Cancel => {}
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
        let mut this = Self {
            editor: None,
            confirm_delete: false,
            confirm_remove_attachment: None,
            copied: None,
            sidebar_order: std::cell::Cell::new(0),
            sync_feedback: None,
            new_folder: None,
            pinned: Vec::new(),
            renaming_folder: None,
            confirm_delete_folder: false,
            pending_filter: None,
            sync_count: 0,
            downloading: None,
            temp_files: Vec::new(),
            password_options: PasswordOptions::default(),
            _ticker: ticker,
            session,
            keys: Arc::new(keys),
            list,
            selected: None,
            revealed: HashSet::new(),
            syncing: false,
            _subscriptions: vec![sub],
        };
        if errors > 0 {
            crate::toast::error(format!("{errors} item(ns) não puderam ser decifrados."), cx);
        }
        // Aberto a partir da sessão lembrada (sem access token): busca o que
        // mudou no servidor. Sem rede, o cofre segue aberto com o cache.
        if this.session.access_token.is_empty() {
            this.sync(cx);
        }
        this
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    pub fn list(&self) -> &Entity<ListState<ItemList>> {
        &self.list
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
        cx.notify();
    }

    pub fn set_filter(&mut self, filter: Filter, window: &mut Window, cx: &mut Context<Self>) {
        self.list.update(cx, |list, cx| {
            list.delegate_mut().set_filter(filter);
            list.set_selected_index(None, window, cx);
            cx.notify();
        });
        self.selected = None;
        self.revealed.clear();
        cx.notify();
    }

    fn toggle_reveal(&mut self, key: &str, cx: &mut Context<Self>) {
        if !self.revealed.remove(key) {
            self.revealed.insert(key.to_owned());
        }
        cx.notify();
    }

    /// Atalho de cópia (`Ctrl+U`, `Ctrl+P`, `Ctrl+T`) no item aberto.
    pub fn copy_shortcut(
        &mut self,
        what: CopyShortcut,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = &self.selected else {
            let _ = window;
            crate::toast::info("Abra um item primeiro.", cx);
            return;
        };
        let key = what.key();
        if field_rows(item).iter().any(|r| r.key == key) {
            self.copy_field(key, window, cx);
        } else {
            crate::toast::info(what.missing(), cx);
        }
    }

    /// Foca a busca da lista (`Ctrl+F`).
    pub fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.list.update(cx, |list, cx| list.focus(window, cx));
    }

    fn copy_field(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = &self.selected else { return };
        let Some(row) = field_rows(item).into_iter().find(|r| r.key == key) else {
            return;
        };
        let value = if row.key == TOTP_CODE_KEY {
            Cow::Owned(row.value.replace(' ', ""))
        } else {
            row.value.clone()
        };
        clipboard::copy_and_clear_later(&value, cx);
        // ✓ no botão por um instante (o contador reinicia a animação a cada cópia).
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
            format!(
                "Copiado: {} · some em {} s",
                row.label,
                clipboard::CLEAR_AFTER.as_secs()
            )
        } else {
            format!("Copiado: {}", row.label)
        };
        let _ = window;
        crate::toast::success(message, cx);
    }

    fn request_lock(&mut self, cx: &mut Context<Self>) {
        cx.emit(VaultEvent::Lock(self.session.clone()));
    }

    fn sync(&mut self, cx: &mut Context<Self>) {
        self.run_write(WriteOp::Nothing, cx);
    }

    pub fn set_password_options(&mut self, options: PasswordOptions) {
        self.password_options = options;
    }

    pub fn editor(&self) -> Option<&Entity<ItemEditor>> {
        self.editor.as_ref().map(|(e, _)| e)
    }

    fn folders(&self, cx: &App) -> Vec<(String, String)> {
        self.list
            .read(cx)
            .delegate()
            .vault()
            .folders
            .iter()
            .map(|f| (f.id.clone(), f.name.clone()))
            .collect()
    }

    /// O servidor deixa editar este item? (itens de organização podem ser só leitura)
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

    /// Quantos itens cabem no acesso rápido.
    pub const MAX_PINNED: usize = 10;

    pub fn set_pinned(&mut self, ids: Vec<String>) {
        self.pinned = ids;
    }

    pub fn pinned(&self) -> &[String] {
        &self.pinned
    }

    /// Fixa ou desafixa o item aberto no "Acesso rápido".
    pub fn toggle_pin(&mut self, cx: &mut Context<Self>) {
        let Some(item) = &self.selected else { return };
        let id = item.id.clone();
        if let Some(pos) = self.pinned.iter().position(|p| *p == id) {
            self.pinned.remove(pos);
            crate::toast::info("Removido do acesso rápido", cx);
        } else if self.pinned.len() >= Self::MAX_PINNED {
            crate::toast::error(
                format!(
                    "O acesso rápido já tem {} itens; tire um antes.",
                    Self::MAX_PINNED
                ),
                cx,
            );
            return;
        } else {
            self.pinned.push(id);
            crate::toast::success("Fixado no acesso rápido", cx);
        }
        cx.emit(VaultEvent::PinsChanged(self.pinned.clone()));
        cx.notify();
    }

    /// Abre um item do acesso rápido (mostrando "Todos" se o filtro atual o esconde).
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

    /// Itens fixados que ainda existem (fora da lixeira), na ordem de fixação.
    fn pinned_items(&self, cx: &App) -> Vec<Item> {
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
            .when(selected, |el| el.text_color(accent).font_medium())
            .when(!selected, |el| el.text_color(fg).hover(|el| el.bg(hover)))
            .active(|el| el.opacity(0.7))
            .when(selected, |el| {
                el.child(motion::accent_bar(
                    motion::key("fixado-barra", &row_id),
                    accent,
                ))
            })
            .child(
                Icon::new(kind_icon(&item.kind))
                    .small()
                    .when(!selected, |i| i.text_color(muted)),
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

    /// Abre o campo de nova pasta na barra lateral.
    pub fn start_new_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.new_folder.is_some() {
            return;
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Nome da pasta"));
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
            crate::toast::error("Dê um nome à pasta.", cx);
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

    /// Renomear a pasta aberta: o título da lista vira um campo.
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
            crate::toast::error("Dê um nome à pasta.", cx);
            return;
        }
        let id = id.clone();
        self.run_write(WriteOp::RenameFolder { id, name }, cx);
    }

    /// Apagar a pasta aberta (segundo clique confirma).
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

    /// Novo item (`Ctrl+N`). Começa na pasta do filtro atual, se houver.
    pub fn new_item(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Não descarta um formulário já aberto.
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
            _ => NewKind::Login,
        };
        let folders = self.folders(cx);
        let options = self.password_options;
        let editor = cx.new(|cx| ItemEditor::create(kind, folder, folders, options, window, cx));
        self.open_editor(editor, window, cx);
    }

    /// Edita o item aberto (`Ctrl+E`).
    pub fn edit_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.selected.clone() else {
            return;
        };
        if self.syncing || self.editor.is_some() || item.deleted || !self.can_edit(&item) {
            return;
        }
        let folders = self.folders(cx);
        let options = self.password_options;
        let editor = cx.new(|cx| ItemEditor::edit(&item, folders, options, window, cx));
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

    /// Escolhe um arquivo no Windows e anexa ao item aberto.
    fn pick_attachment(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Anexar".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await {
                this.update(cx, |this, cx| this.attach_paths(&paths, cx))
                    .ok();
            }
        })
        .detach();
    }

    /// Anexa o arquivo em `path` ao item aberto.
    pub fn attach_file_from_path(&mut self, path: &std::path::Path, cx: &mut Context<Self>) {
        self.attach_paths(&[path.to_path_buf()], cx);
    }

    /// Anexa vários arquivos ao item aberto (escolhidos ou arrastados).
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

    /// Baixa e decifra; depois `then` decide o que fazer com o conteúdo.
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

    /// Abre o anexo com o programa padrão (cópia temporária, apagada ao bloquear).
    pub fn open_attachment(&mut self, attachment_id: String, cx: &mut Context<Self>) {
        self.fetch_attachment(
            attachment_id,
            |this, name, bytes, cx| match crate::attachments::write_temp(&name, &bytes) {
                Ok(path) => {
                    if crate::attachments::system_open_enabled() {
                        cx.open_with_system(&path);
                    }
                    this.temp_files.push(path);
                    crate::toast::success(format!("{name} aberto."), cx);
                }
                Err(e) => crate::toast::error(format!("Não foi possível abrir: {e}"), cx),
            },
            cx,
        );
    }

    /// Salva o anexo decifrado onde a pessoa escolher.
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
                            crate::toast::success(format!("Salvo em {}.", path.display()), cx)
                        }
                        Err(e) => crate::toast::error(format!("Não foi possível salvar: {e}"), cx),
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
                        .tooltip("Abrir")
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
                        .tooltip("Salvar como…")
                        .disabled(busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.save_attachment(save_id.clone(), cx)
                        })),
                )
                .when(can_add, |row| {
                    row.child(
                        Button::new(SharedString::from(format!("attach-remove-{}", a.id)))
                            .when(confirming, |b| b.danger().label("Remover?"))
                            .when(!confirming, |b| {
                                b.ghost().icon(IconName::Trash).tooltip("Remover anexo")
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
                .child(ui::section_label("ANEXOS", cx).px_0())
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
                // Sempre visível em itens editáveis: clicar ou arrastar arquivos.
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

    /// Manda uma alteração (ou só sincroniza) e atualiza a tela com o resultado.
    fn run_write(&mut self, op: WriteOp, cx: &mut Context<Self>) {
        if self.syncing {
            return;
        }
        let done_message = match &op {
            WriteOp::Nothing => None,
            WriteOp::Save(item, _) if item.id.is_empty() => Some("Item criado."),
            WriteOp::Save(..) => Some("Alterações salvas."),
            WriteOp::Trash(_) => Some("Item movido para a lixeira."),
            WriteOp::Restore(_) => Some("Item restaurado."),
            WriteOp::Delete(_) => Some("Item apagado de vez."),
            WriteOp::AddAttachments { files, .. } if files.len() > 1 => Some("Arquivos anexados."),
            WriteOp::AddAttachments { .. } => Some("Arquivo anexado."),
            WriteOp::RemoveAttachment { .. } => Some("Anexo removido."),
            WriteOp::CreateFolder(_) => Some("Pasta criada."),
            WriteOp::RenameFolder { .. } => Some("Pasta renomeada."),
            WriteOp::DeleteFolder(_) => Some("Pasta apagada; os itens foram para Sem pasta."),
        };
        let closes_detail = matches!(op, WriteOp::Delete(_));
        // Pastas: depois de salvar, abre a pasta; depois de apagar, volta para todos.
        let folder_saved = matches!(op, WriteOp::CreateFolder(_) | WriteOp::RenameFolder { .. });
        let folder_deleted = matches!(op, WriteOp::DeleteFolder(_));
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
                    }) => {
                        for w in &warnings {
                            crate::toast::error(format!("Anexo não enviado — {w}"), cx);
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
                        let saved_id = if folder_saved || folder_deleted {
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
                        // Mantém o item aberto, agora com os dados novos.
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
                        // Sincronizar sozinho já mostra o ✓ no ícone; o resto vira toast.
                        let attach_only = done_message
                            .is_some_and(|m| m.starts_with("Arquivo") || m.starts_with("Arquivos"));
                        if let Some(m) =
                            done_message.filter(|_| !(attach_only && !warnings.is_empty()))
                        {
                            crate::toast::success(m.trim_end_matches('.'), cx);
                        }
                        if errors > 0 {
                            crate::toast::error(
                                format!("{errors} item(ns) não puderam ser decifrados."),
                                cx,
                            );
                        }
                    }
                    Err(flow::SyncError::Expired) => cx.emit(VaultEvent::SessionExpired),
                    Err(flow::SyncError::Other(e)) => {
                        // Com o editor aberto, o erro aparece nele e nada se perde.
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
            Filter::All => "Todos os itens".into(),
            Filter::Favorites => "Favoritos".into(),
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
            Filter::Folder(None) => "Sem pasta".into(),
            Filter::Trash => "Lixeira".into(),
        }
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
            .when(selected, |el| el.text_color(accent).font_medium())
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
                        .when(!selected, |i| i.text_color(muted)),
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
            // O destaque do filtro escolhido acende suave.
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
        // Ao abrir o cofre, as linhas entram em cascata (uma vez por tela).
        motion::cascade(
            div().w_full().child(row),
            motion::key("filtro-entra", &id_text),
            order,
        )
    }

    /// Botão de sincronizar: gira enquanto trabalha; ao terminar vira ✓ (ou ✗)
    /// por um instante, com transição, e volta às setas.
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
                "Sincronizando…",
            )
        } else if let Some((ok, m)) = self.sync_feedback {
            let (name, color, label) = if ok {
                (IconName::Check, accent, "Sincronizado")
            } else {
                (IconName::Close, danger, "Não foi possível sincronizar")
            };
            (
                // Entra girando um quarto de volta enquanto aparece.
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
                // Volta às setas com meio giro suave (não anima na primeira vez).
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
                "Sincronizar",
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
        let folders: Vec<_> = vault
            .folders
            .iter()
            .map(|f| (f.id.clone(), f.name.clone()))
            .collect();

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
                "Todos os itens".into(),
                cx,
            ))
            .child(self.filter_row(
                "filter-favorites".into(),
                Filter::Favorites,
                IconName::Star,
                "Favoritos".into(),
                cx,
            ));
        // Acesso rápido: itens fixados pela pessoa, logo no topo.
        let pinned = self.pinned_items(cx);
        if !pinned.is_empty() {
            nav = nav.child(ui::section_label("Acesso rápido", cx));
            for item in &pinned {
                nav = nav.child(self.pinned_row(item, cx));
            }
        }
        nav = nav.child(ui::section_label("Tipos", cx));
        for k in KindFilter::ALL {
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
            nav = nav.child(ui::section_label("Organizações", cx));
            for (id, name) in orgs {
                nav = nav.child(self.filter_row(
                    format!("filter-org-{id}").into(),
                    Filter::Organization(id),
                    IconName::Building2,
                    name,
                    cx,
                ));
            }
        }
        nav = nav.child(
            h_flex()
                .justify_between()
                .pr_1()
                .child(ui::section_label("Pastas", cx))
                .child(
                    Button::new("folder-new")
                        .ghost()
                        .xsmall()
                        .icon(IconName::Plus)
                        .tooltip("Nova pasta")
                        .disabled(self.syncing)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.start_new_folder(window, cx)),
                        ),
                ),
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
                            .tooltip("Criar (Enter)")
                            .disabled(self.syncing)
                            .on_click(cx.listener(|this, _, _, cx| this.save_new_folder(cx))),
                    )
                    .child(
                        Button::new("folder-new-cancel")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Close)
                            .tooltip("Cancelar")
                            .on_click(cx.listener(|this, _, _, cx| this.cancel_folder_edit(cx))),
                    ),
                motion::key("pasta-nova", "campo"),
                4.,
            ));
        }
        for (id, name) in folders {
            nav = nav.child(self.filter_row(
                format!("filter-folder-{id}").into(),
                Filter::Folder(Some(id)),
                IconName::Folder,
                name,
                cx,
            ));
        }
        nav = nav
            .child(self.filter_row(
                "filter-no-folder".into(),
                Filter::Folder(None),
                IconName::FolderOpen,
                "Sem pasta".into(),
                cx,
            ))
            .child(div().h_3())
            .child(self.filter_row(
                "filter-trash".into(),
                Filter::Trash,
                IconName::Trash,
                "Lixeira".into(),
                cx,
            ))
            .child(div().h_3());

        let footer = v_flex()
            .gap_2()
            .px_3()
            .py_3()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .gap_1()
                    .child({
                        // E-mail cortado na barra; inteiro ao passar o mouse.
                        let email = SharedString::from(self.session.email.clone());
                        div()
                            .id("vault-email")
                            .test_support()
                            .aria_label(email.clone())
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .tooltip(move |window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(email.clone())
                                    .build(window, cx)
                            })
                            .child(self.session.email.clone())
                    })
                    .child(self.render_sync_button(cx))
                    .child(
                        Button::new("vault-generator")
                            .ghost()
                            .small()
                            .icon(IconName::Dices)
                            .tooltip("Gerador de senhas (Ctrl+G)")
                            .on_click(
                                cx.listener(|_, _, _, cx| cx.emit(VaultEvent::OpenGenerator)),
                            ),
                    )
                    .child(
                        Button::new("vault-settings")
                            .ghost()
                            .small()
                            .icon(IconName::Settings)
                            .tooltip("Configurações (Ctrl+,)")
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(VaultEvent::OpenSettings))),
                    )
                    .child(
                        Button::new("vault-lock")
                            .ghost()
                            .small()
                            .icon(IconName::Lock)
                            .tooltip("Bloquear (Ctrl+L)")
                            .on_click(cx.listener(|this, _, _, cx| this.request_lock(cx))),
                    ),
            );

        v_flex()
            .w(px(240.))
            .h_full()
            .flex_shrink_0()
            .border_r_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().sidebar)
            .child(div().px_4().pt_4().pb_3().child(ui::wordmark(cx)))
            .child(nav)
            .child(footer)
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
                                    .tooltip("Salvar (Enter)")
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
                                    .tooltip("Cancelar")
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
                                    .tooltip("Renomear pasta")
                                    .disabled(self.syncing)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.start_rename_folder(window, cx)
                                    })),
                            )
                            .child(
                                Button::new("folder-delete")
                                    .when(confirming, |b| b.danger().label("Apagar?"))
                                    .when(!confirming, |b| {
                                        b.ghost().icon(IconName::Trash).tooltip("Apagar pasta")
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
                                "1 item".to_owned()
                            } else {
                                format!("{count} itens")
                            }),
                    )
                    .child(
                        Button::new("vault-new")
                            .ghost()
                            .small()
                            .icon(IconName::Plus)
                            .tooltip("Novo item (Ctrl+N)")
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
                .child(
                    div()
                        .text_sm()
                        .child("Selecione um item para ver os detalhes."),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground.opacity(0.75))
                        .child("Use as setas na lista; Enter abre o item."),
                )
                .with_animation("detalhe-vazio", Animation::new(motion::NORMAL), |el, t| {
                    el.opacity(t)
                })
                .into_any_element();
        };

        let vault = self.list.read(cx).delegate().vault();
        let place = match (&item.organization_id, &item.folder_id) {
            (Some(org), _) => vault
                .organizations
                .iter()
                .find(|o| &o.id == org)
                .map(|o| (IconName::Building2, o.name.clone())),
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
                        .label("Restaurar")
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
                            "Confirmar: apagar de vez"
                        } else {
                            "Apagar de vez"
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
                            "Tirar do acesso rápido"
                        } else {
                            "Fixar no acesso rápido"
                        })
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_pin(cx))),
                )
                .child(
                    Button::new("item-edit")
                        .outline()
                        .small()
                        .icon(IconName::Pencil)
                        .label("Editar")
                        .tooltip("Editar (Ctrl+E)")
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
                        .tooltip("Mover para a lixeira")
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| this.trash_selected(cx))),
                );
        }

        let header = h_flex()
            .gap_4()
            .child(ui::icon_chip(kind_icon(&item.kind), px(48.), true, cx))
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
                            .when(item.favorite, |el| {
                                el.child(
                                    Icon::new(IconName::StarFill)
                                        .small()
                                        .text_color(appearance::accent(cx)),
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

        let has_badges = item.deleted || item.reprompt || matches!(item.kind, ItemKind::Unknown(_));
        let mut badges = h_flex().gap_2();
        if item.deleted {
            badges = badges.child(Tag::danger().small().child("Na lixeira"));
        }
        if item.reprompt {
            badges = badges.child(
                Tag::warning()
                    .small()
                    .child("Pede a senha mestra (ainda não suportado)"),
            );
        }
        if let ItemKind::Unknown(code) = item.kind {
            badges = badges.child(
                Tag::secondary()
                    .small()
                    .child(format!("Tipo {code} ainda não suportado")),
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
                                    // Nunca o valor na chave (pode ser uma senha):
                                    // só o campo, se está revelado e, no TOTP, o período.
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
                            .tooltip(if revealed { "Ocultar" } else { "Mostrar" })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle_reveal(&reveal_key, cx)
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
                        .tooltip("Copiar")
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
                    .when(has_badges, |el| el.child(badges))
                    .when(total > 0, |el| el.child(fields))
                    .children(attachments),
            );
        motion::slide(detail, motion::key("detalhe", &item.id), 12.).into_any_element()
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
