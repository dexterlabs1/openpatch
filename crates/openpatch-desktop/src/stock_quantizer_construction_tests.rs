#[test]
fn accepted_stock_quantizer_construction_edits_anchor_actual_keyboard_and_offset_without_preview_activity(
) {
    let workspace = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
    let before = patch("stock_quantizer", "offset", 0.0);
    session.replace_patch(before.clone(), None).unwrap();
    let previous = session.revision();
    let shared = session.into_shared();
    openpatch::control::execute_shared(&shared,"start_construction",json!({"expected_revision":previous,"title":"Shape Stock Quantizer","interval_ms":100,"steps":[{"label":"Major scale and transpose","edits":[{"tool":"set_parameter","arguments":{"id":"target","parameter":"scale_mask","value":2741}},{"tool":"set_parameter","arguments":{"id":"target","parameter":"offset","value":0.25}}]}]})).unwrap();
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
        2
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
    assert_eq!(rack.construction_anchors.len(), 2);
    let keyboard = &rack.construction_anchors[&("target".into(), "scale_mask".into())];
    assert_eq!(
        keyboard.rect,
        Rect::from_min_max(pos2(44.0, 112.0), pos2(304.0, 211.0))
    );
    assert!(keyboard.marker.is_none());
    assert_eq!(keyboard.old_text, "Chromatic");
    assert_eq!(keyboard.new_text, "C D E F G A B");
    let offset = &rack.construction_anchors[&("target".into(), "offset".into())];
    assert!(matches!(offset.marker, Some(Marker::Rotary { .. })));
    assert!(text(&output).iter().any(|label| label == "+3.00 st"));
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "stock_quantizer")
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
    assert!(!text(&output).iter().any(|t| t.contains(" → ")));
    assert!(text(&output).iter().any(|t| t == "PLAYED —"));
    let at = pos2(24.0, 12.0) + vec2(stock_quantizer_key_rect(0).center().x, 181.0);
    let action = frame_revision_state(
        &ctx,
        &mut rack,
        &after,
        0.2,
        vec![egui::Event::PointerMoved(at), button(at, true)],
        true,
        true,
        revision,
    )
    .0;
    assert!(
        matches!(action,Some(RackAction::ScalePaint{arguments,..})if arguments["expected_revision"]==revision&&arguments["value"]==2740)
    );
    assert!(rack
        .construction_animation
        .parameter("target", "stock_quantizer", "scale_mask")
        .is_none());
}
