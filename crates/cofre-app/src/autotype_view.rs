//! Escolha do login a digitar quando o título da janela não decide sozinho.

use cofre_core::vault::{Item, ItemKind};
use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{button::*, *};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::autotype::{self, Request};
use crate::ui;

pub enum AutoTypeEvent {
    Chosen(String),
    Close,
}

pub struct AutoTypeView {
    request: Request,
    /// Logins que podem ser digitados (cópia; sai da memória ao fechar).
    items: Vec<Item>,
    /// Ids sugeridos pelo título da janela.
    suggested: Vec<String>,
    query: Entity<InputState>,
    _subscription: Subscription,
}

impl EventEmitter<AutoTypeEvent> for AutoTypeView {}

const MAX_SHOWN: usize = 40;

impl AutoTypeView {
    pub fn new(
        request: Request,
        all: &[Item],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let items: Vec<Item> = all
            .iter()
            .filter(|i| autotype::typeable(i))
            .cloned()
            .collect();
        let suggested = autotype::candidates(&items, &request.title)
            .into_iter()
            .map(|i| i.id.clone())
            .collect();
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Buscar login"));
        let sub = cx.subscribe_in(
            &query,
            window,
            |this, _, event: &InputEvent, _, cx| match event {
                InputEvent::PressEnter { .. } => {
                    if let Some(id) = this.shown(cx).first().map(|i| i.id.clone()) {
                        cx.emit(AutoTypeEvent::Chosen(id));
                    }
                }
                _ => cx.notify(),
            },
        );
        query.update(cx, |q, cx| q.focus(window, cx));
        Self {
            request,
            items,
            suggested,
            query,
            _subscription: sub,
        }
    }

    pub fn request(&self) -> &Request {
        &self.request
    }

    /// O que a lista mostra: sugestões, ou a busca em todos os logins.
    pub fn shown(&self, cx: &App) -> Vec<&Item> {
        let q = self.query.read(cx).value().trim().to_lowercase();
        if q.is_empty() && !self.suggested.is_empty() {
            return self
                .items
                .iter()
                .filter(|i| self.suggested.contains(&i.id))
                .collect();
        }
        self.items
            .iter()
            .filter(|i| {
                q.is_empty() || {
                    let ItemKind::Login(l) = &i.kind else {
                        return false;
                    };
                    i.name.to_lowercase().contains(&q)
                        || l.username
                            .as_deref()
                            .is_some_and(|u| u.to_lowercase().contains(&q))
                        || l.uris.iter().any(|u| u.to_lowercase().contains(&q))
                }
            })
            .take(MAX_SHOWN)
            .collect()
    }
}

impl Render for AutoTypeView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let shown: Vec<(String, String, Option<String>)> = self
            .shown(cx)
            .into_iter()
            .map(|i| {
                let user = match &i.kind {
                    ItemKind::Login(l) => l.username.clone(),
                    _ => None,
                };
                (i.id.clone(), i.name.clone(), user)
            })
            .collect();
        let suggesting =
            self.query.read(cx).value().trim().is_empty() && !self.suggested.is_empty();
        let title = if self.request.title.trim().is_empty() {
            "a janela em foco".to_owned()
        } else {
            format!("“{}”", self.request.title.trim())
        };

        let header = h_flex()
            .justify_between()
            .gap_4()
            .child(
                v_flex()
                    .min_w_0()
                    .gap_1()
                    .child(div().text_2xl().font_semibold().child("Preencher em outro app"))
                    .child(
                        div()
                            .id("autotype-target")
                            .test_support()
                            .aria_label(SharedString::from(title.clone()))
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!(
                                "Escolha o login. O Cofre volta para {title} e digita usuário ⇥ senha."
                            )),
                    ),
            )
            .child(
                Button::new("autotype-close")
                    .ghost()
                    .small()
                    .icon(IconName::Close)
                    .tooltip("Cancelar (Esc)")
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(AutoTypeEvent::Close))),
            );

        let count = shown.len();
        let rows = shown.into_iter().enumerate().map(|(ix, (id, name, user))| {
            let chosen = id.clone();
            h_flex()
                .id(SharedString::from(format!("autotype-pick-{id}")))
                .test_support()
                .role(Role::Button)
                .aria_label(SharedString::from(name.clone()))
                .gap_3()
                .px_4()
                .py_2p5()
                .cursor_pointer()
                .hover(|el| el.bg(cx.theme().secondary_hover))
                .when(ix + 1 < count, |el| {
                    el.border_b_1().border_color(cx.theme().border)
                })
                .child(ui::icon_chip(IconName::Globe, px(28.), false, cx))
                .child(
                    v_flex()
                        .min_w_0()
                        .child(div().text_sm().font_medium().truncate().child(name))
                        .children(user.map(|u| {
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .truncate()
                                .child(u)
                        })),
                )
                .on_click(
                    cx.listener(move |_, _, _, cx| cx.emit(AutoTypeEvent::Chosen(chosen.clone()))),
                )
        });

        let list = if count == 0 {
            div()
                .p_4()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("Nenhum login encontrado.")
                .into_any_element()
        } else {
            v_flex()
                .rounded(cx.theme().radius_lg)
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().popover)
                .children(rows)
                .into_any_element()
        };

        div()
            .id("autotype")
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .overflow_y_scroll()
            .child(
                v_flex()
                    .w_full()
                    .max_w(px(680.))
                    .mx_auto()
                    .px_8()
                    .py_10()
                    .gap_6()
                    .child(header)
                    .child(Input::new(&self.query).id("autotype-search"))
                    .child(
                        v_flex()
                            .gap_2()
                            .child(
                                ui::section_label(
                                    if suggesting {
                                        "SUGESTÕES PELO TÍTULO"
                                    } else {
                                        "LOGINS"
                                    },
                                    cx,
                                )
                                .px_0(),
                            )
                            .child(list),
                    ),
            )
    }
}
