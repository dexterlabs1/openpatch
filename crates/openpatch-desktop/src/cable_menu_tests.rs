// Included within rack::tests; every menu case uses native pointer events.
mod cable_menu_acceptance {
    use super::*;
    use openpatch::control::Session;

    struct Host {
        workspace: tempfile::TempDir,
        session: Session,
        ctx: egui::Context,
        rack: Rack,
        time: f64,
    }

    impl Host {
        fn new(patch: Patch) -> Self {
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
            Self {
                workspace,
                session,
                ctx,
                rack,
                time: 0.0,
            }
        }
        fn frame(&mut self, events: Vec<egui::Event>) -> (Option<RackAction>, egui::FullOutput) {
            self.time += 0.1;
            frame_revision_state(
                &self.ctx,
                &mut self.rack,
                self.session.patch(),
                self.time,
                events,
                true,
                true,
                self.session.revision(),
            )
        }
        fn jack(&self, id: &str, port: &str, input: bool) -> Pos2 {
            fixture_jack(&self.rack, self.session.patch(), id, port, input)
        }
        fn open(&mut self, input: bool) -> egui::FullOutput {
            let point = self.jack(
                if input { "target" } else { "src" },
                if input { "signal" } else { "out" },
                input,
            );
            self.open_point(point)
        }
        fn open_point(&mut self, point: Pos2) -> egui::FullOutput {
            for pressed in [true, false] {
                assert!(self
                    .frame(vec![
                        egui::Event::PointerMoved(point),
                        egui::Event::PointerButton {
                            pos: point,
                            button: PointerButton::Secondary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        }
                    ])
                    .0
                    .is_none());
            }
            let (action, output) = self.frame(vec![]);
            assert!(action.is_none());
            output
        }
        fn click(
            &mut self,
            output: &egui::FullOutput,
            label: &str,
        ) -> (Option<RackAction>, egui::FullOutput) {
            self.click_point(menu_label(output, label))
        }
        fn click_point(&mut self, point: Pos2) -> (Option<RackAction>, egui::FullOutput) {
            let mut action = None;
            for pressed in [true, false] {
                let (next, _) = self.frame(vec![
                    egui::Event::PointerMoved(point),
                    button(point, pressed),
                ]);
                if next.is_some() {
                    assert!(action.is_none(), "one menu click emits one action");
                    action = next;
                }
            }
            let (next, output) = self.frame(vec![]);
            assert!(next.is_none());
            (action, output)
        }
        fn drop_at(&mut self, point: Pos2) -> Option<RackAction> {
            assert!(self
                .frame(vec![egui::Event::PointerMoved(point)])
                .0
                .is_none());
            // Click-mode pending cables use the same captured menu intent as drags.
            self.click_point(point).0
        }
        fn drag_at(&mut self, from: Pos2, to: Pos2) -> Option<RackAction> {
            assert!(self
                .frame(vec![egui::Event::PointerMoved(from), button(from, true)])
                .0
                .is_none());
            assert!(self.frame(vec![egui::Event::PointerMoved(to)]).0.is_none());
            self.frame(vec![button(to, false)]).0
        }
        fn assert_undo(&mut self, before: &Value) {
            let after = self.session.snapshot();
            assert_eq!(after["revision"], before["revision"].as_u64().unwrap() + 1);
            assert_eq!(
                after["history"]["undo_steps"],
                before["history"]["undo_steps"].as_u64().unwrap() + 1
            );
            self.session.execute("undo", json!({})).unwrap();
            assert_eq!(self.session.snapshot()["patch"], before["patch"]);
            self.session.execute("redo", json!({})).unwrap();
            assert_eq!(self.session.snapshot()["patch"], after["patch"]);
        }
    }

    fn menu_label(output: &egui::FullOutput, prefix: &str) -> Pos2 {
        fn find(shape: &egui::Shape, prefix: &str) -> Option<Pos2> {
            match shape {
                egui::Shape::Text(text) if text.galley.text().starts_with(prefix) => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                egui::Shape::Vec(children) => children.iter().find_map(|shape| find(shape, prefix)),
                _ => None,
            }
        }
        output
            .shapes
            .iter()
            .find_map(|shape| find(&shape.shape, prefix))
            .unwrap_or_else(|| panic!("missing cable menu label {prefix}"))
    }

    fn fanout_fixture() -> Patch {
        serde_json::from_value(json!({"version":2,"nodes":[
            {"id":"src","kind":"constant"},{"id":"alt","kind":"constant"},
            {"id":"target","kind":"gain"},{"id":"dest","kind":"gain"}
        ],"cables":[
            {"id":"one","from":"src","to":"target","input":"signal","color":[20,50,90]},
            {"id":"two","from":"src","to":"dest","input":"signal","color":[140,90,30]},
            {"id":"destination_sibling","from":"alt","to":"target","input":"signal"}
        ],"output":"target"}))
        .unwrap()
    }

    #[test]
    fn input_and_output_menus_delete_top_or_all_by_id_in_one_history_step() {
        for input in [false, true] {
            for all in [false, true] {
                let mut host = Host::new(stacked_fixture());
                let before = host.session.snapshot();
                let output = host.open(input);
                for label in [
                    "Delete top",
                    "Clone top",
                    "Create new",
                    "Move all",
                    "Delete all",
                ] {
                    menu_label(&output, label);
                }
                let (action, _) =
                    host.click(&output, if all { "Delete all" } else { "Delete top" });
                run_cable_action(&mut host.session, action.expect("menu delete"));
                if all {
                    assert!(host.session.patch().cables.is_empty());
                } else {
                    assert_eq!(host.session.patch().cables.len(), 1);
                    assert_eq!(host.session.patch().cables[0].id.as_deref(), Some("one"));
                }
                assert!(host.rack.pending.is_none());
                host.assert_undo(&before);
            }
        }
    }

    #[test]
    fn output_menu_moves_or_clones_selected_non_top_cable_with_its_effective_color() {
        for clone in [false, true] {
            let mut patch = fanout_fixture();
            patch.cables.truncate(2);
            let mut host = Host::new(patch);
            let before = host.session.snapshot();
            let original = host.session.patch().cables[0].clone();
            let color = cable_instance_color(&original);
            let menu = host.open(false);
            let (action, submenu) = host.click(&menu, "one · ");
            assert!(action.is_none());
            let (action, _) = host.click(
                &submenu,
                if clone {
                    "Clone this cable"
                } else {
                    "Move this cable"
                },
            );
            assert!(action.is_none());
            let to = host.jack("alt", "out", false);
            let action = host.drop_at(to).expect("selected output endpoint plan");
            run_cable_action(&mut host.session, action);
            assert_eq!(host.session.patch().cables.len(), if clone { 3 } else { 2 });
            let moved = host
                .session
                .patch()
                .cables
                .iter()
                .find(|cable| cable.from == "alt")
                .unwrap();
            assert_eq!((&*moved.to, &*moved.input), ("target", "signal"));
            assert_eq!(cable_instance_color(moved), color);
            if clone {
                assert_ne!(moved.id, original.id);
                assert!(host
                    .session
                    .patch()
                    .cables
                    .iter()
                    .any(|cable| same_cable(cable, &original)));
            } else {
                assert_eq!(moved.id, original.id);
            }
            assert!(host
                .session
                .patch()
                .cables
                .iter()
                .any(|cable| cable.id.as_deref() == Some("two")
                    && cable.from == "src"
                    && cable.to == "dest"));
            host.assert_undo(&before);
        }
    }

    #[test]
    fn move_all_menu_discards_only_duplicate_members_preserves_siblings_colors_and_one_undo() {
        for input in [false, true] {
            let mut host = Host::new(fanout_fixture());
            let before = host.session.snapshot();
            let layout = host.rack.layout();
            let menu = host.open(input);
            let (action, _) = host.click(&menu, "Move all");
            assert!(action.is_none());
            let to = host.jack(
                if input { "dest" } else { "alt" },
                if input { "signal" } else { "out" },
                input,
            );
            let action = host.drop_at(to).expect("group move");
            run_cable_action(&mut host.session, action);
            let after = host.session.snapshot();
            assert_eq!(host.session.patch().cables.len(), 2);
            assert!(!host
                .session
                .patch()
                .cables
                .iter()
                .any(|cable| cable.id.as_deref() == Some("one")));
            for id in ["two", "destination_sibling"] {
                let old = before["patch"]["cables"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|cable| cable["id"] == id)
                    .unwrap();
                let cable = host
                    .session
                    .patch()
                    .cables
                    .iter()
                    .find(|cable| cable.id.as_deref() == Some(id))
                    .unwrap();
                assert_eq!(serde_json::to_value(cable).unwrap()["color"], old["color"]);
                assert_eq!(
                    cable.to,
                    if input || id == "two" {
                        "dest"
                    } else {
                        "target"
                    }
                );
                assert_eq!(cable.from, if input && id == "two" { "src" } else { "alt" });
            }
            assert_eq!(after["patch"]["nodes"], before["patch"]["nodes"]);
            assert_eq!(host.rack.layout(), layout);
            host.session
                .execute("save_patch", json!({"filename":"group-menu.json"}))
                .unwrap();
            let saved: Patch = serde_json::from_slice(
                &std::fs::read(host.workspace.path().join("group-menu.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(serde_json::to_value(saved).unwrap(), after["patch"]);
            host.assert_undo(&before);
        }
    }

    #[test]
    fn move_all_bad_drop_deletes_only_group_while_same_origin_escape_focus_and_revision_cancel() {
        for input in [false, true] {
            for outcome in ["empty", "wrong", "same", "escape", "focus", "stale"] {
                let mut host = Host::new(fanout_fixture());
                let menu = host.open(input);
                assert!(host.click(&menu, "Move all").0.is_none());
                let from = host.jack(
                    if input { "target" } else { "src" },
                    if input { "signal" } else { "out" },
                    input,
                );
                let to = match outcome {
                    "same" => from,
                    "wrong" => host.jack(
                        if input { "alt" } else { "dest" },
                        if input { "out" } else { "signal" },
                        !input,
                    ),
                    _ => pos2(1000.0, 600.0),
                };
                let before = host.session.snapshot();
                let action = if outcome == "same" {
                    assert!(host
                        .frame(vec![egui::Event::PointerMoved(from), button(from, true)])
                        .0
                        .is_none());
                    assert!(host
                        .frame(vec![egui::Event::PointerMoved(from + vec2(30.0, 0.0))])
                        .0
                        .is_none());
                    host.frame(vec![egui::Event::PointerMoved(from), button(from, false)])
                        .0
                } else if matches!(outcome, "empty" | "wrong") {
                    host.drag_at(from, to)
                } else {
                    assert!(host
                        .frame(vec![egui::Event::PointerMoved(from), button(from, true)])
                        .0
                        .is_none());
                    assert!(host.frame(vec![egui::Event::PointerMoved(to)]).0.is_none());
                    if outcome == "stale" {
                        host.session
                            .execute(
                                "set_parameter",
                                json!({"id":"alt","parameter":"value","value":0.25}),
                            )
                            .unwrap();
                    }
                    let events = if outcome == "escape" {
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
                    host.time += 0.1;
                    assert!(frame_revision_state(
                        &host.ctx,
                        &mut host.rack,
                        host.session.patch(),
                        host.time,
                        events,
                        outcome != "focus",
                        true,
                        host.session.revision()
                    )
                    .0
                    .is_none());
                    host.frame(vec![button(to, false)]).0
                };
                if matches!(outcome, "empty" | "wrong") {
                    run_cable_action(
                        &mut host.session,
                        action.expect("bad group drop deletes selected group"),
                    );
                    let remaining = &host.session.patch().cables;
                    assert_eq!(remaining.len(), 1);
                    assert_eq!(
                        remaining[0].id.as_deref(),
                        Some(if input { "two" } else { "destination_sibling" })
                    );
                    host.assert_undo(&before);
                } else {
                    assert!(action.is_none(), "{input}: {outcome}");
                    assert_eq!(
                        host.session.snapshot()["patch"]["cables"],
                        before["patch"]["cables"]
                    );
                    if outcome != "stale" {
                        assert_eq!(host.session.snapshot(), before);
                    }
                }
                assert!(host.rack.pending.is_none());
            }
        }
    }

    #[test]
    fn menu_snapshot_revision_guard_blocks_old_buttons_after_edit_and_reused_cable_id() {
        for input in [false, true] {
            for reuse in [false, true] {
                let mut host = Host::new(stacked_fixture());
                let menu = host.open(input);
                let delete = menu_label(&menu, "Delete top");
                if reuse {
                    host.session.execute("apply_edits", json!({"edits":[
                        {"tool":"disconnect","arguments":{"cable_id":"two"}},
                        {"tool":"connect","arguments":{"cable_id":"two","from":"alt","to":"target","input":"signal","mode":"stack"}}
                    ]})).unwrap();
                } else {
                    host.session
                        .execute(
                            "set_parameter",
                            json!({"id":"alt","parameter":"value","value":0.25}),
                        )
                        .unwrap();
                }
                let current = host.session.snapshot();
                assert!(host.click_point(delete).0.is_none());
                assert_eq!(host.session.snapshot(), current);
                assert!(host.rack.pending.is_none());
            }
        }
    }

    #[test]
    fn per_cable_palette_recolors_only_selected_id_from_either_port_and_undo_restores_rgb() {
        for input in [false, true] {
            let mut patch = fanout_fixture();
            // Place both selected and sibling cables at both tested endpoints.
            patch.cables[1].to = "target".into();
            patch.cables.truncate(2);
            let mut host = Host::new(patch);
            let before = host.session.snapshot();
            let menu = host.open(input);
            let (action, submenu) = host.click(&menu, "one · ");
            assert!(action.is_none());
            let (action, _) = host.click(&submenu, "● Teal");
            run_cable_action(&mut host.session, action.expect("palette recolor"));
            let after = host.session.snapshot();
            let teal = CABLES[2];
            assert_eq!(
                after["patch"]["cables"][0]["color"],
                json!([teal.r(), teal.g(), teal.b()])
            );
            assert_eq!(after["patch"]["cables"][1], before["patch"]["cables"][1]);
            assert_eq!(after["patch"]["nodes"], before["patch"]["nodes"]);
            host.assert_undo(&before);
        }
    }

    #[test]
    fn custom_rgb_fields_require_apply_and_change_only_reviewed_cable() {
        let mut host = Host::new(fanout_fixture());
        let before = host.session.snapshot();
        let menu = host.open(false);
        let (action, submenu) = host.click(&menu, "one · ");
        assert!(action.is_none());
        let (action, custom) = host.click(&submenu, "Custom RGB");
        assert!(action.is_none());
        let red = menu_label(&custom, "20");
        for pressed in [true, false, true, false] {
            assert!(host
                .frame(vec![egui::Event::PointerMoved(red), button(red, pressed)])
                .0
                .is_none());
        }
        assert!(host
            .frame(vec![
                egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::COMMAND
                },
                egui::Event::Text("17".into()),
                egui::Event::Key {
                    key: egui::Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE
                },
            ])
            .0
            .is_none());
        assert_eq!(
            host.session.snapshot(),
            before,
            "editing custom RGB is a local draft until Apply"
        );
        let (_, output) = host.frame(vec![]);
        let (action, _) = host.click(&output, "Apply colour");
        run_cable_action(&mut host.session, action.expect("explicit RGB Apply"));
        let after = host.session.snapshot();
        assert_eq!(after["patch"]["cables"][0]["color"], json!([17, 50, 90]));
        assert_eq!(after["patch"]["cables"][1], before["patch"]["cables"][1]);
        assert_eq!(after["patch"]["cables"][2], before["patch"]["cables"][2]);
        host.assert_undo(&before);
    }

    #[test]
    fn menu_chosen_new_cable_rgb_survives_press_drag_from_occupied_and_empty_ends() {
        for input in [false, true] {
            for occupied in [false, true] {
                let mut host = Host::new(stacked_fixture());
                let before = host.session.snapshot();
                let from = host.jack(
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
                let menu = host.open_point(from);
                let (action, palette) = host.click(&menu, "New cable colour");
                assert!(action.is_none());
                assert!(host.click(&palette, "● Teal").0.is_none());
                let to = host.jack(
                    if input {
                        if occupied {
                            "alt"
                        } else {
                            "src"
                        }
                    } else if occupied {
                        "dest"
                    } else {
                        "target"
                    },
                    if input { "out" } else { "signal" },
                    !input,
                );
                let action = host
                    .drag_at(from, to)
                    .expect("fresh selected-color cable drag");
                run_cable_action(&mut host.session, action);
                assert_eq!(host.session.patch().cables.len(), 3);
                let after = host.session.snapshot();
                assert_eq!(after["patch"]["cables"][0], before["patch"]["cables"][0]);
                assert_eq!(after["patch"]["cables"][1], before["patch"]["cables"][1]);
                let teal = CABLES[2];
                assert_eq!(
                    after["patch"]["cables"][2]["color"],
                    json!([teal.r(), teal.g(), teal.b()])
                );
                host.assert_undo(&before);
            }
        }
    }

    #[test]
    fn pickup_menu_rejects_modifier_press_even_when_released_before_button_activation() {
        for modifiers in [
            egui::Modifiers::SHIFT,
            egui::Modifiers::COMMAND,
            egui::Modifiers::CTRL,
        ] {
            let mut host = Host::new(stacked_fixture());
            let before = host.session.snapshot();
            let menu = host.open(false);
            let point = menu_label(&menu, "Clone top");
            assert!(host
                .frame(vec![
                    egui::Event::PointerMoved(point),
                    egui::Event::PointerButton {
                        pos: point,
                        button: PointerButton::Primary,
                        pressed: true,
                        modifiers,
                    }
                ])
                .0
                .is_none());
            assert!(host.frame(vec![button(point, false)]).0.is_none());
            assert!(host.frame(vec![]).0.is_none());
            assert!(
                host.rack.pending.is_none(),
                "modified menu press cannot become a clone on modifierless release"
            );
            assert_eq!(host.session.snapshot(), before);
        }
    }

    #[test]
    fn pickup_menu_cannot_recapture_draft_after_same_frame_blur_refocus_or_escape() {
        for interrupt in ["blur_refocus", "escape"] {
            for label in ["Clone top", "Create new", "Move all"] {
                let mut host = Host::new(stacked_fixture());
                let before = host.session.snapshot();
                let menu = host.open(false);
                let point = menu_label(&menu, label);
                let mut events = vec![egui::Event::PointerMoved(point)];
                if interrupt == "blur_refocus" {
                    events.extend([
                        egui::Event::WindowFocused(false),
                        egui::Event::WindowFocused(true),
                    ]);
                } else {
                    events.push(egui::Event::Key {
                        key: egui::Key::Escape,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    });
                }
                events.push(button(point, true));
                assert!(host.frame(events).0.is_none());
                assert!(host.frame(vec![button(point, false)]).0.is_none());
                assert!(host.rack.pending.is_none(), "{interrupt}: {label}");
                assert_eq!(host.session.snapshot(), before);
            }
        }
    }

    #[test]
    fn dragging_menu_pickup_items_closes_all_ancestors_and_commits_once_at_target_release() {
        for input in [false, true] {
            for operation in ["all", "clone", "colored"] {
                let mut host = Host::new(if operation == "all" {
                    fanout_fixture()
                } else {
                    stacked_fixture()
                });
                let before = host.session.snapshot();
                let menu = host.open(input);
                let point = if operation == "colored" {
                    let (action, palette) = host.click(&menu, "New cable colour");
                    assert!(action.is_none());
                    menu_label(&palette, "● Teal")
                } else {
                    menu_label(
                        &menu,
                        if operation == "all" {
                            "Move all"
                        } else {
                            "Clone top"
                        },
                    )
                };
                assert!(host
                    .frame(vec![egui::Event::PointerMoved(point), button(point, true)])
                    .0
                    .is_none());
                // Start the drag while still over the menu item, then travel to
                // the rack in a later frame after every popup ancestor closes.
                assert!(host
                    .frame(vec![egui::Event::PointerMoved(point + vec2(10.0, 0.0))])
                    .0
                    .is_none());
                assert!(
                    !egui::Popup::is_any_open(&host.ctx),
                    "{input}: {operation} leaves no ancestor popup"
                );
                assert!(host.rack.pending.is_some());
                let target_input = if operation == "colored" {
                    !input
                } else {
                    input
                };
                let target_node = if operation == "colored" {
                    if input {
                        "alt"
                    } else {
                        "dest"
                    }
                } else if input {
                    "dest"
                } else {
                    "alt"
                };
                let to = host.jack(
                    target_node,
                    if target_input { "signal" } else { "out" },
                    target_input,
                );
                assert!(host.frame(vec![egui::Event::PointerMoved(to)]).0.is_none());
                assert!(
                    host.rack.pending.is_some(),
                    "menu drag survives entering the rack"
                );
                assert_eq!(host.session.snapshot(), before);
                let action = host
                    .frame(vec![button(to, false)])
                    .0
                    .expect("menu drag release commits once");
                run_cable_action(&mut host.session, action);
                let after = host.session.snapshot();
                if operation == "all" {
                    assert_eq!(host.session.patch().cables.len(), 2);
                    assert!(!host
                        .session
                        .patch()
                        .cables
                        .iter()
                        .any(|cable| cable.id.as_deref() == Some("one")));
                    let moved_id = if input { "destination_sibling" } else { "two" };
                    let moved = host
                        .session
                        .patch()
                        .cables
                        .iter()
                        .find(|cable| cable.id.as_deref() == Some(moved_id))
                        .unwrap();
                    assert_eq!((&*moved.from, &*moved.to), ("alt", "dest"));
                } else {
                    assert_eq!(host.session.patch().cables.len(), 3);
                    assert_eq!(after["patch"]["cables"][0], before["patch"]["cables"][0]);
                    assert_eq!(after["patch"]["cables"][1], before["patch"]["cables"][1]);
                    let added = &host.session.patch().cables[2];
                    assert!(!matches!(added.id.as_deref(), Some("one" | "two")));
                    assert_eq!(
                        (&*added.from, &*added.to),
                        if operation == "colored" {
                            if input {
                                ("alt", "target")
                            } else {
                                ("src", "dest")
                            }
                        } else if input {
                            ("src", "dest")
                        } else {
                            ("alt", "target")
                        }
                    );
                    if operation == "colored" {
                        let teal = CABLES[2];
                        assert_eq!(
                            after["patch"]["cables"][2]["color"],
                            json!([teal.r(), teal.g(), teal.b()])
                        );
                    }
                }
                assert!(host.rack.pending.is_none());
                assert!(
                    host.frame(vec![]).0.is_none(),
                    "release cannot emit a second operation"
                );
                host.assert_undo(&before);
            }
        }
    }

    fn opposite_tops_fixture() -> Patch {
        let mut encoded = serde_json::to_value(stacked_fixture()).unwrap();
        encoded["cables"][0]["input_order"] = json!(2);
        encoded["cables"][0]["output_order"] = json!(1);
        encoded["cables"][0]["color"] = json!([20, 50, 90]);
        encoded["cables"][1]["input_order"] = json!(1);
        encoded["cables"][1]["output_order"] = json!(2);
        encoded["cables"][1]["color"] = json!([140, 90, 30]);
        serde_json::from_value(encoded).unwrap()
    }

    #[test]
    fn independent_endpoint_tops_drive_shift_delete_menu_delete_and_clone_top() {
        for input in [false, true] {
            for operation in ["shift", "delete", "clone"] {
                let mut host = Host::new(opposite_tops_fixture());
                let before = host.session.snapshot();
                let top_id = if input { "one" } else { "two" };
                let from = host.jack(
                    if input { "target" } else { "src" },
                    if input { "signal" } else { "out" },
                    input,
                );
                let action = if operation == "shift" {
                    host.frame(vec![
                        egui::Event::PointerMoved(from),
                        egui::Event::PointerButton {
                            pos: from,
                            button: PointerButton::Primary,
                            pressed: true,
                            modifiers: egui::Modifiers::SHIFT,
                        },
                    ])
                    .0
                } else {
                    let menu = host.open(input);
                    let (action, _) = host.click(
                        &menu,
                        if operation == "clone" {
                            "Clone top"
                        } else {
                            "Delete top"
                        },
                    );
                    if operation == "clone" {
                        assert!(action.is_none());
                        let to = host.jack(
                            if input { "dest" } else { "alt" },
                            if input { "signal" } else { "out" },
                            input,
                        );
                        host.drop_at(to)
                    } else {
                        action
                    }
                };
                run_cable_action(
                    &mut host.session,
                    action.expect("endpoint-specific top operation"),
                );
                if operation == "clone" {
                    assert_eq!(host.session.patch().cables.len(), 3);
                    assert_eq!(
                        host.session.snapshot()["patch"]["cables"][0],
                        before["patch"]["cables"][0]
                    );
                    assert_eq!(
                        host.session.snapshot()["patch"]["cables"][1],
                        before["patch"]["cables"][1]
                    );
                    let source = before["patch"]["cables"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|c| c["id"] == top_id)
                        .unwrap();
                    assert_eq!(
                        host.session.snapshot()["patch"]["cables"][2]["color"],
                        source["color"]
                    );
                } else {
                    assert_eq!(host.session.patch().cables.len(), 1);
                    assert_ne!(host.session.patch().cables[0].id.as_deref(), Some(top_id));
                }
                host.assert_undo(&before);
            }
        }
    }

    #[test]
    fn selected_non_top_same_origin_promotes_only_clicked_end_and_cancel_discards_provisional_order(
    ) {
        for cancel in [false, true] {
            let mut host = Host::new(opposite_tops_fixture());
            let before = host.session.snapshot();
            let menu = host.open(false);
            let (_, submenu) = host.click(&menu, "one · ");
            assert!(host.click(&submenu, "Move this cable").0.is_none());
            assert_eq!(
                host.session.snapshot(),
                before,
                "pickup promotion is provisional"
            );
            let from = host.jack("src", "out", false);
            if cancel {
                assert!(host
                    .frame(vec![egui::Event::Key {
                        key: egui::Key::Escape,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE
                    }])
                    .0
                    .is_none());
                assert!(host.rack.pending.is_none());
                assert_eq!(host.session.snapshot(), before);
            } else {
                let action = host
                    .drop_at(from)
                    .expect("non-top same-origin return commits only plug promotion");
                assert!(
                    matches!(&action, RackAction::Command("promote_cables", args) if args["end"] == "output" && args["cable_ids"] == json!(["one"]))
                );
                run_cable_action(&mut host.session, action);
                let after = host.session.snapshot();
                assert_eq!(after["patch"]["cables"][0]["id"], "one");
                assert_eq!(after["patch"]["cables"][1]["id"], "two");
                for index in 0..2 {
                    assert_eq!(
                        after["patch"]["cables"][index]["input_order"],
                        before["patch"]["cables"][index]["input_order"]
                    );
                }
                let end = CableEnd::Output {
                    node: "src".into(),
                    port: "out".into(),
                };
                assert_eq!(
                    ordered_endpoint_cables(host.session.patch(), &end)
                        .last()
                        .unwrap()
                        .id
                        .as_deref(),
                    Some("one")
                );
                host.assert_undo(&before);
            }
        }
        let mut host = Host::new(opposite_tops_fixture());
        let before = host.session.snapshot();
        let menu = host.open(false);
        let (_, submenu) = host.click(&menu, "two · ");
        assert!(host.click(&submenu, "Move this cable").0.is_none());
        let from = host.jack("src", "out", false);
        assert!(
            host.drop_at(from).is_none(),
            "returning an already-top plug is a true no-op"
        );
        assert_eq!(host.session.snapshot(), before);
    }

    #[test]
    fn move_all_endpoint_plug_order_does_not_change_native_array_duplicate_winner() {
        let mut host = Host::new(opposite_tops_fixture());
        let before = host.session.snapshot();
        let menu = host.open(true);
        assert!(host.click(&menu, "Move all").0.is_none());
        let to = host.jack("dest", "signal", true);
        let action = host.drop_at(to).expect("ordered bundle move");
        assert!(matches!(&action, RackAction::Command("move_cables", args)
            if args["cable_ids"] == json!(["one", "two"]) && args["plug_order"] == json!(["two", "one"])));
        run_cable_action(&mut host.session, action);
        assert_eq!(host.session.patch().cables.len(), 1);
        assert_eq!(host.session.patch().cables[0].id.as_deref(), Some("one"));
        assert_eq!(host.session.patch().cables[0].to, "dest");
        assert_eq!(host.session.patch().cables[0].output_order, Some(1));
        host.assert_undo(&before);
    }

    #[test]
    fn independently_ordered_caps_keep_curves_in_array_order_and_move_preview_preserves_opposite_top(
    ) {
        fn cap_color(output: &egui::FullOutput, point: Pos2) -> Color32 {
            output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Circle(circle)
                        if circle.center == point && (circle.radius - 6.1).abs() < 0.001 =>
                    {
                        Some(circle.fill)
                    }
                    _ => None,
                })
                .next_back()
                .expect("cable cap fill")
        }
        let mut host = Host::new(opposite_tops_fixture());
        let input = host.jack("target", "signal", true);
        let output = host.jack("src", "out", false);
        let one = cable_instance_color(&host.session.patch().cables[0]);
        let two = cable_instance_color(&host.session.patch().cables[1]);
        let (_, rendered) = host.frame(vec![egui::Event::PointerMoved(pos2(1000.0, 600.0))]);
        assert_eq!(cap_color(&rendered, input), one.gamma_multiply(0.58));
        assert_eq!(cap_color(&rendered, output), two.gamma_multiply(0.58));
        let curves: Vec<_> = rendered
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::CubicBezier(curve)
                    if curve.points[0] == output
                        && curve.points[3] == input
                        && (curve.stroke.width - 6.0).abs() < 0.001 =>
                {
                    Some(curve.stroke.color.clone())
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            curves,
            vec![
                egui::epaint::ColorMode::Solid(one.gamma_multiply(0.64)),
                egui::epaint::ColorMode::Solid(two.gamma_multiply(0.64))
            ]
        );
        for (point, expected) in [(input, one), (output, two)] {
            let (_, hovered) = host.frame(vec![egui::Event::PointerMoved(point)]);
            let light = hovered
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Circle(circle)
                        if circle.center == point && (circle.radius - 11.4).abs() < 0.001 =>
                    {
                        Some(circle.stroke.color)
                    }
                    _ => None,
                })
                .next_back()
                .expect("top plug light");
            assert_eq!(light, expected);
        }
        let before = host.session.snapshot();
        // The grabbed IN plug is one; its fixed OUT plug is below two and must
        // not be repainted over it by the moving cable's preview.
        assert!(host
            .frame(vec![egui::Event::PointerMoved(input), button(input, true)])
            .0
            .is_none());
        let (_, preview) = host.frame(vec![egui::Event::PointerMoved(pos2(900.0, 550.0))]);
        assert_eq!(cap_color(&preview, output), two.gamma_multiply(0.58));
        assert_eq!(host.session.snapshot(), before);
        host.frame(vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        assert_eq!(host.session.snapshot(), before);
    }
}
