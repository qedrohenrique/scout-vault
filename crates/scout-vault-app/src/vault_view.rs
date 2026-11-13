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
use crate::{appearance, clipboard, ui};
use gpui_kit::assets::IconName;
use scout_vault_core::generator::PasswordOptions;

pub enum VaultEvent {
    /// Pede para bloquear; leva a sessão (com tokens possivelmente renovados).
    Lock(Session),
    OpenSettings,
    /// A sessão expirou no servidor: volta para o login.
    SessionExpired,
    /// Sincronizou: tokens e cache novos para guardar.
    Synced(Session),
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
    status: Option<SharedString>,
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
            status: (errors > 0)
                .then(|| format!("{errors} item(ns) não puderam ser decifrados.").into()),
            _subscriptions: vec![sub],
        };
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
            window.push_notification(SharedString::from("Abra um item primeiro."), cx);
            return;
        };
        let key = what.key();
        if field_rows(item).iter().any(|r| r.key == key) {
            self.copy_field(key, window, cx);
        } else {
            window.push_notification(SharedString::from(what.missing()), cx);
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
        let message = if row.secret {
            format!(
                "{} copiado. A área de transferência será limpa em {} s.",
                row.label,
                clipboard::CLEAR_AFTER.as_secs()
            )
        } else {
            format!("{} copiado.", row.label)
        };
        window.push_notification(SharedString::from(message), cx);
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
                EditorEvent::Save(item) => this.run_write(WriteOp::Save(item.clone()), cx),
            },
        );
        self.editor = Some((editor, sub));
        self.confirm_delete = false;
        cx.notify();
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
            multiple: false,
            prompt: Some("Anexar".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await {
                if let Some(path) = paths.into_iter().next() {
                    this.update(cx, |this, cx| this.attach_file_from_path(&path, cx))
                        .ok();
                }
            }
        })
        .detach();
    }

    /// Anexa o arquivo em `path` ao item aberto.
    pub fn attach_file_from_path(&mut self, path: &std::path::Path, cx: &mut Context<Self>) {
        let Some(item) = &self.selected else { return };
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "arquivo".into());
        match std::fs::read(path) {
            Ok(bytes) if bytes.len() > scout_vault_core::attachment::MAX_SIZE => {
                self.status = Some("O arquivo passa do limite de 500 MB.".into());
                cx.notify();
            }
            Ok(bytes) => self.run_write(
                WriteOp::AddAttachment {
                    item_id: item.id.clone(),
                    file_name,
                    contents: zeroize::Zeroizing::new(bytes),
                },
                cx,
            ),
            Err(e) => {
                self.status = Some(format!("Não foi possível ler o arquivo: {e}").into());
                cx.notify();
            }
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
        self.status = Some(format!("Baixando {}…", att.file_name).into());
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
                    Err(flow::SyncError::Other(e)) => this.status = Some(e.into()),
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
                    this.status = Some(
                        format!("{name} aberto. A cópia temporária é apagada ao bloquear.").into(),
                    );
                }
                Err(e) => this.status = Some(format!("Não foi possível abrir: {e}").into()),
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
                    let message = match std::fs::write(&path, &bytes[..]) {
                        Ok(()) => format!("Salvo em {}.", path.display()),
                        Err(e) => format!("Não foi possível salvar: {e}"),
                    };
                    this.update(cx, |this, cx| {
                        this.status = Some(message.into());
                        cx.notify();
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
                .child(
                    h_flex()
                        .justify_between()
                        .child(ui::section_label("ANEXOS", cx).px_0())
                        .when(can_add, |h| {
                            h.child(
                                Button::new("attach-add")
                                    .ghost()
                                    .small()
                                    .icon(IconName::Paperclip)
                                    .label("Anexar arquivo")
                                    .disabled(busy)
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.pick_attachment(cx)),
                                    ),
                            )
                        }),
                )
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
            WriteOp::Save(item) if item.id.is_empty() => Some("Item criado."),
            WriteOp::Save(_) => Some("Alterações salvas."),
            WriteOp::Trash(_) => Some("Item movido para a lixeira."),
            WriteOp::Restore(_) => Some("Item restaurado."),
            WriteOp::Delete(_) => Some("Item apagado de vez."),
            WriteOp::AddAttachment { .. } => Some("Arquivo anexado."),
            WriteOp::RemoveAttachment { .. } => Some("Anexo removido."),
        };
        let closes_detail = matches!(op, WriteOp::Delete(_));
        self.syncing = true;
        self.status = Some(
            if matches!(op, WriteOp::Nothing) {
                "Sincronizando…"
            } else {
                "Salvando…"
            }
            .into(),
        );
        if let Some((editor, _)) = &self.editor {
            editor.update(cx, |e, cx| e.set_saving(true, None, cx));
        }
        cx.notify();
        let work = flow::write(self.session.clone(), self.keys.clone(), op);
        cx.spawn(async move |this, cx| {
            let result = work.await;
            this.update(cx, |this, cx| {
                this.syncing = false;
                match result {
                    Ok((session, vault, saved_id)) => {
                        if done_message.is_some() {
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
                        this.status = Some(match (done_message, errors) {
                            (Some(m), _) => m.into(),
                            (None, 0) => "Sincronizado.".into(),
                            (None, n) => {
                                format!("Sincronizado; {n} item(ns) não puderam ser decifrados.")
                                    .into()
                            }
                        });
                    }
                    Err(flow::SyncError::Expired) => cx.emit(VaultEvent::SessionExpired),
                    Err(flow::SyncError::Other(e)) => {
                        // Com o editor aberto, o erro aparece nele e nada se perde.
                        if let Some((editor, _)) = &this.editor {
                            let msg = e.clone();
                            editor.update(cx, |ed, cx| ed.set_saving(false, Some(msg), cx));
                        }
                        this.status = Some(e.into());
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
        let tint = appearance::accent_tint(cx, 0.13);
        let theme = cx.theme();
        let (fg, muted, hover) = (
            theme.foreground.opacity(0.85),
            theme.muted_foreground,
            theme.secondary_hover,
        );
        h_flex()
            .id(id)
            .test_support()
            .role(Role::Button)
            .aria_label(SharedString::from(label.clone()))
            .w_full()
            .gap_2p5()
            .px_2()
            .py_1()
            .rounded(theme.radius)
            .text_sm()
            .cursor_pointer()
            .when(selected, |el| el.bg(tint).text_color(accent).font_medium())
            .when(!selected, |el| el.text_color(fg).hover(|el| el.bg(hover)))
            .child(
                Icon::new(icon)
                    .small()
                    .when(!selected, |i| i.text_color(muted)),
            )
            .child(div().flex_1().min_w_0().truncate().child(label))
            .child(
                div()
                    .text_xs()
                    .when(!selected, |d| d.text_color(muted))
                    .child(count.to_string()),
            )
            .on_click(
                cx.listener(move |this, _, window, cx| this.set_filter(filter.clone(), window, cx)),
            )
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
            ))
            .child(ui::section_label("Tipos", cx));
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
        nav = nav.child(ui::section_label("Pastas", cx));
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
                div()
                    .id("vault-status")
                    .role(Role::Status)
                    .test_support()
                    .aria_label(self.status.clone().unwrap_or_default())
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .children(self.status.clone()),
            )
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(self.session.email.clone()),
                    )
                    .child(
                        Button::new("vault-sync")
                            .ghost()
                            .small()
                            .icon(IconName::RefreshCw)
                            .tooltip("Sincronizar")
                            .loading(self.syncing)
                            .disabled(self.syncing)
                            .on_click(cx.listener(|this, _, _, cx| this.sync(cx))),
                    )
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
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_lg()
                            .font_semibold()
                            .child(title),
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
            .child(div().flex_1().min_h_0().child(List::new(&self.list)))
    }

    fn render_detail(&self, cx: &mut Context<Self>) -> AnyElement {
        if let Some((editor, _)) = &self.editor {
            return editor.clone().into_any_element();
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
            actions = actions
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
                                .child(shown),
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
                .child(
                    Button::new(SharedString::from(format!("copy-{}", row.key)))
                        .ghost()
                        .small()
                        .icon(IconName::Copy)
                        .tooltip("Copiar")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.copy_field(&copy_key, window, cx)
                        })),
                )
        });

        let fields = v_flex()
            .rounded(cx.theme().radius_lg)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .children(rows);

        v_flex()
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
            )
            .into_any_element()
    }
}

impl Render for VaultView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
