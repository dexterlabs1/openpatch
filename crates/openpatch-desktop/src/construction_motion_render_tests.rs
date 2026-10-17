use super::*;
use crate::construction_motion::{Marker, MotionMode};
use std::time::{Duration, Instant};
fn patch(kind: &str, name: &str, value: f32) -> Patch {
    serde_json::from_value(
        json!({"version":2,"nodes":[{"id":"target","kind":kind,"params":{name:value}}]}),
    )
    .unwrap()
}
fn snapshot(previous: u64, revision: u64, kind: &str, name: &str, old: f32, new: f32) -> Value {
    json!({"plan_id":13,"completed_steps":1,"last_revision":revision,"current_revision":revision,"last_commit":{"age_ms":0,"invalidate_existing_cables":false,"plan_id":13,"step_index":0,"before_revision":previous,"revision":revision,"label":"Shape tone","new_modules":[],"new_cables":[],"parameter_changes":[{"id":"target","kind":kind,"parameter":name,"old_effective":old,"new_effective":new}]}})
}
fn text(output: &egui::FullOutput) -> Vec<String> {
    fn collect(s: &egui::Shape, out: &mut Vec<String>) {
        match s {
            egui::Shape::Text(t) => out.push(t.galley.text().into()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| collect(s, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for s in &output.shapes {
        collect(&s.shape, &mut out);
    }
    out
}
#[test]
fn committed_value_is_immediate_while_control_anchor_tracks_pan_zoom_and_human_edit() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let now = Instant::now();
    let before = patch("constant", "value", 0.0);
    let after = patch("constant", "value", 3.0);
    frame_revision_state(&ctx, &mut rack, &before, 0.0, vec![], true, true, 9);
    rack.observe_construction_transition_at(
        &before,
        &after,
        9,
        10,
        &snapshot(9, 10, "constant", "value", 0.0, 3.0),
        MotionMode::Full,
        now,
    );
    rack.set_construction_clock_override(Some(now + Duration::from_millis(180)));
    let pan = rack.pan;
    let selection = rack.selection.clone();
    let positions = rack.positions.clone();
    let (action, output) =
        frame_revision_state(&ctx, &mut rack, &after, 0.1, vec![], true, true, 10);
    assert!(action.is_none());
    assert_eq!(rack.pan, pan);
    assert_eq!(rack.selection, selection);
    assert_eq!(rack.positions, positions);
    assert!(text(&output).iter().any(|t| t == "3.000"));
    assert!(text(&output).iter().any(|t| t == "0.000 → 3.000"));
    let anchor = rack.construction_anchors[&("target".into(), "value".into())].clone();
    assert!(matches!(anchor.marker, Some(Marker::VerticalSlider { .. })));
    rack.pan += vec2(80.0, 50.0);
    rack.set_zoom(0.85);
    frame_revision_state(&ctx, &mut rack, &after, 0.2, vec![], true, true, 10);
    let shifted = rack.construction_anchors[&("target".into(), "value".into())].rect;
    assert_ne!(anchor.rect, shifted);
    // The real numeric hit target still opens its editor, while its cue yields.
    let at = pos2(shifted.center().x, shifted.bottom() - 8.0 * rack.zoom);
    frame_revision_state(
        &ctx,
        &mut rack,
        &after,
        0.3,
        vec![egui::Event::PointerMoved(at), button(at, true)],
        true,
        true,
        10,
    );
    assert!(rack.construction_animation.parameters.is_empty());
    frame_revision_state(
        &ctx,
        &mut rack,
        &after,
        0.4,
        vec![button(at, false)],
        true,
        true,
        10,
    );
    assert!(rack.editing.is_some());
}
#[test]
fn known_slider_mapping_and_discrete_choice_have_honest_markers_and_missing_anchor_has_header_badge(
) {
    for (kind, name, old, new, marker, expected) in [
        (
            "modal_resonator",
            "frequency",
            220.0,
            440.0,
            true,
            "220.0 Hz → 440.0 Hz",
        ),
        ("sh_asr", "range", 0.0, 5.0, false, "0..10 V → −1..1 V"),
        ("sh_asr", "random", 0.0, 1.0, false, "RND OFF → RND ON"),
        ("score_sequencer", "running", 0.0, 1.0, false, "Off → On"),
        ("quantizer", "scale_mask", 4095.0, 1.0, false, ""),
    ] {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let now = Instant::now();
        let before = patch(kind, name, old);
        let after = patch(kind, name, new);
        frame_revision_state(&ctx, &mut rack, &before, 0.0, vec![], true, true, 9);
        rack.observe_construction_transition_at(
            &before,
            &after,
            9,
            10,
            &snapshot(9, 10, kind, name, old, new),
            MotionMode::Full,
            now,
        );
        rack.set_construction_clock_override(Some(now + Duration::from_millis(160)));
        let output = frame_revision_state(&ctx, &mut rack, &after, 0.1, vec![], true, true, 10).1;
        if name == "scale_mask" {
            assert!(!rack
                .construction_anchors
                .contains_key(&("target".into(), name.into())));
            assert!(text(&output).iter().any(|t| t.starts_with("scale_mask  ")));
            assert!(text(&output).iter().any(|t| t == "1 NOTE ENABLED"));
        } else {
            let anchor = &rack.construction_anchors[&("target".into(), name.into())];
            assert_eq!(anchor.marker.is_some(), marker);
            assert!(
                text(&output).iter().any(|t| t == expected),
                "{kind} {name}: {:?}",
                text(&output)
            );
        }
    }
}
#[test]
fn module_arrival_is_fixed_geometry_selection_neutral_and_human_press_cancels_wash() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let now = Instant::now();
    let after = patch("constant", "value", 0.0);
    let before = Patch::new_v2();
    frame_revision_state(&ctx, &mut rack, &after, 0.0, vec![], true, true, 10);
    let positions = rack.positions.clone();
    let snapshot = json!({"plan_id":2,"completed_steps":1,"last_revision":10,"current_revision":10,"last_commit":{"age_ms":0,"invalidate_existing_cables":false,"plan_id":2,"step_index":0,"before_revision":9,"revision":10,"label":"Add DC","new_modules":[{"id":"target","kind":"constant"}],"new_cables":[],"parameter_changes":[]}});
    rack.observe_construction_transition_at(
        &before,
        &after,
        9,
        10,
        &snapshot,
        MotionMode::Full,
        now,
    );
    rack.set_construction_clock_override(Some(now + Duration::from_millis(100)));
    assert_eq!(
        rack.construction_effect_counts_at(now + Duration::from_millis(100))
            .0,
        1
    );
    let selection = rack.selection.clone();
    let jack = fixture_jack(&rack, &after, "target", "out", false);
    frame_revision_state(&ctx, &mut rack, &after, 0.1, vec![], true, true, 10);
    assert_eq!(positions, rack.positions);
    assert_eq!(selection, rack.selection);
    assert_eq!(jack, fixture_jack(&rack, &after, "target", "out", false));
    frame_revision_state(
        &ctx,
        &mut rack,
        &after,
        0.2,
        vec![egui::Event::PointerMoved(jack), button(jack, true)],
        true,
        true,
        10,
    );
    assert!(rack.construction_animation.modules.is_empty());
}
#[test]
fn cable_phase_uses_three_hundred_ms_prefix_and_one_destination_settle_or_static_reduced() {
    let now = Instant::now();
    let after = stacked_fixture();
    let mut before = after.clone();
    before.cables.clear();
    let mut rack = Rack::default();
    let snapshot = construction_test_snapshot(&before, &after, 9, 10);
    rack.observe_construction_transition_at(
        &before,
        &after,
        9,
        10,
        &snapshot,
        MotionMode::Full,
        now,
    );
    let phase = rack
        .construction_animation
        .cables
        .phase(&after.cables[0], now + Duration::from_millis(150))
        .unwrap();
    assert_eq!(phase.progress, 0.875);
    assert_eq!(phase.settle, 0.0);
    let settled = rack
        .construction_animation
        .cables
        .phase(&after.cables[0], now + Duration::from_millis(360))
        .unwrap();
    assert_eq!(settled.progress, 1.0);
    assert!((settled.settle - 0.5).abs() < 0.00001);
    assert!(rack
        .construction_animation
        .cables
        .phase(&after.cables[0], now + Duration::from_millis(420))
        .is_none());
    let mut reduced = Rack::default();
    reduced.observe_construction_transition_at(
        &before,
        &after,
        9,
        10,
        &snapshot,
        MotionMode::Reduced,
        now,
    );
    let phase = reduced
        .construction_animation
        .cables
        .phase(&after.cables[0], now + Duration::from_millis(150))
        .unwrap();
    assert!(phase.reduced);
    assert_eq!(phase.progress, 1.0);
    reduced.set_construction_motion_mode(MotionMode::Off);
    assert!(reduced
        .construction_animation
        .progress(&after.cables[0], now)
        .is_none());
    reduced.set_construction_motion_mode(MotionMode::Full);
    reduced.observe_construction_transition_at(
        &before,
        &after,
        9,
        10,
        &snapshot,
        MotionMode::Full,
        now,
    );
    assert!(reduced
        .construction_animation
        .progress(&after.cables[0], now)
        .is_none());
}

#[test]
fn cable_body_hover_settles_the_real_route_permanently_without_a_new_pointer_target() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let now = Instant::now();
    let after = stacked_fixture();
    let mut before = after.clone();
    before.cables.clear();
    frame_revision_state(&ctx, &mut rack, &before, 0.0, vec![], true, true, 9);
    rack.observe_construction_transition_at(
        &before,
        &after,
        9,
        10,
        &construction_test_snapshot(&before, &after, 9, 10),
        MotionMode::Full,
        now,
    );
    rack.set_construction_clock_override(Some(now + Duration::from_millis(150)));
    let cable = &after.cables[0];
    let from = fixture_jack(&rack, &after, &cable.from, &cable.output, false);
    let to = fixture_jack(&rack, &after, &cable.to, &cable.input, true);
    let count = after
        .cables
        .iter()
        .filter(|c| c.to == cable.to && c.input == cable.input)
        .count();
    let fan = if count > 1 { -20.0 } else { 0.0 };
    let point = cable_curve_prefix(
        cable_curve_points(from, to, rack.zoom, fan, rack.cable_presentation.tension),
        0.5,
    )[3];
    let action = frame_revision_state(
        &ctx,
        &mut rack,
        &after,
        0.1,
        vec![egui::Event::PointerMoved(point)],
        true,
        true,
        10,
    )
    .0;
    assert!(action.is_none());
    assert!(rack.construction_animation.progress(cable, now).is_none());
    frame_revision_state(
        &ctx,
        &mut rack,
        &after,
        0.2,
        vec![egui::Event::PointerMoved(pos2(1000.0, 600.0))],
        true,
        true,
        10,
    );
    assert!(rack.construction_animation.progress(cable, now).is_none());
    assert!(rack.pending.is_none());
    assert!(rack.selection.is_empty());
}

#[test]
fn every_catalog_kind_has_an_actual_anchor_or_a_named_header_fallback_for_visible_parameter_cues() {
    let catalog = openpatch_core::catalog();
    let now = Instant::now();
    let mut covered = 0;
    for module in catalog["modules"].as_array().unwrap() {
        let kind = module["kind"].as_str().unwrap();
        let Some((name, spec)) = module["params"].as_object().and_then(|p| {
            p.iter()
                .find(|(_, s)| s["hidden"] != true && s["min"].as_f64() < s["max"].as_f64())
        }) else {
            continue;
        };
        let old = spec["default"].as_f64().unwrap() as f32;
        let max = spec["max"].as_f64().unwrap() as f32;
        let new = if old == max {
            spec["min"].as_f64().unwrap() as f32
        } else {
            max
        };
        let before = patch(kind, name, old);
        let after = patch(kind, name, new);
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        frame_revision_state(&ctx, &mut rack, &before, 0.0, vec![], true, true, 9);
        rack.observe_construction_transition_at(
            &before,
            &after,
            9,
            10,
            &snapshot(9, 10, kind, name, old, new),
            MotionMode::Full,
            now,
        );
        rack.set_construction_clock_override(Some(now + Duration::from_millis(160)));
        let (action, output) =
            frame_revision_state(&ctx, &mut rack, &after, 0.1, vec![], true, true, 10);
        assert!(action.is_none(), "motion does not emit commands: {kind}");
        assert_eq!(rack.construction_animation.parameters.len(), 1, "{kind}");
        if let Some(anchor) = rack
            .construction_anchors
            .get(&("target".into(), name.to_owned()))
        {
            assert!(anchor.rect.is_finite(), "{kind}");
            assert!(
                text(&output).iter().any(|t| t.contains('→')),
                "{kind}: real anchor badge"
            );
        } else {
            assert!(
                text(&output)
                    .iter()
                    .any(|t| t.starts_with(&format!("{}  ", parameter_label(name)))),
                "{kind}: named fallback for {name}"
            );
        }
        assert_eq!(after.nodes[0].params[name], new);
        covered += 1;
    }
    assert!(
        covered >= 40,
        "all parameter-bearing catalog kinds exercised: {covered}"
    );
}

#[test]
fn overlapping_same_id_same_route_recreation_invalidates_old_cues_once_and_preserves_fresh_arrivals(
) {
    let now = Instant::now();
    let after = stacked_fixture();
    let mut before = after.clone();
    before.cables.clear();
    let mut rack = Rack::default();
    rack.observe_construction_transition_at(
        &before,
        &after,
        9,
        10,
        &construction_test_snapshot(&before, &after, 9, 10),
        MotionMode::Full,
        now,
    );
    let old = &after.cables[0];
    assert!(rack.construction_animation.progress(old, now).is_some());
    // Transaction retired/recreated `old` identically, and added a genuinely new
    // stable ID. Before/after route diff alone cannot see the recreation.
    let mut next = after.clone();
    let mut fresh = old.clone();
    fresh.id = Some("fresh-arrival".into());
    next.cables.push(fresh.clone());
    let mut snapshot = construction_test_snapshot(&after, &next, 10, 11);
    snapshot["completed_steps"] = json!(2);
    snapshot["last_commit"]["step_index"] = json!(1);
    snapshot["last_commit"]["invalidate_existing_cables"] = json!(true);
    let time = now + Duration::from_millis(50);
    rack.observe_construction_transition_at(
        &after,
        &next,
        10,
        11,
        &snapshot,
        MotionMode::Full,
        time,
    );
    assert!(rack.construction_animation.progress(old, time).is_none());
    assert_eq!(
        rack.construction_animation.progress(&fresh, time),
        Some(0.0)
    );
    rack.observe_construction_transition_at(
        &next,
        &next,
        11,
        11,
        &snapshot,
        MotionMode::Full,
        time + Duration::from_millis(50),
    );
    assert!(
        rack.construction_animation
            .progress(&fresh, time + Duration::from_millis(50))
            .is_some(),
        "repeated invalidation flag cannot clear its own fresh cue"
    );
}

#[test]
fn aged_cable_and_module_receipts_never_reveal_late_or_restart_on_redraw() {
    let now = Instant::now();
    let after = stacked_fixture();
    let mut before = after.clone();
    before.cables.clear();
    let mut snapshot = construction_test_snapshot(&before, &after, 9, 10);
    snapshot["last_commit"]["age_ms"] = json!(30_000);
    let mut rack = Rack::default();
    rack.observe_construction_transition_at(
        &before,
        &after,
        9,
        10,
        &snapshot,
        MotionMode::Full,
        now,
    );
    assert!(rack
        .construction_animation
        .progress(&after.cables[0], now)
        .is_none());
    assert_eq!(rack.construction_effect_counts_at(now), (0, 0, 0));
    let mut snapshot = construction_test_snapshot(&before, &after, 9, 10);
    snapshot["last_commit"]["age_ms"] = json!(350);
    let mut rack = Rack::default();
    rack.observe_construction_transition_at(
        &before,
        &after,
        9,
        10,
        &snapshot,
        MotionMode::Full,
        now,
    );
    let phase = rack
        .construction_animation
        .cables
        .phase(&after.cables[0], now)
        .unwrap();
    assert_eq!(phase.progress, 1.0);
    assert!(phase.settle > 0.4 && phase.settle < 0.5);
    let before = Patch::new_v2();
    let after = patch("constant", "value", 0.0);
    let mut snapshot = self::snapshot(9, 10, "constant", "value", 0.0, 0.0);
    snapshot["last_commit"]["parameter_changes"] = json!([]);
    snapshot["last_commit"]["new_modules"] = json!([{"id":"target","kind":"constant"}]);
    snapshot["last_commit"]["age_ms"] = json!(320);
    let mut rack = Rack::default();
    rack.observe_construction_transition_at(
        &before,
        &after,
        9,
        10,
        &snapshot,
        MotionMode::Full,
        now,
    );
    assert_eq!(rack.construction_effect_counts_at(now).0, 0);
    assert_eq!(rack.construction_animation.focus.len(), 1);
}

#[test]
fn accepted_stock_lfo_construction_edits_anchor_all_six_controls_and_manual_toggles_take_over() {
    let workspace = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
    let before = patch("stock_lfo", "rate", 1.0);
    session.replace_patch(before.clone(), None).unwrap();
    let previous = session.revision();
    let shared = session.into_shared();
    let changes = [
        ("rate", 2.0),
        ("pulse_width", 0.75),
        ("fm_amount", 0.3),
        ("pwm_amount", -0.4),
        ("offset", 0.0),
        ("invert", 1.0),
    ];
    let edits = changes.iter().map(|(name, value)| json!({"tool":"set_parameter","arguments":{"id":"target","parameter":name,"value":value}})).collect::<Vec<_>>();
    openpatch::control::execute_shared(&shared, "start_construction", json!({"expected_revision":previous,"title":"Shape Stock LFO","interval_ms":100,"steps":[{"label":"Set all six controls","edits":edits}]})).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let (after, revision, receipt) = loop {
        let session = shared.lock().unwrap();
        let receipt = session.construction_snapshot();
        if receipt["completed_steps"] == 1 {
            break (session.patch().clone(), session.revision(), receipt);
        }
        assert!(
            Instant::now() < deadline,
            "Construction did not commit: {receipt}"
        );
        drop(session);
        std::thread::sleep(Duration::from_millis(1));
    };
    assert_eq!(
        receipt["last_commit"]["parameter_changes"]
            .as_array()
            .unwrap()
            .len(),
        6
    );
    let ctx = egui::Context::default();
    crate::panel_skin::install_fonts(&ctx);
    let mut rack = Rack::default();
    frame_revision_state(&ctx, &mut rack, &before, 0.0, vec![], true, true, previous);
    let now = Instant::now();
    rack.observe_construction_transition_at(
        &before,
        &after,
        previous,
        revision,
        &receipt,
        MotionMode::Full,
        now,
    );
    rack.set_construction_clock_override(Some(now + Duration::from_millis(160)));
    let (action, output) =
        frame_revision_state(&ctx, &mut rack, &after, 0.1, vec![], true, true, revision);
    assert!(action.is_none());
    assert_eq!(rack.construction_anchors.len(), 6);
    fn label_rect(shape: &egui::Shape, label: &str) -> Option<Rect> {
        match shape {
            egui::Shape::Text(t) if t.galley.text() == label => {
                Some(Rect::from_min_size(t.pos, t.galley.size()))
            }
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| label_rect(shape, label)),
            _ => None,
        }
    }
    for (name, value) in changes {
        let anchor = &rack.construction_anchors[&("target".into(), name.into())];
        let label = match name {
            "offset" => "±5 V".into(),
            "invert" => "Invert".into(),
            _ => stock_lfo_parameter_value(name, value, None),
        };
        let actual_text = output
            .shapes
            .iter()
            .find_map(|shape| label_rect(&shape.shape, &label))
            .expect("Actual corresponding control text");
        assert!(
            anchor.rect.contains_rect(actual_text),
            "{name}: cue must surround its actual control"
        );
        if matches!(name, "offset" | "invert") {
            assert!(anchor.marker.is_none());
            assert!((anchor.rect.height() - 24.0).abs() < 0.01);
            assert_eq!(
                (anchor.old_text.as_str(), anchor.new_text.as_str()),
                if name == "offset" {
                    ("0–10 V", "±5 V")
                } else {
                    ("Invert Off", "Invert On")
                }
            );
        } else {
            assert!(matches!(anchor.marker, Some(Marker::Rotary { .. })));
        }
    }
    // The same accepted receipt cannot add interactive cue targets to a preview.
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|module| module["kind"] == "stock_lfo")
        .unwrap();
    let mut preview = ModulePreview::new(module);
    preview.patch = after.clone();
    preview.renderer.positions.clear();
    preview
        .renderer
        .positions
        .insert("target".into(), Pos2::ZERO);
    preview.renderer.observe_construction_transition_at(
        &before,
        &after,
        previous,
        revision,
        &receipt,
        MotionMode::Full,
        now,
    );
    let preview_ctx = egui::Context::default();
    let output = preview_ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(900.0, 600.0))),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                preview.show(
                    ui,
                    Rect::from_min_size(pos2(24.0, 12.0), preview.size()),
                    &catalog,
                )
            });
        },
    );
    assert!(preview.renderer.construction_anchors.is_empty());
    for anchor in rack.construction_anchors.values() {
        let badge = format!("{} → {}", anchor.old_text, anchor.new_text);
        assert!(!text(&output).iter().any(|label| label == &badge));
    }
    // A real press on each mode button cancels that cue immediately; release
    // retains the original parameter command and revision guard.
    for (index, name) in ["offset", "invert"].into_iter().enumerate() {
        let at = rack.construction_anchors[&("target".into(), name.into())]
            .rect
            .center();
        frame_revision_state(
            &ctx,
            &mut rack,
            &after,
            0.2 + index as f64 * 0.2,
            vec![egui::Event::PointerMoved(at), button(at, true)],
            true,
            true,
            revision,
        );
        assert!(rack
            .construction_animation
            .parameter("target", "stock_lfo", name)
            .is_none());
        let action = frame_revision_state(
            &ctx,
            &mut rack,
            &after,
            0.3 + index as f64 * 0.2,
            vec![button(at, false)],
            true,
            true,
            revision,
        )
        .0;
        assert!(
            matches!(action, Some(RackAction::Command("set_parameter", args)) if args["parameter"] == name && args["expected_revision"] == revision)
        );
    }
    assert_eq!(
        serde_json::to_value(shared.lock().unwrap().patch()).unwrap(),
        serde_json::to_value(&after).unwrap(),
        "Presentation and manual actions await host publication"
    );
}

#[test]
fn accepted_stock_vco_construction_edits_anchor_all_six_controls_and_manual_toggles_take_over() {
    let workspace = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
    let before = patch("stock_vco", "frequency", 0.0);
    session.replace_patch(before.clone(), None).unwrap();
    let previous = session.revision();
    let shared = session.into_shared();
    let changes = [
        ("frequency", 12.0),
        ("pulse_width", 0.75),
        ("fm_amount", 0.3),
        ("pwm_amount", -0.4),
        ("linear_fm", 1.0),
        ("soft_sync", 1.0),
    ];
    let edits = changes.iter().map(|(name, value)| json!({"tool":"set_parameter","arguments":{"id":"target","parameter":name,"value":value}})).collect::<Vec<_>>();
    openpatch::control::execute_shared(&shared, "start_construction", json!({"expected_revision":previous,"title":"Shape Stock VCO","interval_ms":100,"steps":[{"label":"Set all six controls","edits":edits}]})).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let (after, revision, receipt) = loop {
        let session = shared.lock().unwrap();
        let receipt = session.construction_snapshot();
        if receipt["completed_steps"] == 1 {
            break (session.patch().clone(), session.revision(), receipt);
        }
        assert!(
            Instant::now() < deadline,
            "Construction did not commit: {receipt}"
        );
        drop(session);
        std::thread::sleep(Duration::from_millis(1));
    };
    assert_eq!(
        receipt["last_commit"]["parameter_changes"]
            .as_array()
            .unwrap()
            .len(),
        6
    );
    let ctx = egui::Context::default();
    crate::panel_skin::install_fonts(&ctx);
    let mut rack = Rack::default();
    frame_revision_state(&ctx, &mut rack, &before, 0.0, vec![], true, true, previous);
    let now = Instant::now();
    rack.observe_construction_transition_at(
        &before,
        &after,
        previous,
        revision,
        &receipt,
        MotionMode::Full,
        now,
    );
    rack.set_construction_clock_override(Some(now + Duration::from_millis(160)));
    let (action, output) =
        frame_revision_state(&ctx, &mut rack, &after, 0.1, vec![], true, true, revision);
    assert!(action.is_none());
    assert_eq!(rack.construction_anchors.len(), 6);
    fn label_rect(shape: &egui::Shape, label: &str) -> Option<Rect> {
        match shape {
            egui::Shape::Text(t) if t.galley.text() == label => {
                Some(Rect::from_min_size(t.pos, t.galley.size()))
            }
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| label_rect(shape, label)),
            _ => None,
        }
    }
    for (name, value) in changes {
        let anchor = &rack.construction_anchors[&("target".into(), name.into())];
        let label = match name {
            "linear_fm" => "Linear FM".into(),
            "soft_sync" => "Soft sync".into(),
            _ => stock_vco_parameter_value(name, value),
        };
        let actual_text = output
            .shapes
            .iter()
            .find_map(|shape| label_rect(&shape.shape, &label))
            .expect("Actual corresponding control text");
        assert!(
            anchor.rect.contains_rect(actual_text),
            "{name}: cue must surround its actual control"
        );
        if matches!(name, "linear_fm" | "soft_sync") {
            assert!(anchor.marker.is_none());
            assert!((anchor.rect.height() - 24.0).abs() < 0.01);
            assert_eq!(
                (anchor.old_text.as_str(), anchor.new_text.as_str()),
                if name == "linear_fm" {
                    ("Exponential FM", "Linear FM")
                } else {
                    ("Hard sync", "Soft sync")
                }
            );
        } else {
            assert!(matches!(anchor.marker, Some(Marker::Rotary { .. })));
        }
    }
    // The same accepted receipt cannot add interactive cue targets to a preview.
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|module| module["kind"] == "stock_vco")
        .unwrap();
    let mut preview = ModulePreview::new(module);
    preview.patch = after.clone();
    preview.renderer.positions.clear();
    preview
        .renderer
        .positions
        .insert("target".into(), Pos2::ZERO);
    preview.renderer.observe_construction_transition_at(
        &before,
        &after,
        previous,
        revision,
        &receipt,
        MotionMode::Full,
        now,
    );
    let preview_ctx = egui::Context::default();
    let output = preview_ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(900.0, 600.0))),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                preview.show(
                    ui,
                    Rect::from_min_size(pos2(24.0, 12.0), preview.size()),
                    &catalog,
                )
            });
        },
    );
    assert!(preview.renderer.construction_anchors.is_empty());
    for anchor in rack.construction_anchors.values() {
        let badge = format!("{} → {}", anchor.old_text, anchor.new_text);
        assert!(!text(&output).iter().any(|label| label == &badge));
    }
    // A real press on each mode button cancels that cue immediately; release
    // retains the original parameter command and revision guard.
    for (index, name) in ["linear_fm", "soft_sync"].into_iter().enumerate() {
        let at = rack.construction_anchors[&("target".into(), name.into())]
            .rect
            .center();
        frame_revision_state(
            &ctx,
            &mut rack,
            &after,
            0.2 + index as f64 * 0.2,
            vec![egui::Event::PointerMoved(at), button(at, true)],
            true,
            true,
            revision,
        );
        assert!(rack
            .construction_animation
            .parameter("target", "stock_vco", name)
            .is_none());
        let action = frame_revision_state(
            &ctx,
            &mut rack,
            &after,
            0.3 + index as f64 * 0.2,
            vec![button(at, false)],
            true,
            true,
            revision,
        )
        .0;
        assert!(
            matches!(action, Some(RackAction::Command("set_parameter", args)) if args["parameter"] == name && args["expected_revision"] == revision)
        );
    }
    assert_eq!(
        serde_json::to_value(shared.lock().unwrap().patch()).unwrap(),
        serde_json::to_value(&after).unwrap(),
        "Presentation and manual actions await host publication"
    );
}

#[test]
fn accepted_stock_seq3_edits_anchor_every_scalar_and_durable_control_and_manual_press_takes_over() {
    assert_seq3_construction_controls("stock_seq3");
}
#[test]
fn accepted_legacy_seq3_edits_anchor_every_scalar_and_saved_control_and_manual_press_takes_over() {
    assert_seq3_construction_controls("seq3");
}
fn assert_seq3_construction_controls(kind: &str) {
    let mut changes = vec![
        (
            "tempo".to_owned(),
            if kind == "stock_seq3" { 2.0 } else { 240.0 },
        ),
        ("steps".into(), 5.0),
        ("tempo_amount".into(), 0.5),
        ("steps_amount".into(), 0.5),
    ];
    for row in 1..=3 {
        for step in 1..=8 {
            changes.push((format!("row{row}_{step}"), 1.25));
        }
    }
    for gate in 1..=8 {
        changes.push((format!("gate{gate}"), 0.0));
    }
    changes.extend([("running".into(), 0.0), ("passthrough".into(), 1.0)]);
    assert_eq!(changes.len(), 38);
    for batch in changes.chunks(8) {
        let workspace = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
        let before = patch(
            kind,
            "tempo",
            if kind == "stock_seq3" { 1.0 } else { 120.0 },
        );
        session.replace_patch(before.clone(), None).unwrap();
        let previous = session.revision();
        let shared = session.into_shared();
        let edits=batch.iter().map(|(name,value)|json!({"tool":"set_parameter","arguments":{"id":"target","parameter":name,"value":value}})).collect::<Vec<_>>();
        openpatch::control::execute_shared(&shared,"start_construction",json!({"expected_revision":previous,"title":"Shape Stock SEQ3","interval_ms":100,"steps":[{"label":"Set actual panel controls","edits":edits}]})).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let (after, revision, receipt) = loop {
            let session = shared.lock().unwrap();
            let receipt = session.construction_snapshot();
            if receipt["completed_steps"] == 1 {
                break (session.patch().clone(), session.revision(), receipt);
            }
            assert!(
                Instant::now() < deadline,
                "Construction did not commit: {receipt}"
            );
            drop(session);
            std::thread::sleep(Duration::from_millis(1));
        };
        assert_eq!(
            receipt["last_commit"]["parameter_changes"]
                .as_array()
                .unwrap()
                .len(),
            batch.len(),
            "Every requested value actually changes: {batch:?}; receipt {receipt}"
        );
        let ctx = egui::Context::default();
        crate::panel_skin::install_fonts(&ctx);
        let mut rack = Rack::default();
        rack.set_sequence_states(BTreeMap::new(), true);
        frame_revision_state(&ctx, &mut rack, &before, 0.0, vec![], true, true, previous);
        let now = Instant::now();
        rack.observe_construction_transition_at(
            &before,
            &after,
            previous,
            revision,
            &receipt,
            MotionMode::Full,
            now,
        );
        rack.set_construction_clock_override(Some(now + Duration::from_millis(160)));
        let (action, _) =
            frame_revision_state(&ctx, &mut rack, &after, 0.1, vec![], true, true, revision);
        assert!(action.is_none());
        assert_eq!(rack.construction_anchors.len(), batch.len());
        let mut targets = vec![];
        for (name, _) in batch {
            let anchor = rack.construction_anchors[&("target".into(), name.clone())].clone();
            if stock_seq3_scalar(name) {
                assert!(anchor.rect.width() >= 32.0);
                assert!(anchor.rect.height() >= 19.0);
            } else {
                assert!(anchor.marker.is_none());
                assert!(anchor.rect.height() <= 25.1);
            }
            assert!(
                anchor.rect.top() > 70.0,
                "{name}: actual control, not header badge"
            );
            targets.push((name.clone(), anchor.rect.center()));
        }
        let catalog = openpatch_core::catalog();
        let module = catalog["modules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["kind"] == kind)
            .unwrap();
        let mut preview = ModulePreview::new(module);
        preview.patch = after.clone();
        preview.renderer.positions.clear();
        preview
            .renderer
            .positions
            .insert("target".into(), Pos2::ZERO);
        preview.renderer.observe_construction_transition_at(
            &before,
            &after,
            previous,
            revision,
            &receipt,
            MotionMode::Full,
            now,
        );
        let preview_ctx = egui::Context::default();
        let out = preview_ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 600.0))),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    preview.show(
                        ui,
                        Rect::from_min_size(pos2(24.0, 12.0), preview.size()),
                        &catalog,
                    )
                });
            },
        );
        assert!(preview.renderer.construction_anchors.is_empty());
        // Configured clock-path arrows are static notation, not edit cues.
        for anchor in rack.construction_anchors.values() {
            let cue = format!("{} → {}", anchor.old_text, anchor.new_text);
            assert!(!text(&out).iter().any(|t| t == &cue));
        }
        // A pointer press on the real control cancels the corresponding cue.
        // Rotaries use their actual center, not the union's lower readout.
        for (index, (name, mut at)) in targets.into_iter().enumerate() {
            if stock_seq3_scalar(&name) {
                let (local, _) = atlas_sequence_control(&name);
                at = pos2(24.0, 12.0) + local.to_vec2();
            }
            let t = 0.3 + index as f64 * 0.3;
            frame_revision_state(
                &ctx,
                &mut rack,
                &after,
                t,
                vec![egui::Event::PointerMoved(at), button(at, true)],
                true,
                true,
                revision,
            );
            assert!(
                rack.construction_animation
                    .parameter("target", kind, &name)
                    .is_none(),
                "{name} manual takeover"
            );
            frame_revision_state(
                &ctx,
                &mut rack,
                &after,
                t + 0.1,
                vec![button(at, false)],
                true,
                true,
                revision,
            );
            if name == "passthrough" {
                egui::Popup::close_all(&ctx);
            }
        }
    }
}

include!("stock_quantizer_construction_tests.rs");

#[test]
fn committed_rotary_edits_have_a_moving_angular_cursor_without_delaying_values() {
    for (kind, parameter, old, new) in [
        ("stock_vco", "frequency", -12.0, 24.0),
        ("stock_lfo", "rate", 0.0, 2.0),
        ("stock_vca_mix", "level1", 0.25, 1.0),
        ("gate", "duty", 0.2, 0.8),
        ("score_sequencer", "tempo", 120.0, 180.0),
        ("poly_sum", "level", 0.2, 0.8),
        ("poly_constant", "v1", -4.0, 8.0),
        ("poly_constant", "v16", -7.25, 3.5),
        ("score_sequencer", "swing", 50.0, 65.0),
        ("mid_side", "encode_width", 0.5, 1.5),
        ("mid_side", "decode_width", 0.5, 1.5),
        ("stock_mid_side", "encode_width", 0.5, 1.5),
        ("stock_mid_side", "decode_width", 0.5, 1.5),
        ("analog_vco", "pulse_width", 0.2, 0.8),
    ] {
        let ctx = egui::Context::default();
        crate::panel_skin::install_fonts(&ctx);
        let mut rack = Rack::default();
        let before = patch(kind, parameter, old);
        let after = patch(kind, parameter, new);
        frame_revision_state(&ctx, &mut rack, &before, 0.0, vec![], true, true, 9);
        let now = Instant::now();
        rack.observe_construction_transition_at(
            &before,
            &after,
            9,
            10,
            &snapshot(9, 10, kind, parameter, old, new),
            MotionMode::Full,
            now,
        );
        let mut positions = Vec::new();
        for (frame, millis) in [90, 180].into_iter().enumerate() {
            rack.set_construction_clock_override(Some(now + Duration::from_millis(millis)));
            let (action, output) = frame_revision_state(
                &ctx,
                &mut rack,
                &after,
                0.1 + frame as f64 * 0.1,
                vec![],
                true,
                true,
                10,
            );
            assert!(action.is_none(), "Motion must not issue parameter commands");
            assert_eq!(after.nodes[0].params[parameter], new);
            let anchor = &rack.construction_anchors[&("target".into(), parameter.into())];
            let Some(Marker::Rotary { center, radius, .. }) = anchor.marker.as_ref() else {
                panic!("{kind}: accepted rotary edit needs its real rotary anchor");
            };
            let cursor = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Circle(circle)
                        if (circle.radius - 1.7 * rack.zoom).abs() < 0.001
                            && circle.fill.a() > 0
                            && (circle.center.distance(*center) - *radius).abs() < 0.01 =>
                    {
                        Some(circle.center)
                    }
                    _ => None,
                })
                .expect("Visible construction cursor follows the knob's actual scale");
            assert!(anchor.rect.expand(4.0).contains(cursor));
            positions.push(cursor);
        }
        assert!(
            positions[0].distance(positions[1]) > 0.1,
            "{kind}: cursor must move with construction time"
        );
        // Reduced motion keeps the accepted control/value cue but suppresses
        // traveling cursors, matching other construction controls.
        let mut reduced = Rack::default();
        frame_revision_state(&ctx, &mut reduced, &before, 0.4, vec![], true, true, 9);
        reduced.observe_construction_transition_at(
            &before,
            &after,
            9,
            10,
            &snapshot(9, 10, kind, parameter, old, new),
            MotionMode::Reduced,
            now,
        );
        reduced.set_construction_clock_override(Some(now + Duration::from_millis(90)));
        let (action, output) =
            frame_revision_state(&ctx, &mut reduced, &after, 0.5, vec![], true, true, 10);
        assert!(action.is_none());
        let anchor = &reduced.construction_anchors[&("target".into(), parameter.into())];
        let Some(Marker::Rotary { center, radius, .. }) = anchor.marker.as_ref() else {
            panic!("Missing reduced control anchor");
        };
        assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Circle(circle) if (circle.radius - 1.7 * reduced.zoom).abs() < 0.001
            && circle.fill.a() > 0 && (circle.center.distance(*center) - *radius).abs() < 0.01)));
    }
}

#[test]
fn converted_rotary_cue_tracks_pan_zoom_and_yields_to_human_precision_entry() {
    let ctx = egui::Context::default();
    crate::panel_skin::install_fonts(&ctx);
    let mut rack = Rack::default();
    let now = Instant::now();
    let before = patch("gate", "duty", 0.2);
    let after = patch("gate", "duty", 0.8);
    frame_revision_state(&ctx, &mut rack, &before, 0.0, vec![], true, true, 9);
    rack.observe_construction_transition_at(
        &before,
        &after,
        9,
        10,
        &snapshot(9, 10, "gate", "duty", 0.2, 0.8),
        MotionMode::Full,
        now,
    );
    rack.set_construction_clock_override(Some(now + Duration::from_millis(140)));
    let (action, output) =
        frame_revision_state(&ctx, &mut rack, &after, 0.1, vec![], true, true, 10);
    assert!(action.is_none());
    assert!(text(&output).iter().any(|label| label == "80.0%"));
    assert_eq!(after.nodes[0].params["duty"], 0.8);
    let anchor = rack.construction_anchors[&("target".into(), "duty".into())].clone();
    let Some(Marker::Rotary {
        center,
        radius,
        old,
        new,
    }) = anchor.marker
    else {
        panic!("converted gate must emit a rotary marker");
    };
    assert!((radius - 30.0 * rack.zoom).abs() < 0.001);
    assert!((old - (0.2 - 0.01) / 0.98).abs() < 0.001);
    assert!((new - (0.8 - 0.01) / 0.98).abs() < 0.001);

    rack.pan += vec2(80.0, 50.0);
    rack.set_zoom(0.85);
    frame_revision_state(&ctx, &mut rack, &after, 0.2, vec![], true, true, 10);
    let shifted = rack.construction_anchors[&("target".into(), "duty".into())].clone();
    let Some(Marker::Rotary {
        center: moved_center,
        radius: moved_radius,
        ..
    }) = shifted.marker
    else {
        panic!("pan/zoom preserves angular cue");
    };
    assert_ne!(center, moved_center);
    assert!((moved_radius - 30.0 * rack.zoom).abs() < 0.001);
    assert_ne!(anchor.rect, shifted.rect);

    // The readout remains a separate exact-entry target after the view change.
    let (local, layout_radius) = parameter_position(2, 1, module_width(&after.nodes[0]));
    let readout = moved_center + vec2(0.0, (layout_radius + 15.0) * rack.zoom);
    assert!(
        (moved_center.distance(
            rack.viewport.unwrap().min
                + rack.pan
                + (rack.positions["target"].to_vec2() + local.to_vec2()) * rack.zoom
        )) < 0.001
    );
    frame_revision_state(
        &ctx,
        &mut rack,
        &after,
        0.3,
        vec![egui::Event::PointerMoved(readout), button(readout, true)],
        true,
        true,
        10,
    );
    assert!(rack.construction_animation.parameters.is_empty());
    frame_revision_state(
        &ctx,
        &mut rack,
        &after,
        0.4,
        vec![button(readout, false)],
        true,
        true,
        10,
    );
    assert!(rack
        .editing
        .as_ref()
        .is_some_and(|edit| edit.id == "target" && edit.parameter == "duty"));
}
