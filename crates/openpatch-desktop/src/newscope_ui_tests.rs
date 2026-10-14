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
        serde_json::from_value(json!({"version":2,"nodes":[{"id":"scope","kind":"scope"}]}))
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
