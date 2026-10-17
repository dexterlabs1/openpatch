use super::*;
use crate::construction_motion::MotionMode;
use std::time::Instant;

fn rescale_node() -> Node {
    serde_json::from_value(json!({"id":"rescale","kind":"rescale","params":{"gain":0.25,"min":-10,"max":10,"multiplier":1}})).unwrap()
}
fn rescale_frame(
    ctx: &egui::Context,
    rack: &mut Rack,
    node: &Node,
    events: Vec<egui::Event>,
    interactive: bool,
    focus: Option<(&str, f32)>,
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
                if let Some((name, value)) = focus {
                    ui.memory_mut(|m| {
                        m.request_focus(ui.id().with((
                            &node.id,
                            "rescale-setting",
                            name,
                            value.to_bits(),
                        )))
                    });
                }
                action = rack.atlas_rescale_panel(
                    ui,
                    node,
                    Rect::from_min_size(pos2(24.0, 12.0), vec2(240.0, 396.0) * rack.zoom),
                    37,
                    interactive,
                );
            });
        },
    );
    action
}
fn rescale_key(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}
#[test]
fn rescale_visible_settings_emit_revision_guarded_saved_edits() {
    for (name, target) in [
        ("multiplier", 10.0),
        ("multiplier", 100.0),
        ("multiplier", 1000.0),
        ("reflect_min", 1.0),
        ("reflect_max", 1.0),
    ] {
        for key in [egui::Key::Space, egui::Key::Enter] {
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            let node = rescale_node();
            let focus = if name == "multiplier" { target } else { 0.0 };
            rescale_frame(&ctx, &mut rack, &node, vec![], true, Some((name, focus)));
            let action = rescale_frame(&ctx, &mut rack, &node, vec![rescale_key(key)], true, None);
            assert!(
                matches!(action,Some(RackAction::Command("set_parameter",ref args)) if args["parameter"]==name && args["value"].as_f64()==Some(f64::from(target)) && args["expected_revision"]==37),
                "{name}: {target}/{key:?}"
            );
            assert!(
                rack.module_lights.is_empty(),
                "editing configuration cannot invent crossings"
            );
        }
    }
    for name in ["reflect_min", "reflect_max"] {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let mut node = rescale_node();
        node.params.insert(name.into(), 1.0);
        rescale_frame(&ctx, &mut rack, &node, vec![], true, Some((name, 0.0)));
        assert!(
            matches!(rescale_frame(&ctx,&mut rack,&node,vec![rescale_key(egui::Key::Space)],true,None),Some(RackAction::Command("set_parameter",ref args)) if args["value"]==0.0)
        );
    }
}
#[test]
fn rescale_visible_settings_respect_preview_disabled_escape_and_editor_guards() {
    for guard in ["preview", "escape", "shortcuts", "editor"] {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let node = rescale_node();
        rescale_frame(
            &ctx,
            &mut rack,
            &node,
            vec![],
            true,
            Some(("reflect_min", 0.0)),
        );
        if guard == "shortcuts" {
            rack.shortcuts_enabled = false;
        }
        if guard == "editor" {
            // Use the ordinary editor request path to establish a real open editor.
            let patch: Patch =
                serde_json::from_value(json!({"version":2,"nodes":[node.clone()]})).unwrap();
            rack.request_precision_control_for_smoke("rescale", "gain");
            frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 37);
            assert!(rack.editing.is_some());
        }
        let mut events = vec![rescale_key(egui::Key::Space)];
        if guard == "escape" {
            events.push(rescale_key(egui::Key::Escape));
        }
        assert!(
            rescale_frame(&ctx, &mut rack, &node, events, guard != "preview", None).is_none(),
            "{guard}"
        );
    }
}
#[test]
fn rescale_disabled_parent_blocks_focused_setting() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let node = rescale_node();
    rescale_frame(
        &ctx,
        &mut rack,
        &node,
        vec![],
        true,
        Some(("reflect_min", 0.0)),
    );
    let mut action = None;
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(600.0, 500.0))),
            events: vec![rescale_key(egui::Key::Space)],
            focused: true,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.disable();
                action = rack.atlas_rescale_panel(
                    ui,
                    &node,
                    Rect::from_min_size(pos2(24.0, 12.0), vec2(240.0, 396.0)),
                    37,
                    true,
                );
            });
        },
    );
    assert!(action.is_none());
}
#[test]
fn rescale_configured_guide_handles_degenerate_bounds_and_reflection_topology() {
    let mut node = rescale_node();
    assert_eq!(atlas_rescale_bound_guide(&node), "AFFINE → CLIP TO MIN…MAX");
    node.params.insert("reflect_min".into(), 1.0);
    assert_eq!(
        atlas_rescale_bound_guide(&node),
        "REFLECT MIN ONCE → CLIP MAX"
    );
    node.params.insert("reflect_max".into(), 1.0);
    assert_eq!(
        atlas_rescale_bound_guide(&node),
        "AFFINE → REPEATED REFLECTION"
    );
    node.params.insert("reflect_min".into(), 0.0);
    assert_eq!(
        atlas_rescale_bound_guide(&node),
        "REFLECT MAX ONCE → CLIP MIN"
    );
    for max in [-10.0, -11.0] {
        node.params.insert("max".into(), max);
        assert_eq!(atlas_rescale_bound_guide(&node), "MAX ≤ MIN: OUTPUT = MIN");
    }
}
#[test]
fn rescale_settings_and_readouts_clear_original_jacks() {
    assert_eq!(panel_width("rescale"), 240.0);
    for (input, x) in [(true, 60.0), (false, 180.0)] {
        assert_eq!(port_position("rescale", input, 0, 1, 240.0), pos2(x, 360.0));
        for name in ["gain", "offset", "min", "max"] {
            let (center, radius) = atlas_rescale_control(name);
            assert!(center.distance(pos2(x, 360.0)) > radius + 19.0);
            assert!(atlas_rescale_value_center(name).distance(pos2(x, 360.0)) > 35.0);
        }
        for name in ["multiplier", "reflect_min", "reflect_max"] {
            for value in [1.0, 10.0, 100.0, 1000.0] {
                let r = atlas_rescale_setting_rect(name, value);
                assert!(r.distance_to_pos(pos2(x, 360.0)) > 22.0);
            }
        }
    }
}
#[test]
fn rescale_all_seven_parameter_cues_have_real_anchors() {
    let before: Patch =
        serde_json::from_value(json!({"version":2,"nodes":[rescale_node()]})).unwrap();
    let mut after = before.clone();
    let changes = [
        ("gain", 0.25, 0.5),
        ("offset", 0.0, 2.0),
        ("min", -10.0, -5.0),
        ("max", 10.0, 5.0),
        ("multiplier", 1.0, 100.0),
        ("reflect_min", 0.0, 1.0),
        ("reflect_max", 0.0, 1.0),
    ];
    let parameter_changes:Vec<_>=changes.iter().map(|(name,old,new)| {after.nodes[0].params.insert((*name).into(),*new);json!({"id":"rescale","kind":"rescale","parameter":name,"old_effective":old,"new_effective":new})}).collect();
    let receipt = json!({"plan_id":7,"completed_steps":1,"last_revision":5,"current_revision":5,"last_commit":{"age_ms":0,"invalidate_existing_cables":false,"plan_id":7,"step_index":0,"before_revision":4,"revision":5,"label":"Configure Rescale","new_modules":[],"new_cables":[],"parameter_changes":parameter_changes}});
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    frame_revision_state(&ctx, &mut rack, &before, 0.0, vec![], true, true, 4);
    let now = Instant::now();
    rack.observe_construction_transition_at(&before, &after, 4, 5, &receipt, MotionMode::Full, now);
    rack.set_construction_clock_override(Some(now + std::time::Duration::from_millis(160)));
    assert!(
        frame_revision_state(&ctx, &mut rack, &after, 0.1, vec![], true, true, 5)
            .0
            .is_none()
    );
    for (name, _, _) in changes {
        let anchor = &rack.construction_anchors[&("rescale".into(), name.into())];
        assert!(anchor.rect.is_positive(), "{name}");
        assert!(!anchor.old_text.is_empty() && !anchor.new_text.is_empty());
    }
    assert!(rack.module_lights.is_empty());
}
