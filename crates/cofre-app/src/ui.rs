//! Peças visuais reaproveitadas pelas telas: marca, rótulos de seção,
//! "chips" de ícone e o controle segmentado.

use gpui_kit::assets::IconName;
use gpui_kit::component::{button::*, *};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::appearance;

/// A marca do Cofre: um quadrado na cor de destaque com a fechadura.
pub fn brand_mark(size: Pixels, cx: &App) -> Div {
    div()
        .size(size)
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded(cx.theme().radius_lg)
        .bg(appearance::accent(cx))
        .text_color(cx.theme().primary_foreground)
        .child(Icon::new(IconName::LockKeyhole).size(size * 0.5))
}

/// Marca + nome, para o topo da barra lateral e das telas de entrada.
pub fn wordmark(cx: &App) -> Div {
    h_flex()
        .gap_2()
        .child(brand_mark(px(22.), cx))
        .child(div().text_sm().font_semibold().child("cofre"))
}

/// Título pequeno de grupo ("TIPOS", "PASTAS"…).
pub fn section_label(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .px_2()
        .pt_3()
        .pb_1()
        .text_size(px(11.))
        .font_medium()
        .text_color(cx.theme().muted_foreground.opacity(0.8))
        .child(text.into())
}

/// Ícone dentro de um quadrado suave — usado na lista e no detalhe.
pub fn icon_chip(icon: IconName, size: Pixels, highlighted: bool, cx: &App) -> Div {
    let (bg, fg) = if highlighted {
        (appearance::accent_tint(cx, 0.14), appearance::accent(cx))
    } else {
        (cx.theme().muted, cx.theme().muted_foreground)
    };
    div()
        .size(size)
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded(cx.theme().radius)
        .bg(bg)
        .text_color(fg)
        .child(Icon::new(icon).size(size * 0.5))
}

/// Uma opção de um [`segmented`].
pub struct Segment {
    pub id: SharedString,
    pub label: SharedString,
    pub icon: Option<IconName>,
    pub selected: bool,
}

/// Controle segmentado: uma "pílula" com as opções lado a lado; a escolhida
/// fica em relevo. `on_select` recebe o índice clicado.
pub fn segmented(
    id: impl Into<ElementId>,
    segments: Vec<Segment>,
    on_select: impl Fn(usize, &mut Window, &mut App) + 'static,
    cx: &App,
) -> Stateful<Div> {
    let on_select = std::rc::Rc::new(on_select);
    let theme = cx.theme();
    let items = segments.into_iter().enumerate().map(|(ix, s)| {
        let on_select = on_select.clone();
        h_flex()
            .id(s.id.clone())
            .test_support()
            .role(Role::Button)
            .aria_label(s.label.clone())
            .gap_1p5()
            .px_3()
            .py_1()
            .rounded(theme.radius)
            .text_sm()
            .cursor_pointer()
            .when(s.selected, |el| {
                el.bg(theme.popover)
                    .text_color(theme.foreground)
                    .font_medium()
                    .border_1()
                    .border_color(theme.border)
            })
            .when(!s.selected, |el| {
                el.text_color(theme.muted_foreground)
                    .border_1()
                    .border_color(transparent_black())
                    .hover(|el| el.text_color(theme.foreground))
            })
            .children(s.icon.map(|i| Icon::new(i).small()))
            .child(s.label)
            .on_click(move |_, window, cx| on_select(ix, window, cx))
    });
    h_flex()
        .id(id.into())
        .p(px(3.))
        .gap(px(2.))
        .rounded(theme.radius + px(3.))
        .bg(theme.muted)
        .children(items)
}

/// Moldura das telas de entrada (login e desbloqueio): marca no topo,
/// engrenagem de configurações no canto e um cartão central.
pub fn entry_screen(
    settings_id: &'static str,
    title: impl Into<SharedString>,
    subtitle: impl Into<SharedString>,
    width: Pixels,
    body: impl IntoElement,
    on_settings: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> Div {
    let top = h_flex()
        .w_full()
        .px_5()
        .py_4()
        .justify_between()
        .child(wordmark(cx))
        .child(
            Button::new(settings_id)
                .ghost()
                .small()
                .icon(IconName::Settings)
                .tooltip("Configurações (Ctrl+,)")
                .on_click(on_settings),
        );
    let card = v_flex()
        .w(width)
        .gap_6()
        .p_8()
        .rounded(cx.theme().radius_lg)
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().popover)
        .child(
            v_flex()
                .gap_1()
                .child(div().text_xl().font_semibold().child(title.into()))
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(subtitle.into()),
                ),
        )
        .child(body);
    v_flex()
        .size_full()
        .bg(cx.theme().background)
        .text_color(cx.theme().foreground)
        .child(top)
        .child(
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .pb_16()
                .child(card),
        )
}

/// Linha de status (erro em vermelho, progresso em cinza) com altura fixa.
pub fn status_line(
    id: &'static str,
    text: SharedString,
    error: bool,
    cx: &App,
) -> impl IntoElement {
    div()
        .id(id)
        .role(Role::Status)
        .test_support()
        .aria_label(text.clone())
        .min_h(px(20.))
        .text_sm()
        .text_color(if error {
            cx.theme().danger
        } else {
            cx.theme().muted_foreground
        })
        .child(text)
}
