use super::*;
use crate::construction_motion::MotionMode;
use std::time::{Duration, Instant};
fn follow_patch(value: f32) -> Patch {
    serde_json::from_value(json!({"version":2,"nodes":[
        {"id":"near","kind":"constant"},
        {"id":"target","kind":"constant","params":{"value":value}},
        {"id":"other","kind":"constant"}]}))
    .unwrap()
}
fn follow_receipt(id: &str, previous: u64, step: u64, old: f32, new: f32, age: u64) -> Value {
    json!({"plan_id":13,"completed_steps":step+1,"last_revision":previous+1,"current_revision":previous+1,
        "last_commit":{"plan_id":13,"step_index":step,"before_revision":previous,"revision":previous+1,
        "age_ms":age,"invalidate_existing_cables":false,"new_modules":[],"new_cables":[],"moved_cables":[],
        "parameter_changes":[{"id":id,"kind":"constant","parameter":"value","old_effective":old,"new_effective":new}]}})
}
fn follow_rack(ctx: &egui::Context, patch: &Patch) -> Rack {
    let mut rack = Rack::default();
    rack.restore_layout(&json!({"version":1,"zoom":1,"pan":[24,12],"positions":{
        "near":[0,0],"target":[1500,0],"other":[3000,0]}}))
        .unwrap();
    frame_revision_state(ctx, &mut rack, patch, 0.0, vec![], true, true, 9);
    rack
}
fn follow_frame(
    ctx: &egui::Context,
    rack: &mut Rack,
    patch: &Patch,
    revision: u64,
    now: Instant,
    millis: u64,
) {
    rack.set_construction_clock_override(Some(now + Duration::from_millis(millis)));
    let (action, _) = frame_revision_state(
        ctx,
        rack,
        patch,
        1.0 + millis as f64 / 1000.0,
        vec![],
        true,
        true,
        revision,
    );
    assert!(action.is_none());
}
#[test]
fn follow_defaults_off_and_enabling_does_not_replay_consumed_or_invalid_receipts() {
    let ctx = egui::Context::default();
    let before = follow_patch(0.0);
    let after = follow_patch(3.0);
    let now = Instant::now();
    let mut rack = follow_rack(&ctx, &before);
    let pan = rack.pan;
    assert!(!rack.agent_follow_enabled());
    let receipt = follow_receipt("target", 9, 0, 0.0, 3.0, 0);
    rack.observe_construction_transition_at(
        &before,
        &after,
        9,
        10,
        &receipt,
        MotionMode::Full,
        now,
    );
    follow_frame(&ctx, &mut rack, &after, 10, now, 0);
    rack.set_agent_follow(true);
    rack.observe_construction_transition_at(
        &after,
        &after,
        10,
        10,
        &receipt,
        MotionMode::Full,
        now,
    );
    follow_frame(&ctx, &mut rack, &after, 10, now, 220);
    assert_eq!(rack.pan, pan);
    rack.observe_construction_transition_at(
        &before,
        &after,
        11,
        12,
        &receipt,
        MotionMode::Full,
        now,
    );
    follow_frame(&ctx, &mut rack, &after, 12, now, 300);
    assert_eq!(rack.pan, pan);
}
#[test]
fn offscreen_control_follows_smoothly_without_mutating_saved_layout_zoom_or_selection() {
    let ctx = egui::Context::default();
    let now = Instant::now();
    let before = follow_patch(0.0);
    let after = follow_patch(3.0);
    let mut rack = follow_rack(&ctx, &before);
    rack.select_only("near".into());
    let layout = rack.layout();
    let revision = rack.layout_revision();
    let positions = rack.positions.clone();
    let selection = rack.selection.clone();
    let pan = rack.pan;
    rack.set_agent_follow(true);
    rack.observe_construction_transition_at(
        &before,
        &after,
        9,
        10,
        &follow_receipt("target", 9, 0, 0.0, 3.0, 0),
        MotionMode::Full,
        now,
    );
    follow_frame(&ctx, &mut rack, &after, 10, now, 0);
    assert_eq!(rack.pan, pan, "first geometry resolution must not jump");
    assert!(
        rack.construction_anchors
            .contains_key(&("target".into(), "value".into())),
        "actual offscreen control must register its focus"
    );
    follow_frame(&ctx, &mut rack, &after, 10, now, 110);
    let midway = rack.pan;
    assert!(midway.x < pan.x);
    follow_frame(&ctx, &mut rack, &after, 10, now, 220);
    let settled = rack.pan;
    assert!(settled.x < midway.x && settled.is_finite());
    let anchor = &rack.construction_anchors[&("target".into(), "value".into())];
    assert!(rack
        .viewport
        .unwrap()
        .shrink(23.0)
        .contains_rect(anchor.rect));
    let node = after.nodes.iter().find(|n| n.id == "target").unwrap();
    let module = Rect::from_min_size(
        rack.viewport.unwrap().min + rack.pan + rack.positions["target"].to_vec2() * rack.zoom,
        vec2(module_width(node), PANEL_HEIGHT) * rack.zoom,
    );
    assert!(
        rack.viewport.unwrap().shrink(23.0).contains_rect(module),
        "normal-sized panel context must be visible with its actual control"
    );
    assert_eq!(rack.positions, positions);
    assert_eq!(rack.selection, selection);
    assert_eq!(rack.zoom(), 1.0);
    assert_eq!(rack.layout(), layout);
    assert_eq!(rack.layout_revision(), revision);
    rack.set_agent_follow(false);
    follow_frame(&ctx, &mut rack, &after, 10, now, 400);
    assert_eq!(rack.pan, settled);
    assert_eq!(rack.layout(), layout);
    // An actual manual toolbar zoom adopts the visible camera normally.
    rack.set_zoom(0.85);
    assert!(!rack.agent_follow_enabled());
    assert_ne!(rack.layout(), layout);
    assert_eq!(rack.layout()["pan"], json!([rack.pan.x, rack.pan.y]));
}
#[test]
fn delayed_observation_starts_at_current_camera_and_too_late_cues_are_skipped() {
    for age in [100, 150, 300] {
        let ctx = egui::Context::default();
        let now = Instant::now();
        let before = follow_patch(0.0);
        let after = follow_patch(3.0);
        let mut rack = follow_rack(&ctx, &before);
        let pan = rack.pan;
        rack.set_agent_follow(true);
        rack.observe_construction_transition_at(
            &before,
            &after,
            9,
            10,
            &follow_receipt("target", 9, 0, 0.0, 3.0, age),
            MotionMode::Full,
            now,
        );
        follow_frame(&ctx, &mut rack, &after, 10, now, 0);
        assert_eq!(rack.pan, pan);
        follow_frame(&ctx, &mut rack, &after, 10, now, 60);
        if age == 100 {
            assert!(rack.pan.x < pan.x);
        } else {
            assert_eq!(rack.pan, pan);
        }
        follow_frame(&ctx, &mut rack, &after, 10, now, 250);
        if age != 100 {
            assert_eq!(rack.pan, pan);
        }
    }
}
#[test]
fn manual_wheel_and_escape_cancel_follow_while_modal_or_unfocus_discards_pending_only() {
    for interruption in ["wheel", "escape", "modal", "unfocus"] {
        let ctx = egui::Context::default();
        let now = Instant::now();
        let before = follow_patch(0.0);
        let after = follow_patch(3.0);
        let mut rack = follow_rack(&ctx, &before);
        let layout = rack.layout();
        let pan = rack.pan;
        rack.set_agent_follow(true);
        rack.observe_construction_transition_at(
            &before,
            &after,
            9,
            10,
            &follow_receipt("target", 9, 0, 0.0, 3.0, 0),
            MotionMode::Full,
            now,
        );
        let events = match interruption {
            "wheel" => vec![
                egui::Event::PointerMoved(pos2(700.0, 600.0)),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: vec2(0.0, -30.0),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            "escape" => vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            _ => vec![],
        };
        rack.set_construction_clock_override(Some(now));
        frame_revision_state(
            &ctx,
            &mut rack,
            &after,
            1.0,
            events,
            interruption != "unfocus",
            interruption != "modal",
            10,
        );
        let stopped = rack.pan;
        follow_frame(&ctx, &mut rack, &after, 10, now, 250);
        if interruption == "wheel" {
            assert!(!rack.agent_follow_enabled());
            assert_ne!(rack.layout(), layout);
        } else {
            assert_eq!(rack.pan, pan);
            assert_eq!(rack.layout(), layout);
        }
        if interruption == "wheel" {
            assert_eq!(
                rack.pan.x, pan.x,
                "manual wheel must not resume horizontal follow"
            );
        } else {
            assert_eq!(rack.pan, stopped, "discarded pending cues never resume");
        }
        assert_eq!(
            rack.agent_follow_enabled(),
            matches!(interruption, "modal" | "unfocus")
        );
    }
}
#[test]
fn reduced_follows_immediately_off_does_not_and_visible_targets_keep_camera() {
    for (mode, onscreen) in [
        (MotionMode::Reduced, false),
        (MotionMode::Off, false),
        (MotionMode::Full, true),
    ] {
        let ctx = egui::Context::default();
        let now = Instant::now();
        let before = follow_patch(0.0);
        let after = follow_patch(3.0);
        let mut rack = follow_rack(&ctx, &before);
        if onscreen {
            rack.positions.insert("target".into(), pos2(300.0, 0.0));
        }
        let layout = rack.layout();
        let pan = rack.pan;
        rack.set_agent_follow(true);
        rack.observe_construction_transition_at(
            &before,
            &after,
            9,
            10,
            &follow_receipt("target", 9, 0, 0.0, 3.0, 0),
            mode,
            now,
        );
        follow_frame(&ctx, &mut rack, &after, 10, now, 0);
        if mode == MotionMode::Reduced {
            assert_eq!(rack.pan, pan, "planning frame retains rendered geometry");
            follow_frame(&ctx, &mut rack, &after, 10, now, 1);
            assert!(rack.pan.x < pan.x);
            assert!(rack
                .viewport
                .unwrap()
                .contains_rect(rack.construction_anchors[&("target".into(), "value".into())].rect));
        } else {
            assert_eq!(rack.pan, pan);
        }
        assert_eq!(rack.layout(), layout);
    }
}
#[test]
fn direct_cable_move_follows_only_actual_new_destination_plug() {
    let ctx = egui::Context::default();
    let now = Instant::now();
    let before: Patch = serde_json::from_value(json!({"version":2,"nodes":[
        {"id":"source","kind":"constant"},{"id":"old","kind":"gain"},{"id":"target","kind":"gain"}],
        "cables":[{"id":"wire","from":"source","to":"old","input":"signal"}]}))
    .unwrap();
    let mut after = before.clone();
    after.cables[0].to = "target".into();
    let receipt = json!({"status":"idle","current_revision":10,"last_commit":null,"last_move":{
        "before_revision":9,"revision":10,"age_ms":0,"moved_cables":[{"id":"wire",
            "before":{"from":"source","output":"out","to":"old","input":"signal"},
            "after":{"from":"source","output":"out","to":"target","input":"signal"}}]}});
    let mut rack = Rack::default();
    rack.restore_layout(&json!({"version":1,"zoom":1,"pan":[24,12],
        "positions":{"source":[-3000,0],"old":[0,0],"target":[1800,420]}}))
        .unwrap();
    frame_revision_state(&ctx, &mut rack, &before, 0.0, vec![], true, true, 9);
    let layout = rack.layout();
    rack.set_agent_follow(true);
    rack.observe_construction_transition_at(
        &before,
        &after,
        9,
        10,
        &receipt,
        MotionMode::Full,
        now,
    );
    follow_frame(&ctx, &mut rack, &after, 10, now, 0);
    follow_frame(&ctx, &mut rack, &after, 10, now, 220);
    let canvas = rack.viewport.unwrap();
    let catalog = openpatch_core::catalog();
    let destination = rack
        .port_center_for_smoke(canvas, &after, &catalog, "target", "signal", true)
        .unwrap();
    let source = rack
        .port_center_for_smoke(canvas, &after, &catalog, "source", "out", false)
        .unwrap();
    assert!(canvas.shrink(20.0).contains(destination));
    assert!(!canvas.contains(source));
    assert_eq!(rack.layout(), layout);
    let settled = rack.pan;
    follow_frame(&ctx, &mut rack, &after, 10, now, 700);
    assert_eq!(rack.pan, settled);
}
#[test]
fn newest_commit_replaces_pending_target_and_clear_or_restore_cannot_replay_it() {
    let ctx = egui::Context::default();
    let now = Instant::now();
    let before = follow_patch(0.0);
    let after = follow_patch(3.0);
    let mut latest = after.clone();
    latest
        .nodes
        .iter_mut()
        .find(|n| n.id == "other")
        .unwrap()
        .params
        .insert("value".into(), 4.0);
    let mut rack = follow_rack(&ctx, &before);
    rack.set_agent_follow(true);
    rack.observe_construction_transition_at(
        &before,
        &after,
        9,
        10,
        &follow_receipt("target", 9, 0, 0.0, 3.0, 0),
        MotionMode::Full,
        now,
    );
    // A later commit before painting replaces the earlier request entirely.
    rack.observe_construction_transition_at(
        &after,
        &latest,
        10,
        11,
        &follow_receipt("other", 10, 1, 0.0, 4.0, 0),
        MotionMode::Full,
        now,
    );
    follow_frame(&ctx, &mut rack, &latest, 11, now, 0);
    follow_frame(&ctx, &mut rack, &latest, 11, now, 220);
    assert!(rack
        .viewport
        .unwrap()
        .contains_rect(rack.construction_anchors[&("other".into(), "value".into())].rect));
    let stopped = rack.pan;
    rack.observe_construction_transition_at(
        &latest,
        &latest,
        11,
        11,
        &json!({"plan_id":14,"current_revision":11,"last_commit":null}),
        MotionMode::Full,
        now,
    );
    follow_frame(&ctx, &mut rack, &latest, 11, now, 500);
    assert_eq!(rack.pan, stopped);
    let restored = json!({"version":1,"zoom":0.5,"pan":[70,80],"positions":{"near":[0,0],"target":[1500,0],"other":[3000,0]}});
    rack.restore_layout(&restored).unwrap();
    let canonical_restored = rack.layout();
    follow_frame(&ctx, &mut rack, &latest, 11, now, 600);
    assert_eq!(rack.layout(), canonical_restored);
    assert_eq!(rack.pan, vec2(70.0, 80.0));
}

#[test]
fn stalled_frame_expires_inflight_camera_instead_of_jumping_to_old_edit() {
    let ctx = egui::Context::default();
    let now = Instant::now();
    let before = follow_patch(0.0);
    let after = follow_patch(3.0);
    let mut rack = follow_rack(&ctx, &before);
    let layout = rack.layout();
    rack.set_agent_follow(true);
    rack.observe_construction_transition_at(
        &before,
        &after,
        9,
        10,
        &follow_receipt("target", 9, 0, 0.0, 3.0, 0),
        MotionMode::Full,
        now,
    );
    follow_frame(&ctx, &mut rack, &after, 10, now, 0);
    follow_frame(&ctx, &mut rack, &after, 10, now, 60);
    let interrupted = rack.pan;
    follow_frame(&ctx, &mut rack, &after, 10, now, 3000);
    assert_eq!(rack.pan, interrupted);
    assert_eq!(rack.layout(), layout);
}

#[test]
fn small_viewport_reveals_real_control_on_oversized_panel_without_forced_zoom() {
    let ctx = egui::Context::default();
    let now = Instant::now();
    let mut before = follow_patch(0.0);
    before.nodes[1].kind = "macro_voice".into();
    before.nodes[1].params.clear();
    before.nodes[1].params.insert("harmonics".into(), 0.3);
    let mut after = before.clone();
    after.nodes[1].params.insert("harmonics".into(), 0.7);
    after.nodes[2].params.insert("value".into(), 4.0);
    let mut rack = Rack::default();
    rack.restore_layout(&json!({"version":1,"zoom":1,"pan":[24,12],
        "positions":{"near":[0,0],"target":[1500,0],"other":[3000,0]}}))
        .unwrap();
    let frame = |rack: &mut Rack, patch: &Patch, revision, ms| {
        rack.set_construction_clock_override(Some(now + Duration::from_millis(ms)));
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(240.0, 160.0))),
                time: Some(ms as f64 / 1000.0),
                focused: true,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| {
                        assert!(rack
                            .show(ui, patch, &openpatch_core::catalog(), 48000, revision)
                            .is_none());
                    });
            },
        );
    };
    frame(&mut rack, &before, 9, 0);
    let layout = rack.layout();
    let mut receipt = follow_receipt("target", 9, 0, 0.3, 0.7, 0);
    receipt["last_commit"]["parameter_changes"][0]["kind"] = json!("macro_voice");
    receipt["last_commit"]["parameter_changes"][0]["parameter"] = json!("harmonics");
    receipt["last_commit"]["parameter_changes"].as_array_mut().unwrap().push(json!({
        "id":"other","kind":"constant","parameter":"value","old_effective":0.0,"new_effective":4.0}));
    rack.set_agent_follow(true);
    rack.observe_construction_transition_at(
        &before,
        &after,
        9,
        10,
        &receipt,
        MotionMode::Full,
        now,
    );
    frame(&mut rack, &after, 10, 0);
    frame(&mut rack, &after, 10, 260);
    let control = rack.construction_anchors[&("target".into(), "harmonics".into())].rect;
    assert!(rack.viewport.unwrap().shrink(4.0).contains_rect(control));
    assert_eq!(rack.zoom(), 1.0);
    assert_eq!(rack.layout(), layout);
    assert!(rack.pan.is_finite());
    let settled = rack.pan;
    // Two simultaneous far-apart controls choose the nearest meaningful
    // focus once; overlapping live cues never alternate the camera.
    frame(&mut rack, &after, 10, 300);
    frame(&mut rack, &after, 10, 500);
    assert_eq!(rack.pan, settled);
}
