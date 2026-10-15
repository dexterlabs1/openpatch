use super::*;

fn frame(
    ctx: &egui::Context,
    rack: &mut Rack,
    patch: &Patch,
    time: f64,
    events: Vec<egui::Event>,
) -> (Option<RackAction>, egui::FullOutput) {
    frame_revision_state(ctx, rack, patch, time, events, true, true, 0)
}

fn guidance_text(output: &egui::FullOutput) -> String {
    output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text().contains("Esc cancels") => {
                Some(text.galley.text())
            }
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn modifiers_event(modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key: egui::Key::F1,
        physical_key: None,
        pressed: false,
        repeat: false,
        modifiers,
    }
}

fn press(position: Pos2, down: bool, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::PointerButton {
        pos: position,
        button: PointerButton::Primary,
        pressed: down,
        modifiers,
    }
}

#[test]
fn pointer_drag_guidance_tracks_release_shift_and_press_captured_intent() {
    let mut patch = stacked_fixture();
    let mut collision = patch.cables[0].clone();
    collision.id = Some("collision".into());
    collision.to = "dest".into();
    patch.cables.push(collision);
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    frame(&ctx, &mut rack, &patch, 0.0, vec![]);
    let from = fixture_jack(&rack, &patch, "target", "signal", true);
    let to = fixture_jack(&rack, &patch, "dest", "signal", true);
    frame(
        &ctx,
        &mut rack,
        &patch,
        1.0,
        vec![egui::Event::PointerMoved(from), button(from, true)],
    );
    let (_, output) = frame(
        &ctx,
        &mut rack,
        &patch,
        1.1,
        vec![egui::Event::PointerMoved(to)],
    );
    assert!(rack.dragging_cable);
    assert!(
        guidance_text(&output).contains("Release: Delete"),
        "{}",
        guidance_text(&output)
    );
    assert!(guidance_text(&output).contains("src · OUT -> dest · IN"));
    let before = serde_json::to_value(&patch).unwrap();
    let orders = (
        openpatch_core::cable_endpoint_order(
            &patch.cables,
            openpatch_core::CableEndpoint::Input,
            "target",
            "signal",
        ),
        rack.positions.clone(),
    );
    // Changing command/control/alt after pickup cannot silently change Move to
    // Clone/Create. Shift alone affects the actual release duplicate policy.
    let modifiers = egui::Modifiers {
        shift: true,
        command: true,
        ctrl: true,
        alt: true,
        ..Default::default()
    };
    let (_, output) = frame(
        &ctx,
        &mut rack,
        &patch,
        1.2,
        vec![modifiers_event(modifiers)],
    );
    assert!(guidance_text(&output).contains("Release: Move · stack"));
    let (_, output) = frame(
        &ctx,
        &mut rack,
        &patch,
        1.21,
        vec![modifiers_event(egui::Modifiers::NONE)],
    );
    assert!(guidance_text(&output).contains("Release: Delete"));
    let (_, output) = frame(
        &ctx,
        &mut rack,
        &patch,
        1.22,
        vec![modifiers_event(modifiers)],
    );
    assert!(guidance_text(&output).contains("Release: Move · stack"));
    assert_eq!(
        rack.pending.as_ref().unwrap().operation,
        CableOperation::Move
    );
    assert_eq!(serde_json::to_value(&patch).unwrap(), before);
    assert_eq!(rack.positions, orders.1);
    assert_eq!(
        openpatch_core::cable_endpoint_order(
            &patch.cables,
            openpatch_core::CableEndpoint::Input,
            "target",
            "signal"
        ),
        orders.0
    );
    let action = frame(
        &ctx,
        &mut rack,
        &patch,
        1.3,
        vec![press(to, false, modifiers)],
    )
    .0
    .unwrap();
    assert!(
        matches!(action, RackAction::Command("move_cables", args) if args["duplicate_mode"] == "stack")
    );
    assert!(rack.pending.is_none());
}

#[test]
fn pointer_invalid_drop_and_escape_distinguish_move_clone_and_new_cables() {
    for (origin, modifiers, expected, tool) in [
        (
            "target",
            egui::Modifiers::NONE,
            "Delete",
            Some("disconnect"),
        ),
        (
            "target",
            egui::Modifiers {
                command: true,
                shift: true,
                ..Default::default()
            },
            "Discard",
            None,
        ),
        ("alt", egui::Modifiers::NONE, "Discard", None),
    ] {
        for cancel in [false, true] {
            let patch = stacked_fixture();
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            frame(&ctx, &mut rack, &patch, 0.0, vec![]);
            let from = fixture_jack(
                &rack,
                &patch,
                origin,
                if origin == "alt" { "out" } else { "signal" },
                origin != "alt",
            );
            let empty = pos2(1050.0, 650.0);
            frame(
                &ctx,
                &mut rack,
                &patch,
                1.0,
                vec![
                    egui::Event::PointerMoved(from),
                    press(from, true, modifiers),
                ],
            );
            let (_, output) = frame(
                &ctx,
                &mut rack,
                &patch,
                1.1,
                vec![egui::Event::PointerMoved(empty)],
            );
            assert!(rack.dragging_cable, "{origin} {modifiers:?}");
            assert!(
                guidance_text(&output).contains(&format!("Release: {expected}")),
                "{}",
                guidance_text(&output)
            );
            let event = if cancel {
                egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }
            } else {
                button(empty, false)
            };
            let action = frame(&ctx, &mut rack, &patch, 1.2, vec![event]).0;
            if !cancel && tool.is_some() {
                assert!(matches!(action, Some(RackAction::Command(name, _)) if Some(name) == tool));
            } else {
                assert!(action.is_none());
            }
            assert!(rack.pending.is_none());
            assert_eq!(patch.cables.len(), 2);
        }
    }
}

#[test]
fn click_pending_uses_click_action_and_does_not_claim_invalid_drop_deletion() {
    let patch = stacked_fixture();
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    frame(&ctx, &mut rack, &patch, 0.0, vec![]);
    let from = fixture_jack(&rack, &patch, "target", "signal", true);
    frame(
        &ctx,
        &mut rack,
        &patch,
        1.0,
        vec![egui::Event::PointerMoved(from), button(from, true)],
    );
    frame(&ctx, &mut rack, &patch, 1.1, vec![button(from, false)]);
    let (_, output) = frame(
        &ctx,
        &mut rack,
        &patch,
        1.2,
        vec![egui::Event::PointerMoved(pos2(1050.0, 650.0))],
    );
    assert!(!rack.dragging_cable);
    assert!(rack.pending.is_some());
    assert!(guidance_text(&output).contains("Click: Choose a jack"));
    assert!(!guidance_text(&output).contains("Delete"));
}

#[test]
fn guidance_reports_v1_rejection_and_v2_stack_using_actual_action() {
    for version in [1, 2] {
        let mut patch = stacked_fixture();
        patch.version = version;
        patch.cables.truncate(1);
        let origin = CableEnd::Output {
            node: "alt".into(),
            port: "out".into(),
        };
        let pending = PendingCable::new(origin, &patch);
        let target = CableEnd::Input {
            node: "target".into(),
            port: "signal".into(),
        };
        let guidance = cable_gesture_guidance(
            &patch,
            &pending,
            Some(&target),
            &openpatch_core::catalog(),
            false,
            true,
        );
        let action = cable_release_action(
            &patch,
            &pending,
            Some(&target),
            &openpatch_core::catalog(),
            false,
        );
        if version == 1 {
            assert!(action.is_none());
            assert_eq!(guidance.action, "Release: Discard · requires engine v2");
        } else {
            assert!(action.is_some());
            assert_eq!(guidance.action, "Release: Create · stack");
        }
    }
}

#[test]
fn guidance_does_not_misdiagnose_capacity_or_missing_ports_as_version_limits() {
    let mut patch = stacked_fixture();
    patch.version = 1;
    patch.cables.truncate(1);
    let pending = PendingCable::new(
        CableEnd::Output {
            node: "alt".into(),
            port: "out".into(),
        },
        &patch,
    );
    let catalog = openpatch_core::catalog();
    let missing = CableEnd::Input {
        node: "dest".into(),
        port: "missing".into(),
    };
    let guidance = cable_gesture_guidance(&patch, &pending, Some(&missing), &catalog, false, true);
    assert_eq!(guidance.action, "Release: Discard");
    patch
        .cables
        .resize(openpatch_core::MAX_CABLES, patch.cables[0].clone());
    let target = CableEnd::Input {
        node: "dest".into(),
        port: "signal".into(),
    };
    let guidance = cable_gesture_guidance(&patch, &pending, Some(&target), &catalog, false, true);
    assert_eq!(guidance.action, "Release: Discard");
}

#[test]
fn badge_stays_in_offset_canvas_at_edges_and_uses_screen_sized_text() {
    let canvas = Rect::from_min_size(pos2(180.0, 40.0), vec2(430.0, 270.0));
    for pointer in [canvas.min, canvas.max, pos2(-400.0, 1000.0)] {
        for size in [vec2(240.0, 48.0), vec2(1000.0, 1000.0)] {
            assert!(canvas.contains_rect(cable_guidance_rect(canvas, pointer, size)));
        }
    }
    let ctx = egui::Context::default();
    let output = ctx.run(egui::RawInput::default(), |ctx| {
        let painter = ctx.layer_painter(egui::LayerId::background());
        paint_cable_gesture_guidance(
            &painter,
            canvas,
            canvas.max,
            &CableGestureGuidance {
                route: "src · OUT -> target · IN".into(),
                action: "Release: Move".into(),
            },
            true,
        );
    });
    assert!(output
        .shapes
        .iter()
        .all(|shape| canvas.contains_rect(shape.clip_rect)));
    assert!(guidance_text(&output).contains("Release: Move"));
    for shape in output.shapes {
        if let egui::Shape::Text(text) = shape.shape {
            assert_eq!(text.galley.job.sections[0].format.font_id.size, 12.0);
        }
    }
}

#[test]
fn darkroom_keeps_draft_and_badge_readable_while_physical_routes_dim() {
    let patch = stacked_fixture();
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    frame(&ctx, &mut rack, &patch, 0.0, vec![]);
    let from = fixture_jack(&rack, &patch, "target", "signal", true);
    let to = fixture_jack(&rack, &patch, "dest", "signal", true);
    frame(
        &ctx,
        &mut rack,
        &patch,
        1.0,
        vec![egui::Event::PointerMoved(from), button(from, true)],
    );
    frame(
        &ctx,
        &mut rack,
        &patch,
        1.1,
        vec![egui::Event::PointerMoved(to)],
    );
    let (_, lit) = frame(&ctx, &mut rack, &patch, 1.2, vec![]);
    rack.set_room_brightness(0.0);
    let (_, black) = frame(&ctx, &mut rack, &patch, 1.3, vec![]);
    assert_eq!(guidance_text(&lit), guidance_text(&black));
    let text = |output: &egui::FullOutput| {
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text().contains("Esc cancels") => {
                    Some(format!("{text:?}"))
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(text(&lit), text(&black));
    let curves = |output: &egui::FullOutput| {
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::CubicBezier(curve) => Some((curve.points, curve.stroke.color.clone())),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let lit = curves(&lit);
    let black = curves(&black);
    assert_eq!(
        lit.iter().map(|curve| curve.0).collect::<Vec<_>>(),
        black.iter().map(|curve| curve.0).collect::<Vec<_>>()
    );
    assert!(lit.iter().zip(&black).any(|(a, b)| a.1 != b.1));
    assert!(black.iter().any(|(_, color)| matches!(color, egui::epaint::ColorMode::Solid(color) if color.r() > 0 || color.g() > 0 || color.b() > 0)));
}

#[test]
fn active_drag_softens_only_unrelated_routes_without_changing_curve_geometry() {
    let mut patch = stacked_fixture();
    let mut side = patch.cables[0].clone();
    side.id = Some("side".into());
    side.from = "alt".into();
    side.to = "dest".into();
    patch.cables.push(side);
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    frame(&ctx, &mut rack, &patch, 0.0, vec![]);
    let from = fixture_jack(&rack, &patch, "target", "signal", true);
    frame(
        &ctx,
        &mut rack,
        &patch,
        1.0,
        vec![egui::Event::PointerMoved(from), button(from, true)],
    );
    let (_, active) = frame(
        &ctx,
        &mut rack,
        &patch,
        1.1,
        vec![egui::Event::PointerMoved(pos2(1050.0, 650.0))],
    );
    assert!(rack.dragging_cable);
    // Keep exactly the same pending route, pointer and live primary press so
    // only the active-gesture presentation differs from click-to-connect.
    rack.dragging_cable = false;
    let (_, click) = frame(&ctx, &mut rack, &patch, 1.2, vec![]);
    let curves = |output: &egui::FullOutput| {
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::CubicBezier(curve) => {
                    Some((curve.points, curve.stroke.width, curve.stroke.color.clone()))
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let active = curves(&active);
    let click = curves(&click);
    assert_eq!(active.len(), click.len());
    assert!(active
        .iter()
        .zip(&click)
        .all(|(a, b)| a.0 == b.0 && a.1 == b.1));
    assert!(active.iter().zip(&click).any(|(a, b)| a.2 != b.2));
    assert!(active.iter().zip(&click).any(|(a, b)| a.2 == b.2));
    let pending = rack.pending.as_ref().unwrap();
    assert!(cable_gesture_related(&patch.cables[0], pending, None));
    assert!(cable_gesture_related(&patch.cables[1], pending, None));
    assert!(!cable_gesture_related(&patch.cables[2], pending, None));
    assert!(cable_gesture_related(
        &patch.cables[2],
        pending,
        Some(&CableEnd::Input {
            node: "dest".into(),
            port: "signal".into()
        })
    ));
}
