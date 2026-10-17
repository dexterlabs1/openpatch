// Signal Atlas analog voice family, extending the approved two-panel design. Design coordinates follow the
// reviewed 420×720 artwork; existing HP widths and rack height stay unchanged.
fn atlas_voice(kind: &str) -> bool {
    matches!(
        kind,
        "adsr" | "ladder_vcf" | "analog_vco" | "analog_vca" | "lfo" | "lowpass" | "gain"
    )
}

#[derive(Clone, Copy)]
struct AtlasVoiceLayout {
    width: f32,
}
impl AtlasVoiceLayout {
    fn point(self, x: f32, y: f32) -> Pos2 {
        pos2(x * self.width / 420.0, y * PANEL_HEIGHT / 720.0)
    }
    fn unit(self) -> f32 {
        (self.width / 420.0).clamp(0.5, PANEL_HEIGHT / 720.0)
    }
    fn control(self, kind: &str, name: &str) -> (Pos2, f32) {
        let (x, y, radius) = match (kind, name) {
            ("adsr", "attack") => (94.0, 166.0, 45.0),
            ("adsr", "decay") => (326.0, 166.0, 45.0),
            ("adsr", "sustain") => (94.0, 360.0, 45.0),
            ("adsr", _) => (326.0, 360.0, 45.0),
            ("analog_vca" | "lowpass" | "gain", _) => (210.0, 180.0, 70.0),
            ("lfo", "frequency") => (131.0, 170.0, 57.0),
            ("lfo", "amplitude") => (94.0, 360.0, 45.0),
            ("analog_vco", "frequency") | (_, "cutoff") => (131.0, 170.0, 57.0),
            ("analog_vco", "waveform") => (94.0, 360.0, 45.0),
            (_, "resonance") => (94.0, 360.0, 45.0),
            _ => (326.0, 360.0, 45.0),
        };
        (self.point(x, y), radius * self.unit())
    }
    fn value_center(self, kind: &str, name: &str) -> Pos2 {
        let (center, radius) = self.control(kind, name);
        center + vec2(0.0, radius + 53.0 * PANEL_HEIGHT / 720.0)
    }
    fn value_width(self, kind: &str, name: &str) -> f32 {
        match (kind, name) {
            ("analog_vca" | "gain", _) => 68.0,
            ("lowpass", _) => 84.0,
            ("lfo", "frequency") => 76.0,
            ("lfo", _) => 62.0,
            ("analog_vco", "waveform") => 82.0,
            ("analog_vco", "frequency") => 74.0,
            _ => 96.0 * self.width / 420.0,
        }
    }
}
fn atlas_voice_value(kind: &str, name: &str, value: f32, spec: &Value) -> String {
    if name == "resonance" || (kind == "adsr" && name == "sustain") {
        format!("{:.0}%", value * 100.0)
    } else if kind == "lfo" && matches!(name, "amplitude" | "offset") {
        let text = atlas_signal_number(f64::from(value));
        if name == "offset" && value >= 0.0 {
            format!("+{text}")
        } else {
            text
        }
    } else if kind == "gain" && name == "gain" {
        format_value(value, "x").replace('x', "×")
    } else if matches!(name, "drive" | "gain") {
        format!("{value:.2}×")
    } else {
        parameter_value(name, value, spec)
    }
}

// A small painter in the review's coordinate system. Curves and diagrams are
// vector primitives, shared by the interactive rack and browser renderer.
struct AtlasVoicePaint<'a> {
    painter: &'a egui::Painter,
    origin: Pos2,
    layout: AtlasVoiceLayout,
    zoom: f32,
}
impl AtlasVoicePaint<'_> {
    fn p(&self, x: f32, y: f32) -> Pos2 {
        self.origin + self.layout.point(x, y).to_vec2() * self.zoom
    }
    fn unit(&self) -> f32 {
        self.layout.unit() * self.zoom
    }
    fn rect(&self, x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_max(self.p(x, y), self.p(x + w, y + h))
    }
    fn text(&self, x: f32, y: f32, text: &str, size: f32, color: Color32) {
        self.painter.text(
            self.p(x, y),
            Align2::CENTER_CENTER,
            text,
            crate::panel_skin::print_font(self.painter.ctx(), size * self.unit()),
            color,
        );
    }
    fn line(&self, points: &[(f32, f32)], weight: f32, arrow: bool) {
        let points: Vec<_> = points.iter().map(|&(x, y)| self.p(x, y)).collect();
        if arrow && points.len() >= 2 {
            let end = points[points.len() - 1];
            let direction = (end - points[points.len() - 2]).normalized();
            let side = vec2(-direction.y, direction.x);
            self.painter.add(egui::Shape::convex_polygon(
                vec![
                    end,
                    end - direction * 7.0 * self.unit() + side * 3.5 * self.unit(),
                    end - direction * 7.0 * self.unit() - side * 3.5 * self.unit(),
                ],
                INK,
                Stroke::NONE,
            ));
        }
        self.painter.add(egui::Shape::line(
            points,
            Stroke::new(weight * self.unit(), INK),
        ));
    }
    fn dashed(&self, points: &[(f32, f32)]) {
        for pair in points.windows(2) {
            let from = self.p(pair[0].0, pair[0].1);
            let to = self.p(pair[1].0, pair[1].1);
            let direction = (to - from).normalized();
            let length = from.distance(to);
            let mut t = 0.0;
            while t < length {
                self.painter.line_segment(
                    [
                        from + direction * t,
                        from + direction * (t + 4.0 * self.unit()).min(length),
                    ],
                    Stroke::new(self.unit(), MUTED),
                );
                t += 8.0 * self.unit();
            }
        }
    }
    fn screen(&self, rect: Rect, lighting: &crate::lighting::RoomLighting) {
        lighting.screen_glow(self.painter, rect, self.zoom);
        self.painter.rect_filled(
            rect.expand(3.0 * self.unit()),
            4.0 * self.unit(),
            Color32::from_rgb(126, 121, 94),
        );
        self.painter
            .rect_filled(rect, 3.0 * self.unit(), Color32::from_rgb(15, 24, 18));
        self.painter.rect_stroke(
            rect,
            3.0 * self.unit(),
            Stroke::new(1.5 * self.unit(), Color32::from_rgb(5, 12, 8)),
            egui::StrokeKind::Inside,
        );
    }
}

fn paint_atlas_voice_face(painter: &egui::Painter, rect: Rect, node: &Node, zoom: f32) {
    let draw = AtlasVoicePaint {
        painter,
        origin: rect.min,
        layout: AtlasVoiceLayout {
            width: rect.width() / zoom,
        },
        zoom,
    };
    painter.rect_filled(rect, 1.0 * zoom, Color32::from_rgb(108, 108, 92));
    let face = rect.shrink(1.5 * zoom);
    // Subtle warm gradient, using one mesh rather than a generated bitmap.
    let mut mesh = egui::Mesh::default();
    for (p, c) in [
        (face.left_top(), Color32::from_rgb(242, 234, 209)),
        (face.right_top(), Color32::from_rgb(238, 229, 201)),
        (face.right_bottom(), Color32::from_rgb(224, 213, 181)),
        (face.left_bottom(), Color32::from_rgb(236, 225, 196)),
    ] {
        mesh.colored_vertex(p, c);
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
    painter.rect_stroke(
        face,
        0.0,
        Stroke::new(0.7 * zoom, Color32::from_rgb(255, 246, 220)),
        egui::StrokeKind::Inside,
    );
    // Sparse deterministic grain only when it resolves to a screen pixel.
    if zoom >= 0.8 {
        let mut grain = egui::Mesh::default();
        for i in 0..420_u32 {
            let x = 5.0 + ((i * 193 + 67) % 410) as f32;
            let y = 5.0 + ((i * 317 + 31) % 710) as f32;
            grain.add_colored_rect(
                Rect::from_center_size(draw.p(x, y), Vec2::splat(0.5 * zoom)),
                Color32::from_black_alpha(10),
            );
        }
        painter.add(egui::Shape::mesh(grain));
    }
    for x in [20.0, 400.0] {
        for y in [21.0, 699.0] {
            let center = draw.p(x, y);
            painter.circle_filled(
                center + vec2(0.6, 1.0) * zoom,
                11.0 * draw.unit(),
                Color32::from_black_alpha(100),
            );
            painter.circle_filled(center, 10.0 * draw.unit(), Color32::from_rgb(22, 30, 24));
            painter.circle_stroke(
                center,
                8.2 * draw.unit(),
                Stroke::new(draw.unit(), Color32::from_rgb(133, 144, 127)),
            );
            for direction in [vec2(1.0, 0.0), vec2(0.0, 1.0)] {
                painter.line_segment(
                    [
                        center - direction * 4.5 * draw.unit(),
                        center + direction * 4.5 * draw.unit(),
                    ],
                    Stroke::new(2.4 * draw.unit(), Color32::from_rgb(5, 12, 8)),
                );
            }
        }
    }
    painter.text(
        draw.p(42.0, 44.0),
        Align2::LEFT_CENTER,
        title(&node.kind),
        crate::panel_skin::print_font(painter.ctx(), 49.0 * draw.unit()),
        INK,
    );
    draw.line(&[(23.0, 77.0), (397.0, 77.0)], 1.0, false);
    if node.bypassed {
        draw.text(328.0, 51.0, "BYPASSED", 12.0, MUTED);
    }
}

fn paint_atlas_voice_rotary(
    painter: &egui::Painter,
    center: Pos2,
    normalized: f32,
    active: bool,
    radius: f32,
    zoom: f32,
) {
    let radius = radius * zoom;
    let u = radius / 45.0;
    let angle = |t: f32| std::f32::consts::PI * (0.75 + 1.5 * t);
    let arc: Vec<_> = (0..=48)
        .map(|i| {
            let a = angle(i as f32 / 48.0);
            center + vec2(a.cos(), a.sin()) * (radius + 13.0 * u)
        })
        .collect();
    painter.add(egui::Shape::line(
        arc,
        Stroke::new(
            if active { 1.6 * u } else { u },
            if active {
                Color32::from_rgb(169, 121, 49)
            } else {
                MUTED
            },
        ),
    ));
    for i in 0..=20 {
        let a = angle(i as f32 / 20.0);
        let d = vec2(a.cos(), a.sin());
        painter.line_segment(
            [
                center + d * (radius + 9.0 * u),
                center + d * (radius + if i % 5 == 0 { 16.0 } else { 12.0 } * u),
            ],
            Stroke::new(if i % 5 == 0 { 1.6 } else { 0.8 } * u, INK),
        );
    }
    painter.circle_filled(
        center + vec2(1.5, 3.0) * u,
        radius + 2.5 * u,
        Color32::from_black_alpha(76),
    );
    painter.circle_filled(center, radius + 2.0 * u, Color32::from_rgb(16, 23, 18));
    painter.circle_filled(center, radius, Color32::from_rgb(35, 41, 35));
    if radius >= 12.0 {
        for i in 0..56 {
            let a = std::f32::consts::TAU * i as f32 / 56.0;
            let d = vec2(a.cos(), a.sin());
            let light = (-d.x * 0.6 - d.y * 0.8).max(0.0);
            let shade = (25.0 + 44.0 * light) as u8;
            painter.line_segment(
                [center + d * (radius - 4.0 * u), center + d * (radius + u)],
                Stroke::new(2.0 * u, Color32::from_rgb(shade, shade + 3, shade)),
            );
        }
    }
    painter.circle_filled(center, radius - 6.0 * u, Color32::from_rgb(56, 59, 57));
    // Ring shading gives the rubber cap depth without a blur/render target.
    for ring in (0..20).rev() {
        let t = ring as f32 / 19.0;
        let shade = (44.0 + (1.0 - t) * 12.0) as u8;
        painter.circle_filled(
            center - vec2(1.2, 1.5) * (1.0 - t) * u,
            (radius - 6.0 * u) * (0.3 + 0.7 * t),
            Color32::from_rgb(shade, shade + 3, shade),
        );
    }
    painter.circle_stroke(
        center,
        radius - 6.0 * u,
        Stroke::new(0.8 * u, Color32::from_rgb(77, 84, 75)),
    );
    let a = angle(normalized.clamp(0.0, 1.0));
    let d = vec2(a.cos(), a.sin());
    painter.line_segment(
        [center + d * radius * 0.24, center + d * (radius - 10.0 * u)],
        Stroke::new(4.5 * u, Color32::from_rgb(230, 227, 209)),
    );
}

fn paint_atlas_voice_readout(
    painter: &egui::Painter,
    center: Pos2,
    text: &str,
    width: f32,
    zoom: f32,
) {
    paint_atlas_voice_readout_size(painter, center, text, width, zoom, 9.0);
}
fn paint_atlas_voice_readout_size(
    painter: &egui::Painter,
    center: Pos2,
    text: &str,
    width: f32,
    zoom: f32,
    font_size: f32,
) {
    let rect = Rect::from_center_size(center, vec2(width, 14.0 * zoom));
    painter.rect_filled(rect, 1.7 * zoom, Color32::from_rgb(222, 212, 184));
    painter.rect_stroke(
        rect,
        1.7 * zoom,
        Stroke::new(0.5 * zoom, Color32::from_rgb(177, 168, 140)),
        egui::StrokeKind::Inside,
    );
    let mut font = crate::panel_skin::print_font(painter.ctx(), font_size * zoom);
    let measured = painter.layout_no_wrap(text.into(), font.clone(), INK);
    font.size *= ((width - 3.0 * zoom) / measured.size().x.max(1.0)).min(1.0);
    painter.text(center, Align2::CENTER_CENTER, text, font, INK);
}

#[allow(clippy::too_many_arguments)]
fn paint_atlas_voice_socket(
    painter: &egui::Painter,
    center: Pos2,
    kind: &str,
    port: &str,
    connected: bool,
    output: bool,
    zoom: f32,
) {
    let u = AtlasVoiceLayout {
        width: panel_width(kind),
    }
    .unit()
        * zoom;
    let label = match (kind, port) {
        ("adsr", "out") => "ENV",
        (_, "signal") => "IN",
        (_, "cutoff_cv") => "V/OCT",
        (_, "pitch") => "V/OCT",
        (_, "fm") => "FM",
        (_, "cv") => "CV",
        (_, "gate") => "GATE",
        (_, "retrigger") => "RETRIG",
        _ => "OUT",
    };
    let label_center = center - vec2(0.0, 45.5 * PANEL_HEIGHT / 720.0 * zoom);
    if output {
        painter.rect_filled(
            Rect::from_center_size(label_center, vec2(78.0 * u, 24.0 * u)),
            1.2 * zoom,
            INK,
        );
    }
    painter.text(
        label_center,
        Align2::CENTER_CENTER,
        label,
        crate::panel_skin::print_font(painter.ctx(), 16.0 * u),
        if output {
            Color32::from_rgb(238, 230, 205)
        } else {
            INK
        },
    );
    paint_atlas_socket_hardware(painter, center, u, connected);
}

// Stock panels retain their caption/target geometry and share this metallic
// hardware at the same outer radius.
fn paint_atlas_socket_hardware(painter: &egui::Painter, center: Pos2, u: f32, connected: bool) {
    painter.circle_filled(
        center + vec2(1.5, 3.0) * u,
        32.0 * u,
        Color32::from_black_alpha(80),
    );
    painter.circle_filled(center, 32.0 * u, Color32::from_rgb(130, 127, 108));
    let points: Vec<_> = (0..6)
        .map(|i| {
            let a = (30.0 + 60.0 * i as f32).to_radians();
            center + vec2(a.cos(), a.sin()) * 29.0 * u
        })
        .collect();
    painter.add(egui::Shape::convex_polygon(
        points.clone(),
        Color32::from_rgb(164, 171, 154),
        Stroke::new(1.2 * u, Color32::from_rgb(104, 111, 97)),
    ));
    for i in 0..6 {
        let shade = if i < 3 {
            Color32::from_rgb(235, 236, 218)
        } else {
            Color32::from_rgb(104, 112, 98)
        };
        painter.line_segment(
            [points[i], points[(i + 1) % 6]],
            Stroke::new(2.0 * u, shade),
        );
    }
    painter.circle_filled(center, 24.0 * u, Color32::from_rgb(183, 190, 170));
    painter.circle_stroke(
        center,
        24.0 * u,
        Stroke::new(1.2 * u, Color32::from_rgb(240, 237, 218)),
    );
    painter.circle_filled(center, 20.0 * u, Color32::from_rgb(52, 61, 51));
    painter.circle_stroke(
        center,
        20.0 * u,
        Stroke::new(1.3 * u, Color32::from_rgb(133, 143, 124)),
    );
    painter.circle_filled(center, 16.0 * u, Color32::from_rgb(5, 12, 8));
    painter.circle_stroke(
        center,
        13.0 * u,
        Stroke::new(1.0 * u, Color32::from_rgb(20, 34, 24)),
    );
    if connected {
        painter.circle_stroke(
            center,
            20.0 * u,
            Stroke::new(1.1 * u, Color32::from_rgb(163, 135, 86)),
        );
    }
}

fn atlas_voice_parameter(node: &Node, name: &str, default: f64, min: f64, max: f64) -> f64 {
    f64::from(node.params.get(name).copied().unwrap_or(default as f32)).clamp(min, max)
}

fn atlas_envelope_points(node: &Node) -> Vec<(f32, f32)> {
    let a = atlas_voice_parameter(node, "attack", 0.01, 0.001, 10.0);
    let d = atlas_voice_parameter(node, "decay", 0.2, 0.001, 10.0);
    let s = atlas_voice_parameter(node, "sustain", 0.5, 0.0, 1.0) * 10.0;
    let r = atlas_voice_parameter(node, "release", 0.3, 0.001, 10.0);
    let off = a + d + 0.18;
    let end = off + r * 1.25;
    let v_off = s + (10.0 - s) * (-100.0_f64.ln() * (off - a) / d).exp();
    let mut points = Vec::with_capacity(164);
    // Sample every stage separately so a 1 ms attack remains represented when
    // release is 10 s. X remains proportional to actual configured time.
    for (start, finish) in [(0.0, a), (a, a + d), (a + d, off), (off, end)] {
        for i in 0..=40 {
            let t = start + (finish - start) * i as f64 / 40.0;
            let v = if t < a {
                12.0 * (1.0 - (-6.0_f64.ln() * t / a).exp())
            } else if t < off {
                s + (10.0 - s) * (-100.0_f64.ln() * (t - a) / d).exp()
            } else {
                v_off * (-100.0_f64.ln() * (t - off) / r).exp()
            };
            points.push((
                170.0 + (t / end * 80.0) as f32,
                303.0 - v.clamp(0.0, 10.0) as f32 * 8.3,
            ));
        }
    }
    points
}
fn atlas_ladder_points(node: &Node, rate: u32) -> Vec<(f32, f32)> {
    let max = 18000.0_f64.min(f64::from(rate) * 0.4).max(20.0);
    let fc = atlas_voice_parameter(node, "cutoff", 1200.0, 20.0, max);
    let k = 4.0 * atlas_voice_parameter(node, "resonance", 0.3, 0.0, 1.1);
    let drive = atlas_voice_parameter(node, "drive", 1.0, 0.0, 4.0);
    (0..=130)
        .map(|i| {
            let t = i as f64 / 130.0;
            let f = 20.0 * (max / 20.0).powf(t);
            let w = f / fc;
            let re = 1.0 - 6.0 * w * w + w.powi(4) + k;
            let im = 4.0 * w - 4.0 * w.powi(3);
            let db = if drive == 0.0 {
                -60.0
            } else {
                20.0 * (drive / re.hypot(im)).log10()
            };
            (
                256.0 + t as f32 * 112.0,
                237.0 - (db.clamp(-60.0, 12.0) as f32 + 60.0) / 72.0 * 91.0,
            )
        })
        .collect()
}

fn paint_atlas_voice_configuration(
    painter: &egui::Painter,
    node: &Node,
    rect: Rect,
    zoom: f32,
    rate: u32,
    lighting: &crate::lighting::RoomLighting,
) {
    let draw = AtlasVoicePaint {
        painter,
        origin: rect.min,
        layout: AtlasVoiceLayout {
            width: rect.width() / zoom,
        },
        zoom,
    };
    if matches!(node.kind.as_str(), "lfo" | "lowpass" | "gain") {
        paint_atlas_utility_configuration(&draw, node, rate, lighting);
        return;
    }
    if matches!(node.kind.as_str(), "analog_vco" | "analog_vca") {
        paint_atlas_analog_configuration(&draw, node, rate, lighting);
        return;
    }
    let quiet = Color32::from_rgb(133, 143, 123);
    let screen_ink = Color32::from_rgb(195, 200, 180);
    let grid = Color32::from_rgb(45, 59, 44);
    let amber = Color32::from_rgb(229, 183, 103);
    let start = crate::lighting::next(painter);
    if node.kind == "adsr" {
        draw.screen(draw.rect(160.0, 148.0, 100.0, 239.0), lighting);
        draw.text(210.0, 168.0, "ENVELOPE", 9.0, screen_ink);
        draw.text(210.0, 182.0, "CONFIGURED", 8.0, quiet);
        draw.text(181.0, 205.0, "10 V", 8.0, quiet);
        for y in [220.0, 262.0, 303.0] {
            painter.line_segment(
                [draw.p(170.0, y), draw.p(250.0, y)],
                Stroke::new(0.5 * draw.unit(), grid),
            );
        }
        for x in [190.0, 210.0, 230.0] {
            painter.line_segment(
                [draw.p(x, 212.0), draw.p(x, 310.0)],
                Stroke::new(0.5 * draw.unit(), grid),
            );
        }
        painter.add(egui::Shape::line(
            atlas_envelope_points(node)
                .into_iter()
                .map(|(x, y)| draw.p(x, y))
                .collect(),
            Stroke::new(2.0 * draw.unit(), amber),
        ));
        draw.text(173.0, 317.0, "0", 8.0, quiet);
        draw.text(230.0, 317.0, "TIME →", 8.0, quiet);
        draw.text(210.0, 345.0, "A · D · S · R", 9.0, screen_ink);
        draw.text(210.0, 364.0, "0–10 V NOMINAL", 8.0, quiet);
    } else {
        draw.screen(draw.rect(245.0, 109.0, 136.0, 164.0), lighting);
        draw.text(313.0, 127.0, "SMALL SIGNAL", 9.0, screen_ink);
        draw.text(
            313.0,
            139.0,
            if node.params.get("resonance").copied().unwrap_or(0.3) >= 1.0 {
                "OSCILLATION REGION"
            } else {
                "LINEARIZED · ZERO CV"
            },
            8.0,
            quiet,
        );
        for y in [161.0, 194.0, 237.0] {
            painter.line_segment(
                [draw.p(256.0, y), draw.p(368.0, y)],
                Stroke::new(0.5 * draw.unit(), grid),
            );
        }
        for x in [282.0, 312.0, 344.0] {
            painter.line_segment(
                [draw.p(x, 150.0), draw.p(x, 242.0)],
                Stroke::new(0.5 * draw.unit(), grid),
            );
        }
        painter.add(egui::Shape::line(
            atlas_ladder_points(node, rate)
                .into_iter()
                .map(|(x, y)| draw.p(x, y))
                .collect(),
            Stroke::new(2.0 * draw.unit(), amber),
        ));
        let max = 18000.0_f32.min(rate as f32 * 0.4);
        draw.text(258.0, 251.0, "20", 8.0, quiet);
        draw.text(320.0, 251.0, "Hz", 8.0, quiet);
        draw.text(359.0, 251.0, &format!("{:.0}k", max / 1000.0), 8.0, quiet);
    }
    lighting.preserve(painter, start); // configured displays emit, not signal telemetry
    if node.kind == "adsr" {
        for (x, y, label) in [
            (35.0, 102.0, "A"),
            (277.0, 102.0, "D"),
            (35.0, 297.0, "S"),
            (277.0, 297.0, "R"),
        ] {
            draw.text(x, y, label, 16.0, INK);
        }
        for points in [
            &[(168.0, 119.0), (168.0, 91.0), (194.0, 91.0)][..],
            &[(226.0, 91.0), (226.0, 119.0), (252.0, 119.0)],
            &[(171.0, 420.0), (171.0, 447.0), (197.0, 447.0)],
            &[(223.0, 420.0), (223.0, 447.0), (249.0, 447.0)],
        ] {
            draw.line(points, 1.9, false);
        }
        for (x, y, rise) in [
            (171.0, 94.0, true),
            (229.0, 94.0, false),
            (226.0, 422.0, false),
        ] {
            let pts: Vec<_> = (0..=20)
                .map(|i| {
                    let t = i as f32 / 20.0;
                    (
                        x + 20.0 * t,
                        y + 22.0
                            * if rise {
                                (-4.0 * t).exp()
                            } else {
                                1.0 - (-4.0 * t).exp()
                            },
                    )
                })
                .collect();
            draw.line(&pts, 1.9, false);
        }
        draw.line(&[(174.0, 433.0), (194.0, 433.0)], 1.9, false);
        // No stage telemetry exists for the legacy analog ADSR. These are
        // passive lenses, never inferred from gate or a running animation.
        for (x, y) in [(66.0, 221.0), (298.0, 221.0), (64.0, 415.0), (294.0, 415.0)] {
            painter.circle_filled(
                draw.p(x, y),
                6.5 * draw.unit(),
                Color32::from_rgb(102, 93, 60),
            );
            painter.circle_filled(
                draw.p(x - 0.5, y - 0.8),
                4.5 * draw.unit(),
                Color32::from_rgb(76, 70, 44),
            );
        }
        draw.text(
            210.0,
            492.0,
            "GATE HIGH → A · D · S    GATE LOW → R",
            10.0,
            INK,
        );
        for (i, label) in ["A", "D", "S", "R"].iter().enumerate() {
            let x = 75.0 + 90.0 * i as f32;
            painter.circle_stroke(
                draw.p(x, 515.0),
                17.0 * draw.unit(),
                Stroke::new(1.5 * draw.unit(), INK),
            );
            draw.text(x, 515.0, label, 16.0, INK);
            if i < 3 {
                draw.line(&[(x + 21.0, 515.0), (x + 69.0, 515.0)], 1.9, true);
            }
        }
        draw.line(&[(87.0, 566.0), (87.0, 548.0), (75.0, 538.0)], 1.0, true);
        draw.dashed(&[
            (210.0, 566.0),
            (210.0, 549.0),
            (103.0, 549.0),
            (88.0, 533.0),
        ]);
        draw.text(210.0, 549.0, "RESTART A WHILE GATED", 8.0, INK);
    } else {
        let max = 18000.0_f32.min(rate as f32 * 0.4);
        draw.text(
            131.0,
            93.0,
            &format!("{:.0} Hz", (20.0 * max).sqrt()),
            8.0,
            INK,
        );
        draw.text(58.0, 241.0, "20", 10.0, INK);
        draw.text(207.0, 241.0, &format!("{:.0}k", max / 1000.0), 10.0, INK);
        draw.text(313.0, 292.0, "FOUR NONLINEAR STAGES", 10.0, INK);
        draw.line(&[(28.0, 507.0), (391.0, 507.0)], 1.9, true);
        painter.circle_filled(
            draw.p(48.0, 507.0),
            11.0 * draw.unit(),
            Color32::from_rgb(233, 223, 195),
        );
        painter.circle_stroke(
            draw.p(48.0, 507.0),
            11.0 * draw.unit(),
            Stroke::new(1.5 * draw.unit(), INK),
        );
        draw.text(48.0, 507.0, "+", 16.0, INK);
        for i in 0..4 {
            let x = 78.0 + 75.0 * i as f32;
            painter.rect_filled(
                draw.rect(x, 494.0, 48.0, 26.0),
                1.0 * draw.unit(),
                Color32::from_rgb(233, 223, 195),
            );
            painter.rect_stroke(
                draw.rect(x, 494.0, 48.0, 26.0),
                1.0 * draw.unit(),
                Stroke::new(1.5 * draw.unit(), INK),
                egui::StrokeKind::Inside,
            );
            draw.text(x + 24.0, 507.0, &(i + 1).to_string(), 16.0, INK);
        }
        draw.line(
            &[(359.0, 507.0), (359.0, 540.0), (48.0, 540.0), (48.0, 523.0)],
            1.9,
            true,
        );
        draw.text(204.0, 532.0, "RESONANCE: NEGATIVE FEEDBACK", 8.0, INK);
        draw.text(39.0, 533.0, "−", 16.0, INK);
        draw.line(
            &[(52.0, 610.0), (28.0, 610.0), (28.0, 507.0), (35.0, 507.0)],
            1.9,
            true,
        );
        draw.line(&[(391.0, 507.0), (391.0, 610.0), (370.0, 610.0)], 1.9, true);
        draw.dashed(&[
            (210.0, 575.0),
            (210.0, 476.0),
            (231.0, 476.0),
            (231.0, 175.0),
            (207.0, 175.0),
        ]);
        draw.text(204.0, 463.0, "DRIVE → FOUR POLES → OUT", 8.0, INK);
    }
    draw.line(&[(22.0, 655.0), (398.0, 655.0)], 1.9, false);
    for (x, text) in if node.kind == "adsr" {
        [
            (87.0, "SCHMITT GATE"),
            (210.0, "RETRIGGER"),
            (333.0, "ENVELOPE OUT"),
        ]
    } else {
        [
            (87.0, "AUDIO IN"),
            (210.0, "1 V / OCT"),
            (333.0, "LOW-PASS OUT"),
        ]
    } {
        draw.text(x, 671.0, text, 10.0, INK);
    }
}

fn paint_atlas_voice_control_print(
    painter: &egui::Painter,
    center: Pos2,
    kind: &str,
    name: &str,
    radius: f32,
    zoom: f32,
) {
    let label = if kind == "ladder_vcf" && name == "cutoff" {
        "POLE FREQUENCY"
    } else {
        parameter_label(name)
    };
    // The utility labels need two logical pixels above their value wells;
    // older envelope labels keep their clearance from the stage lenses.
    let label_gap = if matches!(kind, "lfo" | "lowpass" | "gain") {
        25.0
    } else {
        29.0
    };
    painter.text(
        center + vec2(0.0, radius + label_gap * PANEL_HEIGHT / 720.0) * zoom,
        Align2::CENTER_CENTER,
        label,
        crate::panel_skin::print_font(painter.ctx(), 9.0 * zoom),
        INK,
    );
    let range = match name {
        "attack" | "decay" | "release" => "1 ms – 10 s",
        "sustain" => "0 – 100%",
        "resonance" => "0 – 110%",
        "drive" => "0 – 4×",
        "gain" if kind == "gain" => "−16 – +16×",
        "gain" => "0 – 2×",
        "amplitude" => "0 – 1 PEAK",
        "offset" => "−1 – +1",
        "pulse_width" => "5 – 95%",
        _ => "",
    };
    painter.text(
        center + vec2(0.0, radius + 75.0 * PANEL_HEIGHT / 720.0) * zoom,
        Align2::CENTER_CENTER,
        range,
        crate::panel_skin::print_font(painter.ctx(), 6.0 * zoom),
        INK,
    );
}

#[cfg(test)]
mod atlas_voice_response_tests {
    use super::*;

    #[test]
    fn configured_ladder_pole_is_minus_twelve_db_and_zero_drive_is_silent() {
        let mut node = Node {
            id: "filter".into(),
            kind: "ladder_vcf".into(),
            params: [
                ("cutoff".into(), 600.0),
                ("resonance".into(), 0.0),
                ("drive".into(), 1.0),
            ]
            .into(),
            bypassed: false,
            content: None,
            score: None,
        };
        let points = atlas_ladder_points(&node, 48000);
        // Geometric midpoint of the 20 Hz–18 kHz display is 600 Hz. Four
        // uncoupled one-pole stages attenuate by 12.0412 dB at this frequency.
        let db = (237.0 - points[65].1) / 91.0 * 72.0 - 60.0;
        assert!((db + 12.0412).abs() < 0.001);
        node.params.insert("drive".into(), 0.0);
        assert!(atlas_ladder_points(&node, 48000)
            .iter()
            .all(|p| p.1 == 237.0));
        node.params.insert("drive".into(), 1.0);
        node.params.insert("cutoff".into(), 18000.0);
        let lower_rate = atlas_ladder_points(&node, 8000);
        let db = (237.0 - lower_rate.last().unwrap().1) / 91.0 * 72.0 - 60.0;
        assert!(
            (db + 12.0412).abs() < 0.001,
            "display uses the effective 3.2 kHz cap"
        );
    }

    #[test]
    fn configured_envelope_retains_ten_volt_attack_and_one_percent_release() {
        let node = Node {
            id: "envelope".into(),
            kind: "adsr".into(),
            params: [
                ("attack".into(), 0.001),
                ("decay".into(), 0.2),
                ("sustain".into(), 0.5),
                ("release".into(), 10.0),
            ]
            .into(),
            bypassed: false,
            content: None,
            score: None,
        };
        let points = atlas_envelope_points(&node);
        let voltage = |p: (f32, f32)| (303.0 - p.1) / 8.3;
        assert!(voltage(points[0]).abs() < 0.0001);
        assert!(
            (voltage(points[40]) - 10.0).abs() < 0.0001,
            "even a short attack reaches its physical 10 V endpoint"
        );
        let release_start = voltage(points[123]);
        assert!(
            (voltage(points[155]) - release_start * 0.01).abs() < 0.0001,
            "selected release time is 99% settling"
        );
        assert!(points.iter().all(|(x, y)| x.is_finite() && y.is_finite()));
    }
}
