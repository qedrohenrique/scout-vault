//! Transição ao trocar a aparência: a interface **se transforma** no lugar.
//!
//! Cada cor do tema (fundos, textos, bordas, botões, destaque) vai do valor
//! antigo ao novo quadro a quadro, junto com o tamanho do texto e os cantos —
//! nada some, os botões só mudam de cor. Ao trocar entre claro e escuro, uma
//! onda suave sai do botão clicado.
//!
//! Com Configurações → Movimento → Parado, troca direto, sem animação.
//! O tempo vem do relógio do executor do GPUI (os testes conseguem avançá-lo).

use std::time::{Duration, Instant};

use gpui_kit::component::{ActiveTheme as _, Theme, ThemeColor};
use gpui_kit::*;
use serde_json::Value;

use crate::appearance::{self, Appearance, Motion};

/// Duração da transformação ao trocar entre claro e escuro.
pub const FLIP: Duration = Duration::from_millis(520);
/// Duração das outras mudanças (cor de destaque, texto, cantos).
pub const TWEAK: Duration = Duration::from_millis(280);
/// Intervalo entre quadros da mistura.
const FRAME: Duration = Duration::from_millis(16);

/// O que a transição mistura.
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

/// Mistura pronta: as cores de saída e chegada já separadas por campo.
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

/// Onda que sai do botão clicado (só ao trocar claro ↔ escuro).
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

/// Há uma transição em andamento? (testes)
pub fn running(cx: &App) -> bool {
    cx.try_global::<State>().is_some_and(|s| s.0.is_some())
}

fn ease(t: f32) -> f32 {
    // Suave nas pontas (ease-in-out cúbico).
    if t < 0.5 {
        4. * t * t * t
    } else {
        1. - (-2. * t + 2.).powi(3) / 2.
    }
}

/// Troca de `prev` para `next` com transição; `origin` é onde foi o clique.
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
    // Parte do que está na tela agora (inclusive no meio de outra transição).
    let from = Look::of(cx);
    let system = window.appearance();
    let flips = appearance::mode_for(prev.theme, system).is_dark()
        != appearance::mode_for(next.theme, system).is_dark();
    // Aplica o novo (modo, configs) e guarda aonde chegar; a tela volta ao começo.
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
                    // Fim: o tema exato (tokens e tudo), sem resto de mistura.
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

/// Raio que cobre a janela inteira a partir de `origin`.
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

/// A onda por cima da interface (translúcida: a interface continua à vista).
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
