use super::*;

fn noise_patch() -> Patch {
    serde_json::from_value(json!({"version":2,"nodes":[{"id":"noise","kind":"colored_noise"}]}))
        .unwrap()
}

fn key(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

#[test]
fn noise_level_and_full_24_bit_seed_have_guarded_exact_entry() {
    for (index, name, target) in [(0, "level", 1.25), (1, "seed", 16_777_215.0)] {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let patch = noise_patch();
        frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 7);
        let readout = pos2(24.0, 12.0) + noise_control_rects(index).1.center().to_vec2();
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.0,
            vec![egui::Event::PointerMoved(readout), button(readout, true)],
            true,
            true,
            7,
        );
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.1,
            vec![button(readout, false)],
            true,
            true,
            7,
        );
        let edit = rack
            .editing
            .as_mut()
            .expect("one click opens exact value entry");
        assert_eq!(edit.parameter, name);
        assert_eq!(edit.surface, ParameterEditSurface::Window);
        assert_eq!(edit.revision, 7);
        assert_eq!(edit.display_scale, 1.0);
        assert_eq!(edit.min, 0.0);
        assert_eq!(edit.max, if name == "seed" { 16_777_215.0 } else { 2.0 });
        assert_eq!(edit.step, if name == "seed" { Some(1.0) } else { None });
        edit.text = target.to_string();
        let action = frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.2,
            vec![key(egui::Key::Enter)],
            true,
            true,
            7,
        )
        .0;
        assert!(
            matches!(action, Some(RackAction::Command("set_parameter", args))
            if args["id"] == "noise" && args["parameter"] == name && args["value"] == json!(target)
            && args["expected_revision"] == 7),
            "{name} publishes a guarded raw value"
        );
    }
}

#[test]
fn noise_level_drag_and_keyboard_use_normal_history_commands() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let mut patch = noise_patch();
    frame(&ctx, &mut rack, &patch, 0.0, vec![]);
    let rail = noise_control_rects(0).0.translate(vec2(24.0, 12.0));
    let start = pos2(
        rail.left() + 5.0 + (rail.width() - 10.0) * 0.65,
        rail.center().y,
    );
    let grouped = |action| match action {
        Some(RackAction::Command("set_parameter", args)) => {
            assert_eq!(args["id"], "noise");
            assert_eq!(args["parameter"], "level");
            (
                args["history_group"]
                    .as_str()
                    .expect("grouped gesture")
                    .to_owned(),
                args["value"].as_f64().unwrap() as f32,
            )
        }
        _ => panic!("expected level parameter command"),
    };
    let (first_group, pressed) = grouped(frame(
        &ctx,
        &mut rack,
        &patch,
        1.0,
        vec![egui::Event::PointerMoved(start), button(start, true)],
    ));
    patch.nodes[0].params.insert("level".into(), pressed);
    let (group, first) = grouped(frame(
        &ctx,
        &mut rack,
        &patch,
        1.1,
        vec![egui::Event::PointerMoved(start + vec2(5.0, 0.0))],
    ));
    patch.nodes[0].params.insert("level".into(), first);
    let (same, second) = grouped(frame(
        &ctx,
        &mut rack,
        &patch,
        1.2,
        vec![egui::Event::PointerMoved(start + vec2(10.0, 0.0))],
    ));
    assert_eq!(first_group, group);
    assert_eq!(group, same);
    assert!(second > first && first > pressed);
    patch.nodes[0].params.insert("level".into(), second);
    frame(
        &ctx,
        &mut rack,
        &patch,
        1.3,
        vec![button(start + vec2(10.0, 0.0), false)],
    );
    let action = frame(
        &ctx,
        &mut rack,
        &patch,
        1.4,
        vec![key(egui::Key::ArrowLeft)],
    );
    assert!(
        matches!(action, Some(RackAction::Command("set_parameter", args))
        if args["parameter"] == "level" && args["value"].as_f64().unwrap() < f64::from(second)
        && args.get("history_group").is_none())
    );
}

#[test]
fn noise_seven_named_mono_outputs_are_separate_and_patchable_at_all_scales() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|module| module["kind"] == "colored_noise")
        .unwrap();
    assert_eq!(module["outputs"], json!(NOISE_OUTPUTS));
    assert_eq!(module["inputs"], json!([]));
    assert_eq!(panel_width("colored_noise"), 240.0);
    for scale in [0.42, 0.85, 1.0] {
        let panel = Rect::from_min_size(Pos2::ZERO, vec2(240.0, PANEL_HEIGHT) * scale);
        let sockets = (0..7)
            .map(|index| {
                Rect::from_center_size(
                    port_position("colored_noise", false, index, 7, 240.0) * scale,
                    Vec2::splat((29.0 * scale).max(12.0)),
                )
            })
            .collect::<Vec<_>>();
        for (index, socket) in sockets.iter().enumerate() {
            assert!(panel.contains_rect(*socket));
            for other in &sockets[index + 1..] {
                assert!(!socket.intersects(*other), "{scale}: jack targets overlap");
            }
            for control in 0..2 {
                let (slider, value) = noise_control_rects(control);
                assert!(!socket.intersects(slider * scale));
                assert!(!socket.intersects(value * scale));
            }
        }
        for port in NOISE_OUTPUTS {
            let ctx = egui::Context::default();
            let mut rack = Rack { zoom: scale, ..Rack::default() };
            let patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[
                {"id":"noise","kind":"colored_noise"},{"id":"sink","kind":"audio_out"}
            ]}))
            .unwrap();
            frame(&ctx, &mut rack, &patch, 0.0, vec![]);
            let output = fixture_jack(&rack, &patch, "noise", port, false);
            let sink = fixture_jack(&rack, &patch, "sink", "signal", true);
            let canvas = Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 700.0));
            assert_eq!(
                rack.cable_press_target(&patch, &catalog, canvas, canvas, output),
                Some(CableEnd::Output {
                    node: "noise".into(),
                    port: port.into()
                })
            );
            assert!(
                matches!(drag(&ctx, &mut rack, &patch, 1.0, output, sink),
                Some(RackAction::Command("connect", args))
                if args["from"] == "noise" && args["output"] == port && args["input"] == "signal"),
                "{port} at {scale}: named jack must route the normal connect action"
            );
            assert!(rack.editing.is_none());
        }
    }
}

#[test]
fn noise_seed_context_menu_uses_integer_entry_and_revision_guard() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch = noise_patch();
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 5);
    rack.requested_parameter_menu = Some(("noise".into(), "seed".into()));
    frame_revision_state(&ctx, &mut rack, &patch, 0.1, vec![], true, true, 5);
    let edit = rack
        .editing
        .as_ref()
        .expect("programmatic seed menu opens native editor");
    assert_eq!(edit.parameter, "seed");
    assert_eq!(edit.surface, ParameterEditSurface::KnobMenu);
    assert_eq!(edit.step, Some(1.0));
    assert_eq!(edit.revision, 5);
    assert_eq!(edit.max, 16_777_215.0);
}

#[test]
fn noise_static_spectra_have_no_runtime_seed_or_level_animation() {
    fn curves(output: &egui::FullOutput) -> Vec<String> {
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Path(path) if path.stroke.width == 1.5 && path.points.len() == 33 => {
                    Some(format!("{:?}", path.points))
                }
                _ => None,
            })
            .collect()
    }
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let mut patch = noise_patch();
    let first = curves(&frame_output(&ctx, &mut rack, &patch, 0.0, vec![]).1);
    assert_eq!(first.len(), 7);
    patch.nodes[0].params.insert("seed".into(), 8_392_421.0);
    patch.nodes[0].params.insert("level".into(), 0.25);
    let later = curves(&frame_output(&ctx, &mut rack, &patch, 30.0, vec![]).1);
    assert_eq!(
        first, later,
        "fixed response plots must not masquerade as live audio"
    );
    assert!(noise_parameter_help("seed").contains("sample rate"));
    assert!(noise_parameter_help("seed").contains("Changing the seed"));
}
