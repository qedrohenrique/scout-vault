//! Busca rápida global (`Ctrl+Shift+Espaço`): uma janelinha por cima de tudo
//! para achar um item e **preencher** no app que estava em foco, **copiar**
//! usuário/senha/código ou **abrir** no Scout Vault — sem trocar de janela.
//!
//! A janela só recebe nomes e usuários (nada secreto); cada ação vira um
//! [`Command`] no mesmo canal da bandeja, e é o app quem copia ou digita
//! (com o pedido de senha mestra, se o item exigir).

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use scout_vault_core::vault::{Item, ItemKind};
use tokio::sync::mpsc::UnboundedSender;

use crate::item_list::kind_icon;
use crate::tray::Command;
use crate::vault_view::{CopyShortcut, VaultView};
use crate::{appearance, motion};

/// Atalho global, para mostrar na interface.
pub const HOTKEY_LABEL: &str = "Ctrl+Shift+Espaço";
/// Tamanho da janela (com a folga transparente da sombra).
pub const WIDTH: f32 = 580.;
pub const HEIGHT: f32 = 452.;
pub const MARGIN: f32 = 14.;
/// Quantos resultados aparecem.
pub const MAX_RESULTS: usize = 7;

/// Um item como a busca rápida mostra (sem segredos).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickItem {
    pub id: String,
    pub name: String,
    /// Usuário, site ou tipo — a segunda linha.
    pub detail: String,
    pub icon: IconName,
    /// Endereços salvos (só para a busca).
    pub uris: Vec<String>,
    pub favorite: bool,
    pub has_username: bool,
    pub has_password: bool,
    pub has_totp: bool,
    /// Dá para digitar (login com usuário ou senha).
    pub typeable: bool,
}

impl QuickItem {
    pub fn from_item(item: &Item) -> Self {
        let (has_username, has_password, has_totp) = VaultView::copyable(item);
        let (detail, uris) = match &item.kind {
            ItemKind::Login(l) => (
                l.username
                    .clone()
                    .filter(|u| !u.is_empty())
                    .or_else(|| l.uris.first().cloned())
                    .unwrap_or_default(),
                l.uris.clone(),
            ),
            ItemKind::Card(c) => (
                c.brand.clone().unwrap_or_else(|| "Cartão".into()),
                Vec::new(),
            ),
            ItemKind::Identity(_) => ("Identidade".into(), Vec::new()),
            ItemKind::SecureNote => ("Nota".into(), Vec::new()),
            ItemKind::SshKey(_) => ("Chave SSH".into(), Vec::new()),
            ItemKind::Unknown(_) => (String::new(), Vec::new()),
        };
        Self {
            id: item.id.clone(),
            name: item.name.clone(),
            detail,
            icon: kind_icon(&item.kind),
            uris,
            favorite: item.favorite,
            has_username,
            has_password,
            has_totp,
            typeable: crate::autotype::typeable(item),
        }
    }
}

/// Os itens que combinam com a busca, do mais ao menos provável.
pub fn search(items: &[QuickItem], query: &str) -> Vec<QuickItem> {
    let q = query.trim().to_lowercase();
    let mut scored: Vec<(u8, &QuickItem)> = items
        .iter()
        .filter_map(|i| {
            let name = i.name.to_lowercase();
            let score = if q.is_empty() {
                // Sem busca: favoritos primeiro.
                if i.favorite {
                    0
                } else {
                    5
                }
            } else if name.starts_with(&q) {
                0
            } else if name.split_whitespace().any(|w| w.starts_with(&q)) {
                1
            } else if name.contains(&q) {
                2
            } else if i.detail.to_lowercase().contains(&q) {
                3
            } else if i.uris.iter().any(|u| u.to_lowercase().contains(&q)) {
                4
            } else {
                return None;
            };
            Some((score, i))
        })
        .collect();
    scored.sort_by(|(sa, a), (sb, b)| {
        sa.cmp(sb)
            .then(b.favorite.cmp(&a.favorite))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    scored
        .into_iter()
        .take(MAX_RESULTS)
        .map(|(_, i)| i.clone())
        .collect()
}

pub struct QuickSearch {
    items: Vec<QuickItem>,
    results: Vec<QuickItem>,
    selected: usize,
    query: Entity<InputState>,
    /// Há um app de destino para digitar (o atalho não foi usado no próprio Scout Vault).
    can_fill: bool,
    /// Título do app de destino.
    target: String,
    tx: UnboundedSender<Command>,
    /// Aviso depois de copiar ("Senha copiada"), antes de fechar.
    flash: Option<SharedString>,
    activated: bool,
    _subscriptions: Vec<Subscription>,
}

impl QuickSearch {
    pub fn new(
        items: Vec<QuickItem>,
        target: Option<String>,
        tx: UnboundedSender<Command>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let query = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Buscar no cofre — nome, usuário ou site")
        });
        let input_sub = cx.subscribe_in(&query, window, |this, _, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                this.refresh(cx);
            }
        });
        // Clicar fora fecha, como um menu.
        let activation = cx.observe_window_activation(window, |this, window, _| {
            if window.is_window_active() {
                this.activated = true;
            } else if this.activated {
                window.remove_window();
            }
        });
        query.update(cx, |q, cx| q.focus(window, cx));
        let results = search(&items, "");
        Self {
            items,
            results,
            selected: 0,
            query,
            can_fill: target.is_some(),
            target: target.unwrap_or_default(),
            tx,
            flash: None,
            activated: false,
            _subscriptions: vec![input_sub, activation],
        }
    }

    pub fn results(&self) -> &[QuickItem] {
        &self.results
    }

    pub fn selected(&self) -> Option<&QuickItem> {
        self.results.get(self.selected)
    }

    pub fn set_query(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.query
            .update(cx, |q, cx| q.set_value(text.to_owned(), window, cx));
        self.refresh(cx);
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let q = self.query.read(cx).value().to_string();
        self.results = search(&self.items, &q);
        self.selected = 0;
        cx.notify();
    }

    pub fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.results.is_empty() {
            return;
        }
        let n = self.results.len() as isize;
        self.selected = (self.selected as isize + delta).rem_euclid(n) as usize;
        cx.notify();
    }

    /// Enter: digita no app de destino (logins) ou abre no Scout Vault.
    pub fn activate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.selected().cloned() else {
            return;
        };
        if self.can_fill && item.typeable {
            // Fecha antes: o foco volta ao app de destino, onde o app vai digitar.
            window.remove_window();
            let _ = self.tx.send(Command::Fill { item_id: item.id });
        } else {
            self.open(window, cx);
        }
    }

    pub fn open(&mut self, window: &mut Window, _: &mut Context<Self>) {
        let Some(item) = self.selected().cloned() else {
            return;
        };
        window.remove_window();
        let _ = self.tx.send(Command::OpenItem { item_id: item.id });
    }

    pub fn copy(&mut self, what: CopyShortcut, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.selected().cloned() else {
            return;
        };
        let has = match what {
            CopyShortcut::Username => item.has_username,
            CopyShortcut::Password => item.has_password,
            CopyShortcut::TotpCode => item.has_totp,
        };
        if !has {
            self.flash = Some(
                match what {
                    CopyShortcut::Username => "Este item não tem usuário",
                    CopyShortcut::Password => "Este item não tem senha",
                    CopyShortcut::TotpCode => "Este item não tem código",
                }
                .into(),
            );
            cx.notify();
            return;
        }
        let _ = self.tx.send(Command::Copy {
            item_id: item.id,
            what,
        });
        self.flash = Some(
            match what {
                CopyShortcut::Username => "Usuário copiado",
                CopyShortcut::Password => "Senha copiada · some em 30 s",
                CopyShortcut::TotpCode => "Código copiado",
            }
            .into(),
        );
        cx.notify();
        // Mostra o aviso por um instante e fecha.
        cx.spawn_in(window, async move |_, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(650))
                .await;
            cx.update(|window, _| window.remove_window()).ok();
        })
        .detach();
    }

    fn on_key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        let ctrl = k.modifiers.control;
        match (ctrl, k.key.as_str()) {
            (false, "escape") => window.remove_window(),
            (false, "down") => self.move_selection(1, cx),
            (false, "up") => self.move_selection(-1, cx),
            (false, "enter") => self.activate(window, cx),
            (true, "p") => self.copy(CopyShortcut::Password, window, cx),
            (true, "u") => self.copy(CopyShortcut::Username, window, cx),
            (true, "t") => self.copy(CopyShortcut::TotpCode, window, cx),
            (true, "o") => self.open(window, cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    fn row(&self, ix: usize, item: &QuickItem, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = ix == self.selected;
        let accent = appearance::accent(cx);
        let theme = cx.theme();
        let (muted, hover) = (theme.muted_foreground, theme.secondary_hover);
        h_flex()
            .id(SharedString::from(format!("quick-item-{}", item.id)))
            .test_support()
            .gap_3()
            .px_3()
            .py_2()
            .mx_1p5()
            .rounded(px(9.))
            .cursor_pointer()
            .when(selected, |el| el.bg(accent.opacity(0.14)))
            .when(!selected, |el| el.hover(|el| el.bg(hover)))
            .child(crate::ui::icon_chip(item.icon, px(30.), selected, cx))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
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
                            .text_color(muted)
                            .child(item.detail.clone()),
                    ),
            )
            .when(selected, |el| {
                el.child(div().text_xs().text_color(accent).child(
                    if self.can_fill && item.typeable {
                        "↵ preencher"
                    } else {
                        "↵ abrir"
                    },
                ))
            })
            .on_click(cx.listener(move |this, _, window, cx| {
                this.selected = ix;
                this.activate(window, cx);
            }))
    }
}

impl Render for QuickSearch {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (border, popover, fg, muted) = (
            theme.border,
            theme.popover,
            theme.popover_foreground,
            theme.muted_foreground,
        );
        let rows: Vec<AnyElement> = self
            .results
            .clone()
            .iter()
            .enumerate()
            .map(|(ix, item)| self.row(ix, item, cx).into_any_element())
            .collect();
        let empty = rows.is_empty();
        let target = if self.can_fill {
            format!("Preencher em: {}", self.target.trim())
        } else {
            "Busca rápida".to_owned()
        };
        let footer: SharedString = match &self.flash {
            Some(text) => text.clone(),
            None => "↵ preencher/abrir · Ctrl+P senha · Ctrl+U usuário · Ctrl+T código · Ctrl+O abrir · Esc fechar".into(),
        };
        let card = v_flex()
            .size_full()
            .rounded(px(14.))
            .border_1()
            .border_color(border)
            .bg(popover)
            .text_color(fg)
            .shadow_lg()
            .overflow_hidden()
            .child(
                v_flex()
                    .gap_1()
                    .px_4()
                    .pt_3()
                    .pb_2()
                    .child(div().text_xs().text_color(muted).truncate().child(target))
                    .child(Input::new(&self.query).id("quick-search-input").large()),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .py_1()
                    .gap_px()
                    .children(rows)
                    .when(empty, |el| {
                        el.child(
                            v_flex()
                                .flex_1()
                                .items_center()
                                .justify_center()
                                .gap_2()
                                .text_sm()
                                .text_color(muted)
                                .child(Icon::new(IconName::Search))
                                .child("Nada encontrado."),
                        )
                    }),
            )
            .child(
                div()
                    .id("quick-search-footer")
                    .test_support()
                    .aria_label(footer.clone())
                    .px_4()
                    .py_2()
                    .border_t_1()
                    .border_color(border)
                    .text_xs()
                    .text_color(if self.flash.is_some() {
                        appearance::accent(cx)
                    } else {
                        muted
                    })
                    .truncate()
                    .child(footer),
            );
        div()
            .id("quick-search")
            .size_full()
            .p(px(MARGIN))
            .capture_key_down(cx.listener(Self::on_key))
            .child(motion::rise(card, motion::key("busca-rapida", "abre"), 8.))
    }
}

#[cfg(test)]
mod tests {
    use super::{search, QuickItem};
    use gpui_kit::assets::IconName;

    fn item(id: &str, name: &str, detail: &str, favorite: bool) -> QuickItem {
        QuickItem {
            id: id.into(),
            name: name.into(),
            detail: detail.into(),
            icon: IconName::Globe,
            uris: vec![format!("https://{id}.example")],
            favorite,
            has_username: true,
            has_password: true,
            has_totp: false,
            typeable: true,
        }
    }

    #[test]
    fn ranks_prefix_then_word_then_detail() {
        let items = vec![
            item("a", "Conta do Banco", "eu", false),
            item("b", "Banco Inter", "eu", false),
            item("c", "Steam", "banco@mail", false),
            item("d", "Netflix", "x", true),
        ];
        let ids = |q: &str| {
            search(&items, q)
                .into_iter()
                .map(|i| i.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids("banco"), ["b", "a", "c"]);
        assert_eq!(ids("d.example"), ["d"]);
        assert!(ids("nada-disso").is_empty());
        // Sem busca: favoritos primeiro.
        assert_eq!(ids("")[0], "d");
    }
}
