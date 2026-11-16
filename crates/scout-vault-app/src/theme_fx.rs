//! Transition when changing the appearance: the interface **transforms** in place.
//!
//! Each theme color (backgrounds, texts, borders, buttons, accent) goes from the
//! old value to the new one frame by frame, along with the text size and the corners —
//! nothing disappears, buttons just change color. When switching between light and dark, a
//! soft wave starts from the clicked button.
//!
//! With Settings → Motion → Still, it switches directly, without animation.
//! Time comes from the GPUI executor's clock (tests can advance it).

use std::time::{Duration, Instant};

use gpui_kit::component::{ActiveTheme as _, Theme, ThemeColor};
use gpui_kit::*;
use serde_json::Value;

use crate::appearance::{self, Appearance, Motion};

/// Duration of the transformation when switching between light and dark.
pub const FLIP: Duration = Duration::from_millis(520);
/// Duration of the other changes (accent color, text, corners).
pub const TWEAK: Duration = Duration::from_millis(280);
/// Interval between blend frames.
const FRAME: Duration = Duration::from_millis(16);

/// What the transition blends.
#[derive(Clone)]
struct Look {
    colors: ThemeColor,
    font_size: Pixels,
    radius: Pixels,
    radius_lg: Pixels,
}

impl Look {
    fn of(cx: &App) -> Self {
        let t = cx.theme();
        Self {
            colors: t.colors,
            font_size: t.font_size,
            radius: t.radius,
            radius_lg: t.radius_lg,
        }
    }

    fn put(&self, cx: &mut App) {
        let look = self.clone();
        Theme::update(cx, move |t| {
            t.colors = look.colors;
            t.font_size = look.font_size;
            t.radius = look.radius;
            t.radius_lg = look.radius_lg;
        });
    }
}

/// Ready blend: the start and end colors already split per field.
struct Blend {
    base: serde_json::Map<String, Value>,
    pairs: Vec<(String, Rgba, Rgba)>,
    from: Look,
    to: Look,
}

impl Blend {
    fn new(from: Look, to: Look) -> Self {
        let as_map = |c: &ThemeColor| match serde_json::to_value(c) {
            Ok(Value::Object(map)) => map,
            _ => serde_json::Map::new(),
        };
        let (a, b) = (as_map(&from.colors), as_map(&to.colors));
        let pairs = a
            .iter()
            .filter_map(|(k, va)| {
                let ca = serde_json::from_value::<Rgba>(va.clone()).ok()?;
                let cb = serde_json::from_value::<Rgba>(b.get(k)?.clone()).ok()?;
                Some((k.clone(), ca, cb))
            })
            .collect();
        Self {
            base: b,
            pairs,
            from,
            to,
        }
    }

    fn at(&self, t: f32) -> Look {
        let mut map = self.base.clone();
        for (k, a, b) in &self.pairs {
            let mix = Rgba {
                r: a.r + (b.r - a.r) * t,
                g: a.g + (b.g - a.g) * t,
                b: a.b + (b.b - a.b) * t,
                a: a.a + (b.a - a.a) * t,
            };
            if let Ok(v) = serde_json::to_value(mix) {
                map.insert(k.clone(), v);
            }
        }
        let colors = serde_json::from_value(Value::Object(map)).unwrap_or(self.to.colors);
        let px_mix = |a: Pixels, b: Pixels| a + (b - a) * t;
        Look {
            colors,
            font_size: px_mix(self.from.font_size, self.to.font_size),
            radius: px_mix(self.from.radius, self.to.radius),
            radius_lg: px_mix(self.from.radius_lg, self.to.radius_lg),
        }
    }
}

/// Wave that starts from the clicked button (only when switching light ↔ dark).
#[derive(Clone, Copy)]
struct Ripple {
    origin: Point<Pixels>,
    color: Hsla,
}

struct Fx {
    started: Instant,
    duration: Duration,
    ripple: Option<Ripple>,
    _task: Task<()>,
}

#[derive(Default)]
struct State(Option<Fx>);

impl Global for State {}

/// Is a transition in progress? (tests)
pub fn running(cx: &App) -> bool {
    cx.try_global::<State>().is_some_and(|s| s.0.is_some())
}

fn ease(t: f32) -> f32 {
    // Smooth at both ends (cubic ease-in-out).
    if t < 0.5 {
        4. * t * t * t
    } else {
        1. - (-2. * t + 2.).powi(3) / 2.
    }
}

/// Switches from `prev` to `next` with a transition; `origin` is where the click was.
pub fn change(
    prev: &Appearance,
    next: &Appearance,
    origin: Point<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    if prev.motion == Motion::Off || next.motion == Motion::Off {
        cx.set_global(State(None));
        appearance::apply(next, Some(window), cx);
        return;
    }
    // Starts from what is on screen now (even in the middle of another transition).
    let from = Look::of(cx);
    let system = window.appearance();
    let flips = appearance::mode_for(prev.theme, system).is_dark()
        != appearance::mode_for(next.theme, system).is_dark();
    // Applies the new one (mode, settings) and keeps the target; the screen goes back to the start.
    appearance::apply(next, Some(window), cx);
    let to = Look::of(cx);
    from.put(cx);
    let duration = if flips { FLIP } else { TWEAK };
    let ripple = flips.then_some(Ripple {
        origin,
        color: to.colors.background,
    });
    let blend = Blend::new(from, to);
    let started = cx.background_executor().now();
    let next = *next;
    let task = window.spawn(cx, async move |cx| loop {
        cx.background_executor().timer(FRAME).await;
        let done = cx
            .update(|window, cx| {
                let elapsed = cx
                    .background_executor()
                    .now()
                    .saturating_duration_since(started);
                let t = (elapsed.as_secs_f32() / duration.as_secs_f32()).min(1.);
                if t >= 1. {
                    // End: the exact theme (tokens and all), with no blend leftovers.
                    cx.set_global(State(None));
                    appearance::apply(&next, Some(window), cx);
                    window.refresh();
                    return true;
                }
                blend.at(ease(t)).put(cx);
                window.refresh();
                false
            })
            .unwrap_or(true);
        if done {
            break;
        }
    });
    cx.set_global(State(Some(Fx {
        started,
        duration,
        ripple,
        _task: task,
    })));
    window.refresh();
}

/// Radius that covers the whole window from `origin`.
fn cover_radius(bounds: Bounds<Pixels>, origin: Point<Pixels>) -> f32 {
    [
        bounds.origin,
        bounds.top_right(),
        bounds.bottom_left(),
        bounds.bottom_right(),
    ]
    .iter()
    .map(|c| {
        let (dx, dy) = (f32::from(c.x - origin.x), f32::from(c.y - origin.y));
        (dx * dx + dy * dy).sqrt()
    })
    .fold(0., f32::max)
}

/// The wave on top of the interface (translucent: the interface stays visible).
pub fn overlay(cx: &App) -> Option<AnyElement> {
    let fx = cx.try_global::<State>()?.0.as_ref()?;
    let ripple = fx.ripple?;
    let (started, duration) = (fx.started, fx.duration);
    Some(
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, cx| {
                        let elapsed = cx
                            .background_executor()
                            .now()
                            .saturating_duration_since(started);
                        let t = (elapsed.as_secs_f32() / duration.as_secs_f32()).min(1.);
                        let grow = 1. - (1. - t).powi(3);
                        let r = cover_radius(bounds, ripple.origin) * grow + 1.;
                        let circle = Bounds::new(
                            point(ripple.origin.x - px(r), ripple.origin.y - px(r)),
                            size(px(2. * r), px(2. * r)),
                        );
                        window.paint_quad(
                            fill(circle, ripple.color.opacity(0.28 * (1. - t))).corner_radii(px(r)),
                        );
                    },
                )
                .size_full(),
            )
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::{cover_radius, ease};
    use gpui_kit::{point, px, size, Bounds};

    #[test]
    fn radius_reaches_the_farthest_corner() {
        let b = Bounds::new(point(px(0.), px(0.)), size(px(300.), px(400.)));
        assert!((cover_radius(b, point(px(0.), px(0.))) - 500.).abs() < 0.01);
        assert!((cover_radius(b, point(px(150.), px(200.))) - 250.).abs() < 0.01);
    }

    #[test]
    fn easing_starts_and_ends_in_place() {
        assert_eq!(ease(0.), 0.);
        assert!((ease(1.) - 1.).abs() < 1e-6);
        assert!((ease(0.5) - 0.5).abs() < 1e-6);
    }
}
