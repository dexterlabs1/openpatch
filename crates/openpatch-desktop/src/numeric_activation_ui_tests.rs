use super::*;
fn patch() -> Patch {
    serde_json::from_value(json!({"version":2,"nodes":[{"id":"dc","kind":"constant"}]})).unwrap()
}
fn key(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}
fn target() -> Pos2 {
    let (center, radius) = parameter_position(1, 0, panel_width("constant"));
    center + vec2(24.0, 12.0 + radius + 15.0)
}

#[test]
fn ordinary_numeric_click_opens_selected_draft_without_graph_edit_and_cancel_discards() {
    for cancellation in ["escape", "focus", "modal"] {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let patch = patch();
        frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 19);
        let at = target();
        assert!(frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.0,
            vec![egui::Event::PointerMoved(at), button(at, true)],
            true,
            true,
            19
        )
        .0
        .is_none());
        assert!(frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.1,
            vec![button(at, false)],
            true,
            true,
            19
        )
        .0
        .is_none());
        let edit = rack
            .editing
            .as_mut()
            .expect("one click opens numeric editor");
        assert_eq!(
            (&*edit.id, &*edit.parameter, edit.revision),
            ("dc", "value", 19)
        );
        edit.text = "3.25".into();
        let (events, focused, enabled) = match cancellation {
            "escape" => (vec![key(egui::Key::Escape)], true, true),
            "focus" => (vec![egui::Event::WindowFocused(false)], false, true),
            _ => (vec![], true, false),
        };
        assert!(
            frame_revision_state(&ctx, &mut rack, &patch, 1.2, events, focused, enabled, 19)
                .0
                .is_none()
        );
        assert!(rack.editing.is_none(), "{cancellation}");
        assert!(patch.nodes[0].params.is_empty());
    }
}

#[test]
fn focused_numeric_enter_and_space_activate_without_same_key_commit_or_host_shortcut() {
    for activation in [egui::Key::Enter, egui::Key::Space] {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let patch = patch();
        frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 23);
        let mut action = None;
        let mut host_sees_key = true;
        let mut focused_value = false;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 700.0))),
                time: Some(0.1),
                focused: true,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| {
                        ui.add_enabled_ui(true, |ui| {
                            // Rack renders modules directly in this UI.
                            let id = ui.id().with(("value", "dc", "value"));
                            ctx.memory_mut(|m| m.request_focus(id));
                            rack.show(ui, &patch, &openpatch_core::catalog(), 48000, 23);
                            focused_value = ctx.memory(|m| m.has_focus(id));
                        });
                    });
            },
        );
        assert!(focused_value, "value widget must own keyboard focus");
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 700.0))),
                time: Some(1.0),
                focused: true,
                events: vec![key(activation)],
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| {
                        ui.add_enabled_ui(true, |ui| {
                            action = rack.show(ui, &patch, &openpatch_core::catalog(), 48000, 23);
                        });
                    });
                host_sees_key = ctx.input(|i| i.key_pressed(activation));
            },
        );
        assert!(
            action.is_none(),
            "activation must not immediately apply the draft"
        );
        assert!(
            !host_sees_key,
            "activation must not reach library/transport shortcuts"
        );
        assert_eq!(
            rack.editing
                .as_ref()
                .expect("keyboard opens editor")
                .parameter,
            "value"
        );
        rack.editing.as_mut().unwrap().text = "3.25".into();
        // Release the activation key before Enter commits the independent draft.
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.1,
            vec![egui::Event::Key {
                key: activation,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            true,
            true,
            23,
        );
        assert!(
            matches!(frame_revision_state(&ctx,&mut rack,&patch,1.2,vec![key(egui::Key::Enter)],true,true,23).0,Some(RackAction::Command("set_parameter",args)) if args["id"]=="dc" && args["parameter"]=="value" && args["value"]==3.25 && args["expected_revision"]==23)
        );
    }
}

#[test]
fn ordinary_numeric_draft_cannot_commit_after_remote_revision_change() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch = patch();
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 4);
    let at = target();
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.0,
        vec![egui::Event::PointerMoved(at), button(at, true)],
        true,
        true,
        4,
    );
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.1,
        vec![button(at, false)],
        true,
        true,
        4,
    );
    rack.editing.as_mut().unwrap().text = "5".into();
    assert!(frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.2,
        vec![key(egui::Key::Enter)],
        true,
        true,
        5
    )
    .0
    .is_none());
    assert!(rack.editing.as_ref().unwrap().stale);
}
