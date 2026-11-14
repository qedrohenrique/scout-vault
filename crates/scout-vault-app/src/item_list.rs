//! Lista de itens do cofre: dono dos dados decifrados, do filtro e da busca.
//! A `ListState` do gpui-kit cuida da virtualização, seleção e teclado.

use gpui_kit::component::list::{ListDelegate, ListItem, ListState};
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use scout_vault_core::vault::{Item, ItemKind, Vault};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Filter {
    All,
    Favorites,
    Kind(KindFilter),
    Folder(Option<String>),
    Organization(String),
    Trash,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KindFilter {
    Login,
    Card,
    Identity,
    Note,
    SshKey,
}

impl KindFilter {
    pub const ALL: [KindFilter; 5] = [
        Self::Login,
        Self::Card,
        Self::Identity,
        Self::Note,
        Self::SshKey,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Login => "Logins",
            Self::Card => "Cartões",
            Self::Identity => "Identidades",
            Self::Note => "Notas seguras",
            Self::SshKey => "Chaves SSH",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Login => "login",
            Self::Card => "card",
            Self::Identity => "identity",
            Self::Note => "note",
            Self::SshKey => "ssh",
        }
    }

    fn matches(self, kind: &ItemKind) -> bool {
        matches!(
            (self, kind),
            (Self::Login, ItemKind::Login(_))
                | (Self::Card, ItemKind::Card(_))
                | (Self::Identity, ItemKind::Identity(_))
                | (Self::Note, ItemKind::SecureNote)
                | (Self::SshKey, ItemKind::SshKey(_))
        )
    }
}

impl Filter {
    pub fn matches(&self, item: &Item) -> bool {
        match self {
            Filter::Trash => item.deleted,
            _ if item.deleted => false,
            Filter::All => true,
            Filter::Favorites => item.favorite,
            Filter::Kind(k) => k.matches(&item.kind),
            Filter::Folder(id) => item.folder_id == *id && item.organization_id.is_none(),
            Filter::Organization(id) => item.organization_id.as_deref() == Some(id),
        }
    }
}

pub fn kind_icon(kind: &ItemKind) -> gpui_kit::assets::IconName {
    use gpui_kit::assets::IconName as I;
    match kind {
        ItemKind::Login(_) => I::Globe,
        ItemKind::Card(_) => I::CreditCard,
        ItemKind::Identity(_) => I::IdCard,
        ItemKind::SecureNote => I::StickyNote,
        ItemKind::SshKey(_) => I::KeyRound,
        ItemKind::Unknown(_) => I::FileText,
    }
}

/// Linha secundária da lista. Nunca mostra segredos inteiros.
fn subtitle(item: &Item) -> String {
    match &item.kind {
        ItemKind::Login(l) => l.username.clone().unwrap_or_default(),
        ItemKind::Card(c) => {
            let last4 = c
                .number
                .as_ref()
                .map(|n| {
                    let digits: Vec<char> =
                        n.expose().chars().filter(char::is_ascii_digit).collect();
                    digits[digits.len().saturating_sub(4)..]
                        .iter()
                        .collect::<String>()
                })
                .unwrap_or_default();
            match (&c.brand, last4.is_empty()) {
                (Some(b), false) => format!("{b}, *{last4}"),
                (None, false) => format!("*{last4}"),
                (Some(b), true) => b.clone(),
                (None, true) => String::new(),
            }
        }
        ItemKind::Identity(i) => {
            let name = [i.first_name.as_deref(), i.last_name.as_deref()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" ");
            if name.is_empty() {
                i.email.clone().unwrap_or_default()
            } else {
                name
            }
        }
        ItemKind::SshKey(k) => k.fingerprint.clone().unwrap_or_default(),
        ItemKind::SecureNote | ItemKind::Unknown(_) => String::new(),
    }
}

pub struct ItemList {
    vault: Vault,
    filter: Filter,
    query: String,
    /// Índices em `vault.items`, na ordem exibida.
    visible: Vec<usize>,
    selected: Option<IndexPath>,
}

impl ItemList {
    pub fn new(vault: Vault) -> Self {
        let mut list = Self {
            vault,
            filter: Filter::All,
            query: String::new(),
            visible: Vec::new(),
            selected: None,
        };
        list.refresh();
        list
    }

    pub fn vault(&self) -> &Vault {
        &self.vault
    }

    pub fn filter(&self) -> &Filter {
        &self.filter
    }

    /// Troca o cofre (após sincronizar) mantendo o filtro e a busca.
    pub fn set_vault(&mut self, vault: Vault) {
        self.vault = vault;
        self.selected = None;
        self.refresh();
    }

    pub fn set_filter(&mut self, filter: Filter) {
        self.filter = filter;
        self.selected = None;
        self.refresh();
    }

    pub fn count(&self, filter: &Filter) -> usize {
        self.vault
            .items
            .iter()
            .filter(|i| filter.matches(i))
            .count()
    }

    pub fn visible_items(&self) -> impl Iterator<Item = &Item> {
        self.visible.iter().map(|&ix| &self.vault.items[ix])
    }

    pub fn item_at(&self, ix: IndexPath) -> Option<&Item> {
        self.visible.get(ix.row).map(|&i| &self.vault.items[i])
    }

    pub fn position_of(&self, id: &str) -> Option<IndexPath> {
        self.visible_items()
            .position(|i| i.id == id)
            .map(IndexPath::new)
    }

    fn refresh(&mut self) {
        let filter = &self.filter;
        let query = &self.query;
        self.visible = self
            .vault
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| filter.matches(item) && item.matches(query))
            .map(|(ix, _)| ix)
            .collect();
    }
}

impl ListDelegate for ItemList {
    type Item = ListItem;

    fn perform_search(
        &mut self,
        query: &str,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Task<()> {
        self.query = query.to_owned();
        self.selected = None;
        self.refresh();
        cx.notify();
        Task::ready(())
    }

    fn items_count(&self, _section: usize, _cx: &App) -> usize {
        self.visible.len()
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let item = self.item_at(ix)?;
        let sub = subtitle(item);
        let selected = self.selected == Some(ix);
        Some(
            ListItem::new(ix).selected(selected).py_2().child(
                h_flex()
                    .relative()
                    .gap_3()
                    .w_full()
                    // Item escolhido: a seleção envolve o ícone.
                    .child({
                        let chip =
                            crate::ui::icon_chip(kind_icon(&item.kind), px(32.), selected, cx);
                        if selected {
                            crate::motion::selection_ring(
                                chip,
                                crate::motion::key("item-anel", &item.id),
                                crate::appearance::accent(cx),
                                32.,
                            )
                            .into_any_element()
                        } else {
                            chip.into_any_element()
                        }
                    })
                    .child(
                        v_flex()
                            .min_w_0()
                            .flex_1()
                            .child(
                                div()
                                    .truncate()
                                    .text_sm()
                                    .font_medium()
                                    .child(item.name.clone()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    // Altura fixa mesmo sem subtítulo: a lista exige linhas iguais.
                                    .child(if sub.is_empty() { " ".to_owned() } else { sub }),
                            ),
                    )
                    .when(item.favorite, |el| {
                        el.child(
                            Icon::new(IconName::StarFill)
                                .xsmall()
                                .text_color(crate::appearance::accent(cx)),
                        )
                    }),
            ),
        )
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_2()
            .text_color(cx.theme().muted_foreground)
            .child(Icon::new(IconName::Search))
            .child(if self.query.is_empty() {
                "Nenhum item aqui."
            } else {
                "Nada encontrado para esta busca."
            })
    }

    fn set_selected_index(
        &mut self,
        ix: Option<IndexPath>,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) {
        self.selected = ix;
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    // Sem `use super::*`: o glob do gpui_kit traria o macro `test` do GPUI,
    // que sombreia o `#[test]` do Rust.
    use super::{subtitle, Filter, ItemList, KindFilter};
    use scout_vault_core::vault::{Card, Item, ItemKind, Login, Vault};

    fn item(id: &str, name: &str, kind: ItemKind) -> Item {
        Item {
            id: id.into(),
            organization_id: None,
            folder_id: None,
            name: name.into(),
            notes: None,
            favorite: false,
            reprompt: false,
            deleted: false,
            revision_date: None,
            attachments: Vec::new(),
            fields: Vec::new(),
            kind,
        }
    }

    fn sample() -> Vault {
        let mut fav = item(
            "a",
            "Banco",
            ItemKind::Login(Login {
                username: Some("pessoa".into()),
                ..Login::default()
            }),
        );
        fav.favorite = true;
        let mut trash = item("b", "Velho", ItemKind::SecureNote);
        trash.deleted = true;
        let mut org = item("c", "Cartão", ItemKind::Card(Card::default()));
        org.organization_id = Some("org".into());
        Vault {
            items: vec![fav, trash, org],
            ..Vault::default()
        }
    }

    fn ids(list: &ItemList) -> Vec<&str> {
        list.visible_items().map(|i| i.id.as_str()).collect()
    }

    #[test]
    fn filters() {
        let mut list = ItemList::new(sample());
        assert_eq!(ids(&list), ["a", "c"], "lixeira fica fora de Todos");
        list.set_filter(Filter::Trash);
        assert_eq!(ids(&list), ["b"]);
        list.set_filter(Filter::Favorites);
        assert_eq!(ids(&list), ["a"]);
        list.set_filter(Filter::Kind(KindFilter::Card));
        assert_eq!(ids(&list), ["c"]);
        list.set_filter(Filter::Folder(None));
        assert_eq!(
            ids(&list),
            ["a"],
            "itens de organização não ficam em 'Sem pasta'"
        );
        list.set_filter(Filter::Organization("org".into()));
        assert_eq!(ids(&list), ["c"]);
        assert_eq!(list.count(&Filter::All), 2);
    }

    #[test]
    fn card_subtitle_shows_only_last_four() {
        let card = item(
            "c",
            "Cartão",
            ItemKind::Card(Card {
                brand: Some("Visa".into()),
                number: Some(scout_vault_core::Secret::new("4111 1111 1111 1234".into())),
                ..Card::default()
            }),
        );
        assert_eq!(subtitle(&card), "Visa, *1234");
    }
}
