//! Cofre aberto: filtros à esquerda, lista no meio, detalhe do item à direita.

use std::borrow::Cow;
use std::collections::HashSet;
use std::sync::Arc;

use cofre_core::vault::{FieldKind, Item, ItemKind, Keys};
use gpui_kit::component::list::{List, ListEvent, ListState};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{button::*, *};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::flow::{self, Session, Unlocked};
use crate::item_list::{kind_icon, Filter, ItemList, KindFilter};
use crate::{appearance, clipboard, ui};
use gpui_kit::assets::IconName;

pub enum VaultEvent {
    /// Pede para bloquear; leva a sessão (com tokens possivelmente renovados).
    Lock(Session),
    OpenSettings,
}

/// Um campo exibível do item selecionado.
struct FieldRow<'a> {
    key: String,
    label: String,
    value: Cow<'a, str>,
    secret: bool,
    multiline: bool,
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
        });
    }
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
        Self {
            session,
            keys: Arc::new(keys),
            list,
            selected: None,
            revealed: HashSet::new(),
            syncing: false,
            status: (errors > 0)
                .then(|| format!("{errors} item(ns) não puderam ser decifrados.").into()),
            _subscriptions: vec![sub],
        }
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

    fn copy_field(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = &self.selected else { return };
        let Some(row) = field_rows(item).into_iter().find(|r| r.key == key) else {
            return;
        };
        clipboard::copy_and_clear_later(&row.value, cx);
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
        if self.syncing {
            return;
        }
        self.syncing = true;
        self.status = Some("Sincronizando…".into());
        cx.notify();
        let work = flow::resync(self.session.clone(), self.keys.clone());
        cx.spawn(async move |this, cx| {
            let result = work.await;
            this.update(cx, |this, cx| {
                this.syncing = false;
                match result {
                    Ok((session, vault)) => {
                        let errors = vault.errors.len();
                        this.session = session;
                        let selected_id = this.selected.as_ref().map(|i| i.id.clone());
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
                        this.status = Some(if errors > 0 {
                            format!("Sincronizado; {errors} item(ns) não puderam ser decifrados.")
                                .into()
                        } else {
                            "Sincronizado.".into()
                        });
                    }
                    Err(e) => this.status = Some(e.into()),
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
                            .tooltip("Bloquear")
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
                    ),
            )
            .child(div().flex_1().min_h_0().child(List::new(&self.list)))
    }

    fn render_detail(&self, cx: &mut Context<Self>) -> AnyElement {
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
            );

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
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(row.label.clone()),
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
                    .when(total > 0, |el| el.child(fields)),
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
