// Configured guides for the existing capacitor-ramp VCO and matched-pair OTA.
// They explain the model's controls; no running phase or fake meter is drawn.
fn atlas_vco_shape(node: &Node) -> Vec<(f32, f32)> {
    if node.params.get("waveform").copied().unwrap_or(0.0) >= 0.5 {
        let width = atlas_voice_parameter(node, "pulse_width", 0.5, 0.05, 0.95) as f32;
        let edge = 256.0 + 112.0 * width;
        vec![(256.0, 175.0), (edge, 175.0), (edge, 227.0), (368.0, 227.0)]
    } else {
        vec![(256.0, 227.0), (368.0, 175.0), (368.0, 227.0)]
    }
}
fn atlas_vca_transfer(node: &Node, input: f64) -> f64 {
    // Same documented constants as analog::Vca: 25°C, input divider 1:100,
    // transimpedance normalized to unity small-signal gain at CV 10 V.
    let scale = 2.0 * 1.380649e-23 * 298.15 / 1.602176634e-19 / 0.01;
    scale * (input / scale).tanh() * atlas_voice_parameter(node, "gain", 1.0, 0.0, 2.0)
}
fn paint_atlas_analog_configuration(
    draw: &AtlasVoicePaint<'_>,
    node: &Node,
    rate: u32,
    lighting: &crate::lighting::RoomLighting,
) {
    let painter = draw.painter;
    let quiet = Color32::from_rgb(133, 143, 123);
    let ink = Color32::from_rgb(195, 200, 180);
    let grid = Color32::from_rgb(45, 59, 44);
    let amber = Color32::from_rgb(229, 183, 103);
    let start = crate::lighting::next(painter);
    if node.kind == "analog_vco" {
        draw.screen(draw.rect(245.0, 109.0, 136.0, 164.0));
        draw.text(313.0, 127.0, "NOMINAL SHAPE", 9.0, ink);
        draw.text(313.0, 142.0, "±5 V · ONE CYCLE", 8.0, quiet);
        for y in [175.0, 201.0, 227.0] {
            painter.line_segment(
                [draw.p(256.0, y), draw.p(368.0, y)],
                Stroke::new(0.5 * draw.unit(), grid),
            );
        }
        for x in [284.0, 312.0, 340.0] {
            painter.line_segment(
                [draw.p(x, 162.0), draw.p(x, 233.0)],
                Stroke::new(0.5 * draw.unit(), grid),
            );
        }
        painter.add(egui::Shape::line(
            atlas_vco_shape(node)
                .into_iter()
                .map(|(x, y)| draw.p(x, y))
                .collect(),
            Stroke::new(2.0 * draw.unit(), amber),
        ));
        let max = 18000.0_f64.min(f64::from(rate) * 0.4);
        let hz = atlas_voice_parameter(node, "frequency", 261.6256, 0.1, max);
        let period = if hz >= 1.0 {
            format!("{:.2} ms / CYCLE", 1000.0 / hz)
        } else {
            format!("{:.2} s / CYCLE", 1.0 / hz)
        };
        draw.text(313.0, 252.0, &period, 8.0, quiet);
    } else {
        draw.screen(draw.rect(38.0, 345.0, 344.0, 112.0));
        draw.text(210.0, 358.0, "DC TRANSFER · CV 10 V · 25°C", 12.0, ink);
        for y in [380.0, 408.0, 436.0] {
            painter.line_segment(
                [draw.p(80.0, y), draw.p(340.0, y)],
                Stroke::new(0.6 * draw.unit(), grid),
            );
        }
        for x in [80.0, 210.0, 340.0] {
            painter.line_segment(
                [draw.p(x, 380.0), draw.p(x, 436.0)],
                Stroke::new(0.6 * draw.unit(), grid),
            );
        }
        let points = (0..=100)
            .map(|i| {
                let input = -10.0 + i as f64 * 0.2;
                draw.p(
                    80.0 + 260.0 * i as f32 / 100.0,
                    408.0 - atlas_vca_transfer(node, input) as f32 / 12.0 * 28.0,
                )
            })
            .collect();
        painter.add(egui::Shape::line(
            points,
            Stroke::new(2.0 * draw.unit(), amber),
        ));
        draw.text(58.0, 380.0, "+12", 9.0, quiet);
        draw.text(58.0, 436.0, "−12", 9.0, quiet);
        draw.text(102.0, 447.0, "−10 V IN", 10.0, quiet);
        draw.text(318.0, 447.0, "+10 V IN", 10.0, quiet);
    }
    lighting.preserve(painter, start);
    if node.kind == "analog_vco" {
        let max = 18000.0_f64.min(f64::from(rate) * 0.4);
        draw.text(
            131.0,
            93.0,
            &format!("{:.1} Hz", (0.1 * max).sqrt()),
            8.0,
            INK,
        );
        draw.text(58.0, 241.0, "0.1", 10.0, INK);
        draw.text(
            207.0,
            241.0,
            &if max >= 10000.0 {
                format!("{:.0}k", max / 1000.0)
            } else {
                format!("{:.1}k", max / 1000.0)
            },
            10.0,
            INK,
        );
        draw.text(313.0, 292.0, "WIDTH APPLIES TO PULSE", 10.0, INK);
        // Pitch is exponential, linear FM adds Hz before the phase integrator.
        draw.line(&[(87.0, 566.0), (87.0, 532.0)], 1.8, true);
        painter.rect_stroke(
            draw.rect(58.0, 506.0, 92.0, 26.0),
            draw.unit(),
            Stroke::new(1.5 * draw.unit(), INK),
            egui::StrokeKind::Inside,
        );
        draw.text(104.0, 519.0, "EXP", 14.0, INK);
        draw.line(&[(150.0, 519.0), (180.0, 519.0)], 1.8, true);
        painter.circle_stroke(
            draw.p(195.0, 519.0),
            12.0 * draw.unit(),
            Stroke::new(1.5 * draw.unit(), INK),
        );
        draw.text(195.0, 519.0, "+", 16.0, INK);
        draw.line(
            &[
                (210.0, 566.0),
                (210.0, 549.0),
                (195.0, 549.0),
                (195.0, 535.0),
            ],
            1.8,
            true,
        );
        draw.line(&[(210.0, 519.0), (240.0, 519.0)], 1.8, true);
        painter.rect_stroke(
            draw.rect(244.0, 506.0, 100.0, 26.0),
            draw.unit(),
            Stroke::new(1.5 * draw.unit(), INK),
            egui::StrokeKind::Inside,
        );
        draw.text(294.0, 519.0, "RAMP / SHAPE", 12.0, INK);
        draw.line(
            &[
                (344.0, 519.0),
                (391.0, 519.0),
                (391.0, 610.0),
                (370.0, 610.0),
            ],
            1.8,
            true,
        );
        draw.text(210.0, 482.0, "PITCH → CURRENT → CAPACITOR RAMP", 9.0, INK);
    } else {
        draw.text(210.0, 476.0, "CV 0–10 V · UNPATCHED 10 V", 10.0, INK);
        draw.line(
            &[(87.0, 610.0), (28.0, 610.0), (28.0, 511.0), (176.0, 511.0)],
            1.8,
            true,
        );
        painter.circle_stroke(
            draw.p(210.0, 511.0),
            27.0 * draw.unit(),
            Stroke::new(1.8 * draw.unit(), INK),
        );
        draw.text(210.0, 511.0, "OTA", 14.0, INK);
        draw.line(
            &[
                (248.0, 511.0),
                (391.0, 511.0),
                (391.0, 610.0),
                (370.0, 610.0),
            ],
            1.8,
            true,
        );
        draw.dashed(&[(210.0, 566.0), (210.0, 541.0)]);
    }
    draw.line(&[(22.0, 655.0), (398.0, 655.0)], 1.9, false);
    for (x, text) in if node.kind == "analog_vco" {
        [
            (87.0, "1 V / OCT"),
            (210.0, "1 Hz / V"),
            (333.0, "±5 V OUT"),
        ]
    } else {
        [(87.0, "AUDIO IN"), (210.0, "CONTROL"), (333.0, "OTA OUT")]
    } {
        draw.text(x, 671.0, text, 10.0, INK);
    }
}

#[cfg(test)]
mod atlas_analog_guide_tests {
    use super::*;
    #[test]
    fn ota_dc_guide_agrees_with_settled_real_engine_at_nominal_cv() {
        for gain in [0.0, 1.0, 2.0] {
            for input in [-10.0, -1.0, 0.0, 1.0, 10.0] {
                let patch: Patch = serde_json::from_value(json!({"version":2,"sample_rate":48000,
                    "nodes":[{"id":"source","kind":"constant","params":{"value":input}},
                        {"id":"vca","kind":"analog_vca","params":{"gain":gain}}],
                    "cables":[{"from":"source","output":"out","to":"vca","input":"signal"}],
                    "output":"vca"}))
                .unwrap();
                let mut engine = patch.compile().unwrap();
                let mut samples = [0.0; 512];
                engine.process_block(&mut samples);
                let actual = samples[448..].iter().copied().sum::<f32>() / 64.0;
                let expected = atlas_vca_transfer(&patch.nodes[1], input);
                assert!(
                    (f64::from(actual) - expected).abs() < 0.0002,
                    "gain={gain} input={input}: {actual} vs {expected}"
                );
            }
        }
    }
}
