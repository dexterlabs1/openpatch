use super::*;

fn labels(shape: &egui::Shape, result: &mut Vec<(String, Rect)>) {
    match shape {
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                labels(shape, result);
            }
        }
        egui::Shape::Text(text) => result.push((
            text.galley.text().into(),
            text.galley.rect.translate(text.pos.to_vec2()),
        )),
        _ => {}
    }
}

#[test]
fn rackmcp_authored_selection_distinguishes_notes_drum_hits_and_accents_without_live_state() {
    let ctx = egui::Context::default();
    crate::panel_skin::install_fonts(&ctx);
    let patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[{
        "id":"seq","kind":"rackmcp_sequencer","params":{"bpm":137,"swing":61},
        "rackmcp_sequence":{"data":{"selectedPattern":"Bass","patterns":[
            {"name":"A","length":16},
            {"name":"Bass","length":8,"stepsPerBeat":2,"tracks":[{"voices":4,"steps":[
                {"step":0,"notes":[0,4,7],"accent":true},
                {"step":1,"notes":[]},
                {"step":2,"notes":[12],"prob":0,"accent":true}
            ]}]}
        ],"lanes":[{"moduleId":"12","paramId":3,"points":[{"bar":0,"value":0.5}]}]}}
    }]}))
    .unwrap();
    patch.compile().unwrap();
    let mut rack = Rack::default();
    let mut snapshots = Vec::new();
    for time in [0.0, 1.0] {
        let (_, output) =
            frame_revision_state(&ctx, &mut rack, &patch, time, vec![], true, true, 9);
        let mut texts = Vec::new();
        for shape in &output.shapes {
            labels(&shape.shape, &mut texts);
        }
        for expected in [
            "RACKMCP SEQUENCER",
            "AUTHORED · Bass · 8 STEPS · 2 / BEAT",
            "AUTOMATION · 1 / 8 LANES · 0 NATIVE",
            "SOURCE TARGETS · 12:3",
            "137.0 BPM",
            "61.0%",
            "AMBER NOTES · GREEN HITS · DOT ACCENT",
        ] {
            assert!(
                texts.iter().any(|(text, _)| text == expected),
                "Missing {expected}"
            );
        }
        let mut cells = Vec::new();
        let mut accents = 0;
        for shape in &output.shapes {
            match &shape.shape {
                egui::Shape::Rect(cell)
                    if (cell.rect.height() - 8.0).abs() < 0.01
                        && [
                            Color32::from_rgb(229, 183, 103),
                            Color32::from_rgb(137, 174, 137),
                        ]
                        .contains(&cell.fill) =>
                {
                    cells.push((cell.rect, cell.fill))
                }
                egui::Shape::Circle(dot)
                    if (dot.radius - 1.4).abs() < 0.01
                        && dot.fill == Color32::from_rgb(250, 234, 190) =>
                {
                    accents += 1
                }
                _ => {}
            }
        }
        assert_eq!(
            cells.len(),
            2,
            "Notes and source drum hits play; zero probability stays unfilled"
        );
        assert_eq!(accents, 1, "Only the enabled authored accent is marked");
        snapshots.push(cells);
    }
    assert_eq!(
        snapshots[0], snapshots[1],
        "Elapsed UI time must not invent a playhead"
    );
    assert!(rack.sequence_states.is_empty());
    assert!(rack.module_lights.is_empty());
}

#[test]
fn rackmcp_default_preview_exposes_all_ports_with_readable_print_at_rack_and_browser_scales() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|module| module["kind"] == "rackmcp_sequencer")
        .unwrap();
    for zoom in [0.25, 0.42, 1.0, 1.4] {
        let ctx = egui::Context::default();
        crate::panel_skin::install_fonts(&ctx);
        let mut preview = ModulePreview::new(module);
        assert!(
            preview.patch.nodes[0].rackmcp_sequence.is_none(),
            "Default preview must exercise missing-payload fallback"
        );
        let bounds = Rect::from_min_size(pos2(24.0, 12.0), preview.size() * zoom);
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 800.0))),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| preview.show(ui, bounds, &catalog));
            },
        );
        let mut texts = Vec::new();
        for shape in output.shapes {
            labels(&shape.shape, &mut texts);
        }
        for expected in [
            "AUTHORED · A · 16 STEPS · 4 / BEAT",
            "120.0 BPM",
            "50.0%",
            "Idle",
        ] {
            assert!(
                texts.iter().any(|(text, _)| text == expected),
                "{zoom}: missing {expected}"
            );
        }
        let mut ports = Vec::new();
        for input in [true, false] {
            let names = module[if input { "inputs" } else { "outputs" }]
                .as_array()
                .unwrap();
            for (index, name) in names.iter().enumerate() {
                let center = bounds.min
                    + port_position("rackmcp_sequencer", input, index, names.len(), 720.0)
                        .to_vec2()
                        * zoom;
                let radius = if input { 10.3 } else { 13.0 }
                    * port_material_scale("rackmcp_sequencer")
                    * zoom;
                assert!(
                    bounds.contains_rect(Rect::from_center_size(center, Vec2::splat(2.0 * radius))),
                    "{zoom}: {name} outside panel"
                );
                ports.push((center, radius));
                for (label, printed) in &texts {
                    assert!(
                        printed.distance_to_pos(center) >= radius - 0.1 * zoom,
                        "{zoom}: {label} touches {name}"
                    );
                }
            }
        }
        assert_eq!(ports.len(), 20);
        for (index, (center, radius)) in ports.iter().enumerate() {
            for (other, other_radius) in &ports[index + 1..] {
                assert!(
                    center.distance(*other) > radius + other_radius + 8.0 * zoom,
                    "{zoom}: sockets overlap"
                );
            }
        }
        for (label, printed) in texts {
            assert!(
                bounds.expand(0.5).contains_rect(printed),
                "{zoom}: {label} outside panel"
            );
        }
    }
}

#[test]
fn rackmcp_controls_and_all_eighteen_output_cable_targets_are_independent() {
    let ctx = egui::Context::default();
    let patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[{"id":"seq","kind":"rackmcp_sequencer"},{"id":"out","kind":"audio_out"}]})).unwrap();
    let mut rack = Rack::default();
    frame(&ctx, &mut rack, &patch, 0.0, vec![]);
    let origin = pos2(24.0, 12.0);
    for (index, name) in ["running", "reset"].into_iter().enumerate() {
        let target = origin + atlas_rackmcp_control(name).0.to_vec2() + vec2(20.0, 0.0);
        let time = 1.0 + index as f64;
        frame(
            &ctx,
            &mut rack,
            &patch,
            time,
            vec![egui::Event::PointerMoved(target), button(target, true)],
        );
        let action = frame(
            &ctx,
            &mut rack,
            &patch,
            time + 0.1,
            vec![button(target, false)],
        );
        assert!(
            matches!(action, Some(RackAction::Command("set_parameter", ref args)) if args["parameter"] == name && args["value"].as_f64() == Some(1.0)),
            "{name}: did not emit its own enabled choice"
        );
    }
    for (index, name) in ["bpm", "swing"].into_iter().enumerate() {
        let target = origin + atlas_rackmcp_control(name).0.to_vec2();
        let time = 3.0 + index as f64;
        frame(
            &ctx,
            &mut rack,
            &patch,
            time,
            vec![egui::Event::PointerMoved(target), button(target, true)],
        );
        let action = frame(
            &ctx,
            &mut rack,
            &patch,
            time + 0.1,
            vec![egui::Event::PointerMoved(target - vec2(0.0, 12.0))],
        );
        let initial = if name == "bpm" { 120.0 } else { 50.0 };
        assert!(
            matches!(action, Some(RackAction::Command("set_parameter", ref args)) if args["parameter"] == name && args["value"].as_f64().unwrap() > initial),
            "{name}: did not emit its own increased value"
        );
        frame(
            &ctx,
            &mut rack,
            &patch,
            time + 0.2,
            vec![button(target - vec2(0.0, 12.0), false)],
        );
    }
    let outputs = [
        "pitch1",
        "pitch2",
        "pitch3",
        "pitch4",
        "gate1",
        "gate2",
        "gate3",
        "gate4",
        "velocity1",
        "velocity2",
        "velocity3",
        "velocity4",
        "accent1",
        "accent2",
        "accent3",
        "accent4",
        "clock",
        "reset",
    ];
    let destination = origin
        + vec2(720.0, 0.0)
        + port_position("audio_out", true, 0, 1, panel_width("audio_out")).to_vec2();
    for (index, name) in outputs.into_iter().enumerate() {
        let source = origin + port_position("rackmcp_sequencer", false, index, 18, 720.0).to_vec2();
        let action = drag(
            &ctx,
            &mut rack,
            &patch,
            10.0 + index as f64,
            source,
            destination,
        );
        assert!(
            matches!(action, Some(RackAction::Command("connect", ref args)) if args["output"] == name && args["to"] == "out"),
            "{name}: did not connect its own output"
        );
    }
}

#[test]
#[ignore = "writes twelve CPU review PNGs to OPENPATCH_RACKMCP_REVIEW_DIR"]
fn rackmcp_sequencer_headless_review() {
    let destination =
        std::path::PathBuf::from(std::env::var_os("OPENPATCH_RACKMCP_REVIEW_DIR").expect(
            "OPENPATCH_RACKMCP_REVIEW_DIR must explicitly select the review output directory",
        ));
    std::fs::create_dir_all(&destination).unwrap();
    let patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[{
        "id":"seq","kind":"rackmcp_sequencer","params":{"bpm":124,"swing":58,"running":0},
        "rackmcp_sequence":{"data":{"selectedPattern":"Verse","patterns":[
            {"name":"Intro","length":8,"stepsPerBeat":4,"tracks":[{"steps":[{"step":0,"notes":[0],"accent":true},{"step":4,"notes":[7]}]}]},
            {"name":"Verse","length":16,"stepsPerBeat":4,"tracks":[
                {"voices":2,"glideMs":35,"steps":[
                    {"step":0,"notes":[0,7],"vel":0.9,"accent":true},
                    {"step":2,"notes":[3,10],"vel":0.7,"slide":true},
                    {"step":4,"notes":[5,12],"vel":0.8,"accent":true},
                    {"step":6,"notes":[7,14],"vel":0.7,"ratchet":2},
                    {"step":8,"notes":[0,12],"vel":1,"accent":true},
                    {"step":10,"notes":[-2,5],"prob":0},
                    {"step":12,"notes":[-5,2],"vel":0.8,"accent":true},
                    {"step":14,"notes":[-2,5],"vel":0.7}
                ]},
                {"voices":1,"steps":[
                    {"step":0,"notes":[],"accent":true},{"step":4,"notes":[]},
                    {"step":8,"notes":[],"accent":true},{"step":12,"notes":[]}
                ]},
                {"voices":4,"steps":[
                    {"step":0,"notes":[0,3,7,10],"gate":0.8,"accent":true},
                    {"step":8,"notes":[5,8,12,15],"gate":0.8}
                ]},
                {"voices":1,"steps":[
                    {"step":2,"notes":[24],"vel":0.5},{"step":6,"notes":[19],"vel":0.6},
                    {"step":10,"notes":[24],"vel":0.5},{"step":14,"notes":[26],"vel":0.7,"accent":true}
                ]}
            ]},
            {"name":"Fill","length":8,"stepsPerBeat":4,"tracks":[]}
        ],"lanes":[
            {"moduleId":"42","paramId":0,"points":[{"bar":0,"value":0.25},{"bar":1,"value":0.8,"curve":"easeInOut"}]},
            {"moduleId":"99","paramId":4,"mode":"raw","points":[{"bar":0,"value":0.5}]}
        ]},"targets":[{"source_module_id":"42","source_param_id":0,"module":"vca","parameter":"level"}]}
    },{"id":"vca","kind":"stock_vca"}]})).unwrap();
    patch.compile().unwrap();
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|module| module["kind"] == "rackmcp_sequencer")
        .unwrap();
    for (illumination, brightness) in [("normal", 1.0), ("dim", 0.0)] {
        for (view, scale) in [("rack", 1.0), ("thumbnail", 0.42)] {
            for panel_state in ["live", "authored", "default-preview"] {
                let authored = panel_state != "default-preview";
                let ctx = egui::Context::default();
                crate::panel_skin::install_fonts(&ctx);
                let mut capture = crate::headless_review::CpuCapture::default();
                let panel = vec2(720.0, PANEL_HEIGHT) * scale;
                let size = vec2((panel.x + 40.0).ceil(), (panel.y + 36.0).ceil());
                let bounds = Rect::from_min_size(pos2(20.0, 18.0), panel);
                let mut rack = Rack {
                    zoom: scale,
                    pan: Vec2::ZERO,
                    ..Default::default()
                };
                rack.positions.insert("seq".into(), Pos2::ZERO);
                // The actual compiled target remains in the graph, beyond this
                // single-panel capture. There are no invented signal lights.
                rack.positions.insert("vca".into(), pos2(10_000.0, 0.0));
                rack.set_room_brightness(brightness);
                if panel_state == "live" {
                    let mut receipt = live_sequence_receipt(0, "Verse", "Fill", json!("Fill"), 6);
                    receipt["rackmcp_sequence"]["bpm"] = json!(124.0);
                    rack.accept_rackmcp_receipt("inspect_rackmcp_sequence", receipt);
                }
                let mut preview = ModulePreview::new(module);
                preview.renderer.set_room_brightness(brightness);
                assert!(preview.patch.nodes[0].rackmcp_sequence.is_none());
                let mut last = None;
                for _ in 0..3 {
                    let output = ctx.run(
                        egui::RawInput {
                            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                            ..Default::default()
                        },
                        |ctx| {
                            egui::CentralPanel::default().frame(egui::Frame::NONE).show(
                                ctx,
                                |ui| {
                                    if authored {
                                        let rendered = rack.render_modules(
                                            ui,
                                            &patch,
                                            &catalog,
                                            bounds,
                                            48_000,
                                            0,
                                            RenderMode::Interactive {
                                                drag_interactive: false,
                                                allow_manual_press: false,
                                            },
                                        );
                                        assert!(rendered.action.is_none());
                                    } else {
                                        preview.show(ui, bounds, &catalog);
                                    }
                                },
                            );
                        },
                    );
                    capture.observe(&output);
                    last = Some(output);
                }
                assert!(rack.sequence_states.is_empty() && rack.module_lights.is_empty());
                assert!(
                    preview.renderer.sequence_states.is_empty()
                        && preview.renderer.module_lights.is_empty()
                );
                capture.save(
                    &ctx,
                    last.unwrap(),
                    size,
                    destination.join(format!("rackmcp-{}-{view}-{illumination}.png", panel_state)),
                );
            }
        }
    }
}

fn live_sequence_receipt(
    revision: u64,
    active: &str,
    desired: &str,
    pending: Value,
    step: i64,
) -> Value {
    json!({"id":"seq","revision":revision,"available":true,"frame":48000,
        "rackmcp_sequence":{"running":true,"active_pattern_name":active,
            "selected_pattern_name":desired,"pending_pattern_name":pending,"step":step,
            "loop_count":3,"beat":2.5,"bar":0.625,"bpm":137.0,"swing":61.0,
            "external_clock":true,"pattern_count":2,"lane_count":0}})
}

fn live_sequence_patch() -> Patch {
    serde_json::from_value(
        json!({"version":2,"nodes":[{"id":"seq","kind":"rackmcp_sequencer",
        "rackmcp_sequence":{"data":{"selectedPattern":"A","patterns":[
            {"name":"A","length":8,"tracks":[{"steps":[{"step":0,"notes":[0]}]}]},
            {"name":"Bass","length":16,"tracks":[{"steps":[{"step":2,"notes":[7]}]}]}
        ]}}}]}),
    )
    .unwrap()
}

fn live_texts(output: &egui::FullOutput) -> Vec<(String, Rect)> {
    let mut result = Vec::new();
    for shape in &output.shapes {
        labels(&shape.shape, &mut result);
    }
    result
}

fn live_playheads(output: &egui::FullOutput) -> Vec<Rect> {
    output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Rect(cell) if cell.stroke.color == Color32::from_rgb(190, 225, 193) => {
                Some(cell.rect)
            }
            _ => None,
        })
        .collect()
}

#[test]
fn rackmcp_live_external_updates_show_actual_active_pending_clock_and_source_position() {
    let ctx = egui::Context::default();
    let patch = live_sequence_patch();
    let mut rack = Rack::default();
    rack.accept_rackmcp_receipt(
        "inspect_rackmcp_sequence",
        live_sequence_receipt(9, "A", "Bass", json!("Bass"), 3),
    );
    let (_, first) = frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 9);
    let texts = live_texts(&first);
    for expected in [
        "ACTIVE A · NEXT Bass",
        "SELECT Bass  ▾",
        "RUNNING · 137.0 BPM · EXT CLOCK",
        "BAR 0.62 · BEAT 2.50 · STEP 4 · LOOP 3",
    ] {
        assert!(
            texts.iter().any(|(text, _)| text == expected),
            "Missing {expected}"
        );
    }
    assert_eq!(live_playheads(&first).len(), 1);
    let (_, unchanged) =
        frame_revision_state(&ctx, &mut rack, &patch, 100.0, vec![], true, true, 9);
    assert_eq!(
        live_playheads(&first),
        live_playheads(&unchanged),
        "UI elapsed time does not advance source position"
    );
    rack.accept_rackmcp_receipt(
        "rackmcp_transport",
        json!({"revision":9,"receipt":{"accepted":true}}),
    );
    let (_, accepted) = frame_revision_state(&ctx, &mut rack, &patch, 101.0, vec![], true, true, 9);
    assert_eq!(
        live_playheads(&first),
        live_playheads(&accepted),
        "Queue acceptance does not invent callback application"
    );
    rack.accept_rackmcp_receipt(
        "inspect_rackmcp_sequence",
        live_sequence_receipt(9, "Bass", "Bass", Value::Null, 5),
    );
    let (_, advanced) = frame_revision_state(&ctx, &mut rack, &patch, 102.0, vec![], true, true, 9);
    assert!(live_texts(&advanced)
        .iter()
        .any(|(text, _)| text == "ACTIVE Bass · NEXT —"));
    assert_ne!(live_playheads(&first), live_playheads(&advanced));
    assert!(rack.sequence_states.is_empty() && rack.module_lights.is_empty());
}

#[test]
fn rackmcp_stale_unavailable_and_revision_changed_status_clear_live_position_and_controls() {
    let patch = live_sequence_patch();
    for cause in ["expired", "unavailable", "revision", "error"] {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        rack.accept_rackmcp_receipt(
            "inspect_rackmcp_sequence",
            live_sequence_receipt(9, "A", "Bass", json!("Bass"), 3),
        );
        match cause {
            "expired" => rack.rackmcp_ui.statuses.get_mut("seq").unwrap().1 -= std::time::Duration::from_secs(1),
            "unavailable" => rack.accept_rackmcp_receipt("inspect_rackmcp_sequence", json!({"id":"seq","revision":9,"available":false,"reason":"document execution is suspended"})),
            "error" => rack.rackmcp_command_error("inspect_rackmcp_sequence", &json!({"id":"seq"}), "stale revision".into()),
            _ => (),
        }
        let revision = if cause == "revision" { 10 } else { 9 };
        let (_, output) =
            frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, revision);
        assert!(live_playheads(&output).is_empty(), "{cause}");
        assert!(
            live_texts(&output)
                .iter()
                .any(|(text, _)| text == "SAVED NOTES · LIVE POSITION UNAVAILABLE"),
            "{cause}"
        );
        let target = pos2(24.0, 12.0) + atlas_rackmcp_native_control("pattern").center().to_vec2();
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.0,
            vec![egui::Event::PointerMoved(target), button(target, true)],
            true,
            true,
            revision,
        );
        let (action, _) = frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.1,
            vec![button(target, false)],
            true,
            true,
            revision,
        );
        assert!(
            action.is_none() && rack.rackmcp_ui.menu.is_none(),
            "{cause}: unavailable control accepted input"
        );
    }
}

#[test]
fn rackmcp_pattern_dropdown_emits_real_revision_fenced_wrap_and_now_transport() {
    let patch = live_sequence_patch();
    for now in [false, true] {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        rack.rackmcp_ui.select_now.insert("seq".into(), now);
        rack.accept_rackmcp_receipt(
            "inspect_rackmcp_sequence",
            live_sequence_receipt(9, "A", "A", Value::Null, 3),
        );
        frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 9);
        let target = pos2(24.0, 12.0) + atlas_rackmcp_native_control("pattern").center().to_vec2();
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.0,
            vec![egui::Event::PointerMoved(target), button(target, true)],
            true,
            true,
            9,
        );
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.1,
            vec![button(target, false)],
            true,
            true,
            9,
        );
        let (_, menu) = frame_revision_state(&ctx, &mut rack, &patch, 1.2, vec![], true, true, 9);
        let choice = live_texts(&menu)
            .iter()
            .find(|(text, _)| text == "Bass")
            .expect("anchored pattern choice")
            .1
            .center();
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            2.0,
            vec![egui::Event::PointerMoved(choice), button(choice, true)],
            true,
            true,
            9,
        );
        let (action, _) = frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            2.1,
            vec![button(choice, false)],
            true,
            true,
            9,
        );
        assert!(
            matches!(action, Some(RackAction::Command("rackmcp_transport", ref args)) if args["id"] == "seq" && args["action"] == "select" && args["pattern"] == "Bass" && args["quantize"] == if now { "now" } else { "wrap" } && args["expected_revision"] == 9)
        );
    }
}

#[test]
fn rackmcp_polling_is_throttled_and_discards_removed_or_previous_revision_observations() {
    let patch = live_sequence_patch();
    let mut rack = Rack::default();
    assert_eq!(
        rack.rackmcp_poll_commands(&patch, 9),
        vec![(
            "inspect_rackmcp_sequence",
            json!({"id":"seq","expected_revision":9})
        )]
    );
    assert!(rack.rackmcp_poll_commands(&patch, 9).is_empty());
    rack.accept_rackmcp_receipt(
        "inspect_rackmcp_sequence",
        live_sequence_receipt(9, "A", "A", Value::Null, 3),
    );
    rack.rackmcp_poll_commands(&patch, 10);
    assert!(rack.rackmcp_ui.statuses.is_empty());
    rack.accept_rackmcp_receipt(
        "inspect_rackmcp_sequence",
        live_sequence_receipt(10, "A", "A", Value::Null, 3),
    );
    rack.rackmcp_poll_commands(&Patch::default(), 10);
    assert!(rack.rackmcp_ui.statuses.is_empty());
}

#[test]
fn rackmcp_seek_dropdown_emits_source_bars_and_closes_on_revision_change() {
    let ctx = egui::Context::default();
    let patch = live_sequence_patch();
    let mut rack = Rack::default();
    rack.rackmcp_ui.seek_bars.insert("seq".into(), 2.5);
    rack.accept_rackmcp_receipt(
        "inspect_rackmcp_sequence",
        live_sequence_receipt(9, "A", "A", Value::Null, 3),
    );
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 9);
    let target = pos2(24.0, 12.0) + atlas_rackmcp_native_control("seek").center().to_vec2();
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.0,
        vec![egui::Event::PointerMoved(target), button(target, true)],
        true,
        true,
        9,
    );
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.1,
        vec![button(target, false)],
        true,
        true,
        9,
    );
    let (_, menu) = frame_revision_state(&ctx, &mut rack, &patch, 1.2, vec![], true, true, 9);
    let choice = live_texts(&menu)
        .iter()
        .find(|(text, _)| text == "Seek")
        .expect("anchored seek action")
        .1
        .center();
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        2.0,
        vec![egui::Event::PointerMoved(choice), button(choice, true)],
        true,
        true,
        9,
    );
    let (action, _) = frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        2.1,
        vec![button(choice, false)],
        true,
        true,
        9,
    );
    assert!(
        matches!(action, Some(RackAction::Command("rackmcp_transport", ref args)) if args["id"] == "seq" && args["action"] == "seek" && args["bar"] == 2.5 && args["expected_revision"] == 9)
    );
    rack.rackmcp_ui.menu = Some(("seq".into(), 9, "seek"));
    rack.rackmcp_poll_commands(&patch, 10);
    assert!(
        rack.rackmcp_ui.menu.is_none(),
        "An open editor cannot apply to a later document revision"
    );
}
