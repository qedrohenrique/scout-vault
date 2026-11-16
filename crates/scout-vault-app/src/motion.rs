//! Interface animations: soft entrances, changing text, loading.
//!
//! Everything goes through GPUI's `with_animation`, which respects
//! `App::reduce_motion` — on when Settings → Motion is set to
//! "Still". Then each animation shows up directly in its final state, with no extra frames.
//!
//! Animations are short (up to ~¼ s) and only touch opacity and position: the
//! layout doesn't jump and nothing waits for the animation to end to be clickable.

use std::time::Duration;

use gpui_kit::*;

/// Small changes (text, icon).
pub const QUICK: Duration = Duration::from_millis(160);
/// Panels and screens.
pub const NORMAL: Duration = Duration::from_millis(230);

fn ease() -> impl Fn(f32) -> f32 {
    ease_out_quint()
}

/// Animation id from a text: changing the key restarts the animation.
pub fn key(prefix: &str, value: impl std::fmt::Display) -> ElementId {
    ElementId::Name(SharedString::from(format!("{prefix}:{value}")))
}

/// Enters rising `dy` px and fading in (screens, overlays, cards).
pub fn rise<E>(el: E, id: ElementId, dy: f32) -> AnimationElement<E>
where
    E: IntoElement + Styled + 'static,
{
    el.with_animation(
        id,
        Animation::new(NORMAL).with_easing(ease()),
        move |el, t| el.opacity(t).relative().top(px(dy * (1.0 - t))),
    )
}

/// Enters sliding `dx` px horizontally (detail panel, editor).
pub fn slide<E>(el: E, id: ElementId, dx: f32) -> AnimationElement<E>
where
    E: IntoElement + Styled + 'static,
{
    el.with_animation(
        id,
        Animation::new(NORMAL).with_easing(ease()),
        move |el, t| el.opacity(t).relative().left(px(dx * (1.0 - t))),
    )
}

/// Just fades in (changed text, list switch).
pub fn fade<E>(el: E, id: ElementId) -> AnimationElement<E>
where
    E: IntoElement + Styled + 'static,
{
    el.with_animation(id, Animation::new(QUICK).with_easing(ease()), |el, t| {
        el.opacity(0.25 + 0.75 * t)
    })
}

/// Short shake for errors (e.g. wrong password).
pub fn shake<E>(el: E, id: ElementId) -> AnimationElement<E>
where
    E: IntoElement + Styled + 'static,
{
    el.with_animation(id, Animation::new(Duration::from_millis(380)), |el, t| {
        let offset = (t * std::f32::consts::PI * 6.0).sin() * 5.0 * (1.0 - t);
        el.opacity((t * 4.0).min(1.0)).relative().left(px(offset))
    })
}

/// "Breathes" while something loads.
pub fn pulse<E>(el: E, id: ElementId) -> AnimationElement<E>
where
    E: IntoElement + Styled + 'static,
{
    el.with_animation(
        id,
        Animation::new(Duration::from_millis(1400))
            .repeat()
            .with_easing(pulsating_between(0.45, 1.0)),
        |el, t| el.opacity(t),
    )
}

/// Icon that "pops" when it appears (e.g. ✓ after copying).
pub fn pop<E>(el: E, id: ElementId) -> AnimationElement<E>
where
    E: IntoElement + Styled + 'static,
{
    el.with_animation(
        id,
        Animation::new(Duration::from_millis(260)).with_easing(ease()),
        |el, t| el.opacity(t).relative().top(px(3.0 * (1.0 - t))),
    )
}

/// Cascading entrance: the `order`-th row waits a little before entering,
/// sliding in from the left. Only runs when the element first appears.
pub fn cascade<E>(el: E, id: ElementId, order: usize) -> AnimationElement<E>
where
    E: IntoElement + Styled + 'static,
{
    let wait = Duration::from_millis(18 * order.min(20) as u64 + 1);
    el.with_animations(
        id,
        vec![
            Animation::new(wait),
            Animation::new(NORMAL).with_easing(ease()),
        ],
        |el, step, t| match step {
            0 => el.opacity(0.),
            _ => el.opacity(t).relative().left(px(-8. * (1.0 - t))),
        },
    )
}

/// Selection bar on the left edge: grows from the center.
/// The parent must be `relative`.
pub fn accent_bar(id: ElementId, color: Hsla) -> impl IntoElement {
    div()
        .absolute()
        .left_0()
        .w(px(3.))
        .rounded_full()
        .bg(color)
        .with_animation(
            id,
            Animation::new(Duration::from_millis(280)).with_easing(ease()),
            |el, t| {
                let h = 0.7 * t;
                el.top(relative((1.0 - h) / 2.0)).h(relative(h))
            },
        )
}

/// Selection that "hugs" the icon: starts as the bar on the side of the square
/// and turns into a rounded frame around it, in a single motion.
pub fn selection_ring(
    chip: impl IntoElement,
    id: ElementId,
    color: Hsla,
    size: f32,
) -> impl IntoElement {
    const PAD: f32 = 4.;
    let bar_h = size * 0.55;
    div()
        .relative()
        .flex()
        .flex_shrink_0()
        .size(px(size))
        .child(div().absolute().border_1().with_animation(
            id,
            Animation::new(Duration::from_millis(340)).with_easing(ease()),
            move |el, t| {
                let lerp = |a: f32, b: f32| a + (b - a) * t;
                el.left(px(lerp(-PAD - 3., -PAD)))
                    .top(px(lerp((size - bar_h) / 2., -PAD)))
                    .w(px(lerp(3., size + 2. * PAD)))
                    .h(px(lerp(bar_h, size + 2. * PAD)))
                    .rounded(px(lerp(2., 10.)))
                    // The solid bar becomes an outline with a light fill.
                    .bg(color.opacity(lerp(1., 0.10)))
                    .border_color(color.opacity(lerp(0., 0.9)))
            },
        ))
        .child(chip)
}

/// Shifts `dx` px to the side and stays there (icon of the chosen item).
pub fn nudge<E>(el: E, id: ElementId, dx: f32) -> AnimationElement<E>
where
    E: IntoElement + Styled + 'static,
{
    el.with_animation(
        id,
        Animation::new(NORMAL).with_easing(ease()),
        move |el, t| el.relative().left(px(dx * t)),
    )
}

/// Background that lights up when chosen.
pub fn light_up<E>(el: E, id: ElementId, color: Hsla) -> AnimationElement<E>
where
    E: IntoElement + Styled + 'static,
{
    el.with_animation(
        id,
        Animation::new(NORMAL).with_easing(ease()),
        move |el, t| el.bg(color.opacity(color.a * t)),
    )
}

/// Thin indeterminate progress bar (syncing, saving, downloading).
pub fn progress_bar(id: &'static str, color: Hsla) -> impl IntoElement {
    div()
        .id(id)
        .test_support()
        .role(Role::ProgressIndicator)
        .aria_label(SharedString::from(t!("common.loading")))
        .relative()
        .w_full()
        .h(px(2.))
        .overflow_hidden()
        .child(
            div()
                .absolute()
                .top_0()
                .h_full()
                .w(relative(0.35))
                .rounded_full()
                .bg(color)
                .with_animation(
                    id,
                    Animation::new(Duration::from_millis(1100))
                        .repeat()
                        .with_easing(ease_in_out),
                    |el, t| el.left(relative(-0.35 + 1.35 * t)),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::key;

    #[test]
    fn keys_change_with_value() {
        assert_ne!(key("detail", "a"), key("detail", "b"));
        assert_eq!(key("detail", "a"), key("detail", "a"));
    }
}
