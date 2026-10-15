// Included inside audio::tests. These exercise the real callback with offline
// graphs and queues; no device, stream, MIDI connection or GUI is opened.
fn stock_delay_callback_patch(value: f32, dynamic: bool) -> Patch {
    let source = if dynamic {
        serde_json::json!({"id":"audio","kind":"oscillator","params":{"frequency":137,"amplitude":0.7}})
    } else {
        serde_json::json!({"id":"audio","kind":"constant","params":{"value":value}})
    };
    let mut patch: Patch = serde_json::from_value(serde_json::json!({
        "version":2,"sample_rate":48000,
        "nodes":[source,
            {"id":"clock","kind":"gate","params":{"frequency":23,"duty":0.2}},
            {"id":"delay","kind":"stock_delay","params":{"time":0,"feedback":0.73,"tone":0.61,"mix":if dynamic {1.0} else {0.0}}},
            {"id":"tempo","kind":"constant","params":{"value":16}},
            {"id":"tracker","kind":"seq3","params":{"tempo":960,"tempo_amount":1,"running":1,"steps":8}}
        ],
        "cables":[
            {"id":"audio_delay","from":"audio","to":"delay","input":"signal"},
            {"id":"clock_delay","from":"clock","to":"delay","input":"clock"},
            {"id":"tempo_tracker","from":"tempo","to":"tracker","input":"tempo"}
        ],
        "output":"delay","output_port":"mix"
    })).unwrap();
    patch.assign_missing_cable_ids().unwrap();
    patch
}

fn stock_delay_fixed_gain(callback: &mut Callback, value: f32) {
    callback
        .shared
        .volume
        .store(value.to_bits(), Ordering::Relaxed);
    callback.gain.value = value;
    callback.gain.coefficient = 0.0;
}

fn stock_delay_clock_light(graph: &PreparedGraph) -> openpatch_core::ModuleLightState {
    graph
        .engine
        .module_lights(graph.engine.resolve_module_lights("delay").unwrap())
        .unwrap()
}

fn stock_delay_tracker(graph: &PreparedGraph) -> openpatch_core::ModuleTelemetry {
    graph
        .engine
        .module_telemetry(graph.engine.resolve_module_state("tracker").unwrap())
        .unwrap()
}

#[test]
fn stock_delay_callback_moves_runtime_and_freezes_entire_donor_while_tail_continues() {
    let patch = stock_delay_callback_patch(0.0, true);
    let (mut callback, mut updates, _disposal) = sequence_callback(&patch);
    let mut warm = [0.0_f32; 4099];
    without_heap_activity(|| callback.render(&mut warm, 1));
    let cached = callback.last_active_raw;
    let clock_before = stock_delay_clock_light(&callback.graphs.active);
    assert!(clock_before.brightness[0][0] > 0.0);

    let mut edited = patch.clone();
    edited.nodes.push(
        serde_json::from_value(serde_json::json!({
            "id":"unrelated","kind":"constant","params":{"value":7}
        }))
        .unwrap(),
    );
    edited.nodes.reverse();
    let mut reference = edited.compile().unwrap();
    reference.adopt_state_from(&callback.graphs.active.engine);
    let mut raw = [[0.0_f32; 2]; 257];
    reference.process_audio_block(&mut raw);
    assert!(raw.iter().any(|sample| sample[0].abs() > 0.001));
    let reference_light = reference
        .module_lights(reference.resolve_module_lights("delay").unwrap())
        .unwrap();
    assert!(updates.push(sequence_graph(&callback, &edited, 2)).is_ok());

    // Adopt without processing a frame. Ownership transfer leaves the donor's
    // fresh receiving runtime behind, so its clock light is now zero.
    let mut empty = [0.0_f32; 0];
    without_heap_activity(|| callback.render(&mut empty, 1));
    assert_eq!(callback.frozen_previous, Some((cached, false)));
    assert_eq!(
        stock_delay_clock_light(&callback.graphs.active),
        clock_before
    );
    let donor = callback.graphs.previous.as_ref().unwrap();
    let donor_clock = stock_delay_clock_light(donor);
    let donor_tracker = stock_delay_tracker(donor);
    assert_eq!(donor_clock.brightness[0][0], 0.0);

    let mut actual = [0.0_f32; 257];
    without_heap_activity(|| callback.render(&mut actual[..120], 1));
    assert_eq!(
        stock_delay_clock_light(callback.graphs.previous.as_ref().unwrap()),
        donor_clock
    );
    assert_eq!(
        stock_delay_tracker(callback.graphs.previous.as_ref().unwrap()),
        donor_tracker
    );
    without_heap_activity(|| callback.render(&mut actual[120..239], 1));
    assert_eq!(
        stock_delay_clock_light(callback.graphs.previous.as_ref().unwrap()),
        donor_clock
    );
    assert_eq!(
        stock_delay_tracker(callback.graphs.previous.as_ref().unwrap()),
        donor_tracker
    );
    without_heap_activity(|| callback.render(&mut actual[239..], 1));
    assert!(callback.graphs.previous.is_none());
    assert_eq!(callback.frozen_previous, None);
    assert_eq!(
        stock_delay_clock_light(&callback.graphs.active),
        reference_light
    );
    for (index, (actual, raw)) in actual.iter().zip(raw).enumerate() {
        let mix = (index as f32 / callback.fade_length as f32).min(1.0);
        let expected = (cached[0] * (1.0 - mix) + raw[0] * mix).clamp(-1.0, 1.0);
        assert!(
            (actual - expected).abs() < 1e-6,
            "frame {index}: {actual} vs {expected}"
        );
    }
}

#[test]
fn stock_delay_callback_caches_raw_before_gain_and_clipping_and_applies_gain_once() {
    let patch = stock_delay_callback_patch(6.0, false);
    let (mut callback, mut updates, _disposal) = sequence_callback(&patch);
    stock_delay_fixed_gain(&mut callback, 0.25);
    let mut warm = [0.0_f32; 129];
    without_heap_activity(|| callback.render(&mut warm, 1));
    assert_eq!(warm[128], 1.0);
    assert_eq!(callback.last_active_raw, [6.0; 2]);
    let mut edited = patch.clone();
    edited.nodes[0].params.insert("value".into(), -3.0);
    let mut reference = edited.compile().unwrap();
    reference.adopt_state_from(&callback.graphs.active.engine);
    let mut raw = [[0.0_f32; 2]; 257];
    reference.process_audio_block(&mut raw);
    assert!(updates.push(sequence_graph(&callback, &edited, 2)).is_ok());
    let mut actual = [0.0_f32; 257];
    without_heap_activity(|| callback.render(&mut actual, 1));
    for (index, (actual, current)) in actual.iter().zip(raw).enumerate() {
        let mix = (index as f32 / callback.fade_length as f32).min(1.0);
        let expected = ((6.0 * (1.0 - mix) + current[0] * mix) * 0.25).clamp(-1.0, 1.0);
        assert!(
            (actual - expected).abs() < 1e-6,
            "frame {index}: {actual} vs {expected}"
        );
    }
    assert_eq!(actual[0], 1.0);
    assert_eq!(actual[256], -0.75);
    assert_eq!(callback.gain.value, 0.25);
    assert_eq!(callback.last_active_raw, [-3.0; 2]);
}

#[test]
fn stock_delay_callback_frozen_mono_maps_into_stereo_one_and_six_channel_devices() {
    for channels in [1, 2, 6] {
        let patch = stock_delay_callback_patch(6.0, false);
        let (mut callback, mut updates, _disposal) = sequence_callback(&patch);
        stock_delay_fixed_gain(&mut callback, 0.25);
        let mut warm = [0.0_f32; 129];
        without_heap_activity(|| callback.render(&mut warm, 1));
        let mut stereo = patch.clone();
        stereo.nodes.push(
            serde_json::from_value(serde_json::json!({
                "id":"right","kind":"constant","params":{"value":-2}
            }))
            .unwrap(),
        );
        stereo.nodes.push(
            serde_json::from_value(serde_json::json!({
                "id":"stereo","kind":"audio_stereo"
            }))
            .unwrap(),
        );
        stereo.cables.push(
            serde_json::from_value(serde_json::json!({
                "id":"delay_left","from":"delay","output":"mix","to":"stereo","input":"left"
            }))
            .unwrap(),
        );
        stereo.cables.push(
            serde_json::from_value(serde_json::json!({
                "id":"right_stereo","from":"right","to":"stereo","input":"right"
            }))
            .unwrap(),
        );
        stereo.output = Some("stereo".into());
        stereo.output_port = "out".into();
        let mut reference = stereo.compile().unwrap();
        reference.adopt_state_from(&callback.graphs.active.engine);
        let mut raw = [[0.0_f32; 2]; 257];
        reference.process_audio_block(&mut raw);
        assert!(updates.push(sequence_graph(&callback, &stereo, 2)).is_ok());
        let mut actual = vec![0.0_f32; 257 * channels];
        without_heap_activity(|| callback.render(&mut actual, channels));
        for (index, (frame, current)) in actual.chunks_exact(channels).zip(raw).enumerate() {
            let mix = (index as f32 / callback.fade_length as f32).min(1.0);
            for (channel, actual) in frame.iter().enumerate() {
                let new = if channels == 1 {
                    current[0] * 0.5 + current[1] * 0.5
                } else if channel < 2 {
                    current[channel]
                } else {
                    0.0
                };
                let expected = ((6.0 * (1.0 - mix) + new * mix) * 0.25).clamp(-1.0, 1.0);
                assert!(
                    (actual - expected).abs() < 1e-6,
                    "{channels} channels, frame {index}, lane {channel}: {actual} vs {expected}"
                );
            }
        }
        if channels == 6 {
            assert!(actual[..6].iter().all(|sample| *sample == 1.0));
            assert_eq!(&actual[actual.len() - 4..], &[0.0; 4]);
        }
        assert!(callback.last_active_stereo);
    }
}

#[test]
fn stock_delay_callback_adoption_without_complete_frames_keeps_cache_clock_and_fade_position() {
    for case in 0..3 {
        let patch = stock_delay_callback_patch(6.0, false);
        let (mut callback, mut updates, _disposal) = sequence_callback(&patch);
        stock_delay_fixed_gain(&mut callback, 0.25);
        let mut warm = [0.0_f32; 129];
        without_heap_activity(|| callback.render(&mut warm, 1));
        let frame_clock = callback.frame_clock;
        let cached = callback.last_active_raw;
        assert!(updates.push(sequence_graph(&callback, &patch, 2)).is_ok());
        let mut incomplete = [99.0_f32; 5];
        let mut empty = [0.0_f32; 0];
        match case {
            0 => without_heap_activity(|| callback.render(&mut incomplete, 0)),
            1 => without_heap_activity(|| callback.render(&mut empty, 2)),
            _ => without_heap_activity(|| callback.render(&mut incomplete, 6)),
        }
        assert_eq!(callback.graphs.active.revision, 2);
        assert_eq!(callback.graphs.previous.as_ref().unwrap().revision, 1);
        assert_eq!(callback.frozen_previous, Some((cached, false)));
        assert_eq!(callback.last_active_raw, cached);
        assert_eq!(callback.frame_clock, frame_clock);
        assert_eq!(callback.fade_position, 0);
        assert_eq!(callback.gain.value, 0.25);
        assert_eq!(incomplete, if case == 2 { [0.0; 5] } else { [99.0; 5] });
        let clock = stock_delay_clock_light(callback.graphs.previous.as_ref().unwrap());
        let tracker = stock_delay_tracker(callback.graphs.previous.as_ref().unwrap());
        assert!(updates.push(sequence_graph(&callback, &patch, 3)).is_ok());
        without_heap_activity(|| callback.render(&mut empty, 1));
        assert_eq!(callback.graphs.active.revision, 2);
        assert_eq!(callback.fade_position, 0);
        assert_eq!(
            stock_delay_clock_light(callback.graphs.previous.as_ref().unwrap()),
            clock
        );
        assert_eq!(
            stock_delay_tracker(callback.graphs.previous.as_ref().unwrap()),
            tracker
        );
        let mut first = [0.0_f32; 6];
        without_heap_activity(|| callback.render(&mut first, 6));
        assert_eq!(first, [1.0; 6]);
        assert_eq!(callback.frame_clock, frame_clock + 1);
        assert_eq!(callback.fade_position, 1);
    }
}

#[test]
fn stock_delay_callback_queued_transitions_and_full_retirement_queue_keep_all_donors_off_callback_drop_path(
) {
    let patch = stock_delay_callback_patch(0.0, true);
    let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
    let mut output = [0.0_f32; 241];
    without_heap_activity(|| callback.render(&mut output, 1));
    let second = sequence_graph(&callback, &patch, 2);
    let third = sequence_graph(&callback, &patch, 3);
    let fourth = sequence_graph(&callback, &patch, 4);
    let fifth = sequence_graph(&callback, &patch, 5);
    assert!(updates.push(second).is_ok());
    assert!(updates.push(third).is_ok());
    without_heap_activity(|| callback.render(&mut output, 1));
    assert_eq!(callback.graphs.active.revision, 2);
    without_heap_activity(|| callback.render(&mut output, 1));
    assert_eq!(callback.graphs.active.revision, 3);
    assert!(updates.push(fourth).is_ok());
    assert!(updates.push(fifth).is_ok());
    without_heap_activity(|| callback.render(&mut output, 1));
    assert_eq!(callback.graphs.active.revision, 4);
    let blocked = callback.graphs.waiting_retirement.as_ref().unwrap();
    assert_eq!(blocked.revision, 3);
    let blocked_clock = stock_delay_clock_light(blocked);
    let blocked_tracker = stock_delay_tracker(blocked);
    for _ in 0..4 {
        without_heap_activity(|| callback.render(&mut output, 1));
        assert_eq!(callback.graphs.active.revision, 4);
        let blocked = callback.graphs.waiting_retirement.as_ref().unwrap();
        assert_eq!(stock_delay_clock_light(blocked), blocked_clock);
        assert_eq!(stock_delay_tracker(blocked), blocked_tracker);
        assert!(callback.graphs.previous.is_none());
        assert_eq!(callback.frozen_previous, None);
    }
    // Retain every popped graph locally. Disposal and destruction happen after
    // each heap guard, exactly as they do on the control thread.
    let first_retired = disposal.pop().unwrap();
    assert_eq!(first_retired.revision, 1);
    let cached_fourth = callback.last_active_raw;
    let mut empty = [0.0_f32; 0];
    without_heap_activity(|| callback.render(&mut empty, 1));
    assert_eq!(callback.graphs.active.revision, 5);
    assert_eq!(callback.fade_position, 0);
    assert_eq!(callback.frozen_previous, Some((cached_fourth, false)));
    assert_eq!(callback.graphs.previous.as_ref().unwrap().revision, 4);
    without_heap_activity(|| callback.render(&mut output, 1));
    assert_eq!(callback.graphs.active.revision, 5);
    assert_eq!(
        callback
            .graphs
            .waiting_retirement
            .as_ref()
            .unwrap()
            .revision,
        4
    );
    let second_retired = disposal.pop().unwrap();
    let third_retired = disposal.pop().unwrap();
    assert_eq!((second_retired.revision, third_retired.revision), (2, 3));
    without_heap_activity(|| callback.render(&mut empty, 1));
    assert!(callback.graphs.waiting_retirement.is_none());
    let fourth_retired = disposal.pop().unwrap();
    assert_eq!(fourth_retired.revision, 4);
    assert!(disposal.pop().is_err());
    assert!(output.iter().all(|sample| sample.is_finite()));
}

#[test]
fn stock_delay_callback_non_delay_edits_still_render_both_evolving_graphs() {
    let mut patch = stock_delay_callback_patch(0.0, true);
    patch
        .nodes
        .retain(|node| node.kind != "stock_delay" && node.id != "clock");
    patch.cables.retain(|cable| cable.to == "tracker");
    patch.output = Some("audio".into());
    patch.output_port = "out".into();
    let (mut callback, mut updates, _disposal) = sequence_callback(&patch);
    let mut warm = [0.0_f32; 129];
    without_heap_activity(|| callback.render(&mut warm, 1));
    let mut edited = patch.clone();
    edited
        .nodes
        .iter_mut()
        .find(|node| node.id == "audio")
        .unwrap()
        .params
        .insert("frequency".into(), 251.0);
    let mut old_reference = patch.compile().unwrap();
    old_reference.adopt_state_from(&callback.graphs.active.engine);
    let mut new_reference = edited.compile().unwrap();
    new_reference.adopt_state_from(&callback.graphs.active.engine);
    let mut old = [0.0_f32; 257];
    let mut new = [0.0_f32; 257];
    old_reference.process_block(&mut old);
    new_reference.process_block(&mut new);
    assert!(updates.push(sequence_graph(&callback, &edited, 2)).is_ok());
    let mut empty = [0.0_f32; 0];
    without_heap_activity(|| callback.render(&mut empty, 1));
    assert_eq!(callback.frozen_previous, None);
    let before = stock_delay_tracker(callback.graphs.previous.as_ref().unwrap());
    let mut actual = [0.0_f32; 257];
    without_heap_activity(|| callback.render(&mut actual[..120], 1));
    let after = stock_delay_tracker(callback.graphs.previous.as_ref().unwrap());
    assert_ne!(before.step, after.step);
    without_heap_activity(|| callback.render(&mut actual[120..], 1));
    for (index, actual) in actual.iter().enumerate() {
        let mix = (index as f32 / callback.fade_length as f32).min(1.0);
        let expected = (old[index] * (1.0 - mix) + new[index] * mix).clamp(-1.0, 1.0);
        assert!(
            (actual - expected).abs() < 1e-6,
            "frame {index}: {actual} vs {expected}"
        );
    }
}

#[test]
fn stock_delay_callback_frozen_unequal_stereo_maps_into_mono_one_and_six_channel_devices() {
    for channels in [1, 2, 6] {
        let mut stereo = stock_delay_callback_patch(16.0, false);
        stereo.nodes.push(
            serde_json::from_value(serde_json::json!({
                "id":"right","kind":"constant","params":{"value":-8}
            }))
            .unwrap(),
        );
        stereo.nodes.push(
            serde_json::from_value(serde_json::json!({
                "id":"stereo","kind":"audio_stereo","params":{"gain":2}
            }))
            .unwrap(),
        );
        stereo.cables.push(
            serde_json::from_value(serde_json::json!({
                "id":"delay_left","from":"delay","output":"mix","to":"stereo","input":"left"
            }))
            .unwrap(),
        );
        stereo.cables.push(
            serde_json::from_value(serde_json::json!({
                "id":"right_stereo","from":"right","to":"stereo","input":"right"
            }))
            .unwrap(),
        );
        stereo.output = Some("stereo".into());
        stereo.output_port = "out".into();
        let (mut callback, mut updates, _disposal) = sequence_callback(&stereo);
        stock_delay_fixed_gain(&mut callback, 0.5);
        let mut warm = vec![0.0_f32; 129 * channels];
        without_heap_activity(|| callback.render(&mut warm, channels));
        let cached = [3.2, -1.6];
        assert_eq!(callback.last_active_raw, cached);
        assert!(callback.last_active_stereo);
        if channels >= 2 {
            assert_eq!(&warm[warm.len() - channels..][..2], &[1.0, -0.8]);
        }

        let mut mono = stereo.clone();
        mono.nodes[0].params.insert("value".into(), -1.5);
        mono.output = Some("delay".into());
        mono.output_port = "mix".into();
        let mut reference = mono.compile().unwrap();
        reference.adopt_state_from(&callback.graphs.active.engine);
        let mut raw = [[0.0_f32; 2]; 257];
        reference.process_audio_block(&mut raw);
        assert!(updates.push(sequence_graph(&callback, &mono, 2)).is_ok());
        let mut empty = [0.0_f32; 0];
        without_heap_activity(|| callback.render(&mut empty, channels));
        assert_eq!(callback.frozen_previous, Some((cached, true)));
        let mut actual = vec![0.0_f32; 257 * channels];
        without_heap_activity(|| callback.render(&mut actual, channels));
        for (index, (frame, current)) in actual.chunks_exact(channels).zip(raw).enumerate() {
            let mix = (index as f32 / callback.fade_length as f32).min(1.0);
            for (channel, actual) in frame.iter().enumerate() {
                let old = if channels == 1 {
                    cached[0] * 0.5 + cached[1] * 0.5
                } else if channel < 2 {
                    cached[channel]
                } else {
                    0.0
                };
                let expected = ((old * (1.0 - mix) + current[0] * mix) * 0.5).clamp(-1.0, 1.0);
                assert!(
                    (actual - expected).abs() < 1e-6,
                    "{channels} channels, frame {index}, lane {channel}: {actual} vs {expected}"
                );
            }
        }
        assert!(actual[actual.len() - channels..]
            .iter()
            .all(|sample| *sample == -0.75));
        if channels == 1 {
            assert!((actual[0] - 0.4).abs() < 1e-6);
        } else {
            assert_eq!(&actual[..2], &[1.0, -0.8]);
        }
        if channels == 6 {
            assert_eq!(&actual[2..6], &[0.0; 4]);
        }
        assert_eq!(callback.gain.value, 0.5);
        assert_eq!(callback.last_active_raw, [-1.5; 2]);
        assert!(!callback.last_active_stereo);
        assert_eq!(callback.frozen_previous, None);
    }
}
