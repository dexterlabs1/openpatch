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
fn stock_vcf_all_six_sliders_accept_real_pointer_drags_and_group_history() {
    for (i, parameter) in STOCK_VCF_PARAMS.iter().enumerate() {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let patch = patch();
        frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 29);
        let start = at(stock_vcf_slider_rect(i).center());
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
        let end = start + vec2(if i >= 3 { -22.0 } else { 22.0 }, 0.0);
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
            panic!("slider {i}")
        };
        assert_eq!(args["parameter"], *parameter);
        assert_eq!(args["expected_revision"], 29);
        assert!(args["history_group"]
            .as_str()
            .unwrap()
            .starts_with("rack-stock-vcf-slider-"));
        let value = args["value"].as_f64().unwrap();
        assert!(
            if i >= 3 { value < 0.0 } else { value > 0.0 },
            "{parameter} = {value}"
        );
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
    let controls = (0..6)
        .flat_map(|i| [stock_vcf_slider_rect(i), stock_vcf_value_rect(i)])
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
    for scale in [0.42, 0.85, 1.0] {
        let mut surfaces = vec![];
        for browser in [false, true] {
            let ctx = egui::Context::default();
            crate::panel_skin::install_fonts(&ctx);
            let mut rack = Rack { zoom: scale, ..Rack::default() };
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
                for shape in output.shapes {
                    texts(&shape.shape, &mut drawn);
                }
            }
            for required in [
                "VCF",
                "CUTOFF",
                "RESONANCE",
                "DRIVE",
                "CV DEPTH",
                "FREQ CV",
                "RES CV",
                "DRIVE CV",
                "IN",
                "LP",
                "HP",
                "8 Hz – 22 kHz",
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
fn stock_vcf_held_slider_keeps_node_and_history_identity_across_graph_edits() {
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
        let start = at(stock_vcf_slider_rect(0).center() + vec2(330.0, 0.0));
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
        let moved = start + vec2(15.0, 0.0);
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
            vec![egui::Event::PointerMoved(moved + vec2(15.0, 0.0))],
            true,
            true,
            30,
        )
        .0;
        let Some(RackAction::Command("set_parameter", after)) = after else {
            panic!("held slider lost after {change}")
        };
        assert_eq!(
            after["id"], "held",
            "active slider transferred after {change}"
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
fn stock_vcf_deleted_held_slider_never_transfers_drag_to_remaining_filter() {
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
    let start = at(stock_vcf_slider_rect(0).center());
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
        vec![egui::Event::PointerMoved(start + vec2(15.0, 0.0))],
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
        vec![egui::Event::PointerMoved(start + vec2(30.0, 0.0))],
        true,
        true,
        30,
    )
    .0;
    assert!(
        after.is_none(),
        "a deleted slider's drag must not edit the remaining node"
    );
    let released = frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.3,
        vec![button(start + vec2(30.0, 0.0), false)],
        true,
        true,
        30,
    )
    .0;
    assert!(released.is_none());
}
