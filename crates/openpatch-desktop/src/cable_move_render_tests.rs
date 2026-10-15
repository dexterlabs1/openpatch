// Included in rack::tests: exercise the actual rack, shared painter and input.
fn move_fixture() -> (Patch, Patch) {
    let before = stacked_fixture();
    let mut after = before.clone();
    after.cables[0].from = "alt".into();
    after.cables[0].to = "dest".into();
    (before, after)
}
fn move_receipt(before: &Patch, after: &Patch, previous: u64, revision: u64) -> Value {
    let route = |c: &Cable| json!({"from":c.from,"output":c.output,"to":c.to,"input":c.input});
    json!({"last_move":{"before_revision":previous,"revision":revision,"age_ms":0,"moved_cables":[{"id":"one","before":route(&before.cables[0]),"after":route(&after.cables[0])}]}})
}
fn observe_move(
    rack: &mut Rack,
    before: &Patch,
    after: &Patch,
    snapshot: &Value,
    now: std::time::Instant,
) {
    rack.observe_construction_transition_at(
        before,
        after,
        9,
        10,
        snapshot,
        crate::construction_motion::MotionMode::Full,
        now,
    );
}
#[test]
fn cable_move_receipt_is_exact_deduplicated_age_bounded_and_human_revision_specific() {
    let now = std::time::Instant::now();
    let (before, after) = move_fixture();
    let receipt = move_receipt(&before, &after, 9, 10);
    let original = serde_json::to_value(&after).unwrap();
    let mut rack = Rack::default();
    observe_move(&mut rack, &before, &after, &receipt, now);
    assert_eq!(rack.construction_effect_counts_at(now), (0, 1, 0));
    let first = rack
        .construction_animation
        .cables
        .movement(&after.cables[0], now)
        .unwrap()
        .2;
    assert_eq!(first, 0.0);
    observe_move(
        &mut rack,
        &before,
        &after,
        &receipt,
        now + std::time::Duration::from_millis(210),
    );
    assert_eq!(
        rack.construction_animation
            .cables
            .movement(
                &after.cables[0],
                now + std::time::Duration::from_millis(210)
            )
            .unwrap()
            .2,
        0.5
    );
    rack.clear_construction_animation();
    observe_move(
        &mut rack,
        &before,
        &after,
        &receipt,
        now + std::time::Duration::from_millis(220),
    );
    assert_eq!(
        rack.construction_effect_counts_at(now),
        (0, 0, 0),
        "consumed receipt cannot replay after takeover"
    );
    for suppressed in [10, 11] {
        let mut rack = Rack::default();
        rack.suppress_cable_move_revision(suppressed);
        observe_move(&mut rack, &before, &after, &receipt, now);
        assert_eq!(
            rack.construction_animation.cables.len(),
            usize::from(suppressed != 10)
        );
    }
    for bad in [
        "route",
        "identity",
        "age",
        "gap",
        "duplicate",
        "oversized",
        "missing",
    ] {
        let mut receipt = receipt.clone();
        let mut after = after.clone();
        match bad {
            "route" => receipt["last_move"]["moved_cables"][0]["before"]["input"] = json!("other"),
            "identity" => {
                after.nodes.iter_mut().find(|n| n.id == "src").unwrap().kind = "noise".into()
            }
            "age" => receipt["last_move"]["age_ms"] = json!(420),
            "gap" => receipt["last_move"]["before_revision"] = json!(8),
            "duplicate" => {
                let m = receipt["last_move"]["moved_cables"][0].clone();
                receipt["last_move"]["moved_cables"]
                    .as_array_mut()
                    .unwrap()
                    .push(m);
            }
            "oversized" => {
                let m = receipt["last_move"]["moved_cables"][0].clone();
                receipt["last_move"]["moved_cables"] = json!(vec![m; 129]);
            }
            _ => receipt = json!({}),
        }
        let mut rack = Rack::default();
        observe_move(&mut rack, &before, &after, &receipt, now);
        assert_eq!(rack.construction_animation.cables.len(), 0, "{bad}");
    }
    assert_eq!(serde_json::to_value(&after).unwrap(), original);
}
#[test]
fn cable_move_paced_receipt_clears_old_cues_then_installs_shared_budget_moves() {
    let (before, after) = move_fixture();
    let now = std::time::Instant::now();
    let direct = move_receipt(&before, &after, 9, 10);
    let mut paced = construction_test_snapshot(&before, &after, 9, 10);
    paced["last_commit"]["invalidate_existing_cables"] = json!(true);
    paced["last_commit"]["moved_cables"] = direct["last_move"]["moved_cables"].clone();
    paced["last_move"] = direct["last_move"].clone();
    let mut rack = Rack::default();
    rack.construction_animation
        .cables
        .observe(&Patch::new_v2(), &before, now);
    assert_eq!(rack.construction_animation.cables.len(), 2);
    observe_move(&mut rack, &before, &after, &paced, now);
    assert_eq!(
        rack.construction_animation.cables.len(),
        1,
        "invalidated arrivals must not survive a move"
    );
    assert!(rack
        .construction_animation
        .cables
        .movement(&after.cables[0], now)
        .is_some());
    // Matching paced receipt wins, even if its shared budget excluded moves.
    paced["last_commit"]["moved_cables"] = json!([]);
    let mut rack = Rack::default();
    observe_move(&mut rack, &before, &after, &paced, now);
    assert_eq!(rack.construction_animation.cables.len(), 0);
}
#[test]
fn cable_move_geometry_unplugs_travels_settles_and_tracks_view_for_each_endpoint() {
    for zoom in [0.25, 0.5, 1.0, 2.0] {
        for (output_moving, input_moving) in [(false, true), (true, false), (true, true)] {
            let old_from = pos2(40.0, 100.0);
            let old_to = pos2(100.0, 200.0);
            let from = if output_moving {
                pos2(300.0, 80.0)
            } else {
                old_from
            };
            let to = if input_moving {
                pos2(400.0, 240.0)
            } else {
                old_to
            };
            let get = |t| {
                cable_move_geometry(
                    old_from,
                    old_to,
                    from,
                    to,
                    -20.0,
                    20.0,
                    zoom,
                    t,
                    output_moving,
                    input_moving,
                )
            };
            let start = get(0.0);
            assert_eq!(start.from, old_from);
            assert_eq!(start.to, old_to);
            let unplug = get(55.0 / 420.0);
            if output_moving {
                assert!((unplug.from.distance(old_from) - 9.0 * zoom).abs() < 0.001);
            } else {
                assert_eq!(unplug.from, from);
            }
            if input_moving {
                assert!((unplug.to.distance(old_to) - 9.0 * zoom).abs() < 0.001);
            } else {
                assert_eq!(unplug.to, to);
            }
            let middle = get(0.5);
            assert!(middle.fan > -20.0 && middle.fan < 20.0);
            let end = get(1.0);
            assert_eq!(end.from, from);
            assert_eq!(end.to, to);
            assert_eq!(end.fan, 20.0);
            assert_eq!(end.settle, 1.0);
            let pan = vec2(80.0, 45.0);
            let transformed = cable_move_geometry(
                (old_from.to_vec2() * zoom + pan).to_pos2(),
                (old_to.to_vec2() * zoom + pan).to_pos2(),
                (from.to_vec2() * zoom + pan).to_pos2(),
                (to.to_vec2() * zoom + pan).to_pos2(),
                -20.0,
                20.0,
                zoom,
                0.5,
                output_moving,
                input_moving,
            );
            let world = cable_move_geometry(
                old_from,
                old_to,
                from,
                to,
                -20.0,
                20.0,
                1.0,
                0.5,
                output_moving,
                input_moving,
            );
            assert!(
                transformed
                    .from
                    .distance((world.from.to_vec2() * zoom + pan).to_pos2())
                    < 0.001
            );
            assert!(
                transformed
                    .to
                    .distance((world.to.to_vec2() * zoom + pan).to_pos2())
                    < 0.001
            );
        }
    }
}
#[test]
fn cable_move_actual_paint_keeps_final_route_bus_tension_and_passive_ghost_plugs() {
    for zoom in [0.25, 1.0, 2.0] {
        for tension in [0.0, 0.5, 1.0] {
            for bus in [false, true] {
                let ctx = egui::Context::default();
                let from = pos2(300.0, 100.0);
                let to = pos2(500.0, 200.0);
                let m = cable_move_geometry(
                    pos2(10.0, 20.0),
                    pos2(90.0, 150.0),
                    from,
                    to,
                    -20.0,
                    20.0,
                    zoom,
                    0.5,
                    true,
                    true,
                );
                let output = ctx.run(egui::RawInput::default(), |ctx| {
                    let painter = ctx.layer_painter(egui::LayerId::new(
                        egui::Order::Middle,
                        egui::Id::new("move-paint"),
                    ));
                    let lighting = crate::lighting::RoomLighting::default();
                    let start = lighting.begin(&painter);
                    paint_moving_cable_curves(
                        &painter,
                        from,
                        to,
                        CABLES[0],
                        zoom,
                        bus,
                        20.0,
                        m,
                        crate::cable_presentation::CablePresentation {
                            tension,
                            ..Default::default()
                        },
                    );
                    lighting.finish(&painter, start);
                });
                let curves = output
                    .shapes
                    .iter()
                    .filter_map(|s| {
                        if let egui::Shape::CubicBezier(b) = &s.shape {
                            Some(b)
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>();
                assert_eq!(curves.len(), 6);
                assert_eq!(
                    curves[1].points,
                    cable_curve_points(from, to, zoom, 20.0, tension)
                );
                assert_eq!(
                    curves[4].points,
                    cable_curve_points(m.from, m.to, zoom, m.fan, tension)
                );
                assert_eq!(curves[1].stroke.width, curves[4].stroke.width);
                // Passive plug lenses, no port-signal halos or moving emissions.
                assert_eq!(output.shapes.iter().filter(|s|matches!(&s.shape,egui::Shape::Circle(c) if c.radius==2.0*zoom && c.center==m.to && c.fill==Color32::from_rgb(34,35,33))).count(),1);
            }
        }
    }
}
#[test]
fn cable_move_actual_rack_targets_current_route_and_hover_takes_over_ghost_immediately() {
    let (before, after) = move_fixture();
    let snapshot = move_receipt(&before, &after, 9, 10);
    let now = std::time::Instant::now();
    for ghost in [false, true] {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        frame_revision_state(&ctx, &mut rack, &before, 0.0, vec![], true, true, 9);
        observe_move(&mut rack, &before, &after, &snapshot, now);
        rack.set_construction_clock_override(Some(now + std::time::Duration::from_millis(210)));
        frame_revision_state(&ctx, &mut rack, &after, 0.1, vec![], true, true, 10);
        assert!(rack
            .construction_animation
            .cables
            .movement(&after.cables[0], now)
            .is_some());
        let destination = fixture_jack(&rack, &after, "dest", "signal", true);
        let pointer = if ghost {
            let old_from = fixture_jack(&rack, &after, "src", "out", false);
            let old_to = fixture_jack(&rack, &after, "target", "signal", true);
            let from = fixture_jack(&rack, &after, "alt", "out", false);
            cable_move_geometry(
                old_from,
                old_to,
                from,
                destination,
                -20.0,
                0.0,
                rack.zoom,
                0.5,
                true,
                true,
            )
            .to
        } else {
            destination
        };
        let action = frame_revision_state(
            &ctx,
            &mut rack,
            &after,
            0.2,
            vec![egui::Event::PointerMoved(pointer)],
            true,
            true,
            10,
        )
        .0;
        assert!(action.is_none());
        assert_eq!(
            rack.construction_animation.cables.len(),
            0,
            "hover must cancel decorative ghost={ghost}"
        );
        assert!(rack.pending.is_none());
        frame_revision_state(
            &ctx,
            &mut rack,
            &after,
            0.3,
            vec![
                egui::Event::PointerMoved(destination),
                button(destination, true),
            ],
            true,
            true,
            10,
        );
        // A click pickup completes on release, as in the real jack interaction.
        frame_revision_state(
            &ctx,
            &mut rack,
            &after,
            0.4,
            vec![button(destination, false)],
            true,
            true,
            10,
        );
        let pending = rack
            .pending
            .as_ref()
            .expect("actual committed destination must remain a pickup target");
        assert_eq!(
            pending.original.as_ref().unwrap().id.as_deref(),
            Some("one")
        );
    }
}
#[test]
fn cable_move_reduced_is_destination_only_off_is_static_and_expired_never_replays() {
    let (before, after) = move_fixture();
    let snapshot = move_receipt(&before, &after, 9, 10);
    let now = std::time::Instant::now();
    for mode in [
        crate::construction_motion::MotionMode::Reduced,
        crate::construction_motion::MotionMode::Off,
    ] {
        let mut rack = Rack::default();
        rack.observe_construction_transition_at(&before, &after, 9, 10, &snapshot, mode, now);
        assert!(rack
            .construction_animation
            .cables
            .movement(&after.cables[0], now)
            .is_none());
        let phase = rack
            .construction_animation
            .cables
            .phase(&after.cables[0], now);
        if mode == crate::construction_motion::MotionMode::Reduced {
            let p = phase.unwrap();
            assert!(p.reduced);
            assert_eq!(p.progress, 1.0);
        } else {
            assert!(phase.is_none());
        }
        rack.construction_animation
            .tick(now + std::time::Duration::from_secs(30));
        assert_eq!(rack.construction_animation.cables.len(), 0);
        rack.observe_construction_transition_at(
            &before,
            &after,
            9,
            10,
            &snapshot,
            crate::construction_motion::MotionMode::Full,
            now + std::time::Duration::from_secs(30),
        );
        assert_eq!(rack.construction_animation.cables.len(), 0);
    }
}

#[test]
fn cable_move_passive_overlay_dims_in_dark_room_and_reduced_marks_only_changed_ends() {
    let ctx = egui::Context::default();
    let from = pos2(120.0, 100.0);
    let to = pos2(450.0, 260.0);
    let movement = cable_move_geometry(
        pos2(20.0, 40.0),
        pos2(70.0, 180.0),
        from,
        to,
        0.0,
        0.0,
        1.0,
        0.5,
        true,
        true,
    );
    let output = ctx.run(egui::RawInput::default(), |ctx| {
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Middle,
            egui::Id::new("dark-move"),
        ));
        let mut lighting = crate::lighting::RoomLighting::default();
        lighting.set(0.0);
        let start = lighting.begin(&painter);
        paint_moving_cable_curves(
            &painter,
            from,
            to,
            CABLES[0],
            1.0,
            false,
            0.0,
            movement,
            Default::default(),
        );
        lighting.finish(&painter, start);
    });
    let lenses = output
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Circle(c) if c.radius == 2.0 => Some(c),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(lenses.len(), 2);
    for lens in lenses {
        assert_eq!(
            [lens.fill.r(), lens.fill.g(), lens.fill.b()],
            [0, 0, 0],
            "traveling plugs are not signal lamps"
        );
    }
    for ends in [(true, false), (false, true), (true, true)] {
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Middle,
                egui::Id::new("reduced-move"),
            ));
            paint_reduced_cable_move(
                &painter,
                from,
                to,
                CABLES[0],
                1.0,
                false,
                0.0,
                0.8,
                ends,
                &crate::lighting::RoomLighting::default(),
                Default::default(),
            );
        });
        let rings = output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Circle(c) if c.radius == 11.5 => Some(c.center),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(rings.contains(&from), ends.0);
        assert_eq!(rings.contains(&to), ends.1);
        assert_eq!(rings.len(), usize::from(ends.0) + usize::from(ends.1));
    }
}
