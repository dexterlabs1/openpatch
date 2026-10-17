use super::*;

#[test]
fn scope_controls_and_jacks_have_separated_bounds() {
    let panel = Rect::from_min_size(Pos2::ZERO, vec2(panel_width("scope"), PANEL_HEIGHT));
    assert_eq!(panel.width(), 630.0);
    let jacks: Vec<_> = [(true, 3), (false, 2)]
        .into_iter()
        .flat_map(|(input, count)| {
            (0..count).map(move |index| {
                Rect::from_center_size(
                    port_position("scope", input, index, count, 630.0),
                    vec2(36.0, 36.0),
                )
            })
        })
        .collect();
    for (index, (_, label, _)) in SCOPE_CONTROLS.iter().enumerate() {
        let control = scope_control_rect(index);
        assert!(panel.contains_rect(control));
        let label_bounds = Rect::from_min_size(
            control.min - vec2(0.0, 14.0),
            vec2(label.len() as f32 * 5.0, 12.0),
        );
        assert!(panel.contains_rect(label_bounds));
        for jack in &jacks {
            assert!(!jack.intersects(control));
            assert!(!jack.intersects(label_bounds));
        }
        for other in index + 1..8 {
            assert!(!control.intersects(scope_control_rect(other)));
        }
    }
    for (index, jack) in jacks.iter().enumerate() {
        assert!(panel.contains_rect(*jack));
        for other in &jacks[index + 1..] {
            assert!(!jack.intersects(*other));
        }
    }
    let mut colors = BTreeSet::new();
    for voice in 0..16 {
        assert!(colors.insert(scope_voice_color(Color32::from_rgb(86, 215, 222), voice).to_array()));
    }
}

#[test]
fn scope_each_input_accepts_real_cable_drag() {
    for port in ["x", "y", "trigger"] {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let patch:Patch=serde_json::from_value(json!({"version":2,"nodes":[{"id":"scope","kind":"scope"},{"id":"source","kind":"constant"}]})).unwrap();
        frame(&ctx, &mut rack, &patch, 0.0, vec![]);
        let source = fixture_jack(&rack, &patch, "source", "out", false);
        let input = fixture_jack(&rack, &patch, "scope", port, true);
        let action = drag(&ctx, &mut rack, &patch, 1.0, source, input);
        assert!(
            matches!(action,Some(RackAction::Command("connect",ref args)) if args["to"]=="scope" && args["input"]==port)
        );
    }
}

#[test]
fn scope_numeric_drag_emits_parameter_with_gesture_history() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch: Patch =
        serde_json::from_value(json!({"version":2,"nodes":[{"id":"scope","kind":"scope","params":{"auto_range":0}}]}))
            .unwrap();
    frame(&ctx, &mut rack, &patch, 0.0, vec![]);
    let target = pos2(24.0, 12.0) + scope_control_rect(4).center().to_vec2();
    frame(
        &ctx,
        &mut rack,
        &patch,
        1.0,
        vec![egui::Event::PointerMoved(target), button(target, true)],
    );
    let action = frame(
        &ctx,
        &mut rack,
        &patch,
        1.1,
        vec![egui::Event::PointerMoved(target + vec2(40.0, 0.0))],
    );
    assert!(
        matches!(action,Some(RackAction::Command("set_parameter",ref args)) if args["parameter"]=="x_offset" && args["value"].as_f64().unwrap()>0.0 && args["history_group"].is_string())
    );
}

#[test]
fn scope_unavailable_and_sixteen_voice_capture_are_distinct() {
    fn texts(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(t) => out.push(t.galley.text().into()),
            egui::Shape::Vec(v) => {
                for s in v {
                    texts(s, out);
                }
            }
            _ => {}
        }
    }
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch: Patch =
        serde_json::from_value(json!({"version":2,"nodes":[{"id":"scope","kind":"scope"}]}))
            .unwrap();
    let output = frame_output(&ctx, &mut rack, &patch, 0.0, vec![]).1;
    let mut labels = vec![];
    for shape in output.shapes {
        texts(&shape.shape, &mut labels);
    }
    assert!(labels.iter().any(|t| t.contains("NO ENGINE CAPTURE")));
    let mut snapshot = openpatch_core::ScopeSnapshot {
        x_channels: 16,
        y_channels: 16,
        ..Default::default()
    };
    for voice in 0..16 {
        for bin in 0..256 {
            snapshot.x[voice][bin] = openpatch_core::ScopePoint {
                min: -1.0,
                max: 1.0,
            };
            snapshot.y[voice][bin] = openpatch_core::ScopePoint {
                min: -2.0,
                max: 2.0,
            };
        }
    }
    let snapshot = Arc::new(snapshot);
    rack.set_scope_states(BTreeMap::from([("scope".into(), snapshot.clone())]));
    let output = frame_output(&ctx, &mut rack, &patch, 1.0, vec![]).1;
    labels.clear();
    let mut colors = BTreeSet::new();
    for shape in output.shapes {
        texts(&shape.shape, &mut labels);
        if let egui::Shape::LineSegment { stroke, .. } = shape.shape {
            colors.insert(stroke.color.to_array());
        }
    }
    assert_eq!(Arc::strong_count(&snapshot), 2);
    assert!(labels.iter().any(|t| t.contains("X 16 ch   Y 16 ch")));
    for voice in 0..16 {
        assert!(
            colors.contains(&scope_voice_color(Color32::from_rgb(86, 215, 222), voice).to_array())
        );
        assert!(
            colors.contains(&scope_voice_color(Color32::from_rgb(250, 185, 80), voice).to_array())
        );
    }
}

#[test]
fn scope_time_exact_numeric_entry_preserves_minimum() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch: Patch =
        serde_json::from_value(json!({"version":2,"nodes":[{"id":"scope","kind":"scope"}]}))
            .unwrap();
    frame(&ctx, &mut rack, &patch, 0.0, vec![]);
    let target = pos2(24.0, 12.0) + scope_control_rect(2).center().to_vec2();
    frame(
        &ctx,
        &mut rack,
        &patch,
        1.0,
        vec![egui::Event::PointerMoved(target), button(target, true)],
    );
    frame(&ctx, &mut rack, &patch, 1.1, vec![button(target, false)]);
    let action = frame(
        &ctx,
        &mut rack,
        &patch,
        1.2,
        vec![
            egui::Event::Text("0.005".into()),
            egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    assert!(
        matches!(action,Some(RackAction::Command("set_parameter",ref args)) if args["parameter"]=="time" && (args["value"].as_f64().unwrap()-0.005).abs()<1e-6)
    );
}

#[test]
fn scope_empty_capture_and_gaps_never_emit_nonfinite_or_connected_traces() {
    fn lines(shape: &egui::Shape, out: &mut Vec<([Pos2; 2], Color32)>) {
        match shape {
            egui::Shape::LineSegment { points, stroke } => {
                assert!(points.iter().all(|p| p.x.is_finite() && p.y.is_finite()));
                out.push((*points, stroke.color));
            }
            egui::Shape::Vec(v) => {
                for s in v {
                    lines(s, out);
                }
            }
            _ => {}
        }
    }
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch: Patch =
        serde_json::from_value(json!({"version":2,"nodes":[{"id":"scope","kind":"scope"}]}))
            .unwrap();
    let color = scope_voice_color(Color32::from_rgb(86, 215, 222), 0);
    for xy in [false, true] {
        let snapshot = openpatch_core::ScopeSnapshot {
            x_channels: 1,
            y_channels: 1,
            xy,
            ..Default::default()
        };
        rack.set_scope_states(BTreeMap::from([("scope".into(), Arc::new(snapshot))]));
        let output = frame_output(&ctx, &mut rack, &patch, 1.0, vec![]).1;
        let mut rendered = vec![];
        for s in output.shapes {
            lines(&s.shape, &mut rendered);
        }
        assert_eq!(rendered.iter().filter(|(_, c)| *c == color).count(), 0);
    }
    let mut snapshot = openpatch_core::ScopeSnapshot {
        x_channels: 1,
        ..Default::default()
    };
    for bin in [0, 2, 4] {
        snapshot.x[0][bin] = openpatch_core::ScopePoint {
            min: -1.0,
            max: 1.0,
        };
    }
    rack.set_scope_states(BTreeMap::from([("scope".into(), Arc::new(snapshot))]));
    let output = frame_output(&ctx, &mut rack, &patch, 2.0, vec![]).1;
    let mut rendered = vec![];
    for s in output.shapes {
        lines(&s.shape, &mut rendered);
    }
    assert_eq!(
        rendered.iter().filter(|(_, c)| *c == color).count(),
        3,
        "Only min/max envelopes, no bridge across empty bins"
    );
}

#[test]
fn scope_xy_rotation_starts_at_oldest_bin_and_wraps() {
    assert_eq!(
        scope_xy_order(254).take(4).collect::<Vec<_>>(),
        vec![254, 255, 0, 1]
    );
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch: Patch =
        serde_json::from_value(json!({"version":2,"nodes":[{"id":"scope","kind":"scope"}]}))
            .unwrap();
    let mut snapshot = openpatch_core::ScopeSnapshot {
        x_channels: 1,
        y_channels: 1,
        xy: true,
        buffer_index: 254,
        ..Default::default()
    };
    for (bin, v) in [(254, -3.0), (255, -1.0), (0, 2.0)] {
        snapshot.x[0][bin] = openpatch_core::ScopePoint { min: v, max: v };
        snapshot.y[0][bin] = openpatch_core::ScopePoint { min: 0.0, max: 0.0 };
    }
    rack.set_scope_states(BTreeMap::from([("scope".into(), Arc::new(snapshot))]));
    let output = frame_output(&ctx, &mut rack, &patch, 1.0, vec![]).1;
    let color = scope_voice_color(Color32::from_rgb(86, 215, 222), 0);
    let lines: Vec<_> = output
        .shapes
        .into_iter()
        .filter_map(|s| {
            if let egui::Shape::LineSegment { points, stroke } = s.shape {
                (stroke.color == color).then_some(points)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(lines.len(), 2);
    assert!(
        lines.iter().all(|p| p[0].x < p[1].x),
        "Trace must move -3 → -1 → 2, without closing across capture gap"
    );
}

fn scope_text_bounds(shape: &egui::Shape, texts: &mut Vec<(String, Rect)>) {
    match shape {
        egui::Shape::Text(t) => texts.push((
            t.galley.text().into(),
            Rect::from_min_size(t.pos, t.galley.size()),
        )),
        egui::Shape::Vec(children) => {
            for child in children {
                scope_text_bounds(child, texts);
            }
        }
        _ => {}
    }
}

#[test]
fn scope_range_choices_show_exact_volts_and_emit_native_selection() {
    for axis in ["x_scale", "y_scale"] {
        for (index, choice) in (if axis=="x_scale" {&ATLAS_SCOPE_X_RANGE_CHOICES} else {&ATLAS_SCOPE_Y_RANGE_CHOICES}).iter().enumerate() {
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            let old = if index == 0 { 8 } else { 0 };
            let patch: Patch = serde_json::from_value(
                json!({"version":2,"nodes":[{"id":"scope","kind":"scope","params":{axis:old,"auto_range":0}}]}),
            )
            .unwrap();
            frame(&ctx, &mut rack, &patch, 0.0, vec![]);
            let target = pos2(24.0, 12.0)
                + scope_control_rect(usize::from(axis == "y_scale"))
                    .center()
                    .to_vec2();
            frame(
                &ctx,
                &mut rack,
                &patch,
                1.0,
                vec![egui::Event::PointerMoved(target), button(target, true)],
            );
            frame(&ctx, &mut rack, &patch, 1.1, vec![button(target, false)]);
            let output = frame_output(&ctx, &mut rack, &patch, 1.2, vec![]).1;
            let mut texts = vec![];
            for shape in output.shapes {
                scope_text_bounds(&shape.shape, &mut texts);
            }
            let bounds = texts
                .iter()
                .find(|(text, bounds)| text == *choice && bounds.center().y > target.y + 15.0)
                .expect("Full exact voltage option visible")
                .1;
            frame(
                &ctx,
                &mut rack,
                &patch,
                1.3,
                vec![
                    egui::Event::PointerMoved(bounds.center()),
                    button(bounds.center(), true),
                ],
            );
            let action = frame(
                &ctx,
                &mut rack,
                &patch,
                1.4,
                vec![button(bounds.center(), false)],
            );
            assert!(
                matches!(action,Some(RackAction::Command("set_parameter",ref args)) if args["parameter"]==axis && args["value"]==index)
            );
            assert_eq!(
                scope_volts_per_screen(index as u8),
                20.0 / 2.0_f32.powi(index as i32)
            );
        }
    }
}

#[test]
fn scope_stats_are_raw_voltage_and_reserved_outside_plot() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch: Patch =
        serde_json::from_value(json!({"version":2,"nodes":[{"id":"scope","kind":"scope"}]}))
            .unwrap();
    let mut state = openpatch_core::ScopeSnapshot {
        x_channels: 1,
        x_scale: 8,
        x_offset: 10.0,
        ..Default::default()
    };
    state.x[0][0] = openpatch_core::ScopePoint {
        min: -2.5,
        max: 4.25,
    };
    rack.set_scope_states(BTreeMap::from([("scope".into(), Arc::new(state))]));
    let output = frame_output(&ctx, &mut rack, &patch, 1.0, vec![]).1;
    let mut texts = vec![];
    for shape in output.shapes {
        scope_text_bounds(&shape.shape, &mut texts);
    }
    let display = Rect::from_min_max(pos2(54.0, 82.0), pos2(624.0, 262.0));
    let plot = Rect::from_min_max(pos2(64.0, 113.0), pos2(614.0, 225.0));
    for (text, bounds) in &texts {
        if text.starts_with("X  MIN") || text.starts_with("Y  MIN") {
            assert!(display.contains_rect(*bounds), "{text}: {bounds:?}");
            assert!(!plot.intersects(*bounds));
        }
    }
    assert!(texts
        .iter()
        .any(|(text, _)| text == "X  MIN -2.5 V   MAX 4.25 V   P-P 6.75 V"));
    assert!(texts
        .iter()
        .any(|(text, _)| text == "Y  MIN —   MAX —   P-P —   NO CAPTURE"));
    assert_eq!(scope_measurement_number(0.078125), "0.078125");
    assert_eq!(scope_measurement_number(0.0), "0");
}

#[test]
fn scope_trigger_guide_uses_actual_normalled_x_and_current_plot_scale() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch: Patch =
        serde_json::from_value(json!({"version":2,"nodes":[{"id":"scope","kind":"scope","params":{"auto_range":0}}]}))
            .unwrap();
    for (enabled, xy, external, threshold, expected) in [
        (true, false, 0, 2.0, true),
        (true, false, 1, 2.0, false),
        (false, false, 0, 2.0, false),
        (true, true, 0, 2.0, false),
        (true, false, 0, 20.0, false),
    ] {
        let state = openpatch_core::ScopeSnapshot {
            x_channels: 1,
            y_channels: 1,
            trigger_enabled: enabled,
            xy,
            trigger_input_channels: external,
            threshold,
            ..Default::default()
        };
        let status = scope_trigger_status(&state);
        if xy {
            assert!(status.contains("trigger ignored"));
        } else if external > 0 {
            assert!(status.starts_with("EXT trigger"));
        }
        rack.set_scope_states(BTreeMap::from([("scope".into(), Arc::new(state))]));
        let output = frame_output(&ctx, &mut rack, &patch, 1.0, vec![]).1;
        let lines: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|s| {
                if let egui::Shape::LineSegment { points, stroke } = &s.shape {
                    (stroke.color == SCOPE_TRIGGER_COLOR).then_some(*points)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(lines.len(), usize::from(expected));
        if expected {
            assert!((lines[0][0].y - 157.8).abs() < 0.01);
            assert_eq!(lines[0][0].x, 64.0);
            assert_eq!(lines[0][1].x, 614.0);
        }
        if threshold == 20.0 {
            let mut texts = vec![];
            for shape in output.shapes {
                scope_text_bounds(&shape.shape, &mut texts);
            }
            assert!(texts.iter().any(|(t, _)| t.contains("TRIGGER ABOVE RANGE")));
        }
    }
}

#[test]
fn scope_extreme_finite_bins_never_emit_infinite_trace_positions() {
    fn inspect(shape: &egui::Shape, texts: &mut Vec<String>) {
        match shape {
            egui::Shape::LineSegment { points, .. } => {
                assert!(points.iter().all(|p| scope_position_finite(*p)))
            }
            egui::Shape::Text(t) => {
                assert!(scope_position_finite(t.pos));
                texts.push(t.galley.text().into());
            }
            egui::Shape::Vec(children) => {
                for child in children {
                    inspect(child, texts);
                }
            }
            _ => {}
        }
    }
    assert_eq!(
        scope_midpoint(openpatch_core::ScopePoint {
            min: f32::MAX,
            max: f32::MAX
        }),
        f32::MAX
    );
    assert_eq!(
        scope_midpoint(openpatch_core::ScopePoint {
            min: -f32::MAX,
            max: f32::MAX
        }),
        0.0
    );
    assert_eq!(scope_measurement_number(1e-9), "1.000e-9");
    assert_eq!(scope_measurement_number(-1e-9), "-1.000e-9");
    assert_eq!(scope_measurement_number(-0.0), "0");
    for xy in [false, true] {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let patch: Patch =
            serde_json::from_value(json!({"version":2,"nodes":[{"id":"scope","kind":"scope"}]}))
                .unwrap();
        let mut state = openpatch_core::ScopeSnapshot {
            x_channels: 1,
            y_channels: 1,
            x_scale: 8,
            y_scale: 8,
            xy,
            ..Default::default()
        };
        for axis in [&mut state.x, &mut state.y] {
            axis[0][0] = openpatch_core::ScopePoint { min: 0.0, max: 0.0 };
            axis[0][1] = openpatch_core::ScopePoint {
                min: -f32::MAX,
                max: f32::MAX,
            };
            axis[0][2] = openpatch_core::ScopePoint {
                min: f32::MAX,
                max: f32::MAX,
            };
            axis[0][3] = openpatch_core::ScopePoint { min: 0.0, max: 0.0 };
        }
        rack.set_scope_states(BTreeMap::from([("scope".into(), Arc::new(state))]));
        let output = frame_output(&ctx, &mut rack, &patch, 1.0, vec![]).1;
        let mut texts = vec![];
        for shape in output.shapes {
            inspect(&shape.shape, &mut texts);
        }
        assert!(texts.iter().any(|t| t.starts_with("X  MIN -3.403e38 V")
            && t.contains("MAX 3.403e38 V")
            && t.contains("P-P 6.806e38 V")));
    }
}
