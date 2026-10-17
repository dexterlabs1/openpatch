use super::*;

fn process_patch(slew: f32) -> Patch {
    serde_json::from_value(json!({
        "version": 2,
        "nodes": [{"id":"process","kind":"stock_process","params":{"slew":slew}}]
    }))
    .unwrap()
}

fn process_screen(point: Pos2, zoom: f32) -> Pos2 {
    pos2(24.0, 12.0) + point.to_vec2() * zoom
}

fn process_text(output: &egui::FullOutput) -> Vec<String> {
    fn visit(shape: &egui::Shape, result: &mut Vec<String>) {
        match shape {
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    visit(shape, result);
                }
            }
            egui::Shape::Text(text) => result.push(text.galley.text().to_owned()),
            _ => {}
        }
    }
    let mut result = Vec::new();
    for shape in &output.shapes {
        visit(&shape.shape, &mut result);
    }
    result
}

#[test]
fn process_geometry_keeps_nine_ports_and_controls_independently_hittable() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|module| module["kind"] == "stock_process")
        .unwrap();
    assert_eq!(module["inputs"], json!(["slew_cv", "signal", "gate"]));
    assert_eq!(
        module["outputs"],
        json!(["sh1", "sh2", "track_hold", "hold_track", "slew", "glide"])
    );
    assert_eq!(panel_width("stock_process"), 300.0);
    assert_eq!(title("stock_process"), "STOCK PROCESS");
    let bounds = Rect::from_min_size(Pos2::ZERO, vec2(300.0, PANEL_HEIGHT));
    let ports = (0..3)
        .map(|index| port_position("stock_process", true, index, 3, 300.0))
        .chain((0..6).map(|index| port_position("stock_process", false, index, 6, 300.0)))
        .collect::<Vec<_>>();
    assert_eq!(ports[0], pos2(53.0, 269.0));
    assert_eq!(ports[8], pos2(247.0, 376.0));
    assert!(ports.iter().all(|point| bounds.contains(*point)));
    for (index, point) in ports.iter().enumerate() {
        for other in ports.iter().skip(index + 1) {
            assert!(point.distance(*other) >= 53.0);
        }
    }
    let knob = Rect::from_center_size(STOCK_PROCESS_KNOB, vec2(68.0, 68.0));
    let gate = Rect::from_center_size(STOCK_PROCESS_GATE, vec2(105.0, 44.0));
    let readout = Rect::from_center_size(pos2(76.0, 174.0), vec2(116.0, 25.0));
    let guide = Rect::from_min_max(pos2(15.0, 202.0), pos2(285.0, 238.0));
    assert!(!knob.intersects(gate));
    assert!(!guide.intersects(readout));
    assert!(guide.max.y < 244.0); // input captions remain above their sockets
    assert!(bounds.contains(guide.min) && bounds.contains(guide.max));
    assert!(ports
        .iter()
        .all(|point| point.distance(STOCK_PROCESS_GATE) > 60.0));
    assert_eq!(STOCK_PROCESS_PORT_SCALE, 1.30);
}

#[test]
fn process_exact_time_editor_round_trips_endpoints_and_guards_revision() {
    let patch = process_patch(STOCK_PROCESS_MIN_SLEW);
    let edit = stock_process_edit(&patch.nodes[0], 42);
    assert_eq!(edit.text, "0");
    assert_eq!(edit.display_unit, "ms/V");
    assert_eq!(edit.min, 0.0);
    assert_eq!(edit.max, 10_000.0);
    assert_eq!(stock_process_ms_label(STOCK_PROCESS_MIN_SLEW), "0 ms/V");
    for (text, valid) in [
        ("0", true),
        ("1", false),
        ("0.5", false),
        ("1.01", true),
        ("5 ms/V", true),
        ("10000", true),
        ("-1", false),
        ("10001", false),
        ("nan", false),
    ] {
        let mut draft = edit.clone();
        draft.text = text.into();
        assert_eq!(parsed_parameter_result(&draft).is_ok(), valid, "{text}");
    }
    let mut boundary = edit.clone();
    boundary.text = "1 ms/V".into();
    assert!(parsed_parameter_result(&boundary)
        .unwrap_err()
        .contains("0 for instant"));
    assert_eq!(parameter_edit_raw_value(&edit, 0.0), STOCK_PROCESS_MIN_SLEW);
    assert!((parameter_edit_raw_value(&edit, 1000.0) - 0.0).abs() < 1e-6);
    let RackAction::Command("set_parameter", args) =
        parameter_edit_action(&edit, STOCK_PROCESS_MAX_SLEW)
    else {
        panic!("edit command")
    };
    assert_eq!(args["expected_revision"], 42);
    assert_eq!(args["parameter"], "slew");
}

#[test]
fn process_gate_uses_live_manual_action_and_releases_after_pointer_up_or_blur() {
    let ctx = egui::Context::default();
    crate::panel_skin::install_fonts(&ctx);
    let mut rack = Rack::default();
    rack.set_sequence_states(BTreeMap::new(), true);
    let patch = process_patch(STOCK_PROCESS_MIN_SLEW);
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 7);
    let point = process_screen(STOCK_PROCESS_GATE, 1.0);
    let pressed = frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        0.1,
        vec![egui::Event::PointerMoved(point), button(point, true)],
        true,
        true,
        7,
    )
    .0;
    assert!(
        matches!(pressed, Some(RackAction::Command("trigger_module", ref args))
        if args["id"] == "process" && args["action"] == "gate_press")
    );
    assert!(rack.held_manual.is_some());
    let released = frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        0.2,
        vec![button(point, false)],
        true,
        true,
        7,
    )
    .0;
    assert!(
        matches!(released, Some(RackAction::Command("trigger_module", ref args))
        if args["action"] == "gate_release")
    );
    assert!(rack.held_manual.is_none());
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        0.3,
        vec![egui::Event::PointerMoved(point), button(point, true)],
        true,
        true,
        7,
    );
    let blur = frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        0.4,
        vec![egui::Event::WindowFocused(false)],
        false,
        true,
        7,
    )
    .0;
    assert!(
        matches!(blur, Some(RackAction::Command("trigger_module", ref args))
        if args["action"] == "gate_release")
    );
}

#[test]
fn process_knob_center_click_does_not_jump_and_drag_uses_revision() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch = process_patch(0.0);
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 31);
    let center = process_screen(STOCK_PROCESS_KNOB, 1.0);
    let press = frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        0.1,
        vec![egui::Event::PointerMoved(center), button(center, true)],
        true,
        true,
        31,
    )
    .0;
    assert!(press.is_none());
    let moved = center - vec2(0.0, 15.0);
    let drag = frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        0.2,
        vec![egui::Event::PointerMoved(moved)],
        true,
        true,
        31,
    )
    .0;
    let Some(RackAction::Command("set_parameter", args)) = drag else {
        panic!("slew drag")
    };
    assert_eq!(args["parameter"], "slew");
    assert_eq!(args["expected_revision"], 31);
    assert!(args["value"].as_f64().unwrap() > 0.0);
    assert!(args["history_group"].as_str().is_some());
}

#[test]
fn process_bypass_releases_held_button_without_editing_slew() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    rack.set_sequence_states(BTreeMap::new(), true);
    let patch = process_patch(0.0);
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 8);
    let point = process_screen(STOCK_PROCESS_GATE, 1.0);
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        0.1,
        vec![egui::Event::PointerMoved(point), button(point, true)],
        true,
        true,
        8,
    );
    assert!(rack.held_manual.is_some());
    let mut bypassed = patch.clone();
    bypassed.nodes[0].bypassed = true;
    let action = frame_revision_state(&ctx, &mut rack, &bypassed, 0.2, vec![], true, true, 9).0;
    assert!(
        matches!(action, Some(RackAction::Command("trigger_module", ref args))
        if args["action"] == "gate_release")
    );
    assert_eq!(bypassed.nodes[0].params["slew"], 0.0);
    assert!(rack.held_manual.is_none());
}

#[test]
fn process_modal_and_preview_never_press_or_rewrite_controls() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    rack.set_sequence_states(BTreeMap::new(), true);
    rack.set_shortcuts_enabled(false);
    let patch = process_patch(0.0);
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 17);
    let gate = process_screen(STOCK_PROCESS_GATE, 1.0);
    let action = frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        0.1,
        vec![egui::Event::PointerMoved(gate), button(gate, true)],
        true,
        true,
        17,
    )
    .0;
    assert!(action.is_none());
    assert!(rack.held_manual.is_none());
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|module| module["kind"] == "stock_process")
        .unwrap();
    let mut preview = ModulePreview::new(module);
    let preview_ctx = egui::Context::default();
    crate::panel_skin::install_fonts(&preview_ctx);
    let output = preview_ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(900.0, 600.0))),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                preview.show(
                    ui,
                    Rect::from_min_size(pos2(20.0, 20.0), vec2(300.0, PANEL_HEIGHT)),
                    &catalog,
                );
            });
        },
    );
    let labels = process_text(&output);
    assert!(labels.iter().any(|text| text == "S/H EDGE"));
    assert!(labels.iter().any(|text| text == "TRACK / HOLD"));
    assert!(labels.iter().any(|text| text == "SLEW / GLIDE"));
    assert!(labels.iter().any(|text| text == "NOW → 1   /   PREV → 2"));
    assert!(labels
        .iter()
        .any(|text| text == "HIGH: TH HOLDS  /  HT TRACKS"));
    assert!(labels
        .iter()
        .any(|text| text == "LOW: TH TRACKS  /  HT HOLDS"));
    assert!(preview.renderer.held_manual.is_none());
    assert!(preview.renderer.editing.is_none());
}

#[test]
fn process_reduced_zoom_keeps_gate_and_slew_separate() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    rack.set_zoom(0.35);
    rack.set_sequence_states(BTreeMap::new(), true);
    let patch = process_patch(STOCK_PROCESS_MIN_SLEW);
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 27);
    let gate = process_screen(STOCK_PROCESS_GATE, rack.zoom());
    let action = frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        0.1,
        vec![egui::Event::PointerMoved(gate), button(gate, true)],
        true,
        true,
        27,
    )
    .0;
    assert!(
        matches!(action, Some(RackAction::Command("trigger_module", ref args))
        if args["action"] == "gate_press")
    );
    assert!(patch.nodes[0].params["slew"] == STOCK_PROCESS_MIN_SLEW);
}

#[test]
fn process_positive_slew_cv_halves_rate_and_doubles_time_in_real_engine() {
    let render = |cv: f32| {
        let patch: Patch = serde_json::from_value(json!({"version":2,"sample_rate":48000,
            "nodes":[{"id":"signal","kind":"constant","params":{"value":1.0}},
                {"id":"cv","kind":"constant","params":{"value":cv}},
                {"id":"process","kind":"stock_process","params":{"slew":0.0}},
                {"id":"out","kind":"output"}],"cables":[
                {"from":"signal","output":"out","to":"process","input":"signal"},
                {"from":"cv","output":"out","to":"process","input":"slew_cv"},
                {"from":"process","output":"slew","to":"out","input":"signal"}],"output":"out"}))
        .unwrap();
        let mut engine = patch.compile().unwrap();
        let mut out = [0.0; 256];
        engine.process_block(&mut out);
        out
    };
    let base = render(0.0);
    let positive = render(1.0);
    assert!(base[255] > 0.0 && base[255] < 0.1);
    assert!((positive[255] - base[255] * 0.5).abs() < 1e-7);
}

#[test]
fn process_configured_guide_distinguishes_instant_and_linear_slew_settings() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    for (raw, label) in [(STOCK_PROCESS_MIN_SLEW, "INSTANT"), (0.0, "LIMITED")] {
        let patch = process_patch(raw);
        let output = frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            raw as f64 + 20.0,
            vec![],
            true,
            true,
            27,
        )
        .1;
        assert!(process_text(&output).iter().any(|text| text == label));
        assert!(
            rack.module_lights.is_empty(),
            "configured guide cannot fabricate button telemetry"
        );
    }
}
