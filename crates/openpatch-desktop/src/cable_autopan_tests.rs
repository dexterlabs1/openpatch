use super::*;

fn pointer(position: Pos2, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::PointerButton {
        pos: position,
        button: PointerButton::Primary,
        pressed,
        modifiers,
    }
}
fn pending(rack: &mut Rack) {
    rack.pending = Some(PendingCable::at_revision(
        CableEnd::Output {
            node: "source".into(),
            port: "out".into(),
        },
        &Patch::new_v2(),
        rack.observed_revision,
    ));
    rack.dragging_cable = true;
}

// Calls the real method with egui pointer ownership, clipping and layer state.
// stable_dt is set explicitly to test pathological clock inputs as well as fps.
fn direct_frame(
    ctx: &egui::Context,
    rack: &mut Rack,
    dt: f32,
    events: Vec<egui::Event>,
    guard: &str,
) -> bool {
    let mut moved = false;
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(600.0, 400.0))),
            events,
            focused: guard != "blur",
            predicted_dt: 1.0 / 60.0,
            ..Default::default()
        },
        |ctx| {
            ctx.input_mut(|input| input.stable_dt = dt);
            if guard == "other-layer" {
                egui::Area::new(egui::Id::new("other-dock"))
                    .order(egui::Order::Foreground)
                    .fixed_pos(pos2(500.0, 150.0))
                    .show(ctx, |ui| {
                        ui.allocate_exact_size(vec2(100.0, 100.0), Sense::hover());
                    });
            }
            if guard == "modal" {
                egui::Modal::new(egui::Id::new("blocking-modal")).show(ctx, |ui| {
                    ui.label("Modal");
                });
            }
            if guard == "popup" {
                egui::Popup::open_id(ctx, egui::Id::new("blocking-popup"));
            }
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    if guard == "disabled" {
                        ui.disable();
                    }
                    if guard == "clipped" {
                        ui.set_clip_rect(Rect::from_min_max(pos2(0.0, 0.0), pos2(500.0, 400.0)));
                    }
                    moved = rack.cable_edge_autopan(
                        ui,
                        Rect::from_min_size(Pos2::ZERO, vec2(600.0, 400.0)),
                        guard != "off",
                    );
                });
        },
    );
    moved
}

#[test]
fn cable_autopan_edge_zone_is_smooth_bounded_and_has_no_tiny_viewport_overlap() {
    let visible = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 500.0));
    assert_eq!(
        cable_autopan_velocity(pos2(400.0, 250.0), visible),
        Vec2::ZERO
    );
    assert_eq!(
        cable_autopan_velocity(pos2(48.0, 250.0), visible),
        Vec2::ZERO
    );
    assert_eq!(
        cable_autopan_velocity(pos2(24.0, 250.0), visible),
        vec2(300.0, 0.0)
    );
    assert_eq!(
        cable_autopan_velocity(pos2(0.0, 250.0), visible),
        vec2(600.0, 0.0)
    );
    assert_eq!(
        cable_autopan_velocity(pos2(800.0, 250.0), visible),
        vec2(-600.0, 0.0)
    );
    let diagonal = cable_autopan_velocity(visible.left_top(), visible);
    assert!((diagonal.length() - 600.0).abs() < 0.001);
    assert!((diagonal.x - diagonal.y).abs() < 0.001);
    for size in [vec2(1.0, 1.0), vec2(30.0, 20.0), vec2(96.0, 96.0)] {
        let tiny = Rect::from_min_size(Pos2::ZERO, size);
        assert_eq!(cable_autopan_velocity(tiny.center(), tiny), Vec2::ZERO);
        assert!(cable_autopan_velocity(tiny.left_top(), tiny).length() <= 600.001);
        assert!(cable_autopan_velocity(tiny.center() - vec2(0.01, 0.0), tiny).x >= 0.0);
        assert!(cable_autopan_velocity(tiny.center() + vec2(0.01, 0.0), tiny).x <= 0.0);
    }
    for pointer in [pos2(-1.0, 250.0), pos2(801.0, 250.0), pos2(f32::NAN, 0.0)] {
        assert_eq!(cable_autopan_velocity(pointer, visible), Vec2::ZERO);
    }
    assert_eq!(
        cable_autopan_velocity(Pos2::ZERO, Rect::NOTHING),
        Vec2::ZERO
    );
}

#[test]
fn cable_autopan_only_runs_for_a_current_held_drag_in_the_visible_rack_layer() {
    for guard in [
        "off",
        "disabled",
        "shortcuts",
        "blur",
        "popup",
        "modal",
        "other-layer",
        "clipped",
        "idle-click-draft",
        "no-pending",
        "stale-revision",
        "precision",
        "annotation",
        "annotation-guard",
        "escape",
        "pointer-gone",
        "focus-event",
        "release",
        "outside",
    ] {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        direct_frame(&ctx, &mut rack, 1.0 / 60.0, vec![], guard);
        pending(&mut rack);
        match guard {
            "shortcuts" => rack.shortcuts_enabled = false,
            "idle-click-draft" => rack.dragging_cable = false,
            "no-pending" => rack.pending = None,
            "stale-revision" => rack.observed_revision += 1,
            "precision" => {
                let node: Node =
                    serde_json::from_value(json!({"id":"vca","kind":"stock_vca"})).unwrap();
                rack.editing = Some(stock_vca_edit(&node, 0, 0));
            }
            "annotation" => {
                rack.annotation_edit = Some(AnnotationEdit {
                    id: "notes".into(),
                    kind: "notes".into(),
                    revision: 0,
                    text: String::new(),
                    focus: false,
                    stale: false,
                    error: None,
                })
            }
            "annotation-guard" => rack.annotation_input_guard = true,
            _ => {}
        }
        let position = if guard == "outside" {
            pos2(601.0, 200.0)
        } else {
            pos2(599.0, 200.0)
        };
        let mut events = vec![
            egui::Event::PointerMoved(position),
            pointer(position, true, egui::Modifiers::NONE),
        ];
        match guard {
            "escape" => events.push(egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }),
            "pointer-gone" => events.push(egui::Event::PointerGone),
            "focus-event" => events.push(egui::Event::WindowFocused(false)),
            "release" => events.push(pointer(position, false, egui::Modifiers::NONE)),
            _ => {}
        }
        let before = rack.pan;
        assert!(
            !direct_frame(&ctx, &mut rack, 1.0 / 60.0, events, guard),
            "guard {guard}"
        );
        assert_eq!(rack.pan, before, "guard {guard}");
    }
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    direct_frame(&ctx, &mut rack, 1.0 / 60.0, vec![], "");
    pending(&mut rack);
    let edge = pos2(599.0, 200.0);
    assert!(direct_frame(
        &ctx,
        &mut rack,
        1.0 / 60.0,
        vec![
            egui::Event::PointerMoved(edge),
            pointer(edge, true, egui::Modifiers::NONE)
        ],
        ""
    ));
}

#[test]
fn cable_autopan_is_screen_space_cadence_independent_and_clamps_bad_time_and_pan() {
    let mut pans = vec![];
    for zoom in [0.25, 0.85, 1.0, 4.0] {
        for fps in [30, 60, 144] {
            let ctx = egui::Context::default();
            let mut rack = Rack {
                zoom,
                pan: Vec2::ZERO,
                ..Default::default()
            };
            direct_frame(&ctx, &mut rack, 1.0 / fps as f32, vec![], "");
            pending(&mut rack);
            let edge = pos2(599.0, 200.0);
            for index in 0..fps {
                let events = if index == 0 {
                    vec![
                        egui::Event::PointerMoved(edge),
                        pointer(edge, true, egui::Modifiers::NONE),
                    ]
                } else {
                    vec![]
                };
                assert!(direct_frame(&ctx, &mut rack, 1.0 / fps as f32, events, ""));
            }
            assert_eq!(rack.zoom, zoom);
            pans.push(rack.pan);
        }
    }
    for pan in &pans {
        assert!(
            (*pan - pans[0]).length() < 0.005,
            "fps/zoom drift {pan:?} vs {:?}",
            pans[0]
        );
    }
    for dt in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        direct_frame(&ctx, &mut rack, 0.0, vec![], "");
        pending(&mut rack);
        let edge = pos2(599.0, 200.0);
        let before = rack.pan;
        assert!(!direct_frame(
            &ctx,
            &mut rack,
            dt,
            vec![
                egui::Event::PointerMoved(edge),
                pointer(edge, true, egui::Modifiers::NONE)
            ],
            ""
        ));
        assert_eq!(rack.pan, before, "dt {dt}");
    }
    let ctx = egui::Context::default();
    let mut rack = Rack {
        pan: vec2(MAX_PAN - 1.0, 0.0),
        ..Default::default()
    };
    direct_frame(&ctx, &mut rack, 0.0, vec![], "");
    pending(&mut rack);
    let edge = pos2(0.0, 200.0);
    assert!(direct_frame(
        &ctx,
        &mut rack,
        100.0,
        vec![
            egui::Event::PointerMoved(edge),
            pointer(edge, true, egui::Modifiers::NONE)
        ],
        ""
    ));
    assert_eq!(rack.pan.x, MAX_PAN);
    assert!(!direct_frame(&ctx, &mut rack, 0.05, vec![], ""));
    let ctx = egui::Context::default();
    let mut rack = Rack {
        pan: Vec2::ZERO,
        ..Default::default()
    };
    direct_frame(&ctx, &mut rack, 0.0, vec![], "");
    pending(&mut rack);
    assert!(direct_frame(
        &ctx,
        &mut rack,
        100.0,
        vec![
            egui::Event::PointerMoved(edge),
            pointer(edge, true, egui::Modifiers::NONE)
        ],
        ""
    ));
    assert_eq!(
        rack.pan,
        vec2(30.0, 0.0),
        "a stalled frame moves at most 600*.05 screen pixels"
    );
}

fn scene(
    ctx: &egui::Context,
    rack: &mut Rack,
    session: &openpatch::control::Session,
    time: f64,
    events: Vec<egui::Event>,
    focused: bool,
    enabled: bool,
) -> Option<RackAction> {
    let mut action = None;
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 500.0))),
            time: Some(time),
            predicted_dt: 1.0 / 60.0,
            events,
            focused,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    ui.add_enabled_ui(enabled, |ui| {
                        action = rack.show(
                            ui,
                            session.patch(),
                            &openpatch_core::catalog(),
                            48000,
                            session.revision(),
                        );
                    });
                });
        },
    );
    action
}
fn setup() -> (
    tempfile::TempDir,
    openpatch::control::Session,
    egui::Context,
    Rack,
) {
    let dir = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(dir.path()).unwrap();
    let patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[
        {"id":"source","kind":"constant"},{"id":"alternate","kind":"constant"},
        {"id":"target","kind":"gain"},{"id":"distant","kind":"gain"}],
        "cables":[{"id":"one","from":"source","to":"target","input":"signal"},
            {"id":"two","from":"alternate","to":"target","input":"signal"}]}))
    .unwrap();
    session.replace_patch(patch, None).unwrap();
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    rack.restore_layout(&json!({"version":1,"zoom":1,"pan":[24,12],"positions":{
        "source":[0,0],"alternate":[0,420],"target":[180,0],"distant":[1000,0]}}))
        .unwrap();
    scene(&ctx, &mut rack, &session, 0.0, vec![], true, true);
    (dir, session, ctx, rack)
}
fn jack(
    rack: &Rack,
    session: &openpatch::control::Session,
    id: &str,
    port: &str,
    input: bool,
) -> Pos2 {
    rack.port_center_for_smoke(
        rack.viewport.unwrap(),
        session.patch(),
        &openpatch_core::catalog(),
        id,
        port,
        input,
    )
    .unwrap()
}
fn start(ctx: &egui::Context, rack: &mut Rack, session: &openpatch::control::Session, mode: &str) {
    let from = if mode == "new" {
        jack(rack, session, "source", "out", false)
    } else {
        jack(rack, session, "target", "signal", true)
    };
    let modifiers = match mode {
        "new" => egui::Modifiers::COMMAND,
        "clone" => egui::Modifiers {
            command: true,
            shift: true,
            ..Default::default()
        },
        _ => egui::Modifiers::NONE,
    };
    assert!(scene(
        ctx,
        rack,
        session,
        1.0,
        vec![
            egui::Event::PointerMoved(from),
            pointer(from, true, modifiers)
        ],
        true,
        true
    )
    .is_none());
    assert!(scene(
        ctx,
        rack,
        session,
        1.0 + 1.0 / 60.0,
        vec![egui::Event::PointerMoved(from + vec2(30.0, 0.0))],
        true,
        true
    )
    .is_none());
    assert!(rack.dragging_cable, "actual pointer drag {mode}");
    if mode == "bundle" {
        let picked = rack.pending.as_mut().unwrap();
        let other = session
            .patch()
            .cables
            .iter()
            .find(|c| !same_cable(c, picked.original.as_ref().unwrap()))
            .unwrap()
            .clone();
        // This is the same pending selection installed by a reviewed bundle
        // menu; pointer drag and subsequent drop still use the real rack path.
        picked.companions.push(other);
    }
    assert_eq!(
        rack.pending.as_ref().unwrap().operation,
        match mode {
            "new" => CableOperation::Create,
            "clone" => CableOperation::Clone,
            _ => CableOperation::Move,
        }
    );
}

#[test]
fn cable_autopan_new_move_clone_and_bundle_reach_offscreen_jacks_without_graph_edits_until_drop() {
    for mode in ["new", "move", "clone", "bundle"] {
        let (_dir, mut session, ctx, mut rack) = setup();
        let before = session.snapshot();
        let positions = rack.positions.clone();
        assert!(jack(&rack, &session, "distant", "signal", true).x > 800.0);
        start(&ctx, &mut rack, &session, mode);
        let initial_pan = rack.pan;
        let edge = pos2(799.0, 344.0);
        let mut time = 1.0 + 1.0 / 60.0;
        for frame in 0..120 {
            time += 1.0 / 60.0;
            let events = if frame == 0 {
                vec![egui::Event::PointerMoved(edge)]
            } else {
                vec![]
            };
            assert!(
                scene(&ctx, &mut rack, &session, time, events, true, true).is_none(),
                "{mode}: held frame may not edit graph"
            );
            assert_eq!(
                session.snapshot(),
                before,
                "{mode}: view movement cannot edit history/revision/graph"
            );
            assert_eq!(
                rack.positions, positions,
                "{mode}: view pan cannot move modules"
            );
            if jack(&rack, &session, "distant", "signal", true).x < 780.0 {
                break;
            }
        }
        let target = jack(&rack, &session, "distant", "signal", true);
        assert!(
            target.x < 780.0 && rack.viewport.unwrap().contains(target),
            "{mode}: offscreen jack not revealed: {target:?}"
        );
        assert!(rack.pan.x < initial_pan.x - 250.0);
        assert_eq!(rack.pan.y, initial_pan.y);
        let last_shown_pan = rack.pan;
        let action = scene(
            &ctx,
            &mut rack,
            &session,
            time + 1.0 / 60.0,
            vec![pointer(target, false, egui::Modifiers::NONE)],
            true,
            true,
        )
        .expect("drop on last painted jack");
        assert_eq!(
            rack.pan, last_shown_pan,
            "release must use last shown geometry even inside the edge zone"
        );
        assert_eq!(
            session.snapshot(),
            before,
            "proposal has no graph effect until host accepts it"
        );
        assert_eq!(rack.positions, positions);
        run_cable_action(&mut session, action);
        let after = session.snapshot();
        assert_eq!(
            after["revision"].as_u64().unwrap(),
            before["revision"].as_u64().unwrap() + 1
        );
        assert_eq!(
            after["history"]["undo_steps"].as_u64().unwrap(),
            before["history"]["undo_steps"].as_u64().unwrap() + 1
        );
        assert_eq!(
            session
                .patch()
                .cables
                .iter()
                .filter(|c| c.to == "distant")
                .count(),
            if mode == "bundle" { 2 } else { 1 }
        );
        assert_eq!(
            session.patch().cables.len(),
            if matches!(mode, "new" | "clone") {
                3
            } else {
                2
            }
        );
        session.execute("undo", json!({})).unwrap();
        assert_eq!(session.snapshot()["patch"], before["patch"]);
        assert_eq!(
            rack.pan, last_shown_pan,
            "graph history does not undo view pan"
        );
        let layout = rack.layout();
        let mut restored = Rack::default();
        restored.restore_layout(&layout).unwrap();
        assert_eq!(
            restored.pan, last_shown_pan,
            "assistance is persisted only as view state"
        );
    }
}

#[test]
fn cable_autopan_real_gesture_cancellation_stops_view_movement_and_preserves_graph() {
    for gate in ["escape", "blur", "disabled", "shortcuts", "revision"] {
        let (_dir, mut session, ctx, mut rack) = setup();
        let before = session.snapshot();
        start(&ctx, &mut rack, &session, "move");
        let edge = pos2(799.0, 344.0);
        assert!(scene(
            &ctx,
            &mut rack,
            &session,
            1.04,
            vec![egui::Event::PointerMoved(edge)],
            true,
            true
        )
        .is_none());
        let pan = rack.pan;
        let events = if gate == "escape" {
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
        if gate == "shortcuts" {
            rack.set_shortcuts_enabled(false);
        }
        if gate == "revision" {
            session
                .execute(
                    "set_parameter",
                    json!({"id":"source","parameter":"value","value":2}),
                )
                .unwrap();
        }
        let guarded = session.snapshot();
        assert!(scene(
            &ctx,
            &mut rack,
            &session,
            1.06,
            events,
            gate != "blur",
            gate != "disabled"
        )
        .is_none());
        assert_eq!(rack.pan, pan, "{gate}: no extra movement on canceled frame");
        assert!(
            rack.pending.is_none() && !rack.dragging_cable,
            "{gate}: draft canceled"
        );
        assert_eq!(session.snapshot(), guarded);
        assert_eq!(
            session.patch().cables.len(),
            before["patch"]["cables"].as_array().unwrap().len()
        );
    }
}
