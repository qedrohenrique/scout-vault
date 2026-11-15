//! Editor de item: criar e editar logins, notas, cartões, identidades e
//! chaves SSH. Só monta o [`Item`] em texto claro; cifrar e enviar é com
//! [`crate::flow::write`].

use gpui_kit::assets::IconName;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::{button::*, *};
use gpui_kit::prelude::*;
use gpui_kit::*;
use scout_vault_core::generator::{self, PasswordOptions};
use scout_vault_core::vault::{Card, Field, FieldKind, Identity, Item, ItemKind, Login, SshKey};
use scout_vault_core::Secret;

use crate::flow::NewFile;
use crate::ui::{self, Segment};

/// Onde um item pode morar: pastas pessoais, organizações e as coleções delas.
#[derive(Clone, Default)]
pub struct Places {
    pub folders: Vec<(String, String)>,
    pub orgs: Vec<(String, String)>,
    pub collections: Vec<scout_vault_core::vault::Collection>,
}

impl Places {
    /// Coleções em que dá para pôr itens, de uma organização.
    fn writable(&self, org: &str) -> Vec<&scout_vault_core::vault::Collection> {
        self.collections
            .iter()
            .filter(|c| c.organization_id == org && !c.read_only)
            .collect()
    }
}

pub enum EditorEvent {
    /// O item e os arquivos novos a anexar.
    Save(Box<Item>, Vec<NewFile>),
    Cancel,
}

/// Tipos que dá para criar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewKind {
    Login,
    Note,
    Card,
    Identity,
}

impl NewKind {
    pub const ALL: [NewKind; 4] = [Self::Login, Self::Note, Self::Card, Self::Identity];

    fn label(self) -> &'static str {
        match self {
            Self::Login => "Login",
            Self::Note => "Nota",
            Self::Card => "Cartão",
            Self::Identity => "Identidade",
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Login => "login",
            Self::Note => "note",
            Self::Card => "card",
            Self::Identity => "identity",
        }
    }

    fn empty(self) -> ItemKind {
        match self {
            Self::Login => ItemKind::Login(Login::default()),
            Self::Note => ItemKind::SecureNote,
            Self::Card => ItemKind::Card(Card::default()),
            Self::Identity => ItemKind::Identity(Box::default()),
        }
    }
}

/// Um campo de texto simples do formulário.
struct Line {
    key: &'static str,
    label: &'static str,
    state: Entity<InputState>,
    secret: bool,
}

struct CustomField {
    kind: FieldKind,
    name: Entity<InputState>,
    value: Entity<InputState>,
}

pub struct ItemEditor {
    /// Item original (edição) ou um molde vazio (novo).
    base: Item,
    is_new: bool,
    name: Entity<InputState>,
    lines: Vec<Line>,
    uris: Vec<Entity<InputState>>,
    fields: Vec<CustomField>,
    notes: Entity<TextareaState>,
    /// Chave privada SSH (várias linhas).
    ssh_private: Option<Entity<TextareaState>>,
    folder_id: Option<String>,
    favorite: bool,
    places: Places,
    /// Organização dona do item (`None` = cofre pessoal).
    owner: Option<String>,
    collection_ids: Vec<String>,
    generator: PasswordOptions,
    saving: bool,
    error: Option<SharedString>,
    /// Arquivos escolhidos para anexar ao salvar.
    files: Vec<NewFile>,
}

impl EventEmitter<EditorEvent> for ItemEditor {}

fn input_with(
    window: &mut Window,
    cx: &mut App,
    value: Option<&str>,
    secret: bool,
    placeholder: &'static str,
) -> Entity<InputState> {
    let value = value.unwrap_or_default().to_owned();
    cx.new(|cx| {
        InputState::new(window, cx)
            .masked(secret)
            .placeholder(placeholder)
            .default_value(value)
    })
}

const URI_PLACEHOLDER: &str = "https://exemplo.com";
const FIELD_NAME_PLACEHOLDER: &str = "Nome (ex.: PIN)";

fn field_value_placeholder(kind: FieldKind) -> &'static str {
    match kind {
        FieldKind::Hidden => "Valor oculto",
        FieldKind::Boolean => "true ou false",
        _ => "Valor",
    }
}

/// Exemplo dentro do campo vazio: mostra o formato esperado.
fn placeholder_for(key: &str) -> &'static str {
    match key {
        "username" => "nome ou e-mail",
        "password" => "senha",
        "totp" => "chave ou otpauth://…",
        "card-holder" => "como está no cartão",
        "card-brand" => "Visa",
        "card-number" => "0000 0000 0000 0000",
        "card-month" => "MM",
        "card-year" => "AAAA",
        "card-code" => "123",
        "id-title" => "Sr., Sra.",
        "id-email" => "nome@exemplo.com",
        "id-phone" => "(00) 00000-0000",
        "id-postal" => "00000-000",
        "id-state" => "UF",
        "id-country" => "Brasil",
        "id-ssn" => "000.000.000-00",
        "ssh-fingerprint" => "SHA256:…",
        _ => "",
    }
}

/// Como os campos se arrumam em linhas: `None` = ocupa o espaço que sobra,
/// `Some(px)` = largura fixa para campos curtos (mês, CEP, CVV…).
fn layout(kind: &ItemKind) -> &'static [&'static [(&'static str, Option<f32>)]] {
    match kind {
        ItemKind::Login(_) => &[&[("username", None), ("password", None)], &[("totp", None)]],
        ItemKind::Card(_) => &[
            &[("card-holder", None), ("card-brand", Some(120.))],
            &[("card-number", None)],
            &[
                ("card-month", Some(72.)),
                ("card-year", Some(88.)),
                ("card-code", Some(88.)),
            ],
        ],
        ItemKind::Identity(_) => &[
            &[("id-title", Some(100.)), ("id-first", None)],
            &[("id-middle", None), ("id-last", None)],
            &[("id-email", None), ("id-phone", Some(170.))],
            &[("id-username", None), ("id-company", None)],
            &[("id-address1", None)],
            &[("id-address2", None), ("id-address3", None)],
            &[
                ("id-city", None),
                ("id-state", Some(64.)),
                ("id-postal", Some(112.)),
            ],
            &[("id-country", None)],
            &[
                ("id-ssn", None),
                ("id-passport", None),
                ("id-license", None),
            ],
        ],
        ItemKind::SshKey(_) => &[&[("ssh-public", None)], &[("ssh-fingerprint", None)]],
        _ => &[],
    }
}

impl ItemEditor {
    pub fn edit(
        item: &Item,
        places: Places,
        generator: PasswordOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::build(item.clone(), false, places, generator, window, cx)
    }

    /// Item novo. `owner`: organização (e coleção) já escolhidas pelo filtro atual.
    pub fn create(
        kind: NewKind,
        folder_id: Option<String>,
        owner: Option<(String, Option<String>)>,
        places: Places,
        generator: PasswordOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (organization_id, collection_ids) = match owner {
            Some((org, col)) => {
                let cols = match col {
                    Some(c) => vec![c],
                    None => places
                        .writable(&org)
                        .first()
                        .map(|c| vec![c.id.clone()])
                        .unwrap_or_default(),
                };
                (Some(org), cols)
            }
            None => (None, Vec::new()),
        };
        let item = Item {
            id: String::new(),
            organization_id,
            folder_id,
            name: String::new(),
            notes: None,
            favorite: false,
            reprompt: false,
            deleted: false,
            revision_date: None,
            attachments: Vec::new(),
            collection_ids,
            fields: Vec::new(),
            kind: kind.empty(),
        };
        Self::build(item, true, places, generator, window, cx)
    }

    fn build(
        base: Item,
        is_new: bool,
        places: Places,
        generator: PasswordOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Nome do item")
                .default_value(base.name.clone())
        });
        let notes_text = base
            .notes
            .as_ref()
            .map(|s| s.expose().to_owned())
            .unwrap_or_default();
        let notes = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(3, 12)
                .placeholder("Notas")
                .default_value(notes_text)
        });
        let mut lines = Vec::new();
        let mut uris = Vec::new();
        let mut ssh_private = None;
        {
            let mut line = |key,
                            label,
                            value: Option<&str>,
                            secret,
                            window: &mut Window,
                            cx: &mut Context<Self>| {
                lines.push(Line {
                    key,
                    label,
                    state: input_with(window, cx, value, secret, placeholder_for(key)),
                    secret,
                });
            };
            match &base.kind {
                ItemKind::Login(l) => {
                    line(
                        "username",
                        "Usuário",
                        l.username.as_deref(),
                        false,
                        window,
                        cx,
                    );
                    line(
                        "password",
                        "Senha",
                        l.password.as_ref().map(|s| s.expose()),
                        true,
                        window,
                        cx,
                    );
                    line(
                        "totp",
                        "Autenticador (TOTP)",
                        l.totp.as_ref().map(|s| s.expose()),
                        true,
                        window,
                        cx,
                    );
                    for u in &l.uris {
                        uris.push(input_with(window, cx, Some(u), false, URI_PLACEHOLDER));
                    }
                    if uris.is_empty() {
                        uris.push(input_with(window, cx, None, false, URI_PLACEHOLDER));
                    }
                }
                ItemKind::Card(c) => {
                    line(
                        "card-holder",
                        "Nome no cartão",
                        c.cardholder_name.as_deref(),
                        false,
                        window,
                        cx,
                    );
                    line(
                        "card-brand",
                        "Bandeira",
                        c.brand.as_deref(),
                        false,
                        window,
                        cx,
                    );
                    line(
                        "card-number",
                        "Número",
                        c.number.as_ref().map(|s| s.expose()),
                        true,
                        window,
                        cx,
                    );
                    line(
                        "card-month",
                        "Mês",
                        c.exp_month.as_deref(),
                        false,
                        window,
                        cx,
                    );
                    line("card-year", "Ano", c.exp_year.as_deref(), false, window, cx);
                    line(
                        "card-code",
                        "CVV",
                        c.code.as_ref().map(|s| s.expose()),
                        true,
                        window,
                        cx,
                    );
                }
                ItemKind::Identity(i) => {
                    let s = |v: &Option<Secret>| v.as_ref().map(|s| s.expose().to_owned());
                    let pairs: [(&'static str, &'static str, Option<String>, bool); 18] = [
                        ("id-title", "Tratamento", i.title.clone(), false),
                        ("id-first", "Nome", i.first_name.clone(), false),
                        ("id-middle", "Nome do meio", i.middle_name.clone(), false),
                        ("id-last", "Sobrenome", i.last_name.clone(), false),
                        ("id-company", "Empresa", i.company.clone(), false),
                        ("id-email", "E-mail", i.email.clone(), false),
                        ("id-phone", "Telefone", i.phone.clone(), false),
                        ("id-username", "Usuário", i.username.clone(), false),
                        ("id-address1", "Endereço", i.address1.clone(), false),
                        ("id-address2", "Complemento", i.address2.clone(), false),
                        ("id-address3", "Complemento 2", i.address3.clone(), false),
                        ("id-city", "Cidade", i.city.clone(), false),
                        ("id-state", "Estado", i.state.clone(), false),
                        ("id-postal", "CEP", i.postal_code.clone(), false),
                        ("id-country", "País", i.country.clone(), false),
                        ("id-ssn", "CPF / SSN", s(&i.ssn), true),
                        ("id-passport", "Passaporte", s(&i.passport_number), true),
                        ("id-license", "CNH", s(&i.license_number), true),
                    ];
                    for (key, label, value, secret) in pairs {
                        line(key, label, value.as_deref(), secret, window, cx);
                    }
                }
                ItemKind::SshKey(k) => {
                    line(
                        "ssh-public",
                        "Chave pública",
                        k.public_key.as_deref(),
                        false,
                        window,
                        cx,
                    );
                    line(
                        "ssh-fingerprint",
                        "Impressão digital",
                        k.fingerprint.as_deref(),
                        false,
                        window,
                        cx,
                    );
                    let private = k
                        .private_key
                        .as_ref()
                        .map(|s| s.expose().to_owned())
                        .unwrap_or_default();
                    ssh_private = Some(cx.new(|cx| {
                        TextareaState::new(window, cx)
                            .auto_grow(4, 12)
                            .placeholder("Chave privada")
                            .default_value(private)
                    }));
                }
                ItemKind::SecureNote | ItemKind::Unknown(_) => {}
            }
        }
        let fields = base
            .fields
            .iter()
            .map(|f| CustomField {
                kind: f.kind,
                name: input_with(window, cx, f.name.as_deref(), false, FIELD_NAME_PLACEHOLDER),
                value: input_with(
                    window,
                    cx,
                    f.value.as_ref().map(|s| s.expose()),
                    f.kind == FieldKind::Hidden,
                    field_value_placeholder(f.kind),
                ),
            })
            .collect();
        name.update(cx, |n, cx| n.focus(window, cx));
        Self {
            owner: base.organization_id.clone(),
            collection_ids: base.collection_ids.clone(),
            folder_id: base.folder_id.clone(),
            favorite: base.favorite,
            base,
            is_new,
            name,
            lines,
            uris,
            fields,
            notes,
            ssh_private,
            places,
            generator,
            saving: false,
            error: None,
            files: Vec::new(),
        }
    }

    pub fn is_new(&self) -> bool {
        self.is_new
    }

    pub fn is_saving(&self) -> bool {
        self.saving
    }

    pub fn set_saving(&mut self, saving: bool, error: Option<String>, cx: &mut Context<Self>) {
        self.saving = saving;
        self.error = error.map(Into::into);
        cx.notify();
    }

    /// Troca o tipo de um item novo (só antes de salvar).
    pub fn set_kind(&mut self, kind: NewKind, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_new {
            return;
        }
        let mut draft = self.to_item(cx);
        draft.kind = kind.empty();
        let files = std::mem::take(&mut self.files);
        *self = Self::build(draft, true, self.places.clone(), self.generator, window, cx);
        self.files = files;
        cx.notify();
    }

    fn text(&self, key: &str, cx: &App) -> Option<String> {
        let v = self
            .lines
            .iter()
            .find(|l| l.key == key)?
            .state
            .read(cx)
            .value()
            .trim()
            .to_owned();
        (!v.is_empty()).then_some(v)
    }

    fn secret(&self, key: &str, cx: &App) -> Option<Secret> {
        // Senhas não são aparadas: espaço pode fazer parte delas.
        let v = self
            .lines
            .iter()
            .find(|l| l.key == key)?
            .state
            .read(cx)
            .value()
            .to_string();
        (!v.is_empty()).then(|| Secret::new(v))
    }

    /// O item como está no formulário.
    pub fn to_item(&self, cx: &App) -> Item {
        let mut item = self.base.clone();
        item.name = self.name.read(cx).value().trim().to_owned();
        let notes = self.notes.read(cx).value().to_string();
        item.notes = (!notes.trim().is_empty()).then(|| Secret::new(notes));
        item.folder_id = self.folder_id.clone();
        item.organization_id = self.owner.clone();
        item.collection_ids = if self.owner.is_some() {
            self.collection_ids.clone()
        } else {
            Vec::new()
        };
        item.favorite = self.favorite;
        item.fields = self
            .fields
            .iter()
            .filter_map(|f| {
                let name = f.name.read(cx).value().trim().to_owned();
                let value = f.value.read(cx).value().to_string();
                (!name.is_empty() || !value.is_empty()).then(|| Field {
                    kind: f.kind,
                    name: (!name.is_empty()).then_some(name),
                    value: (!value.is_empty()).then(|| Secret::new(value)),
                })
            })
            .collect();
        item.kind = match &self.base.kind {
            ItemKind::Login(_) => ItemKind::Login(Login {
                username: self.text("username", cx),
                password: self.secret("password", cx),
                totp: self.secret("totp", cx),
                uris: self
                    .uris
                    .iter()
                    .map(|u| u.read(cx).value().trim().to_owned())
                    .filter(|u| !u.is_empty())
                    .collect(),
            }),
            ItemKind::Card(_) => ItemKind::Card(Card {
                cardholder_name: self.text("card-holder", cx),
                brand: self.text("card-brand", cx),
                number: self.secret("card-number", cx),
                exp_month: self.text("card-month", cx),
                exp_year: self.text("card-year", cx),
                code: self.secret("card-code", cx),
            }),
            ItemKind::Identity(_) => ItemKind::Identity(Box::new(Identity {
                title: self.text("id-title", cx),
                first_name: self.text("id-first", cx),
                middle_name: self.text("id-middle", cx),
                last_name: self.text("id-last", cx),
                company: self.text("id-company", cx),
                email: self.text("id-email", cx),
                phone: self.text("id-phone", cx),
                username: self.text("id-username", cx),
                address1: self.text("id-address1", cx),
                address2: self.text("id-address2", cx),
                address3: self.text("id-address3", cx),
                city: self.text("id-city", cx),
                state: self.text("id-state", cx),
                postal_code: self.text("id-postal", cx),
                country: self.text("id-country", cx),
                ssn: self.secret("id-ssn", cx),
                passport_number: self.secret("id-passport", cx),
                license_number: self.secret("id-license", cx),
            })),
            ItemKind::SshKey(_) => ItemKind::SshKey(SshKey {
                public_key: self.text("ssh-public", cx),
                fingerprint: self.text("ssh-fingerprint", cx),
                private_key: self.ssh_private.as_ref().and_then(|p| {
                    let v = p.read(cx).value().to_string();
                    (!v.trim().is_empty()).then(|| Secret::new(v))
                }),
            }),
            other => other.clone(),
        };
        item
    }

    pub fn save(&mut self, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let item = self.to_item(cx);
        if item.organization_id.is_some() && item.collection_ids.is_empty() {
            self.error = Some("Escolha pelo menos uma coleção da organização.".into());
            cx.notify();
            return;
        }
        if item.name.is_empty() {
            self.error = Some("Dê um nome ao item.".into());
            cx.notify();
            return;
        }
        self.error = None;
        cx.emit(EditorEvent::Save(Box::new(item), self.files.clone()));
    }

    fn generate_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pw = generator::password(&self.generator);
        if let Some(line) = self.lines.iter().find(|l| l.key == "password") {
            let pw = pw.to_string();
            line.state.update(cx, |s, cx| s.set_value(pw, window, cx));
        }
    }

    /// Arquivos que serão anexados ao salvar.
    pub fn pending_files(&self) -> Vec<String> {
        self.files.iter().map(|f| f.name.clone()).collect()
    }

    /// Lê os arquivos e põe na lista para anexar (erros viram toast).
    pub fn add_paths(&mut self, paths: &[std::path::PathBuf], cx: &mut Context<Self>) {
        for path in paths {
            if path.is_dir() {
                continue;
            }
            match NewFile::read(path) {
                Ok(file) => self.files.push(file),
                Err(e) => crate::toast::error(e, cx),
            }
        }
        cx.notify();
    }

    fn pick_files(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Anexar".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await {
                this.update(cx, |this, cx| this.add_paths(&paths, cx)).ok();
            }
        })
        .detach();
    }

    fn render_files(&self, busy: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let mut list = v_flex().gap_1p5();
        // Os que já estão no item (gerenciados no detalhe).
        for a in &self.base.attachments {
            list = list.child(
                h_flex()
                    .gap_2()
                    .text_sm()
                    .text_color(muted)
                    .child(Icon::new(IconName::File).small())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(a.file_name.clone()),
                    )
                    .child(div().text_xs().child("já anexado")),
            );
        }
        for (ix, f) in self.files.iter().enumerate() {
            list = list.child(crate::motion::rise(
                h_flex()
                    .gap_2()
                    .text_sm()
                    .child(
                        Icon::new(IconName::File)
                            .small()
                            .text_color(crate::appearance::accent(cx)),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("editor-file-{ix}")))
                            .test_support()
                            .aria_label(SharedString::from(f.name.clone()))
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(f.name.clone()),
                    )
                    .child(div().text_xs().text_color(muted).child(
                        scout_vault_core::attachment::human_size(f.contents.len() as u64),
                    ))
                    .child(
                        Button::new(SharedString::from(format!("editor-file-remove-{ix}")))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Close)
                            .tooltip("Tirar da lista")
                            .disabled(busy)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if ix < this.files.len() {
                                    this.files.remove(ix);
                                }
                                cx.notify();
                            })),
                    ),
                crate::motion::key("arquivo-novo", format!("{ix}:{}", f.name)),
                4.,
            ));
        }
        v_flex().gap_2().child(list).child(ui::drop_zone(
            "editor-attach-zone",
            busy,
            cx.listener(|this, _, _, cx| {
                if !this.saving {
                    this.pick_files(cx)
                }
            }),
            cx.listener(|this, paths: &ExternalPaths, _, cx| this.add_paths(paths.paths(), cx)),
            cx,
        ))
    }

    fn add_uri(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.uris
            .push(input_with(window, cx, None, false, URI_PLACEHOLDER));
        cx.notify();
    }

    fn add_field(&mut self, hidden: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.fields.push(CustomField {
            kind: if hidden {
                FieldKind::Hidden
            } else {
                FieldKind::Text
            },
            name: input_with(window, cx, None, false, FIELD_NAME_PLACEHOLDER),
            value: input_with(
                window,
                cx,
                None,
                hidden,
                if hidden { "Valor oculto" } else { "Valor" },
            ),
        });
        cx.notify();
    }

    fn labeled(label: &'static str, control: impl IntoElement, cx: &App) -> Div {
        v_flex()
            .gap_1p5()
            .child(
                div()
                    .text_sm()
                    .font_medium()
                    .text_color(cx.theme().foreground.opacity(0.8))
                    .child(label),
            )
            .child(control)
    }

    /// Campo de uma linha do formulário (com o botão de gerar, na senha).
    fn render_line(&self, line: &Line, busy: bool, cx: &mut Context<Self>) -> Div {
        let mut control = h_flex().gap_1().child(
            div().flex_1().min_w_0().child(
                Input::new(&line.state)
                    .id(SharedString::from(format!("editor-{}", line.key)))
                    .when(line.secret, |i| i.mask_toggle())
                    .disabled(busy),
            ),
        );
        if line.key == "password" {
            control = control.child(
                Button::new("editor-generate")
                    .ghost()
                    .small()
                    .icon(IconName::Dices)
                    .tooltip("Gerar senha")
                    .disabled(busy)
                    .on_click(
                        cx.listener(|this, _, window, cx| this.generate_password(window, cx)),
                    ),
            );
        }
        Self::labeled(line.label, control, cx)
    }

    /// Organização escolhida para o item (para os testes).
    pub fn owner(&self) -> Option<&str> {
        self.owner.as_deref()
    }

    pub fn collections(&self) -> &[String] {
        &self.collection_ids
    }

    /// Escolhe o dono de um item novo: o cofre pessoal ou uma organização
    /// (já com a primeira coleção em que dá para escrever).
    pub fn set_owner(&mut self, org: Option<String>, cx: &mut Context<Self>) {
        if !self.is_new {
            return;
        }
        self.collection_ids = match &org {
            Some(o) => self
                .places
                .writable(o)
                .first()
                .map(|c| vec![c.id.clone()])
                .unwrap_or_default(),
            None => Vec::new(),
        };
        self.owner = org;
        cx.notify();
    }

    pub fn toggle_collection(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(pos) = self.collection_ids.iter().position(|c| c == id) {
            self.collection_ids.remove(pos);
        } else {
            self.collection_ids.push(id.to_owned());
        }
        cx.notify();
    }

    /// Botão-etiqueta que acende quando escolhido.
    fn chip(
        id: SharedString,
        label: String,
        selected: bool,
        locked: bool,
        on_click: impl Fn(&mut Self, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let accent = crate::appearance::accent(cx);
        let tint = crate::appearance::accent_tint(cx, 0.1);
        let chip_id = id.clone();
        div()
            .id(id)
            .test_support()
            .role(Role::Button)
            .aria_label(SharedString::from(label.clone()))
            .flex()
            .items_center()
            .gap_1()
            .px_2p5()
            .py_1()
            .rounded(cx.theme().radius)
            .border_1()
            .text_sm()
            .when(!locked, |d| d.cursor_pointer().active(|el| el.opacity(0.7)))
            .when(locked, |d| d.opacity(0.6))
            .when(selected, |d| {
                d.border_color(accent).text_color(accent).bg(tint)
            })
            .when(!selected, |d| d.border_color(cx.theme().border))
            .when(locked, |d| d.child(Icon::new(IconName::Lock).xsmall()))
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                if !locked {
                    on_click(this, cx)
                }
            }))
            .with_animation(
                crate::motion::key("chip", format!("{chip_id}:{selected}")),
                Animation::new(crate::motion::NORMAL).with_easing(ease_out_quint()),
                move |el, t| {
                    if selected {
                        el.bg(tint.opacity(tint.a * t))
                    } else {
                        el
                    }
                },
            )
    }

    /// Dono (cofre pessoal ou organização) e coleções do item.
    fn render_owner(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let orgs: Vec<(String, String)> = self
            .places
            .orgs
            .iter()
            .filter(|(id, _)| {
                !self.places.writable(id).is_empty() || self.owner.as_deref() == Some(id)
            })
            .cloned()
            .collect();
        if orgs.is_empty() && self.owner.is_none() {
            return None;
        }
        let mut col = v_flex().gap_2();
        if self.is_new {
            let mut row = h_flex().flex_wrap().gap_2().child(Self::chip(
                "editor-owner-personal".into(),
                "Meu cofre".into(),
                self.owner.is_none(),
                false,
                |this, cx| this.set_owner(None, cx),
                cx,
            ));
            for (id, name) in orgs {
                let selected = self.owner.as_deref() == Some(id.as_str());
                let org = id.clone();
                row = row.child(Self::chip(
                    format!("editor-owner-{id}").into(),
                    name,
                    selected,
                    false,
                    move |this, cx| this.set_owner(Some(org.clone()), cx),
                    cx,
                ));
            }
            col = col.child(row);
        } else if let Some(org) = &self.owner {
            let name = self
                .places
                .orgs
                .iter()
                .find(|(id, _)| id == org)
                .map(|(_, n)| n.clone())
                .unwrap_or_default();
            col = col.child(
                h_flex()
                    .gap_1p5()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(Icon::new(IconName::Building2).small())
                    .child(name),
            );
        } else {
            return None;
        }
        if let Some(org) = self.owner.clone() {
            let mut cols = h_flex().flex_wrap().gap_2();
            for c in self
                .places
                .collections
                .iter()
                .filter(|c| c.organization_id == org)
            {
                let selected = self.collection_ids.contains(&c.id);
                // Só leitura: mostra se o item já está nela, mas não deixa mexer.
                if c.read_only && !selected {
                    continue;
                }
                let cid = c.id.clone();
                cols = cols.child(Self::chip(
                    format!("editor-col-{}", c.id).into(),
                    c.name.clone(),
                    selected,
                    c.read_only,
                    move |this, cx| this.toggle_collection(&cid, cx),
                    cx,
                ));
            }
            col = col.child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Coleções (quem tem acesso a elas vê o item)"),
                    )
                    .child(cols),
            );
        }
        Some(col.into_any_element())
    }

    fn render_folders(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let chip = |id: SharedString,
                    label: String,
                    folder: Option<String>,
                    selected: bool,
                    cx: &mut Context<Self>| {
            let chip_id = id.clone();
            let tint = crate::appearance::accent_tint(cx, 0.1);
            div()
                .id(id)
                .test_support()
                .role(Role::Button)
                .aria_label(SharedString::from(label.clone()))
                .px_2p5()
                .py_1()
                .rounded(cx.theme().radius)
                .border_1()
                .text_sm()
                .cursor_pointer()
                .active(|el| el.opacity(0.7))
                .when(selected, |d| {
                    d.border_color(crate::appearance::accent(cx))
                        .text_color(crate::appearance::accent(cx))
                        .bg(crate::appearance::accent_tint(cx, 0.1))
                })
                .when(!selected, |d| d.border_color(cx.theme().border))
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.folder_id = folder.clone();
                    cx.notify();
                }))
                .with_animation(
                    crate::motion::key("pasta", format!("{chip_id}:{selected}")),
                    Animation::new(crate::motion::NORMAL).with_easing(ease_out_quint()),
                    move |el, t| {
                        if selected {
                            el.bg(tint.opacity(tint.a * t))
                        } else {
                            el
                        }
                    },
                )
        };
        let mut row = h_flex().flex_wrap().gap_2();
        row = row.child(chip(
            "editor-folder-none".into(),
            "Sem pasta".into(),
            None,
            self.folder_id.is_none(),
            cx,
        ));
        for (id, name) in self.places.folders.clone() {
            let selected = self.folder_id.as_deref() == Some(id.as_str());
            row = row.child(chip(
                format!("editor-folder-{id}").into(),
                name,
                Some(id),
                selected,
                cx,
            ));
        }
        row
    }
}

impl Render for ItemEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.saving;
        let mut form = v_flex().gap_3p5();

        let kind_selector = self.is_new.then(|| {
            let current = NewKind::ALL
                .into_iter()
                .find(|k| {
                    std::mem::discriminant(&k.empty()) == std::mem::discriminant(&self.base.kind)
                })
                .unwrap_or(NewKind::Login);
            let this = cx.entity().downgrade();
            ui::segmented_compact(
                "editor-kind",
                NewKind::ALL
                    .iter()
                    .map(|k| Segment {
                        id: format!("editor-kind-{}", k.id()).into(),
                        label: k.label().into(),
                        icon: None,
                        selected: *k == current,
                    })
                    .collect(),
                move |ix, window, cx| {
                    this.update(cx, |e, cx| e.set_kind(NewKind::ALL[ix], window, cx))
                        .ok();
                },
                cx,
            )
        });

        form = form.child(Self::labeled(
            "Nome",
            Input::new(&self.name).id("editor-name").disabled(busy),
            cx,
        ));
        // Dono logo abaixo do nome: define quem vê o item.
        if let Some(owner) = self.render_owner(cx) {
            form = form.child(Self::labeled("Dono", owner, cx));
        }

        let rows = layout(&self.base.kind);
        let mut placed: Vec<&str> = Vec::new();
        for row in rows {
            let mut line_row = h_flex().gap_3().items_start();
            for (key, width) in row.iter() {
                let Some(line) = self.lines.iter().find(|l| l.key == *key) else {
                    continue;
                };
                placed.push(key);
                let field = self.render_line(line, busy, cx);
                line_row = line_row.child(match width {
                    Some(w) => div().w(px(*w)).flex_shrink_0().child(field),
                    None => div().flex_1().min_w_0().child(field),
                });
            }
            form = form.child(line_row);
        }
        // Algum campo sem lugar no layout ainda aparece, numa linha só dele.
        for line in &self.lines {
            if !placed.contains(&line.key) {
                form = form.child(self.render_line(line, busy, cx));
            }
        }

        if matches!(self.base.kind, ItemKind::Login(_)) {
            let count = self.uris.len();
            let mut list = v_flex().gap_2();
            for (ix, uri) in self.uris.iter().enumerate() {
                list = list.child(
                    h_flex()
                        .gap_2()
                        .child(
                            div().flex_1().child(
                                Input::new(uri)
                                    .id(SharedString::from(format!("editor-uri-{ix}")))
                                    .disabled(busy),
                            ),
                        )
                        .when(count > 1, |row| {
                            row.child(
                                Button::new(SharedString::from(format!("editor-uri-remove-{ix}")))
                                    .ghost()
                                    .small()
                                    .icon(IconName::Close)
                                    .tooltip("Remover endereço")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if ix < this.uris.len() {
                                            this.uris.remove(ix);
                                        }
                                        cx.notify();
                                    })),
                            )
                        }),
                );
            }
            list = list.child(
                Button::new("editor-uri-add")
                    .ghost()
                    .small()
                    .icon(IconName::Plus)
                    .label("Adicionar endereço")
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, window, cx| this.add_uri(window, cx))),
            );
            form = form.child(Self::labeled("Sites (https://…)", list, cx));
        }

        if let Some(private) = &self.ssh_private {
            form = form.child(Self::labeled(
                "Chave privada",
                Textarea::new(private).disabled(busy),
                cx,
            ));
        }

        // Campos personalizados.
        let mut custom = v_flex().gap_2();
        for (ix, f) in self.fields.iter().enumerate() {
            custom = custom.child(
                h_flex()
                    .gap_2()
                    .child(
                        div().w(px(160.)).child(
                            Input::new(&f.name)
                                .id(SharedString::from(format!("editor-field-name-{ix}")))
                                .disabled(busy),
                        ),
                    )
                    .child(
                        div().flex_1().child(
                            Input::new(&f.value)
                                .id(SharedString::from(format!("editor-field-value-{ix}")))
                                .when(f.kind == FieldKind::Hidden, |i| i.mask_toggle())
                                .disabled(busy),
                        ),
                    )
                    .child(
                        Button::new(SharedString::from(format!("editor-field-remove-{ix}")))
                            .ghost()
                            .small()
                            .icon(IconName::Close)
                            .tooltip("Remover campo")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if ix < this.fields.len() {
                                    this.fields.remove(ix);
                                }
                                cx.notify();
                            })),
                    ),
            );
        }
        custom = custom.child(
            h_flex()
                .gap_2()
                .child(
                    Button::new("editor-field-add")
                        .ghost()
                        .small()
                        .icon(IconName::Plus)
                        .label("Campo de texto")
                        .disabled(busy)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.add_field(false, window, cx)),
                        ),
                )
                .child(
                    Button::new("editor-field-add-hidden")
                        .ghost()
                        .small()
                        .icon(IconName::EyeOff)
                        .label("Campo oculto")
                        .disabled(busy)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.add_field(true, window, cx)),
                        ),
                ),
        );
        form = form
            .child(Self::labeled("Campos personalizados", custom, cx))
            .child(Self::labeled(
                "Notas",
                Textarea::new(&self.notes).disabled(busy),
                cx,
            ))
            .child(Self::labeled("Anexos", self.render_files(busy, cx), cx))
            .child(Self::labeled("Pasta", self.render_folders(cx), cx))
            .child(
                Checkbox::new("editor-favorite")
                    .label("Favorito")
                    .checked(self.favorite)
                    .disabled(busy)
                    .on_change(cx.listener(|this, v: &bool, _, cx| {
                        this.favorite = *v;
                        cx.notify();
                    })),
            );

        let title = if self.is_new {
            "Novo item"
        } else {
            "Editar item"
        };
        // Barra fixa no topo: título, erro e os botões sempre visíveis.
        let bar = h_flex()
            .gap_2()
            .items_center()
            .px_6()
            .py_3()
            .border_b_1()
            .border_color(cx.theme().border)
            // Item novo: o seletor de tipo fica centralizado no espaço livre da
            // barra (entre a borda e os botões). Edição: título à esquerda.
            .map(|bar| match kind_selector {
                Some(selector) => bar
                    .child(div().flex_1())
                    .child(selector)
                    .child(div().flex_1()),
                None => bar
                    .child(div().text_lg().font_semibold().child(title))
                    .child(div().flex_1()),
            })
            .child(
                Button::new("editor-cancel")
                    .ghost()
                    .small()
                    .label("Cancelar")
                    .disabled(busy)
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(EditorEvent::Cancel))),
            )
            .child(
                Button::new("editor-save")
                    .primary()
                    .small()
                    .label(if busy { "Salvando…" } else { "Salvar" })
                    .loading(busy)
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
            );
        // Erro logo abaixo da barra, onde há espaço para a frase inteira.
        let status = div()
            .id("editor-status")
            .test_support()
            .role(Role::Status)
            .aria_label(self.error.clone().unwrap_or_default())
            .min_w_0()
            .text_sm()
            .text_color(cx.theme().danger)
            .children(self.error.clone().map(|e| {
                crate::motion::shake(div().child(e.clone()), crate::motion::key("erro", &e))
            }));

        v_flex().flex_1().min_w_0().h_full().child(bar).child(
            div()
                .id("item-editor")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .child(
                    v_flex()
                        .w_full()
                        .max_w(px(720.))
                        .px_6()
                        .py_5()
                        .gap_2()
                        .child(status)
                        .child(form),
                ),
        )
    }
}
