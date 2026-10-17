use super::*;

#[test]
fn svf_configured_core_guide_agrees_with_actual_engine_at_zero_drive() {
    // Measure the fundamental from actual source/output taps after settling.
    // This bounded passband check includes the real AA stage; the guide itself
    // describes only the TPT core, not AA rolloff or nonlinear drive/latency.
    for resonance in [0.0, 0.6, 1.0] {
        for frequency in [100.0, 400.0, 800.0] {
            let patch: Patch = serde_json::from_value(json!({"version":2,"sample_rate":48000,
                "nodes":[{"id":"source","kind":"oscillator","params":{"frequency":frequency}},
                    {"id":"filter","kind":"svf_filter","params":{"cutoff":400.0,"resonance":resonance,"drive":0.0}}],
                "cables":[{"from":"source","output":"out","to":"filter","input":"signal"}],
                "output":"filter","output_port":"low"})).unwrap();
            let mut engine = patch.compile().unwrap();
            let taps = [
                Some(engine.resolve_tap("source", "out", 0).unwrap()),
                Some(engine.resolve_tap("filter", "low", 0).unwrap()),
                Some(engine.resolve_tap("filter", "band", 0).unwrap()),
                Some(engine.resolve_tap("filter", "high", 0).unwrap()),
            ];
            let mut audio = vec![[0.0; 2]; 12_000];
            let mut samples = vec![[openpatch_core::TapSample::default(); 4]; 12_000];
            engine.process_audio_block_with_taps(&mut audio, &taps, &mut samples);
            engine.process_audio_block_with_taps(&mut audio, &taps, &mut samples);
            let magnitude = |port: usize| {
                let mut re = 0.0;
                let mut im = 0.0;
                for (index, sample) in samples.iter().enumerate() {
                    assert!(sample[port].active);
                    let phase = std::f64::consts::TAU * frequency * index as f64 / 48000.0;
                    let (sin, cos) = phase.sin_cos();
                    let value = f64::from(sample[port].value);
                    re += value * cos;
                    im += value * sin;
                }
                re.hypot(im)
            };
            let expected = atlas_svf_core_magnitudes(&patch.nodes[1], 48000, frequency);
            for (port, expected) in expected.into_iter().enumerate() {
                let actual = magnitude(port + 1) / magnitude(0);
                let error_db = 20.0 * (actual / expected).log10();
                assert!(
                    error_db.abs() < 0.02,
                    "res={resonance} f={frequency} port={port}: {error_db} dB"
                );
            }
        }
    }
}

#[test]
fn modal_and_svf_mcp_cues_cover_every_rotary_with_human_readouts() {
    use crate::construction_motion::MotionMode;
    let catalog = openpatch_core::catalog();
    for (kind, count) in [("svf_filter", 4), ("modal_resonator", 8)] {
        let module = catalog["modules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["kind"] == kind)
            .unwrap();
        let mut before: Patch =
            serde_json::from_value(json!({"version":2,"nodes":[{"id":"target","kind":kind}]}))
                .unwrap();
        let mut after = before.clone();
        let names: Vec<_> = ordered_parameters(&before.nodes[0], Some(module))
            .iter()
            .map(|(name, _)| name.to_string())
            .collect();
        let changes:Vec<_>=names.iter().map(|name| {
            let spec=&module["params"][name];
            let min=spec["min"].as_f64().unwrap() as f32;
            let max=spec["max"].as_f64().unwrap() as f32;
            let old=min+(max-min)*0.25;let new=min+(max-min)*0.75;
            before.nodes[0].params.insert(name.clone(),old);
            after.nodes[0].params.insert(name.clone(),new);
            json!({"id":"target","kind":kind,"parameter":name,"old_effective":old,"new_effective":new})
        }).collect();
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        frame_revision_state(&ctx, &mut rack, &before, 0.0, vec![], true, true, 9);
        let now = std::time::Instant::now();
        let receipt = json!({"plan_id":25,"completed_steps":1,"last_revision":10,"current_revision":10,"last_commit":{"age_ms":0,"invalidate_existing_cables":false,"plan_id":25,"step_index":0,"before_revision":9,"revision":10,"label":"Shape filter and resonator","new_modules":[],"new_cables":[],"parameter_changes":changes}});
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
        assert_eq!(rack.construction_anchors.len(), count);
        for (name, _) in ordered_parameters(&after.nodes[0], Some(module)) {
            let anchor = &rack.construction_anchors[&("target".into(), name.into())];
            assert!(matches!(
                anchor.marker,
                Some(crate::construction_motion::Marker::Rotary { .. })
            ));
            assert_ne!(anchor.old_text, anchor.new_text);
            if kind == "svf_filter" && name == "cutoff_cv_amount" {
                assert_eq!(
                    (&*anchor.old_text, &*anchor.new_text),
                    ("-2.00 oct/V", "+2.00 oct/V")
                );
            }
            if kind == "svf_filter" && matches!(name, "resonance" | "drive") {
                assert_eq!((&*anchor.old_text, &*anchor.new_text), ("25%", "75%"));
            }
        }
    }
}

#[test]
fn svf_response_display_updates_with_settings_and_remains_configured_in_dark_room() {
    let mut patch: Patch =
        serde_json::from_value(json!({"version":2,"nodes":[{"id":"f","kind":"svf_filter"}]}))
            .unwrap();
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    rack.set_room_brightness(0.0);
    let text = |output: &egui::FullOutput| {
        output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) => Some(t.galley.text().to_string()),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let before = frame_output(&ctx, &mut rack, &patch, 0.0, vec![]).1;
    let labels = text(&before);
    for expected in [
        "TPT CORE · dB",
        "CONFIG · ZERO CV",
        "LOW",
        "BAND",
        "HIGH",
        "Q 0.62 · LOG Hz",
    ] {
        assert!(labels.iter().any(|s| s == expected), "{expected}");
    }
    patch.nodes[0].params.insert("resonance".into(), 1.0);
    let after = frame_output(&ctx, &mut rack, &patch, 0.1, vec![]).1;
    assert!(text(&after).iter().any(|s| s == "Q 20.00 · LOG Hz"));
    let curves = |output: &egui::FullOutput| {
        output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Path(path) if path.points.len() == 81 => Some(format!("{path:?}")),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(curves(&before).len(), 3);
    assert_ne!(curves(&before), curves(&after));
    assert!(rack.module_lights.is_empty());
    assert_eq!(
        atlas_svf_value("cutoff_cv_amount", -2.0, &json!({})),
        "-2.00 oct/V"
    );
    for rate in [8000, 48000] {
        for f in [0.0, 1.0, 400.0, 20000.0] {
            assert!(atlas_svf_core_magnitudes(&patch.nodes[0], rate, f)
                .iter()
                .all(|m| m.is_finite() && *m >= 0.0));
        }
    }
}
