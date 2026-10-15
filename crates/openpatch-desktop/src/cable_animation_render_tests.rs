fn construction_test_snapshot(
    before: &Patch,
    after: &Patch,
    previous: u64,
    revision: u64,
) -> Value {
    let cables = after
        .cables
        .iter()
        .filter(|c| !before.cables.iter().any(|old| old.id == c.id))
        .map(|c| json!({"id":c.id,"from":c.from,"output":c.output,"to":c.to,"input":c.input}))
        .collect::<Vec<_>>();
    json!({"plan_id":1,"completed_steps":1,"current_revision":revision,"last_revision":revision,"last_commit":{"age_ms":0,"invalidate_existing_cables":false,"plan_id":1,"step_index":0,"before_revision":previous,"revision":revision,"label":"Connect","new_modules":[],"new_cables":cables,"parameter_changes":[]}})
}
// Included in rack::tests to exercise production geometry and input handling.
#[test]
fn construction_reveal_is_an_exact_prefix_of_the_committed_curve() {
    let evaluate = |p: [Pos2; 4], t: f32| {
        let s = 1.0 - t;
        (p[0].to_vec2() * s.powi(3)
            + p[1].to_vec2() * (3.0 * s * s * t)
            + p[2].to_vec2() * (3.0 * s * t * t)
            + p[3].to_vec2() * t.powi(3))
        .to_pos2()
    };
    for zoom in [0.25, 1.0, 2.0] {
        for fan in [-20.0, 0.0, 20.0] {
            let points = cable_curve_points(pos2(80.0, 400.0), pos2(700.0, 100.0), zoom, fan, 0.5);
            for progress in [0.0, 0.1, 0.5, 0.9, 1.0] {
                let prefix = cable_curve_prefix(points, progress);
                for u in [0.0, 0.2, 0.7, 1.0] {
                    assert!(evaluate(prefix, u).distance(evaluate(points, u * progress)) < 0.001);
                }
            }
        }
    }
}

#[test]
fn construction_paint_keeps_the_full_committed_route_and_returns_to_static_geometry() {
    let draw = |progress: Option<f32>, decorated: bool| {
        let ctx = egui::Context::default();
        ctx.run(egui::RawInput::default(), |ctx| {
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Middle,
                egui::Id::new("test"),
            ));
            if decorated {
                paint_committed_cable_curves(
                    &painter,
                    pos2(10.0, 20.0),
                    pos2(300.0, 70.0),
                    CABLES[0],
                    1.0,
                    true,
                    20.0,
                    progress,
                    crate::cable_presentation::CablePresentation::default(),
                );
            } else {
                paint_cable_curves(
                    &painter,
                    pos2(10.0, 20.0),
                    pos2(300.0, 70.0),
                    CABLES[0],
                    1.0,
                    true,
                    20.0,
                    crate::cable_presentation::CablePresentation::default(),
                );
            }
        })
        .shapes
        .into_iter()
        .map(|shape| shape.shape)
        .collect::<Vec<_>>()
    };
    let static_shapes = draw(None, false);
    assert_eq!(draw(None, true), static_shapes);
    let revealing = draw(Some(0.4), true);
    assert_eq!(revealing.len(), 7);
    let egui::Shape::CubicBezier(full) = &revealing[1] else {
        panic!("missing full route")
    };
    let egui::Shape::CubicBezier(prefix) = &revealing[4] else {
        panic!("missing reveal prefix")
    };
    let egui::Shape::CubicBezier(original) = &static_shapes[1] else {
        panic!("missing static route")
    };
    assert_eq!(full.points, original.points);
    assert_eq!(full.points[0], prefix.points[0]);
    assert!(full.points[3].distance(prefix.points[3]) > 10.0);
    assert_eq!(prefix.stroke.width, original.stroke.width);
}

#[test]
fn construction_animation_requires_an_exact_observed_commit_and_clears_on_human_or_skipped_edits() {
    let after = stacked_fixture();
    let mut before = after.clone();
    before.cables.clear();
    let mut rack = Rack::default();
    let now = std::time::Instant::now();
    rack.observe_construction_transition(
        &before,
        &after,
        9,
        10,
        &construction_test_snapshot(&before, &after, 9, 10),
        true,
    );
    assert!(rack
        .construction_animation
        .progress(&after.cables[0], now)
        .is_some());
    // Human edit/undo: current revision is no longer the construction receipt.
    rack.observe_construction_transition(
        &after,
        &after,
        10,
        11,
        &construction_test_snapshot(&before, &after, 9, 10),
        true,
    );
    assert!(rack
        .construction_animation
        .progress(&after.cables[0], now)
        .is_none());
    // Missing intermediate frames never triggers a replay of their additions.
    rack.observe_construction_transition(
        &before,
        &after,
        11,
        14,
        &construction_test_snapshot(&before, &after, 11, 14),
        true,
    );
    assert!(rack
        .construction_animation
        .progress(&after.cables[0], now)
        .is_none());
    rack.observe_construction_transition(
        &before,
        &after,
        14,
        15,
        &construction_test_snapshot(&before, &after, 14, 15),
        false,
    );
    assert!(rack
        .construction_animation
        .progress(&after.cables[0], now)
        .is_none());
    // Re-enabling while observing an unchanged graph must not revive old cables.
    rack.observe_construction_transition(
        &after,
        &after,
        15,
        16,
        &construction_test_snapshot(&before, &after, 15, 16),
        true,
    );
    assert!(rack
        .construction_animation
        .progress(&after.cables[0], now)
        .is_none());
}

#[test]
fn construction_cables_keep_connected_jack_pickup_and_guarded_editing() {
    let after = stacked_fixture();
    let mut before = after.clone();
    before.cables.clear();
    let mut rack = Rack::default();
    let ctx = egui::Context::default();
    frame_revision_state(&ctx, &mut rack, &before, 0.0, vec![], true, true, 9);
    rack.observe_construction_transition(
        &before,
        &after,
        9,
        10,
        &construction_test_snapshot(&before, &after, 9, 10),
        true,
    );
    frame_revision_state(&ctx, &mut rack, &after, 0.1, vec![], true, true, 10);
    let from = fixture_jack(&rack, &after, "target", "signal", true);
    let to = fixture_jack(&rack, &after, "dest", "signal", true);
    frame_revision_state(
        &ctx,
        &mut rack,
        &after,
        0.2,
        vec![egui::Event::PointerMoved(from), button(from, true)],
        true,
        true,
        10,
    );
    frame_revision_state(
        &ctx,
        &mut rack,
        &after,
        0.3,
        vec![egui::Event::PointerMoved(to)],
        true,
        true,
        10,
    );
    let action = frame_revision_state(
        &ctx,
        &mut rack,
        &after,
        0.4,
        vec![button(to, false)],
        true,
        true,
        10,
    )
    .0;
    let Some(RackAction::Command(tool, args)) = action else {
        panic!("connected jack did not produce an edit")
    };
    let mut session = openpatch::control::Session::new(&std::env::temp_dir()).unwrap();
    session.replace_patch(after.clone(), None).unwrap();
    // The real GUI action keeps its captured revision instead of bypassing the
    // shared guard. Verify payload and use that guard in a matching session.
    assert_eq!(args["expected_revision"], 10);
    let mut args = args;
    args["expected_revision"] = json!(session.revision());
    session.execute(tool, args).unwrap();
    assert_eq!(session.patch().cables.len(), 2);
    assert!(session.patch().cables.iter().any(|c| c.to == "dest"));
    assert!(session.patch().cables.iter().any(|c| c.to == "target"));
}
