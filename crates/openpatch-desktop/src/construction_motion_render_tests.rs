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
        ("sh_asr", "random", 0.0, 1.0, false, ""),
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
        if name == "random" {
            assert!(!rack
                .construction_anchors
                .contains_key(&("target".into(), name.into())));
            assert!(text(&output).iter().any(|t| t.starts_with("random  ")));
            assert!(text(&output).iter().any(|t| t == "RND ON"));
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
