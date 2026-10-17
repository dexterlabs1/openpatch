// Opt-in CPU rendering of the real shared panel renderer. No window, graphics
// context or audio device is opened. Used for focused design reviews on Mac.
use super::*;

#[test]
#[ignore = "writes a headless design review PNG to OPENPATCH_PANEL_PREVIEW"]
fn export_panel_design_preview() {
    let path = std::env::var("OPENPATCH_PANEL_PREVIEW").expect("set PNG output path");
    let catalog = openpatch_core::catalog();
    let defaults = "adsr_eg,adsr,lfo,analog_vca,lowpass,svf_filter";
    let selection = std::env::var("OPENPATCH_PANEL_KINDS").unwrap_or_else(|_| defaults.into());
    let kinds: Vec<_> = selection.split(',').collect();
    let mut previews: Vec<_> = kinds
        .iter()
        .map(|kind| {
            ModulePreview::new(
                catalog["modules"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|module| module["kind"] == *kind)
                    .unwrap(),
            )
        })
        .collect();
    let light = std::env::var("OPENPATCH_PANEL_LIGHT")
        .ok()
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(1.0);
    for preview in &mut previews {
        preview.renderer.lighting.set(light);
        if std::env::var_os("OPENPATCH_PANEL_ENGINE_STATE").is_some() {
            // Opt-in offline Core processing supplies real lights/held values;
            // this export never opens an audio device or invents a sample trace.
            let id = format!("preview_{}", preview.patch.nodes[0].kind);
            preview.patch.nodes[0].id = id.clone();
            preview.renderer.positions.clear();
            preview.renderer.positions.insert(id, Pos2::ZERO);
            let mut engine = preview.patch.compile().unwrap();
            engine.set_port_lights_enabled(true);
            engine.process_block(&mut vec![0.0; 60_000]);
            let node = &preview.patch.nodes[0];
            if let Some(lights) = engine
                .resolve_module_lights(&node.id)
                .ok()
                .and_then(|h| engine.module_lights(h))
            {
                preview
                    .renderer
                    .module_lights
                    .insert(node.id.clone(), lights);
            }
            if let Some(scope) = engine
                .resolve_module_scope(&node.id)
                .ok()
                .and_then(|h| engine.module_scope(h))
            {
                preview
                    .renderer
                    .scope_states
                    .insert(node.id.clone(), scope.clone().into());
            }
            if let Some(state) = engine
                .resolve_module_telemetry(&node.id)
                .ok()
                .and_then(|h| engine.module_telemetry(h))
            {
                preview
                    .renderer
                    .sequence_states
                    .insert(node.id.clone(), state);
            }
            match engine
                .resolve_module_state(&node.id)
                .ok()
                .and_then(|h| engine.module_saved_state(h))
            {
                Some(openpatch_core::SavedModuleState::RandomValues { values, rng }) => {
                    let values = values.map(|row| row.map(f32::from_bits));
                    let channels = preview
                        .renderer
                        .module_lights
                        .get(&node.id)
                        .and_then(|l| l.input_channels)
                        .unwrap_or(1);
                    preview.renderer.set_random_values_states(BTreeMap::from([(
                        node.id.clone(),
                        json!({"values":values,"rng_state":rng,"active_channels":channels}),
                    )]));
                }
                Some(openpatch_core::SavedModuleState::ShiftRegister { values, rng }) => {
                    preview.renderer.set_shift_register_states(BTreeMap::from([(
                        node.id.clone(),
                        json!({"values":values.map(f32::from_bits),"rng_state":rng}),
                    )]));
                }
                _ => {}
            }
            preview.renderer.port_lights = Some(
                crate::port_light_transport::offline_frame(&preview.patch, &engine, 0).unwrap(),
            );
        }
    }
    let ctx = egui::Context::default();
    crate::panel_skin::install_fonts(&ctx);
    let review_scale = std::env::var("OPENPATCH_PANEL_SCALE")
        .ok()
        .and_then(|s| s.parse::<f32>().ok())
        .filter(|s| s.is_finite())
        .unwrap_or(1.0)
        .clamp(0.5, 3.0);
    let size = vec2(
        previews.iter().map(|p| p.size().x + 12.0).sum::<f32>() + 28.0,
        650.0,
    ) * review_scale;
    let mut capture = crate::headless_review::CpuCapture::default();
    let mut output = None;
    for frame in 0..2 {
        let result = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                time: Some(frame as f64),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| {
                        ui.painter()
                            .rect_filled(ui.max_rect(), 0.0, Color32::from_rgb(28, 31, 31));
                        for (scale, y) in [
                            (review_scale, 26.0 * review_scale),
                            (0.42 * review_scale, 456.0 * review_scale),
                        ] {
                            let mut x = 20.0 * review_scale;
                            for preview in &mut previews {
                                let bounds =
                                    Rect::from_min_size(pos2(x, y), preview.size() * scale);
                                ui.push_id(
                                    (scale.to_bits(), preview.patch.nodes[0].kind.clone()),
                                    |ui| {
                                        preview.show(ui, bounds, &catalog);
                                    },
                                );
                                x = bounds.right() + 12.0 * review_scale;
                            }
                        }
                    });
            },
        );
        capture.observe(&result);
        output = Some(result);
    }
    let output = output.unwrap();
    for clipped in &output.shapes {
        if let egui::Shape::Text(text) = &clipped.shape {
            assert!(
                !text.galley.text().contains("use of widget ID"),
                "simultaneous panel previews reused a widget identity"
            );
        }
    }
    capture.save(&ctx, output, size, path);
}

#[test]
#[ignore = "writes actual signal-driven CPU lighting PNGs to OPENPATCH_UX_PREVIEW_DIR"]
fn export_dark_room_ux_review() {
    let destination = std::path::PathBuf::from(std::env::var("OPENPATCH_UX_PREVIEW_DIR").unwrap());
    let mut patch: Patch =
        serde_json::from_str(include_str!("../../../examples/stock-vca-mix-voice.json")).unwrap();
    patch
        .nodes
        .retain(|n| matches!(n.id.as_str(), "low" | "mix" | "motion" | "scope" | "out"));
    patch.cables.retain(|c| {
        patch.nodes.iter().any(|n| n.id == c.from) && patch.nodes.iter().any(|n| n.id == c.to)
    });
    let mut engine = patch.compile().unwrap();
    engine.set_port_lights_enabled(true);
    engine.process_block(&mut vec![0.0; 24_000]);
    let lights: BTreeMap<_, _> = patch
        .nodes
        .iter()
        .filter_map(|n| {
            engine
                .resolve_module_lights(&n.id)
                .ok()
                .and_then(|h| engine.module_lights(h))
                .map(|state| (n.id.clone(), state))
        })
        .collect();
    let ports = crate::port_light_transport::offline_frame(&patch, &engine, 0).unwrap();
    assert!(ports
        .ports
        .values()
        .any(|s| s.raw_rgb.iter().any(|v| *v > 0.0)));
    for amount in [1.0, 0.08, 0.0] {
        let ctx = egui::Context::default();
        crate::panel_skin::install_fonts(&ctx);
        let mut rack = Rack {
            zoom: 1.15,
            pan: vec2(24.0, 70.0),
            module_lights: lights.clone(),
            port_lights: Some(ports.clone()),
            ..Default::default()
        };
        rack.set_room_brightness(amount);
        let mut x = 0.0;
        for id in ["low", "motion", "mix", "scope", "out"] {
            let node = patch.nodes.iter().find(|n| n.id == id).unwrap();
            rack.positions.insert(id.into(), pos2(x, 0.0));
            x += module_width(node);
        }
        let size = vec2(x * 1.15 + 48.0, 620.0);
        let mut capture = crate::headless_review::CpuCapture::default();
        let mut last = None;
        for time in [0.0, 0.1] {
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                    time: Some(time),
                    focused: true,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE)
                        .show(ctx, |ui| {
                            rack.show(ui, &patch, &openpatch_core::catalog(), 48_000, 0);
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
            destination.join(format!("room-{amount:.2}.png")),
        );
    }
}

#[test]
#[ignore = "writes an actual knob drag CPU PNG to OPENPATCH_UX_PREVIEW_DIR"]
fn export_knob_bubble_ux_review() {
    let destination = std::path::PathBuf::from(std::env::var("OPENPATCH_UX_PREVIEW_DIR").unwrap());
    let dir = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(dir.path()).unwrap();
    let patch: Patch =
        serde_json::from_value(json!({"version":2,"nodes":[{"id":"delay","kind":"stock_delay"}]}))
            .unwrap();
    session.replace_patch(patch, Some(0)).unwrap();
    let ctx = egui::Context::default();
    crate::panel_skin::install_fonts(&ctx);
    let mut rack = Rack {
        zoom: 1.4,
        pan: vec2(65.0, 80.0),
        ..Default::default()
    };
    rack.positions.insert("delay".into(), Pos2::ZERO);
    let size = vec2(600.0, 720.0);
    let at = pos2(65.0 + 46.0 * 1.4, 80.0 + 120.0 * 1.4);
    let mut capture = crate::headless_review::CpuCapture::default();
    let mut last = None;
    for (time, events) in [
        (0.0, vec![]),
        (0.1, vec![]),
        (1.0, vec![egui::Event::PointerMoved(at), button(at, true)]),
        (1.1, vec![egui::Event::PointerMoved(at - vec2(0.0, 20.0))]),
        (1.25, vec![]),
    ] {
        let mut action = None;
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                time: Some(time),
                events,
                focused: true,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| {
                        action = rack.show(
                            ui,
                            session.patch(),
                            &openpatch_core::catalog(),
                            48_000,
                            session.revision(),
                        );
                    });
            },
        );
        if let Some(RackAction::Command(name, args)) = action {
            session.execute(name, args).unwrap();
        }
        capture.observe(&output);
        last = Some(output);
    }
    assert!(rack.knob_value_bubble.is_some());
    capture.save(
        &ctx,
        last.unwrap(),
        size,
        destination.join("knob-value-bubble.png"),
    );
}
