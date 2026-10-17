use super::*;

fn stock_patch(time: f32) -> Patch {
    serde_json::from_value(json!({"version":2,"nodes":[
        {"id":"stock","kind":"stock_scope","params":{"time":time}}
    ]})).unwrap()
}

fn stock_texts(output: &egui::FullOutput) -> Vec<String> {
    fn visit(shape: &egui::Shape, texts: &mut Vec<String>) {
        match shape {
            egui::Shape::Vec(children) => {
                for shape in children { visit(shape, texts); }
            }
            egui::Shape::Text(text) => texts.push(text.galley.text().to_owned()),
            _ => {}
        }
    }
    let mut texts = Vec::new();
    for shape in &output.shapes { visit(&shape.shape, &mut texts); }
    texts
}

#[test]
fn stock_scope_eight_source_controls_view_and_five_ports_fit_original_face_at_reduced_zoom() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"].as_array().unwrap().iter()
        .find(|module| module["kind"] == "stock_scope").unwrap();
    let params = module["params"].as_object().unwrap();
    assert_eq!(params.len(), 8);
    assert!(!params.contains_key("auto_range"));
    assert_eq!(params["trigger_disabled"]["default"], 1);
    assert_eq!(params["time"]["default"], 1);
    assert_eq!(panel_width("stock_scope"), 630.0);
    assert_eq!(title("stock_scope"), "STOCK SCOPE");
    assert_eq!(module["inputs"], json!(["x","y","trigger"]));
    assert_eq!(module["outputs"], json!(["x","y"]));
    let centers = [(true,0,pos2(70.0,360.0)),(true,1,pos2(170.0,360.0)),
        (true,2,pos2(285.0,360.0)),(false,0,pos2(460.0,360.0)),
        (false,1,pos2(560.0,360.0))];
    for zoom in [0.35, 0.5, 1.0] {
        let panel = Rect::from_min_size(Pos2::ZERO, vec2(630.0, PANEL_HEIGHT) * zoom);
        let controls = (0..9).map(|index| {
            let local = scope_control_rect(index);
            Rect::from_min_max(
                pos2(local.min.x * zoom, local.min.y * zoom),
                pos2(local.max.x * zoom, local.max.y * zoom),
            )
        }).collect::<Vec<_>>();
        for (index, rect) in controls.iter().enumerate() {
            assert!(panel.contains_rect(*rect), "{zoom} control {index}");
            for other in controls.iter().skip(index + 1) {
                assert!(!rect.intersects(*other), "{zoom} independent controls");
            }
        }
        for (input, index, local) in centers {
            let actual = port_position("stock_scope", input, index, if input {3} else {2}, 630.0);
            assert_eq!(actual, local);
            let target = jack_target_rect("stock_scope", pos2(local.x * zoom, local.y * zoom),
                zoom, 18.0, port_material_scale("stock_scope"));
            assert!(panel.contains(target.center()));
            for control in &controls {
                assert!(!target.intersects(*control), "{zoom} {input} {index} jack/control overlap");
            }
        }
    }
}

#[test]
fn stock_scope_time_entry_is_milliseconds_but_commits_raw_log2_with_opening_revision() {
    for millis in [5.0, 500.0, 50_000.0] {
        let raw = stock_scope_time_raw(millis);
        assert!((stock_scope_nominal_ms(raw) - millis).abs() < millis * 0.000001 + 0.00001);
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let patch = stock_patch(1.0);
        frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 17);
        let target = pos2(24.0, 12.0) + scope_control_rect(2).center().to_vec2();
        frame_revision_state(&ctx, &mut rack, &patch, 0.1,
            vec![egui::Event::PointerMoved(target), button(target, true)], true, true, 17);
        frame_revision_state(&ctx, &mut rack, &patch, 0.2,
            vec![button(target, false)], true, true, 17);
        let action = frame_revision_state(&ctx, &mut rack, &patch, 0.3,
            vec![egui::Event::Text(format!("{millis}")), egui::Event::Key {
                key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false,
                modifiers: egui::Modifiers::NONE,
            }], true, true, 17).0;
        if millis == 500.0 {
            assert!(action.is_none(), "default nominal 500 ms must be a no-op");
        } else {
            assert!(matches!(action, Some(RackAction::Command("set_parameter", ref args))
                if args["parameter"] == "time" && args["expected_revision"] == 17
                && (args["value"].as_f64().unwrap() - f64::from(raw)).abs() < 0.000001),
                "{millis} ms must store raw log2 seconds: {action:?}");
        }
    }
}

#[test]
fn stock_scope_native_view_and_manual_range_edit_leave_source_history_separate() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    rack.set_scope_identities(BTreeMap::from([("stock".into(), 91)]));
    let patch = stock_patch(1.0);
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 7);
    assert!(rack.stock_scope_views["stock"].automatic);
    let target = pos2(24.0, 12.0) + scope_control_rect(8).center().to_vec2();
    let press = frame_revision_state(&ctx, &mut rack, &patch, 0.1,
        vec![egui::Event::PointerMoved(target), button(target, true)], true, true, 7).0;
    let release = frame_revision_state(&ctx, &mut rack, &patch, 0.2,
        vec![button(target, false)], true, true, 7).0;
    assert!(press.is_none() && release.is_none());
    assert!(!rack.stock_scope_views["stock"].automatic);
    assert!(!patch.nodes[0].params.contains_key("auto_range"));

    // Switching back to AUTO is still view-only; editing saved manual offset
    // then switches locally to MANUAL and emits only its source parameter.
    frame_revision_state(&ctx, &mut rack, &patch, 0.3,
        vec![egui::Event::PointerMoved(target), button(target, true)], true, true, 7);
    let action = frame_revision_state(&ctx, &mut rack, &patch, 0.4,
        vec![button(target, false)], true, true, 7).0;
    assert!(action.is_none());
    assert!(rack.stock_scope_views["stock"].automatic);
    let offset = pos2(24.0, 12.0) + scope_control_rect(4).center().to_vec2();
    frame_revision_state(&ctx, &mut rack, &patch, 0.5,
        vec![egui::Event::PointerMoved(offset), button(offset, true)], true, true, 7);
    let action = frame_revision_state(&ctx, &mut rack, &patch, 0.6,
        vec![egui::Event::PointerMoved(offset + vec2(40.0, 0.0))], true, true, 7).0;
    assert!(matches!(action, Some(RackAction::Command("set_parameter", ref args))
        if args["parameter"] == "x_offset" && args["expected_revision"] == 7
        && args["history_group"].is_string()
        && args["value"].as_f64().unwrap() > 0.0));
    assert!(!rack.stock_scope_views["stock"].automatic);
    rack.set_scope_identities(BTreeMap::from([("stock".into(), 92)]));
    assert!(!rack.stock_scope_views.contains_key("stock"));
    frame_revision_state(&ctx, &mut rack, &patch, 0.7,
        vec![button(offset + vec2(40.0, 0.0), false)], true, true, 7);
    assert!(rack.stock_scope_views["stock"].automatic);
    assert_eq!(rack.stock_scope_views["stock"].token, 92);
}

#[test]
fn stock_scope_real_weak_capture_shares_waveform_measurements_with_native_manual_view() {
    let patch: Patch = serde_json::from_value(json!({"version":2,
        "nodes":[{"id":"weak","kind":"constant","params":{"value":0.0001}},
            {"id":"stock","kind":"stock_scope","params":{"time":7}}],
        "cables":[{"from":"weak","to":"stock","input":"x"}],"output":"weak"
    })).unwrap();
    let mut engine = patch.compile().unwrap();
    engine.process_block(&mut [0.0; 2048]);
    let handle = engine.resolve_module_scope("stock").unwrap();
    let capture = Arc::new(engine.module_scope(handle).unwrap().clone());
    assert!(capture.stock.is_some());
    let before = capture.measurements();
    assert!(before.x.is_some(), "actual source must have completed weak bins");

    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    rack.set_scope_identities(BTreeMap::from([("stock".into(), 73)]));
    rack.set_scope_states(BTreeMap::from([("stock".into(), capture.clone())]));
    let automatic = frame_output(&ctx, &mut rack, &patch, 0.0, vec![]).1;
    assert!(stock_texts(&automatic).iter().any(|text|
        text.starts_with("AUTO · NATIVE VIEW X") && text.contains("µV/screen")));
    let target = pos2(24.0, 12.0) + scope_control_rect(8).center().to_vec2();
    frame(&ctx, &mut rack, &patch, 0.1,
        vec![egui::Event::PointerMoved(target), button(target, true)]);
    assert!(frame(&ctx, &mut rack, &patch, 0.2,
        vec![button(target, false)]).is_none());
    let manual = frame_output(&ctx, &mut rack, &patch, 0.3, vec![]).1;
    assert!(stock_texts(&manual).iter().any(|text|
        text.starts_with("MANUAL · SOURCE RANGE X 20 V/screen")));
    assert_eq!(capture.measurements(), before);
    assert_eq!(Arc::strong_count(&capture), 2);
}

#[test]
fn stock_scope_lamps_use_only_published_levels_and_hold_that_display_through_bypass() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch = stock_patch(1.0);
    let lens = |output: &egui::FullOutput, index: usize| {
        let center = pos2(24.0 + 554.0, 12.0 + [225.0, 240.0][index]);
        output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Circle(circle)
                if circle.center.distance(center) < 0.01
                    && (circle.radius - crate::lighting::lamp_radius(2.3)).abs() < 0.01 =>
            {
                Some(circle.fill)
            }
            _ => None,
        })
    };
    let absent = frame_output(&ctx, &mut rack, &patch, 0.0, vec![]).1;
    assert!(lens(&absent, 0).is_none() && lens(&absent, 1).is_none());

    let mut levels = ModuleLightState { count: 2, ..Default::default() };
    levels.brightness[0][0] = 1.0;
    levels.brightness[1][0] = 0.0;
    rack.module_lights.insert("stock".into(), levels);
    let live = frame_output(&ctx, &mut rack, &patch, 0.1, vec![]).1;
    let xy_lit = lens(&live, 0).unwrap();
    let trigger_dark = lens(&live, 1).unwrap();
    assert!(xy_lit.r() > trigger_dark.r());
    assert!(xy_lit.g() > trigger_dark.g());

    let mut bypassed = patch.clone();
    bypassed.nodes[0].bypassed = true;
    let held = frame_output(&ctx, &mut rack, &bypassed, 0.2, vec![]).1;
    assert_eq!(lens(&held, 0), Some(xy_lit));
    assert_eq!(lens(&held, 1), Some(trigger_dark));
}

#[test]
fn stock_scope_browser_uses_same_control_geometry_and_neutral_unavailable_display() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"].as_array().unwrap().iter()
        .find(|module| module["kind"] == "stock_scope").unwrap();
    let mut preview = ModulePreview::new(module);
    let ctx = egui::Context::default();
    crate::panel_skin::install_fonts(&ctx);
    let output = ctx.run(egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(950.0, 700.0))),
        ..Default::default()
    }, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            preview.show(ui, Rect::from_min_size(pos2(20.0,20.0), vec2(630.0,PANEL_HEIGHT)), &catalog);
        });
    });
    let labels = stock_texts(&output);
    assert!(labels.iter().any(|text| text.contains("NO ENGINE CAPTURE")));
    assert!(labels.iter().any(|text| text == "VIEW · NATIVE"));
    assert!(labels.iter().any(|text| text == "AUTO"));
    assert!(labels.iter().any(|text| text == "ENABLED" || text == "DISABLED"));
    assert!(preview.renderer.scope_states.is_empty());
    assert!(preview.renderer.module_lights.is_empty());
    assert!(preview.renderer.stock_scope_views.is_empty());
    assert_eq!(preview.size(), vec2(630.0, PANEL_HEIGHT));
}
