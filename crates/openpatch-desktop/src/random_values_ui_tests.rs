use super::*;

fn random_patch() -> Patch {
    serde_json::from_value(json!({"version":2,"nodes":[{"id":"random","kind":"random_values"}]}))
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

fn text_bounds(shape: &egui::Shape, result: &mut Vec<(String, Rect)>) {
    match shape {
        egui::Shape::Vec(children) => {
            for child in children {
                text_bounds(child, result);
            }
        }
        egui::Shape::Text(text) if !text.galley.text().is_empty() => result.push((
            text.galley.text().into(),
            Rect::from_min_size(text.pos, text.galley.size()),
        )),
        _ => {}
    }
}

fn texts(output: &egui::FullOutput) -> Vec<(String, Rect)> {
    let mut result = Vec::new();
    for shape in &output.shapes {
        text_bounds(&shape.shape, &mut result);
    }
    result
}

#[test]
fn random_values_hidden_runtime_parameters_never_become_controls_or_preview_defaults() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "random_values")
        .unwrap();
    let patch = random_patch();
    assert_eq!(
        ordered_parameters(&patch.nodes[0], Some(module))
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>(),
        ["range", "seed"]
    );
    assert_eq!(
        module["params"]
            .as_object()
            .unwrap()
            .values()
            .filter(|spec| spec["hidden"] == true)
            .count(),
        114
    );
    let preview = ModulePreview::new(module);
    assert_eq!(
        preview.patch.nodes[0]
            .params
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        ["range", "seed"]
    );
}

#[test]
fn random_values_seed_supports_exact_24_bit_expressions_and_guarded_commands() {
    for (text, expected) in [
        ("0", 0.0),
        ("2^24 - 1", 16_777_215.0),
        ("1234567", 1_234_567.0),
    ] {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let patch = random_patch();
        frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 77);
        let target = random_values_seed_rect().center() + vec2(24.0, 12.0);
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.0,
            vec![egui::Event::PointerMoved(target), button(target, true)],
            true,
            true,
            77,
        );
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.1,
            vec![button(target, false)],
            true,
            true,
            77,
        );
        let edit = rack
            .editing
            .as_mut()
            .expect("single click opens exact seed entry");
        assert_eq!(edit.parameter, "seed");
        assert_eq!(edit.surface, ParameterEditSurface::Window);
        assert_eq!(
            (edit.min, edit.max, edit.step, edit.display_scale),
            (0.0, 16_777_215.0, Some(1.0), 1.0)
        );
        edit.text = text.into();
        let action = frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.2,
            vec![key(egui::Key::Enter)],
            true,
            true,
            77,
        )
        .0;
        assert!(
            matches!(action, Some(RackAction::Command("set_parameter", args)) if args["id"] == "random" && args["parameter"] == "seed" && args["value"] == expected && args["expected_revision"] == 77),
            "expression {text}"
        );
    }
}

#[test]
fn random_values_seed_menu_rejects_invalid_and_stale_edits() {
    for invalid in ["-1", "16777216", "1/0", "bad"] {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let patch = random_patch();
        frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 5);
        rack.requested_parameter_menu = Some(("random".into(), "seed".into()));
        frame_revision_state(&ctx, &mut rack, &patch, 0.1, vec![], true, true, 5);
        let edit = rack.editing.as_mut().expect("seed context editor");
        assert_eq!(edit.surface, ParameterEditSurface::KnobMenu);
        assert_eq!(edit.step, Some(1.0));
        assert_eq!(edit.revision, 5);
        edit.text = invalid.into();
        assert!(frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            0.2,
            vec![key(egui::Key::Enter)],
            true,
            true,
            5
        )
        .0
        .is_none());
        assert!(rack.editing.is_some(), "invalid input must remain editable");
        rack.editing.as_mut().unwrap().text = "42".into();
        assert!(frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            0.3,
            vec![key(egui::Key::Enter)],
            true,
            true,
            6
        )
        .0
        .is_none());
        assert!(rack.editing.as_ref().unwrap().stale);
        assert!(frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            0.4,
            vec![egui::Event::WindowFocused(false)],
            false,
            true,
            6
        )
        .0
        .is_none());
        assert!(rack.editing.is_none());
    }
}

#[test]
fn random_values_range_chooser_publishes_guarded_enum_without_runtime_values() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch = random_patch();
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 17);
    let target = random_values_range_rect().center() + vec2(24.0, 12.0);
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.0,
        vec![egui::Event::PointerMoved(target), button(target, true)],
        true,
        true,
        17,
    );
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.1,
        vec![button(target, false)],
        true,
        true,
        17,
    );
    let output = frame_revision_state(&ctx, &mut rack, &patch, 1.2, vec![], true, true, 17).1;
    let option = texts(&output)
        .into_iter()
        .find(|(label, _)| label == RANDOM_VALUES_RANGES[5])
        .expect("all six native range choices are visible")
        .1
        .center();
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.3,
        vec![egui::Event::PointerMoved(option), button(option, true)],
        true,
        true,
        17,
    );
    let action = frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.4,
        vec![button(option, false)],
        true,
        true,
        17,
    )
    .0;
    assert!(
        matches!(action, Some(RackAction::Command("set_parameter", args)) if args["id"] == "random" && args["parameter"] == "range" && args["value"] == 5 && args["expected_revision"] == 17 && args.get("history_group").is_none())
    );
    assert!(rack.editing.is_none());
    assert!(patch.nodes[0].params.is_empty());
}

#[test]
fn random_values_push_has_one_press_per_hold_and_paired_quick_click() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    rack.set_sequence_states(BTreeMap::new(), true);
    let patch = random_patch();
    frame(&ctx, &mut rack, &patch, 0.0, vec![]);
    let target = random_values_push_rect().center() + vec2(24.0, 12.0);
    let action = frame(
        &ctx,
        &mut rack,
        &patch,
        1.0,
        vec![egui::Event::PointerMoved(target), button(target, true)],
    );
    assert!(
        matches!(action, Some(RackAction::Command("trigger_module", args)) if args["id"] == "random" && args["action"] == "push_press")
    );
    assert!(
        frame(&ctx, &mut rack, &patch, 1.1, vec![]).is_none(),
        "held pointer never sends another draw"
    );
    let action = frame(&ctx, &mut rack, &patch, 1.2, vec![button(target, false)]);
    assert!(
        matches!(action, Some(RackAction::Command("trigger_module", args)) if args["action"] == "push_release")
    );
    assert!(rack.held_manual.is_none());
    let action = frame(
        &ctx,
        &mut rack,
        &patch,
        1.3,
        vec![button(target, true), button(target, false)],
    );
    assert!(
        matches!(action, Some(RackAction::ManualClick { id, press: "push_press", release: "push_release" }) if id == "random")
    );
    assert!(rack.held_manual.is_none());
    assert!(frame(&ctx, &mut rack, &patch, 1.4, vec![]).is_none());
}

#[test]
fn random_values_manual_press_releases_on_exit_focus_modal_bypass_removal_and_engine_loss() {
    for cause in [
        "release",
        "exit",
        "focus",
        "modal",
        "shortcuts",
        "bypass",
        "removal",
        "engine",
    ] {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        rack.set_sequence_states(BTreeMap::new(), true);
        let mut patch = random_patch();
        frame(&ctx, &mut rack, &patch, 0.0, vec![]);
        let target = random_values_push_rect().center() + vec2(24.0, 12.0);
        frame(
            &ctx,
            &mut rack,
            &patch,
            1.0,
            vec![egui::Event::PointerMoved(target), button(target, true)],
        );
        assert!(rack.held_manual.is_some());
        let (events, focused, enabled) = match cause {
            "release" => (vec![button(target, false)], true, true),
            "exit" => (
                vec![egui::Event::PointerMoved(target + vec2(0.0, 35.0))],
                true,
                true,
            ),
            "focus" => (vec![egui::Event::WindowFocused(false)], false, true),
            "modal" => (vec![], true, false),
            "shortcuts" => {
                rack.set_shortcuts_enabled(false);
                (vec![], true, true)
            }
            "bypass" => {
                patch.nodes[0].bypassed = true;
                (vec![], true, true)
            }
            "removal" => {
                patch.nodes.clear();
                (vec![], true, true)
            }
            _ => {
                rack.set_sequence_states(BTreeMap::new(), false);
                (vec![], true, true)
            }
        };
        let action = frame_state(&ctx, &mut rack, &patch, 1.1, events, focused, enabled).0;
        assert!(
            matches!(action, Some(RackAction::Command("trigger_module", args)) if args["id"] == "random" && args["action"] == "push_release"),
            "{cause}"
        );
        assert!(rack.held_manual.is_none());
    }
}

#[test]
fn random_values_seven_named_outputs_and_trigger_are_patchable_at_every_scale() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "random_values")
        .unwrap();
    assert_eq!(module["outputs"], json!(RANDOM_VALUES_OUTPUTS));
    assert_eq!(module["inputs"], json!(["trig"]));
    assert_eq!(panel_width("random_values"), 300.0);
    for scale in [0.42, 0.85, 1.0] {
        let panel = Rect::from_min_size(Pos2::ZERO, vec2(300.0, PANEL_HEIGHT) * scale);
        let sockets = (0..7)
            .map(|index| {
                Rect::from_center_size(
                    port_position("random_values", false, index, 7, 300.0) * scale,
                    Vec2::splat((29.0 * scale).max(12.0)),
                )
            })
            .collect::<Vec<_>>();
        let trigger = Rect::from_center_size(
            port_position("random_values", true, 0, 1, 300.0) * scale,
            Vec2::splat((29.0 * scale).max(18.0)),
        );
        for (index, socket) in sockets.iter().enumerate() {
            assert!(panel.contains_rect(*socket));
            assert!(!socket.intersects(trigger));
            for other in &sockets[index + 1..] {
                assert!(!socket.intersects(*other), "{scale}: outputs overlap");
            }
            for control in [
                random_values_range_rect(),
                random_values_seed_rect(),
                random_values_push_rect(),
                random_values_readout_rect(index),
            ] {
                assert!(!socket.intersects(control * scale));
            }
        }
        for port in RANDOM_VALUES_OUTPUTS {
            let ctx = egui::Context::default();
            let mut rack = Rack {
                zoom: scale,
                ..Rack::default()
            };
            let patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[{"id":"random","kind":"random_values"},{"id":"sink","kind":"audio_out"},{"id":"clock","kind":"gate"}]})).unwrap();
            frame(&ctx, &mut rack, &patch, 0.0, vec![]);
            let output = fixture_jack(&rack, &patch, "random", port, false);
            let sink = fixture_jack(&rack, &patch, "sink", "signal", true);
            let canvas = Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 700.0));
            assert_eq!(
                rack.cable_press_target(&patch, &catalog, canvas, canvas, output),
                Some(CableEnd::Output {
                    node: "random".into(),
                    port: port.into()
                })
            );
            assert!(
                matches!(drag(&ctx, &mut rack, &patch, 1.0, output, sink), Some(RackAction::Command("connect", args)) if args["from"] == "random" && args["output"] == port && args["input"] == "signal")
            );
            let clock = fixture_jack(&rack, &patch, "clock", "out", false);
            let trigger = fixture_jack(&rack, &patch, "random", "trig", true);
            assert!(
                matches!(drag(&ctx, &mut rack, &patch, 2.0, clock, trigger), Some(RackAction::Command("connect", args)) if args["to"] == "random" && args["input"] == "trig")
            );
            assert!(rack.editing.is_none());
        }
    }
}

#[test]
fn random_values_readouts_use_only_complete_actual_engine_snapshots() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let mut patch = random_patch();
    patch.nodes[0].params.insert("value1_1".into(), 8.0);
    let first = texts(&frame_output(&ctx, &mut rack, &patch, 0.0, vec![]).1);
    assert_eq!(
        first
            .iter()
            .filter(|(text, _)| text == "UNAVAILABLE")
            .count(),
        7
    );
    assert!(first
        .iter()
        .any(|(text, _)| text == "HELD VALUES UNAVAILABLE"));
    let snapshot = json!({"values":(0..7).map(|output| (0..16).map(|channel| -9.75 + output as f64 + channel as f64 / 100.0).collect::<Vec<_>>()).collect::<Vec<_>>(), "rng_state":4_000_000_000_u32,"active_channels":16});
    rack.set_random_values_states(BTreeMap::from([("random".into(), snapshot.clone())]));
    rack.set_sequence_states(BTreeMap::new(), true);
    let live = texts(&frame_output(&ctx, &mut rack, &patch, 0.1, vec![]).1);
    for output in 0..7 {
        assert!(live
            .iter()
            .any(|(text, _)| text == &format!("CH 1  {:+.3} V", -9.75 + output as f64)));
    }
    assert!(live.iter().any(|(text, _)| text == "LIVE · 16 CHANNELS"));
    patch.nodes[0].params.insert("range".into(), 2.0);
    patch.nodes[0].params.insert("seed".into(), 123.0);
    assert_eq!(
        rack.random_values_states["random"], snapshot,
        "parameter values never simulate a held draw"
    );
    rack.set_sequence_states(BTreeMap::new(), false);
    assert!(texts(&frame_output(&ctx, &mut rack, &patch, 0.2, vec![]).1)
        .iter()
        .any(|(text, _)| text == "LAST ENGINE STATE · 16 CHANNELS"));
    for invalid in [
        json!({}),
        json!({"values":[[1.0]], "rng_state":1,"active_channels":1}),
        json!({"values":snapshot["values"], "rng_state":1,"active_channels":17}),
    ] {
        rack.set_random_values_states(BTreeMap::from([("random".into(), invalid)]));
        assert!(rack.random_values_states.is_empty());
    }
}

#[test]
fn random_values_committed_edits_anchor_the_actual_controls_without_drawing_values() {
    use crate::construction_motion::MotionMode;
    use std::time::{Duration, Instant};

    for zoom in [0.42, 0.85, 1.0] {
        let ctx = egui::Context::default();
        let mut rack = Rack {
            zoom,
            ..Rack::default()
        };
        let before = random_patch();
        let mut after = before.clone();
        after.nodes[0].params.insert("range".into(), 5.0);
        after.nodes[0].params.insert("seed".into(), 16_777_215.0);
        frame_revision_state(&ctx, &mut rack, &before, 0.0, vec![], true, true, 9);
        let receipt = json!({"plan_id":13,"completed_steps":1,"last_revision":10,"current_revision":10,
        "last_commit":{"age_ms":0,"invalidate_existing_cables":false,"plan_id":13,"step_index":0,
        "before_revision":9,"revision":10,"label":"Set random draw controls","new_modules":[],"new_cables":[],
        "parameter_changes":[
            {"id":"random","kind":"random_values","parameter":"range","old_effective":0.0,"new_effective":5.0},
            {"id":"random","kind":"random_values","parameter":"seed","old_effective":1.0,"new_effective":16777215.0}
        ]}});
        let now = Instant::now();
        rack.observe_construction_transition_at(
            &before,
            &after,
            9,
            10,
            &receipt,
            MotionMode::Full,
            now,
        );
        rack.set_construction_clock_override(Some(now + Duration::from_millis(160)));
        let (action, output) =
            frame_revision_state(&ctx, &mut rack, &after, 0.1, vec![], true, true, 10);
        assert!(action.is_none());
        assert!(
            rack.random_values_states.is_empty(),
            "an edit cue cannot synthesize a held draw"
        );
        assert_eq!(rack.construction_anchors.len(), 2);
        for (parameter, local) in [
            ("range", random_values_range_rect()),
            ("seed", random_values_seed_rect()),
        ] {
            let anchor = &rack.construction_anchors[&("random".into(), parameter.into())];
            assert_eq!(anchor.rect, (local * zoom).translate(vec2(24.0, 12.0)));
            assert!(anchor.marker.is_none());
        }
        let seed = &rack.construction_anchors[&("random".into(), "seed".into())];
        assert_eq!(
            (seed.old_text.as_str(), seed.new_text.as_str()),
            ("1", "16777215")
        );
        assert_eq!(
            texts(&output)
                .iter()
                .filter(|(text, _)| text == "UNAVAILABLE")
                .count(),
            7
        );
    }
}
