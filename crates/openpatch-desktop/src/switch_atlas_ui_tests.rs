use super::*;
use crate::construction_motion::MotionMode;
use std::time::Instant;

fn switch_node(kind: &str) -> Node {
    serde_json::from_value(json!({"id":"switch","kind":kind,"params":{"steps":4,"declick":0}}))
        .unwrap()
}
fn switch_control_frame(
    ctx: &egui::Context,
    rack: &mut Rack,
    node: &Node,
    events: Vec<egui::Event>,
    interactive: bool,
    focus: Option<usize>,
) -> Option<RackAction> {
    let mut action = None;
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(600.0, 500.0))),
            events,
            focused: true,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                if let Some(index) = focus {
                    let id = if index < 3 {
                        ui.id().with((&node.id, "length", index as u8 + 2))
                    } else {
                        ui.id().with((&node.id, "declick"))
                    };
                    ui.memory_mut(|m| m.request_focus(id));
                }
                action = rack.switch_panel(
                    ui,
                    node,
                    Rect::from_min_size(pos2(24.0, 12.0), vec2(180.0, 396.0) * rack.zoom),
                    37,
                    interactive,
                );
            });
        },
    );
    action
}
fn switch_key(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

#[test]
fn switch_keyboard_controls_emit_guarded_configuration_edits_without_audio() {
    for kind in ["switch_1to4", "switch_4to1"] {
        for (index, parameter, value) in [(0, "steps", 2.0), (1, "steps", 3.0), (3, "declick", 1.0)]
        {
            for key in [egui::Key::Space, egui::Key::Enter] {
                let ctx = egui::Context::default();
                let mut rack = Rack::default();
                let node = switch_node(kind);
                switch_control_frame(&ctx, &mut rack, &node, vec![], true, Some(index));
                let action =
                    switch_control_frame(&ctx, &mut rack, &node, vec![switch_key(key)], true, None);
                assert!(
                    matches!(action, Some(RackAction::Command("set_parameter", ref args))
                    if args["parameter"] == parameter && args["value"].as_f64() == Some(value)
                    && args["expected_revision"] == 37),
                    "{kind}: {parameter}/{key:?}"
                );
                assert!(
                    rack.sequence_states.is_empty(),
                    "configuration must not synthesize a selected route"
                );
            }
        }
    }
}

#[test]
fn switch_keyboard_configuration_respects_preview_escape_and_shortcut_guards() {
    for guard in ["preview", "escape", "shortcuts"] {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let node = switch_node("switch_1to4");
        switch_control_frame(&ctx, &mut rack, &node, vec![], true, Some(3));
        if guard == "shortcuts" {
            rack.shortcuts_enabled = false;
        }
        let mut events = vec![switch_key(egui::Key::Space)];
        if guard == "escape" {
            events.push(switch_key(egui::Key::Escape));
        }
        assert!(
            switch_control_frame(&ctx, &mut rack, &node, events, guard != "preview", None)
                .is_none(),
            "{guard}"
        );
    }
}

#[test]
fn switch_configured_length_and_reported_route_have_separate_provenance() {
    assert_eq!(
        switch_route_status(None, false, false),
        ("ROUTE —".into(), "WAITING FOR ENGINE")
    );
    assert_eq!(
        switch_route_status(None, true, true),
        ("ROUTE —".into(), "BYPASSED · FROZEN")
    );
    let state = Some(ModuleTelemetry {
        step: 4,
        length: 4,
        running: true,
    });
    assert_eq!(
        switch_route_status(state, false, true),
        ("ROUTE 4 / 4".into(), "ENGINE SELECTED")
    );
    assert_eq!(
        switch_route_status(state, false, false),
        ("ROUTE 4 / 4".into(), "OFFLINE SNAPSHOT")
    );
    assert_eq!(
        switch_route_status(state, true, false),
        ("ROUTE 4 / 4".into(), "BYPASSED · FROZEN")
    );
}

#[test]
fn switch_controls_clear_original_socket_targets_at_all_shared_zooms() {
    for kind in ["switch_1to4", "switch_4to1"] {
        let inputs = if kind == "switch_1to4" { 3 } else { 6 };
        let outputs = if kind == "switch_1to4" { 4 } else { 1 };
        assert_eq!(panel_width(kind), 180.0);
        let controls = [
            switch_steps_rect(2),
            switch_steps_rect(3),
            switch_steps_rect(4),
            switch_declick_rect(),
        ];
        for zoom in [0.42, 1.0, 1.4] {
            for (i, control) in controls.iter().enumerate() {
                let screen = Rect::from_min_max(
                    (control.min.to_vec2() * zoom).to_pos2(),
                    (control.max.to_vec2() * zoom).to_pos2(),
                );
                for other in &controls[i + 1..] {
                    assert!(!control.intersects(*other));
                }
                for (input, count) in [(true, inputs), (false, outputs)] {
                    for index in 0..count {
                        let jack = port_position(kind, input, index, count, 180.0) * zoom;
                        assert!(
                            !screen.expand(20.0 * zoom).contains(jack),
                            "{kind}/{zoom}: control overlaps socket"
                        );
                    }
                }
            }
        }
        assert_eq!(
            port_position(kind, true, 0, inputs, 180.0),
            pos2(50.0, 213.0)
        );
        assert_eq!(
            port_position(kind, true, 1, inputs, 180.0),
            pos2(130.0, 213.0)
        );
        for index in 0..4 {
            let at = if kind == "switch_1to4" {
                port_position(kind, false, index, outputs, 180.0)
            } else {
                port_position(kind, true, index + 2, inputs, 180.0)
            };
            assert_eq!(
                at,
                pos2(
                    if kind == "switch_1to4" { 130.0 } else { 50.0 },
                    262.0 + index as f32 * 36.0
                )
            );
        }
    }
}

#[test]
fn switch_mcp_cues_cover_both_configuration_controls_without_route_telemetry() {
    for kind in ["switch_1to4", "switch_4to1"] {
        let before: Patch =
            serde_json::from_value(json!({"version":2,"nodes":[{"id":"switch","kind":kind}]}))
                .unwrap();
        let mut after = before.clone();
        after.nodes[0].params.insert("steps".into(), 2.0);
        after.nodes[0].params.insert("declick".into(), 1.0);
        let receipt = json!({"plan_id":7,"completed_steps":1,"last_revision":5,"current_revision":5,
            "last_commit":{"age_ms":0,"invalidate_existing_cables":false,"plan_id":7,"step_index":0,
                "before_revision":4,"revision":5,"label":"Configure routes","new_modules":[],"new_cables":[],
                "parameter_changes":[
                    {"id":"switch","kind":kind,"parameter":"steps","old_effective":4,"new_effective":2},
                    {"id":"switch","kind":kind,"parameter":"declick","old_effective":0,"new_effective":1}]}});
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        frame_revision_state(&ctx, &mut rack, &before, 0.0, vec![], true, true, 4);
        let now = Instant::now();
        rack.observe_construction_transition_at(
            &before,
            &after,
            4,
            5,
            &receipt,
            MotionMode::Full,
            now,
        );
        rack.set_construction_clock_override(Some(now + std::time::Duration::from_millis(160)));
        assert!(
            frame_revision_state(&ctx, &mut rack, &after, 0.1, vec![], true, true, 5)
                .0
                .is_none()
        );
        for name in ["steps", "declick"] {
            let anchor = &rack.construction_anchors[&("switch".into(), name.into())];
            assert!(anchor.rect.is_positive());
            assert!(!anchor.old_text.is_empty() && !anchor.new_text.is_empty());
            assert!(anchor.marker.is_none());
        }
        assert!(rack.sequence_states.is_empty());
    }
}

#[test]
fn switch_only_observed_route_emits_a_shared_dark_room_lamp() {
    for kind in ["switch_1to4", "switch_4to1"] {
        for room in [1.0, 0.0] {
            for selected in [None, Some(1), Some(4)] {
                let patch: Patch =
                    serde_json::from_value(json!({"version":2,"nodes":[{"id":"s","kind":kind}]}))
                        .unwrap();
                let ctx = egui::Context::default();
                let mut rack = Rack::default();
                rack.set_room_brightness(room);
                if let Some(step) = selected {
                    rack.sequence_states.insert(
                        "s".into(),
                        ModuleTelemetry {
                            step,
                            length: 4,
                            running: true,
                        },
                    );
                }
                let output = frame_output(&ctx, &mut rack, &patch, 0.0, vec![]).1;
                let halos=output.shapes.iter().filter(|s|matches!(&s.shape,egui::Shape::Mesh(m) if m.vertices.len()==120 && m.vertices.iter().all(|v|v.color.a()==0))).count();
                assert_eq!(
                    halos,
                    usize::from(selected.is_some()),
                    "{kind}/{room}/{selected:?}"
                );
                if let Some(step) = selected {
                    let at = pos2(
                        24.0 + if kind == "switch_1to4" { 98.0 } else { 82.0 },
                        12.0 + 262.0 + f32::from(step - 1) * 36.0,
                    );
                    assert!(output.shapes.iter().any(|s|matches!(&s.shape,egui::Shape::Circle(c) if c.center.distance(at)<0.001 && c.fill==Color32::from_rgb(235,173,64))));
                }
            }
        }
    }
}
