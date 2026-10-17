use super::*;

fn listen_patch() -> Patch {
    serde_json::from_value(json!({"version":2,"nodes":[{"id":"tap","kind":"rackmcp_listen"}]}))
        .unwrap()
}

fn labels(shape: &egui::Shape, result: &mut Vec<(String, Rect)>) {
    match shape {
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                labels(shape, result);
            }
        }
        egui::Shape::Text(text) => result.push((
            text.galley.text().into(),
            Rect::from_min_size(text.pos, text.galley.size()),
        )),
        _ => {}
    }
}

fn click(
    ctx: &egui::Context,
    rack: &mut Rack,
    patch: &Patch,
    name: &str,
    time: f64,
) -> Option<RackAction> {
    let target = pos2(24.0, 12.0) + atlas_listen_control(name).center().to_vec2();
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
fn listen_unavailable_is_distinct_from_silence_and_cannot_start_capture() {
    let ctx = egui::Context::default();
    crate::panel_skin::install_fonts(&ctx);
    let patch = listen_patch();
    patch.compile().unwrap();
    let mut rack = Rack::default();
    let (_, output) = frame_output(&ctx, &mut rack, &patch, 0.0, vec![]);
    let mut printed = Vec::new();
    for shape in output.shapes {
        labels(&shape.shape, &mut printed);
    }
    assert!(printed.iter().any(|(text, _)| text == "WAITING FOR AUDIO"));
    assert!(printed
        .iter()
        .any(|(text, _)| text == "INPUT STATUS UNAVAILABLE"));
    assert!(click(&ctx, &mut rack, &patch, "capture", 1.0).is_none());
    assert!(rack.listen_ui.capture.is_none());
    assert!(rack.module_lights.is_empty());
    assert!(patch.nodes[0].params.is_empty());
}

#[test]
fn listen_capture_and_cancel_emit_real_revision_guarded_commands_with_explicit_wav() {
    let ctx = egui::Context::default();
    let patch = listen_patch();
    let mut rack = Rack::default();
    rack.accept_listen_receipt("inspect_listen", json!({"id":"tap","available":true,"listen":{"frames_seen":100,"dropped":0,"left_connected":true,"right_connected":false,"capturing":false}}));
    frame(&ctx, &mut rack, &patch, 0.0, vec![]);
    assert!(click(&ctx, &mut rack, &patch, "wav", 1.0).is_none());
    let action = click(&ctx, &mut rack, &patch, "capture", 2.0);
    assert!(
        matches!(action, Some(RackAction::Command("start_listen_capture", ref args)) if args == &json!({"id":"tap","seconds":1.0,"write_wav":true,"expected_revision":17}))
    );
    rack.accept_listen_receipt("start_listen_capture", json!({"id":"tap","capture":{"job":"listen-7","state":"capturing","phase":"arming","target_frames":48000,"collected_frames":0}}));
    let action = click(&ctx, &mut rack, &patch, "capture", 3.0);
    assert!(
        matches!(action, Some(RackAction::Command("cancel_listen_capture", ref args)) if args == &json!({"job":"listen-7","expected_revision":17}))
    );
    rack.accept_listen_receipt("cancel_listen_capture", json!({"capture":{"id":"tap","job":"listen-7","state":"capturing","cancel_requested":true}}));
    rack.accept_listen_receipt(
        "inspect_listen_capture",
        json!({"capture":{"id":"tap","job":"listen-7","state":"capturing","phase":"collecting"}}),
    );
    assert_eq!(
        rack.listen_ui.capture.as_ref().unwrap()["cancel_requested"],
        true
    );
    assert_eq!(
        rack.listen_ui.capture.as_ref().unwrap()["state"],
        "capturing"
    );
}

#[test]
fn listen_polling_observes_real_source_and_active_job_then_stops_terminal_job_polling() {
    let patch = listen_patch();
    let mut rack = Rack::default();
    rack.accept_listen_receipt(
        "start_listen_capture",
        json!({"id":"tap","capture":{"job":"listen-3","state":"capturing"}}),
    );
    let commands = rack.listen_poll_commands(&patch, 9);
    assert_eq!(
        commands,
        vec![
            ("inspect_listen", json!({"id":"tap","expected_revision":9})),
            (
                "inspect_listen_capture",
                json!({"job":"listen-3","expected_revision":9})
            )
        ]
    );
    assert!(rack.listen_poll_commands(&patch, 9).is_empty());
    rack.accept_listen_receipt("inspect_listen_capture", json!({"capture":{"id":"tap","job":"listen-3","state":"failed","result":{"error":"no audio frames arrived for 2 s"}}}));
    rack.listen_ui.next_poll = None;
    assert_eq!(rack.listen_poll_commands(&patch, 9).len(), 1);
    assert!(rack.listen_ui.capture.as_ref().unwrap()["result"]["error"]
        .as_str()
        .unwrap()
        .contains("no audio frames"));
    let rows = listen_analysis_rows(
        &json!({"channels":[{"present":false},{"present":false}],"loudnessValid":false,"tempoValid":false,"pitchValid":false}),
    );
    assert!(rows.iter().any(|row| row == "Pitch · unavailable"));
    assert!(!rows.iter().any(|row| row.contains("0.00 Hz")));
}

#[test]
fn listen_shared_preview_print_and_four_large_ports_fit_multiple_scales() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|module| module["kind"] == "rackmcp_listen")
        .unwrap();
    for zoom in [0.25, 0.42, 1.0, 1.4] {
        let ctx = egui::Context::default();
        crate::panel_skin::install_fonts(&ctx);
        let mut preview = ModulePreview::new(module);
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
        let mut printed = Vec::new();
        for shape in output.shapes {
            labels(&shape.shape, &mut printed);
        }
        assert!(printed.iter().any(|(text, _)| text == "RACKMCP LISTEN"));
        assert!(printed.iter().any(|(text, _)| text == "WAITING FOR AUDIO"));
        for (text, location) in &printed {
            assert!(
                bounds.expand(0.5).contains_rect(*location),
                "{zoom}: {text} outside panel"
            );
        }
        for input in [true, false] {
            for index in 0..2 {
                let center = bounds.min + atlas_listen_port(input, index).to_vec2() * zoom;
                let radius =
                    if input { 10.3 } else { 13.0 } * port_material_scale("rackmcp_listen") * zoom;
                assert!(
                    bounds.contains_rect(Rect::from_center_size(center, Vec2::splat(radius * 2.0)))
                );
                for (text, location) in &printed {
                    assert!(
                        location.distance_to_pos(center) >= radius - 0.1 * zoom,
                        "{zoom}: {text} touches socket"
                    );
                }
            }
        }
        assert!(preview.renderer.listen_ui.statuses.is_empty());
        assert!(preview.renderer.listen_ui.capture.is_none());
    }
}

#[test]
#[ignore = "writes CPU-only Listen panel review captures"]
fn listen_headless_review() {
    let destination = std::path::PathBuf::from(
        std::env::var("OPENPATCH_LISTEN_REVIEW_DIR")
            .unwrap_or_else(|_| "native-test-output/rackmcp-listen/panels".into()),
    );
    std::fs::create_dir_all(&destination).unwrap();
    let patch = listen_patch();
    struct Armed;
    impl openpatch_core::ListenSink for Armed {
        fn armed(&self) -> bool { true }
        fn push(&mut self, _: openpatch_core::ListenFrame) -> bool { true }
    }
    let mut engine = patch.compile().unwrap();
    let handle = engine.resolve_module_state("tap").unwrap();
    engine.process_audio_block_with_listen_capture(&mut [[0.0; 2]; 256], handle, &mut Armed);
    let actual = engine.listen_status(handle).unwrap();
    assert!(actual.capturing);
    for (name, brightness, armed) in [("normal", 1.0, false), ("dim", 0.0, false), ("armed-normal", 1.0, true), ("armed-dim", 0.0, true)] {
        let ctx = egui::Context::default();
        crate::panel_skin::install_fonts(&ctx);
        let mut capture = crate::headless_review::CpuCapture::default();
        let mut rack = Rack::default();
        rack.set_room_brightness(brightness);
        if armed {
            rack.accept_listen_receipt("inspect_listen", json!({"id":"tap","available":true,"frame":256,"listen":actual}));
        }
        let mut last = None;
        for frame_index in 0..3 {
            let (_, output) = frame_output(&ctx, &mut rack, &patch, f64::from(frame_index), vec![]);
            capture.observe(&output);
            last = Some(output);
        }
        capture.save(
            &ctx,
            last.unwrap(),
            vec2(410.0, 440.0),
            destination.join(format!("listen-rack-{name}.png")),
        );
    }
}

#[test]
fn native_inspector_discovers_a_capture_started_by_an_external_mcp_client() {
    let mut rack = Rack::default();
    rack.accept_listen_receipt("inspect_listen", json!({"id":"tap","available":true,"listen":{"capturing":true},"capture":{"id":"tap","job":"listen-9","state":"capturing","phase":"collecting"}}));
    assert_eq!(rack.listen_ui.capture.as_ref().unwrap()["job"], "listen-9");
    assert!(rack.listen_poll_commands(&listen_patch(), 7).iter().any(|(tool,args)| *tool == "inspect_listen_capture" && args["job"] == "listen-9"));
}
