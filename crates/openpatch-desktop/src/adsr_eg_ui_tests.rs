use super::*;
fn patch() -> Patch {
    serde_json::from_value(json!({"version":2,"nodes":[{"id":"env","kind":"adsr_eg"}]})).unwrap()
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
fn at(local: Pos2) -> Pos2 {
    local + vec2(24.0, 12.0)
}
#[test]
fn adsr_eg_human_units_and_bipolar_depth_validate_and_convert() {
    let patch = patch();
    for i in 0..8 {
        let mut edit = adsr_eg_edit(&patch.nodes[0], i, 73);
        edit.text = if i < 4 && i != 2 { "100 ms" } else { "-35%" }.into();
        if i == 2 {
            edit.text = "35".into();
        }
        let human = parsed_parameter_result(&edit).unwrap();
        let raw = parameter_edit_raw_value(&edit, human);
        let expected = if i < 4 && i != 2 {
            0.5
        } else if i == 2 {
            0.35
        } else {
            -0.35
        };
        assert!((raw - expected).abs() < 1e-6);
        edit.text = if i < 4 && i != 2 { "0" } else { "101" }.into();
        assert!(parsed_parameter_result(&edit).is_err());
        if i < 4 && i != 2 {
            for (text, raw) in [("1", 0.0), ("10000", 1.0)] {
                edit.text = text.into();
                assert_eq!(
                    parameter_edit_raw_value(&edit, parsed_parameter_result(&edit).unwrap()),
                    raw
                );
            }
        }
    }
}
#[test]
fn adsr_eg_time_entry_and_tab_use_all_eight_guarded_controls() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch = patch();
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 9);
    rack.request_precision_control_for_smoke("env", "attack");
    frame_revision_state(&ctx, &mut rack, &patch, 0.1, vec![], true, true, 9);
    frame_revision_state(&ctx, &mut rack, &patch, 0.2, vec![], true, true, 9);
    assert_eq!(rack.precision_controls.len(), 8);
    assert_eq!(
        rack.precision_controls
            .iter()
            .map(|c| c.edit.parameter.as_str())
            .collect::<Vec<_>>(),
        ADSR_EG_PARAMS
    );
    rack.editing.as_mut().unwrap().text = "100 ms".into();
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
        matches!(action,Some(RackAction::ParameterEditAndNavigate{arguments:args,forward:true}) if args["parameter"]=="attack" && args["value"]==0.5 && args["expected_revision"]==9)
    );
    rack.complete_parameter_navigation(Ok(10), true);
    assert_eq!(rack.precision_next.as_ref().unwrap().parameter, "decay");
}
#[test]
fn adsr_eg_sliders_change_primary_and_bipolar_depth_values() {
    for (i, parameter) in ADSR_EG_PARAMS.iter().enumerate() {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let patch = patch();
        frame(&ctx, &mut rack, &patch, 0.0, vec![]);
        let start = at(adsr_eg_slider_rect(i).center());
        frame(
            &ctx,
            &mut rack,
            &patch,
            1.0,
            vec![egui::Event::PointerMoved(start), button(start, true)],
        );
        let end = start + vec2(if i < 4 { 25.0 } else { -25.0 }, 0.0);
        let action = frame(
            &ctx,
            &mut rack,
            &patch,
            1.1,
            vec![egui::Event::PointerMoved(end)],
        );
        assert!(
            matches!(action,Some(RackAction::Command("set_parameter",args)) if args["parameter"]==*parameter && if i<4 {args["value"].as_f64().unwrap()>0.5} else {args["value"].as_f64().unwrap()<0.0}),
            "slider {i}"
        );
    }
}
#[test]
fn adsr_eg_push_held_and_coalesced_click_release_safely() {
    for fast in [false, true] {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let patch = patch();
        rack.set_sequence_states(BTreeMap::new(), true);
        frame(&ctx, &mut rack, &patch, 0.0, vec![]);
        let pos = at(adsr_eg_push_rect().center());
        let mut events = vec![egui::Event::PointerMoved(pos), button(pos, true)];
        if fast {
            events.push(button(pos, false));
        }
        let action = frame(&ctx, &mut rack, &patch, 1.0, events);
        if fast {
            assert!(
                matches!(action,Some(RackAction::ManualClick {id,press:"push_press",release:"push_release"}) if id=="env")
            );
            assert!(rack.held_manual.is_none());
            assert!(frame(&ctx, &mut rack, &patch, 1.1, vec![]).is_none());
            continue;
        }
        assert!(
            matches!(action,Some(RackAction::Command("trigger_module",args)) if args["action"]=="push_press")
        );
        assert!(rack.held_manual.is_some());
        if !fast {
            assert!(frame(&ctx, &mut rack, &patch, 1.1, vec![]).is_none());
        }
        let action = frame(
            &ctx,
            &mut rack,
            &patch,
            1.2,
            if fast {
                vec![]
            } else {
                vec![button(pos, false)]
            },
        );
        assert!(
            matches!(action,Some(RackAction::Command("trigger_module",args)) if args["action"]=="push_release")
        );
        assert!(rack.held_manual.is_none());
    }
}
#[test]
fn adsr_eg_ports_and_control_bounds_are_separate_and_inside_panel() {
    let bounds = Rect::from_min_size(Pos2::ZERO, vec2(panel_width("adsr_eg"), PANEL_HEIGHT));
    let controls = (0..8)
        .flat_map(|i| [adsr_eg_slider_rect(i), adsr_eg_value_rect(i)])
        .chain([adsr_eg_push_rect()])
        .collect::<Vec<_>>();
    for r in &controls {
        assert!(bounds.contains_rect(*r));
    }
    for (i, a) in controls.iter().enumerate() {
        for b in &controls[i + 1..] {
            assert!(!a.intersects(*b));
        }
    }
    for i in 0..6 {
        let port = port_position("adsr_eg", true, i, 6, bounds.width());
        for control in &controls {
            assert!(!control.expand(14.0).contains(port));
        }
    }
}
fn collect_texts(shape: &egui::Shape, out: &mut Vec<(String, Rect)>) {
    match shape {
        egui::Shape::Vec(children) => {
            for child in children {
                collect_texts(child, out)
            }
        }
        egui::Shape::Text(t) if !t.galley.text().is_empty() => out.push((
            t.galley.text().into(),
            Rect::from_min_size(t.pos, t.galley.size()),
        )),
        _ => {}
    }
}
#[test]
fn adsr_eg_shared_browser_text_fits_at_rack_and_thumbnail_scales() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "adsr_eg")
        .unwrap();
    for scale in [0.42, 0.85, 1.0] {
        let ctx = egui::Context::default();
        crate::panel_skin::install_fonts(&ctx);
        let mut preview = ModulePreview::new(module);
        let rect = Rect::from_min_size(
            pos2(12.0, 12.0),
            vec2(panel_width("adsr_eg"), PANEL_HEIGHT) * scale,
        );
        for _ in 0..2 {
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        preview.show(ui, rect, &catalog);
                    });
                },
            );
            let mut texts = vec![];
            for shape in output.shapes {
                collect_texts(&shape.shape, &mut texts);
            }
            for required in [
                "NOMINAL SHAPE",
                "ATTACK",
                "DECAY",
                "SUSTAIN",
                "RELEASE",
                "ENV",
                "PUSH",
                "A CV",
                "D CV",
                "S CV",
                "R CV",
            ] {
                assert!(
                    texts.iter().any(|(t, _)| t == required),
                    "missing {required}"
                );
            }
            for (i, (label, a)) in texts.iter().enumerate() {
                assert!(
                    rect.expand(0.5).contains_rect(*a),
                    "{scale}: {label} outside panel"
                );
                for (other, b) in &texts[i + 1..] {
                    assert!(
                        !a.shrink(0.2 * scale).intersects(b.shrink(0.2 * scale)),
                        "{scale}: {label} overlaps {other}"
                    );
                }
            }
            assert!(preview.renderer.held_manual.is_none());
            assert!(preview.renderer.editing.is_none());
        }
    }
}
#[test]
fn adsr_eg_stage_and_gate_lamps_use_only_actual_engine_telemetry() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch = patch();
    let centers = (0..4)
        .map(|i| at(pos2(35.0 + i as f32 * 80.0, 148.0)))
        .chain([at(pos2(267.0, 367.0))])
        .collect::<Vec<_>>();
    let colors = |output: egui::FullOutput| {
        centers
            .iter()
            .map(|center| {
                output
                    .shapes
                    .iter()
                    .filter_map(|s| match &s.shape {
                        egui::Shape::Circle(c)
                            if c.center.distance(*center) < 0.1 && (c.radius - crate::lighting::lamp_radius(2.5)).abs() < 0.1 =>
                        {
                            Some(c.fill)
                        }
                        _ => None,
                    })
                    .next_back()
                    .expect("lamp circle")
            })
            .collect::<Vec<_>>()
    };
    let dark = colors(frame_output(&ctx, &mut rack, &patch, 0.0, vec![]).1);
    let mut lights = ModuleLightState { count:5, ..ModuleLightState::default() };
    for i in 0..5 {
        lights.brightness[i][0] = 1.0;
        rack.module_lights.insert("env".into(), lights);
        let lit = colors(frame_output(&ctx, &mut rack, &patch, 1.0 + i as f64, vec![]).1);
        for j in 0..5 {
            assert_eq!(lit[j] != dark[j], j <= i, "lamp {j}, telemetry {i}");
        }
    }
    rack.module_lights.clear();
    rack.module_controls.insert(
        "env".into(),
        ModuleControlState {
            pressed_mask: 1,
            ..Default::default()
        },
    );
    assert_eq!(
        colors(frame_output(&ctx, &mut rack, &patch, 8.0, vec![]).1),
        dark,
        "physical PUSH state must not fabricate any-gate telemetry"
    );
}
#[test]
fn adsr_eg_push_releases_on_exit_focus_bypass_modal_and_engine_loss() {
    for cause in ["exit", "focus", "bypass", "modal", "engine", "replace"] {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let mut patch = patch();
        rack.set_sequence_states(BTreeMap::new(), true);
        frame(&ctx, &mut rack, &patch, 0.0, vec![]);
        let pos = at(adsr_eg_push_rect().center());
        frame(
            &ctx,
            &mut rack,
            &patch,
            1.0,
            vec![egui::Event::PointerMoved(pos), button(pos, true)],
        );
        let mut events = vec![];
        let mut focus = true;
        let mut enabled = true;
        match cause {
            "exit" => events.push(egui::Event::PointerMoved(pos + vec2(80.0, 0.0))),
            "focus" => {
                focus = false;
                events.push(egui::Event::WindowFocused(false));
            }
            "bypass" => patch.nodes[0].bypassed = true,
            "modal" => enabled = false,
            "engine" => rack.performance_available = false,
            "replace" => patch.nodes[0].kind = "push".into(),
            _ => {}
        }
        let action = frame_state(&ctx, &mut rack, &patch, 1.1, events, focus, enabled).0;
        if cause == "replace" {
            assert!(action.is_none());
        } else {
            assert!(
                matches!(action,Some(RackAction::Command("trigger_module",args)) if args["action"]=="push_release"),
                "{cause}"
            );
        }
        assert!(rack.held_manual.is_none(), "{cause}");
    }
}
