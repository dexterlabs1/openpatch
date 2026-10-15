use super::*;

fn patch() -> Patch {
    serde_json::from_value(json!({"version":2,"nodes":[{"id":"shift","kind":"sh_asr"}]})).unwrap()
}
fn texts(output: &egui::FullOutput) -> Vec<(String, Rect)> {
    fn collect(shape: &egui::Shape, out: &mut Vec<(String, Rect)>) {
        match shape {
            egui::Shape::Vec(children) => children.iter().for_each(|s| collect(s, out)),
            egui::Shape::Text(t) => out.push((
                t.galley.text().into(),
                Rect::from_min_size(t.pos, t.galley.size()),
            )),
            _ => {}
        }
    }
    let mut out = vec![];
    for s in &output.shapes {
        collect(&s.shape, &mut out);
    }
    out
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
fn click(
    ctx: &egui::Context,
    rack: &mut Rack,
    patch: &Patch,
    local: Rect,
    revision: u64,
) -> Option<RackAction> {
    let at = local.center() + vec2(24.0, 12.0);
    assert!(frame_revision_state(
        ctx,
        rack,
        patch,
        1.0,
        vec![egui::Event::PointerMoved(at), button(at, true)],
        true,
        true,
        revision
    )
    .0
    .is_none());
    frame_revision_state(
        ctx,
        rack,
        patch,
        1.1,
        vec![button(at, false)],
        true,
        true,
        revision,
    )
    .0
}

#[test]
fn shift_register_hidden_snapshots_are_absent_from_controls_previews_and_state_meters() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "sh_asr")
        .unwrap();
    assert_eq!(
        ordered_parameters(&patch().nodes[0], Some(module))
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>(),
        ["random", "range", "seed"]
    );
    assert_eq!(
        module["params"]
            .as_object()
            .unwrap()
            .values()
            .filter(|spec| spec["hidden"] == true)
            .count(),
        10
    );
    let preview = ModulePreview::new(module);
    assert_eq!(
        preview.patch.nodes[0]
            .params
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["random", "range", "seed"]
    );
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let mut patch = patch();
    patch.nodes[0].params.insert("value1".into(), 99.0);
    assert_eq!(
        texts(&frame_output(&ctx, &mut rack, &patch, 0.0, vec![]).1)
            .iter()
            .filter(|(t, _)| t == "UNAVAILABLE")
            .count(),
        8
    );
    let actual =
        json!({"values":[-10.0,-5.0,-1.0,0.0,1.0,5.0,10.0,1_000_000.0],"rng_state":u32::MAX});
    rack.set_shift_register_states(BTreeMap::from([("shift".into(), actual.clone())]));
    rack.set_sequence_states(BTreeMap::new(), true);
    let output = texts(&frame_output(&ctx, &mut rack, &patch, 0.1, vec![]).1);
    for value in actual["values"].as_array().unwrap() {
        assert!(output
            .iter()
            .any(|(t, _)| t == &format!("{:+.3} V", value.as_f64().unwrap())));
    }
    assert!(output
        .iter()
        .any(|(t, _)| t.starts_with("LIVE MONO VALUES")));
    rack.set_sequence_states(BTreeMap::new(), false);
    assert!(texts(&frame_output(&ctx, &mut rack, &patch, 0.2, vec![]).1)
        .iter()
        .any(|(t, _)| t.starts_with("LAST ENGINE STATE")));
    for invalid in [
        json!({}),
        json!({"values":[0,0,0,0,0,0,0],"rng_state":1}),
        json!({"values":[0,0,0,0,0,0,0,0,0],"rng_state":1}),
        json!({"values":[0,0,0,0,0,0,0,1_000_001],"rng_state":1}),
        json!({"values":[0,0,0,0,0,0,0,-1_000_001],"rng_state":1}),
        json!({"values":[0,0,0,0,0,0,0,null],"rng_state":1}),
        json!({"values":[0,0,0,0,0,0,0,0],"rng_state":0}),
        json!({"values":[0,0,0,0,0,0,0,0],"rng_state":4_294_967_296_u64}),
        json!({"values":[0,0,0,0,0,0,0,0],"rng_state":1.5}),
    ] {
        rack.set_shift_register_states(BTreeMap::from([("shift".into(), invalid)]));
        assert!(rack.shift_register_states.is_empty());
    }
}

#[test]
fn shift_register_seed_and_rnd_publish_only_revision_guarded_controls() {
    for (text, expected) in [
        ("0", 0.0),
        ("2^24-1", 16_777_215.0),
        ("1234567", 1_234_567.0),
    ] {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let patch = patch();
        frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 77);
        assert!(click(&ctx, &mut rack, &patch, shift_register_seed_rect(), 77).is_none());
        let edit = rack.editing.as_mut().expect("one-click seed editor");
        assert_eq!(
            (edit.min, edit.max, edit.step),
            (0.0, 16_777_215.0, Some(1.0))
        );
        edit.text = text.into();
        assert!(
            matches!(frame_revision_state(&ctx, &mut rack, &patch, 1.2, vec![key(egui::Key::Enter)], true, true, 77).0, Some(RackAction::Command("set_parameter", args)) if args["id"]=="shift" && args["parameter"]=="seed" && args["expected_revision"]==77 && args["value"]==expected)
        );
    }
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch = patch();
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 17);
    assert!(
        matches!(click(&ctx, &mut rack, &patch, shift_register_random_rect(),17),Some(RackAction::Command("set_parameter",args)) if args["parameter"]=="random" && args["value"]==1 && args["expected_revision"]==17)
    );
}

#[test]
fn shift_register_range_chooser_exposes_six_guarded_choices() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch = patch();
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 17);
    assert!(click(&ctx, &mut rack, &patch, shift_register_range_rect(), 17).is_none());
    let output = frame_revision_state(&ctx, &mut rack, &patch, 1.2, vec![], true, true, 17).1;
    let options = texts(&output);
    for label in RANDOM_VALUES_RANGES {
        assert!(options.iter().any(|(t, _)| t == label));
    }
    let target = options
        .iter()
        .find(|(t, _)| t == RANDOM_VALUES_RANGES[5])
        .unwrap()
        .1
        .center();
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.3,
        vec![egui::Event::PointerMoved(target), button(target, true)],
        true,
        true,
        17,
    );
    assert!(
        matches!(frame_revision_state(&ctx,&mut rack,&patch,1.4,vec![button(target,false)],true,true,17).0,Some(RackAction::Command("set_parameter",args)) if args["parameter"]=="range" && args["value"]==5 && args["expected_revision"]==17)
    );
}

#[test]
fn shift_register_manual_controls_pair_once_and_release_under_every_interruption() {
    for (index, press, release) in [
        (0, "push_press", "push_release"),
        (1, "reset_press", "reset_release"),
    ] {
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
            let mut patch = patch();
            rack.set_sequence_states(BTreeMap::new(), true);
            frame(&ctx, &mut rack, &patch, 0.0, vec![]);
            let at = shift_register_button_rect(index).center() + vec2(24.0, 12.0);
            assert!(
                matches!(frame(&ctx,&mut rack,&patch,1.0,vec![egui::Event::PointerMoved(at),button(at,true)]),Some(RackAction::Command("trigger_module",args)) if args["id"]=="shift" && args["action"]==press)
            );
            assert!(frame(&ctx, &mut rack, &patch, 1.05, vec![]).is_none());
            let (events, focused, enabled) = match cause {
                "release" => (vec![button(at, false)], true, true),
                "exit" => (
                    vec![egui::Event::PointerMoved(at + vec2(0.0, 35.0))],
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
            assert!(
                matches!(frame_state(&ctx,&mut rack,&patch,1.1,events,focused,enabled).0,Some(RackAction::Command("trigger_module",args)) if args["action"]==release),
                "{press} {cause}"
            );
            assert!(rack.held_manual.is_none());
        }
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let patch = patch();
        rack.set_sequence_states(BTreeMap::new(), true);
        frame(&ctx, &mut rack, &patch, 0.0, vec![]);
        let at = shift_register_button_rect(index).center() + vec2(24.0, 12.0);
        assert!(
            matches!(frame(&ctx,&mut rack,&patch,1.0,vec![egui::Event::PointerMoved(at),button(at,true),button(at,false)]),Some(RackAction::ManualClick{id,press:p,release:r}) if id=="shift" && p==press && r==release)
        );
    }
}

#[test]
fn shift_register_all_twenty_four_ports_keep_separate_controls_and_patch_at_each_scale() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "sh_asr")
        .unwrap();
    let expected_inputs = (1..=8)
        .map(|i| format!("in{i}"))
        .chain((1..=8).map(|i| format!("trig{i}")))
        .collect::<Vec<_>>();
    assert_eq!(module["inputs"], json!(expected_inputs));
    for scale in [0.42, 0.85, 1.0] {
        let panel = Rect::from_min_size(
            Pos2::ZERO,
            vec2(panel_width("sh_asr"), PANEL_HEIGHT) * scale,
        );
        let sockets = [true, false]
            .into_iter()
            .flat_map(|input| {
                (0..if input { 16 } else { 8 }).map(move |index| {
                    Rect::from_center_size(
                        port_position(
                            "sh_asr",
                            input,
                            index,
                            if input { 16 } else { 8 },
                            panel_width("sh_asr"),
                        ) * scale,
                        Vec2::splat((29.0 * scale).max(12.0)),
                    )
                })
            })
            .collect::<Vec<_>>();
        for (index, socket) in sockets.iter().enumerate() {
            assert!(panel.contains_rect(*socket));
            for other in &sockets[index + 1..] {
                assert!(!socket.intersects(*other), "socket collision at {scale}");
            }
            for control in [
                shift_register_random_rect(),
                shift_register_range_rect(),
                shift_register_seed_rect(),
                shift_register_button_rect(0),
                shift_register_button_rect(1),
            ] {
                assert!(!socket.intersects(control * scale));
            }
            for row in 0..8 {
                assert!(!socket.intersects(shift_register_readout_rect(row) * scale));
            }
        }
        for input in [true, false] {
            for port in module[if input { "inputs" } else { "outputs" }]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| p.as_str().unwrap())
            {
                let ctx = egui::Context::default();
                let mut rack = Rack {
                    zoom: scale,
                    ..Rack::default()
                };
                let patch:Patch=serde_json::from_value(json!({"version":2,"nodes":[{"id":"shift","kind":"sh_asr"},{"id":"sink","kind":"audio_out"},{"id":"clock","kind":"gate"}]})).unwrap();
                frame(&ctx, &mut rack, &patch, 0.0, vec![]);
                let target = fixture_jack(&rack, &patch, "shift", port, input);
                let canvas = Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 700.0));
                let expected = if input {
                    CableEnd::Input {
                        node: "shift".into(),
                        port: port.into(),
                    }
                } else {
                    CableEnd::Output {
                        node: "shift".into(),
                        port: port.into(),
                    }
                };
                assert_eq!(
                    rack.cable_press_target(&patch, &catalog, canvas, canvas, target),
                    Some(expected)
                );
                let other = fixture_jack(
                    &rack,
                    &patch,
                    if input { "clock" } else { "sink" },
                    if input { "out" } else { "signal" },
                    !input,
                );
                let (from, to) = if input {
                    (other, target)
                } else {
                    (target, other)
                };
                assert!(
                    matches!(drag(&ctx,&mut rack,&patch,1.0,from,to),Some(RackAction::Command("connect",args)) if if input{args["to"]=="shift" && args["input"]==port}else{args["from"]=="shift" && args["output"]==port}),
                    "{port} {scale}"
                );
            }
        }
    }
}

#[test]
fn shift_register_seed_rejects_invalid_or_stale_edits_and_preserves_actual_readouts() {
    for draft in ["-1", "16777216", "1.5", "1/0", "bad"] {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let patch = patch();
        frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 9);
        let state = json!({"values":[1,2,3,4,5,6,7,8],"rng_state":123});
        rack.set_shift_register_states(BTreeMap::from([("shift".into(), state.clone())]));
        assert!(click(&ctx, &mut rack, &patch, shift_register_seed_rect(), 9).is_none());
        rack.editing.as_mut().unwrap().text = draft.into();
        assert!(
            frame_revision_state(
                &ctx,
                &mut rack,
                &patch,
                1.2,
                vec![key(egui::Key::Enter)],
                true,
                true,
                9
            )
            .0
            .is_none(),
            "{draft}"
        );
        assert!(rack.editing.is_some());
        rack.editing.as_mut().unwrap().text = "42".into();
        assert!(frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.3,
            vec![key(egui::Key::Enter)],
            true,
            true,
            10
        )
        .0
        .is_none());
        assert!(rack.editing.as_ref().unwrap().stale);
        assert_eq!(rack.shift_register_states["shift"], state);
    }
}
