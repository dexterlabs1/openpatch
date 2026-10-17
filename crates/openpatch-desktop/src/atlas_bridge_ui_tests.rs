use super::*;

fn bridge_patch() -> Patch {
    serde_json::from_value(json!({"version":2,"nodes":[{"id":"bridge","kind":"rackmcp_bridge"}]}))
        .unwrap()
}

fn bridge_status(revision: u64, state: &str, save: &str) -> Value {
    json!({"revision":revision,"document_generation":2,
        "transport":{"state":state,"generation":3,"attachment_id":if state == "running" { Some("attachment-3") } else { None },"admitted_clients":2,"stdio_enabled":true,"last_error":null},
        "authority":{"kind":"same_user_shared_mutation","writer_lease":false,"stdio_client_count":null},
        "save":{"state":save,"target":"/work/project.opatch","basis":"native_saved_content","revision":revision},
        "last_operation":{"name":"set_parameter","origin":"mcp","outcome":"accepted","revision":revision}})
}

fn bridge_labels(shape: &egui::Shape, labels: &mut Vec<(String, Rect)>) {
    match shape {
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                bridge_labels(shape, labels);
            }
        }
        egui::Shape::Text(text) => labels.push((
            text.galley.text().into(),
            Rect::from_min_size(text.pos, text.galley.size()),
        )),
        _ => (),
    }
}

fn bridge_texts(output: &egui::FullOutput) -> Vec<(String, Rect)> {
    let mut labels = Vec::new();
    for shape in &output.shapes {
        bridge_labels(&shape.shape, &mut labels);
    }
    labels
}

fn bridge_click(
    ctx: &egui::Context,
    rack: &mut Rack,
    patch: &Patch,
    name: &str,
    time: f64,
) -> Option<RackAction> {
    let target = pos2(24.0, 12.0) + atlas_bridge_control(name).center().to_vec2();
    frame_revision_state(
        ctx,
        rack,
        patch,
        time,
        vec![egui::Event::PointerMoved(target), button(target, true)],
        true,
        true,
        17,
    );
    frame_revision_state(
        ctx,
        rack,
        patch,
        time + 0.01,
        vec![button(target, false)],
        true,
        true,
        17,
    )
    .0
}

#[test]
fn bridge_offline_drawing_is_inert_and_does_not_invent_host_or_audio_status() {
    let ctx = egui::Context::default();
    let patch = bridge_patch();
    patch.compile().unwrap();
    let mut rack = Rack::default();
    let (_, output) = frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 17);
    let labels = bridge_texts(&output);
    assert_eq!(
        labels
            .iter()
            .filter(|(text, _)| text == "HOST UNAVAILABLE")
            .count(),
        3
    );
    for (index, name) in ["start", "stop", "restart", "save", "show"]
        .into_iter()
        .enumerate()
    {
        assert!(
            bridge_click(&ctx, &mut rack, &patch, name, 1.0 + index as f64).is_none(),
            "{name}: offline control was enabled"
        );
    }
    assert!(rack.bridge_ui.status.is_none() && rack.bridge_ui.next_poll.is_none());
    assert!(rack.module_lights.is_empty());
    assert!(patch.nodes[0].params.is_empty());
}

#[test]
fn bridge_native_buttons_emit_explicit_actions_with_observed_revision() {
    for (index, name) in ["start", "stop", "restart", "save", "show"]
        .into_iter()
        .enumerate()
    {
        let ctx = egui::Context::default();
        let patch = bridge_patch();
        let mut rack = Rack::default();
        rack.accept_bridge_status(bridge_status(
            17,
            if name == "start" {
                "stopped"
            } else {
                "running"
            },
            "modified",
        ));
        frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 17);
        let action = bridge_click(&ctx, &mut rack, &patch, name, 1.0 + index as f64);
        let expected = format!("_native_bridge_{name}");
        assert!(
            matches!(action, Some(RackAction::Command(command, ref args)) if command == expected && args == &json!({"id":"bridge","expected_revision":17})),
            "{name}"
        );
        assert_eq!(
            rack.bridge_ui.status.as_ref().unwrap().0["transport"]["state"],
            if name == "start" {
                "stopped"
            } else {
                "running"
            },
            "A click emits a request; drawing never fabricates host application"
        );
    }
}

#[test]
fn bridge_status_changes_update_readiness_authority_save_and_last_operation() {
    let ctx = egui::Context::default();
    let patch = bridge_patch();
    let mut rack = Rack::default();
    rack.accept_bridge_status(bridge_status(17, "running", "modified"));
    let (_, output) = frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 17);
    let labels = bridge_texts(&output);
    for expected in [
        "READY · LOCAL ATTACHMENT",
        "SAME USER · SHARED MUTATION",
        "MODIFIED · SAVE CHANGES",
        "LOCAL CLIENTS 2 · STDIO ENABLED",
        "SAVE TARGET · project.opatch",
        "set_parameter · mcp · accepted",
    ] {
        assert!(
            labels.iter().any(|(text, _)| text == expected),
            "Missing {expected}"
        );
    }
    assert!(bridge_click(&ctx, &mut rack, &patch, "start", 1.0).is_none());
    rack.accept_bridge_status(bridge_status(17, "stopped", "saved"));
    let (_, output) = frame_revision_state(&ctx, &mut rack, &patch, 2.0, vec![], true, true, 17);
    let labels = bridge_texts(&output);
    assert!(labels
        .iter()
        .any(|(text, _)| text == "STOPPED · START TO ATTACH"));
    assert!(labels
        .iter()
        .any(|(text, _)| text == "SAVED · NATIVE CONTENT"));
    assert!(bridge_click(&ctx, &mut rack, &patch, "restart", 3.0).is_none());
}

#[test]
fn bridge_expired_and_wrong_revision_observations_cannot_keep_ready_lamps_or_buttons() {
    let patch = bridge_patch();
    for expired in [false, true] {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        rack.accept_bridge_status(bridge_status(
            if expired { 17 } else { 16 },
            "running",
            "modified",
        ));
        if expired {
            rack.bridge_ui.status.as_mut().unwrap().1 -= std::time::Duration::from_secs(1);
        }
        let (_, output) =
            frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 17);
        assert_eq!(
            bridge_texts(&output)
                .iter()
                .filter(|(text, _)| text == "HOST UNAVAILABLE")
                .count(),
            3
        );
        assert!(bridge_click(&ctx, &mut rack, &patch, "restart", 1.0).is_none());
    }
}

#[test]
fn duplicate_bridge_panels_share_one_global_observation_and_one_poll() {
    let mut patch = bridge_patch();
    let mut duplicate = patch.nodes[0].clone();
    duplicate.id = "bridge-two".into();
    patch.nodes.push(duplicate);
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    rack.accept_bridge_status(bridge_status(17, "running", "saved"));
    let (_, output) = frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 17);
    assert_eq!(
        bridge_texts(&output)
            .iter()
            .filter(|(text, _)| text == "READY · LOCAL ATTACHMENT")
            .count(),
        2
    );
    assert!(rack.bridge_poll_due(&patch, 17));
    assert!(!rack.bridge_poll_due(&patch, 17));
    assert!(!rack.bridge_poll_due(&Patch::default(), 17));
    assert!(rack.bridge_ui.status.is_none());
}

#[test]
fn bridge_shared_preview_status_print_and_controls_fit_multiple_scales() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|module| module["kind"] == "rackmcp_bridge")
        .unwrap();
    for zoom in [0.25, 0.42, 1.0, 1.4] {
        let ctx = egui::Context::default();
        crate::panel_skin::install_fonts(&ctx);
        let mut preview = ModulePreview::new(module);
        preview
            .renderer
            .accept_bridge_status(bridge_status(0, "running", "modified"));
        let bounds = Rect::from_min_size(pos2(24.0, 12.0), preview.size() * zoom);
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 800.0))),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| preview.show(ui, bounds, &catalog));
            },
        );
        let labels = bridge_texts(&output);
        assert_eq!(
            labels
                .iter()
                .filter(|(text, _)| text == "HOST UNAVAILABLE")
                .count(),
            3,
            "Preview must ignore cached real host state"
        );
        for (text, location) in labels {
            assert!(
                bounds.expand(0.5).contains_rect(location),
                "{zoom}: {text} outside panel"
            );
        }
        for name in ["start", "stop", "restart", "save", "show"] {
            let local = atlas_bridge_control(name);
            let control = Rect::from_min_max(
                bounds.min + local.min.to_vec2() * zoom,
                bounds.min + local.max.to_vec2() * zoom,
            );
            assert!(bounds.contains_rect(control));
        }
        assert!(preview.renderer.module_lights.is_empty());
    }
}

#[test]
#[ignore = "writes CPU-only Bridge panel review captures"]
fn bridge_headless_review() {
    let destination = std::path::PathBuf::from(
        std::env::var_os("OPENPATCH_BRIDGE_REVIEW_DIR")
            .expect("OPENPATCH_BRIDGE_REVIEW_DIR must select the review directory"),
    );
    std::fs::create_dir_all(&destination).unwrap();
    let patch = bridge_patch();
    let catalog = openpatch_core::catalog();
    for (name, brightness, state) in [
        ("normal", 1.0, Some("running")),
        ("dim", 0.0, Some("running")),
        ("offline-normal", 1.0, None),
        ("offline-dim", 0.0, None),
    ] {
        let ctx = egui::Context::default();
        crate::panel_skin::install_fonts(&ctx);
        let size = vec2(400.0, PANEL_HEIGHT + 36.0);
        let bounds = Rect::from_min_size(pos2(20.0, 18.0), vec2(360.0, PANEL_HEIGHT));
        let mut rack = Rack { pan: Vec2::ZERO, ..Default::default() };
        rack.set_room_brightness(brightness);
        rack.positions.insert("bridge".into(), Pos2::ZERO);
        let mut capture = crate::headless_review::CpuCapture::default();
        let mut last = None;
        for _ in 0..3 {
            if let Some(state) = state {
                rack.accept_bridge_status(bridge_status(0, state, "modified"));
            }
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE)
                        .show(ctx, |ui| {
                            let rendered = rack.render_modules(
                                ui,
                                &patch,
                                &catalog,
                                bounds,
                                48_000,
                                0,
                                RenderMode::Interactive {
                                    drag_interactive: false,
                                    allow_manual_press: false,
                                },
                            );
                            assert!(rendered.action.is_none());
                        });
                },
            );
            capture.observe(&output);
            last = Some(output);
        }
        capture.save(
            &ctx,
            last.unwrap(),
            size,
            destination.join(format!("bridge-{name}.png")),
        );
    }
}
