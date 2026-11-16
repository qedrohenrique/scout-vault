//! Animated vector art for the background of the entry screens (login and unlock).
//!
//! Two wavy bands on the sides, like smoke or seaweed, swaying slowly,
//! and small shapes (bubbles, pebbles, arcs, dots) rising through the middle. All in
//! thin strokes in the accent color, very transparent, so as not to compete with the
//! form. No images: only paths computed every frame.
//!
//! With animations off (Settings → Motion), the art stays still.

use std::f32::consts::TAU;
use std::sync::OnceLock;
use std::time::Instant;

use gpui_kit::*;

/// Motion preference, kept in the app state (not in the process:
/// that way each window/test has its own).
struct MotionSetting(bool);

impl Global for MotionSetting {}

/// Turns motion on/off (called when applying the appearance).
pub fn set_motion(on: bool, cx: &mut App) {
    cx.set_global(MotionSetting(on));
}

pub fn motion(cx: &App) -> bool {
    cx.try_global::<MotionSetting>().is_none_or(|m| m.0)
}

fn seconds(animated: bool) -> f32 {
    static START: OnceLock<Instant> = OnceLock::new();
    if !animated {
        // Frozen at a nice moment (the bands already curved).
        return 7.0;
    }
    START.get_or_init(Instant::now).elapsed().as_secs_f32()
}

#[derive(Clone, Copy)]
enum Shape {
    /// Bubble: hollow circle.
    Ring,
    /// Pebble: very rounded square.
    Pebble,
    /// Filled dot.
    Dot,
    /// Hollow regular polygon with `n` sides (triangle, square, diamond, hexagon…).
    Polygon(u8),
    /// Small filled regular polygon.
    SolidPolygon(u8),
}

struct Particle {
    shape: Shape,
    /// Horizontal position, fraction of the width.
    x: f32,
    /// Initial vertical phase, fraction of the height.
    y: f32,
    size: f32,
    /// Pixels per second, upwards.
    speed: f32,
    phase: f32,
}

const PARTICLES: usize = 32;

/// Fixed pseudo-random generator: the drawing is always the same, with no system seed.
fn particles() -> &'static [Particle] {
    static LIST: OnceLock<Vec<Particle>> = OnceLock::new();
    LIST.get_or_init(|| {
        let mut state = 0x2545_f491u32;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state % 10_000) as f32 / 10_000.0
        };
        // Only closed shapes: bubbles, dots and geometric figures.
        let shapes = [
            Shape::Ring,
            Shape::Polygon(3),
            Shape::Dot,
            Shape::Polygon(4),
            Shape::Pebble,
            Shape::Polygon(6),
            Shape::SolidPolygon(3),
            Shape::Ring,
            Shape::Polygon(5),
            Shape::Dot,
            Shape::SolidPolygon(4),
            Shape::Polygon(4),
        ];
        (0..PARTICLES)
            .map(|i| {
                let shape = shapes[i % shapes.len()];
                Particle {
                    shape,
                    // Central band, away from the waves on the sides.
                    x: 0.2 + next() * 0.6,
                    y: next(),
                    size: match shape {
                        Shape::Dot => 3.5 + next() * 3.0,
                        Shape::SolidPolygon(_) => 6.0 + next() * 4.0,
                        _ => 10.0 + next() * 12.0,
                    },
                    speed: 7.0 + next() * 10.0,
                    phase: next() * TAU,
                }
            })
            .collect()
    })
}

fn pt(x: f32, y: f32) -> Point<Pixels> {
    point(px(x), px(y))
}

/// Wavy outline of a side: distance from the edge as a function of `y`.
fn edge_offset(y: f32, height: f32, width: f32, t: f32, layer: f32) -> f32 {
    let base = (width * 0.07).clamp(40.0, 110.0) + layer * (width * 0.022).clamp(12.0, 26.0);
    let big = (width * 0.04).clamp(22.0, 52.0);
    let k1 = TAU / (height * 0.80).max(160.0);
    let k2 = TAU / (height * 0.36).max(70.0);
    base + big * (y * k1 + t * 0.35 + layer * 0.22).sin()
        + big * 0.38 * (y * k2 - t * 0.55 + layer * 0.45).sin()
}

fn paint_edges(bounds: Bounds<Pixels>, accent: Hsla, t: f32, window: &mut Window) {
    let (ox, oy) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let step = 6.0;
    let steps = (h / step).ceil() as usize + 1;

    for right in [false, true] {
        // Right side in antiphase, so it doesn't look mirrored.
        let t = if right { t + 11.0 } else { t };
        let x_at = |y: f32, layer: f32| {
            let d = edge_offset(y, h, w, t, layer);
            if right {
                ox + w - d
            } else {
                ox + d
            }
        };

        // Filled band, very light.
        let mut fill = PathBuilder::fill();
        let edge = if right { ox + w } else { ox };
        fill.move_to(pt(edge, oy));
        for i in 0..steps {
            let y = (i as f32 * step).min(h);
            fill.line_to(pt(x_at(y, 0.0), oy + y));
        }
        fill.line_to(pt(edge, oy + h));
        fill.close();
        if let Ok(path) = fill.build() {
            window.paint_path(path, accent.opacity(0.07));
        }

        // Outlines: the band's and two more further in, each lighter.
        for (layer, alpha, width) in [(0.0, 0.30, 1.6), (1.0, 0.18, 1.3), (2.0, 0.10, 1.1)] {
            let mut line = PathBuilder::stroke(px(width));
            line.move_to(pt(x_at(0.0, layer), oy));
            for i in 1..steps {
                let y = (i as f32 * step).min(h);
                line.line_to(pt(x_at(y, layer), oy + y));
            }
            if let Ok(path) = line.build() {
                window.paint_path(path, accent.opacity(alpha));
            }
        }
    }
}

/// Closed or open polygon from a parametric curve over `[a, b]`.
fn param_path(
    stroke: Option<f32>,
    a: f32,
    b: f32,
    segments: usize,
    f: impl Fn(f32) -> (f32, f32),
    close: bool,
) -> Option<Path<Pixels>> {
    let mut builder = match stroke {
        Some(width) => PathBuilder::stroke(px(width)),
        None => PathBuilder::fill(),
    };
    let (x, y) = f(a);
    builder.move_to(pt(x, y));
    for i in 1..=segments {
        let (x, y) = f(a + (b - a) * i as f32 / segments as f32);
        builder.line_to(pt(x, y));
    }
    if close {
        builder.close();
    }
    builder.build().ok()
}

fn paint_particles(bounds: Bounds<Pixels>, accent: Hsla, t: f32, window: &mut Window) {
    let (ox, oy) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let travel = h + 60.0;
    for p in particles() {
        // Rises and starts again at the bottom.
        let y = (p.y * travel - p.speed * t).rem_euclid(travel) - 30.0;
        let x = p.x * w + 10.0 * (t * 0.5 + p.phase).sin();
        // Fades out smoothly near the top and bottom.
        let edge_fade = (y / 90.0).clamp(0.0, 1.0) * ((h - y) / 90.0).clamp(0.0, 1.0);
        let alpha = 0.5 * edge_fade;
        if alpha <= 0.01 {
            continue;
        }
        let (cx, cy, r) = (ox + x, oy + y, p.size / 2.0);
        let spin = t * 0.25 + p.phase;
        let path = match p.shape {
            Shape::Ring => param_path(
                Some(1.5),
                0.0,
                TAU,
                24,
                |a| (cx + r * a.cos(), cy + r * a.sin()),
                true,
            ),
            Shape::Pebble => param_path(
                Some(1.5),
                0.0,
                TAU,
                32,
                |a| {
                    // Superellipse: square with very round corners, slightly tilted.
                    let (c, s) = (a.cos(), a.sin());
                    let ex = c.signum() * c.abs().powf(0.5) * r * 1.15;
                    let ey = s.signum() * s.abs().powf(0.5) * r * 0.9;
                    let (rc, rs) = ((spin * 0.3).cos(), (spin * 0.3).sin());
                    (cx + ex * rc - ey * rs, cy + ex * rs + ey * rc)
                },
                true,
            ),
            Shape::Polygon(n) | Shape::SolidPolygon(n) => {
                let stroke = matches!(p.shape, Shape::Polygon(_)).then_some(1.5);
                // Spins slowly; the vertices stay on the circle of radius `r`.
                param_path(
                    stroke,
                    spin * 0.6,
                    spin * 0.6 + TAU,
                    n as usize,
                    |a| (cx + r * a.cos(), cy + r * a.sin()),
                    true,
                )
            }
            Shape::Dot => param_path(
                None,
                0.0,
                TAU,
                12,
                |a| (cx + r * a.cos(), cy + r * a.sin()),
                true,
            ),
        };
        if let Some(path) = path {
            window.paint_path(path, accent.opacity(alpha));
        }
    }
}

/// Background layer: fills the whole parent (which must be `relative`).
pub fn backdrop(accent: Hsla, cx: &App) -> impl IntoElement {
    let animated = motion(cx);
    div()
        .id("entry-backdrop")
        .test_support()
        .aria_label(if animated {
            t!("appearance.motion_on_lower")
        } else {
            t!("appearance.motion_off_lower")
        })
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    let t = seconds(animated);
                    paint_edges(bounds, accent, t, window);
                    paint_particles(bounds, accent, t, window);
                    // Animates even without focus (the window is still visible); only stops
                    // when minimized, when nobody sees it and it isn't worth spending CPU.
                    if animated && !crate::tray::is_minimized(window) {
                        window.request_animation_frame();
                    }
                },
            )
            .size_full(),
        )
}

#[cfg(test)]
mod tests {
    // No `use super::*`: `gpui_kit::*` brings in another `test` attribute.
    use super::{edge_offset, particles, PARTICLES};

    #[test]
    fn waves_stay_near_the_edges() {
        for (w, h) in [(800.0, 600.0), (1100.0, 720.0), (1920.0, 1080.0)] {
            let mut t = 0.0;
            while t < 120.0 {
                let mut y = 0.0;
                while y <= h {
                    for layer in [0.0, 1.0, 2.0] {
                        let d = edge_offset(y, h, w, t, layer);
                        // Never invades the middle (the form card is ~400 px).
                        assert!(d > 0.0 && d < w * 0.5 - 220.0, "w={w} y={y} t={t} d={d}");
                    }
                    y += 37.0;
                }
                t += 3.7;
            }
        }
    }

    #[test]
    fn particles_are_fixed_and_central() {
        let a: Vec<f32> = particles().iter().map(|p| p.x).collect();
        assert_eq!(a.len(), PARTICLES);
        assert!(a.iter().all(|x| (0.2..0.8).contains(x)));
        assert!(particles().iter().all(|p| p.speed > 0.0 && p.size > 0.0));
    }
}
