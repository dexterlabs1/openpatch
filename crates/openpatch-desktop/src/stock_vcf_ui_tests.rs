use super::*;
fn patch() -> Patch {
    serde_json::from_value(json!({"version":2,"nodes":[{"id":"filter","kind":"stock_vcf"}]}))
        .unwrap()
}
fn key(key: egui::Key, shift: bool) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: if shift {
            egui::Modifiers::SHIFT
        } else {
            egui::Modifiers::NONE
        },
    }
}
fn at(local: Pos2) -> Pos2 {
    local + vec2(24.0, 12.0)
}
fn open(index: usize) -> (egui::Context, Rack, Patch) {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch = patch();
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 29);
    let pos = at(stock_vcf_value_rect(index).center());
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.0,
        vec![egui::Event::PointerMoved(pos), button(pos, true)],
        true,
        true,
        29,
    );
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.1,
        vec![button(pos, false)],
        true,
        true,
        29,
    );
    assert_eq!(
        rack.editing.as_ref().unwrap().parameter,
        STOCK_VCF_PARAMS[index]
    );
    frame_revision_state(&ctx, &mut rack, &patch, 1.2, vec![], true, true, 29);
    (ctx, rack, patch)
}
#[test]
fn stock_vcf_exact_units_and_bounds_round_trip_to_source_parameters() {
    let patch = patch();
    for (index, text, raw) in [
        (0, "8 Hz", stock_vcf_frequency_raw(8.0)),
        (0, "22000 Hz", stock_vcf_frequency_raw(22000.0)),
        (0, "A4", stock_vcf_frequency_raw(440.0)),
        (1, "35%", 0.35),
        (2, "0%", -1.0),
        (2, "100%", 0.0),
        (2, "200%", 1.0),
        (3, "-100%", -1.0),
        (4, "-35%", -0.35),
        (5, "100%", 1.0),
    ] {
        let mut edit = stock_vcf_edit(&patch.nodes[0], index, 29);
        edit.text = text.into();
        let result = parameter_edit_raw_value(&edit, parsed_parameter_result(&edit).unwrap());
        assert!((result - raw).abs() < 0.000001, "{text}: {result} vs {raw}");
    }
    for i in 0..6 {
        let mut edit = stock_vcf_edit(&patch.nodes[0], i, 29);
        for text in [
            "nan",
            "inf",
            "-inf",
            "1/0",
            "100000",
            if i == 0 {
                "7.99"
            } else if i >= 3 {
                "-100.01"
            } else {
                "-0.01"
            },
        ] {
            edit.text = text.into();
            assert!(parsed_parameter_result(&edit).is_err(), "{i}: {text}");
        }
    }
    for hz in [8.0, 22000.0] {
        let mut node = patch.nodes[0].clone();
        node.params
            .insert("frequency".into(), stock_vcf_frequency_raw(hz));
        let edit = stock_vcf_edit(&node, 0, 29);
        assert!(parsed_parameter_result(&edit).is_ok(), "{}", edit.text);
    }
}
#[test]
fn stock_vcf_all_six_rotaries_accept_relative_pointer_drags_and_group_history() {
    for (i, parameter) in STOCK_VCF_PARAMS.iter().enumerate() {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let patch = patch();
        frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 29);
        let start = at(stock_vcf_knob_rect(i).center());
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.0,
            vec![egui::Event::PointerMoved(start), button(start, true)],
            true,
            true,
            29,
        );
        let end = start + vec2(0.0, if i >= 3 { 15.0 } else { -15.0 });
        let action = frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.1,
            vec![egui::Event::PointerMoved(end)],
            true,
            true,
            29,
        )
        .0;
        let Some(RackAction::Command("set_parameter", args)) = action else {
            panic!("rotary {i}")
        };
        assert_eq!(args["parameter"], *parameter);
        assert_eq!(args["expected_revision"], 29);
        assert!(args["history_group"]
            .as_str()
            .unwrap()
            .starts_with("rack-fader-"));
        let value = args["value"].as_f64().unwrap();
        assert!(
            if i >= 3 { value < 0.0 } else { value > 0.0 },
            "{parameter} = {value}"
        );
    }
}
#[test]
fn stock_vcf_rotary_center_click_fine_keys_and_modal_guard_keep_units() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let mut patch = patch();
    patch.nodes[0].params.insert("resonance".into(), 0.5);
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 29);
    let center = at(stock_vcf_knob_rect(1).center());
    let press = frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.0,
        vec![egui::Event::PointerMoved(center), button(center, true)],
        true,
        true,
        29,
    )
    .0;
    assert!(
        press.is_none(),
        "center press must not write a new resonance"
    );
    let release = frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.1,
        vec![button(center, false)],
        true,
        true,
        29,
    )
    .0;
    assert!(
        release.is_none(),
        "center click must not set an absolute value"
    );
    let knob_id = ctx
        .memory(|memory| memory.focused())
        .expect("clicked resonance knob has focus");
    assert!(ctx.memory(|memory| memory.has_focus(knob_id)));
    let end = frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.2,
        vec![key(egui::Key::End, false)],
        true,
        true,
        29,
    )
    .0;
    assert!(
        matches!(end, Some(RackAction::Command("set_parameter", ref args))
        if args["parameter"] == "resonance" && args["value"] == 1.0)
    );
    let home = frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.3,
        vec![key(egui::Key::Home, false)],
        true,
        true,
        29,
    )
    .0;
    assert!(
        matches!(home, Some(RackAction::Command("set_parameter", ref args))
        if args["value"] == 0.0)
    );
    assert!(ctx.memory(|memory| memory.has_focus(knob_id)));

    rack.set_shortcuts_enabled(false);
    let blocked = frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.4,
        vec![egui::Event::PointerMoved(center), button(center, true)],
        true,
        true,
        29,
    )
    .0;
    assert!(blocked.is_none());
    assert!(frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.5,
        vec![egui::Event::PointerMoved(center - vec2(0.0, 30.0))],
        true,
        true,
        29
    )
    .0
    .is_none());
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.6,
        vec![button(center - vec2(0.0, 30.0), false)],
        true,
        true,
        29,
    );
}

#[test]
fn stock_vcf_focused_arrow_edits_do_not_pan_and_idle_arrows_still_pan() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let mut patch = patch();
    patch.nodes[0].params.insert("resonance".into(), 0.5);
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 29);
    let center = at(stock_vcf_knob_rect(1).center());
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.0,
        vec![egui::Event::PointerMoved(center), button(center, true)],
        true,
        true,
        29,
    );
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.1,
        vec![button(center, false)],
        true,
        true,
        29,
    );
    let knob_id = ctx
        .memory(|memory| memory.focused())
        .expect("clicked resonance knob has focus");
    assert!(ctx.memory(|memory| memory.has_focus(knob_id)));
    for (index, modifiers) in [egui::Modifiers::NONE, egui::Modifiers::CTRL]
        .into_iter()
        .enumerate()
    {
        let time = 2.0 + index as f64;
        let held_pan = rack.pan;
        let held_viewport = rack.viewport;
        let arrow = egui::Event::Key {
            key: egui::Key::ArrowDown,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        let action = frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            time,
            vec![arrow],
            true,
            true,
            30 + index as u64,
        )
        .0;
        let Some(RackAction::Command("set_parameter", args)) = action else {
            panic!("focused resonance rotary must accept arrow edit {index}")
        };
        assert_eq!(args["parameter"], "resonance");
        assert_eq!(args["expected_revision"], 30 + index as u64);
        let next = args["value"].as_f64().unwrap() as f32;
        assert!(next < patch.nodes[0].params["resonance"]);
        assert_eq!(rack.pan, held_pan, "focused arrow moved the rack camera");
        assert_eq!(rack.viewport, held_viewport);
        assert!(ctx.memory(|memory| memory.has_focus(knob_id)));
        patch.nodes[0].params.insert("resonance".into(), next);
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            time + 0.1,
            vec![egui::Event::Key {
                key: egui::Key::ArrowDown,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers,
            }],
            true,
            true,
            31 + index as u64,
        );
        assert_eq!(
            rack.pan, held_pan,
            "releasing the arrow moved the rack camera"
        );
    }
    ctx.memory_mut(|memory| memory.surrender_focus(knob_id));
    frame_revision_state(&ctx, &mut rack, &patch, 4.0, vec![], true, true, 32);
    assert!(!ctx.memory(|memory| memory.has_focus(knob_id)));
    let idle_pan = rack.pan;
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        4.1,
        vec![key(egui::Key::ArrowRight, false)],
        true,
        true,
        32,
    );
    assert!(
        rack.pan.x < idle_pan.x,
        "idle right arrow must still pan the rack"
    );
}

#[test]
fn stock_vcf_signed_depth_ctrl_drag_is_fine_and_modifier_release_stays_relative() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let mut patch = patch();
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 29);
    let center = at(stock_vcf_knob_rect(3).center());
    let fine = egui::Modifiers {
        ctrl: true,
        ..Default::default()
    };
    let press = egui::Event::PointerButton {
        pos: center,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: fine,
    };
    assert!(frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.0,
        vec![egui::Event::PointerMoved(center), press],
        true,
        true,
        29
    )
    .0
    .is_none());
    let carrier = |modifiers| egui::Event::Key {
        key: egui::Key::F12,
        physical_key: None,
        pressed: false,
        repeat: false,
        modifiers,
    };
    let fine_end = center - vec2(0.0, 15.0);
    let fine_action = frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.1,
        vec![egui::Event::PointerMoved(fine_end), carrier(fine)],
        true,
        true,
        29,
    )
    .0;
    let Some(RackAction::Command("set_parameter", args)) = fine_action else {
        panic!("fine depth movement must edit its raw parameter")
    };
    assert_eq!(args["parameter"], "cutoff_cv_amount");
    assert!((args["value"].as_f64().unwrap() - 0.02).abs() < 1e-6);
    let group = args["history_group"].clone();
    patch.nodes[0].params.insert(
        "cutoff_cv_amount".into(),
        args["value"].as_f64().unwrap() as f32,
    );
    assert!(frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.2,
        vec![
            egui::Event::PointerMoved(fine_end),
            carrier(egui::Modifiers::NONE)
        ],
        true,
        true,
        29
    )
    .0
    .is_none());
    let next = fine_end - vec2(0.0, 15.0);
    let resumed = frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.3,
        vec![egui::Event::PointerMoved(next)],
        true,
        true,
        29,
    )
    .0;
    let Some(RackAction::Command("set_parameter", args)) = resumed else {
        panic!("normal movement must continue the same depth gesture")
    };
    assert!((args["value"].as_f64().unwrap() - 0.22).abs() < 1e-6);
    assert_eq!(args["history_group"], group);
    patch.nodes[0].params.insert(
        "cutoff_cv_amount".into(),
        args["value"].as_f64().unwrap() as f32,
    );
    assert!(frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.4,
        vec![button(next, false)],
        true,
        true,
        29
    )
    .0
    .is_none());
}

#[test]
fn stock_vcf_drive_rotary_double_click_restores_nominal_one_hundred_percent() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let mut patch = patch();
    patch.nodes[0].params.insert("drive".into(), 0.5);
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 29);
    let center = at(stock_vcf_knob_rect(2).center());
    let mut actions = vec![];
    for (time, pressed) in [(1.0, true), (1.1, false), (1.2, true), (1.3, false)] {
        actions.push(
            frame_revision_state(
                &ctx,
                &mut rack,
                &patch,
                time,
                vec![egui::Event::PointerMoved(center), button(center, pressed)],
                true,
                true,
                29,
            )
            .0,
        );
    }
    assert!(
        actions[..3].iter().all(Option::is_none),
        "single click never jumps"
    );
    let Some(RackAction::Command("set_parameter", args)) = actions.pop().unwrap() else {
        panic!("double click must reset drive")
    };
    assert_eq!(args["parameter"], "drive");
    assert_eq!(args["value"], 0.0);
    assert_eq!(args["expected_revision"], 29);
    assert_eq!(stock_vcf_readout(2, 0.0), "100%");
}

#[test]
fn stock_vcf_accepted_edits_have_six_angular_construction_anchors() {
    use crate::construction_motion::{Marker, MotionMode};
    for (index, parameter) in STOCK_VCF_PARAMS.iter().enumerate() {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let before = patch();
        let mut after = before.clone();
        let old = stock_vcf_default(index);
        let new = if index == 0 { 0.8 } else { 0.5 };
        after.nodes[0].params.insert((*parameter).into(), new);
        frame_revision_state(&ctx, &mut rack, &before, 0.0, vec![], true, true, 9);
        let now = std::time::Instant::now();
        let receipt = json!({"plan_id":19,"completed_steps":1,"last_revision":10,"current_revision":10,
            "last_commit":{"age_ms":0,"invalidate_existing_cables":false,"plan_id":19,
            "step_index":0,"before_revision":9,"revision":10,"label":"Shape filter",
            "new_modules":[],"new_cables":[],"parameter_changes":[{"id":"filter",
            "kind":"stock_vcf","parameter":parameter,"old_effective":old,"new_effective":new}]}});
        rack.observe_construction_transition_at(
            &before,
            &after,
            9,
            10,
            &receipt,
            MotionMode::Full,
            now,
        );
        rack.set_construction_clock_override(Some(now + std::time::Duration::from_millis(150)));
        let (action, _) =
            frame_revision_state(&ctx, &mut rack, &after, 0.1, vec![], true, true, 10);
        assert!(action.is_none());
        let anchor = &rack.construction_anchors[&("filter".into(), (*parameter).into())];
        assert_eq!(anchor.old_text, stock_vcf_readout(index, old));
        assert_eq!(anchor.new_text, stock_vcf_readout(index, new));
        let Some(Marker::Rotary {
            center,
            radius,
            old: from,
            new: to,
        }) = anchor.marker.as_ref()
        else {
            panic!("{parameter}: accepted edit needs angular cue");
        };
        assert!(center.distance(at(stock_vcf_knob_rect(index).center())) < 0.001);
        assert!((*radius - (stock_vcf_control_radius(index) + 3.0) * rack.zoom).abs() < 0.001);
        assert!(from < to, "{parameter}: saved value moves clockwise");
        assert!(anchor
            .rect
            .contains_rect(stock_vcf_knob_rect(index).translate(vec2(24.0, 12.0))));
    }
}
#[test]
fn stock_vcf_direct_pointer_entry_and_keyboard_apply_use_displayed_units() {
    for (i, text, expected) in [
        (0, "880/2 Hz", stock_vcf_frequency_raw(440.0)),
        (1, "73%", 0.73),
        (2, "175%", 0.75),
        (3, "-60%", -0.6),
        (4, "80%", 0.8),
        (5, "-25%", -0.25),
    ] {
        let (ctx, mut rack, patch) = open(i);
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.3,
            vec![egui::Event::Text(text.into())],
            true,
            true,
            29,
        );
        let action = frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.4,
            vec![key(egui::Key::Enter, false)],
            true,
            true,
            29,
        )
        .0;
        let Some(RackAction::Command("set_parameter", args)) = action else {
            panic!("entry {i}")
        };
        assert_eq!(args["parameter"], STOCK_VCF_PARAMS[i]);
        assert_eq!(args["expected_revision"], 29);
        assert!((args["value"].as_f64().unwrap() - f64::from(expected)).abs() < 0.000001);
    }
}
#[test]
fn stock_vcf_tab_order_covers_six_controls_and_wraps_both_directions() {
    for (index, backwards, next) in [
        (0, false, 1),
        (1, false, 2),
        (2, false, 3),
        (3, false, 4),
        (4, false, 5),
        (5, false, 0),
        (0, true, 5),
    ] {
        let (ctx, mut rack, patch) = open(index);
        assert_eq!(
            rack.precision_controls
                .iter()
                .map(|c| c.edit.parameter.as_str())
                .collect::<Vec<_>>(),
            STOCK_VCF_PARAMS
        );
        let action = frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.3,
            vec![key(egui::Key::Tab, backwards)],
            true,
            true,
            29,
        )
        .0;
        let Some(RackAction::ParameterEditAndNavigate { arguments, forward }) = action else {
            panic!("Tab {index}")
        };
        assert_eq!(forward, !backwards);
        assert_eq!(arguments["expected_revision"], 29);
        assert_eq!(arguments["parameter"], STOCK_VCF_PARAMS[index]);
        rack.complete_parameter_navigation(Ok(30), !backwards);
        assert_eq!(
            rack.precision_next.as_ref().unwrap().parameter,
            STOCK_VCF_PARAMS[next]
        );
    }
}
#[test]
fn stock_vcf_controls_and_six_jacks_are_inside_and_separate() {
    let bounds = Rect::from_min_size(Pos2::ZERO, vec2(panel_width("stock_vcf"), PANEL_HEIGHT));
    assert_eq!(panel_width("stock_vcf"), 270.0, "Stock VCF remains 18 HP");
    for column in 0..3 {
        let main = stock_vcf_knob_rect(column);
        let depth = stock_vcf_knob_rect(column + 3);
        assert!(
            main.width() > depth.width(),
            "main control is larger than CV depth"
        );
        assert_eq!(main.center().x, depth.center().x, "paired controls align");
        assert!(
            main.center().y < depth.center().y,
            "depth controls form the second row"
        );
    }
    let controls = (0..6)
        .flat_map(|i| [stock_vcf_knob_rect(i), stock_vcf_value_rect(i)])
        .collect::<Vec<_>>();
    for (i, a) in controls.iter().enumerate() {
        assert!(bounds.contains_rect(*a));
        for b in &controls[i + 1..] {
            assert!(!a.intersects(*b));
        }
    }
    let ports = (0..4)
        .map(|i| port_position("stock_vcf", true, i, 4, bounds.width()))
        .chain((0..2).map(|i| port_position("stock_vcf", false, i, 2, bounds.width())))
        .collect::<Vec<_>>();
    for (i, p) in ports.iter().enumerate() {
        assert!(bounds.shrink(14.0).contains(*p));
        for c in &controls {
            assert!(!c.expand(14.0).contains(*p));
        }
        for q in &ports[i + 1..] {
            assert!(p.distance(*q) > 32.0);
        }
    }
}
fn texts(shape: &egui::Shape, out: &mut Vec<(String, Rect)>) {
    match shape {
        egui::Shape::Vec(children) => {
            for child in children {
                texts(child, out)
            }
        }
        egui::Shape::Text(t) if !t.galley.text().is_empty() => out.push((
            t.galley.text().into(),
            Rect::from_min_size(t.pos, t.galley.size()),
        )),
        _ => {}
    }
}
#[test]
fn stock_vcf_rack_and_shared_browser_have_identical_fitting_faceplates_at_all_scales() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "stock_vcf")
        .unwrap();
    for scale in [0.25, 0.42, 0.85, 1.0] {
        let mut surfaces = vec![];
        for browser in [false, true] {
            let ctx = egui::Context::default();
            crate::panel_skin::install_fonts(&ctx);
            let mut rack = Rack {
                zoom: scale,
                ..Rack::default()
            };
            let mut preview = ModulePreview::new(module);
            let bounds = Rect::from_min_size(
                pos2(24.0, 12.0),
                vec2(panel_width("stock_vcf"), PANEL_HEIGHT) * scale,
            );
            let mut drawn = vec![];
            for _ in 0..2 {
                let output = if browser {
                    ctx.run(
                        egui::RawInput {
                            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 700.0))),
                            ..Default::default()
                        },
                        |ctx| {
                            egui::CentralPanel::default()
                                .frame(egui::Frame::NONE)
                                .show(ctx, |ui| preview.show(ui, bounds, &catalog));
                        },
                    )
                } else {
                    frame_output(&ctx, &mut rack, &patch(), 0.0, vec![]).1
                };
                drawn.clear();
                fn count_faces(shape: &egui::Shape) -> usize {
                    match shape {
                        egui::Shape::Vec(shapes) => shapes.iter().map(count_faces).sum(),
                        egui::Shape::Circle(circle)
                            if circle.fill == Color32::from_rgb(56, 59, 57) =>
                        {
                            1
                        }
                        _ => 0,
                    }
                }
                let rotary_faces: usize = output
                    .shapes
                    .iter()
                    .map(|shape| count_faces(&shape.shape))
                    .sum();
                assert_eq!(
                    rotary_faces, 6,
                    "{scale} browser={browser}: six actual rotary faces"
                );
                for shape in output.shapes {
                    texts(&shape.shape, &mut drawn);
                }
            }
            for required in [
                "VCF",
                "CUTOFF",
                "RESONANCE",
                "DRIVE",
                "FREQ CV DEPTH",
                "RES CV DEPTH",
                "DRIVE CV DEPTH",
                "SIGNAL FLOW",
                "−RESONANCE",
                "4P LADDER",
                "LP / HP",
                "FREQ CV",
                "RES CV",
                "DRIVE CV",
                "IN",
                "LP",
                "HP",
                "262 Hz",
                "0%",
                "100%",
                "+0%",
            ] {
                assert!(
                    drawn.iter().any(|(t, _)| t == required),
                    "{scale} browser={browser}: missing {required}"
                );
            }
            let first_caption = drawn.iter().find(|(label, _)| label == "CUTOFF").unwrap().1;
            let header_divider_y = bounds.top() + 65.0 * scale;
            assert!(
                first_caption.top() >= header_divider_y + 1.0 * scale,
                "{scale} browser={browser}: cutoff caption touches the header divider"
            );
            let routing = drawn
                .iter()
                .find(|(label, _)| label == "−RESONANCE")
                .unwrap()
                .1;
            let input_caption = drawn
                .iter()
                .find(|(label, _)| label == "FREQ CV")
                .unwrap()
                .1;
            assert!(
                routing.bottom() < input_caption.top(),
                "{scale} browser={browser}: routing schematic collides with input caption"
            );
            for index in 0..6 {
                let local = stock_vcf_knob_rect(index);
                let hit = Rect::from_min_max(
                    bounds.min + local.min.to_vec2() * scale,
                    bounds.min + local.max.to_vec2() * scale,
                );
                let radius = stock_vcf_control_radius(index);
                let arc_extent = radius + 16.0 * radius / 45.0 + 1.0;
                let arc = Rect::from_center_size(
                    bounds.min + stock_vcf_control_center(index).to_vec2() * scale,
                    Vec2::splat(2.0 * arc_extent * scale),
                );
                for (label, text_rect) in &drawn {
                    assert!(
                        !hit.intersects(*text_rect),
                        "{scale} browser={browser}: {label:?} overlaps rotary {index} hit target"
                    );
                    assert!(
                        !arc.intersects(*text_rect),
                        "{scale} browser={browser}: {label:?} overlaps rotary {index} arc"
                    );
                }
            }
            for (i, (label, a)) in drawn.iter().enumerate() {
                assert!(
                    bounds.expand(0.5).contains_rect(*a),
                    "{scale}: {label} outside panel"
                );
                for (other, b) in &drawn[i + 1..] {
                    assert!(
                        !a.shrink(0.2 * scale).intersects(b.shrink(0.2 * scale)),
                        "{scale}: {label} overlaps {other}"
                    );
                }
            }
            surfaces.push(drawn);
            assert!(preview.renderer.editing.is_none());
        }
        assert_eq!(
            surfaces[0].len(),
            surfaces[1].len(),
            "rack/browser text count at {scale}"
        );
        for ((rack_label, rack_rect), (browser_label, browser_rect)) in
            surfaces[0].iter().zip(&surfaces[1])
        {
            assert_eq!(rack_label, browser_label, "rack/browser label at {scale}");
            assert!(rack_rect.min.distance(browser_rect.min) < 0.01 && rack_rect.max.distance(browser_rect.max) < 0.01,
                "rack/browser geometry for {rack_label} at {scale}: {rack_rect:?} / {browser_rect:?}");
        }
    }
}

#[test]
fn stock_vcf_held_rotary_keeps_node_and_history_identity_across_graph_edits() {
    for change in ["remove", "insert", "reorder"] {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let mut patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[
            {"id":"earlier","kind":"stock_vcf"},
            {"id":"held","kind":"stock_vcf"}
        ]}))
        .unwrap();
        rack.positions.insert("earlier".into(), pos2(0.0, 0.0));
        rack.positions.insert("held".into(), pos2(330.0, 0.0));
        frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 29);
        let start = at(stock_vcf_knob_rect(0).center() + vec2(330.0, 0.0));
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.0,
            vec![egui::Event::PointerMoved(start), button(start, true)],
            true,
            true,
            29,
        );
        let moved = start - vec2(0.0, 15.0);
        let before = frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.1,
            vec![egui::Event::PointerMoved(moved)],
            true,
            true,
            29,
        )
        .0;
        let Some(RackAction::Command("set_parameter", before)) = before else {
            panic!("initial drag {change}")
        };
        assert_eq!(before["id"], "held");
        let history = before["history_group"].clone();
        assert!(history.is_string());
        match change {
            "remove" => {
                patch.nodes.remove(0);
            }
            "insert" => {
                let mut inserted = patch.nodes[0].clone();
                inserted.id = "inserted".into();
                patch.nodes.insert(0, inserted);
                rack.positions.insert("inserted".into(), pos2(660.0, 0.0));
            }
            "reorder" => patch.nodes.swap(0, 1),
            _ => unreachable!(),
        }
        let after = frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.2,
            vec![egui::Event::PointerMoved(moved - vec2(0.0, 15.0))],
            true,
            true,
            30,
        )
        .0;
        let Some(RackAction::Command("set_parameter", after)) = after else {
            panic!("held rotary lost after {change}")
        };
        assert_eq!(
            after["id"], "held",
            "active rotary transferred after {change}"
        );
        assert_eq!(
            after["parameter"], "frequency",
            "wrong parameter after {change}"
        );
        assert_eq!(after["expected_revision"], 30);
        assert_eq!(
            after["history_group"], history,
            "gesture history changed after {change}"
        );
        assert!(after["value"].as_f64().unwrap() > before["value"].as_f64().unwrap());
    }
}

#[test]
fn stock_vcf_deleted_held_rotary_never_transfers_drag_to_remaining_filter() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let mut patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[
        {"id":"held","kind":"stock_vcf"},
        {"id":"remaining","kind":"stock_vcf"}
    ]}))
    .unwrap();
    rack.positions.insert("held".into(), pos2(0.0, 0.0));
    rack.positions.insert("remaining".into(), pos2(330.0, 0.0));
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 29);
    let start = at(stock_vcf_knob_rect(0).center());
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.0,
        vec![egui::Event::PointerMoved(start), button(start, true)],
        true,
        true,
        29,
    );
    let before = frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.1,
        vec![egui::Event::PointerMoved(start - vec2(0.0, 15.0))],
        true,
        true,
        29,
    )
    .0;
    assert!(
        matches!(before, Some(RackAction::Command("set_parameter", args)) if args["id"] == "held" && args["parameter"] == "frequency")
    );
    patch.nodes.remove(0);
    let after = frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.2,
        vec![egui::Event::PointerMoved(start - vec2(0.0, 30.0))],
        true,
        true,
        30,
    )
    .0;
    assert!(
        after.is_none(),
        "a deleted rotary's drag must not edit the remaining node"
    );
    let released = frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.3,
        vec![button(start - vec2(0.0, 30.0), false)],
        true,
        true,
        30,
    )
    .0;
    assert!(released.is_none());
}

#[test]
fn stock_vcf_multiple_preview_surfaces_do_not_share_widget_identity() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "stock_vcf")
        .unwrap();
    let ctx = egui::Context::default();
    crate::panel_skin::install_fonts(&ctx);
    let mut preview = ModulePreview::new(module);
    for _ in 0..2 {
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 600.0))),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    for (salt, scale, x) in [("detail", 1.0, 24.0), ("thumbnail", 0.42, 350.0)] {
                        ui.push_id(salt, |ui| {
                            preview.show(
                                ui,
                                Rect::from_min_size(
                                    pos2(x, 12.0),
                                    vec2(270.0, PANEL_HEIGHT) * scale,
                                ),
                                &catalog,
                            )
                        });
                    }
                });
            },
        );
        let mut labels = vec![];
        for shape in output.shapes {
            texts(&shape.shape, &mut labels);
        }
        assert!(
            labels
                .iter()
                .all(|(text, _)| !text.contains("use of widget ID")),
            "different preview surfaces need isolated controls"
        );
        assert!(preview.renderer.editing.is_none());
        assert!(preview.renderer.held_manual.is_none());
    }
}
