use super::*;

fn patch(automatic: bool) -> Patch {
    serde_json::from_value(json!({"version":2,"nodes":[
        {"id":"scope","kind":"scope","params":{"auto_range":if automatic {1}else{0}}}
    ]}))
    .unwrap()
}
fn capture(channels: u8, xy: bool, amplitude: f32) -> openpatch_core::ScopeSnapshot {
    let mut state = openpatch_core::ScopeSnapshot {
        x_channels: channels,
        y_channels: channels,
        xy,
        ..Default::default()
    };
    for voice in 0..usize::from(channels.min(16)) {
        for bin in 0..256 {
            let phase = bin as f32 / 256.0 * std::f32::consts::TAU;
            let x = amplitude * phase.sin() * (voice + 1) as f32;
            let y = amplitude * phase.cos() * (voice + 1) as f32;
            state.x[voice][bin] = openpatch_core::ScopePoint { min: x, max: x };
            state.y[voice][bin] = openpatch_core::ScopePoint { min: y, max: y };
        }
    }
    state
}
fn text_in(shape: &egui::Shape, result: &mut Vec<String>) {
    match shape {
        egui::Shape::Text(text) => result.push(text.galley.text().into()),
        egui::Shape::Vec(children) => {
            for child in children {
                text_in(child, result);
            }
        }
        _ => {}
    }
}
fn texts(output: &egui::FullOutput) -> Vec<String> {
    let mut result = vec![];
    for shape in &output.shapes {
        text_in(&shape.shape, &mut result);
    }
    result
}
fn trace_points(output: &egui::FullOutput, voice: usize) -> Vec<Pos2> {
    let color = scope_voice_color(Color32::from_rgb(86, 215, 222), voice);
    output
        .shapes
        .iter()
        .flat_map(|shape| match &shape.shape {
            egui::Shape::LineSegment { points, stroke } if stroke.color == color => points.to_vec(),
            _ => vec![],
        })
        .collect()
}

#[test]
fn tiny_raw_wave_is_visible_bounded_and_truthfully_calibrated_without_mutating_capture() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let snapshot = Arc::new(capture(1, false, 0.0001));
    let before = snapshot.measurements();
    rack.set_scope_states(BTreeMap::from([("scope".into(), snapshot.clone())]));
    let automatic = frame_output(&ctx, &mut rack, &patch(true), 0.0, vec![]).1;
    let points = trace_points(&automatic, 0);
    assert!(points.len() > 500);
    let plot = Rect::from_min_max(pos2(64.0, 113.0), pos2(614.0, 225.0));
    assert!(points
        .iter()
        .all(|point| point.is_finite() && plot.expand(0.01).contains(*point)));
    let height = |points: &[Pos2]| {
        points.iter().map(|p| p.y).fold(f32::NEG_INFINITY, f32::max)
            - points.iter().map(|p| p.y).fold(f32::INFINITY, f32::min)
    };
    assert!(
        height(&points) > plot.height() * 0.35,
        "100 µV signal must be inspectable"
    );
    assert!(texts(&automatic)
        .iter()
        .any(|text| text.starts_with("AUTO X 500 µV/screen offset 0 V")));
    assert!(texts(&automatic)
        .iter()
        .any(|text| text.starts_with("X  MIN -0.0001 V") && text.contains("P-P 0.0002 V")));
    let manual = frame_output(&ctx, &mut rack, &patch(false), 1.0, vec![]).1;
    assert!(
        height(&trace_points(&manual, 0)) < 0.01,
        "manual 20V range is restored exactly"
    );
    assert!(texts(&manual)
        .iter()
        .any(|text| text.starts_with("MANUAL X 20 V/screen offset 0 V")));
    assert_eq!(snapshot.measurements(), before);
    assert_eq!(Arc::strong_count(&snapshot), 2);
}

#[test]
fn automatic_range_handles_zero_dc_all_voices_invalid_bins_and_stable_hysteresis() {
    let empty = openpatch_core::ScopeSnapshot::default();
    assert_eq!(
        scope_display_ranges(&empty, true, None),
        [ScopeDisplayRange {
            span: 20.0,
            offset: 0.0
        }; 2]
    );
    let mut state = capture(1, false, 0.0);
    assert_eq!(
        scope_display_ranges(&state, true, None)[0],
        ScopeDisplayRange {
            span: 20.0,
            offset: 0.0
        }
    );
    for dc in [-0.0001_f32, 0.0001, 5.0] {
        state.x[0].fill(openpatch_core::ScopePoint { min: dc, max: dc });
        let range = scope_display_ranges(&state, true, None)[0];
        assert!(scope_display_fraction(dc, range).abs() < 0.5);
        assert!(
            scope_display_fraction(0.0, range).abs() < 0.5,
            "DC must not erase the real zero"
        );
        assert_ne!(
            scope_display_fraction(dc, range),
            scope_display_fraction(0.0, range)
        );
    }
    for dc in [f32::from_bits(1), f32::MIN_POSITIVE, f32::MAX] {
        state.x[0].fill(openpatch_core::ScopePoint { min: dc, max: dc });
        let range = scope_display_ranges(&state, true, None)[0];
        assert!(range.span.is_finite() && range.span > 0.0 && range.offset.is_finite());
        assert!(scope_display_fraction(dc, range).is_finite());
        assert!(scope_display_fraction(dc, range).abs() < 0.5);
    }
    state.x_scale = 7;
    state.x_offset = 2.0;
    let manual = scope_display_ranges(&state, false, None)[0];
    assert_eq!(
        manual,
        ScopeDisplayRange {
            span: 0.15625,
            offset: 2.0
        }
    );
    scope_display_ranges(&state, true, None);
    assert_eq!((state.x_scale, state.x_offset), (7, 2.0));
    let mut state = capture(1, false, 0.0001);
    let expected = scope_display_ranges(&state, true, None);
    state.x[15].fill(openpatch_core::ScopePoint {
        min: -f32::MAX,
        max: f32::MAX,
    });
    state.x[0][1] = openpatch_core::ScopePoint {
        min: f32::NAN,
        max: f32::INFINITY,
    };
    assert_eq!(scope_display_ranges(&state, true, None), expected);
    let almost = capture(1, false, 0.000105);
    assert_eq!(
        scope_display_ranges(&almost, true, Some(expected)),
        expected
    );
    let smaller = capture(1, false, 0.00001);
    assert!(scope_display_ranges(&smaller, true, Some(expected))[0].span < expected[0].span);
    let full = capture(16, false, 1.0);
    let ranges = scope_display_ranges(&full, true, None);
    assert!(ranges[0].span >= 32.0);
    for (axis, range) in [(&full.x, ranges[0]), (&full.y, ranges[1])] {
        for point in axis.iter().flatten() {
            assert!(scope_display_fraction(point.min, range).abs() < 0.5);
            assert!(scope_display_fraction(point.max, range).abs() < 0.5);
        }
    }
}

#[test]
fn xy_tiny_traces_fit_both_axes_and_unpaired_voices_cannot_poison_the_range() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let mut state = capture(16, true, 0.0001);
    let ranges = scope_display_ranges(&state, true, None);
    assert!(scope_range_label(true, ranges).contains("mV/screen"));
    rack.set_scope_states(BTreeMap::from([("scope".into(), Arc::new(state.clone()))]));
    let output = frame_output(&ctx, &mut rack, &patch(true), 0.0, vec![]).1;
    let plot = Rect::from_min_max(pos2(64.0, 113.0), pos2(614.0, 225.0));
    for voice in 0..16 {
        let points = trace_points(&output, voice);
        assert!(points.len() >= 500);
        assert!(points
            .iter()
            .all(|point| plot.expand(0.01).contains(*point)));
    }
    state.y_channels = 1;
    let paired = scope_display_ranges(&state, true, None);
    assert!(paired[0].span < ranges[0].span);
    state.x[15].fill(openpatch_core::ScopePoint {
        min: -f32::MAX,
        max: f32::MAX,
    });
    assert_eq!(scope_display_ranges(&state, true, None), paired);
    state.x[0][1] = openpatch_core::ScopePoint {
        min: f32::MAX,
        max: f32::MAX,
    };
    state.y[0][1] = openpatch_core::ScopePoint {
        min: f32::NAN,
        max: f32::NAN,
    };
    assert_eq!(
        scope_display_ranges(&state, true, None),
        paired,
        "unrenderable XY pairs cannot set the range"
    );
    assert_eq!(scope_calibrated_voltage(0.000001), "1 µV");
    assert_eq!(scope_calibrated_voltage(0.001), "1 mV");
    assert_eq!(scope_calibrated_voltage(1.0), "1 V");
}

#[test]
fn manual_offset_gesture_atomically_disables_auto_and_undo_restores_both_settings() {
    let directory = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(directory.path()).unwrap();
    let mut patch = patch(true);
    patch.nodes[0].params.insert("x_offset".into(), 0.0);
    session.replace_patch(patch.clone(), None).unwrap();
    let revision = session.revision();
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, revision);
    let target = pos2(24.0, 12.0) + scope_control_rect(4).center().to_vec2();
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.0,
        vec![egui::Event::PointerMoved(target), button(target, true)],
        true,
        true,
        revision,
    );
    let action = frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.1,
        vec![egui::Event::PointerMoved(target + vec2(40.0, 0.0))],
        true,
        true,
        revision,
    )
    .0;
    let Some(RackAction::Command("apply_edits", args)) = action else {
        panic!("manual takeover must be atomic");
    };
    assert_eq!(args["expected_revision"], revision);
    assert_eq!(args["edits"].as_array().unwrap().len(), 2);
    assert!(args["edits"][0]["arguments"].get("history_group").is_none());
    let group = args["history_group"]
        .as_str()
        .expect("takeover retains drag gesture")
        .to_owned();
    assert_eq!(
        args["history_parameter"],
        json!({"id":"scope","parameter":"x_offset"})
    );
    session.execute("apply_edits", args).unwrap();
    assert_eq!(session.revision(), revision + 1);
    assert_eq!(session.patch().nodes[0].params["auto_range"], 0.0);
    assert!(session.patch().nodes[0].params["x_offset"] > 0.0);
    for (time, delta) in [(1.2, 70.0), (1.3, 100.0), (1.4, 130.0)] {
        let live_patch = session.patch().clone();
        let revision = session.revision();
        let action = frame_revision_state(
            &ctx,
            &mut rack,
            &live_patch,
            time,
            vec![egui::Event::PointerMoved(target + vec2(delta, 0.0))],
            true,
            true,
            revision,
        )
        .0;
        let Some(RackAction::Command("set_parameter", args)) = action else {
            panic!("continued drag remains a parameter gesture");
        };
        assert_eq!(args["history_group"], group);
        session.execute("set_parameter", args).unwrap();
    }
    let final_patch = session.patch().reference_view();
    assert!(session.patch().nodes[0].params["x_offset"] > 0.1);
    session.execute("undo", json!({})).unwrap();
    assert_eq!(session.patch().reference_view(), patch.reference_view());
    session.execute("redo", json!({})).unwrap();
    assert_eq!(session.patch().reference_view(), final_patch);
}

#[test]
fn auto_toggle_is_a_native_revision_guarded_parameter_and_preset_setting() {
    let directory = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(directory.path()).unwrap();
    let patch = patch(true);
    session.replace_patch(patch.clone(), None).unwrap();
    let revision = session.revision();
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, revision);
    let target = pos2(24.0, 12.0) + scope_control_rect(8).center().to_vec2();
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.0,
        vec![egui::Event::PointerMoved(target), button(target, true)],
        true,
        true,
        revision,
    );
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.1,
        vec![button(target, false)],
        true,
        true,
        revision,
    );
    let output = frame_revision_state(&ctx, &mut rack, &patch, 1.2, vec![], true, true, revision).1;
    fn find_off(shape: &egui::Shape, below: f32) -> Option<Pos2> {
        match shape {
            egui::Shape::Text(text) if text.galley.text() == "OFF" && text.pos.y > below => {
                Some(text.pos + text.galley.size() * 0.5)
            }
            egui::Shape::Vec(children) => children.iter().find_map(|shape| find_off(shape, below)),
            _ => None,
        }
    }
    let off = output
        .shapes
        .iter()
        .find_map(|shape| find_off(&shape.shape, target.y + 10.0))
        .expect("native auto-range menu");
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.3,
        vec![egui::Event::PointerMoved(off), button(off, true)],
        true,
        true,
        revision,
    );
    let action = frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.4,
        vec![button(off, false)],
        true,
        true,
        revision,
    )
    .0;
    let Some(RackAction::Command("set_parameter", args)) = action else {
        panic!("native toggle edit");
    };
    assert_eq!(args["parameter"], "auto_range");
    assert_eq!(args["expected_revision"], revision);
    session.execute("set_parameter", args).unwrap();
    let captured = session
        .execute("capture_module_preset", json!({"id":"scope"}))
        .unwrap()["preset"]
        .clone();
    assert_eq!(captured["params"]["auto_range"].as_f64(), Some(0.0));
    session
        .execute(
            "set_parameter",
            json!({"id":"scope","parameter":"auto_range","value":1}),
        )
        .unwrap();
    session
        .execute(
            "apply_module_preset",
            json!({"id":"scope","preset":captured}),
        )
        .unwrap();
    assert_eq!(session.patch().nodes[0].params["auto_range"], 0.0);
    session.execute("undo", json!({})).unwrap();
    assert_eq!(session.patch().nodes[0].params["auto_range"], 1.0);
}
