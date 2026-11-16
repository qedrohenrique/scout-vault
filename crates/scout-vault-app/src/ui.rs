//! Visual pieces reused by the screens: brand mark, section labels,
//! icon "chips" and the segmented control.

use gpui_kit::assets::IconName;
use gpui_kit::component::{button::*, *};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::appearance;

/// The Scout Vault mark: the padlock with a hat (`assets/brand/mark.svg`) on a
/// square in the accent color — follows the theme, unlike the `.exe` icon.
pub fn brand_mark(size: Pixels, cx: &App) -> Div {
    div()
        .size(size)
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded(cx.theme().radius_lg)
        // Monochrome mark: the glyph in the text color, on a neutral square.
        .bg(cx.theme().foreground)
        .child(
            svg()
                .path(crate::assets::BRAND_MARK)
                .size(size * 0.72)
                .text_color(cx.theme().background),
        )
}

/// Mark + name, for the top of the sidebar and of the entry screens.
pub fn wordmark(cx: &App) -> Div {
    h_flex()
        .gap_2()
        .child(brand_mark(px(22.), cx))
        .child(div().text_sm().font_semibold().child("scout vault"))
}

/// Large mark, centered above the form of the entry screens.
pub fn hero_wordmark(cx: &App) -> Div {
    h_flex()
        .gap_3()
        .items_center()
        .justify_center()
        .child(brand_mark(px(52.), cx))
        .child(
            div()
                .text_size(px(38.))
                .font_semibold()
                .line_height(px(44.))
                .child("scout vault"),
        )
}

/// Small group heading ("TYPES", "FOLDERS"…).
pub fn section_label(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .px_2()
        .pt_3()
        .pb_1()
        .text_size(px(11.))
        .font_semibold()
        .text_color(cx.theme().muted_foreground.opacity(0.8))
        .child(text.into())
}

/// Icon inside a soft square — used in the list and the detail.
pub fn icon_chip(icon: impl Into<Icon>, size: Pixels, highlighted: bool, cx: &App) -> Div {
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

/// An option of a [`segmented`].
pub struct Segment {
    pub id: SharedString,
    pub label: SharedString,
    pub icon: Option<IconName>,
    pub selected: bool,
}

/// Segmented control: a "pill" with the options side by side; the chosen one
/// is raised. `on_select` receives the clicked index.
pub fn segmented(
    id: impl Into<ElementId>,
    segments: Vec<Segment>,
    on_select: impl Fn(usize, &mut Window, &mut App) + 'static,
    cx: &App,
) -> Stateful<Div> {
    segmented_sized(id, segments, on_select, false, cx)
}

/// Compact version of [`segmented`], for toolbars.
pub fn segmented_compact(
    id: impl Into<ElementId>,
    segments: Vec<Segment>,
    on_select: impl Fn(usize, &mut Window, &mut App) + 'static,
    cx: &App,
) -> Stateful<Div> {
    segmented_sized(id, segments, on_select, true, cx)
}

fn segmented_sized(
    id: impl Into<ElementId>,
    segments: Vec<Segment>,
    on_select: impl Fn(usize, &mut Window, &mut App) + 'static,
    compact: bool,
    cx: &App,
) -> Stateful<Div> {
    let on_select = std::rc::Rc::new(on_select);
    let theme = cx.theme();
    let popover = theme.popover;
    let items = segments.into_iter().enumerate().map(|(ix, s)| {
        let on_select = on_select.clone();
        let (selected, anim_id) = (
            s.selected,
            crate::motion::key("seg", format!("{}:{}", s.id, s.selected)),
        );
        h_flex()
            .id(s.id.clone())
            .test_support()
            .role(Role::Button)
            .aria_label(s.label.clone())
            .gap_1p5()
            .when(compact, |el| el.px_2().py_0p5())
            .when(!compact, |el| el.px_3().py_1())
            .rounded(theme.radius)
            .text_sm()
            .cursor_pointer()
            .active(|el| el.opacity(0.7))
            .when(s.selected, |el| {
                el.bg(theme.popover)
                    .text_color(theme.foreground)
                    .font_semibold()
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
            // The chosen option lights up and "settles" in place.
            .with_animation(
                anim_id,
                Animation::new(crate::motion::NORMAL).with_easing(ease_out_quint()),
                move |el, t| {
                    if selected {
                        el.bg(popover.opacity(t))
                            .relative()
                            .top(px(1.5 * (1.0 - t)))
                    } else {
                        el
                    }
                },
            )
    });
    h_flex()
        .id(id.into())
        .p(px(3.))
        .gap(px(2.))
        .rounded(theme.radius + px(3.))
        .bg(theme.muted)
        .children(items)
}

/// Frame of the entry screens (login and unlock): settings gear in the
/// corner and, in the center, the large mark above the card.
pub fn entry_screen(
    settings_id: &'static str,
    title: impl Into<SharedString>,
    subtitle: impl Into<SharedString>,
    width: Pixels,
    body: impl IntoElement,
    on_settings: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> Div {
    let top = h_flex().w_full().px_5().py_4().justify_end().child(
        Button::new(settings_id)
            .ghost()
            .small()
            .icon(IconName::Settings)
            .tooltip(t!("vault.settings_tooltip"))
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
        .relative()
        .size_full()
        .bg(cx.theme().background)
        .text_color(cx.theme().foreground)
        // The art stays behind everything; the (opaque) card goes on top.
        .child(crate::backdrop::backdrop(crate::appearance::accent(cx), cx))
        .child(top)
        .child(
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .pb_16()
                .child(
                    v_flex()
                        .items_center()
                        .gap_6()
                        .child(hero_wordmark(cx).id("entry-brand"))
                        .child(card),
                ),
        )
}

/// Status line (error in red, progress in gray) with a fixed height.
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
        .child(animated_status(text, error))
}

/// Status text with motion: "…" (working) pulses; an error arrives with
/// a slight shake; everything else just fades in.
fn animated_status(text: SharedString, error: bool) -> AnyElement {
    let key = crate::motion::key("status", &text);
    let body = div().child(text.clone());
    if text.is_empty() {
        return body.into_any_element();
    }
    if !error && text.ends_with('…') {
        return crate::motion::pulse(body, key).into_any_element();
    }
    if error {
        return crate::motion::shake(body, key).into_any_element();
    }
    crate::motion::fade(body, key).into_any_element()
}

/// Attach area: click to choose files or drag from Explorer.
pub fn drop_zone(
    id: &'static str,
    busy: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_drop: impl Fn(&ExternalPaths, &mut Window, &mut App) + 'static,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    let accent = appearance::accent(cx);
    let tint = appearance::accent_tint(cx, 0.08);
    v_flex()
        .id(id)
        .role(Role::Button)
        .aria_label(SharedString::from(t!("ui.attach_files")))
        .w_full()
        .items_center()
        .gap_1()
        .px_4()
        .py_4()
        .rounded(theme.radius_lg)
        .border_1()
        .border_color(theme.border)
        .text_color(theme.muted_foreground)
        .when(!busy, |el| {
            el.cursor_pointer()
                .hover(|el| el.bg(theme.secondary_hover))
                .active(|el| el.opacity(0.7))
        })
        // File being dragged over it: lights up in the accent color.
        .drag_over::<ExternalPaths>(move |style, _, _, _| style.border_color(accent).bg(tint))
        .on_click(on_click)
        .on_drop(on_drop)
        .child(Icon::new(IconName::Paperclip).text_color(accent))
        .child(
            div()
                .text_sm()
                .font_semibold()
                .text_color(theme.foreground)
                .child(t!("ui.drop_here")),
        )
        .child(div().text_xs().child(t!("ui.drop_hint")))
        .test_support()
}
