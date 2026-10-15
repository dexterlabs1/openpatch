use super::*;
fn patch() -> Patch {
    serde_json::from_value(json!({"version":2,"nodes":[{"id":"gate","kind":"midi_gate"}]})).unwrap()
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
fn snapshot() -> Value {
    json!({"assignments":MIDI_GATE_DEFAULT_NOTES,"learning":null})
}
fn texts(output: &egui::FullOutput) -> Vec<(String, Rect)> {
    fn visit(shape: &egui::Shape, out: &mut Vec<(String, Rect)>) {
        match shape {
            egui::Shape::Text(t) if !t.galley.text().is_empty() => out.push((
                t.galley.text().into(),
                Rect::from_min_size(t.pos, t.galley.size()),
            )),
            egui::Shape::Vec(children) => {
                for child in children {
                    visit(child, out);
                }
            }
            _ => {}
        }
    }
    let mut out = vec![];
    for shape in &output.shapes {
        visit(&shape.shape, &mut out);
    }
    out
}
#[test]
fn midi_gate_exact_integer_entry_and_readable_note_names_are_unambiguous() {
    let patch = patch();
    for (number, label) in [(-1, "—"), (0, "C-1"), (60, "C4"), (127, "G9")] {
        assert_eq!(midi_gate_note_name(number), label);
        let mut edit = midi_gate_edit(&patch.nodes[0], 15, number, 71);
        edit.text = number.to_string();
        let value = parameter_edit_raw_value(&edit, parsed_parameter_result(&edit).unwrap());
        assert_eq!(value, f32::from(number));
        assert!(
            matches!(parameter_edit_action(&edit,value),RackAction::Command("set_parameter",args)
            if args["parameter"]=="note16" && args["value"].as_f64()==Some(f64::from(number)) && args["expected_revision"]==71)
        );
    }
    for invalid in ["-2", "128", "60.5", "C4"] {
        let mut edit = midi_gate_edit(&patch.nodes[0], 0, 48, 71);
        edit.text = invalid.into();
        assert!(parsed_parameter_result(&edit).is_err());
    }
}
#[test]
fn midi_gate_every_note_cell_click_and_sixteen_control_tab_order_work() {
    for (index, parameter) in MIDI_GATE_PARAMS.iter().enumerate() {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let patch = patch();
        frame(&ctx, &mut rack, &patch, 0.0, vec![]);
        let p = midi_assignment_value_rect(index).center() + vec2(24.0, 12.0);
        frame(
            &ctx,
            &mut rack,
            &patch,
            1.0,
            vec![egui::Event::PointerMoved(p), button(p, true)],
        );
        frame(&ctx, &mut rack, &patch, 1.1, vec![button(p, false)]);
        assert_eq!(rack.editing.as_ref().unwrap().parameter, *parameter);
    }
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch = patch();
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 9);
    rack.request_precision_control_for_smoke("gate", "note1");
    frame_revision_state(&ctx, &mut rack, &patch, 0.1, vec![], true, true, 9);
    frame_revision_state(&ctx, &mut rack, &patch, 0.2, vec![], true, true, 9);
    assert_eq!(
        rack.precision_controls
            .iter()
            .map(|c| c.edit.parameter.as_str())
            .collect::<Vec<_>>(),
        MIDI_GATE_PARAMS
    );
    rack.editing.as_mut().unwrap().text = "49".into();
    let action = frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        0.3,
        vec![key(egui::Key::Tab)],
        true,
        true,
        9,
    )
    .0;
    assert!(
        matches!(action,Some(RackAction::ParameterEditAndNavigate{arguments:args,forward:true})
        if args["parameter"]=="note1" && args["value"]==49.0 && args["expected_revision"]==9)
    );
    assert!(
        patch.nodes[0].params.is_empty(),
        "collision resolution belongs to the host"
    );
    rack.complete_parameter_navigation(Ok(10), true);
    assert_eq!(rack.precision_next.as_ref().unwrap().parameter, "note2");
}
#[test]
fn midi_gate_saved_collision_fallback_and_precision_entry_use_core_normalization() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let mut patch = patch();
    patch.nodes[0].params.insert("note1".into(), 49.0);
    let output = frame_output(&ctx, &mut rack, &patch, 0.0, vec![]).1;
    assert_eq!(
        texts(&output).iter().filter(|(t, _)| t == "49 C#3").count(),
        1
    );
    assert!(texts(&output).iter().any(|(t, _)| t == "—"));
    rack.request_precision_control_for_smoke("gate", "note1");
    frame(&ctx, &mut rack, &patch, 0.1, vec![]);
    frame(&ctx, &mut rack, &patch, 0.2, vec![]);
    assert_eq!(rack.editing.as_ref().unwrap().text, "-1");
}
#[test]
fn midi_gate_popup_learn_and_cancel_are_revision_guarded_requests_only() {
    for cancel in [false, true] {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let patch = patch();
        if cancel {
            let mut s = snapshot();
            s["learning"] = json!({"cell":16,"serial":83});
            rack.set_midi_gate_states(BTreeMap::from([("gate".into(), s)]));
        }
        frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 41);
        let p = midi_assignment_value_rect(15).center() + vec2(24.0, 12.0);
        let right = |pressed| egui::Event::PointerButton {
            pos: p,
            button: PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.0,
            vec![egui::Event::PointerMoved(p), right(true)],
            true,
            true,
            41,
        );
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.1,
            vec![right(false)],
            true,
            true,
            41,
        );
        let output = frame_revision_state(&ctx, &mut rack, &patch, 1.11, vec![], true, true, 41).1;
        let target = texts(&output)
            .into_iter()
            .find(|(t, _)| {
                t == if cancel {
                    "Cancel MIDI Learn"
                } else {
                    "Learn MIDI note"
                }
            })
            .unwrap()
            .1
            .center();
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.2,
            vec![egui::Event::PointerMoved(target), button(target, true)],
            true,
            true,
            41,
        );
        let action = frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.3,
            vec![button(target, false)],
            true,
            true,
            41,
        )
        .0;
        assert!(
            matches!(action,Some(RackAction::Command(command,args)) if command==if cancel {"cancel_midi_gate_learn"} else {"learn_midi_gate"}
            && args["id"]=="gate" && args["expected_revision"]==41 && (cancel || args["cell"]==16))
        );
        assert_eq!(
            rack.midi_gate_states
                .get("gate")
                .and_then(|s| s["learning"]["cell"].as_u64()),
            if cancel { Some(16) } else { None }
        );
    }
}
#[test]
fn midi_gate_panic_is_one_shot_available_bypassed_and_has_no_held_state() {
    for bypassed in [false, true] {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let mut patch = patch();
        patch.nodes[0].bypassed = bypassed;
        frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 31);
        let p = midi_gate_panic_rect().center() + vec2(24.0, 12.0);
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.0,
            vec![egui::Event::PointerMoved(p), button(p, true)],
            true,
            true,
            31,
        );
        let action = frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.1,
            vec![button(p, false)],
            true,
            true,
            31,
        )
        .0;
        assert!(
            matches!(action,Some(RackAction::Command("trigger_module",args))
            if args["action"]=="midi_panic" && args["id"]=="gate" && args["expected_revision"]==31)
        );
        assert!(rack.held_manual.is_none());
        assert!(frame(&ctx, &mut rack, &patch, 1.2, vec![]).is_none());
    }
}
#[test]
fn midi_gate_velocity_and_mpe_controls_dispatch_boolean_parameters() {
    for (index, parameter) in ["velocity", "mpe"].into_iter().enumerate() {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let patch = patch();
        frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 23);
        let p = pos2(24.0 + 28.0 + index as f32 * 150.0, 12.0 + 357.0);
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.0,
            vec![egui::Event::PointerMoved(p), button(p, true)],
            true,
            true,
            23,
        );
        let action = frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.1,
            vec![button(p, false)],
            true,
            true,
            23,
        )
        .0;
        assert!(
            matches!(action,Some(RackAction::Command("set_parameter",args)) if args["parameter"]==parameter && args["value"]==1.0 && args["expected_revision"]==23)
        );
    }
}
#[test]
fn midi_gate_activity_and_learning_use_actual_telemetry_and_emit_in_darkroom() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch = patch();
    let centers = (0..16)
        .map(|i| midi_assignment_cell_rect(i).min + vec2(8.0 + 24.0, 30.0 + 12.0))
        .collect::<Vec<_>>();
    let colors = |out: &egui::FullOutput| {
        centers
            .iter()
            .map(|center| {
                out.shapes
                    .iter()
                    .find_map(|s| match &s.shape {
                        egui::Shape::Circle(c)
                            if c.center.distance(*center) < 0.1
                                && (c.radius - 2.3).abs() < 0.01 =>
                        {
                            Some(c.fill)
                        }
                        _ => None,
                    })
                    .unwrap()
            })
            .collect::<Vec<_>>()
    };
    let output = frame_output(&ctx, &mut rack, &patch, 0.0, vec![]).1;
    let off = colors(&output);
    assert!(!texts(&output).iter().any(|(t, _)| t == "LRN"));
    let mut s = snapshot();
    s["learning"] = json!({"cell":16,"serial":99});
    rack.set_midi_gate_states(BTreeMap::from([("gate".into(), s)]));
    let mut lights = ModuleLightState {
        count: 16,
        ..Default::default()
    };
    for i in 0..16 {
        lights.brightness[i][0] = 1.0;
    }
    rack.set_module_states(BTreeMap::from([("gate".into(), lights)]), BTreeMap::new());
    let output = frame_output(&ctx, &mut rack, &patch, 0.1, vec![]).1;
    let lit = colors(&output);
    assert!(texts(&output).iter().any(|(t, _)| t == "LRN"));
    for i in 0..16 {
        assert_ne!(off[i], lit[i]);
    }
    rack.set_room_brightness(0.0);
    let dark = frame_output(&ctx, &mut rack, &patch, 0.2, vec![]).1;
    assert_eq!(
        colors(&dark),
        lit,
        "fully active gate lamps remain emissive with room lights off"
    );
    rack.set_module_states(BTreeMap::new(), BTreeMap::new());
    rack.set_midi_gate_states(BTreeMap::new());
    let dark_off = colors(&frame_output(&ctx, &mut rack, &patch, 0.3, vec![]).1);
    assert!(dark_off
        .iter()
        .all(|c| c.r() == 0 && c.g() == 0 && c.b() == 0));
}
#[test]
fn midi_gate_shared_browser_geometry_text_and_truthful_defaults_at_all_scales() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "midi_gate")
        .unwrap();
    assert!(module["inputs"].as_array().unwrap().is_empty());
    assert_eq!(module["outputs"].as_array().unwrap().len(), 16);
    assert_eq!(
        ordered_parameters(&patch().nodes[0], Some(module)).len(),
        19
    );
    for z in [0.42, 0.85, 1.0] {
        let ctx = egui::Context::default();
        crate::panel_skin::install_fonts(&ctx);
        let mut preview = ModulePreview::new(module);
        let rect = Rect::from_min_size(
            pos2(12.0, 12.0),
            vec2(panel_width("midi_gate"), PANEL_HEIGHT) * z,
        );
        for _ in 0..2 {
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(900.0, 700.0))),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| preview.show(ui, rect, &catalog));
                },
            );
            let labels = texts(&output);
            for (i, note) in MIDI_GATE_DEFAULT_NOTES.iter().enumerate() {
                for label in [
                    format!("GATE {}", i + 1),
                    format!("{} {}", note, midi_gate_note_name(*note)),
                ] {
                    assert!(labels.iter().any(|(t, _)| t == &label));
                }
            }
            for required in ["Panic", "Velocity", "MPE"] {
                assert!(labels.iter().any(|(t, _)| t == required));
            }
            assert!(labels
                .iter()
                .any(|(t, _)| t.starts_with("Global MIDI input")));
            assert!(!labels.iter().any(|(t, _)| t == "LRN"));
            for (i, (label, a)) in labels.iter().enumerate() {
                assert!(rect.expand(0.5).contains_rect(*a), "{z}: {label} outside");
                for (other, b) in &labels[i + 1..] {
                    assert!(
                        !a.shrink(0.2 * z).intersects(b.shrink(0.2 * z)),
                        "{z}: {label} overlaps {other}"
                    );
                }
            }
            assert!(preview.renderer.editing.is_none());
            assert!(preview.renderer.midi_gate_states.is_empty());
        }
        let local_bounds =
            Rect::from_min_size(Pos2::ZERO, vec2(panel_width("midi_gate"), PANEL_HEIGHT) * z);
        let scale = |r: Rect| {
            Rect::from_min_max(
                (r.min.to_vec2() * z).to_pos2(),
                (r.max.to_vec2() * z).to_pos2(),
            )
        };
        let regions = (0..16)
            .flat_map(|i| {
                [
                    scale(midi_assignment_value_rect(i)),
                    Rect::from_center_size(
                        (port_position("midi_gate", false, i, 16, 480.0).to_vec2() * z).to_pos2(),
                        Vec2::splat((29.0 * z).max(12.0)),
                    ),
                ]
            })
            .chain([scale(midi_gate_panic_rect())])
            .collect::<Vec<_>>();
        for (i, a) in regions.iter().enumerate() {
            assert!(local_bounds.contains_rect(*a));
            for b in &regions[i + 1..] {
                assert!(!a.intersects(*b));
            }
        }
    }
}
