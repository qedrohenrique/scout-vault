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

use crate::clipboard;
use crate::flow::{self, Session, Unlocked};
use crate::item_list::{kind_icon, Filter, ItemList, KindFilter};

pub enum VaultEvent {
    /// Pede para bloquear; leva a sessão (com tokens possivelmente renovados).
    Lock(Session),
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

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let list = self.list.read(cx).delegate();
        let current = list.filter().clone();

        let mut entries: Vec<(SharedString, Filter, gpui_kit::assets::IconName, String)> = vec![
            (
                "filter-all".into(),
                Filter::All,
                gpui_kit::assets::IconName::LayoutGrid,
                "Todos os itens".into(),
            ),
            (
                "filter-favorites".into(),
                Filter::Favorites,
                gpui_kit::assets::IconName::Star,
                "Favoritos".into(),
            ),
        ];
        for k in KindFilter::ALL {
            let icon = match k {
                KindFilter::Login => gpui_kit::assets::IconName::Globe,
                KindFilter::Card => gpui_kit::assets::IconName::CreditCard,
                KindFilter::Identity => gpui_kit::assets::IconName::IdCard,
                KindFilter::Note => gpui_kit::assets::IconName::StickyNote,
                KindFilter::SshKey => gpui_kit::assets::IconName::KeyRound,
            };
            entries.push((
                format!("filter-{}", k.id()).into(),
                Filter::Kind(k),
                icon,
                k.label().into(),
            ));
        }
        let vault = list.vault();
        for org in &vault.organizations {
            entries.push((
                format!("filter-org-{}", org.id).into(),
                Filter::Organization(org.id.clone()),
                gpui_kit::assets::IconName::Building2,
                org.name.clone(),
            ));
        }
        for folder in &vault.folders {
            entries.push((
                format!("filter-folder-{}", folder.id).into(),
                Filter::Folder(Some(folder.id.clone())),
                gpui_kit::assets::IconName::Folder,
                folder.name.clone(),
            ));
        }
        entries.push((
            "filter-no-folder".into(),
            Filter::Folder(None),
            gpui_kit::assets::IconName::FolderOpen,
            "Sem pasta".into(),
        ));
        entries.push((
            "filter-trash".into(),
            Filter::Trash,
            gpui_kit::assets::IconName::Trash,
            "Lixeira".into(),
        ));

        let rows: Vec<_> = entries
            .into_iter()
            .map(|(id, filter, icon, label)| {
                let count = list.count(&filter);
                let selected = filter == current;
                // O Button centraliza o conteúdo; um filho de largura total
                // alinha ícone e nome à esquerda e a contagem à direita.
                Button::new(id)
                    .ghost()
                    .small()
                    .w_full()
                    .selected(selected)
                    .child(
                        h_flex()
                            .w_full()
                            .gap_2()
                            .child(Icon::new(icon).small())
                            .child(div().flex_1().min_w_0().truncate().child(label))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(count.to_string()),
                            ),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_filter(filter.clone(), window, cx)
                    }))
            })
            .collect();

        v_flex()
            .w(px(230.))
            .h_full()
            .flex_shrink_0()
            .p_2()
            .gap_1()
            .border_r_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().sidebar)
            .child(
                v_flex()
                    .id("vault-filters")
                    .flex_1()
                    .min_h_0()
                    .gap_0p5()
                    .overflow_y_scroll()
                    .children(rows),
            )
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
                        Button::new("vault-lock")
                            .ghost()
                            .small()
                            .icon(gpui_kit::assets::IconName::Lock)
                            .label("Bloquear")
                            .on_click(cx.listener(|this, _, _, cx| this.request_lock(cx))),
                    ),
            )
    }

    fn render_detail(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(item) = &self.selected else {
            return v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .text_color(cx.theme().muted_foreground)
                .child("Selecione um item.")
                .into_any_element();
        };

        let vault = self.list.read(cx).delegate().vault();
        let place = match (&item.organization_id, &item.folder_id) {
            (Some(org), _) => vault
                .organizations
                .iter()
                .find(|o| &o.id == org)
                .map(|o| o.name.clone()),
            (None, Some(folder)) => vault
                .folders
                .iter()
                .find(|f| &f.id == folder)
                .map(|f| f.name.clone()),
            (None, None) => None,
        };

        let header = h_flex()
            .gap_3()
            .child(Icon::new(kind_icon(&item.kind)).large())
            .child(
                v_flex()
                    .min_w_0()
                    .child(
                        div()
                            .id("detail-name")
                            .test_support()
                            .aria_label(SharedString::from(item.name.clone()))
                            .text_xl()
                            .font_semibold()
                            .truncate()
                            .child(item.name.clone()),
                    )
                    .children(place.map(|p| {
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(p)
                    })),
            );

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

        let rows = field_rows(item).into_iter().map(|row| {
            let revealed = !row.secret || self.revealed.contains(&row.key);
            let shown: SharedString = if revealed {
                row.value.to_string().into()
            } else {
                "••••••••".into()
            };
            let copy_key = row.key.clone();
            let reveal_key = row.key.clone();
            v_flex()
                .gap_1()
                .py_2()
                .border_b_1()
                .border_color(cx.theme().border)
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(row.label.clone()),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .items_start()
                        .child(
                            div()
                                .id(SharedString::from(format!("value-{}", row.key)))
                                .test_support()
                                .aria_label(shown.clone())
                                .flex_1()
                                .min_w_0()
                                .font_family(if row.secret { "monospace" } else { "" })
                                .when(!row.multiline, |d| d.truncate())
                                .when(row.multiline && revealed, |d| d.whitespace_normal())
                                .child(shown),
                        )
                        .when(row.secret, |el| {
                            el.child(
                                Button::new(SharedString::from(format!("reveal-{}", row.key)))
                                    .ghost()
                                    .xsmall()
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
                                .xsmall()
                                .icon(IconName::Copy)
                                .tooltip("Copiar")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.copy_field(&copy_key, window, cx)
                                })),
                        ),
                )
        });

        v_flex()
            .id("vault-detail")
            .flex_1()
            .min_w_0()
            .h_full()
            .p_6()
            .gap_4()
            .overflow_y_scroll()
            .child(header)
            .child(badges)
            .child(v_flex().children(rows))
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
            .child(
                v_flex()
                    .w(px(340.))
                    .h_full()
                    .flex_shrink_0()
                    .border_r_1()
                    .border_color(cx.theme().border)
                    .child(List::new(&self.list)),
            )
            .child(self.render_detail(cx))
    }
}
