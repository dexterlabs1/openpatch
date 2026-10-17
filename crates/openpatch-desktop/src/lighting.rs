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
    (radius * 2.1).max(1.3)
}

pub(crate) fn port_lamp_radius(zoom: f32) -> f32 {
    (4.4 * zoom).max(1.3)
}

/// One bounded additive mesh, with a continuous falloff to transparent black.
/// Rounded rectangles use the same profile as circular LEDs, so meters and
/// plugs belong to one lighting system. No blur render target or DSP work.
fn bloom(painter: &egui::Painter, rect: Rect, color: Color32, intensity: f32, diffusion: f32) {
    if intensity <= 0.001 {
        return;
    }
    const SEGMENTS: usize = 24;
    const PROFILE: [(f32, f32); 5] = [
        (0.0, 0.48),
        (0.65, 0.34),
        (1.7, 0.18),
        (3.2, 0.055),
        (5.2, 0.0),
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
    let spread = corner.max(1.0) * diffusion;
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
        // Keep the light concentrated in its lens, even with the room off.
        0.08 + 0.18 * (1.0 - self.amount).powf(1.5)
    }

    fn bloom_diffusion(&self) -> f32 {
        0.12 + 0.04 * (1.0 - self.amount).powf(1.5)
    }

    /// Static display illumination, distinct from signal-driven lamps. The
    /// halo is bounded in screen points, so wide displays do not wash out a rack.
    pub(crate) fn screen_glow(&self, painter: &egui::Painter, rect: Rect, zoom: f32) {
        let start = next(painter);
        let corner = (rect.width().min(rect.height()) * 0.5).max(1.0);
        let dark = (1.0 - self.amount).powf(1.5);
        bloom(
            painter,
            rect,
            Color32::from_rgb(104, 155, 119),
            0.004 + 0.016 * dark,
            (0.45 + 0.25 * dark) * zoom / corner,
        );
        self.preserve(painter, start);
    }

    pub(crate) fn setting_glow(&self, color: Color32, strength: f32) -> Color32 {
        additive(color, strength * self.bloom_amount())
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
        let strength = signal(strength);
        let original_level = floor + (1.0 - floor) * strength;
        let lit_floor = floor * self.amount;
        // Lift small measured voltages perceptually while preserving a truly
        // dark zero. Hue and polarity still come directly from the caller.
        let lit_level = lit_floor + (1.0 - lit_floor) * strength.sqrt();
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
        let emitter = tint(
            original,
            1.0 / (floor + (1.0 - floor) * strength).max(0.001),
        );
        bloom(
            painter,
            Rect::from_center_size(center, vec2(radius, radius) * 2.0),
            emitter,
            glow,
            self.bloom_diffusion(),
        );
        painter.circle_filled(center, radius, self.lens_color(original, strength, floor));
        if strength > 0.001 {
            painter.circle_filled(center, radius * 0.58, additive(emitter, glow * 0.25));
        }
        self.preserve(painter, start);
    }

    /// Plug emission is already additive and calibrated from endpoint telemetry.
    pub(crate) fn port(&self, painter: &egui::Painter, center: Pos2, zoom: f32, color: Color32) {
        if color.r() == 0 && color.g() == 0 && color.b() == 0 {
            return;
        }
        let start = if self.amount < 1.0 { next(painter) } else { 0 };
        let radius = port_lamp_radius(zoom);
        bloom(
            painter,
            Rect::from_center_size(center, vec2(radius, radius) * 2.0),
            color,
            self.bloom_amount(),
            self.bloom_diffusion(),
        );
        painter.circle_filled(center, radius, color);
        painter.circle_filled(
            center,
            radius * 0.58,
            additive(color, self.bloom_amount() * 0.25),
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
        let strength = signal(strength);
        if floor == 0.0 && strength == 0.0 {
            return;
        }
        let start = if self.amount < 1.0 { next(painter) } else { 0 };
        let rect = Rect::from_center_size(
            rect.center(),
            vec2((rect.width() * 1.2).max(1.0), rect.height()),
        );
        bloom(
            painter,
            rect,
            if floor == 0.0 {
                original
            } else {
                tint(
                    original,
                    1.0 / (floor + (1.0 - floor) * strength).max(0.001),
                )
            },
            strength.sqrt() * self.bloom_amount(),
            self.bloom_diffusion(),
        );
        // A zero-floor meter already encodes its measured value in its filled
        // geometry and supplies an unscaled accent, rather than a dimmed lens.
        let lens = if floor == 0.0 {
            original
        } else {
            self.lens_color(original, strength, floor)
        };
        painter.rect_filled(rect, rounding, lens);
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
    fn dark_room_reveals_wider_brighter_spill_for_leds_ports_and_meters() {
        let mut room_energy = Vec::new();
        let mut room_extent = Vec::new();
        for amount in [1.0, 0.5, 0.0] {
            let ctx = egui::Context::default();
            let mut lighting = RoomLighting::default();
            lighting.set(amount);
            let output = ctx.run(egui::RawInput::default(), |ctx| {
                let painter = ctx.layer_painter(egui::LayerId::background());
                lighting.circle(
                    &painter,
                    Pos2::ZERO,
                    2.0,
                    Color32::GREEN.gamma_multiply(0.12 + 0.88 * 0.01),
                    0.01,
                    0.12,
                );
                lighting.port(
                    &painter,
                    Pos2::new(100.0, 0.0),
                    1.0,
                    Color32::from_rgb_additive(0, 90, 0),
                );
                lighting.rect(
                    &painter,
                    Rect::from_center_size(Pos2::new(200.0, 0.0), vec2(3.0, 12.0)),
                    1.0,
                    Color32::GREEN,
                    0.4,
                    0.0,
                );
            });
            let meshes: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Mesh(mesh) => Some(mesh),
                    _ => None,
                })
                .collect();
            assert_eq!(meshes.len(), 3);
            room_energy.push(
                meshes
                    .iter()
                    .map(|mesh| {
                        mesh.vertices
                            .iter()
                            .map(|v| u32::from(v.color.g()))
                            .sum::<u32>()
                    })
                    .collect::<Vec<_>>(),
            );
            room_extent.push(
                meshes
                    .iter()
                    .map(|mesh| mesh.calc_bounds().width())
                    .collect::<Vec<_>>(),
            );
            assert!(meshes.iter().all(|mesh| mesh.vertices.len() == 120));
        }
        for light in 0..3 {
            assert!(room_energy[2][light] > room_energy[1][light]);
            assert!(room_energy[1][light] > room_energy[0][light]);
            assert!(room_extent[2][light] > room_extent[1][light]);
            assert!(room_extent[1][light] > room_extent[0][light]);
        }
    }

    #[test]
    fn screen_spill_is_bounded_preserved_and_stronger_in_a_dark_room() {
        for size in [vec2(290.0, 49.0), vec2(100.0, 240.0), vec2(250.0, 25.0)] {
            let mut energy = Vec::new();
            for room in [1.0, 0.0] {
                let ctx = egui::Context::default();
                let mut lighting = RoomLighting::default();
                lighting.set(room);
                let rect = Rect::from_min_size(Pos2::ZERO, size);
                let output = ctx.run(egui::RawInput::default(), |ctx| {
                    let painter = ctx.layer_painter(egui::LayerId::background());
                    let start = lighting.begin(&painter);
                    lighting.screen_glow(&painter, rect, 1.0);
                    lighting.finish(&painter, start);
                });
                let egui::Shape::Mesh(mesh) = &output.shapes[0].shape else {
                    panic!("display halo must be one bounded mesh")
                };
                assert_eq!(mesh.vertices.len(), 120);
                assert!(mesh.vertices.iter().all(|v| v.color.a() == 0));
                let bounds = mesh.calc_bounds();
                assert!(bounds.width() <= size.x + 16.7);
                assert!(bounds.height() <= size.y + 16.7);
                energy.push(
                    mesh.vertices
                        .iter()
                        .map(|v| u32::from(v.color.g()))
                        .sum::<u32>(),
                );
            }
            assert!(
                energy[1] > energy[0] * 4,
                "room-off screen spill must stay visible"
            );
        }
    }

    #[test]
    fn dark_room_led_spill_stays_local_and_keeps_additive_headroom() {
        let ctx = egui::Context::default();
        let mut lighting = RoomLighting::default();
        lighting.set(0.0);
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            let painter = ctx.layer_painter(egui::LayerId::background());
            lighting.circle(&painter, Pos2::ZERO, 3.2, Color32::GREEN, 1.0, 0.12);
        });
        let mesh = output
            .shapes
            .iter()
            .find_map(|s| {
                if let egui::Shape::Mesh(m) = &s.shape {
                    Some(m)
                } else {
                    None
                }
            })
            .unwrap();
        assert!(
            mesh.calc_bounds().width() <= 25.0,
            "spill must stay close to the lens on a dense lamp grid"
        );
        assert!(
            mesh.vertices
                .iter()
                .all(|v| v.color.r().max(v.color.g()).max(v.color.b()) <= 40),
            "halo must leave headroom instead of clipping into a neon cloud"
        );
    }

    #[test]
    fn missing_or_zero_signal_never_creates_bloom() {
        for strength in [0.0, -1.0, f32::NAN, f32::NEG_INFINITY, f32::INFINITY] {
            let ctx = egui::Context::default();
            let mut lighting = RoomLighting::default();
            lighting.set(0.0);
            let output = ctx.run(egui::RawInput::default(), |ctx| {
                let painter = ctx.layer_painter(egui::LayerId::background());
                lighting.circle(&painter, Pos2::ZERO, 2.0, Color32::GREEN, strength, 0.12);
                lighting.port(&painter, Pos2::ZERO, 1.0, Color32::TRANSPARENT);
                lighting.rect(
                    &painter,
                    Rect::from_center_size(Pos2::ZERO, vec2(3.0, 20.0)),
                    0.0,
                    Color32::GREEN,
                    strength,
                    0.0,
                );
            });
            assert!(!output
                .shapes
                .iter()
                .any(|s| matches!(s.shape, egui::Shape::Mesh(_))));
            assert!(output.shapes.iter().all(|s| match &s.shape {
                egui::Shape::Circle(circle) => {
                    circle.fill.r() == 0 && circle.fill.g() == 0 && circle.fill.b() == 0
                }
                egui::Shape::Rect(_) => false,
                _ => true,
            }));
        }
    }

    #[test]
    fn perceptual_lens_response_preserves_zero_polarity_and_voltage_order() {
        let mut lighting = RoomLighting::default();
        for room in [0.0, 1.0] {
            lighting.set(room);
            for base in [Color32::GREEN, Color32::RED] {
                let lens = |strength: f32| {
                    lighting.lens_color(base.gamma_multiply(0.12 + strength * 0.88), strength, 0.12)
                };
                let weak = lens(0.01);
                let strong = lens(0.25);
                let component = |c: Color32| if base == Color32::GREEN { c.g() } else { c.r() };
                assert!(component(weak) > component(lens(0.0)));
                assert!(component(strong) > component(weak));
                assert_eq!(weak.g() == 0, base == Color32::RED);
                assert_eq!(weak.r() == 0, base == Color32::GREEN);
                if room == 0.0 {
                    assert_eq!(component(lens(0.0)), 0);
                }
            }
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
