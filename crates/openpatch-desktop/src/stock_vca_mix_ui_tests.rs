use super::*;

fn stock_vca_mix_patch(level1: f32) -> Patch {
    serde_json::from_value(json!({
        "version":2,
        "nodes":[{"id":"mix","kind":"stock_vca_mix","params":{"level1":level1}}]
    }))
    .unwrap()
}

fn local(point: Pos2) -> Pos2 {
    point + vec2(24.0, 12.0)
}

fn text_shapes(output: &egui::FullOutput) -> Vec<(String, Rect)> {
    fn visit(shape: &egui::Shape, out: &mut Vec<(String, Rect)>) {
        match shape {
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    visit(shape, out);
                }
            }
            egui::Shape::Text(text) if !text.galley.text().is_empty() => out.push((
                text.galley.text().into(),
                Rect::from_min_size(text.pos, text.galley.size()),
            )),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for shape in &output.shapes {
        visit(&shape.shape, &mut out);
    }
    out
}

#[test]
fn stock_vca_mix_compact_geometry_preserves_native_port_order_and_fits() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|module| module["kind"] == "stock_vca_mix")
        .unwrap();
    assert_eq!(panel_width("stock_vca_mix"), 270.0);
    assert_eq!(title("stock_vca_mix"), "STOCK VCA MIX");
    assert_eq!(
        module["inputs"],
        json!(["mix_cv", "in1", "in2", "in3", "in4", "cv1", "cv2", "cv3", "cv4"])
    );
    assert_eq!(module["outputs"], json!(["mix", "out1", "out2", "out3", "out4"]));
    assert_eq!(
        (0..5).map(stock_vca_mix_column).collect::<Vec<_>>(),
        [35.0, 85.0, 135.0, 185.0, 235.0]
    );
    let knobs = (0..5).map(stock_vca_mix_knob_rect).collect::<Vec<_>>();
    assert!(knobs.iter().all(|knob| knob.width() == 44.0 && knob.height() == 44.0));
    for (index, knob) in knobs.iter().enumerate() {
        assert!(Rect::from_min_size(Pos2::ZERO, vec2(270.0, PANEL_HEIGHT)).contains_rect(*knob));
        assert!(knobs.iter().skip(index + 1).all(|other| !knob.intersects(*other)));
    }
    let bounds = Rect::from_min_size(Pos2::ZERO, vec2(panel_width("stock_vca_mix"), PANEL_HEIGHT));
    let inputs = (0..9)
        .map(|index| port_position("stock_vca_mix", true, index, 9, bounds.width()))
        .collect::<Vec<_>>();
    let outputs = (0..5)
        .map(|index| port_position("stock_vca_mix", false, index, 5, bounds.width()))
        .collect::<Vec<_>>();
    for point in inputs.iter().chain(&outputs) {
        assert!(bounds.shrink(14.0).contains(*point));
    }
    assert_eq!(inputs[0], pos2(235.0, 336.0));
    assert_eq!(inputs[1], pos2(35.0, 292.0));
    assert_eq!(inputs[5], pos2(35.0, 336.0));
    assert_eq!(outputs[0], pos2(235.0, 380.0));
    assert_eq!(outputs[1], pos2(35.0, 380.0));
    assert!(outputs[0].y + 13.0 * STOCK_VCA_MIX_PORT_SCALE <= PANEL_HEIGHT);
    assert_eq!(STOCK_VCA_MIX_PORT_SCALE, 1.07);
    for (index, point) in inputs.iter().chain(&outputs).enumerate() {
        for other in inputs.iter().chain(&outputs).skip(index + 1) {
            assert!(point.distance(*other) >= 36.0, "jack centers must remain distinct");
        }
    }
}

#[test]
fn stock_vca_mix_print_never_covers_the_enlarged_socket_rings() {
    let ctx = egui::Context::default();
    crate::panel_skin::install_fonts(&ctx);
    let mut rack = Rack::default();
    let patch = stock_vca_mix_patch(1.0);
    let (_, output) = frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 1);
    let ports = (0..9).map(|index| port_position("stock_vca_mix", true, index, 9, 270.0))
        .chain((0..5).map(|index| port_position("stock_vca_mix", false, index, 5, 270.0)))
        .map(local).collect::<Vec<_>>();
    for (label, bounds) in text_shapes(&output) {
        for center in &ports {
            let nearest = pos2(center.x.clamp(bounds.left(), bounds.right()),
                center.y.clamp(bounds.top(), bounds.bottom()));
            assert!(nearest.distance(*center) > 13.0 * STOCK_VCA_MIX_PORT_SCALE,
                "{label:?} {bounds:?} covers the socket centered at {center:?}");
        }
    }
}

#[test]
fn stock_vca_mix_raw_values_remain_visible_while_precision_commits_are_bounded() {
    let patch = stock_vca_mix_patch(2.75);
    let edit = stock_vca_mix_edit(&patch.nodes[0], "level1", 41);
    assert_eq!(edit.text, "2.75");
    assert!(edit.display_unit.is_empty(), "the raw strip setting is not the squared DSP gain");
    assert_eq!(edit.min, 0.0);
    assert_eq!(edit.max, std::f32::consts::SQRT_2);
    assert!(stock_vca_mix_raw_label(2.75).len() < 12);
    for (value, valid) in [
        ("0", true),
        ("1.4142135", true),
        ("1.4143", false),
        ("-0.01", false),
        ("nan", false),
        ("inf", false),
    ] {
        let mut draft = edit.clone();
        draft.text = value.into();
        assert_eq!(parsed_parameter_result(&draft).is_ok(), valid, "{value}");
    }
    let mix = stock_vca_mix_edit(&patch.nodes[0], "mix_level", 41);
    assert_eq!(mix.max, 2.0);
    assert_eq!(mix.display_unit, "x", "the master setting is linear");
}

#[test]
fn stock_vca_mix_inline_modes_and_rotary_edits_use_the_observed_revision() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch = stock_vca_mix_patch(1.0);
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 41);

    let channel_mode = local(Rect::from_min_max(pos2(10.0, 62.0), pos2(148.0, 84.0)).center());
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.0,
        vec![egui::Event::PointerMoved(channel_mode), button(channel_mode, true)],
        true,
        true,
        41,
    );
    let action = frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.1,
        vec![button(channel_mode, false)],
        true,
        true,
        41,
    )
    .0;
    let Some(RackAction::Command("set_parameter", args)) = action else {
        panic!("channel mode click")
    };
    assert_eq!(args["id"], "mix");
    assert_eq!(args["parameter"], "channel_exp");
    assert_eq!(args["value"], 1.0);
    assert_eq!(args["expected_revision"], 41);

    let knob_center = local(stock_vca_mix_knob_center(0));
    let click = frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        2.0,
        vec![egui::Event::PointerMoved(knob_center), button(knob_center, true)],
        true,
        true,
        42,
    ).0;
    assert!(click.is_none(), "pressing a rotary control must not change its value");
    let click = frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        2.1,
        vec![button(knob_center, false)],
        true,
        true,
        42,
    )
    .0;
    assert!(click.is_none(), "a center click focuses without jumping to half travel");
    let action = frame_revision_state(&ctx, &mut rack, &patch, 2.2,
        vec![stock_vca_mix_key(egui::Key::End)], true, true, 42).0;
    let Some(RackAction::Command("set_parameter", args)) = action else {
        panic!("focused rotary End key must select its maximum")
    };
    assert_eq!(args["parameter"], "level1");
    assert_eq!(args["value"], std::f32::consts::SQRT_2);
    assert_eq!(args["expected_revision"], 42);
}

fn stock_vca_mix_key(key: egui::Key) -> egui::Event {
    egui::Event::Key { key, physical_key: None, pressed: true, repeat: false,
        modifiers: egui::Modifiers::NONE }
}

#[test]
fn stock_vca_mix_rotary_drag_is_relative_and_modal_owners_block_edits() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch = stock_vca_mix_patch(0.5);
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 17);
    let center = local(stock_vca_mix_knob_center(0));
    assert!(frame_revision_state(&ctx, &mut rack, &patch, 1.0,
        vec![egui::Event::PointerMoved(center), button(center, true)], true, true, 17).0.is_none());
    let moved = center - vec2(0.0, 15.0);
    let action = frame_revision_state(&ctx, &mut rack, &patch, 1.1,
        vec![egui::Event::PointerMoved(moved)], true, true, 17).0;
    let Some(RackAction::Command("set_parameter", args)) = action else { panic!("relative knob drag") };
    let actual = args["value"].as_f64().unwrap() as f32;
    assert!((actual - (0.5 + std::f32::consts::SQRT_2 * 0.1)).abs() < 1e-6,
        "15 pixels adds one tenth of travel from the grabbed value");
    assert!(args["history_group"].as_str().is_some());
    frame_revision_state(&ctx, &mut rack, &patch, 1.2, vec![button(moved, false)], true, true, 17);
    rack.set_shortcuts_enabled(false);
    for (index, key) in [egui::Key::ArrowUp, egui::Key::Home, egui::Key::End].into_iter().enumerate() {
        let action = frame_revision_state(&ctx, &mut rack, &patch, 2.0 + index as f64 * 0.1,
            vec![stock_vca_mix_key(key)], true, true, 17).0;
        assert!(action.is_none(), "a modal owner must suppress focused-knob {key:?}");
    }
    let mode = local(pos2(79.0, 73.0));
    frame_revision_state(&ctx, &mut rack, &patch, 3.0,
        vec![egui::Event::PointerMoved(mode), button(mode, true)], true, true, 17);
    assert!(frame_revision_state(&ctx, &mut rack, &patch, 3.1,
        vec![button(mode, false)], true, true, 17).0.is_none(), "modal ownership also suppresses mode clicks");
}

#[test]
fn stock_vca_mix_exact_readout_opens_and_registers_mcp_precision_targets() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch = stock_vca_mix_patch(1.0);
    rack.request_precision_control_for_smoke("mix", "level1");
    rack.precision_collect_id = Some("mix".into());
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 9);
    assert!(rack.editing.as_ref().is_some_and(|edit| {
        edit.id == "mix" && edit.kind == "stock_vca_mix" && edit.parameter == "level1"
    }));
    // Exact entry opens on the requested frame; its rendered sibling controls
    // are collected on the following frame for ordinary Tab traversal.
    frame_revision_state(&ctx, &mut rack, &patch, 0.1, vec![], true, true, 9);
    assert_eq!(
        rack.precision_controls
            .iter()
            .map(|control| control.edit.parameter.as_str())
            .collect::<Vec<_>>(),
        ["channel_exp", "mix_exp", "level1", "level2", "level3", "level4", "mix_level"]
    );
    assert_eq!(
        rack.precision_anchor,
        Some(
            Rect::from_center_size(
                pos2(35.0, STOCK_VCA_MIX_VALUE_Y),
                vec2(48.0, 17.0),
            )
                .translate(vec2(24.0, 12.0))
        )
    );
}

#[test]
fn stock_vca_mix_parameter_cues_anchor_the_actual_control_and_meter_offline_is_explicit() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch = stock_vca_mix_patch(0.5);
    rack.construction_animation.parameters.push(crate::construction_motion::ParameterCue {
        id: "mix".into(),
        kind: "stock_vca_mix".into(),
        parameter: "level1".into(),
        old: 0.25,
        new: 0.5,
        started: std::time::Instant::now(),
    });
    let (_, offline) = frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 9);
    let offline_text = text_shapes(&offline)
        .into_iter()
        .map(|(text, _)| text)
        .collect::<Vec<_>>();
    assert!(offline_text.iter().any(|text| text == "VU OFFLINE"));
    assert!(offline_text.iter().any(|text| text == "h = max(V/10, 0) · 1 unpatched"));
    assert!(rack.construction_anchors.contains_key(&("mix".into(), "level1".into())));

    let mut lights = ModuleLightState {
        count: 4,
        ..Default::default()
    };
    lights.brightness[0][0] = 0.75;
    lights.brightness[1][0] = 0.25;
    rack.module_lights.insert("mix".into(), lights);
    rack.set_room_brightness(0.0);
    let (_, live) = frame_revision_state(&ctx, &mut rack, &patch, 0.1, vec![], true, true, 9);
    let live_text = text_shapes(&live)
        .into_iter()
        .map(|(text, _)| text)
        .collect::<Vec<_>>();
    assert!(live_text.iter().any(|text| text == "VU LIVE"));
    assert!(!live_text.iter().any(|text| text == "VU OFFLINE"));
}

#[test]
fn stock_vca_mix_out_of_range_saved_level_is_flagged_without_rewriting_or_calling_it_gain() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch = stock_vca_mix_patch(2.75);
    let (action, output) = frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 9);
    assert!(action.is_none(), "drawing a saved value must remain read-only");
    assert_eq!(patch.nodes[0].params["level1"], 2.75);
    let rendered = text_shapes(&output)
        .into_iter()
        .map(|(text, _)| text)
        .collect::<Vec<_>>();
    assert!(rendered.iter().any(|text| text == "!"), "out-of-range travel has an explicit warning marker");
    assert!(rendered.iter().any(|text| text == "2.750"));
    assert!(!rendered.iter().any(|text| text == "2.750×"));
}

#[test]
fn stock_vca_mix_signal_atlas_print_is_source_backed_and_flags_have_bounded_entries() {
    let patch = stock_vca_mix_patch(2.75);
    let node = &patch.nodes[0];
    let channel = stock_vca_mix_edit(node, "channel_exp", 41);
    let master = stock_vca_mix_edit(node, "mix_exp", 41);
    for edit in [&channel, &master] {
        assert_eq!((edit.min, edit.max, edit.step), (0.0, 1.0, Some(1.0)));
        for (value, valid) in [("0", true), ("1", true), ("0.5", false), ("2", false)] {
            let mut draft = edit.clone();
            draft.text = value.into();
            assert_eq!(parsed_parameter_result(&draft).is_ok(), valid, "{value}");
        }
    }
    let labels = [
        stock_vca_mix_port_label("in1"),
        stock_vca_mix_port_label("cv1"),
        stock_vca_mix_port_label("out1"),
        stock_vca_mix_port_label("mix_cv"),
        stock_vca_mix_port_label("mix"),
    ];
    assert_eq!(labels, ["IN 1", "CV 1", "OUT 1", "MIX CV", "MIX"]);
}

#[test]
fn stock_vca_mix_browser_preview_and_rack_use_the_same_faceplate_geometry() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|module| module["kind"] == "stock_vca_mix")
        .unwrap();
    for scale in [0.42, 0.85, 1.0] {
        let mut surfaces = Vec::new();
        for browser in [false, true] {
            let ctx = egui::Context::default();
            crate::panel_skin::install_fonts(&ctx);
            let mut preview = ModulePreview::new(module);
            let size = vec2(panel_width("stock_vca_mix"), PANEL_HEIGHT);
            let bounds = Rect::from_min_size(pos2(24.0, 12.0), size * scale);
            let fitted_scale = (bounds.width() / size.x).min(bounds.height() / size.y);
            let fitted = Rect::from_center_size(bounds.center(), size * fitted_scale);
            let patch = preview.patch.clone();
            let mut rack = Rack {
                zoom: fitted_scale,
                pan: Vec2::ZERO,
                ..Rack::default()
            };
            rack.positions.insert(String::new(), Pos2::ZERO);
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 700.0))),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE)
                        .show(ctx, |ui| {
                            if browser {
                                preview.show(ui, bounds, &catalog);
                            } else {
                                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(fitted));
                                child.set_clip_rect(ui.clip_rect().intersect(bounds));
                                rack.render_modules(
                                    &mut child,
                                    &patch,
                                    &catalog,
                                    fitted,
                                    48_000,
                                    0,
                                    RenderMode::Preview,
                                );
                            }
                        });
                },
            );
            surfaces.push(text_shapes(&output));
        }
        assert_eq!(surfaces[0], surfaces[1], "browser/rack face mismatch at {scale}");
    }
}
