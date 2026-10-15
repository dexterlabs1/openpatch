//! Room illumination affects reflective rack surfaces, never host chrome or DSP.
//! Ambient tint changes premultiplied RGB in place, preserving alpha, paint order
//! and clip rectangles. Emission is tagged where it is originally painted, so a
//! later cable still covers a lamp or scope trace.
use eframe::egui::{self, vec2, Color32, Pos2, Rect};
use std::{cell::RefCell, ops::Range};

pub(crate) fn brightness(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        1.0
    }
}

pub(crate) fn tint(color: Color32, amount: f32) -> Color32 {
    if color == Color32::PLACEHOLDER || amount == 1.0 {
        return color;
    }
    Color32::from_rgba_premultiplied(
        (f32::from(color.r()) * amount).round() as u8,
        (f32::from(color.g()) * amount).round() as u8,
        (f32::from(color.b()) * amount).round() as u8,
        color.a(),
    )
}

pub(crate) fn next(painter: &egui::Painter) -> usize {
    painter
        .ctx()
        .graphics_mut(|g| g.entry(painter.layer_id()).next_idx().0)
}

/// Keep a visible lens at overview zoom without enlarging any interaction target.
pub(crate) fn lamp_radius(radius: f32) -> f32 {
    (radius * 1.65).max(1.1)
}

pub(crate) fn port_lamp_radius(zoom: f32) -> f32 {
    (3.2 * zoom).max(1.1)
}

/// One bounded additive mesh, with a continuous falloff to transparent black.
/// Rounded rectangles use the same profile as circular LEDs, so meters and
/// plugs belong to one lighting system. No blur render target or DSP work.
fn bloom(painter: &egui::Painter, rect: Rect, color: Color32, intensity: f32) {
    if intensity <= 0.001 {
        return;
    }
    const SEGMENTS: usize = 24;
    const PROFILE: [(f32, f32); 5] = [
        (0.0, 0.38),
        (0.55, 0.27),
        (1.4, 0.13),
        (2.8, 0.035),
        (4.8, 0.0),
    ];
    // Cache the unit perimeter: hundreds of rack lights must not each evaluate
    // trigonometric functions every animation frame.
    static PERIMETER: std::sync::OnceLock<[[f32; 2]; SEGMENTS]> = std::sync::OnceLock::new();
    let perimeter = PERIMETER.get_or_init(|| {
        std::array::from_fn(|i| {
            let angle = std::f32::consts::TAU * (i as f32 + 0.5) / SEGMENTS as f32;
            let (sin, cos) = angle.sin_cos();
            [cos, sin]
        })
    });
    let half = rect.size() * 0.5;
    let corner = half.x.min(half.y);
    let straight = half - vec2(corner, corner);
    let spread = corner.max(1.0);
    let mut mesh = egui::Mesh::default();
    mesh.vertices.reserve(SEGMENTS * PROFILE.len());
    mesh.indices.reserve(SEGMENTS * (PROFILE.len() - 1) * 6);
    for (expansion, energy) in PROFILE {
        let radius = corner + spread * expansion;
        let color = additive(color, intensity * energy);
        for [x, y] in perimeter {
            let position = rect.center()
                + vec2(straight.x * x.signum(), straight.y * y.signum())
                + vec2(*x, *y) * radius;
            mesh.colored_vertex(position, color);
        }
    }
    for ring in 0..PROFILE.len() - 1 {
        for segment in 0..SEGMENTS {
            let a = (ring * SEGMENTS + segment) as u32;
            let b = (ring * SEGMENTS + (segment + 1) % SEGMENTS) as u32;
            let c = a + SEGMENTS as u32;
            let d = b + SEGMENTS as u32;
            mesh.add_triangle(a, c, b);
            mesh.add_triangle(b, c, d);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
}

fn additive(color: Color32, amount: f32) -> Color32 {
    Color32::from_rgb_additive(
        (f32::from(color.r()) * amount).round().clamp(0.0, 255.0) as u8,
        (f32::from(color.g()) * amount).round().clamp(0.0, 255.0) as u8,
        (f32::from(color.b()) * amount).round().clamp(0.0, 255.0) as u8,
    )
}

pub(crate) struct RoomLighting {
    amount: f32,
    protected: RefCell<Vec<Range<usize>>>,
}

impl Default for RoomLighting {
    fn default() -> Self {
        Self {
            amount: 1.0,
            protected: RefCell::default(),
        }
    }
}

impl RoomLighting {
    fn bloom_amount(&self) -> f32 {
        0.12 + 0.88 * (1.0 - self.amount).powf(1.5)
    }

    pub(crate) fn amount(&self) -> f32 {
        self.amount
    }
    pub(crate) fn set(&mut self, amount: f32) {
        self.amount = brightness(amount);
    }
    pub(crate) fn begin(&self, painter: &egui::Painter) -> usize {
        self.protected.borrow_mut().clear();
        if self.amount < 1.0 {
            next(painter)
        } else {
            0
        }
    }
    pub(crate) fn preserve(&self, painter: &egui::Painter, start: usize) {
        if self.amount < 1.0 {
            self.protected.borrow_mut().push(start..next(painter));
        }
    }
    pub(crate) fn finish(&self, painter: &egui::Painter, start: usize) {
        if self.amount == 1.0 {
            return;
        }
        let end = next(painter);
        let mut protected = self.protected.borrow_mut();
        // Ranges can nest (a screen can contain tagged traces), so merge by
        // advancing one cursor rather than searching every range for each shape.
        protected.sort_unstable_by_key(|range| range.start);
        let mut cursor = start;
        painter.ctx().graphics_mut(|graphics| {
            let list = graphics.entry(painter.layer_id());
            let mut dim = |range: Range<usize>| {
                let amount = self.amount;
                for index in range {
                    list.mutate_shape(egui::layers::ShapeIdx(index), |clipped| {
                        egui::epaint::shape_transform::adjust_colors(
                            &mut clipped.shape,
                            move |color| {
                                *color = tint(*color, amount);
                            },
                        );
                    });
                }
            };
            for range in protected.iter() {
                let lo = range.start.clamp(start, end);
                let hi = range.end.clamp(start, end);
                if cursor < lo {
                    dim(cursor..lo);
                }
                cursor = cursor.max(hi);
            }
            if cursor < end {
                dim(cursor..end);
            }
        });
    }

    fn lens_color(&self, original: Color32, strength: f32, floor: f32) -> Color32 {
        if self.amount == 1.0 {
            return original;
        }
        let strength = signal(strength);
        let original_level = floor + (1.0 - floor) * strength;
        let lit_floor = floor * self.amount;
        let lit_level = lit_floor + (1.0 - lit_floor) * strength;
        tint(
            original,
            if original_level > 0.0 {
                lit_level / original_level
            } else {
                self.amount
            },
        )
    }

    /// Only the passive lens floor dims; the actual DSP signal emits light.
    pub(crate) fn circle(
        &self,
        painter: &egui::Painter,
        center: Pos2,
        radius: f32,
        original: Color32,
        strength: f32,
        floor: f32,
    ) {
        let start = if self.amount < 1.0 { next(painter) } else { 0 };
        let strength = signal(strength);
        let radius = lamp_radius(radius);
        let glow = strength.sqrt() * self.bloom_amount();
        bloom(
            painter,
            Rect::from_center_size(center, vec2(radius, radius) * 2.0),
            original,
            glow,
        );
        painter.circle_filled(center, radius, self.lens_color(original, strength, floor));
        if strength > 0.001 {
            painter.circle_filled(center, radius * 0.48, additive(original, glow * 0.4));
        }
        self.preserve(painter, start);
    }

    /// Plug emission is already additive and calibrated from endpoint telemetry.
    pub(crate) fn port(&self, painter: &egui::Painter, center: Pos2, zoom: f32, color: Color32) {
        let start = if self.amount < 1.0 { next(painter) } else { 0 };
        let radius = port_lamp_radius(zoom);
        bloom(
            painter,
            Rect::from_center_size(center, vec2(radius, radius) * 2.0),
            color,
            self.bloom_amount(),
        );
        painter.circle_filled(center, radius, color);
        painter.circle_filled(
            center,
            radius * 0.48,
            additive(color, self.bloom_amount() * 0.4),
        );
        self.preserve(painter, start);
    }

    pub(crate) fn rect(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        rounding: f32,
        original: Color32,
        strength: f32,
        floor: f32,
    ) {
        let start = if self.amount < 1.0 { next(painter) } else { 0 };
        let strength = signal(strength);
        let rect = Rect::from_center_size(
            rect.center(),
            vec2((rect.width() * 1.2).max(1.0), rect.height()),
        );
        bloom(
            painter,
            rect,
            original,
            strength.sqrt() * self.bloom_amount(),
        );
        painter.rect_filled(rect, rounding, self.lens_color(original, strength, floor));
        self.preserve(painter, start);
    }
}

fn signal(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bloom_is_bounded_additive_and_fades_to_zero_without_a_dark_edge() {
        let ctx = egui::Context::default();
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            let painter = ctx.layer_painter(egui::LayerId::background());
            bloom(
                &painter,
                Rect::from_center_size(Pos2::ZERO, vec2(2.2, 2.2)),
                Color32::GREEN,
                1.0,
            );
        });
        let mesh = output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Mesh(mesh) => Some(mesh),
                _ => None,
            })
            .unwrap();
        assert_eq!(mesh.vertices.len(), 120);
        assert!(mesh
            .vertices
            .iter()
            .all(|v| v.color.a() == 0 && v.pos.is_finite()));
        assert!(mesh.vertices[96..]
            .iter()
            .all(|v| v.color == Color32::TRANSPARENT));
        assert!(mesh.vertices[0].color.g() > mesh.vertices[72].color.g());
        assert!(lamp_radius(0.5) >= 1.1);
        assert!(port_lamp_radius(0.25) >= 1.1);
    }

    #[test]
    fn missing_or_zero_signal_never_creates_bloom() {
        for strength in [0.0, f32::NAN, f32::NEG_INFINITY] {
            let ctx = egui::Context::default();
            let mut lighting = RoomLighting::default();
            lighting.set(0.0);
            let output = ctx.run(egui::RawInput::default(), |ctx| {
                let painter = ctx.layer_painter(egui::LayerId::background());
                lighting.circle(&painter, Pos2::ZERO, 2.0, Color32::GREEN, strength, 0.12);
            });
            assert!(!output
                .shapes
                .iter()
                .any(|s| matches!(s.shape, egui::Shape::Mesh(_))));
        }
    }

    #[test]
    fn illumination_is_finite_and_rgb_only() {
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(brightness(value), 1.0);
        }
        assert_eq!(brightness(-2.0), 0.0);
        assert_eq!(brightness(2.0), 1.0);
        for alpha in [0, 20, 128, 255] {
            let color = Color32::from_rgba_premultiplied(10, 15, 20, alpha);
            assert_eq!(tint(color, 0.1).a(), alpha);
            assert_eq!(tint(color, 1.0), color);
        }
        assert_eq!(tint(Color32::PLACEHOLDER, 0.1), Color32::PLACEHOLDER);
    }
}
