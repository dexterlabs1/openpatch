use super::*;

#[test]
fn multi_lfo_configured_cycles_agree_with_actual_engine_in_both_ranges_and_polarities() {
    for width in [0.01_f32, 0.37, 0.99] {
        for unipolar in [0.0, 1.0] {
            for invert in [0.0, 1.0] {
                let patch:Patch=serde_json::from_value(json!({"version":2,"sample_rate":48000,
                    "nodes":[{"id":"lfo","kind":"multi_lfo","params":{"frequency":4.0,"pulse_width":width,"unipolar":unipolar,"invert":invert}}],
                    "output":"lfo","output_port":"sine"})).unwrap();
                let mut engine = patch.compile().unwrap();
                let taps = ["sine", "triangle", "saw", "square"]
                    .map(|port| Some(engine.resolve_tap("lfo", port, 0).unwrap()));
                let mut audio = vec![[0.0; 2]; 12000];
                let mut samples = vec![[openpatch_core::TapSample::default(); 4]; audio.len()];
                engine.process_audio_block_with_taps(&mut audio, &taps, &mut samples);
                for (index, frame) in samples.iter().enumerate() {
                    let phase = index as f64 / 12000.0;
                    // Accumulated phase can straddle a discontinuity by one
                    // floating-point ulp; compare the two open spans instead.
                    if (phase - f64::from(width)).abs() < 1e-8 {
                        continue;
                    }
                    for (output, sample) in frame.iter().enumerate() {
                        assert!(sample.active);
                        let expected = atlas_multi_nominal_sample(&patch.nodes[0], output, phase);
                        assert!((f64::from(sample.value)-expected).abs()<0.00003,
                            "width={width} offset={unipolar} invert={invert} phase={phase} output={output}");
                    }
                }
            }
        }
    }
}

#[test]
fn wavetable_display_fits_quiet_source_samples_but_keeps_zero_missing_and_telemetry_honest() {
    let mut patch: Patch =
        serde_json::from_value(json!({"version":2,"nodes":[{"id":"wt","kind":"wavetable_lfo"}]}))
            .unwrap();
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    rack.set_room_brightness(0.0);
    let trace = |out: &egui::FullOutput| {
        out.shapes.iter().find_map(|s| match &s.shape {
            egui::Shape::Path(p) if p.points.len() == 161 => Some(p.points.clone()),
            _ => None,
        })
    };
    let texts = |out: &egui::FullOutput| {
        out.shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) => Some(t.galley.text().to_string()),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    for peak in [0.00001_f32, 0.0] {
        let asset = Arc::new(openpatch_core::WaveAsset::new(&[0.0, peak, 0.0, -peak]).unwrap());
        patch.nodes[0].content = Some(openpatch_core::ModuleContent::Wavetable {
            asset_id: asset.id().into(),
            wave_points: 4,
            name: "Quiet source".into(),
        });
        patch.assets.insert(asset.id().into(), asset);
        let out = frame_output(&ctx, &mut rack, &patch, 0.0, vec![]).1;
        let points = trace(&out).expect("actual source geometry");
        let height = points.iter().map(|p| p.y).fold(f32::NEG_INFINITY, f32::max)
            - points.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
        if peak > 0.0 {
            assert!(height > 25.0, "quiet table stays visible");
        } else {
            assert!(height < 0.001, "zero stays flat");
        }
        let labels = texts(&out);
        assert!(labels.iter().any(|s| s == "SET POS"));
        assert!(labels.iter().any(|s| s
            == if peak > 0.0 {
                "AUTO ±1.0e-5"
            } else {
                "AUTO ±0.00"
            }));
        for index in 0..5 {
            let center = pos2(24.0 + 54.0 + index as f32 * 63.0, 12.0 + 136.0);
            assert!(out.shapes.iter().any(|s|matches!(&s.shape,egui::Shape::Circle(c) if c.center.distance(center)<0.01&&c.fill.r()==0&&c.fill.g()==0&&c.fill.b()==0)),"missing telemetry cannot light a lens");
        }
    }
    rack.module_lights.insert(
        "wt".into(),
        ModuleLightState {
            waveform_position: Some(0.75),
            ..Default::default()
        },
    );
    let out = frame_output(&ctx, &mut rack, &patch, 0.1, vec![]).1;
    assert!(texts(&out).iter().any(|s| s == "LIVE POS"));
    patch.assets.clear();
    let out = frame_output(&ctx, &mut rack, &patch, 0.2, vec![]).1;
    assert!(trace(&out).is_none());
    assert!(texts(&out).iter().any(|s| s == "Table unavailable"));
}

#[test]
fn oscillator_family_mcp_cues_cover_all_rotaries_and_saved_modes() {
    use crate::construction_motion::MotionMode;
    let catalog = openpatch_core::catalog();
    for kind in ["multi_vco", "multi_lfo", "wavetable_vco", "wavetable_lfo"] {
        let module = catalog["modules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["kind"] == kind)
            .unwrap();
        let mut before: Patch =
            serde_json::from_value(json!({"version":2,"nodes":[{"id":"osc","kind":kind}]}))
                .unwrap();
        let mut after = before.clone();
        let names: Vec<_> = ordered_parameters(&before.nodes[0], Some(module))
            .iter()
            .map(|(n, _)| n.to_string())
            .collect();
        let changes:Vec<_>=names.iter().map(|name| {
            let spec=&module["params"][name];let min=spec["min"].as_f64().unwrap() as f32;let max=spec["max"].as_f64().unwrap() as f32;
            let (old,new)=if spec["step"].is_number() {(min,max)} else {(min+(max-min)*0.25,min+(max-min)*0.75)};
            before.nodes[0].params.insert(name.clone(),old);after.nodes[0].params.insert(name.clone(),new);
            json!({"id":"osc","kind":kind,"parameter":name,"old_effective":old,"new_effective":new})
        }).collect();
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        frame_revision_state(&ctx, &mut rack, &before, 0.0, vec![], true, true, 9);
        let now = std::time::Instant::now();
        let receipt = json!({"plan_id":26,"completed_steps":1,"last_revision":10,"current_revision":10,"last_commit":{"age_ms":0,"invalidate_existing_cables":false,"plan_id":26,"step_index":0,"before_revision":9,"revision":10,"label":"Shape oscillator","new_modules":[],"new_cables":[],"parameter_changes":changes}});
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
        assert!(
            frame_revision_state(&ctx, &mut rack, &after, 0.1, vec![], true, true, 10)
                .0
                .is_none()
        );
        assert_eq!(rack.construction_anchors.len(), 6, "{kind}");
        for name in names {
            let anchor = &rack.construction_anchors[&("osc".into(), name.clone())];
            assert_ne!(anchor.old_text, anchor.new_text, "{kind}.{name}");
            assert_eq!(
                anchor.marker.is_some(),
                oscillator_voice_rotary_parameter(kind, &name)
            );
        }
    }
}

#[test]
fn multi_oscillator_mode_buttons_commit_with_revision_guard_and_undo() {
    for (kind, name, column) in [
        ("multi_vco", "linear_fm", 0),
        ("multi_vco", "soft_sync", 1),
        ("multi_lfo", "unipolar", 0),
        ("multi_lfo", "invert", 1),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(dir.path()).unwrap();
        session
            .replace_patch(
                serde_json::from_value(json!({"version":2,"nodes":[{"id":"osc","kind":kind}]}))
                    .unwrap(),
                None,
            )
            .unwrap();
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
        let at = pos2(24.0 + 82.5 + column as f32 * 135.0, 12.0 + 291.5);
        frame_revision_state(
            &ctx,
            &mut rack,
            session.patch(),
            1.0,
            vec![egui::Event::PointerMoved(at), button(at, true)],
            true,
            true,
            session.revision(),
        );
        let action = frame_revision_state(
            &ctx,
            &mut rack,
            session.patch(),
            1.1,
            vec![button(at, false)],
            true,
            true,
            session.revision(),
        )
        .0;
        let Some(RackAction::Command("set_parameter", args)) = action else {
            panic!("{kind}.{name}")
        };
        assert_eq!(args["parameter"], name);
        assert_eq!(args["expected_revision"], session.revision());
        session.execute("set_parameter", args).unwrap();
        assert_eq!(session.patch().nodes[0].params[name], 1.0);
        session.execute("undo", json!({})).unwrap();
        assert_eq!(
            session.patch().nodes[0]
                .params
                .get(name)
                .copied()
                .unwrap_or(0.0),
            0.0
        );
        session.execute("redo", json!({})).unwrap();
        assert_eq!(session.patch().nodes[0].params[name], 1.0);
    }
}
