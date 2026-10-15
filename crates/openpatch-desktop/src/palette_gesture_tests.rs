// Real egui pointer gestures exercise palette capture at cable classification.
use super::*;
use crate::cable_palette::Palette;
use openpatch::control::Session;

fn setup() -> (tempfile::TempDir, Session, egui::Context, Rack, Palette) {
    let directory = tempfile::tempdir().unwrap();
    let mut session = Session::new(directory.path()).unwrap();
    session.replace_patch(stacked_fixture(), None).unwrap();
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let palette = Palette::default();
    apply(&mut rack, &palette);
    step(&ctx, &mut rack, &session, 0.0, vec![]);
    (directory, session, ctx, rack, palette)
}

fn apply(rack: &mut Rack, palette: &Palette) {
    rack.set_cable_palette(
        palette.selection(),
        &palette.presets()[palette.active_index()].colors,
    );
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

fn press(position: Pos2, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::PointerButton {
        pos: position,
        button: PointerButton::Primary,
        pressed: true,
        modifiers,
    }
}

fn begin(
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
        vec![egui::Event::PointerMoved(from), press(from, modifiers)]
    )
    .is_none());
    assert!(step(ctx, rack, session, 1.1, vec![egui::Event::PointerMoved(to)]).is_none());
}

#[test]
fn palette_drag_captures_reviewed_rgb_and_commits_exactly_one_undoable_route() {
    let (_directory, mut session, ctx, mut rack, mut palette) = setup();
    palette.select_color(3);
    apply(&mut rack, &palette);
    let captured = palette.selection();
    let before = session.snapshot();
    let from = fixture_jack(&rack, session.patch(), "alt", "out", false);
    let to = fixture_jack(&rack, session.patch(), "dest", "signal", true);
    begin(&ctx, &mut rack, &session, from, to, egui::Modifiers::NONE);
    let pending = rack.pending.as_ref().unwrap();
    assert_eq!(pending.color, Some(captured.color));
    assert_eq!(pending.palette_selection, Some(captured));
    let action = step(&ctx, &mut rack, &session, 1.2, vec![button(to, false)]).unwrap();
    let RackAction::PaletteConnect {
        arguments,
        selection,
    } = action
    else {
        panic!("fresh gesture must carry palette intent")
    };
    assert_eq!(selection, captured);
    assert_eq!(arguments["color"], json!(captured.color));
    session.execute("connect", arguments).unwrap();
    assert!(palette.advance_on_commit(selection));
    let after = session.snapshot();
    assert_eq!(
        session.patch().cables.last().unwrap().color,
        Some(captured.color)
    );
    assert_eq!(
        after["history"]["undo_steps"].as_u64().unwrap(),
        before["history"]["undo_steps"].as_u64().unwrap() + 1
    );
    session.execute("undo", json!({})).unwrap();
    assert_eq!(session.snapshot()["patch"], before["patch"]);
    session.execute("redo", json!({})).unwrap();
    assert_eq!(session.snapshot()["patch"], after["patch"]);
}

#[test]
fn palette_click_to_connect_occupied_output_uses_press_intent_after_modifier_release() {
    let (_directory, session, ctx, mut rack, palette) = setup();
    let captured = palette.selection();
    let from = fixture_jack(&rack, session.patch(), "src", "out", false);
    let to = fixture_jack(&rack, session.patch(), "dest", "signal", true);
    assert!(step(
        &ctx,
        &mut rack,
        &session,
        1.0,
        vec![
            egui::Event::PointerMoved(from),
            press(from, egui::Modifiers::COMMAND),
            button(from, false)
        ]
    )
    .is_none());
    assert_eq!(
        rack.pending.as_ref().unwrap().palette_selection,
        Some(captured)
    );
    let action = step(
        &ctx,
        &mut rack,
        &session,
        1.1,
        vec![
            egui::Event::PointerMoved(to),
            button(to, true),
            button(to, false),
        ],
    )
    .unwrap();
    assert!(
        matches!(action, RackAction::PaletteConnect { arguments, selection } if selection == captured && arguments["color"] == json!(captured.color) && arguments["from"] == "src" && arguments["to"] == "dest")
    );
}

#[test]
fn palette_change_during_drag_preserves_ghost_rgb_and_cannot_consume_new_choice() {
    let (_directory, mut session, ctx, mut rack, mut palette) = setup();
    let captured = palette.selection();
    let from = fixture_jack(&rack, session.patch(), "alt", "out", false);
    let to = fixture_jack(&rack, session.patch(), "dest", "signal", true);
    begin(&ctx, &mut rack, &session, from, to, egui::Modifiers::NONE);
    palette.cycle(true);
    let fresh = palette.selection();
    apply(&mut rack, &palette);
    assert_eq!(rack.pending.as_ref().unwrap().color, Some(captured.color));
    let action = step(&ctx, &mut rack, &session, 1.2, vec![button(to, false)]).unwrap();
    let RackAction::PaletteConnect {
        arguments,
        selection,
    } = action
    else {
        panic!("fresh gesture carries captured intent")
    };
    assert_eq!(selection, captured);
    assert_eq!(arguments["color"], json!(captured.color));
    session.execute("connect", arguments).unwrap();
    assert!(!palette.advance_on_commit(selection));
    assert_eq!(palette.selection(), fresh);
}

#[test]
fn palette_cancel_and_revision_invalidation_leave_choice_and_graph_untouched() {
    for invalidate in [false, true] {
        let (_directory, mut session, ctx, mut rack, palette) = setup();
        let captured = palette.selection();
        let from = fixture_jack(&rack, session.patch(), "alt", "out", false);
        let to = fixture_jack(&rack, session.patch(), "dest", "signal", true);
        begin(&ctx, &mut rack, &session, from, to, egui::Modifiers::NONE);
        if invalidate {
            session
                .execute(
                    "set_parameter",
                    json!({"id":"src","parameter":"value","value":3.0}),
                )
                .unwrap();
        }
        let before = session.snapshot();
        let events = if invalidate {
            vec![button(to, false)]
        } else {
            vec![egui::Event::PointerGone]
        };
        assert!(step(&ctx, &mut rack, &session, 1.2, events).is_none());
        assert!(rack.pending.is_none());
        assert_eq!(palette.selection(), captured);
        assert_eq!(session.snapshot(), before);
    }
}

#[test]
fn palette_move_and_clone_keep_existing_effective_rgb_without_palette_ticket() {
    for explicit in [false, true] {
        for clone in [false, true] {
            let (_directory, mut session, ctx, mut rack, palette) = setup();
            if explicit {
                let mut patch = session.patch().clone();
                patch.cables.last_mut().unwrap().color = Some([9, 42, 199]);
                session.replace_patch(patch, None).unwrap();
                step(&ctx, &mut rack, &session, 0.5, vec![]);
            }
            let original = session.patch().cables.last().unwrap().clone();
            let expected = cable_instance_color(&original).to_array();
            let from = fixture_jack(&rack, session.patch(), "target", "signal", true);
            let to = fixture_jack(&rack, session.patch(), "dest", "signal", true);
            begin(
                &ctx,
                &mut rack,
                &session,
                from,
                to,
                if clone {
                    egui::Modifiers {
                        command: true,
                        shift: true,
                        ..Default::default()
                    }
                } else {
                    egui::Modifiers::NONE
                },
            );
            let pending = rack.pending.as_ref().unwrap();
            assert_eq!(pending.palette_selection, None);
            let ghost = pending.color.unwrap_or_else(|| {
                let color = cable_instance_color(pending.original.as_ref().unwrap());
                [color.r(), color.g(), color.b()]
            });
            assert_eq!(ghost, [expected[0], expected[1], expected[2]]);
            let action = step(&ctx, &mut rack, &session, 1.2, vec![button(to, false)]).unwrap();
            assert!(matches!(&action, RackAction::Command(_, _)));
            run_cable_action(&mut session, action);
            let moved = session
                .patch()
                .cables
                .iter()
                .find(|cable| cable.to == "dest")
                .unwrap();
            assert_eq!(cable_instance_color(moved).to_array(), expected);
            if clone || explicit {
                assert_eq!(moved.color, Some([expected[0], expected[1], expected[2]]));
            }
            assert_eq!(session.patch().cables.len(), if clone { 3 } else { 2 });
            assert_eq!(palette.next_index(), 0);
        }
    }
}

#[test]
fn palette_menu_explicit_same_rgb_override_is_an_ordinary_connect_without_ticket() {
    fn label(output: &egui::FullOutput, prefix: &str) -> Pos2 {
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
            .unwrap_or_else(|| panic!("missing menu label {prefix}"))
    }
    let (_directory, mut session, ctx, mut rack, palette) = setup();
    let choice = palette.selection();
    let from = fixture_jack(&rack, session.patch(), "alt", "out", false);
    let to = fixture_jack(&rack, session.patch(), "dest", "signal", true);
    let mut time = 0.5;
    let mut render = |rack: &mut Rack, events| {
        time += 0.1;
        frame_revision_state(
            &ctx,
            rack,
            session.patch(),
            time,
            events,
            true,
            true,
            session.revision(),
        )
    };
    for pressed in [true, false] {
        assert!(render(
            &mut rack,
            vec![
                egui::Event::PointerMoved(from),
                egui::Event::PointerButton {
                    pos: from,
                    button: PointerButton::Secondary,
                    pressed,
                    modifiers: egui::Modifiers::NONE
                }
            ]
        )
        .0
        .is_none());
    }
    let menu = render(&mut rack, vec![]).1;
    let submenu = label(&menu, "New cable colour");
    for pressed in [true, false] {
        assert!(render(
            &mut rack,
            vec![egui::Event::PointerMoved(submenu), button(submenu, pressed)]
        )
        .0
        .is_none());
    }
    let output = render(&mut rack, vec![]).1;
    let color = label(&output, "● Coral");
    for pressed in [true, false] {
        assert!(render(
            &mut rack,
            vec![egui::Event::PointerMoved(color), button(color, pressed)]
        )
        .0
        .is_none());
    }
    render(&mut rack, vec![]);
    let pending = rack.pending.as_ref().unwrap();
    assert_eq!(pending.color, Some(choice.color));
    assert_eq!(pending.palette_selection, None);
    for pressed in [true, false] {
        let action = render(
            &mut rack,
            vec![egui::Event::PointerMoved(to), button(to, pressed)],
        )
        .0;
        if let Some(action) = action {
            assert!(
                matches!(&action, RackAction::Command("connect", arguments) if arguments["color"] == json!(choice.color))
            );
            drop(render);
            run_cable_action(&mut session, action);
            assert_eq!(palette.selection(), choice);
            assert_eq!(
                session.patch().cables.last().unwrap().color,
                Some(choice.color)
            );
            return;
        }
    }
    panic!("explicit menu cable must connect");
}
