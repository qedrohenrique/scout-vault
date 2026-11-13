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

use crate::ui::{self, Segment};

pub enum EditorEvent {
    Save(Box<Item>),
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
    folders: Vec<(String, String)>,
    generator: PasswordOptions,
    saving: bool,
    error: Option<SharedString>,
}

impl EventEmitter<EditorEvent> for ItemEditor {}

fn input(
    window: &mut Window,
    cx: &mut App,
    value: Option<&str>,
    secret: bool,
) -> Entity<InputState> {
    let value = value.unwrap_or_default().to_owned();
    cx.new(|cx| {
        InputState::new(window, cx)
            .masked(secret)
            .default_value(value)
    })
}

impl ItemEditor {
    pub fn edit(
        item: &Item,
        folders: Vec<(String, String)>,
        generator: PasswordOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::build(item.clone(), false, folders, generator, window, cx)
    }

    pub fn create(
        kind: NewKind,
        folder_id: Option<String>,
        folders: Vec<(String, String)>,
        generator: PasswordOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let item = Item {
            id: String::new(),
            organization_id: None,
            folder_id,
            name: String::new(),
            notes: None,
            favorite: false,
            reprompt: false,
            deleted: false,
            revision_date: None,
            attachments: Vec::new(),
            fields: Vec::new(),
            kind: kind.empty(),
        };
        Self::build(item, true, folders, generator, window, cx)
    }

    fn build(
        base: Item,
        is_new: bool,
        folders: Vec<(String, String)>,
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
                    state: input(window, cx, value, secret),
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
                        "Chave do autenticador (TOTP)",
                        l.totp.as_ref().map(|s| s.expose()),
                        true,
                        window,
                        cx,
                    );
                    for u in &l.uris {
                        uris.push(input(window, cx, Some(u), false));
                    }
                    if uris.is_empty() {
                        uris.push(input(window, cx, None, false));
                    }
                }
                ItemKind::Card(c) => {
                    line(
                        "card-holder",
                        "Titular",
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
                        "Mês de validade",
                        c.exp_month.as_deref(),
                        false,
                        window,
                        cx,
                    );
                    line(
                        "card-year",
                        "Ano de validade",
                        c.exp_year.as_deref(),
                        false,
                        window,
                        cx,
                    );
                    line(
                        "card-code",
                        "Código de segurança",
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
                name: input(window, cx, f.name.as_deref(), false),
                value: input(
                    window,
                    cx,
                    f.value.as_ref().map(|s| s.expose()),
                    f.kind == FieldKind::Hidden,
                ),
            })
            .collect();
        name.update(cx, |n, cx| n.focus(window, cx));
        Self {
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
            folders,
            generator,
            saving: false,
            error: None,
        }
    }

    pub fn is_new(&self) -> bool {
        self.is_new
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
        *self = Self::build(
            draft,
            true,
            self.folders.clone(),
            self.generator,
            window,
            cx,
        );
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
        if item.name.is_empty() {
            self.error = Some("Dê um nome ao item.".into());
            cx.notify();
            return;
        }
        self.error = None;
        cx.emit(EditorEvent::Save(Box::new(item)));
    }

    fn generate_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pw = generator::password(&self.generator);
        if let Some(line) = self.lines.iter().find(|l| l.key == "password") {
            let pw = pw.to_string();
            line.state.update(cx, |s, cx| s.set_value(pw, window, cx));
        }
    }

    fn add_uri(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.uris.push(input(window, cx, None, false));
        cx.notify();
    }

    fn add_field(&mut self, hidden: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.fields.push(CustomField {
            kind: if hidden {
                FieldKind::Hidden
            } else {
                FieldKind::Text
            },
            name: input(window, cx, None, false),
            value: input(window, cx, None, hidden),
        });
        cx.notify();
    }

    fn labeled(label: &'static str, control: impl IntoElement, cx: &App) -> Div {
        v_flex()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(label),
            )
            .child(control)
    }

    fn render_folders(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let chip = |id: SharedString,
                    label: String,
                    folder: Option<String>,
                    selected: bool,
                    cx: &mut Context<Self>| {
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
        };
        let mut row = h_flex().flex_wrap().gap_2();
        row = row.child(chip(
            "editor-folder-none".into(),
            "Sem pasta".into(),
            None,
            self.folder_id.is_none(),
            cx,
        ));
        for (id, name) in self.folders.clone() {
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
        let mut form = v_flex().gap_4();

        if self.is_new {
            let current = NewKind::ALL
                .into_iter()
                .find(|k| {
                    std::mem::discriminant(&k.empty()) == std::mem::discriminant(&self.base.kind)
                })
                .unwrap_or(NewKind::Login);
            let this = cx.entity().downgrade();
            form = form.child(ui::segmented(
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
            ));
        }

        form = form.child(Self::labeled(
            "Nome",
            Input::new(&self.name).id("editor-name").disabled(busy),
            cx,
        ));

        for line in &self.lines {
            let mut control = h_flex().gap_2().child(
                div().flex_1().child(
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
            form = form.child(Self::labeled(line.label, control, cx));
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
            .px_8()
            .py_3()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(div().text_lg().font_semibold().child(title))
            .child(
                div()
                    .id("editor-status")
                    .test_support()
                    .role(Role::Status)
                    .aria_label(self.error.clone().unwrap_or_default())
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .text_color(cx.theme().danger)
                    .children(self.error.clone()),
            )
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

        v_flex().flex_1().min_w_0().h_full().child(bar).child(
            div()
                .id("item-editor")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .child(v_flex().w_full().max_w(px(720.)).px_8().py_6().child(form)),
        )
    }
}
