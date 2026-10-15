use super::*;

fn patch() -> Patch {
    serde_json::from_value(
        json!({"version":2,"nodes":[{"id":"vco","kind":"multi_vco","params":{
            "frequency":440.0,"pulse_width":0.35,"fm_amount":-0.25,"pwm_amount":0.75,
            "linear_fm":1.0,"soft_sync":1.0
        }}]}),
    )
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

fn step(
    ctx: &egui::Context,
    rack: &mut Rack,
    patch: &Patch,
    time: f64,
    revision: u64,
    events: Vec<egui::Event>,
) -> Option<RackAction> {
    frame_revision_state(ctx, rack, patch, time, events, true, true, revision).0
}

fn open(parameter: &str, revision: u64) -> (egui::Context, Rack, Patch) {
    let patch = patch();
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    assert!(step(&ctx, &mut rack, &patch, 0.0, revision, vec![]).is_none());
    let at = match parameter {
        "frequency" => pos2(99.0, 169.0),
        "pulse_width" => pos2(249.0, 169.0),
        "fm_amount" => pos2(99.0, 269.0),
        "pwm_amount" => pos2(249.0, 269.0),
        _ => panic!("not a multi_vco value target"),
    };
    assert!(step(
        &ctx,
        &mut rack,
        &patch,
        1.0,
        revision,
        vec![egui::Event::PointerMoved(at), button(at, true)]
    )
    .is_none());
    assert!(step(
        &ctx,
        &mut rack,
        &patch,
        1.1,
        revision,
        vec![button(at, false)]
    )
    .is_none());
    assert_eq!(rack.editing.as_ref().unwrap().parameter, parameter);
    assert!(
        rack.precision_controls.is_empty(),
        "opening midway through rendering cannot expose a partial traversal"
    );
    assert!(step(&ctx, &mut rack, &patch, 1.2, revision, vec![]).is_none());
    (ctx, rack, patch)
}

fn expect_navigation(
    action: Option<RackAction>,
    parameter: &str,
    raw: f64,
    revision: u64,
    forward: bool,
) {
    let Some(RackAction::ParameterEditAndNavigate {
        arguments,
        forward: direction,
    }) = action
    else {
        panic!("Tab should emit a guarded navigation edit");
    };
    assert_eq!(direction, forward);
    assert_eq!(arguments["id"], "vco");
    assert_eq!(arguments["parameter"], parameter);
    assert!((arguments["value"].as_f64().unwrap() - raw).abs() < 0.00001);
    assert_eq!(arguments["expected_revision"], revision);
}

#[test]
fn opening_collects_complete_actual_multi_vco_controls_on_following_frame() {
    // Open in the middle and at the end to catch partial lists that accidentally
    // include only controls drawn after the clicked readout.
    for parameter in ["pulse_width", "pwm_amount"] {
        let (_, rack, patch) = open(parameter, 19);
        let names = rack
            .precision_controls
            .iter()
            .map(|control| control.edit.parameter.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            ["frequency", "pulse_width", "fm_amount", "pwm_amount"]
        );
        let actual = [440.0, 0.35, -0.25, 0.75];
        for (control, raw) in rack.precision_controls.iter().zip(actual) {
            assert_eq!(control.edit.revision, 19);
            assert_eq!(control.edit.id, "vco");
            assert_eq!(control.edit.kind, "multi_vco");
            assert!(
                (parameter_edit_raw_value(
                    &control.edit,
                    parsed_parameter_result(&control.edit).unwrap()
                ) - raw)
                    .abs()
                    < 0.00001
            );
        }
        assert!(rack.precision_anchor.unwrap().is_finite());
        assert!(patch.nodes[0].params.contains_key("linear_fm"));
        assert!(patch.nodes[0].params.contains_key("soft_sync"));
    }
}

#[test]
fn tab_and_shift_tab_apply_current_value_and_wrap_with_exact_accepted_revision() {
    for (current, raw, backwards, next) in [
        ("pwm_amount", 0.75, false, "frequency"),
        ("frequency", 440.0, true, "pwm_amount"),
    ] {
        let (ctx, mut rack, mut patch) = open(current, 19);
        let action = step(
            &ctx,
            &mut rack,
            &patch,
            1.3,
            19,
            vec![key(egui::Key::Tab, backwards)],
        );
        expect_navigation(action, current, raw, 19, !backwards);
        assert_eq!(
            rack.editing.as_ref().unwrap().parameter,
            current,
            "navigation must await host acceptance"
        );
        rack.complete_parameter_navigation(Ok(31), !backwards);
        assert_eq!(rack.precision_next.as_ref().unwrap().revision, 31);
        patch.nodes[0].params.insert("frequency".into(), 777.0);
        patch.nodes[0].params.insert("pwm_amount".into(), 0.46);
        assert!(step(&ctx, &mut rack, &patch, 1.4, 31, vec![]).is_none());
        let edit = rack.editing.as_ref().unwrap();
        assert_eq!(edit.parameter, next);
        assert_eq!(edit.revision, 31);
        assert!(!edit.stale);
        assert!(rack.precision_next.is_none());
        let next_raw = if next == "frequency" { 777.0 } else { 0.46 };
        let action = step(
            &ctx,
            &mut rack,
            &patch,
            1.5,
            31,
            vec![key(egui::Key::Tab, false)],
        );
        expect_navigation(action, next, next_raw, 31, true);
    }
}

#[test]
fn accepted_navigation_waits_through_old_gui_snapshot_then_reads_fresh_next_value() {
    let (ctx, mut rack, mut patch) = open("frequency", 41);
    assert!(step(
        &ctx,
        &mut rack,
        &patch,
        1.3,
        41,
        vec![egui::Event::Text("880/2".into())]
    )
    .is_none());
    expect_navigation(
        step(
            &ctx,
            &mut rack,
            &patch,
            1.4,
            41,
            vec![key(egui::Key::Tab, false)],
        ),
        "frequency",
        440.0,
        41,
        true,
    );
    rack.complete_parameter_navigation(Ok(47), true);
    for time in [1.5, 1.6] {
        assert!(step(
            &ctx,
            &mut rack,
            &patch,
            time,
            41,
            vec![key(egui::Key::Tab, false)]
        )
        .is_none());
        let edit = rack.editing.as_ref().unwrap();
        assert_eq!(edit.parameter, "frequency");
        assert_eq!(edit.text, "880/2");
        assert!(!edit.stale);
        assert_eq!(rack.precision_next.as_ref().unwrap().revision, 47);
    }
    patch.nodes[0].params.insert("pulse_width".into(), 0.63);
    assert!(step(&ctx, &mut rack, &patch, 1.7, 47, vec![]).is_none());
    let edit = rack.editing.as_ref().unwrap();
    assert_eq!(edit.parameter, "pulse_width");
    assert_eq!(edit.text.parse::<f32>().unwrap(), 63.0);
    assert_eq!(edit.revision, 47);
    expect_navigation(
        step(
            &ctx,
            &mut rack,
            &patch,
            1.8,
            47,
            vec![key(egui::Key::Tab, true)],
        ),
        "pulse_width",
        0.63,
        47,
        false,
    );
}

#[test]
fn invalid_or_stale_drafts_never_apply_or_move_on_tab() {
    for invalid in [true, false] {
        let (ctx, mut rack, patch) = open("frequency", 19);
        let draft = if invalid { "440/0" } else { "880/2" };
        assert!(step(
            &ctx,
            &mut rack,
            &patch,
            1.3,
            19,
            vec![egui::Event::Text(draft.into())]
        )
        .is_none());
        let revision = if invalid { 19 } else { 20 };
        assert!(step(
            &ctx,
            &mut rack,
            &patch,
            1.4,
            revision,
            vec![key(egui::Key::Tab, false)]
        )
        .is_none());
        assert!(step(
            &ctx,
            &mut rack,
            &patch,
            1.5,
            revision,
            vec![key(egui::Key::Tab, true)]
        )
        .is_none());
        let edit = rack.editing.as_ref().unwrap();
        assert_eq!(edit.parameter, "frequency");
        assert_eq!(edit.text, draft);
        assert_eq!(edit.stale, !invalid);
        assert!(rack.precision_next.is_none());
        assert_eq!(patch.nodes[0].params["frequency"], 440.0);
    }
}

#[test]
fn newer_intervening_patch_does_not_open_next_control_with_stale_accepted_revision() {
    let (ctx, mut rack, mut patch) = open("frequency", 19);
    step(
        &ctx,
        &mut rack,
        &patch,
        1.3,
        19,
        vec![egui::Event::Text("880/2".into())],
    );
    expect_navigation(
        step(
            &ctx,
            &mut rack,
            &patch,
            1.4,
            19,
            vec![key(egui::Key::Tab, false)],
        ),
        "frequency",
        440.0,
        19,
        true,
    );
    rack.complete_parameter_navigation(Ok(20), true);
    patch.nodes[0].params.insert("pulse_width".into(), 0.63);
    assert!(step(&ctx, &mut rack, &patch, 1.5, 21, vec![]).is_none());
    assert!(rack.precision_next.is_none());
    assert!(rack
        .precision_error
        .as_ref()
        .unwrap()
        .contains("Patch changed"));
    let edit = rack.editing.as_ref().unwrap();
    assert!(edit.stale);
    assert_eq!(edit.parameter, "frequency");
    assert_eq!(edit.text, "880/2");
    assert!(step(
        &ctx,
        &mut rack,
        &patch,
        1.6,
        21,
        vec![key(egui::Key::Tab, false)]
    )
    .is_none());
}

#[test]
fn offscreen_next_control_preserves_saved_draft_and_reports_visible_fallback() {
    let (ctx, mut rack, patch) = open("frequency", 19);
    step(
        &ctx,
        &mut rack,
        &patch,
        1.3,
        19,
        vec![egui::Event::Text("880/2".into())],
    );
    expect_navigation(
        step(
            &ctx,
            &mut rack,
            &patch,
            1.4,
            19,
            vec![key(egui::Key::Tab, false)],
        ),
        "frequency",
        440.0,
        19,
        true,
    );
    rack.complete_parameter_navigation(Ok(20), true);
    rack.pan = vec2(-4000.0, -4000.0);
    assert!(step(&ctx, &mut rack, &patch, 1.5, 20, vec![]).is_none());
    assert!(rack.precision_controls.is_empty());
    assert!(rack.precision_anchor.is_none());
    assert!(rack.precision_next.is_none());
    assert!(rack
        .precision_error
        .as_ref()
        .unwrap()
        .contains("no longer visible"));
    let edit = rack.editing.as_ref().unwrap();
    assert_eq!(edit.text, "880/2");
    assert_eq!(edit.parameter, "frequency");
    assert_eq!(edit.revision, 20);
}

#[test]
fn missing_next_descriptor_preserves_current_draft_instead_of_using_old_metadata() {
    let (ctx, mut rack, patch) = open("frequency", 19);
    step(
        &ctx,
        &mut rack,
        &patch,
        1.3,
        19,
        vec![egui::Event::Text("880/2".into())],
    );
    expect_navigation(
        step(
            &ctx,
            &mut rack,
            &patch,
            1.4,
            19,
            vec![key(egui::Key::Tab, false)],
        ),
        "frequency",
        440.0,
        19,
        true,
    );
    rack.complete_parameter_navigation(Ok(20), true);
    let mut catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|module| module["kind"] == "multi_vco")
        .unwrap();
    module["params"]["pulse_width"]["hidden"] = json!(true);
    let mut action = None;
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 700.0))),
            time: Some(1.5),
            focused: true,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    action = rack.show(ui, &patch, &catalog, 48000, 20);
                });
        },
    );
    assert!(action.is_none());
    assert!(!rack
        .precision_controls
        .iter()
        .any(|control| control.edit.parameter == "pulse_width"));
    assert!(rack.precision_next.is_none());
    assert!(rack
        .precision_error
        .as_ref()
        .unwrap()
        .contains("no longer visible"));
    let edit = rack.editing.as_ref().unwrap();
    assert_eq!(edit.text, "880/2");
    assert_eq!(edit.parameter, "frequency");
    assert_eq!(edit.revision, 20);
}

#[test]
fn specialized_focused_readouts_consume_enter_and_space_without_applying_opened_draft() {
    for (kind, parameter) in [
        ("random_values", "seed"),
        ("colored_noise", "level"),
        ("modal_resonator", "damping"),
    ] {
        for activation in [egui::Key::Enter, egui::Key::Space] {
            let patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[{
                "id":"m","kind":kind,"params":{parameter:if parameter == "seed" {123.0} else {0.4}}
            }]}))
            .unwrap();
            let before = serde_json::to_value(&patch).unwrap();
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            step(&ctx, &mut rack, &patch, 0.0, 53, vec![]);
            let mut focused = false;
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 700.0))),
                    time: Some(0.1),
                    focused: true,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE)
                        .show(ctx, |ui| {
                            ui.add_enabled_ui(true, |ui| {
                                let id = match kind {
                                    "random_values" => ui.id().with(("m", "random-values-seed")),
                                    "colored_noise" => {
                                        ui.id().with(("noise-value", "m", parameter))
                                    }
                                    _ => ui.id().with(("resonator-value", "m", parameter)),
                                };
                                ctx.memory_mut(|memory| memory.request_focus(id));
                                assert!(rack
                                    .show(ui, &patch, &openpatch_core::catalog(), 48000, 53)
                                    .is_none());
                                focused = ctx.memory(|memory| memory.has_focus(id));
                            });
                        });
                },
            );
            assert!(
                focused,
                "{kind} {parameter} must own focus before activation"
            );
            assert!(
                step(
                    &ctx,
                    &mut rack,
                    &patch,
                    1.0,
                    53,
                    vec![key(activation, false)]
                )
                .is_none(),
                "{kind}: opening cannot commit on the same {activation:?}"
            );
            assert!(
                !ctx.input(|input| input.key_pressed(activation)),
                "{kind}: activation must not reach host shortcuts"
            );
            let edit = rack
                .editing
                .as_ref()
                .expect("focused readout opens a draft");
            assert_eq!(edit.parameter, parameter);
            assert_eq!(edit.id, "m");
            assert_eq!(edit.revision, 53);
            assert_eq!(edit.surface, ParameterEditSurface::Window);
            assert_eq!(serde_json::to_value(&patch).unwrap(), before);
            let release = egui::Event::Key {
                key: activation,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            };
            assert!(step(&ctx, &mut rack, &patch, 1.1, 53, vec![release]).is_none());
            // A later independent text entry and Enter must still work.
            let (text, expected) = if parameter == "seed" {
                ("456", 456.0)
            } else if kind == "modal_resonator" {
                ("25", 0.25)
            } else {
                ("0.25", 0.25)
            };
            assert!(step(
                &ctx,
                &mut rack,
                &patch,
                1.2,
                53,
                vec![egui::Event::Text(text.into())]
            )
            .is_none());
            let action = step(
                &ctx,
                &mut rack,
                &patch,
                1.3,
                53,
                vec![key(egui::Key::Enter, false)],
            );
            assert!(
                matches!(action, Some(RackAction::Command("set_parameter", arguments))
                if arguments["id"] == "m" && arguments["parameter"] == parameter
                    && arguments["expected_revision"] == 53
                    && (arguments["value"].as_f64().unwrap() - expected).abs() < 0.00001),
                "{kind} {activation:?}"
            );
        }
    }
}

#[test]
fn rendered_transformed_descriptors_preserve_db_slew_and_clock_ratio_during_apply() {
    for (kind, parameter, at, text, expected) in [
        ("vca_mix", "level1", pos2(61.5, 171.0), "-12.0412", 0.5),
        ("process", "slew", pos2(144.0, 170.0), "1000*2", 1.0),
        ("multi_lfo", "frequency", pos2(99.0, 169.0), "1+1", 4.0),
    ] {
        let mut patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[
            {"id":"target","kind":kind,"params":{parameter:expected}},
            {"id":"clock","kind":"gate"}
        ]})).unwrap();
        if kind == "vca_mix" {
            patch.nodes[0].params.insert("level2".into(), 0.25);
        }
        if kind == "multi_lfo" {
            patch.nodes[0].params.insert("pulse_width".into(), 0.37);
            let cable: Cable =
                serde_json::from_value(json!({"from":"clock","to":"target","input":"clock"}))
                    .unwrap();
            patch.cables.push(cable);
        }
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        step(&ctx, &mut rack, &patch, 0.0, 61, vec![]);
        step(
            &ctx,
            &mut rack,
            &patch,
            1.0,
            61,
            vec![egui::Event::PointerMoved(at), button(at, true)],
        );
        step(&ctx, &mut rack, &patch, 1.1, 61, vec![button(at, false)]);
        step(&ctx, &mut rack, &patch, 1.2, 61, vec![]);
        let descriptor = &rack
            .precision_controls
            .iter()
            .find(|control| control.edit.parameter == parameter)
            .expect("actual rendered control registers its editor")
            .edit;
        match kind {
            "vca_mix" => {
                assert_eq!(descriptor.gain_power, Some(2.0));
                assert_eq!(descriptor.display_unit, "dB");
            }
            "process" => {
                assert!(descriptor.slew_min.is_some());
                assert_eq!(descriptor.text.parse::<f32>().unwrap(), 2000.0);
            }
            _ => {
                assert_eq!(descriptor.display_scale, 0.5);
                assert_eq!(descriptor.text.parse::<f32>().unwrap(), 2.0);
            }
        }
        assert!(step(
            &ctx,
            &mut rack,
            &patch,
            1.3,
            61,
            vec![egui::Event::Text(text.into())]
        )
        .is_none());
        if kind == "process" {
            assert_eq!(
                rack.precision_controls.len(),
                1,
                "a single native type-entry control retains plain Apply/Enter"
            );
            assert!(
                matches!(step(&ctx, &mut rack, &patch, 1.4, 61, vec![key(egui::Key::Enter, false)]),
                Some(RackAction::Command("set_parameter", arguments)) if arguments["parameter"] == parameter
                    && arguments["expected_revision"] == 61 && (arguments["value"].as_f64().unwrap() - expected).abs() < 0.00001)
            );
        } else {
            assert!(
                matches!(step(&ctx, &mut rack, &patch, 1.4, 61, vec![key(egui::Key::Tab, false)]),
                Some(RackAction::ParameterEditAndNavigate { arguments, forward: true }) if arguments["parameter"] == parameter
                    && arguments["expected_revision"] == 61 && (arguments["value"].as_f64().unwrap() - expected).abs() < 0.00001)
            );
            rack.complete_parameter_navigation(Ok(67), true);
            step(&ctx, &mut rack, &patch, 1.5, 67, vec![]);
            let next = rack.editing.as_ref().unwrap();
            let (next_parameter, next_raw) = if kind == "vca_mix" {
                ("level2", 0.25)
            } else {
                ("pulse_width", 0.37)
            };
            assert_eq!(next.parameter, next_parameter);
            assert_eq!(next.revision, 67);
            assert!(
                matches!(step(&ctx, &mut rack, &patch, 1.6, 67, vec![key(egui::Key::Tab, false)]),
                Some(RackAction::ParameterEditAndNavigate { arguments, forward: true }) if arguments["parameter"] == next_parameter
                    && arguments["expected_revision"] == 67 && (arguments["value"].as_f64().unwrap() - next_raw).abs() < 0.00001)
            );
        }
    }
}
