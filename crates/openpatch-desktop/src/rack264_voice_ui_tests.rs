use super::*;

fn patch(kind: &str) -> Patch {
    serde_json::from_value(json!({"version":2,"nodes":[{"id":"subject","kind":kind}]})).unwrap()
}
fn text_shapes(shape: &egui::Shape, out: &mut Vec<(String, Rect)>) {
    match shape {
        egui::Shape::Vec(children) => {
            for child in children {
                text_shapes(child, out);
            }
        }
        egui::Shape::Text(text) => out.push((
            text.galley.text().into(),
            Rect::from_min_size(text.pos, text.galley.size()),
        )),
        _ => {}
    }
}
fn labels(output: &egui::FullOutput) -> Vec<(String, Rect)> {
    let mut labels = Vec::new();
    for shape in &output.shapes {
        text_shapes(&shape.shape, &mut labels);
    }
    labels
}
fn control_center(kind: &str, name: &str) -> Pos2 {
    if kind == "rack264_vco" {
        stock_vco_control(name).0
    } else {
        stock_vcf_knob_rect(STOCK_VCF_PARAMS.iter().position(|p| *p == name).unwrap()).center()
    }
}

#[test]
fn rack264_voice_panels_keep_named_controls_and_port_geometry_in_rack_and_browser() {
    let catalog = openpatch_core::catalog();
    for (kind, heading, input_count, output_count) in [
        ("rack264_vco", "RACK 2.6.4 VCO", 4, 4),
        ("rack264_vcf", "RACK 2.6.4 VCF", 4, 2),
    ] {
        let module = catalog["modules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["kind"] == kind)
            .unwrap();
        for scale in [0.25, 0.42, 0.85, 1.0] {
            for browser in [false, true] {
                let ctx = egui::Context::default();
                crate::panel_skin::install_fonts(&ctx);
                let bounds = Rect::from_min_size(
                    pos2(24.0, 12.0),
                    vec2(panel_width(kind), PANEL_HEIGHT) * scale,
                );
                let mut rack = Rack {
                    zoom: scale,
                    pan: Vec2::ZERO,
                    ..Default::default()
                };
                rack.positions.insert("subject".into(), Pos2::ZERO);
                let patch = patch(kind);
                let mut preview = ModulePreview::new(module);
                let mut output = egui::FullOutput::default();
                for _ in 0..2 {
                    output = ctx.run(
                        egui::RawInput {
                            screen_rect: Some(Rect::from_min_size(
                                Pos2::ZERO,
                                vec2(1600.0, 1100.0),
                            )),
                            ..Default::default()
                        },
                        |ctx| {
                            egui::CentralPanel::default().frame(egui::Frame::NONE).show(
                                ctx,
                                |ui| {
                                    if browser {
                                        preview.show(ui, bounds, &catalog);
                                    } else {
                                        let mut child =
                                            ui.new_child(egui::UiBuilder::new().max_rect(bounds));
                                        child.set_clip_rect(ui.clip_rect().intersect(bounds));
                                        let rendered = rack.render_modules(
                                            &mut child,
                                            &patch,
                                            &catalog,
                                            bounds,
                                            48000,
                                            17,
                                            RenderMode::Interactive {
                                                drag_interactive: false,
                                                allow_manual_press: false,
                                            },
                                        );
                                        assert!(rendered.action.is_none());
                                        assert_eq!(rendered.inputs.len(), input_count);
                                        assert_eq!(rendered.outputs.len(), output_count);
                                    }
                                },
                            );
                        },
                    );
                }
                let labels = labels(&output);
                let required: &[&str] = if kind == "rack264_vco" {
                    &[
                        heading,
                        "FREQUENCY",
                        "WIDTH",
                        "FM DEPTH",
                        "PWM DEPTH",
                        "−54 st",
                        "+54 st",
                        "PHASE",
                        "TELEMETRY —",
                        "Linear FM",
                        "Soft sync",
                        "V/OCT",
                        "FM",
                        "SYNC",
                        "PWM",
                        "SIN",
                        "TRI",
                        "SAW",
                        "SQR",
                    ]
                } else {
                    &[
                        heading,
                        "CUTOFF",
                        "RESONANCE",
                        "DRIVE",
                        "FREQ CV DEPTH",
                        "RES CV DEPTH",
                        "DRIVE CV DEPTH",
                        "RK4 · NO OVERSAMPLING",
                        "FREQ CV",
                        "RES CV",
                        "DRIVE CV",
                        "IN",
                        "LP",
                        "HP",
                    ]
                };
                for required in required {
                    assert!(
                        labels.iter().any(|(text, _)| text == required),
                        "{kind} scale={scale} browser={browser}: missing {required}"
                    );
                }
                for (text, rect) in &labels {
                    assert!(
                        bounds.expand(0.5).contains_rect(*rect),
                        "{kind} scale={scale} {text} outside panel: {rect:?}"
                    );
                }
                let controls: &[&str] = if kind == "rack264_vco" {
                    &["frequency", "pulse_width", "fm_amount", "pwm_amount"]
                } else {
                    &STOCK_VCF_PARAMS
                };
                for name in controls {
                    let center = control_center(kind, name);
                    let radius = if kind == "rack264_vco" {
                        stock_vco_control(name).1
                    } else {
                        stock_vcf_control_radius(
                            STOCK_VCF_PARAMS.iter().position(|p| *p == *name).unwrap(),
                        )
                    };
                    let body = Rect::from_center_size(
                        bounds.min + center.to_vec2() * scale,
                        Vec2::splat(radius * 2.0 * scale),
                    );
                    for input in [true, false] {
                        let count = if input { input_count } else { output_count };
                        for index in 0..count {
                            let port = bounds.min
                                + port_position(kind, input, index, count, panel_width(kind))
                                    .to_vec2()
                                    * scale;
                            let socket = Rect::from_center_size(
                                port,
                                Vec2::splat(27.0 * port_material_scale(kind) * scale),
                            );
                            assert!(
                                !body.intersects(socket),
                                "{kind} {name} touches port {input}/{index}"
                            );
                        }
                    }
                }
                assert!(preview.renderer.editing.is_none());
            }
        }
    }
    assert_eq!(title("stock_vco"), "STOCK VCO");
    assert_eq!(title("stock_vcf"), "VCF");
    assert_eq!(panel_width("rack264_vco"), panel_width("stock_vco"));
    assert_eq!(panel_width("rack264_vcf"), panel_width("stock_vcf"));
    assert_eq!(
        crate::panel_skin::family_accent("rack264_vco"),
        crate::panel_skin::family_accent("stock_vco")
    );
    assert_eq!(
        crate::panel_skin::family_accent("rack264_vcf"),
        crate::panel_skin::family_accent("stock_vcf")
    );
}

#[test]
fn rack264_voice_rotaries_and_mode_buttons_issue_real_guarded_edits() {
    for kind in ["rack264_vco", "rack264_vcf"] {
        let names: &[&str] = if kind == "rack264_vco" {
            &["frequency", "pulse_width", "fm_amount", "pwm_amount"]
        } else {
            &STOCK_VCF_PARAMS
        };
        for name in names {
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            let patch = patch(kind);
            frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 17);
            let start = control_center(kind, name) + vec2(24.0, 12.0);
            frame_revision_state(
                &ctx,
                &mut rack,
                &patch,
                1.0,
                vec![egui::Event::PointerMoved(start), button(start, true)],
                true,
                true,
                17,
            );
            let end = start - vec2(0.0, 16.0);
            let action = frame_revision_state(
                &ctx,
                &mut rack,
                &patch,
                1.1,
                vec![egui::Event::PointerMoved(end)],
                true,
                true,
                17,
            )
            .0;
            assert!(
                matches!(action, Some(RackAction::Command("set_parameter", ref args)) if args["id"] == "subject" && args["parameter"] == *name && args["expected_revision"] == 17 && args["history_group"].is_string()),
                "{kind} {name}: expected a guarded parameter edit"
            );
        }
    }
    for name in ["linear_fm", "soft_sync"] {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let patch = patch("rack264_vco");
        frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 17);
        let pos = stock_vco_mode_rect(name).center() + vec2(24.0, 12.0);
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.0,
            vec![egui::Event::PointerMoved(pos), button(pos, true)],
            true,
            true,
            17,
        );
        let action = frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.1,
            vec![button(pos, false)],
            true,
            true,
            17,
        )
        .0;
        assert!(
            matches!(action, Some(RackAction::Command("set_parameter", ref args)) if args["parameter"] == name && args["value"] == 1.0 && args["expected_revision"] == 17),
            "{name}: expected a guarded parameter edit"
        );
    }
}

#[test]
fn rack264_voice_precision_editors_use_source_ranges_and_inclusive_hz_inverses() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch = patch("rack264_vco");
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 17);
    let value = stock_vco_value_center("frequency") + vec2(24.0, 12.0);
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.0,
        vec![egui::Event::PointerMoved(value), button(value, true)],
        true,
        true,
        17,
    );
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.1,
        vec![button(value, false)],
        true,
        true,
        17,
    );
    let mut edit = rack
        .editing
        .clone()
        .expect("frequency readout opens exact editor");
    assert_eq!(edit.kind, "rack264_vco");
    assert_eq!(
        (edit.min, edit.max),
        (stock_vco_frequency_hz(-54.0), stock_vco_frequency_hz(54.0))
    );
    for raw in [-54.0, 0.0, 54.0] {
        edit.text = format!("{} Hz", stock_vco_frequency_hz(raw));
        let parsed = parsed_parameter_result(&edit).unwrap();
        assert!((parameter_edit_raw_value(&edit, parsed) - raw).abs() < 1e-5);
    }
    edit.text = (edit.min * 0.99).to_string();
    assert!(parsed_parameter_result(&edit).is_err());
    let filter = self::patch("rack264_vcf");
    let (min, max) = stock_vcf_raw_bounds("rack264_vcf");
    let mut cutoff = stock_vcf_edit(&filter.nodes[0], 0, 17);
    assert_eq!(cutoff.kind, "rack264_vcf");
    for raw in [min, 0.5, max] {
        cutoff.text = format!("{} Hz", stock_vcf_frequency_hz(raw));
        let parsed = parsed_parameter_result(&cutoff).unwrap();
        assert!((parameter_edit_raw_value(&cutoff, parsed) - raw).abs() < 1e-6);
    }
    cutoff.text = "22000 Hz".into();
    assert!(parsed_parameter_result(&cutoff).is_err());
    cutoff.text = "8 Hz".into();
    assert!(parsed_parameter_result(&cutoff).is_err());
    let mut drive = stock_vcf_edit(&filter.nodes[0], 2, 17);
    for (text, expected) in [("0%", -1.0), ("100%", 0.0), ("200%", 1.0)] {
        drive.text = text.into();
        assert_eq!(
            parameter_edit_raw_value(&drive, parsed_parameter_result(&drive).unwrap()),
            expected
        );
    }
    assert_eq!(stock_vcf_hz_bounds("stock_vcf"), (8.0, 22000.0));
    assert!(!logarithmic_parameter("rack264_vco", "frequency"));
}

#[test]
fn rack264_vco_configuration_guide_retains_curves_and_width_without_live_phase() {
    assert_eq!(rack264_vco_nominal(0, 0.25, 0.5), 1.0);
    assert_eq!(rack264_vco_nominal(1, 0.0, 0.5), 9.0 / 16.0);
    assert_eq!(rack264_vco_nominal(2, 0.25, 0.5), 7.0 / 8.0);
    assert_ne!(
        rack264_vco_nominal(1, 0.0, 0.5),
        stock_vco_nominal(1, 0.0, 0.5)
    );
    assert_eq!(rack264_vco_nominal(3, 0.4, 0.5), 1.0);
    assert_eq!(rack264_vco_nominal(3, 0.4, 0.3), -1.0);
}

#[test]
#[ignore = "writes four CPU review PNGs to OPENPATCH_RACK264_REVIEW_DIR"]
fn rack264_voice_headless_review() {
    let destination = std::path::PathBuf::from(
        std::env::var("OPENPATCH_RACK264_REVIEW_DIR").expect("review output directory"),
    );
    std::fs::create_dir_all(&destination).unwrap();
    let catalog = openpatch_core::catalog();
    let (patch, engine) = crate::rack264_voice_fixture::observed_voice();
    for brightness in [1.0, 0.0] {
        let ctx = egui::Context::default();
        crate::panel_skin::install_fonts(&ctx);
        let mut capture = crate::headless_review::CpuCapture::default();
        let size = vec2(720.0, 440.0);
        let bounds = Rect::from_min_size(pos2(20.0, 18.0), size - vec2(40.0, 36.0));
        let mut rack = Rack {
            pan: Vec2::ZERO,
            ..Default::default()
        };
        rack.positions.insert("voice".into(), Pos2::ZERO);
        rack.positions.insert("filter".into(), pos2(378.0, 0.0));
        rack.set_room_brightness(brightness);
        rack.set_module_states(
            BTreeMap::from([(
                "voice".into(),
                engine
                    .module_lights(engine.resolve_module_lights("voice").unwrap())
                    .unwrap(),
            )]),
            BTreeMap::new(),
        );
        rack.set_port_lights(Some(
            crate::audio::offline_frame(&patch, &engine, 17).unwrap(),
        ));
        // render_modules is below the normal Rack::show revision handshake.
        // Match that real host revision so the capture shows actual plug light
        // observations rather than silently rejecting them as stale.
        rack.observed_revision = 17;
        let saw = rack
            .port_signal_state("voice", openpatch_core::PortDirection::Output, "saw")
            .unwrap();
        assert!(saw.connected && saw.raw_rgb.iter().any(|value| *value > 0.0));
        let mut last = None;
        for _ in 0..3 {
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE)
                        .show(ctx, |ui| {
                            let rendered = rack.render_modules(
                                ui,
                                &patch,
                                &catalog,
                                bounds,
                                48000,
                                17,
                                RenderMode::Interactive {
                                    drag_interactive: false,
                                    allow_manual_press: false,
                                },
                            );
                            assert!(rendered.action.is_none());
                        });
                },
            );
            capture.observe(&output);
            last = Some(output);
        }
        capture.save(
            &ctx,
            last.unwrap(),
            size,
            destination.join(if brightness == 0.0 {
                "rack264-voice-lights-out.png"
            } else {
                "rack264-voice.png"
            }),
        );
    }
    for kind in ["rack264_vco", "rack264_vcf"] {
        let ctx = egui::Context::default();
        crate::panel_skin::install_fonts(&ctx);
        let mut capture = crate::headless_review::CpuCapture::default();
        let size = vec2(panel_width(kind) + 40.0, PANEL_HEIGHT + 36.0);
        let bounds = Rect::from_min_size(pos2(20.0, 18.0), vec2(panel_width(kind), PANEL_HEIGHT));
        let module = catalog["modules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["kind"] == kind)
            .unwrap();
        let mut preview = ModulePreview::new(module);
        let mut last = None;
        for _ in 0..3 {
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE)
                        .show(ctx, |ui| preview.show(ui, bounds, &catalog));
                },
            );
            capture.observe(&output);
            last = Some(output);
        }
        assert!(
            preview.renderer.module_lights.is_empty(),
            "library preview must not invent runtime lamps"
        );
        capture.save(
            &ctx,
            last.unwrap(),
            size,
            destination.join(format!("{kind}-library.png")),
        );
    }
}
