use super::*;
use crate::construction_motion::MotionMode;
use std::time::Instant;

#[test]
fn sample_hold_exact_voltage_and_construction_cursor_use_the_saved_initial_value() {
    let ctx = egui::Context::default();
    let before: Patch =
        serde_json::from_value(json!({"version":2,"nodes":[{"id":"hold","kind":"sample_hold"}]}))
            .unwrap();
    let mut rack = Rack::default();
    frame_revision_state(&ctx, &mut rack, &before, 0.0, vec![], true, true, 9);
    let at = pos2(99.0, 199.0);
    for (time, pressed) in [(1.0, true), (1.1, false)] {
        frame_revision_state(
            &ctx,
            &mut rack,
            &before,
            time,
            vec![egui::Event::PointerMoved(at), button(at, pressed)],
            true,
            true,
            9,
        );
    }
    let edit = rack
        .editing
        .as_mut()
        .expect("separate initial-voltage field");
    assert_eq!(edit.parameter, "initial");
    assert_eq!((edit.min, edit.max), (-16.0, 16.0));
    assert_eq!(edit.display_unit, "V");
    edit.text = "-7.25".into();
    let action = frame_revision_state(
        &ctx,
        &mut rack,
        &before,
        1.2,
        vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
        true,
        true,
        9,
    )
    .0;
    assert!(
        matches!(action,Some(RackAction::Command("set_parameter",args)) if args["parameter"]=="initial" && args["value"]== -7.25 && args["expected_revision"]==9)
    );
    let mut after = before.clone();
    after.nodes[0].params.insert("initial".into(), -7.25);
    let receipt = json!({"plan_id":9,"completed_steps":1,"last_revision":10,"current_revision":10,"last_commit":{"age_ms":0,"invalidate_existing_cables":false,"plan_id":9,"step_index":0,"before_revision":9,"revision":10,"label":"Set startup voltage","new_modules":[],"new_cables":[],"parameter_changes":[{"id":"hold","kind":"sample_hold","parameter":"initial","old_effective":0.0,"new_effective":-7.25}]}});
    let now = Instant::now();
    rack.observe_construction_transition_at(
        &before,
        &after,
        9,
        10,
        &receipt,
        MotionMode::Full,
        now,
    );
    rack.set_construction_clock_override(Some(now + std::time::Duration::from_millis(160)));
    frame_revision_state(&ctx, &mut rack, &after, 1.3, vec![], true, true, 10);
    let anchor = &rack.construction_anchors[&("hold".into(), "initial".into())];
    assert!(matches!(
        anchor.marker,
        Some(crate::construction_motion::Marker::Rotary { .. })
    ));
    assert_eq!(
        (anchor.old_text.as_str(), anchor.new_text.as_str()),
        ("0.00 V", "-7.25 V")
    );
    assert!(
        rack.module_lights.is_empty(),
        "configured startup volts must not invent measured state"
    );
}

#[test]
fn capture_and_mult_keep_every_named_socket_independently_patchable() {
    let catalog = openpatch_core::catalog();
    for kind in ["sample_hold", "mult"] {
        let module = catalog["modules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["kind"] == kind)
            .unwrap();
        for zoom in [0.42, 1.0] {
            let ctx = egui::Context::default();
            let patch:Patch=serde_json::from_value(json!({"version":2,"nodes":[{"id":"m","kind":kind},{"id":"source","kind":"constant"},{"id":"sink","kind":"attenuverter"}]})).unwrap();
            let mut rack = Rack {
                zoom,
                pan: vec2(230.0, 12.0),
                ..Default::default()
            };
            rack.positions = BTreeMap::from([
                ("m".into(), Pos2::ZERO),
                ("source".into(), pos2(-200.0, 0.0)),
                ("sink".into(), pos2(panel_width(kind) + 30.0, 0.0)),
            ]);
            frame(&ctx, &mut rack, &patch, 0.0, vec![]);
            let mut time = 1.0;
            for input in [true, false] {
                for port in module[if input { "inputs" } else { "outputs" }]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(Value::as_str)
                {
                    let at = fixture_jack(&rack, &patch, "m", port, input);
                    let other = fixture_jack(
                        &rack,
                        &patch,
                        if input { "source" } else { "sink" },
                        if input { "out" } else { "signal" },
                        !input,
                    );
                    let action = if input {
                        drag(&ctx, &mut rack, &patch, time, other, at)
                    } else {
                        drag(&ctx, &mut rack, &patch, time, at, other)
                    };
                    assert!(
                        matches!(action,Some(RackAction::Command("connect",args)) if args[if input {"to"} else {"from"}]=="m" && args[if input {"input"} else {"output"}]==port),
                        "{kind}/{port}/{zoom}"
                    );
                    time += 2.0;
                }
            }
        }
    }
}

#[test]
fn capture_routing_actual_text_clears_enlarged_socket_rims_at_overview_zoom() {
    fn collect(shape: &egui::Shape, text: &mut Vec<(String, Rect)>) {
        match shape {
            egui::Shape::Vec(children) => {
                for child in children {
                    collect(child, text);
                }
            }
            egui::Shape::Text(t) => text.push((
                t.galley.text().into(),
                t.galley.rect.translate(t.pos.to_vec2()),
            )),
            _ => {}
        }
    }
    let catalog = openpatch_core::catalog();
    for kind in [
        "sample_hold",
        "mult",
        "sh_asr",
        "switch_1to4",
        "switch_4to1",
    ] {
        let module = catalog["modules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["kind"] == kind)
            .unwrap();
        for zoom in [0.25, 0.42, 1.0, 1.4] {
            let ctx = egui::Context::default();
            crate::panel_skin::install_fonts(&ctx);
            let mut preview = ModulePreview::new(module);
            let bounds = Rect::from_min_size(pos2(24.0, 12.0), preview.size() * zoom);
            for _ in 0..2 {
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 800.0))),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default()
                            .frame(egui::Frame::NONE)
                            .show(ctx, |ui| {
                                preview.show(ui, bounds, &catalog);
                            });
                    },
                );
                let mut texts = Vec::new();
                for s in &output.shapes {
                    collect(&s.shape, &mut texts);
                }
                for (label, rect) in &texts {
                    if kind == "sh_asr"
                        && ["IN", "TRIG", "HELD VOLTAGE", "OUT"].contains(&label.as_str())
                    {
                        for control in [
                            shift_register_random_rect(),
                            shift_register_range_rect(),
                            shift_register_seed_rect(),
                            shift_register_button_rect(0),
                            shift_register_button_rect(1),
                        ] {
                            let control = (control * zoom).translate(bounds.min.to_vec2());
                            assert!(
                                !rect.intersects(control),
                                "{zoom}: register column header {label} touches an editing target"
                            );
                        }
                    }
                    assert!(
                        bounds.expand(0.5).contains_rect(*rect),
                        "{kind}/{zoom}: {label} outside panel"
                    );
                    for input in [true, false] {
                        let ports = module[if input { "inputs" } else { "outputs" }]
                            .as_array()
                            .unwrap();
                        for (index, port) in ports.iter().enumerate() {
                            let at = bounds.min
                                + port_position(kind, input, index, ports.len(), panel_width(kind))
                                    .to_vec2()
                                    * zoom;
                            let nearest = pos2(
                                at.x.clamp(rect.left(), rect.right()),
                                at.y.clamp(rect.top(), rect.bottom()),
                            );
                            assert!(
                                nearest.distance(at)
                                    >= (13.0 * port_material_scale(kind) - 0.1) * zoom,
                                "{kind}/{zoom}: {label} touches {port} socket: {rect:?}"
                            );
                        }
                    }
                }
            }
        }
    }
}
