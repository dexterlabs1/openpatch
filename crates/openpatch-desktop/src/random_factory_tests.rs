use super::*;

fn random_factory_patch() -> Patch {
    let mut patch: Patch =
        serde_json::from_str(include_str!("../../../examples/random-motion-voice.json")).unwrap();
    patch.assign_missing_cable_ids().unwrap();
    patch
}

#[test]
fn random_factory_routes_all_five_outputs_and_produces_repeatable_stereo_audio() {
    let patch = random_factory_patch();
    let outputs: std::collections::BTreeSet<_> = patch
        .cables
        .iter()
        .filter(|c| c.from == "motion")
        .map(|c| c.output.as_str())
        .collect();
    assert_eq!(
        outputs,
        ["stepped", "linear", "smooth", "exponential", "trigger"]
            .into_iter()
            .collect()
    );
    let mut first = patch.compile().unwrap();
    let mut second = patch.compile().unwrap();
    let mut left = vec![[0.0; 2]; 24_000];
    let mut right = vec![[0.0; 2]; left.len()];
    first.process_audio_block(&mut left);
    second.process_audio_block(&mut right);
    assert_eq!(
        left, right,
        "saved seed must reproduce a fresh factory render"
    );
    assert!(left.iter().flatten().all(|v| v.is_finite()));
    assert!(left.iter().flatten().any(|v| v.abs() > 0.001));
    assert!(left.iter().all(|v| v[0] == v[1]));
    let scope = first
        .module_scope(first.resolve_module_scope("scope").unwrap())
        .unwrap();
    assert_eq!((scope.x_channels, scope.y_channels), (1, 1));
    assert!(scope.x[0].iter().any(|p| p.max.abs() > 0.001));
    assert!(scope.y[0].iter().any(|p| p.max.abs() > 0.001));
}

#[test]
fn random_factory_layout_and_actual_snapshot_fit_both_room_modes() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = host(directory.path());
    app.replace_now(random_factory_patch(), "Random motion voice");
    let mut expected = Rack::default();
    expected
        .restore_layout(&OpenPatch::random_motion_layout())
        .unwrap();
    assert_eq!(app.rack.layout(), expected.layout());
    let layout = app.rack.layout();
    for (index, node) in app.patch.nodes.iter().enumerate() {
        let position = &layout["positions"][&node.id];
        let x = position[0].as_f64().unwrap();
        let y = position[1].as_f64().unwrap();
        for other in app.patch.nodes.iter().skip(index + 1) {
            let position = &layout["positions"][&other.id];
            let other_x = position[0].as_f64().unwrap();
            let other_y = position[1].as_f64().unwrap();
            assert!(
                x + f64::from(crate::rack::module_width(node)) <= other_x
                    || other_x + f64::from(crate::rack::module_width(other)) <= x
                    || y + f64::from(crate::rack::PANEL_HEIGHT) <= other_y
                    || other_y + f64::from(crate::rack::PANEL_HEIGHT) <= y,
                "{} overlaps {}",
                node.id,
                other.id
            );
        }
    }
    app.prepare_random_fixture();
    assert!(app.audio.is_none());
    assert_eq!(
        app.patch.reference_view(),
        random_factory_patch().reference_view()
    );
    let revision = app.revision;
    let ctx = egui::Context::default();
    crate::panel_skin::install_fonts(&ctx);
    for brightness in [1.0, 0.0] {
        app.rack.set_room_brightness(brightness);
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1440.0, 960.0),
                )),
                focused: true,
                ..Default::default()
            },
            |ctx| app.rack_workspace(ctx, None),
        );
        for label in ["RANDOM", "ADSR EG", "SCOPE"] {
            preset_text(&output, label);
        }
        assert_eq!(app.revision, revision);
    }
}
