use super::*;

struct Scene {
    ctx: egui::Context,
    rack: Rack,
    patch: Patch,
    revision: u64,
    time: f64,
}

impl Scene {
    fn blank(zoom: f32, pan: Vec2, origin: Pos2) -> Self {
        let mut scene = Self {
            ctx: egui::Context::default(),
            rack: Rack::default(),
            patch: serde_json::from_value(
                json!({"version":2,"nodes":[{"id":"blank","kind":"blank"}]}),
            )
            .unwrap(),
            revision: 17,
            time: 0.0,
        };
        scene.rack.zoom = zoom;
        scene.rack.pan = pan;
        scene.rack.positions.insert("blank".into(), origin);
        scene.frame(vec![]);
        scene
    }

    fn notes() -> Self {
        let mut scene = Self::blank(1.0, Vec2::ZERO, pos2(24.0, 12.0));
        scene.patch = serde_json::from_value(json!({"version":2,"nodes":[{
            "id":"notes","kind":"notes","content":{"type":"notes","text":"Saved 日本語\n🎛️"}
        }]}))
        .unwrap();
        scene.frame(vec![]);
        scene
            .rack
            .open_annotation_editor(&scene.patch.nodes[0], scene.revision);
        scene.frame(vec![]);
        scene.frame(vec![]);
        scene
    }

    fn frame(&mut self, events: Vec<egui::Event>) -> (Option<RackAction>, egui::FullOutput) {
        self.time += 0.1;
        frame_revision_state(
            &self.ctx,
            &mut self.rack,
            &self.patch,
            self.time,
            events,
            true,
            true,
            self.revision,
        )
    }

    fn edge(&self, right: bool) -> Pos2 {
        let node = &self.patch.nodes[0];
        let (origin, width) = self.rack.annotation_geometry(node);
        self.rack.viewport.unwrap().min
            + self.rack.pan
            + origin.to_vec2() * self.rack.zoom
            + vec2(if right { width - 4.0 } else { 4.0 }, 120.0) * self.rack.zoom
    }

    fn press(&mut self, position: Pos2) {
        assert!(self
            .frame(vec![
                egui::Event::PointerMoved(position),
                button(position, true)
            ])
            .0
            .is_none());
    }

    fn move_to(&mut self, position: Pos2) {
        assert!(self
            .frame(vec![egui::Event::PointerMoved(position)])
            .0
            .is_none());
    }

    fn click_label(&mut self, label: &str) -> Option<RackAction> {
        let output = self.frame(vec![]).1;
        let target = labels(&output)
            .into_iter()
            .find(|(text, _)| text == label)
            .unwrap_or_else(|| panic!("missing label {label}: {:?}", labels(&output)))
            .1
            .center();
        self.press(target);
        self.frame(vec![button(target, false)]).0
    }

    fn edit(&self) -> &AnnotationEdit {
        self.rack.annotation_edit.as_ref().unwrap()
    }
}

fn labels(output: &egui::FullOutput) -> Vec<(String, Rect)> {
    fn visit(shape: &egui::Shape, labels: &mut Vec<(String, Rect)>) {
        match shape {
            egui::Shape::Text(text) => labels.push((
                text.galley.text().to_owned(),
                Rect::from_min_size(text.pos, text.galley.size()),
            )),
            egui::Shape::Vec(children) => children.iter().for_each(|shape| visit(shape, labels)),
            _ => {}
        }
    }
    let mut result = vec![];
    output
        .shapes
        .iter()
        .for_each(|shape| visit(&shape.shape, &mut result));
    result
}

fn key_event(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

fn resize(action: Option<RackAction>) -> BlankResizeProposal {
    match action {
        Some(RackAction::ResizeBlank(proposal)) => proposal,
        _ => panic!("expected one Blank resize proposal"),
    }
}

fn sidebar_frame(
    scene: &mut Scene,
    events: Vec<egui::Event>,
) -> (Option<RackAction>, egui::FullOutput) {
    scene.time += 0.1;
    let mut action = None;
    let output = scene.ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 700.0))),
            time: Some(scene.time),
            events,
            focused: true,
            ..Default::default()
        },
        |ctx| {
            egui::SidePanel::left("annotation-test-sidebar")
                .exact_width(240.0)
                .show(ctx, |ui| {
                    ui.label("Browser sidebar");
                });
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    action = scene.rack.show(
                        ui,
                        &scene.patch,
                        &openpatch_core::catalog(),
                        48000,
                        scene.revision,
                    );
                });
        },
    );
    (action, output)
}

#[test]
fn annotation_blank_release_after_preview_leaves_canvas_clears_gesture() {
    let mut scene = Scene::blank(1.0, Vec2::ZERO, pos2(24.0, 40.0));
    scene.patch.nodes[0].content = Some(openpatch_core::ModuleContent::Blank { width_hp: 20 });
    scene
        .rack
        .positions
        .insert("blank".into(), pos2(-150.0, 40.0));
    scene.rack.viewport = None;
    sidebar_frame(&mut scene, vec![]);
    sidebar_frame(&mut scene, vec![]);
    let canvas = scene.rack.viewport.unwrap();
    let start = scene.edge(true);
    assert!(
        canvas.contains(start),
        "the initial right edge must be visible"
    );
    assert!(sidebar_frame(
        &mut scene,
        vec![egui::Event::PointerMoved(start), button(start, true)]
    )
    .0
    .is_none());
    let target = start - vec2(17.0 * HP, 0.0);
    assert!(
        target.x > 0.0 && target.x < canvas.left(),
        "release stays inside the app's adjacent sidebar"
    );
    assert!(
        sidebar_frame(&mut scene, vec![egui::Event::PointerMoved(target)])
            .0
            .is_none()
    );
    let drag = scene
        .rack
        .blank_resize
        .as_ref()
        .expect("a resize preview is active");
    assert_eq!(drag.proposal.width_hp, 3);
    let (position, width) = scene.rack.annotation_geometry(&scene.patch.nodes[0]);
    let preview = Rect::from_min_size(canvas.min + position.to_vec2(), vec2(width, PANEL_HEIGHT));
    assert!(
        !preview.intersects(canvas),
        "the shrunken preview is entirely outside the rack"
    );
    assert!(!scene.rack.navigation_idle());
    sidebar_frame(&mut scene, vec![button(target, false)]);
    assert!(
        scene.rack.blank_resize.is_none(),
        "release must finish or cancel a culled resize preview"
    );
    assert!(
        scene.rack.navigation_idle(),
        "offscreen release must restore rack navigation"
    );
    assert!(
        sidebar_frame(&mut scene, vec![]).0.is_none(),
        "a release cannot emit a later duplicate proposal"
    );
}

#[test]
fn annotation_keyboard_guard_survives_escape_and_apply_ack_until_next_show() {
    for apply in [false, true] {
        let mut scene = Scene::notes();
        assert!(scene.rack.annotation_keyboard_active());
        if apply {
            scene.rack.annotation_edit.as_mut().unwrap().text = "new draft".into();
            assert!(matches!(
                scene.click_label("Apply"),
                Some(RackAction::ApplyAnnotation(_))
            ));
            assert!(
                scene.rack.annotation_edit_active(),
                "draft waits for the host acknowledgment"
            );
            scene.rack.finish_annotation_edit(true);
        } else {
            assert!(scene
                .frame(vec![key_event(egui::Key::Escape, egui::Modifiers::NONE)])
                .0
                .is_none());
        }
        assert!(!scene.rack.annotation_edit_active());
        assert!(
            scene.rack.annotation_keyboard_active(),
            "the editor owns the entire dismissal frame"
        );
        scene.frame(vec![]);
        assert!(
            !scene.rack.annotation_keyboard_active(),
            "the next rack frame releases keyboard ownership"
        );
    }
}

#[test]
fn annotation_blank_pointer_resizes_both_edges_with_zoom_pan_and_hp_snapping() {
    for zoom in [0.5, 1.35] {
        for right in [false, true] {
            let origin = pos2(170.0, 40.0);
            let mut scene = Scene::blank(zoom, vec2(53.0, 29.0), origin);
            let start = scene.edge(right);
            let sign = if right { 1.0 } else { -1.0 };
            scene.press(start);
            let target = start + vec2(sign * HP * 2.6 * zoom, 0.0);
            scene.move_to(target);
            let drag = scene.rack.blank_resize.as_ref().expect("edge drag started");
            assert_eq!(drag.proposal.width_hp, 13);
            assert_eq!(
                scene.rack.positions["blank"], origin,
                "preview must not mutate saved placement"
            );
            assert_eq!(scene.patch.nodes[0].effective_blank_width_hp(), Some(10));
            let proposal = resize(scene.frame(vec![button(target, false)]).0);
            assert_eq!(proposal.movement.revision, 17);
            assert_eq!(proposal.before_hp, 10);
            assert_eq!(proposal.width_hp, 13);
            let after = proposal.movement.after["blank"];
            assert_eq!(after.y, origin.y);
            if right {
                assert_eq!(after, origin);
            } else {
                assert!((after.x + 13.0 * HP - (origin.x + 10.0 * HP)).abs() < 0.01);
            }
            scene
                .rack
                .validate_blank_resize(&scene.patch, &proposal)
                .unwrap();
            assert!(scene.rack.blank_resize.is_none());
            assert!(
                scene.frame(vec![]).0.is_none(),
                "release yields exactly one proposal"
            );
        }
    }
}

#[test]
fn annotation_blank_actual_pointer_clamps_both_edges_to_three_and_ninety_six_hp() {
    for right in [false, true] {
        for (signed_hp, expected) in [(-40.0, 3), (150.0, 96)] {
            let origin = if right {
                pos2(100.0, 40.0)
            } else {
                pos2(1500.0, 40.0)
            };
            let mut scene = Scene::blank(0.5, vec2(-30.0, 20.0), origin);
            let start = scene.edge(right);
            scene.press(start);
            let target = start + vec2((if right { 1.0 } else { -1.0 }) * signed_hp * HP * 0.5, 0.0);
            scene.move_to(target);
            let proposal = resize(scene.frame(vec![button(target, false)]).0);
            assert_eq!(proposal.width_hp, expected);
            scene
                .rack
                .validate_blank_resize(&scene.patch, &proposal)
                .unwrap();
        }
    }
}

#[test]
fn annotation_blank_collision_blocks_growth_without_moving_other_modules() {
    for right in [false, true] {
        let origin = pos2(350.0, 40.0);
        let mut scene = Scene::blank(0.85, vec2(25.0, 30.0), origin);
        let other: Node = serde_json::from_value(json!({"id":"obstacle","kind":"blank"})).unwrap();
        scene.patch.nodes.push(other);
        let obstacle = if right {
            pos2(origin.x + 10.0 * HP, origin.y)
        } else {
            pos2(origin.x - 10.0 * HP, origin.y)
        };
        scene.rack.positions.insert("obstacle".into(), obstacle);
        scene.frame(vec![]);
        let start = scene.edge(right);
        scene.press(start);
        let target = start + vec2(if right { 60.0 } else { -60.0 }, 0.0);
        scene.move_to(target);
        let drag = scene.rack.blank_resize.as_ref().unwrap();
        assert!(drag.blocked);
        assert_eq!(drag.proposal.width_hp, 10);
        assert_eq!(scene.rack.positions["obstacle"], obstacle);
        assert!(scene.frame(vec![button(target, false)]).0.is_none());
        assert_eq!(scene.rack.positions["blank"], origin);
    }
}

#[test]
fn annotation_blank_escape_lost_pointer_and_external_revision_cancel_preview() {
    for cancellation in 0..4 {
        let origin = pos2(170.0, 40.0);
        let mut scene = Scene::blank(0.8, vec2(53.0, 29.0), origin);
        let start = scene.edge(true);
        scene.press(start);
        let target = start + vec2(40.0, 0.0);
        scene.move_to(target);
        assert!(scene.rack.blank_resize.is_some());
        let events = match cancellation {
            0 => vec![key_event(egui::Key::Escape, egui::Modifiers::NONE)],
            1 => vec![egui::Event::PointerGone],
            2 => vec![egui::Event::WindowFocused(false)],
            _ => {
                scene.revision += 1;
                vec![]
            }
        };
        assert!(scene.frame(events).0.is_none());
        assert!(scene.rack.blank_resize.is_none());
        assert_eq!(scene.rack.positions["blank"], origin);
        assert_eq!(scene.patch.nodes[0].effective_blank_width_hp(), Some(10));
        assert!(scene.frame(vec![button(target, false)]).0.is_none());
    }
}

#[test]
fn annotation_blank_return_to_origin_is_noop_and_stale_geometry_rejects() {
    let mut scene = Scene::blank(1.0, vec2(40.0, 20.0), pos2(170.0, 40.0));
    let start = scene.edge(true);
    scene.press(start);
    scene.move_to(start + vec2(45.0, 0.0));
    scene.move_to(start);
    assert_eq!(
        scene.rack.blank_resize.as_ref().unwrap().proposal.width_hp,
        10
    );
    assert!(scene.frame(vec![button(start, false)]).0.is_none());
    scene.press(start);
    let target = start + vec2(45.0, 0.0);
    scene.move_to(target);
    let proposal = resize(scene.frame(vec![button(target, false)]).0);
    scene.patch.nodes[0].content = Some(openpatch_core::ModuleContent::Blank { width_hp: 11 });
    assert!(scene
        .rack
        .validate_blank_resize(&scene.patch, &proposal)
        .is_err());
    scene.patch.nodes[0].content = None;
    scene
        .rack
        .positions
        .insert("blank".into(), pos2(171.0, 40.0));
    assert!(scene
        .rack
        .validate_blank_resize(&scene.patch, &proposal)
        .is_err());
}

#[test]
fn annotation_notes_focus_unicode_clipboard_and_escape_keep_document_unchanged() {
    let mut scene = Scene::notes();
    let saved = serde_json::to_value(&scene.patch).unwrap();
    let editor = egui::Id::new(("annotation-text", "notes", "notes"));
    assert!(scene.ctx.memory(|memory| memory.has_focus(editor)));
    assert!(scene
        .frame(vec![egui::Event::Paste("\n\tClipboard 🦀".into())])
        .0
        .is_none());
    assert!(scene.edit().text.contains("\n\tClipboard 🦀"));
    let draft = scene.edit().text.clone();
    scene.frame(vec![key_event(egui::Key::A, egui::Modifiers::COMMAND)]);
    let output = scene.frame(vec![egui::Event::Copy]).1;
    assert!(output
        .platform_output
        .commands
        .iter()
        .any(|command| matches!(command,egui::OutputCommand::CopyText(text) if text==&draft)));
    assert!(scene
        .frame(vec![key_event(egui::Key::Escape, egui::Modifiers::NONE)])
        .0
        .is_none());
    assert!(!scene.rack.annotation_edit_active());
    assert_eq!(serde_json::to_value(&scene.patch).unwrap(), saved);
}

#[test]
fn annotation_notes_stale_revision_and_removed_id_retain_draft_and_disable_apply() {
    for removed in [false, true] {
        let mut scene = Scene::notes();
        scene.rack.annotation_edit.as_mut().unwrap().text = "my unsaved 日本語 draft".into();
        scene.revision += 1;
        if removed {
            scene.patch.nodes.clear();
        } else {
            scene.patch.nodes[0].content = Some(openpatch_core::ModuleContent::Notes {
                text: "external content".into(),
            });
        }
        scene.frame(vec![]);
        assert!(scene.edit().stale);
        assert_eq!(scene.edit().text, "my unsaved 日本語 draft");
        assert!(scene.click_label("Apply").is_none());
        assert_eq!(scene.edit().text, "my unsaved 日本語 draft");
        assert!(scene.click_label("Reload current content").is_none());
        if removed {
            assert!(scene.edit().stale);
            assert_eq!(scene.edit().text, "my unsaved 日本語 draft");
        } else {
            assert!(!scene.edit().stale);
            assert_eq!(scene.edit().text, "external content");
            assert_eq!(scene.edit().revision, 18);
        }
    }
}

#[test]
fn annotation_notes_oversized_draft_is_correctable_and_apply_waits_for_host_ack() {
    let mut scene = Scene::notes();
    let saved = serde_json::to_value(&scene.patch).unwrap();
    scene.rack.annotation_edit.as_mut().unwrap().text = "é".repeat(4097);
    assert!(scene.click_label("Apply").is_none());
    assert_eq!(scene.edit().text.len(), 8194);
    scene.rack.annotation_edit.as_mut().unwrap().text = "fixed\n日本語 🎛️".into();
    let action = scene.click_label("Apply");
    assert!(
        matches!(action,Some(RackAction::ApplyAnnotation(ref args)) if args["id"]=="notes" && args["content"]["text"]=="fixed\n日本語 🎛️" && args["expected_revision"]==17)
    );
    assert_eq!(scene.edit().text, "fixed\n日本語 🎛️");
    assert_eq!(serde_json::to_value(&scene.patch).unwrap(), saved);
    scene.rack.finish_annotation_edit(false);
    assert_eq!(scene.edit().text, "fixed\n日本語 🎛️");
    assert!(scene.edit().error.is_some());
    scene.rack.finish_annotation_edit(true);
    assert!(!scene.rack.annotation_edit_active());
}

#[test]
fn annotation_notes_cancel_and_unchanged_apply_make_no_request() {
    for cancel in [true, false] {
        let mut scene = Scene::notes();
        if cancel {
            scene.rack.annotation_edit.as_mut().unwrap().text = "discard this".into();
        }
        assert!(scene
            .click_label(if cancel { "Cancel" } else { "Apply" })
            .is_none());
        assert!(!scene.rack.annotation_edit_active());
        assert_eq!(
            scene.patch.nodes[0].effective_notes_text(),
            Some("Saved 日本語\n🎛️")
        );
    }
}

#[test]
fn annotation_blank_integer_entry_validates_bounds_and_retains_origin_until_ack() {
    let mut scene = Scene::blank(0.85, vec2(25.0, 30.0), pos2(170.0, 40.0));
    scene
        .rack
        .open_annotation_editor(&scene.patch.nodes[0], scene.revision);
    scene.frame(vec![]);
    scene.frame(vec![]);
    for invalid in ["2", "97", "3.5", "abc", ""] {
        scene.rack.annotation_edit.as_mut().unwrap().text = invalid.into();
        assert!(scene.click_label("Apply").is_none());
        assert_eq!(scene.edit().text, invalid);
    }
    scene.rack.annotation_edit.as_mut().unwrap().text = " 24 ".into();
    let proposal = resize(scene.click_label("Apply"));
    assert_eq!(proposal.width_hp, 24);
    assert_eq!(proposal.before_hp, 10);
    assert_eq!(proposal.movement.revision, 17);
    assert_eq!(proposal.movement.before, proposal.movement.after);
    assert_eq!(scene.patch.nodes[0].effective_blank_width_hp(), Some(10));
    assert!(scene.rack.annotation_edit_active());
    scene.rack.finish_annotation_edit(false);
    assert_eq!(scene.edit().text, " 24 ");
    scene.rack.finish_annotation_edit(true);
    assert!(!scene.rack.annotation_edit_active());
}

#[test]
fn annotation_notes_disabled_by_modal_retains_draft_and_resumes_editing() {
    for disable_parent in [false, true] {
        let mut scene = Scene::notes();
        let draft = "Draft survives modal 🦀";
        scene.rack.annotation_edit.as_mut().unwrap().text = draft.into();
        let saved = serde_json::to_value(&scene.patch).unwrap();
        let visible = scene.frame(vec![]).1;
        let apply = labels(&visible)
            .into_iter()
            .find(|(text, _)| text == "Apply")
            .unwrap()
            .1
            .center();
        scene.rack.shortcuts_enabled = disable_parent;
        for events in [
            vec![egui::Event::PointerMoved(apply), button(apply, true)],
            vec![button(apply, false)],
            vec![
                egui::Event::Paste("must not reach the hidden editor".into()),
                key_event(egui::Key::Escape, egui::Modifiers::NONE),
            ],
        ] {
            scene.time += 0.1;
            let (action, output) = frame_revision_state(
                &scene.ctx,
                &mut scene.rack,
                &scene.patch,
                scene.time,
                events,
                true,
                !disable_parent,
                scene.revision,
            );
            assert!(action.is_none(), "a modal must block background Apply");
            assert!(!labels(&output).iter().any(|(text, _)| text == "Apply"));
            assert_eq!(scene.edit().text, draft, "hidden draft stays intact");
            assert_eq!(serde_json::to_value(&scene.patch).unwrap(), saved);
        }
        scene.rack.shortcuts_enabled = true;
        let visible = scene.frame(vec![]).1;
        let text_position = labels(&visible)
            .into_iter()
            .find(|(text, _)| text == draft)
            .expect("the retained draft is visible after closing the modal")
            .1
            .center();
        scene.press(text_position);
        assert!(scene.frame(vec![button(text_position, false)]).0.is_none());
        assert!(scene
            .frame(vec![egui::Event::Paste(" resumed".into())])
            .0
            .is_none());
        let resumed = scene.edit().text.clone();
        assert!(resumed.contains(" resumed"));
        assert!(
            matches!(scene.click_label("Apply"), Some(RackAction::ApplyAnnotation(args))
            if args["content"]["text"] == resumed && args["expected_revision"] == 17)
        );
        assert_eq!(serde_json::to_value(&scene.patch).unwrap(), saved);
    }
}
