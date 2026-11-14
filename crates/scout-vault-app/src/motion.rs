//! Animações da interface: entradas suaves, texto que muda, carregamento.
//!
//! Tudo passa pelo `with_animation` do GPUI, que respeita
//! `App::reduce_motion` — ligado quando Configurações → Movimento está em
//! "Parado". Aí cada animação aparece direto no estado final, sem quadros extras.
//!
//! As animações são curtas (até ~¼ s) e só mexem em opacidade e posição: o
//! layout não pula e nada fica esperando a animação acabar para ser clicado.

use std::time::Duration;

use gpui_kit::*;

/// Mudanças pequenas (texto, ícone).
pub const QUICK: Duration = Duration::from_millis(160);
/// Painéis e telas.
pub const NORMAL: Duration = Duration::from_millis(230);

fn ease() -> impl Fn(f32) -> f32 {
    ease_out_quint()
}

/// Id de animação a partir de um texto: mudar a chave reinicia a animação.
pub fn key(prefix: &str, value: impl std::fmt::Display) -> ElementId {
    ElementId::Name(SharedString::from(format!("{prefix}:{value}")))
}

/// Entra subindo `dy` px e aparecendo (telas, sobreposições, cartões).
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

/// Entra deslizando `dx` px na horizontal (painel de detalhe, editor).
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

/// Só aparece (texto que mudou, troca de lista).
pub fn fade<E>(el: E, id: ElementId) -> AnimationElement<E>
where
    E: IntoElement + Styled + 'static,
{
    el.with_animation(id, Animation::new(QUICK).with_easing(ease()), |el, t| {
        el.opacity(0.25 + 0.75 * t)
    })
}

/// Sacudida curta para erros (ex.: senha errada).
pub fn shake<E>(el: E, id: ElementId) -> AnimationElement<E>
where
    E: IntoElement + Styled + 'static,
{
    el.with_animation(id, Animation::new(Duration::from_millis(380)), |el, t| {
        let offset = (t * std::f32::consts::PI * 6.0).sin() * 5.0 * (1.0 - t);
        el.opacity((t * 4.0).min(1.0)).relative().left(px(offset))
    })
}

/// "Respira" enquanto algo carrega.
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

/// Ícone que "salta" ao aparecer (ex.: ✓ depois de copiar).
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

/// Entrada em cascata: a `order`-ésima linha espera um pouco antes de entrar,
/// deslizando da esquerda. Só roda quando o elemento aparece pela primeira vez.
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

/// Barrinha de seleção na lateral esquerda: cresce a partir do centro.
/// O pai precisa ser `relative`.
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

/// Seleção que "abraça" o ícone: nasce como a barrinha na lateral do quadrado
/// e se transforma numa moldura arredondada em volta dele, num movimento só.
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
                    // A barra sólida vira contorno com um leve preenchimento.
                    .bg(color.opacity(lerp(1., 0.10)))
                    .border_color(color.opacity(lerp(0., 0.9)))
            },
        ))
        .child(chip)
}

/// Desloca `dx` px para o lado e fica lá (ícone do item escolhido).
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

/// Fundo que acende ao ser escolhido.
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

/// Barra fina de progresso indeterminado (sincronizando, salvando, baixando).
pub fn progress_bar(id: &'static str, color: Hsla) -> impl IntoElement {
    div()
        .id(id)
        .test_support()
        .role(Role::ProgressIndicator)
        .aria_label(SharedString::from("Carregando"))
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
