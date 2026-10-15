use super::*;
fn patch() -> Patch {
    serde_json::from_value(json!({"version":2,"nodes":[{"id":"cc","kind":"midi_cc"}]})).unwrap()
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
    json!({"assignments":(1..=16).collect::<Vec<_>>(),"values":vec![0;128],"learning":null})
}
fn texts(output: &egui::FullOutput) -> Vec<(String, Rect)> {
    fn collect(shape: &egui::Shape, result: &mut Vec<(String, Rect)>) {
        match shape {
            egui::Shape::Text(t) if !t.galley.text().is_empty() => result.push((
                t.galley.text().into(),
                Rect::from_min_size(t.pos, t.galley.size()),
            )),
            egui::Shape::Vec(children) => {
                for child in children {
                    collect(child, result);
                }
            }
            _ => {}
        }
    }
    let mut result = vec![];
    for shape in &output.shapes {
        collect(&shape.shape, &mut result);
    }
    result
}
#[test]
fn midi_cc_exact_integer_entry_preserves_unmapped_zero_and_127() {
    let patch = patch();
    for raw in [-1, 0, 127] {
        let mut edit = midi_cc_edit(&patch.nodes[0], 15, 16, 73);
        edit.text = raw.to_string();
        let value = parameter_edit_raw_value(&edit, parsed_parameter_result(&edit).unwrap());
        assert_eq!(value, raw as f32);
        assert!(
            matches!(parameter_edit_action(&edit,value),RackAction::Command("set_parameter",args)
            if args["id"]=="cc" && args["parameter"]=="cc16" && args["value"].as_f64()==Some(f64::from(raw)) && args["expected_revision"]==73)
        );
    }
    for invalid in ["-2", "128", "1.5"] {
        let mut edit = midi_cc_edit(&patch.nodes[0], 0, 1, 73);
        edit.text = invalid.into();
        assert!(parsed_parameter_result(&edit).is_err(), "{invalid}");
    }
}
#[test]
fn midi_cc_precision_tab_visits_sixteen_cells_and_defers_collision_to_host() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch = patch();
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 9);
    rack.request_precision_control_for_smoke("cc", "cc1");
    frame_revision_state(&ctx, &mut rack, &patch, 0.1, vec![], true, true, 9);
    frame_revision_state(&ctx, &mut rack, &patch, 0.2, vec![], true, true, 9);
    assert_eq!(
        rack.precision_controls
            .iter()
            .map(|c| c.edit.parameter.as_str())
            .collect::<Vec<_>>(),
        MIDI_CC_PARAMS
    );
    rack.editing.as_mut().unwrap().text = "2".into();
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
        if args["parameter"]=="cc1" && args["value"]==2.0 && args["expected_revision"]==9)
    );
    assert!(
        patch.nodes[0].params.is_empty(),
        "the UI must not resolve collisions or modify the patch"
    );
    rack.complete_parameter_navigation(Ok(10), true);
    assert_eq!(rack.precision_next.as_ref().unwrap().parameter, "cc2");
}
#[test]
fn midi_cc_assignment_click_opens_exact_editor_at_every_cell() {
    for (index, parameter) in MIDI_CC_PARAMS.iter().enumerate() {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let patch = patch();
        frame(&ctx, &mut rack, &patch, 0.0, vec![]);
        let p = midi_cc_value_rect(index).center() + vec2(24.0, 12.0);
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
}
#[test]
fn midi_cc_learn_and_cancel_popup_dispatch_revision_guarded_host_commands() {
    for cancel in [false, true] {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let patch = patch();
        if cancel {
            let mut s = snapshot();
            s["learning"] = json!({"cell":4,"serial":83});
            rack.set_midi_cc_states(BTreeMap::from([("cc".into(), s)]));
        }
        frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 41);
        let p = midi_cc_value_rect(3).center() + vec2(24.0, 12.0);
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
        let _ = frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.1,
            vec![right(false)],
            true,
            true,
            41,
        )
        .1;
        let output = frame_revision_state(&ctx, &mut rack, &patch, 1.11, vec![], true, true, 41).1;
        let target = texts(&output)
            .into_iter()
            .find(|(t, _)| {
                t == if cancel {
                    "Cancel MIDI Learn"
                } else {
                    "Learn MIDI CC"
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
        assert!(matches!(action,Some(RackAction::Command(command,args))
            if command==if cancel {"cancel_midi_cc_learn"} else {"learn_midi_cc"}
                && args["id"]=="cc" && args["expected_revision"]==41
                && (cancel || args["cell"]==4)));
        assert_eq!(
            rack.midi_cc_states
                .get("cc")
                .and_then(|s| s["learning"]["cell"].as_u64()),
            if cancel { Some(4) } else { None },
            "command requests must not fabricate or clear authoritative Learn state"
        );
    }
}
#[test]
fn midi_cc_only_actual_telemetry_drives_learn_assignment_and_values() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch = patch();
    let before = texts(&frame_output(&ctx, &mut rack, &patch, 0.0, vec![]).1);
    assert!(!before
        .iter()
        .any(|(t, _)| t == "LRN" || t.starts_with("VALUE ")));
    let mut s = snapshot();
    s["assignments"][0] = json!(127);
    s["values"][127] = json!(92);
    s["learning"] = json!({"cell":16,"serial":5});
    rack.set_midi_cc_states(BTreeMap::from([("cc".into(), s.clone())]));
    let after = texts(&frame_output(&ctx, &mut rack, &patch, 0.1, vec![]).1);
    for label in ["CC 127", "VALUE 92", "LRN"] {
        assert!(after.iter().any(|(t, _)| t == label));
    }
    s["learning"]["cell"] = json!(17);
    rack.set_midi_cc_states(BTreeMap::from([("cc".into(), s)]));
    assert!(rack.midi_cc_states.is_empty());
    assert!(
        !texts(&frame_output(&ctx, &mut rack, &patch, 0.2, vec![]).1)
            .iter()
            .any(|(t, _)| t == "LRN")
    );
}
#[test]
fn midi_cc_sixteen_cells_and_jacks_have_nonoverlapping_targets_at_all_scales() {
    for z in [0.42_f32, 0.85, 1.0] {
        let bounds =
            Rect::from_min_size(Pos2::ZERO, vec2(panel_width("midi_cc"), PANEL_HEIGHT) * z);
        let scale = |r: Rect| {
            Rect::from_min_max(
                (r.min.to_vec2() * z).to_pos2(),
                (r.max.to_vec2() * z).to_pos2(),
            )
        };
        let controls = (0..16)
            .map(|i| scale(midi_cc_value_rect(i)))
            .collect::<Vec<_>>();
        let jacks = (0..16)
            .map(|i| {
                Rect::from_center_size(
                    (port_position("midi_cc", false, i, 16, panel_width("midi_cc")).to_vec2() * z)
                        .to_pos2(),
                    Vec2::splat((29.0 * z).max(12.0)),
                )
            })
            .collect::<Vec<_>>();
        for (i, a) in controls.iter().chain(&jacks).enumerate() {
            assert!(bounds.contains_rect(*a));
            for b in controls.iter().chain(&jacks).skip(i + 1) {
                assert!(!a.intersects(*b), "{z}: {a:?} / {b:?}");
            }
        }
    }
}
#[test]
fn midi_cc_catalog_hidden_values_and_browser_shared_panel_text_fit() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "midi_cc")
        .unwrap();
    assert!(module["inputs"].as_array().unwrap().is_empty());
    assert_eq!(module["outputs"].as_array().unwrap().len(), 16);
    let params = ordered_parameters(&patch().nodes[0], Some(module));
    assert_eq!(params.len(), 20);
    assert!(!params.iter().any(|(name, _)| name.starts_with("cc_value")));
    for z in [0.42, 0.85, 1.0] {
        let ctx = egui::Context::default();
        crate::panel_skin::install_fonts(&ctx);
        let mut preview = ModulePreview::new(module);
        let rect = Rect::from_min_size(
            pos2(12.0, 12.0),
            vec2(panel_width("midi_cc"), PANEL_HEIGHT) * z,
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
            for i in 1..=16 {
                for label in [format!("CC {i}"), format!("OUT {i}")] {
                    assert!(labels.iter().any(|(t, _)| t == &label), "missing {label}");
                }
            }
            assert!(labels
                .iter()
                .any(|(t, _)| t.starts_with("Global MIDI input")));
            for (i, (label, a)) in labels.iter().enumerate() {
                assert!(
                    rect.expand(0.5).contains_rect(*a),
                    "{z}: {label} outside {a:?}"
                );
                for (other, b) in &labels[i + 1..] {
                    assert!(
                        !a.shrink(0.2 * z).intersects(b.shrink(0.2 * z)),
                        "{z}: {label} overlaps {other}"
                    );
                }
            }
            assert!(preview.renderer.editing.is_none());
            assert!(preview.renderer.midi_cc_states.is_empty());
        }
    }
}

#[test]
fn midi_cc_option_controls_dispatch_saved_parameters_with_revision() {
    for (index, parameter, expected) in [(0, "smooth", 0), (1, "mpe", 1), (2, "high_resolution", 1)]
    {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let patch = patch();
        frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 55);
        let p = pos2(24.0 + 28.0 + index as f32 * 150.0, 12.0 + 357.0);
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.0,
            vec![egui::Event::PointerMoved(p), button(p, true)],
            true,
            true,
            55,
        );
        let action = frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.1,
            vec![button(p, false)],
            true,
            true,
            55,
        )
        .0;
        assert!(
            matches!(action,Some(RackAction::Command("set_parameter",args))
            if args["parameter"]==parameter && args["value"].as_f64()==Some(f64::from(expected)) && args["expected_revision"]==55),
            "{parameter}"
        );
    }
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch = patch();
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 55);
    let p = pos2(24.0 + 150.0, 12.0 + 326.0);
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.0,
        vec![egui::Event::PointerMoved(p), button(p, true)],
        true,
        true,
        55,
    );
    let _ = frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.1,
        vec![button(p, false)],
        true,
        true,
        55,
    )
    .1;
    let output = frame_revision_state(&ctx, &mut rack, &patch, 1.11, vec![], true, true, 55).1;
    let target = texts(&output)
        .into_iter()
        .find(|(t, _)| t == "Channel 3")
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
        55,
    );
    let action = frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.3,
        vec![button(target, false)],
        true,
        true,
        55,
    )
    .0;
    assert!(
        matches!(action,Some(RackAction::Command("set_parameter",args))
        if args["parameter"]=="channel" && args["value"]==3 && args["expected_revision"]==55)
    );
}

#[test]
fn midi_cc_saved_collision_fallback_and_exact_entry_match_dsp_normalization() {
    for (parameter, expected) in [("cc1", "-1"), ("cc2", "2")] {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let mut patch = patch();
        patch.nodes[0].params.insert("cc1".into(), 2.0);
        let output = frame_output(&ctx, &mut rack, &patch, 0.0, vec![]).1;
        let rendered = texts(&output);
        assert_eq!(
            rendered.iter().filter(|(text, _)| text == "CC 2").count(),
            1
        );
        assert!(rendered.iter().any(|(text, _)| text == "—"));
        assert!(rack.midi_cc_states.is_empty());
        rack.request_precision_control_for_smoke("cc", parameter);
        frame_output(&ctx, &mut rack, &patch, 0.1, vec![]);
        frame_output(&ctx, &mut rack, &patch, 0.2, vec![]);
        let edit = rack.editing.as_ref().unwrap();
        assert_eq!(edit.parameter, parameter);
        assert_eq!(edit.text, expected);
    }
}
