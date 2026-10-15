//! Room illumination affects reflective rack surfaces, never host chrome or DSP.
//! Ambient tint changes premultiplied RGB in place, preserving alpha, paint order
//! and clip rectangles. Emission is tagged where it is originally painted, so a
//! later cable still covers a lamp or scope trace.
use eframe::egui::{self, Color32, Pos2, Rect};
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

    /// The source's existing color/opacity is retained at full room brightness.
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
        let glow = strength * (1.0 - self.amount);
        if glow > 0.001 {
            for (scale, alpha) in [(1.9, 13.0), (1.4, 24.0)] {
                painter.circle_filled(
                    center,
                    radius * scale,
                    Color32::from_rgba_unmultiplied(
                        original.r(),
                        original.g(),
                        original.b(),
                        (alpha * glow).round() as u8,
                    ),
                );
            }
        }
        painter.circle_filled(center, radius, self.lens_color(original, strength, floor));
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
        let glow = signal(strength) * (1.0 - self.amount);
        if glow > 0.001 {
            for (expand, alpha) in [(2.0, 10.0), (1.0, 19.0)] {
                painter.rect_filled(
                    rect.expand(expand * rect.height() / 6.0),
                    rounding + expand,
                    Color32::from_rgba_unmultiplied(
                        original.r(),
                        original.g(),
                        original.b(),
                        (alpha * glow).round() as u8,
                    ),
                );
            }
        }
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
