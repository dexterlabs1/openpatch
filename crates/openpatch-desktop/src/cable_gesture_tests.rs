// Included inside rack::tests so these acceptance cases use the native pointer helpers.
mod rack_gesture_acceptance {
    use super::*;
    use openpatch::control::Session;

    fn setup(patch: Patch) -> (tempfile::TempDir, Session, egui::Context, Rack) {
        let workspace = tempfile::tempdir().unwrap();
        let mut session = Session::new(workspace.path()).unwrap();
        session.replace_patch(patch, None).unwrap();
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        frame_revision_state(
            &ctx,
            &mut rack,
            session.patch(),
            0.0,
            vec![],
            true,
            true,
            session.revision(),
        );
        (workspace, session, ctx, rack)
    }

    fn pointer(position: Pos2, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::PointerButton {
            pos: position,
            button: PointerButton::Primary,
            pressed,
            modifiers,
        }
    }

    fn step(
        ctx: &egui::Context,
        rack: &mut Rack,
        session: &Session,
        time: f64,
        events: Vec<egui::Event>,
    ) -> Option<RackAction> {
        frame_revision_state(
            ctx,
            rack,
            session.patch(),
            time,
            events,
            true,
            true,
            session.revision(),
        )
        .0
    }

    fn start(
        ctx: &egui::Context,
        rack: &mut Rack,
        session: &Session,
        from: Pos2,
        to: Pos2,
        modifiers: egui::Modifiers,
    ) {
        assert!(step(
            ctx,
            rack,
            session,
            1.0,
            vec![
                egui::Event::PointerMoved(from),
                pointer(from, true, modifiers)
            ]
        )
        .is_none());
        // Deliberately release modifiers before the drag threshold.
        assert!(step(ctx, rack, session, 1.1, vec![egui::Event::PointerMoved(to)]).is_none());
    }

    fn assert_single_undo(session: &mut Session, before: &Value) {
        let after = session.snapshot();
        assert_eq!(after["revision"], before["revision"].as_u64().unwrap() + 1);
        assert_eq!(
            after["history"]["undo_steps"],
            before["history"]["undo_steps"].as_u64().unwrap() + 1
        );
        session.execute("undo", json!({})).unwrap();
        assert_eq!(session.snapshot()["patch"], before["patch"]);
        session.execute("redo", json!({})).unwrap();
        assert_eq!(session.snapshot()["patch"], after["patch"]);
    }

    #[test]
    fn shift_press_deletes_top_at_either_end_and_never_leaves_a_click_or_drag_draft() {
        for input in [false, true] {
            for occupied in [false, true] {
                let (_workspace, mut session, ctx, mut rack) = setup(stacked_fixture());
                let before = session.snapshot();
                let from = fixture_jack(
                    &rack,
                    session.patch(),
                    if input {
                        if occupied {
                            "target"
                        } else {
                            "dest"
                        }
                    } else if occupied {
                        "src"
                    } else {
                        "alt"
                    },
                    if input { "signal" } else { "out" },
                    input,
                );
                let action = step(
                    &ctx,
                    &mut rack,
                    &session,
                    1.0,
                    vec![
                        egui::Event::PointerMoved(from),
                        pointer(from, true, egui::Modifiers::SHIFT),
                    ],
                );
                if occupied {
                    assert!(
                        matches!(&action, Some(RackAction::Command("disconnect", args))
                        if args["cable_id"] == "two" && args["expected_revision"] == before["revision"])
                    );
                    run_cable_action(&mut session, action.unwrap());
                    assert_eq!(session.patch().cables.len(), 1);
                    assert_eq!(
                        session.snapshot()["patch"]["cables"][0],
                        before["patch"]["cables"][0]
                    );
                } else {
                    assert!(action.is_none());
                }
                assert!(step(
                    &ctx,
                    &mut rack,
                    &session,
                    1.1,
                    vec![egui::Event::PointerMoved(pos2(1000.0, 600.0))]
                )
                .is_none());
                assert!(step(
                    &ctx,
                    &mut rack,
                    &session,
                    1.2,
                    vec![pointer(pos2(1000.0, 600.0), false, egui::Modifiers::NONE)]
                )
                .is_none());
                assert!(rack.pending.is_none());
                if occupied {
                    assert_single_undo(&mut session, &before);
                } else {
                    assert_eq!(session.snapshot(), before);
                }
            }
        }
    }

    #[test]
    fn command_or_control_press_creates_fresh_from_either_occupied_end_after_modifiers_release() {
        for input in [false, true] {
            for ctrl in [false, true] {
                let (_workspace, mut session, ctx, mut rack) = setup(stacked_fixture());
                let before = session.snapshot();
                let from = fixture_jack(
                    &rack,
                    session.patch(),
                    if input { "target" } else { "src" },
                    if input { "signal" } else { "out" },
                    input,
                );
                let to = fixture_jack(
                    &rack,
                    session.patch(),
                    if input { "alt" } else { "dest" },
                    if input { "out" } else { "signal" },
                    !input,
                );
                start(
                    &ctx,
                    &mut rack,
                    &session,
                    from,
                    to,
                    egui::Modifiers {
                        command: !ctrl,
                        ctrl,
                        ..Default::default()
                    },
                );
                assert_eq!(session.snapshot(), before);
                let action = step(
                    &ctx,
                    &mut rack,
                    &session,
                    1.2,
                    vec![pointer(to, false, egui::Modifiers::NONE)],
                )
                .expect("fresh compatible drop");
                run_cable_action(&mut session, action);
                assert_eq!(session.patch().cables.len(), 3);
                assert_eq!(
                    session.snapshot()["patch"]["cables"][0],
                    before["patch"]["cables"][0]
                );
                assert_eq!(
                    session.snapshot()["patch"]["cables"][1],
                    before["patch"]["cables"][1]
                );
                let added = &session.patch().cables[2];
                assert!(!matches!(added.id.as_deref(), Some("one" | "two")));
                assert_eq!(
                    (&*added.from, &*added.to),
                    if input {
                        ("alt", "target")
                    } else {
                        ("src", "dest")
                    }
                );
                assert_single_undo(&mut session, &before);
            }
        }
    }

    #[test]
    fn command_shift_clones_clicked_end_with_new_id_and_preserved_effective_rgb() {
        for input in [false, true] {
            for explicit_color in [false, true] {
                let mut patch = stacked_fixture();
                if explicit_color {
                    let mut encoded = serde_json::to_value(&patch).unwrap();
                    encoded["cables"][1]["color"] = json!([23, 67, 101]);
                    patch = serde_json::from_value(encoded).unwrap();
                }
                let (workspace, mut session, ctx, mut rack) = setup(patch);
                let before = session.snapshot();
                let color = cable_instance_color(&session.patch().cables[1]);
                let from = fixture_jack(
                    &rack,
                    session.patch(),
                    if input { "target" } else { "src" },
                    if input { "signal" } else { "out" },
                    input,
                );
                let to = fixture_jack(
                    &rack,
                    session.patch(),
                    if input { "dest" } else { "alt" },
                    if input { "signal" } else { "out" },
                    input,
                );
                start(
                    &ctx,
                    &mut rack,
                    &session,
                    from,
                    to,
                    egui::Modifiers {
                        command: true,
                        shift: true,
                        ..Default::default()
                    },
                );
                assert_eq!(session.snapshot(), before);
                let action = step(
                    &ctx,
                    &mut rack,
                    &session,
                    1.2,
                    vec![pointer(to, false, egui::Modifiers::NONE)],
                )
                .expect("clone same-end drop");
                run_cable_action(&mut session, action);
                assert_eq!(session.patch().cables.len(), 3);
                assert_eq!(
                    session.snapshot()["patch"]["cables"][0],
                    before["patch"]["cables"][0]
                );
                assert_eq!(
                    session.snapshot()["patch"]["cables"][1],
                    before["patch"]["cables"][1]
                );
                let clone = &session.patch().cables[2];
                assert!(!matches!(clone.id.as_deref(), Some("one" | "two")));
                assert_eq!(
                    (&*clone.from, &*clone.to),
                    if input {
                        ("src", "dest")
                    } else {
                        ("alt", "target")
                    }
                );
                assert_eq!(cable_instance_color(clone), color);
                let encoded = serde_json::to_value(clone).unwrap();
                assert_eq!(encoded["color"], json!([color.r(), color.g(), color.b()]));
                session
                    .execute("save_patch", json!({"filename":"clone.json"}))
                    .unwrap();
                let saved: Patch = serde_json::from_slice(
                    &std::fs::read(workspace.path().join("clone.json")).unwrap(),
                )
                .unwrap();
                assert_eq!(
                    serde_json::to_value(saved).unwrap(),
                    session.snapshot()["patch"]
                );
                assert_single_undo(&mut session, &before);
            }
        }
    }

    #[test]
    fn invalid_pointer_drop_deletes_only_moved_cable_but_discards_fresh_and_clone_drafts() {
        for input in [false, true] {
            for intent in ["move", "fresh", "clone"] {
                for outcome in ["wrong", "empty", "duplicate"] {
                    let mut patch = stacked_fixture();
                    if outcome == "duplicate" && intent != "fresh" {
                        let mut duplicate = patch.cables[1].clone();
                        duplicate.id = Some("duplicate".into());
                        if input {
                            duplicate.to = "dest".into();
                        } else {
                            duplicate.from = "alt".into();
                        }
                        patch.cables.push(duplicate);
                    }
                    let (_workspace, mut session, ctx, mut rack) = setup(patch);
                    let before = session.snapshot();
                    let from = fixture_jack(
                        &rack,
                        session.patch(),
                        if input { "target" } else { "src" },
                        if input { "signal" } else { "out" },
                        input,
                    );
                    let to = match outcome {
                        "empty" => pos2(1000.0, 600.0),
                        "wrong" => {
                            let target_input = if intent == "fresh" { input } else { !input };
                            fixture_jack(
                                &rack,
                                session.patch(),
                                if target_input { "dest" } else { "alt" },
                                if target_input { "signal" } else { "out" },
                                target_input,
                            )
                        }
                        "duplicate" if intent == "fresh" => fixture_jack(
                            &rack,
                            session.patch(),
                            if input { "src" } else { "target" },
                            if input { "out" } else { "signal" },
                            !input,
                        ),
                        _ => fixture_jack(
                            &rack,
                            session.patch(),
                            if input { "dest" } else { "alt" },
                            if input { "signal" } else { "out" },
                            input,
                        ),
                    };
                    let modifiers = egui::Modifiers {
                        command: intent != "move",
                        shift: intent == "clone",
                        ..Default::default()
                    };
                    start(&ctx, &mut rack, &session, from, to, modifiers);
                    assert_eq!(
                        session.snapshot(),
                        before,
                        "live route stays intact during preview"
                    );
                    let action = step(
                        &ctx,
                        &mut rack,
                        &session,
                        1.2,
                        vec![pointer(to, false, egui::Modifiers::NONE)],
                    );
                    if intent == "move" {
                        assert!(
                            matches!(&action, Some(RackAction::Command("disconnect", args)) if args["cable_id"] == "two"),
                            "{input}: {outcome}"
                        );
                        run_cable_action(&mut session, action.unwrap());
                        let expected: Vec<_> = before["patch"]["cables"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .filter(|cable| cable["id"] != "two")
                            .cloned()
                            .collect();
                        assert_eq!(session.snapshot()["patch"]["cables"], json!(expected));
                        assert_single_undo(&mut session, &before);
                    } else {
                        assert!(action.is_none(), "{input}: {intent}/{outcome}");
                        assert_eq!(session.snapshot(), before);
                    }
                    assert!(rack.pending.is_none());
                }
            }
        }
    }

    #[test]
    fn compatible_move_keeps_different_route_siblings_and_same_origin_is_a_noop() {
        for input in [false, true] {
            for same_origin in [false, true] {
                let mut patch = stacked_fixture();
                if input && !same_origin {
                    let mut sibling = patch.cables[0].clone();
                    sibling.id = Some("destination_sibling".into());
                    sibling.from = "alt".into();
                    sibling.to = "dest".into();
                    patch.cables.push(sibling);
                }
                let (_workspace, mut session, ctx, mut rack) = setup(patch);
                let before = session.snapshot();
                let layout = rack.layout();
                let from = fixture_jack(
                    &rack,
                    session.patch(),
                    if input { "target" } else { "src" },
                    if input { "signal" } else { "out" },
                    input,
                );
                let to = if same_origin {
                    from
                } else {
                    fixture_jack(
                        &rack,
                        session.patch(),
                        if input { "dest" } else { "alt" },
                        if input { "signal" } else { "out" },
                        input,
                    )
                };
                start(
                    &ctx,
                    &mut rack,
                    &session,
                    from,
                    if same_origin {
                        from + vec2(30.0, 0.0)
                    } else {
                        to
                    },
                    egui::Modifiers::NONE,
                );
                let action = step(
                    &ctx,
                    &mut rack,
                    &session,
                    1.2,
                    vec![
                        egui::Event::PointerMoved(to),
                        pointer(to, false, egui::Modifiers::NONE),
                    ],
                );
                if same_origin {
                    assert!(action.is_none());
                    assert_eq!(session.snapshot(), before);
                } else {
                    run_cable_action(&mut session, action.expect("same-type compatible move"));
                    assert_eq!(
                        session.patch().cables.len(),
                        before["patch"]["cables"].as_array().unwrap().len()
                    );
                    for old in before["patch"]["cables"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter(|cable| cable["id"] != "two")
                    {
                        let current = session.snapshot();
                        let mut actual = current["patch"]["cables"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .find(|cable| cable["id"] == old["id"])
                            .unwrap()
                            .clone();
                        let mut expected = old.clone();
                        // Promotion may densely normalize the grabbed endpoint
                        // family; sibling routes/colors and opposite ranks stay exact.
                        let field = if input { "input_order" } else { "output_order" };
                        actual.as_object_mut().unwrap().remove(field);
                        expected.as_object_mut().unwrap().remove(field);
                        assert_eq!(actual, expected);
                    }
                    let moved = session
                        .patch()
                        .cables
                        .iter()
                        .find(|cable| cable.id.as_deref() == Some("two"))
                        .unwrap();
                    assert_eq!(
                        (&*moved.from, &*moved.to),
                        if input {
                            ("src", "dest")
                        } else {
                            ("alt", "target")
                        }
                    );
                    assert_single_undo(&mut session, &before);
                }
                assert_eq!(rack.layout(), layout);
            }
        }
    }

    #[test]
    fn only_shift_added_at_drop_requests_identical_route_extension_not_clone_press_shift() {
        for mode in ["duplicate", "late_shift", "clone"] {
            let (_workspace, mut session, ctx, mut rack) = setup(stacked_fixture());
            let before = session.snapshot();
            let from = fixture_jack(&rack, session.patch(), "src", "out", false);
            let to = fixture_jack(
                &rack,
                session.patch(),
                if mode == "clone" { "src" } else { "target" },
                if mode == "clone" { "out" } else { "signal" },
                mode != "clone",
            );
            let press = egui::Modifiers {
                command: true,
                shift: mode == "clone",
                ..Default::default()
            };
            start(
                &ctx,
                &mut rack,
                &session,
                from,
                if mode == "clone" {
                    to + vec2(30.0, 0.0)
                } else {
                    to
                },
                press,
            );
            let release = if mode == "duplicate" {
                egui::Modifiers::NONE
            } else {
                egui::Modifiers::SHIFT
            };
            let action = step(
                &ctx,
                &mut rack,
                &session,
                1.2,
                vec![egui::Event::PointerMoved(to), pointer(to, false, release)],
            );
            if mode == "late_shift" {
                run_cable_action(
                    &mut session,
                    action.expect("late Shift explicitly permits duplicate v2 route"),
                );
                assert_eq!(session.patch().cables.len(), 3);
                assert_eq!(
                    session.snapshot()["patch"]["cables"][0],
                    before["patch"]["cables"][0]
                );
                assert_eq!(
                    session.snapshot()["patch"]["cables"][1],
                    before["patch"]["cables"][1]
                );
                assert_single_undo(&mut session, &before);
            } else {
                assert!(action.is_none(), "{mode}");
                assert_eq!(session.snapshot(), before);
            }
        }
    }

    #[test]
    fn escape_focus_modal_and_revision_interruptions_cancel_instead_of_deleting_pickup() {
        for input in [false, true] {
            for interruption in [
                "escape",
                "focus",
                "modal",
                "stale",
                "same_id",
                "before_threshold",
            ] {
                let (_workspace, mut session, ctx, mut rack) = setup(stacked_fixture());
                let from = fixture_jack(
                    &rack,
                    session.patch(),
                    if input { "target" } else { "src" },
                    if input { "signal" } else { "out" },
                    input,
                );
                let to = pos2(1000.0, 600.0);
                if interruption == "before_threshold" {
                    assert!(step(
                        &ctx,
                        &mut rack,
                        &session,
                        1.0,
                        vec![
                            egui::Event::PointerMoved(from),
                            pointer(from, true, egui::Modifiers::NONE)
                        ]
                    )
                    .is_none());
                } else {
                    start(&ctx, &mut rack, &session, from, to, egui::Modifiers::NONE);
                }
                if interruption == "stale" {
                    session
                        .execute(
                            "set_parameter",
                            json!({"id":"alt","parameter":"value","value":2.5}),
                        )
                        .unwrap();
                } else if interruption == "same_id" {
                    let mut patch = session.snapshot()["patch"].clone();
                    patch["cables"][1]["color"] = json!([90, 60, 30]);
                    session
                        .replace_patch(serde_json::from_value(patch).unwrap(), None)
                        .unwrap();
                }
                let before_cancel = session.snapshot();
                let events = if interruption == "escape" {
                    vec![egui::Event::Key {
                        key: egui::Key::Escape,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    }]
                } else {
                    vec![]
                };
                assert!(frame_revision_state(
                    &ctx,
                    &mut rack,
                    session.patch(),
                    if interruption == "before_threshold" {
                        1.05
                    } else {
                        1.15
                    },
                    events,
                    !matches!(interruption, "focus" | "before_threshold"),
                    interruption != "modal",
                    session.revision()
                )
                .0
                .is_none());
                if interruption == "before_threshold" {
                    assert!(step(
                        &ctx,
                        &mut rack,
                        &session,
                        1.1,
                        vec![egui::Event::PointerMoved(to)]
                    )
                    .is_none());
                }
                assert!(
                    step(
                        &ctx,
                        &mut rack,
                        &session,
                        1.2,
                        vec![pointer(to, false, egui::Modifiers::NONE)]
                    )
                    .is_none(),
                    "{input}: {interruption}"
                );
                assert!(rack.pending.is_none());
                assert_eq!(session.snapshot(), before_cancel, "{input}: {interruption}");
            }
        }
    }

    #[test]
    fn version_one_unsupported_stack_and_cycle_cancel_complete_move_without_deleting_original() {
        for cycle in [false, true] {
            let mut patch = stacked_fixture();
            patch.version = 1;
            patch.cables.truncate(1);
            if !cycle {
                let mut sibling = patch.cables[0].clone();
                sibling.id = Some("destination_sibling".into());
                sibling.from = "alt".into();
                sibling.to = "dest".into();
                patch.cables.push(sibling);
            }
            let (_workspace, session, ctx, mut rack) = setup(patch);
            let before = session.snapshot();
            let from = fixture_jack(
                &rack,
                session.patch(),
                if cycle { "src" } else { "target" },
                if cycle { "out" } else { "signal" },
                !cycle,
            );
            let to = fixture_jack(
                &rack,
                session.patch(),
                if cycle { "target" } else { "dest" },
                if cycle { "out" } else { "signal" },
                !cycle,
            );
            start(&ctx, &mut rack, &session, from, to, egui::Modifiers::NONE);
            assert!(step(
                &ctx,
                &mut rack,
                &session,
                1.2,
                vec![pointer(to, false, egui::Modifiers::NONE)]
            )
            .is_none());
            assert!(rack.pending.is_none());
            assert_eq!(session.snapshot(), before);
        }
    }

    #[test]
    fn captured_delete_and_move_actions_reject_stale_backend_revision_without_partial_edits() {
        for delete in [false, true] {
            let (_workspace, mut session, ctx, mut rack) = setup(stacked_fixture());
            let from = fixture_jack(&rack, session.patch(), "src", "out", false);
            let to = if delete {
                pos2(1000.0, 600.0)
            } else {
                fixture_jack(&rack, session.patch(), "alt", "out", false)
            };
            start(&ctx, &mut rack, &session, from, to, egui::Modifiers::NONE);
            let action = step(
                &ctx,
                &mut rack,
                &session,
                1.2,
                vec![pointer(to, false, egui::Modifiers::NONE)],
            )
            .expect("planned move/delete");
            session
                .execute(
                    "set_parameter",
                    json!({"id":"alt","parameter":"value","value":2.5}),
                )
                .unwrap();
            let concurrent = session.snapshot();
            let RackAction::Command(tool, args) = action else {
                panic!("graph command")
            };
            assert!(session
                .execute(tool, args)
                .unwrap_err()
                .contains("revision"));
            assert_eq!(session.snapshot(), concurrent);
        }
    }

    #[test]
    fn cable_capacity_rejects_fresh_and_clone_but_allows_atomic_move_without_losing_routes() {
        for intent in ["fresh", "clone", "move"] {
            let mut patch = stacked_fixture();
            for index in patch.cables.len()..openpatch_core::MAX_CABLES {
                let mut cable = patch.cables[0].clone();
                cable.id = Some(format!("capacity_{index}"));
                patch.cables.push(cable);
            }
            let (_workspace, mut session, ctx, mut rack) = setup(patch);
            let before = session.snapshot();
            let top_id = session.patch().cables.last().unwrap().id.clone().unwrap();
            let from = fixture_jack(&rack, session.patch(), "src", "out", false);
            let to = fixture_jack(
                &rack,
                session.patch(),
                if intent == "fresh" { "dest" } else { "alt" },
                if intent == "fresh" { "signal" } else { "out" },
                intent == "fresh",
            );
            start(
                &ctx,
                &mut rack,
                &session,
                from,
                to,
                egui::Modifiers {
                    command: intent != "move",
                    shift: intent == "clone",
                    ..Default::default()
                },
            );
            let action = step(
                &ctx,
                &mut rack,
                &session,
                1.2,
                vec![pointer(to, false, egui::Modifiers::NONE)],
            );
            if intent == "move" {
                run_cable_action(
                    &mut session,
                    action.expect("moving one existing cable does not exceed capacity"),
                );
                assert_eq!(session.patch().cables.len(), openpatch_core::MAX_CABLES);
                let after = session.snapshot();
                let retained: Vec<_> = after["patch"]["cables"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|cable| cable["id"].as_str() != Some(top_id.as_str()))
                    .cloned()
                    .collect();
                let expected: Vec<_> = before["patch"]["cables"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|cable| cable["id"].as_str() != Some(top_id.as_str()))
                    .cloned()
                    .collect();
                assert_eq!(retained, expected);
                let moved = session
                    .patch()
                    .cables
                    .iter()
                    .find(|cable| cable.id.as_deref() == Some(top_id.as_str()))
                    .unwrap();
                assert_eq!((&*moved.from, &*moved.to), ("alt", "target"));
                assert_single_undo(&mut session, &before);
            } else {
                assert!(action.is_none(), "{intent} must not add at capacity");
                assert_eq!(session.snapshot(), before);
            }
            assert!(rack.pending.is_none());
        }
    }

    #[test]
    fn coalesced_shift_press_and_release_delete_once_at_either_end_without_a_draft() {
        for input in [false, true] {
            for release_away in [false, true] {
                let (_workspace, mut session, ctx, mut rack) = setup(stacked_fixture());
                let before = session.snapshot();
                let from = fixture_jack(
                    &rack,
                    session.patch(),
                    if input { "target" } else { "src" },
                    if input { "signal" } else { "out" },
                    input,
                );
                let release = if release_away {
                    pos2(1200.0, 900.0)
                } else {
                    from
                };
                let mut events = vec![
                    egui::Event::PointerMoved(from),
                    pointer(from, true, egui::Modifiers::SHIFT),
                ];
                if release_away {
                    events.push(egui::Event::PointerMoved(release));
                }
                events.push(pointer(release, false, egui::Modifiers::NONE));
                let action = step(&ctx, &mut rack, &session, 1.0, events)
                    .expect("coalesced Shift gesture deletes on the press");
                assert!(
                    matches!(&action, RackAction::Command("disconnect", args) if args["cable_id"] == "two")
                );
                run_cable_action(&mut session, action);
                assert!(rack.pending.is_none());
                assert!(step(
                    &ctx,
                    &mut rack,
                    &session,
                    1.1,
                    vec![egui::Event::PointerMoved(from + vec2(40.0, 0.0))]
                )
                .is_none());
                assert!(rack.pending.is_none());
                assert_eq!(session.patch().cables.len(), 1);
                assert_single_undo(&mut session, &before);
            }
        }
    }

    #[test]
    fn outside_canvas_release_deletes_move_while_pointer_gone_without_release_cancels() {
        for input in [false, true] {
            for lost_pointer in [false, true] {
                let (_workspace, mut session, ctx, mut rack) = setup(stacked_fixture());
                let before = session.snapshot();
                let from = fixture_jack(
                    &rack,
                    session.patch(),
                    if input { "target" } else { "src" },
                    if input { "signal" } else { "out" },
                    input,
                );
                let outside = pos2(1200.0, 900.0);
                start(
                    &ctx,
                    &mut rack,
                    &session,
                    from,
                    outside,
                    egui::Modifiers::NONE,
                );
                if lost_pointer {
                    assert!(step(
                        &ctx,
                        &mut rack,
                        &session,
                        1.15,
                        vec![egui::Event::PointerGone]
                    )
                    .is_none());
                    assert!(rack.pending.is_none());
                }
                let action = step(
                    &ctx,
                    &mut rack,
                    &session,
                    1.2,
                    vec![pointer(outside, false, egui::Modifiers::NONE)],
                );
                if lost_pointer {
                    assert!(action.is_none());
                    assert_eq!(session.snapshot(), before);
                } else {
                    assert!(
                        matches!(&action, Some(RackAction::Command("disconnect", args)) if args["cable_id"] == "two")
                    );
                    run_cable_action(&mut session, action.unwrap());
                    assert_eq!(session.patch().cables.len(), 1);
                    assert_single_undo(&mut session, &before);
                }
                assert!(rack.pending.is_none());
            }
        }
    }

    #[test]
    fn low_zoom_overlapping_jack_regions_delete_only_the_nearest_ports_cable() {
        let patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[
            {"id":"split","kind":"poly_split"}, {"id":"target","kind":"gain"}, {"id":"dest","kind":"gain"}
        ],"cables":[
            {"id":"neighbor_one","from":"split","output":"out1","to":"target","input":"signal"},
            {"id":"neighbor_two","from":"split","output":"out2","to":"dest","input":"signal"}
        ]})).unwrap();
        let (_workspace, mut session, ctx, mut rack) = setup(patch);
        rack.set_zoom(0.25);
        assert!(step(&ctx, &mut rack, &session, 0.5, vec![]).is_none());
        let first = fixture_jack(&rack, session.patch(), "split", "out1", false);
        let second = fixture_jack(&rack, session.patch(), "split", "out2", false);
        let point = first.lerp(second, 0.55);
        assert!(Rect::from_center_size(first, Vec2::splat(18.0)).contains(point));
        assert!(Rect::from_center_size(second, Vec2::splat(18.0)).contains(point));
        assert!(point.distance(second) < point.distance(first));
        let before = session.snapshot();
        let action = step(
            &ctx,
            &mut rack,
            &session,
            1.0,
            vec![
                egui::Event::PointerMoved(point),
                pointer(point, true, egui::Modifiers::SHIFT),
            ],
        )
        .expect("nearest jack's top cable is deleted");
        assert!(
            matches!(&action, RackAction::Command("disconnect", args) if args["cable_id"] == "neighbor_two")
        );
        run_cable_action(&mut session, action);
        assert_eq!(session.patch().cables.len(), 1);
        assert_eq!(
            session.patch().cables[0].id.as_deref(),
            Some("neighbor_one")
        );
        assert!(step(
            &ctx,
            &mut rack,
            &session,
            1.1,
            vec![pointer(point, false, egui::Modifiers::NONE)]
        )
        .is_none());
        assert!(rack.pending.is_none());
        assert_single_undo(&mut session, &before);
    }

    #[test]
    fn higher_interactive_layer_and_same_batch_blur_refocus_block_shift_deletion() {
        for input in [false, true] {
            for blocked in ["layer", "blur_refocus"] {
                let (_workspace, session, ctx, mut rack) = setup(stacked_fixture());
                let before = session.snapshot();
                let from = fixture_jack(
                    &rack,
                    session.patch(),
                    if input { "target" } else { "src" },
                    if input { "signal" } else { "out" },
                    input,
                );
                let overlay_rect = Rect::from_center_size(from, Vec2::splat(70.0));
                let mut render = |time: f64, events: Vec<egui::Event>| {
                    let mut action = None;
                    let _ = ctx.run(
                        egui::RawInput {
                            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 700.0))),
                            focused: true,
                            time: Some(time),
                            events,
                            ..Default::default()
                        },
                        |ctx| {
                            if blocked == "layer" {
                                let overlay = egui::Area::new(egui::Id::new("cable-test-overlay"))
                                    .order(egui::Order::Foreground)
                                    .fixed_pos(overlay_rect.min)
                                    .interactable(true)
                                    .show(ctx, |ui| {
                                        ui.allocate_exact_size(
                                            overlay_rect.size(),
                                            Sense::click_and_drag(),
                                        );
                                    });
                                if time >= 1.0 {
                                    assert_eq!(
                                        ctx.layer_id_at(from),
                                        Some(overlay.response.layer_id)
                                    );
                                }
                            }
                            egui::CentralPanel::default().frame(egui::Frame::NONE).show(
                                ctx,
                                |ui| {
                                    action = rack.show(
                                        ui,
                                        session.patch(),
                                        &openpatch_core::catalog(),
                                        48000,
                                        session.revision(),
                                    );
                                },
                            );
                        },
                    );
                    action
                };
                assert!(render(0.5, vec![egui::Event::PointerMoved(from)]).is_none());
                let mut events = vec![egui::Event::PointerMoved(from)];
                if blocked == "blur_refocus" {
                    events.extend([
                        egui::Event::WindowFocused(false),
                        egui::Event::WindowFocused(true),
                    ]);
                }
                events.push(pointer(from, true, egui::Modifiers::SHIFT));
                assert!(render(1.0, events).is_none(), "{input}: {blocked}");
                assert!(render(1.1, vec![pointer(from, false, egui::Modifiers::NONE)]).is_none());
                assert!(rack.pending.is_none());
                assert_eq!(session.snapshot(), before, "{input}: {blocked}");
            }
        }
    }
}
