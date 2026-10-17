// Static guides for the existing normalized LFO, discrete low-pass and signed
// gain utility. These are configured transfer/shape views, never telemetry.
fn atlas_lfo_sample(node: &Node, phase: f64) -> f64 {
    let amplitude = atlas_voice_parameter(node, "amplitude", 0.5, 0.0, 1.0);
    let offset = atlas_voice_parameter(node, "offset", 0.5, -1.0, 1.0);
    (phase * std::f64::consts::TAU).sin() * amplitude + offset
}
fn atlas_lowpass_alpha(node: &Node, rate: u32) -> f64 {
    let cutoff = atlas_voice_parameter(
        node,
        "cutoff",
        1000.0,
        1.0,
        20000.0_f64.min(f64::from(rate) * 0.45),
    );
    f64::from((-(-std::f64::consts::TAU * cutoff / f64::from(rate)).exp_m1()) as f32)
}
fn atlas_lowpass_db(a: f64, rate: u32, frequency: f64) -> f64 {
    let p = 1.0 - a;
    let w = std::f64::consts::TAU * frequency / f64::from(rate);
    20.0 * (a / (1.0 - p * w.cos()).hypot(p * w.sin())).log10()
}
fn atlas_gain_sample(node: &Node, input: f64, cv: f64) -> f64 {
    (input * atlas_voice_parameter(node, "gain", 1.0, -16.0, 16.0) * cv)
        .clamp(-1_000_000.0, 1_000_000.0)
}
fn atlas_signal_number(value: f64) -> String {
    if value == 0.0 || value.abs() >= 0.01 {
        format!("{value:.2}")
    } else if value.abs() >= 0.0001 {
        format!("{value:.4}")
    } else {
        format!("{value:.1e}")
    }
}
fn atlas_lfo_range(node: &Node) -> String {
    let a = atlas_voice_parameter(node, "amplitude", 0.5, 0.0, 1.0);
    let o = atlas_voice_parameter(node, "offset", 0.5, -1.0, 1.0);
    if a == 0.0 {
        format!("{} DC", atlas_signal_number(o))
    } else if a >= 0.01 {
        format!("{:.2} … {:.2}", o - a, o + a)
    } else if a >= 0.0001 {
        format!("{:.4} … {:.4}", o - a, o + a)
    } else {
        format!("{} ± {a:.1e}", atlas_signal_number(o))
    }
}
fn atlas_short_hz(hz: f64) -> String {
    if hz >= 10000.0 {
        format!("{:.0}k", hz / 1000.0)
    } else if hz >= 1000.0 {
        format!("{:.1}k", hz / 1000.0)
    } else {
        format!("{hz:.0}")
    }
}
fn paint_atlas_utility_configuration(
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
    if node.kind == "lfo" {
        draw.screen(draw.rect(245.0, 109.0, 136.0, 164.0));
        draw.text(313.0, 127.0, "CONFIGURED SINE", 9.0, ink);
        draw.text(313.0, 142.0, "AUTO FIT · ZERO FM", 8.0, quiet);
        let amplitude = atlas_voice_parameter(node, "amplitude", 0.5, 0.0, 1.0);
        let offset = atlas_voice_parameter(node, "offset", 0.5, -1.0, 1.0);
        let span = if amplitude == 0.0 { 0.01 } else { amplitude };
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
        // The fitted zero reference makes DC bias readable without making a
        // small amplitude disappear. A zero amplitude remains a flat line.
        if offset.abs() <= span {
            let zero = 201.0 + (offset / span * 26.0) as f32;
            painter.line_segment(
                [draw.p(256.0, zero), draw.p(368.0, zero)],
                Stroke::new(draw.unit(), quiet),
            );
        }
        let points = (0..=96)
            .map(|i| {
                let phase = i as f64 / 96.0;
                draw.p(
                    256.0 + 112.0 * phase as f32,
                    201.0 - ((atlas_lfo_sample(node, phase) - offset) / span * 26.0) as f32,
                )
            })
            .collect();
        painter.add(egui::Shape::line(
            points,
            Stroke::new(2.0 * draw.unit(), amber),
        ));
        draw.text(313.0, 248.0, &atlas_lfo_range(node), 8.0, quiet);
        let hz = atlas_voice_parameter(node, "frequency", 2.0, 0.0, 100.0);
        let period = if hz == 0.0 {
            "HOLD · 0 Hz".into()
        } else if hz >= 1.0 {
            format!("{:.2} s / CYCLE", 1.0 / hz)
        } else {
            format!("{:.1} s / CYCLE", 1.0 / hz)
        };
        draw.text(313.0, 263.0, &period, 8.0, quiet);
    } else {
        draw.screen(draw.rect(38.0, 345.0, 344.0, 112.0));
        let lowpass = node.kind == "lowpass";
        draw.text(
            210.0,
            358.0,
            if lowpass {
                "DISCRETE RESPONSE · dB"
            } else {
                "TRANSFER · CV 1 · AUTO FIT"
            },
            12.0,
            ink,
        );
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
        if lowpass {
            let ceiling = f64::from(rate) * 0.5;
            let alpha = atlas_lowpass_alpha(node, rate);
            let points = (0..=100)
                .map(|i| {
                    let t = i as f64 / 100.0;
                    let hz = ceiling.powf(t);
                    draw.p(
                        80.0 + 260.0 * t as f32,
                        380.0
                            + (-atlas_lowpass_db(alpha, rate, hz).clamp(-60.0, 0.0) / 60.0 * 56.0)
                                as f32,
                    )
                })
                .collect();
            painter.add(egui::Shape::line(
                points,
                Stroke::new(2.0 * draw.unit(), amber),
            ));
            draw.text(58.0, 380.0, "0", 9.0, quiet);
            draw.text(58.0, 436.0, "−60", 9.0, quiet);
            draw.text(102.0, 447.0, "1 Hz", 10.0, quiet);
            draw.text(
                318.0,
                447.0,
                &format!("{} Hz", atlas_short_hz(ceiling)),
                10.0,
                quiet,
            );
        } else {
            let gain = atlas_voice_parameter(node, "gain", 1.0, -16.0, 16.0);
            let span = if gain == 0.0 { 1.0 } else { gain.abs() };
            let points = (0..=32)
                .map(|i| {
                    let input = -1.0 + i as f64 / 16.0;
                    draw.p(
                        80.0 + 260.0 * i as f32 / 32.0,
                        408.0 - (atlas_gain_sample(node, input, 1.0) / span * 28.0) as f32,
                    )
                })
                .collect();
            painter.add(egui::Shape::line(
                points,
                Stroke::new(2.0 * draw.unit(), amber),
            ));
            draw.text(
                58.0,
                380.0,
                &format!("+{}", atlas_signal_number(span)),
                9.0,
                quiet,
            );
            draw.text(
                58.0,
                436.0,
                &format!("−{}", atlas_signal_number(span)),
                9.0,
                quiet,
            );
            draw.text(102.0, 447.0, "−1 IN", 10.0, quiet);
            draw.text(318.0, 447.0, "+1 IN", 10.0, quiet);
        }
    }
    lighting.preserve(painter, start);
    if node.kind == "lfo" {
        draw.text(131.0, 93.0, "9.05 Hz", 8.0, INK); // midpoint of log1p(0..100)
        draw.text(58.0, 241.0, "0", 10.0, INK);
        draw.text(207.0, 241.0, "100", 10.0, INK);
        draw.text(313.0, 292.0, "NORMALIZED SIGNAL LEVEL", 10.0, INK);
        // Actual processing order: sin(phase) × amplitude + offset.
        draw.line(&[(105.0, 566.0), (105.0, 530.0)], 1.8, true);
        painter.rect_stroke(
            draw.rect(63.0, 502.0, 84.0, 28.0),
            draw.unit(),
            Stroke::new(1.5 * draw.unit(), INK),
            egui::StrokeKind::Inside,
        );
        draw.text(105.0, 516.0, "SINE OSC", 14.0, INK);
        draw.line(&[(147.0, 516.0), (177.0, 516.0)], 1.8, true);
        painter.circle_stroke(
            draw.p(194.0, 516.0),
            13.0 * draw.unit(),
            Stroke::new(1.5 * draw.unit(), INK),
        );
        draw.text(194.0, 516.0, "× A", 14.0, INK);
        draw.line(&[(212.0, 516.0), (241.0, 516.0)], 1.8, true);
        painter.circle_stroke(
            draw.p(260.0, 516.0),
            13.0 * draw.unit(),
            Stroke::new(1.5 * draw.unit(), INK),
        );
        draw.text(260.0, 516.0, "+ O", 14.0, INK);
        draw.line(
            &[
                (278.0, 516.0),
                (391.0, 516.0),
                (391.0, 610.0),
                (354.0, 610.0),
            ],
            1.8,
            true,
        );
        draw.text(210.0, 482.0, "FM ADDS Hz · 0 Hz HOLDS PHASE", 9.0, INK);
        draw.text(210.0, 552.0, "OUT = A · sin(φ) + O", 10.0, INK);
    } else {
        if node.kind == "lowpass" {
            let max = 20000.0_f64.min(f64::from(rate) * 0.45);
            draw.text(210.0, 90.0, &format!("{:.0} Hz", max.sqrt()), 8.0, INK);
            draw.text(105.0, 270.0, "1", 10.0, INK);
            draw.text(315.0, 270.0, &atlas_short_hz(max), 10.0, INK);
            draw.text(210.0, 476.0, "y += a · (IN − y)", 12.0, INK);
        } else {
            draw.text(210.0, 476.0, "CV MULTIPLIES DIRECTLY", 10.0, INK);
        }
        let two = node.kind == "lowpass";
        let in_x = if two { 105.0 } else { 87.0 };
        let out_x = if two { 315.0 } else { 333.0 };
        draw.line(
            &[
                (in_x - 35.0, 610.0),
                (28.0, 610.0),
                (28.0, 511.0),
                (176.0, 511.0),
            ],
            1.8,
            true,
        );
        painter.circle_stroke(
            draw.p(210.0, 511.0),
            27.0 * draw.unit(),
            Stroke::new(1.8 * draw.unit(), INK),
        );
        draw.text(
            210.0,
            511.0,
            if two { "1P" } else { "×" },
            if two { 14.0 } else { 24.0 },
            INK,
        );
        draw.line(
            &[
                (248.0, 511.0),
                (391.0, 511.0),
                (391.0, 610.0),
                (out_x + 35.0, 610.0),
            ],
            1.8,
            true,
        );
        if !two {
            draw.dashed(&[(210.0, 566.0), (210.0, 541.0)]);
        }
    }
    draw.line(&[(22.0, 655.0), (398.0, 655.0)], 1.9, false);
    let labels: &[(f32, &str)] = match node.kind.as_str() {
        "lfo" => &[(105.0, "ADDITIVE Hz"), (315.0, "SINE OUT")],
        "lowpass" => &[(105.0, "AUDIO / CV IN"), (315.0, "LOW-PASS OUT")],
        _ => &[
            (87.0, "AUDIO / CV IN"),
            (210.0, "UNITY AT 1"),
            (333.0, "MULTIPLIED OUT"),
        ],
    };
    for &(x, label) in labels {
        draw.text(x, 671.0, label, 10.0, INK);
    }
}

#[cfg(test)]
mod atlas_utility_guide_tests {
    use super::*;
    #[test]
    fn lfo_configured_levels_agree_with_real_engine_including_zero_hz() {
        for hz in [0.0, 2.0, 100.0] {
            for amplitude in [0.0, 0.001, 0.5] {
                let patch: Patch=serde_json::from_value(json!({"version":2,"sample_rate":48000,
                    "nodes":[{"id":"lfo","kind":"lfo","params":{"frequency":hz,"amplitude":amplitude,"offset":-0.5}}],"output":"lfo"})).unwrap();
                let mut engine = patch.compile().unwrap();
                let mut output = [0.0; 256];
                engine.process_block(&mut output);
                for i in [0, 127, 255] {
                    let expected = atlas_lfo_sample(&patch.nodes[0], i as f64 * hz / 48000.0);
                    assert!(
                        (f64::from(output[i]) - expected).abs() < 0.000001,
                        "hz={hz} a={amplitude} sample={i}"
                    );
                }
            }
        }
    }
    #[test]
    fn lowpass_configured_coefficient_matches_actual_engine_step() {
        for rate in [8000, 48000, 192000] {
            for requested in [1.0_f64, 1000.0, 20000.0] {
                let cutoff = requested.min((rate as f64 * 0.45).min(20000.0));
                let patch: Patch=serde_json::from_value(json!({"version":1,"sample_rate":rate,
                    "nodes":[{"id":"source","kind":"constant","params":{"value":1.0}},
                        {"id":"filter","kind":"lowpass","params":{"cutoff":cutoff}}],
                    "cables":[{"from":"source","output":"out","to":"filter","input":"signal"}],"output":"filter"})).unwrap();
                let mut engine = patch.compile().unwrap();
                let mut output = [0.0; 16];
                engine.process_block(&mut output);
                let a = atlas_lowpass_alpha(&patch.nodes[1], rate);
                assert_eq!(output[0], a as f32);
                let expected = 1.0 - (1.0 - a).powi(16);
                assert!((f64::from(output[15]) - expected).abs() < 0.000001);
                assert!(atlas_lowpass_db(a, rate, 0.0).abs() < 0.000001);
            }
        }
    }
    #[test]
    fn gain_guide_preserves_signed_cv_and_unpatched_unity_in_real_engine() {
        for gain in [-2.0, 0.0, 4.0] {
            for input in [-0.75, 0.5] {
                for cv in [None, Some(-1.5)] {
                    let mut patch: Patch=serde_json::from_value(json!({"version":2,
                        "nodes":[{"id":"source","kind":"constant","params":{"value":input}},
                            {"id":"gain","kind":"gain","params":{"gain":gain}}],
                        "cables":[{"from":"source","output":"out","to":"gain","input":"signal"}],"output":"gain"})).unwrap();
                    if let Some(cv) = cv {
                        patch.nodes.push(
                            serde_json::from_value(
                                json!({"id":"cv","kind":"constant","params":{"value":cv}}),
                            )
                            .unwrap(),
                        );
                        patch.cables.push(
                            serde_json::from_value(
                                json!({"from":"cv","output":"out","to":"gain","input":"cv"}),
                            )
                            .unwrap(),
                        );
                    }
                    let mut engine = patch.compile().unwrap();
                    let mut output = [0.0; 8];
                    engine.process_block(&mut output);
                    assert!(
                        (f64::from(output[7])
                            - atlas_gain_sample(&patch.nodes[1], input, cv.unwrap_or(1.0)))
                        .abs()
                            < 0.000001
                    );
                }
            }
        }
    }
    #[test]
    fn lfo_auto_fit_keeps_weak_shapes_visible_and_zero_flat() {
        let catalog = openpatch_core::catalog();
        let module = catalog["modules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["kind"] == "lfo")
            .unwrap();
        let ctx = egui::Context::default();
        crate::panel_skin::install_fonts(&ctx);
        for (amplitude, expected_height) in [(1.0, 28.6), (0.001, 28.6), (0.0, 0.0)] {
            let mut preview = ModulePreview::new(module);
            preview.patch.nodes[0]
                .params
                .insert("amplitude".into(), amplitude);
            preview.patch.nodes[0].params.insert("offset".into(), 0.0);
            let mut height = None;
            for _ in 0..2 {
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 500.0))),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            preview.show(
                                ui,
                                Rect::from_min_size(pos2(12.0, 12.0), vec2(180.0, PANEL_HEIGHT)),
                                &catalog,
                            );
                        });
                    },
                );
                for shape in output.shapes {
                    if let egui::Shape::Path(path) = shape.shape {
                        if path.stroke.color
                            == egui::epaint::ColorMode::Solid(Color32::from_rgb(229, 183, 103))
                        {
                            let top = path
                                .points
                                .iter()
                                .map(|p| p.y)
                                .fold(f32::INFINITY, f32::min);
                            let bottom = path
                                .points
                                .iter()
                                .map(|p| p.y)
                                .fold(f32::NEG_INFINITY, f32::max);
                            height = Some(bottom - top);
                        }
                    }
                }
            }
            assert!(
                (height.expect("actual native configured trace") - expected_height).abs() < 0.01,
                "a={amplitude}"
            );
            if amplitude == 0.001 {
                assert!(
                    atlas_lfo_range(&preview.patch.nodes[0]).contains("0.0010"),
                    "fitted range must retain the actual small level"
                );
            }
        }
    }
}
