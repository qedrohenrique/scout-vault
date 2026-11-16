//! Sonner-style toasts: short notices that rise from the bottom of the window, stay
//! for a bit and slide down fading out. They replace the sidebar's fixed messages.
//!
//! Any screen calls [`show`]; the app root draws the stack on top of everything.

use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;

/// How long the toast stays visible (errors stay longer).
const STAY: Duration = Duration::from_millis(2600);
const STAY_ERROR: Duration = Duration::from_millis(4200);
/// Duration of the entrance and the exit.
const ENTER: Duration = Duration::from_millis(280);
const LEAVE: Duration = Duration::from_millis(240);
/// At most this many at the same time; the oldest leaves first.
const MAX: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Success,
    Error,
    Info,
}

struct Toast {
    id: u64,
    kind: Kind,
    text: SharedString,
    leaving: bool,
}

#[derive(Default)]
pub struct Toasts {
    items: Vec<Toast>,
    next: u64,
}

struct Hub(Entity<Toasts>);

impl Global for Hub {}

/// Creates this window's toast stack (called by the app root).
pub fn init(cx: &mut App) -> Entity<Toasts> {
    let toasts = cx.new(|_| Toasts::default());
    cx.set_global(Hub(toasts.clone()));
    toasts
}

/// Shows a toast. With no stack created (e.g. isolated screens in tests), does nothing.
pub fn show(kind: Kind, text: impl Into<SharedString>, cx: &mut App) {
    let Some(hub) = cx.try_global::<Hub>().map(|h| h.0.clone()) else {
        return;
    };
    let text = text.into();
    hub.update(cx, |toasts, cx| toasts.push(kind, text, cx));
}

pub fn success(text: impl Into<SharedString>, cx: &mut App) {
    show(Kind::Success, text, cx);
}

pub fn error(text: impl Into<SharedString>, cx: &mut App) {
    show(Kind::Error, text, cx);
}

pub fn info(text: impl Into<SharedString>, cx: &mut App) {
    show(Kind::Info, text, cx);
}

impl Toasts {
    /// Texts visible right now (for tests).
    pub fn texts(&self) -> Vec<String> {
        self.items
            .iter()
            .filter(|t| !t.leaving)
            .map(|t| t.text.to_string())
            .collect()
    }

    fn push(&mut self, kind: Kind, text: SharedString, cx: &mut Context<Self>) {
        // The same repeated notice doesn't stack: restarts the one already there.
        self.items.retain(|t| t.text != text || t.leaving);
        let id = self.next;
        self.next += 1;
        self.items.push(Toast {
            id,
            kind,
            text,
            leaving: false,
        });
        let excess = self
            .items
            .iter()
            .filter(|t| !t.leaving)
            .count()
            .saturating_sub(MAX);
        for old in self
            .items
            .iter()
            .filter(|t| !t.leaving)
            .take(excess)
            .map(|t| t.id)
            .collect::<Vec<_>>()
        {
            self.leave(old, cx);
        }
        let stay = if kind == Kind::Error {
            STAY_ERROR
        } else {
            STAY
        };
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(stay).await;
            this.update(cx, |this, cx| this.leave(id, cx)).ok();
        })
        .detach();
        cx.notify();
    }

    /// Starts the exit and removes it from the list when the animation ends.
    fn leave(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(t) = self.items.iter_mut().find(|t| t.id == id && !t.leaving) else {
            return;
        };
        t.leaving = true;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(LEAVE).await;
            this.update(cx, |this, cx| {
                this.items.retain(|t| t.id != id);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
}

impl Render for Toasts {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let accent = crate::appearance::accent(cx);
        let (bg, border, fg, danger, muted) = (
            theme.popover,
            theme.border,
            theme.popover_foreground,
            theme.danger,
            theme.muted_foreground,
        );
        let radius = theme.radius_lg;
        let toasts = self.items.iter().map(|t| {
            let (icon, color) = match t.kind {
                Kind::Success => (IconName::Check, accent),
                Kind::Error => (IconName::Close, danger),
                Kind::Info => (IconName::Info, muted),
            };
            let body = h_flex()
                .id("toast")
                .test_support()
                .role(Role::Status)
                .aria_label(t.text.clone())
                .gap_2p5()
                .px_3p5()
                .py_2p5()
                .max_w(px(420.))
                .rounded(radius)
                .border_1()
                .border_color(border)
                .bg(bg)
                .text_color(fg)
                .text_sm()
                .shadow_lg()
                .child(Icon::new(icon).small().text_color(color))
                .child(div().min_w_0().child(t.text.clone()));
            let wrapper = div().child(body);
            if t.leaving {
                // Leaves sliding down and fading.
                wrapper
                    .with_animation(
                        crate::motion::key("toast-sai", t.id),
                        Animation::new(LEAVE).with_easing(ease_in_out),
                        |el, p| el.opacity(1. - p).relative().top(px(18. * p)),
                    )
                    .into_any_element()
            } else {
                // Rises from the bottom of the window with a slight "bounce".
                wrapper
                    .with_animation(
                        crate::motion::key("toast-entra", t.id),
                        Animation::new(ENTER).with_easing(ease_out_quint()),
                        |el, p| el.opacity(p).relative().top(px(22. * (1. - p))),
                    )
                    .into_any_element()
            }
        });
        v_flex()
            .absolute()
            .bottom_5()
            .left_0()
            .right_0()
            .items_center()
            .gap_2()
            .children(toasts)
    }
}
