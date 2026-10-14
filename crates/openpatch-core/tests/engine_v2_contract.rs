//! Independent wire/behavior acceptance tests for the versioned engine.
use openpatch_core::{Cable, Node, Patch, PortFrame};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct CallbackAllocator;
thread_local! {
    static TRACK_MEMORY: Cell<bool> = const { Cell::new(false) };
    static MEMORY_OPERATIONS: Cell<usize> = const { Cell::new(0) };
}

#[test]
fn multi_vco_first_callback_and_state_adoption_use_no_heap() {
    let mut p = patch(
        vec![
            node("pitch", "poly_constant", &[("channels", 16.0)]),
            node(
                "vco",
                "multi_vco",
                &[("frequency", 220.0), ("pulse_width", 0.3)],
            ),
        ],
        vec![cable("pitch", "out", "vco", "pitch")],
        "vco",
    );
    p.output_port = "sine".into();
    let mut old = p.compile().unwrap();
    let mut replacement = p.compile().unwrap();
    let mut frames = [PortFrame::default(); 129];
    MEMORY_OPERATIONS.with(|count| count.set(0));
    TRACK_MEMORY.with(|tracking| tracking.set(true));
    for length in [0, 1, 3, 64, 128, 129] {
        old.process_poly_block(&mut frames[..length]);
        replacement.adopt_state_from(&old);
        replacement.process_poly_block(&mut frames[..length]);
    }
    TRACK_MEMORY.with(|tracking| tracking.set(false));
    assert_eq!(MEMORY_OPERATIONS.with(Cell::get), 0);
    assert!(frames
        .iter()
        .all(|frame| frame.values.iter().all(|value| value.is_finite())));
}
fn record_memory_operation() {
    let _ = TRACK_MEMORY.try_with(|active| {
        if active.get() {
            let _ = MEMORY_OPERATIONS.try_with(|count| count.set(count.get() + 1));
        }
    });
}
unsafe impl GlobalAlloc for CallbackAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record_memory_operation();
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record_memory_operation();
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        record_memory_operation();
        unsafe { System.realloc(ptr, layout, size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        record_memory_operation();
        unsafe { System.dealloc(ptr, layout) }
    }
}
#[global_allocator]
static ALLOCATOR: CallbackAllocator = CallbackAllocator;

fn node(id: &str, kind: &str, params: &[(&str, f32)]) -> Node {
    Node {
        score: None,
        bypassed: false,
        id: id.into(),
        kind: kind.into(),
        params: params.iter().map(|(k, v)| ((*k).into(), *v)).collect(),
    }
}

fn cable(from: &str, output: &str, to: &str, input: &str) -> Cable {
    Cable {
        input_order: None,
        output_order: None,
        color: None,
        id: None,
        from: from.into(),
        output: output.into(),
        to: to.into(),
        input: input.into(),
    }
}

fn patch(nodes: Vec<Node>, cables: Vec<Cable>, output: &str) -> Patch {
    Patch {
        nodes,
        cables,
        output: Some(output.into()),
        ..Patch::new_v2()
    }
}

fn render(patch: &Patch, length: usize) -> Vec<PortFrame> {
    let mut frames = vec![PortFrame::default(); length];
    patch.compile().unwrap().process_poly_block(&mut frames);
    frames
}

fn active(frame: &PortFrame) -> &[f32] {
    assert!(frame.channels <= 16);
    assert!(frame.values[frame.channels as usize..]
        .iter()
        .all(|v| *v == 0.0));
    &frame.values[..frame.channels as usize]
}

#[test]
fn literal_v1_fixture_keeps_defaults_and_same_sample_rendering() {
    // Intentionally omit every new field; this is the old on-disk schema.
    let legacy: Patch = serde_json::from_str(
        r#"{
        "nodes":[{"id":"dc","kind":"constant","params":{"value":3}},
                 {"id":"amp","kind":"gain","params":{"gain":2}}],
        "cables":[{"from":"dc","to":"amp","input":"signal"}],
        "output":"amp"
    }"#,
    )
    .unwrap();
    assert_eq!(legacy.version, 1);
    assert_eq!(legacy.output_port, "out");
    assert_eq!(legacy.cables[0].output, "out");
    let mut actual = [0.0; 137];
    legacy.compile().unwrap().process_block(&mut actual);
    assert_eq!(actual, [6.0; 137]);
    let mut unsupported = legacy.clone();
    unsupported.cables[0].output = "out1".into();
    assert!(unsupported.compile().is_err());
    unsupported = legacy;
    unsupported.nodes[0] = node("dc", "poly_constant", &[]);
    assert!(unsupported.compile().is_err());
}

#[test]
fn two_edges_delay_values_and_channel_counts_one_sample_each() {
    let p = patch(
        vec![
            node(
                "source",
                "poly_constant",
                &[("channels", 3.0), ("v1", 2.0), ("v2", 4.0), ("v3", 6.0)],
            ),
            node("first", "output", &[]),
            node("second", "output", &[]),
        ],
        vec![
            cable("source", "out", "first", "signal"),
            cable("first", "out", "second", "signal"),
        ],
        "second",
    );
    let frames = render(&p, 5);
    assert_eq!(active(&frames[0]), &[0.0]);
    assert_eq!(active(&frames[1]), &[0.0]);
    for frame in &frames[2..] {
        assert_eq!(active(frame), &[2.0, 4.0, 6.0]);
    }
    let mut direct = p;
    direct.output = Some("source".into());
    assert_eq!(active(&render(&direct, 1)[0]), &[2.0, 4.0, 6.0]);
}

#[test]
fn zero_channels_normalize_but_short_poly_auxiliary_channels_are_zero() {
    for (count, expected) in [
        (0.0, [2.0, 3.0, 4.0]),
        (1.0, [20.0, 30.0, 40.0]),
        (2.0, [20.0, 36.0, 0.0]),
    ] {
        let p = patch(
            vec![
                node(
                    "signal",
                    "poly_constant",
                    &[("channels", 3.0), ("v1", 2.0), ("v2", 3.0), ("v3", 4.0)],
                ),
                node(
                    "cv",
                    "poly_constant",
                    &[("channels", count), ("v1", 10.0), ("v2", 12.0)],
                ),
                node("gain", "gain", &[]),
            ],
            vec![
                cable("signal", "out", "gain", "signal"),
                cable("cv", "out", "gain", "cv"),
            ],
            "gain",
        );
        assert_eq!(active(&render(&p, 2)[1]), &expected);
    }
    let p = patch(
        vec![
            node("empty", "poly_constant", &[("channels", 0.0)]),
            node("normal", "attenuverter", &[("gain", 0.5)]),
        ],
        vec![cable("empty", "out", "normal", "signal")],
        "normal",
    );
    assert!(render(&p, 9).iter().all(|f| active(f) == [5.0]));
}

#[test]
fn feedback_recurrences_ignore_node_order_and_callback_partition() {
    for two_nodes in [false, true] {
        let mut p = patch(
            vec![
                node("one", "constant", &[("value", 1.0)]),
                node("sum", "mixer", &[]),
            ],
            vec![cable("one", "out", "sum", "a")],
            "sum",
        );
        if two_nodes {
            p.nodes.push(node("half", "gain", &[("gain", 0.5)]));
            p.cables.extend([
                cable("sum", "out", "half", "signal"),
                cable("half", "out", "sum", "b"),
            ]);
        } else {
            p.cables.push(cable("sum", "out", "sum", "b"));
        }
        let mut expected = vec![0.0; 257];
        for n in 1..expected.len() {
            expected[n] = if two_nodes {
                1.0 + if n >= 2 { 0.5 * expected[n - 2] } else { 0.0 }
            } else {
                n as f32
            };
        }
        p.nodes.reverse();
        p.cables.reverse();
        for chunk_length in [1, 3, 64, 129, 257] {
            let mut engine = p.compile().unwrap();
            let mut actual = vec![0.0; expected.len()];
            for chunk in actual.chunks_mut(chunk_length) {
                engine.process_block(chunk);
            }
            assert_eq!(
                actual, expected,
                "two_nodes={two_nodes}, chunk={chunk_length}"
            );
        }
    }
}

#[test]
fn sparse_merge_split_sum_and_mono_adapter_have_distinct_contracts() {
    let mut p = patch(
        vec![
            node("two", "constant", &[("value", 2.0)]),
            node("five", "constant", &[("value", 5.0)]),
            node("merge", "poly_merge", &[]),
            node("split", "poly_split", &[]),
            node("sum", "poly_sum", &[]),
        ],
        vec![
            cable("two", "out", "merge", "in1"),
            cable("five", "out", "merge", "in4"),
            cable("merge", "out", "split", "signal"),
            cable("merge", "out", "sum", "signal"),
        ],
        "merge",
    );
    let merged = render(&p, 3);
    assert!(active(&merged[0]).is_empty());
    assert_eq!(active(&merged[1]), &[2.0, 0.0, 0.0, 5.0]);
    let mut mono = [0.0; 3];
    p.compile().unwrap().process_block(&mut mono);
    assert_eq!(mono, [0.0, 2.0, 2.0]);
    p.output = Some("sum".into());
    assert_eq!(active(&render(&p, 3)[2]), &[7.0]);
    p.output = Some("split".into());
    for (port, expected) in [("out4", 5.0), ("out2", 0.0), ("out16", 0.0)] {
        p.output_port = port.into();
        assert_eq!(active(&render(&p, 3)[2]), &[expected]);
    }
    p.output_port = "out17".into();
    assert!(p.compile().is_err());
}

#[test]
fn sixteen_channels_and_explicit_zero_are_preserved_without_stale_lanes() {
    let mut source = node("source", "poly_constant", &[("channels", 16.0)]);
    for n in 1..=16 {
        source.params.insert(format!("v{n}"), n as f32);
    }
    let mut p = patch(vec![source], vec![], "source");
    let full = render(&p, 1);
    assert_eq!(
        active(&full[0]),
        &(1..=16).map(|n| n as f32).collect::<Vec<_>>()
    );
    for count in [1.0, 0.0, 16.0] {
        p.nodes[0].params.insert("channels".into(), count);
        assert_eq!(active(&render(&p, 1)[0]).len(), count as usize);
    }
}

#[test]
fn adoption_preserves_unchanged_edge_frames_but_clears_rewired_edges() {
    let mut p = patch(
        vec![
            node("a", "constant", &[("value", 2.0)]),
            node("b", "constant", &[("value", 7.0)]),
            node("sink", "output", &[]),
        ],
        vec![cable("a", "out", "sink", "signal")],
        "sink",
    );
    let mut old = p.compile().unwrap();
    old.process_block(&mut [0.0; 17]);
    p.nodes[0].params.insert("value".into(), 3.0);
    let mut unchanged = p.compile().unwrap();
    unchanged.adopt_state_from(&old);
    let mut out = [0.0; 2];
    unchanged.process_block(&mut out);
    assert_eq!(out, [2.0, 3.0]);
    p.cables[0].from = "b".into();
    let mut rewired = p.compile().unwrap();
    rewired.adopt_state_from(&old);
    rewired.process_block(&mut out);
    assert_eq!(out, [0.0, 7.0]);
    old.process_block(&mut out);
    assert_eq!(
        out,
        [2.0, 2.0],
        "adoption must leave the old crossfade branch intact"
    );
}

#[test]
fn retired_sample_hold_voice_restarts_at_configured_initial_voltage() {
    let mut p = patch(
        vec![
            node("signal", "constant", &[("value", 3.0)]),
            node(
                "trigger",
                "poly_constant",
                &[("channels", 2.0), ("v1", 10.0), ("v2", 10.0)],
            ),
            node("held", "sample_hold", &[("initial", -2.0)]),
        ],
        vec![
            cable("signal", "out", "held", "signal"),
            cable("trigger", "out", "held", "trigger"),
        ],
        "held",
    );
    let mut engine = p.compile().unwrap();
    let mut frames = [PortFrame::default(); 4];
    engine.process_poly_block(&mut frames);
    assert_eq!(active(&frames[3]), &[3.0, 3.0]);
    p.nodes[1].params.insert("channels".into(), 1.0);
    let mut shrunk = p.compile().unwrap();
    shrunk.adopt_state_from(&engine);
    shrunk.process_poly_block(&mut frames);
    assert_eq!(
        active(&frames[0]),
        &[3.0, 3.0],
        "old cable channel count survives one sample"
    );
    assert_eq!(active(&frames[3]), &[3.0]);
    p.nodes[0].params.insert("value".into(), 9.0);
    p.nodes[1].params.insert("channels".into(), 2.0);
    p.nodes[1].params.insert("v2".into(), 0.0);
    let mut grown = p.compile().unwrap();
    grown.adopt_state_from(&shrunk);
    grown.process_poly_block(&mut frames);
    assert_eq!(active(&frames[0]), &[3.0]);
    assert_eq!(active(&frames[3]), &[3.0, -2.0]);
}

#[test]
fn poly_processing_and_state_adoption_neither_allocate_nor_deallocate() {
    let p = patch(
        vec![
            node(
                "source",
                "poly_constant",
                &[("channels", 16.0), ("v1", 1.0)],
            ),
            node("osc", "analog_vco", &[]),
            node("filter", "ladder_vcf", &[]),
            node("vca", "analog_vca", &[]),
            node("sum", "poly_sum", &[]),
        ],
        vec![
            cable("source", "out", "osc", "pitch"),
            cable("osc", "out", "filter", "signal"),
            cable("filter", "out", "vca", "signal"),
            cable("vca", "out", "sum", "signal"),
        ],
        "sum",
    );
    let mut old = p.compile().unwrap();
    let mut replacement = p.compile().unwrap();
    let mut frames = [PortFrame::default(); 129];
    let mut mono = [0.0; 129];
    MEMORY_OPERATIONS.with(|count| count.set(0));
    TRACK_MEMORY.with(|tracking| tracking.set(true));
    for length in [0, 1, 3, 128, 129] {
        old.process_poly_block(&mut frames[..length]);
    }
    replacement.adopt_state_from(&old);
    replacement.process_poly_block(&mut frames);
    replacement.process_block(&mut mono);
    TRACK_MEMORY.with(|tracking| tracking.set(false));
    assert_eq!(MEMORY_OPERATIONS.with(Cell::get), 0);
    assert!(mono.iter().all(|v| v.is_finite()));
}

#[test]
fn invalid_channels_and_ports_reject_while_duplicate_destinations_stack() {
    for kind in ["poly_constant", "poly_merge"] {
        for count in [-1.0, 0.5, 17.0, f32::NAN] {
            let p = patch(
                vec![node("unit", kind, &[("channels", count)])],
                vec![],
                "unit",
            );
            assert!(p.compile().is_err(), "{kind}: channels={count}");
        }
    }
    let p = patch(
        vec![node("a", "constant", &[]), node("b", "output", &[])],
        vec![cable("a", "out", "b", "signal")],
        "b",
    );
    let mut bad = p.clone();
    bad.cables[0].output = "missing".into();
    assert!(bad.compile().is_err());
    bad = p.clone();
    bad.cables[0].input = "out".into();
    assert!(bad.compile().is_err());
    bad = p;
    bad.cables.push(bad.cables[0].clone());
    assert!(bad.compile().is_ok());
}

#[test]
fn max_capacity_stacks_and_bound_route_exclusions_use_no_callback_heap() {
    let mut nodes = vec![
        node("source", "constant", &[("value", 1.0)]),
        node("out", "gain", &[]),
    ];
    for index in 0..openpatch_core::MAX_NODES - nodes.len() {
        nodes.push(node(&format!("idle_{index}"), "constant", &[]));
    }
    let mut p = patch(
        nodes,
        (0..openpatch_core::MAX_CABLES)
            .map(|_| cable("source", "out", "out", "signal"))
            .collect(),
        "out",
    );
    p.assign_missing_cable_ids().unwrap();
    let mut old = p.compile().unwrap();
    let mut replacement = p.compile().unwrap();
    let valid = replacement.resolve_cable_state("cable_1").unwrap();
    let foreign = old.resolve_cable_state("cable_2").unwrap();
    let mut frames = [PortFrame::default(); 3];
    old.process_poly_block(&mut frames[..1]);
    MEMORY_OPERATIONS.with(|count| count.set(0));
    TRACK_MEMORY.with(|tracking| tracking.set(true));
    replacement.adopt_state_from_excluding_routes(&old, &[], &[foreign, valid]);
    replacement.process_poly_block(&mut frames);
    TRACK_MEMORY.with(|tracking| tracking.set(false));
    assert_eq!(MEMORY_OPERATIONS.with(Cell::get), 0);
    assert_eq!(frames[0].values[0], (openpatch_core::MAX_CABLES - 1) as f32);
}

#[test]
fn midi_events_stereo_first_callback_and_adoption_neither_allocate_nor_free() {
    use openpatch_core::MidiEvent;
    let p = patch(
        vec![
            node("m", "midi_cv", &[("channels", 16.0)]),
            node("a", "audio_stereo", &[]),
        ],
        vec![cable("m", "gate", "a", "left")],
        "a",
    );
    let mut old = p.compile().unwrap();
    let mut new = p.compile().unwrap();
    let mut poly = [PortFrame::default(); 129];
    let mut audio = [[0.0; 2]; 129];
    let mut mono = [0.0; 129];
    let events = [
        MidiEvent::NoteOn {
            channel: 0,
            note: 60,
            velocity: 127,
        },
        MidiEvent::ControlChange {
            channel: 0,
            controller: 64,
            value: 127,
        },
        MidiEvent::NoteOff {
            channel: 0,
            note: 60,
            velocity: 0,
        },
        MidiEvent::Clock,
        MidiEvent::Start,
        MidiEvent::Stop,
        MidiEvent::Continue,
        MidiEvent::PitchBend {
            channel: 0,
            value: 16383,
        },
        MidiEvent::ChannelPressure {
            channel: 0,
            pressure: 127,
        },
        MidiEvent::Panic,
        MidiEvent::NoteOn {
            channel: 255,
            note: 255,
            velocity: 255,
        },
    ];
    MEMORY_OPERATIONS.with(|count| count.set(0));
    TRACK_MEMORY.with(|tracking| tracking.set(true));
    for _ in 0..30 {
        for event in events {
            old.handle_midi(event);
            old.process_audio_block(&mut audio);
            new.adopt_state_from(&old);
            new.handle_midi(event);
            new.process_poly_block(&mut poly);
            new.process_block(&mut mono);
        }
    }
    TRACK_MEMORY.with(|tracking| tracking.set(false));
    assert_eq!(MEMORY_OPERATIONS.with(Cell::get), 0);
    assert!(audio.iter().flatten().all(|v| v.is_finite()));
}

#[test]
fn routing_first_callbacks_and_state_adoption_neither_allocate_nor_free() {
    let p = patch(
        vec![
            node("source", "poly_constant", &[("channels", 16.0)]),
            node("multiple", "mult", &[]),
            node("transpose", "octave", &[("octaves", -2.0)]),
            node("mix", "mix6", &[("average", 1.0), ("level", 0.5)]),
            node(
                "stereo",
                "mid_side",
                &[("encode_width", 0.5), ("decode_width", 1.5)],
            ),
            node("audio", "audio_stereo", &[]),
        ],
        vec![
            cable("source", "out", "multiple", "signal"),
            cable("multiple", "out1", "transpose", "pitch"),
            cable("transpose", "out", "mix", "in1"),
            cable("multiple", "out2", "mix", "in6"),
            cable("mix", "out", "stereo", "left"),
            cable("multiple", "out8", "stereo", "right"),
            cable("stereo", "left", "audio", "left"),
            cable("stereo", "right", "audio", "right"),
        ],
        "audio",
    );
    let mut old = p.compile().unwrap();
    let mut new = p.compile().unwrap();
    let mut original = [[0.0; 2]; 129];
    let mut replacement = [[0.0; 2]; 129];
    let mut poly = [PortFrame::default(); 129];
    let mut mono = [0.0; 129];
    MEMORY_OPERATIONS.with(|count| count.set(0));
    TRACK_MEMORY.with(|tracking| tracking.set(true));
    for length in [0, 1, 3, 64, 127, 128, 129] {
        for _ in 0..20 {
            new.adopt_state_from(&old);
            old.process_audio_block(&mut original[..length]);
            new.process_audio_block(&mut replacement[..length]);
            assert_eq!(&original[..length], &replacement[..length]);
            old.process_poly_block(&mut poly[..length]);
            new.process_poly_block(&mut poly[..length]);
            old.process_block(&mut mono[..length]);
            new.process_block(&mut mono[..length]);
        }
    }
    TRACK_MEMORY.with(|tracking| tracking.set(false));
    assert_eq!(MEMORY_OPERATIONS.with(Cell::get), 0);
}

#[test]
fn multi_lfo_first_callbacks_adoption_and_detector_rewiring_neither_allocate_nor_free() {
    let p = patch(
        vec![
            node("fm", "poly_constant", &[("channels", 16.0)]),
            node("clock", "gate", &[("frequency", 100.0)]),
            node("reset", "constant", &[("value", 10.0)]),
            node(
                "lfo",
                "multi_lfo",
                &[("frequency", 4.0), ("fm_amount", 0.5), ("pwm_amount", 0.5)],
            ),
        ],
        vec![
            cable("fm", "out", "lfo", "fm"),
            cable("fm", "out", "lfo", "pwm"),
            cable("clock", "out", "lfo", "clock"),
            cable("reset", "out", "lfo", "reset"),
        ],
        "lfo",
    );
    let mut p = p;
    p.output_port = "sine".into();
    let mut old = p.compile().unwrap();
    let mut new = p.compile().unwrap();
    let mut changed = p.clone();
    changed.nodes[2].kind = "gate".into();
    changed.nodes[2].params.clear();
    changed.cables[2].from = "reset".into();
    changed.cables[3].from = "clock".into();
    let mut rewired = changed.compile().unwrap();
    let mut a = [PortFrame::default(); 129];
    let mut b = [PortFrame::default(); 129];
    let mut audio = [[0.0; 2]; 129];
    let mut mono = [0.0; 129];
    MEMORY_OPERATIONS.with(|count| count.set(0));
    TRACK_MEMORY.with(|tracking| tracking.set(true));
    for count in [0, 1, 3, 64, 127, 128, 129] {
        for _ in 0..20 {
            new.adopt_state_from(&old);
            old.process_poly_block(&mut a[..count]);
            new.process_poly_block(&mut b[..count]);
            assert_eq!(&a[..count], &b[..count]);
            old.process_audio_block(&mut audio[..count]);
            new.process_audio_block(&mut audio[..count]);
            old.process_block(&mut mono[..count]);
            new.process_block(&mut mono[..count]);
            rewired.adopt_state_from(&old);
            rewired.process_poly_block(&mut b[..count]);
        }
    }
    TRACK_MEMORY.with(|tracking| tracking.set(false));
    assert_eq!(MEMORY_OPERATIONS.with(Cell::get), 0);
}

#[test]
fn sequencer_actions_telemetry_identity_reset_and_adoption_neither_allocate_nor_free() {
    use openpatch_core::ModuleAction;
    let mut p = patch(
        vec![
            node("s", "seq3", &[("tempo", 960.0)]),
            node("out", "audio_stereo", &[]),
        ],
        vec![cable("s", "cv1", "out", "left")],
        "out",
    );
    let mut old = p.compile().unwrap();
    let mut new = p.compile().unwrap();
    let old_h = old.resolve_module_event("s").unwrap();
    let new_h = new.resolve_module_event("s").unwrap();
    p.nodes.reverse();
    let mut reordered = p.compile().unwrap();
    let re_h = reordered.resolve_module_event("s").unwrap();
    let mut audio = [[0.0; 2]; 129];
    let mut poly = [PortFrame::default(); 129];
    let mut mono = [0.0; 129];
    MEMORY_OPERATIONS.with(|count| count.set(0));
    TRACK_MEMORY.with(|tracking| tracking.set(true));
    for count in [0, 1, 3, 64, 127, 128, 129] {
        for _ in 0..20 {
            for action in [
                ModuleAction::Run,
                ModuleAction::Clock,
                ModuleAction::Run,
                ModuleAction::Clock,
                ModuleAction::Reset,
            ] {
                assert!(old.handle_module_event(old_h, action));
            }
            old.process_audio_block(&mut audio[..count]);
            new.adopt_state_from(&old);
            reordered.adopt_state_from(&old);
            let _ = old.module_telemetry(old_h);
            let _ = new.module_telemetry(new_h);
            let _ = reordered.module_telemetry(re_h);
            assert!(!new.handle_module_event(old_h, ModuleAction::Clock));
            assert!(reordered.reset_module_state(re_h));
            new.process_poly_block(&mut poly[..count]);
            reordered.process_block(&mut mono[..count]);
            // A zero-length callback leaves pending actions queued, so reset before
            // the next test iteration rather than overflowing a deliberately bounded queue.
            assert!(old.reset_module_state(old_h));
            assert!(new.reset_module_state(new_h));
        }
    }
    TRACK_MEMORY.with(|tracking| tracking.set(false));
    assert_eq!(MEMORY_OPERATIONS.with(Cell::get), 0);
}

#[test]
fn sequential_switch_callbacks_telemetry_reset_and_adoption_neither_allocate_nor_free() {
    use openpatch_core::{MidiEvent, ModuleAction};
    let mut p = patch(
        vec![
            node("clock", "midi_cv", &[("channels", 1.0)]),
            node("poly", "poly_constant", &[("channels", 16.0)]),
            node("a", "switch_1to4", &[("declick", 1.0)]),
            node("b", "switch_4to1", &[("declick", 1.0)]),
            node("out", "audio_stereo", &[]),
        ],
        vec![
            cable("clock", "gate", "a", "clock"),
            cable("clock", "gate", "b", "reset"),
            cable("poly", "out", "a", "signal"),
            cable("a", "out1", "b", "in1"),
            cable("a", "out2", "b", "in2"),
            cable("a", "out3", "b", "in3"),
            cable("a", "out4", "b", "in4"),
            cable("b", "out", "out", "left"),
        ],
        "out",
    );
    let mut old = p.compile().unwrap();
    let mut new = p.compile().unwrap();
    let old_h = old.resolve_module_telemetry("a").unwrap();
    let new_h = new.resolve_module_telemetry("b").unwrap();
    p.nodes.reverse();
    let mut reordered = p.compile().unwrap();
    let re_h = reordered.resolve_module_telemetry("a").unwrap();
    let mut audio = [[0.0; 2]; 129];
    let mut poly = [PortFrame::default(); 129];
    let mut mono = [0.0; 129];
    MEMORY_OPERATIONS.with(|count| count.set(0));
    TRACK_MEMORY.with(|tracking| tracking.set(true));
    for count in [0, 1, 3, 64, 127, 128, 129] {
        for _ in 0..20 {
            old.handle_midi(MidiEvent::NoteOn {
                channel: 0,
                note: 60,
                velocity: 100,
            });
            old.process_audio_block(&mut audio[..count]);
            old.handle_midi(MidiEvent::NoteOff {
                channel: 0,
                note: 60,
                velocity: 0,
            });
            old.process_poly_block(&mut poly[..count]);
            new.adopt_state_from(&old);
            reordered.adopt_state_from(&old);
            assert!(old.module_telemetry(old_h).is_some());
            assert!(new.module_telemetry(new_h).is_some());
            assert!(reordered.module_telemetry(re_h).is_some());
            assert!(!new.handle_module_event(new_h, ModuleAction::Clock));
            assert!(!reordered.reset_module_state(old_h));
            assert!(reordered.reset_module_state(re_h));
            new.process_block(&mut mono[..count]);
            reordered.process_audio_block(&mut audio[..count]);
        }
    }
    TRACK_MEMORY.with(|tracking| tracking.set(false));
    assert_eq!(MEMORY_OPERATIONS.with(Cell::get), 0);
}

#[test]
fn utility_banks_and_bypass_transitions_midi_scratch_and_callbacks_do_not_allocate_or_free() {
    use openpatch_core::MidiEvent;
    let mut p = patch(
        vec![
            node("midi", "midi_cv", &[("channels", 16.0)]),
            node("vert", "attenuverter8", &[("gain8", 1.0)]),
            node("mutes", "mutes10", &[]),
            node("mix", "cv_mix", &[("level1", 1.0), ("level2", -1.0)]),
            node("seq", "seq3", &[("row1_1", 10.0)]),
            node("out", "audio_stereo", &[]),
        ],
        vec![
            cable("midi", "pitch", "vert", "in1"),
            cable("vert", "out8", "mutes", "in1"),
            cable("mutes", "out1", "mix", "in1"),
            cable("mutes", "out10", "mix", "in2"),
            cable("seq", "cv1", "seq", "clock"),
            cable("mix", "out", "out", "left"),
        ],
        "out",
    );
    let mut active = p.compile().unwrap();
    let mut resumed = p.compile().unwrap();
    for node in &mut p.nodes {
        node.bypassed = true;
    }
    let mut muted = p.compile().unwrap();
    let mut audio = [[0.0; 2]; 129];
    let mut poly = [PortFrame::default(); 129];
    MEMORY_OPERATIONS.with(|count| count.set(0));
    TRACK_MEMORY.with(|tracking| tracking.set(true));
    for count in [0, 1, 3, 64, 127, 128, 129] {
        for _ in 0..20 {
            active.handle_midi(MidiEvent::NoteOn {
                channel: 0,
                note: 60,
                velocity: 100,
            });
            active.process_audio_block(&mut audio[..count]);
            muted.adopt_state_from(&active);
            muted.handle_midi(MidiEvent::NoteOff {
                channel: 0,
                note: 60,
                velocity: 0,
            });
            muted.process_poly_block(&mut poly[..count]);
            resumed.adopt_state_from(&muted);
            resumed.process_audio_block(&mut audio[..count]);
            active.adopt_state_from(&resumed);
        }
    }
    TRACK_MEMORY.with(|tracking| tracking.set(false));
    assert_eq!(MEMORY_OPERATIONS.with(Cell::get), 0);
}

#[test]
fn signal_utilities_manual_controls_lights_and_adoption_are_allocation_free() {
    use openpatch_core::ModuleAction;
    let mut p = patch(
        vec![
            node("v", "poly_constant", &[("channels", 16.0)]),
            node("logic", "logic", &[]),
            node("compare", "compare", &[]),
            node("process", "process", &[("slew", 0.0)]),
            node("rescale", "rescale", &[("gain", 1.0)]),
            node("out", "audio_stereo", &[]),
        ],
        vec![
            cable("v", "out", "logic", "a"),
            cable("v", "out", "compare", "a"),
            cable("v", "out", "process", "signal"),
            cable("logic", "or", "process", "gate"),
            cable("process", "slew", "rescale", "signal"),
            cable("rescale", "out", "out", "left"),
        ],
        "out",
    );
    let mut old = p.compile().unwrap();
    let mut new = p.compile().unwrap();
    let logic = old.resolve_module_control("logic").unwrap();
    let process = old.resolve_module_control("process").unwrap();
    let lights = [
        old.resolve_module_lights("logic").unwrap(),
        old.resolve_module_lights("compare").unwrap(),
        old.resolve_module_lights("process").unwrap(),
        old.resolve_module_lights("rescale").unwrap(),
    ];
    for node in &mut p.nodes {
        node.bypassed = true;
    }
    let mut bypassed = p.compile().unwrap();
    let bp = bypassed.resolve_module_control("process").unwrap();
    let mut audio = [[0.0; 2]; 129];
    let mut frames = [PortFrame::default(); 129];
    MEMORY_OPERATIONS.with(|c| c.set(0));
    TRACK_MEMORY.with(|t| t.set(true));
    for count in [0, 1, 3, 64, 127, 128, 129] {
        for _ in 0..20 {
            assert!(old.handle_module_event(logic, ModuleAction::BPress));
            assert!(old.handle_module_event(process, ModuleAction::GatePress));
            old.process_audio_block(&mut audio[..count]);
            new.adopt_state_from(&old);
            bypassed.adopt_state_from(&old);
            assert!(bypassed.handle_module_event(bp, ModuleAction::GateRelease));
            bypassed.process_poly_block(&mut frames[..count]);
            for h in lights {
                assert!(old.module_lights(h).is_some());
            }
            assert!(old.module_control_state(logic).is_some());
            assert!(old.module_control_state(process).is_some());
            assert!(old.release_module_controls(logic));
            assert!(old.release_module_controls(process));
            new.process_poly_block(&mut frames[..count]);
        }
    }
    TRACK_MEMORY.with(|t| t.set(false));
    assert_eq!(MEMORY_OPERATIONS.with(Cell::get), 0);
}

#[test]
fn vca_mix_fade_meters_bypass_and_state_adoption_do_not_allocate_or_free() {
    let mut p = patch(
        vec![
            node("poly", "poly_constant", &[("channels", 16.0)]),
            node("cv", "gate", &[("frequency", 100.0)]),
            node("mix", "vca_mix", &[("channel_exp", 1.0), ("mix_exp", 1.0)]),
            node("fade", "fade", &[("cv_amount", 1.0), ("law", 1.0)]),
            node("out", "audio_stereo", &[]),
        ],
        vec![
            cable("poly", "out", "mix", "in1"),
            cable("cv", "out", "mix", "cv1"),
            cable("cv", "out", "mix", "mix_cv"),
            cable("mix", "mix", "fade", "a"),
            cable("mix", "out1", "fade", "b"),
            cable("cv", "out", "fade", "cv"),
            cable("fade", "out", "out", "left"),
            cable("fade", "complement", "out", "right"),
        ],
        "out",
    );
    let mut old = p.compile().unwrap();
    let mut replacement = p.compile().unwrap();
    let old_h = old.resolve_module_lights("mix").unwrap();
    let new_h = replacement.resolve_module_lights("mix").unwrap();
    p.nodes[2].bypassed = true;
    p.nodes[3].bypassed = true;
    let mut bypassed = p.compile().unwrap();
    let bypass_h = bypassed.resolve_module_lights("mix").unwrap();
    let mut audio = [[0.0; 2]; 129];
    let mut frames = [PortFrame::default(); 129];
    MEMORY_OPERATIONS.with(|c| c.set(0));
    TRACK_MEMORY.with(|t| t.set(true));
    for count in [0, 1, 3, 64, 127, 128, 129] {
        for _ in 0..20 {
            old.process_audio_block(&mut audio[..count]);
            replacement.adopt_state_from(&old);
            bypassed.adopt_state_from(&old);
            assert!(old.module_lights(old_h).is_some());
            assert!(replacement.module_lights(new_h).is_some());
            assert!(bypassed.module_lights(bypass_h).is_some());
            replacement.process_poly_block(&mut frames[..count]);
            bypassed.process_audio_block(&mut audio[..count]);
            assert!(replacement.reset_module_state(new_h));
        }
    }
    TRACK_MEMORY.with(|t| t.set(false));
    assert_eq!(MEMORY_OPERATIONS.with(Cell::get), 0);
}

#[test]
fn generic_state_exclusions_and_incident_edge_clearing_neither_allocate_nor_free() {
    use openpatch_core::MidiEvent;
    for version in [1, 2] {
        let p = if version == 1 {
            Patch {
                sample_rate: 8000,
                nodes: vec![
                    node("osc", "oscillator", &[("frequency", 7.0)]),
                    node("gate", "gate", &[]),
                    node("env", "adsr", &[]),
                ],
                cables: vec![cable("gate", "out", "env", "gate")],
                output: Some("osc".into()),
                ..Patch::default()
            }
        } else {
            patch(
                vec![
                    node("osc", "oscillator", &[("frequency", 7.0)]),
                    node("midi", "midi_cv", &[("channels", 16.0)]),
                    node("env", "adsr", &[]),
                    node("seq", "seq3", &[("row1_1", 10.0)]),
                    node("out", "audio_stereo", &[]),
                ],
                vec![
                    cable("midi", "gate", "env", "gate"),
                    cable("seq", "cv1", "seq", "clock"),
                    cable("osc", "out", "out", "left"),
                    cable("env", "out", "out", "right"),
                ],
                "out",
            )
        };
        let mut old = p.compile().unwrap();
        let mut new = p.compile().unwrap();
        let handles = [
            new.resolve_module_state("osc").unwrap(),
            new.resolve_module_state("env").unwrap(),
            old.resolve_module_state("osc").unwrap(),
        ];
        let mut audio = [[0.0; 2]; 129];
        let mut frames = [PortFrame::default(); 129];
        MEMORY_OPERATIONS.with(|c| c.set(0));
        TRACK_MEMORY.with(|t| t.set(true));
        for count in [0, 1, 3, 64, 127, 128, 129] {
            for _ in 0..20 {
                old.handle_midi(MidiEvent::NoteOn {
                    channel: 0,
                    note: 60,
                    velocity: 100,
                });
                old.process_audio_block(&mut audio[..count]);
                new.adopt_state_from_excluding(&old, &handles);
                new.process_poly_block(&mut frames[..count]);
                old.adopt_state_from_excluding(&new, &handles);
            }
        }
        TRACK_MEMORY.with(|t| t.set(false));
        assert_eq!(MEMORY_OPERATIONS.with(Cell::get), 0);
    }
}

#[test]
fn pulses_actions_twenty_outputs_and_state_adoption_use_no_callback_heap() {
    use openpatch_core::ModuleAction;
    let mut p = patch(vec![node("p", "pulses", &[])], vec![], "p");
    p.output_port = "trig10".into();
    let mut old = p.compile().unwrap();
    let mut replacement = p.compile().unwrap();
    let old_control = old.resolve_module_control("p").unwrap();
    let replacement_control = replacement.resolve_module_control("p").unwrap();
    p.nodes[0].bypassed = true;
    p.output_port = "gate10".into();
    let mut bypassed = p.compile().unwrap();
    let bypass_control = bypassed.resolve_module_control("p").unwrap();
    let mut frames = [PortFrame::default(); 129];
    MEMORY_OPERATIONS.with(|count| count.set(0));
    TRACK_MEMORY.with(|tracking| tracking.set(true));
    for count in [0, 1, 3, 64, 128, 129] {
        assert!(old.release_module_controls(old_control));
        for _ in 0..32 {
            assert!(old.handle_module_event(old_control, ModuleAction::PulsePress(10)));
            assert!(old.handle_module_event(old_control, ModuleAction::PulseRelease(10)));
        }
        assert!(!old.handle_module_event(old_control, ModuleAction::PulsePress(1)));
        old.process_poly_block(&mut frames[..count]);
        replacement.adopt_state_from(&old);
        assert!(replacement
            .module_control_state(replacement_control)
            .is_some());
        assert_eq!(
            replacement
                .module_lights(replacement_control)
                .unwrap()
                .count,
            10
        );
        assert!(replacement.handle_module_event(replacement_control, ModuleAction::PulsePress(1)));
        replacement.process_poly_block(&mut frames[..count]);
        bypassed.adopt_state_from(&replacement);
        assert!(bypassed.handle_module_event(bypass_control, ModuleAction::PulseRelease(10)));
        bypassed.process_poly_block(&mut frames[..count]);
        assert!(bypassed.release_module_controls(bypass_control));
        assert!(replacement.initialize_module_state(replacement_control));
        assert!(replacement.reset_module_state(replacement_control));
    }
    TRACK_MEMORY.with(|tracking| tracking.set(false));
    assert_eq!(MEMORY_OPERATIONS.with(Cell::get), 0);
    assert!(frames.iter().all(|frame| *frame == PortFrame::mono(0.0)));
}

#[test]
fn push_saved_latch_controls_cv_and_state_adoption_use_no_callback_heap() {
    use openpatch_core::{ModuleAction, SavedModuleState};
    let mut p = patch(vec![node("p", "push", &[])], vec![], "p");
    p.output_port = "trig".into();
    let mut old = p.compile().unwrap();
    let mut replacement = p.compile().unwrap();
    let h = old.resolve_module_control("p").unwrap();
    let hr = replacement.resolve_module_state("p").unwrap();
    p.nodes[0].bypassed = true;
    let mut bypassed = p.compile().unwrap();
    let hb = bypassed.resolve_module_control("p").unwrap();
    let mut frames = [PortFrame::default(); 129];
    MEMORY_OPERATIONS.with(|count| count.set(0));
    TRACK_MEMORY.with(|tracking| tracking.set(true));
    for count in [0, 1, 3, 64, 128, 129] {
        assert!(old.release_module_controls(h));
        for _ in 0..16 {
            for action in [
                ModuleAction::PushPress,
                ModuleAction::HoldPress,
                ModuleAction::PushRelease,
                ModuleAction::HoldRelease,
            ] {
                assert!(old.handle_module_event(h, action));
            }
        }
        assert!(!old.handle_module_event(h, ModuleAction::PushPress));
        old.process_poly_block(&mut frames[..count]);
        replacement.adopt_state_from(&old);
        assert!(replacement.set_module_saved_state(hr, SavedModuleState::PushHold(true)));
        assert_eq!(
            replacement.module_saved_state(hr),
            Some(SavedModuleState::PushHold(true))
        );
        assert_eq!(replacement.module_lights(hr).unwrap().count, 2);
        assert!(replacement.release_module_controls(hr));
        replacement.process_poly_block(&mut frames[..count]);
        bypassed.adopt_state_from(&replacement);
        assert!(bypassed.handle_module_event(hb, ModuleAction::PushRelease));
        assert!(bypassed.handle_module_event(hb, ModuleAction::HoldRelease));
        bypassed.process_poly_block(&mut frames[..count]);
        assert!(replacement.initialize_module_state(hr));
        assert!(replacement.reset_module_state(hr));
    }
    TRACK_MEMORY.with(|tracking| tracking.set(false));
    assert_eq!(MEMORY_OPERATIONS.with(Cell::get), 0);
    assert!(frames.iter().all(|frame| *frame == PortFrame::mono(0.0)));
}

#[test]
fn gates_all_voice_history_eviction_render_adoption_and_bypass_use_no_heap() {
    use openpatch_core::ModuleAction;
    let mut p = patch(
        vec![
            node("osc", "oscillator", &[("frequency", 20_000.0)]),
            node("in", "gain", &[("gain", 10.0)]),
            node("poly", "poly_merge", &[("channels", 16.0)]),
            node("g", "gates", &[("length", 0.001)]),
        ],
        vec![
            cable("in", "out", "poly", "in1"),
            cable("poly", "out", "g", "in"),
        ],
        "g",
    );
    p.output_port = "delay".into();
    // Broadcast one mono changing source to every merged lane, exercising all16 rings.
    p.cables = (1..=16)
        .map(|i| cable("in", "out", "poly", &format!("in{i}")))
        .chain([
            cable("poly", "out", "g", "in"),
            cable("osc", "out", "in", "signal"),
        ])
        .collect();
    let mut old = p.compile().unwrap();
    let mut replacement = p.compile().unwrap();
    let h = old.resolve_module_control("g").unwrap();
    let hr = replacement.resolve_module_control("g").unwrap();
    p.nodes.iter_mut().find(|n| n.id == "g").unwrap().bypassed = true;
    let mut bypassed = p.compile().unwrap();
    let hb = bypassed.resolve_module_control("g").unwrap();
    let mut frames = [PortFrame::default(); 129];
    MEMORY_OPERATIONS.with(|c| c.set(0));
    TRACK_MEMORY.with(|t| t.set(true));
    // >1023 transitions per lane before adoption, then wrap/evict afterward.
    for _ in 0..20 {
        old.process_poly_block(&mut frames);
    }
    replacement.adopt_state_from(&old);
    for count in [0, 1, 3, 64, 128, 129] {
        old.process_poly_block(&mut frames[..count]);
        replacement.process_poly_block(&mut frames[..count]);
        for _ in 0..32 {
            assert!(old.handle_module_event(h, ModuleAction::ResetPress));
            assert!(old.handle_module_event(h, ModuleAction::ResetRelease));
        }
        assert!(!old.handle_module_event(h, ModuleAction::ResetPress));
        old.process_poly_block(&mut frames[..count]);
        replacement.adopt_state_from(&old);
        assert!(replacement.module_lights(hr).is_some());
        assert!(replacement.module_control_state(hr).is_some());
        assert!(replacement.release_module_controls(hr));
        assert!(replacement.initialize_module_state(hr));
        bypassed.adopt_state_from(&replacement);
        assert!(bypassed.handle_module_event(hb, ModuleAction::ResetRelease));
        bypassed.process_poly_block(&mut frames[..count]);
        assert!(old.release_module_controls(h));
        assert!(replacement.reset_module_state(hr));
    }
    TRACK_MEMORY.with(|t| t.set(false));
    assert_eq!(MEMORY_OPERATIONS.with(Cell::get), 0);
    assert!(frames.iter().all(|frame| *frame == PortFrame::mono(0.0)));
}

#[test]
fn score_and_studio_chain_callbacks_and_adoption_use_no_heap() {
    let p: Patch = serde_json::from_value(serde_json::json!({"version":2,"sample_rate":48000,"nodes":[
        {"id":"score","kind":"score_sequencer","score":{"length":4,"tracks":[{"steps":[{"step":0,"notes":[0]},{"step":2,"notes":[7]}]}]}},
        {"id":"drum","kind":"drum_voice"}, {"id":"filter","kind":"svf_filter"},
        {"id":"delay","kind":"stereo_delay","params":{"time_left":0.01,"time_right":0.02}},
        {"id":"space","kind":"stereo_reverb"}, {"id":"duck","kind":"sidechain_compressor"},
        {"id":"audio","kind":"audio_stereo"}],"cables":[
        {"from":"score","output":"gate1","to":"drum","input":"trigger"},
        {"from":"drum","to":"filter","input":"signal"},
        {"from":"filter","output":"low","to":"delay","input":"left"},
        {"from":"delay","output":"left","to":"space","input":"left"},
        {"from":"delay","output":"right","to":"space","input":"right"},
        {"from":"space","output":"left","to":"duck","input":"left"},
        {"from":"space","output":"right","to":"duck","input":"right"},
        {"from":"drum","to":"duck","input":"sidechain"},
        {"from":"duck","output":"left","to":"audio","input":"left"},
        {"from":"duck","output":"right","to":"audio","input":"right"}],"output":"audio"})).unwrap();
    let mut old = p.compile().unwrap();
    let mut next = p.compile().unwrap();
    let mut audio = [[0.0; 2]; 128];
    MEMORY_OPERATIONS.with(|c| c.set(0));
    TRACK_MEMORY.with(|c| c.set(true));
    for _ in 0..400 {
        old.process_audio_block(&mut audio);
    }
    next.adopt_state_from(&old);
    let mut expected = audio;
    old.process_audio_block(&mut expected);
    next.process_audio_block(&mut audio);
    TRACK_MEMORY.with(|c| c.set(false));
    assert_eq!(MEMORY_OPERATIONS.with(Cell::get), 0);
    assert_eq!(audio, expected);
    assert!(audio.iter().flatten().all(|x| x.is_finite()));
    assert!(audio.iter().flatten().any(|x| x.abs() > 0.000001));
}

#[test]
fn multistage_svf_polyphonic_lifecycle_uses_no_heap_and_preserves_continuation() {
    for rate in [44100, 48000, 96000] {
        for drive in [0.0, 0.05, 0.1, 1.0] {
            let mut signal = node("signal", "poly_constant", &[("channels", 16.0)]);
            let mut cv = node("cv", "poly_constant", &[("channels", 16.0)]);
            for lane in 1..=16 {
                signal.params.insert(format!("v{lane}"), lane as f32 * 0.25);
                cv.params
                    .insert(format!("v{lane}"), lane as f32 * 0.03125 - 0.25);
            }
            let mut p = patch(
                vec![
                    signal,
                    cv,
                    node(
                        "filter",
                        "svf_filter",
                        &[
                            ("cutoff", 1234.0),
                            ("resonance", 0.8),
                            ("drive", drive),
                            ("cutoff_cv_amount", 0.7),
                        ],
                    ),
                ],
                vec![
                    cable("signal", "out", "filter", "signal"),
                    cable("cv", "out", "filter", "cutoff_cv"),
                ],
                "filter",
            );
            p.sample_rate = rate;
            p.output_port = "low".into();
            let mut old = p.compile().unwrap();
            let mut same = p.compile().unwrap();
            let mut grown = p.compile().unwrap();
            let mut resumed = p.compile().unwrap();
            let mut excluded = p.compile().unwrap();
            let mut fresh = p.compile().unwrap();
            p.nodes[0].params.insert("channels".into(), 1.0);
            let mut mono = p.compile().unwrap();
            p.nodes[0].params.insert("channels".into(), 0.0);
            let mut empty = p.compile().unwrap();
            p.nodes[0].params.insert("channels".into(), 16.0);
            p.nodes[2].bypassed = true;
            let mut bypassed = p.compile().unwrap();
            let handle = resumed.resolve_module_state("filter").unwrap();
            let excluded_handle = excluded.resolve_module_state("filter").unwrap();
            let mut expected = [PortFrame::default(); 257];
            let mut actual = expected;
            let mut continuation_matches = true;
            let mut reset_succeeded = false;
            MEMORY_OPERATIONS.with(|c| c.set(0));
            TRACK_MEMORY.with(|c| c.set(true));
            // Include the first callback, several ring wraps, and arbitrary partitions.
            for length in [0, 1, 3, 64, 128, 257] {
                old.process_poly_block(&mut actual[..length]);
                same.adopt_state_from(&old);
                old.process_poly_block(&mut expected[..length]);
                same.process_poly_block(&mut actual[..length]);
                continuation_matches &= actual[..length] == expected[..length];
            }
            mono.adopt_state_from(&old);
            mono.process_poly_block(&mut actual);
            empty.adopt_state_from(&mono);
            empty.process_poly_block(&mut actual);
            grown.adopt_state_from(&empty);
            grown.process_poly_block(&mut actual);
            bypassed.adopt_state_from(&grown);
            bypassed.process_poly_block(&mut actual);
            resumed.adopt_state_from(&bypassed);
            resumed.process_poly_block(&mut actual);
            reset_succeeded |= resumed.reset_module_state(handle);
            resumed.process_poly_block(&mut actual);
            excluded.adopt_state_from_excluding(&grown, &[excluded_handle]);
            excluded.process_poly_block(&mut actual);
            fresh.process_poly_block(&mut expected);
            continuation_matches &= actual == expected;
            TRACK_MEMORY.with(|c| c.set(false));
            assert_eq!(
                MEMORY_OPERATIONS.with(Cell::get),
                0,
                "rate={rate}, drive={drive}"
            );
            assert!(continuation_matches, "rate={rate}, drive={drive}");
            assert!(reset_succeeded);
            assert_eq!(actual.last().unwrap().channels, 16);
            assert!(actual.iter().flat_map(|f| f.values).all(f32::is_finite));
            assert!(actual.iter().flat_map(|f| f.values).any(|v| v.abs() > 0.01));
        }
    }
}

#[test]
fn svf_drive_latency_is_explicit_and_bypass_remains_direct() {
    fn impulse(port: &str, bypassed: bool) -> Vec<f32> {
        let mut p = patch(
            vec![
                node("source", "constant", &[("value", 1.0)]),
                node(
                    "filter",
                    "svf_filter",
                    &[("drive", 0.0), ("resonance", 0.0)],
                ),
            ],
            vec![cable("source", "out", "filter", "signal")],
            "filter",
        );
        p.output_port = port.into();
        p.nodes[1].bypassed = bypassed;
        let mut first = p.compile().unwrap();
        let mut result = vec![0.0; 513];
        first.process_block(&mut result[..1]);
        p.nodes[0].params.insert("value".into(), 0.0);
        let mut rest = p.compile().unwrap();
        rest.adopt_state_from(&first);
        rest.process_block(&mut result[1..]);
        result
    }
    let low = impulse("low", false);
    let band = impulse("band", false);
    let high = impulse("high", false);
    // The SVF's unity decomposition reconstructs its driven input. This
    // removes the low/band/high filter phase without fitting a measured delay.
    let drive: Vec<f64> = low
        .iter()
        .zip(&band)
        .zip(&high)
        .map(|((&l, &b), &h)| f64::from(l) + 2.0 * f64::from(b) + f64::from(h))
        .collect();
    let peak = drive
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
        .unwrap()
        .0;
    assert_eq!(peak, 1 + 76, "one graph cable plus the declared FIR delay");
    assert!((drive.iter().sum::<f64>() - 1.0).abs() < 2e-6);
    for offset in 1..=76 {
        assert!((drive[peak - offset] - drive[peak + offset]).abs() < 2e-6);
    }
    let direct = impulse("low", true);
    assert_eq!(direct[1], 1.0);
    assert!(direct.iter().enumerate().all(|(n, v)| n == 1 || *v == 0.0));
    for port in ["band", "high"] {
        assert!(impulse(port, true).iter().all(|v| *v == 0.0));
    }
    // Parallel dry/filtered paths deliberately retain their relative delay;
    // no graph compensation is silently inserted by the new preprocessing.
    assert_eq!(direct.iter().position(|v| *v == 1.0).unwrap() + 76, peak);
}

#[test]
fn svf_low_gain_feedback_preserves_partition_order_and_live_rebuild() {
    let mut p = patch(
        vec![
            node("source", "constant", &[("value", 1.0)]),
            node(
                "filter",
                "svf_filter",
                &[("cutoff", 1600.0), ("resonance", 0.0), ("drive", 0.0)],
            ),
            node("feedback", "gain", &[("gain", 0.1)]),
        ],
        vec![
            cable("source", "out", "filter", "signal"),
            cable("filter", "low", "feedback", "signal"),
            cable("feedback", "out", "filter", "signal"),
        ],
        "filter",
    );
    p.output_port = "low".into();
    for rate in [44100, 48000, 96000] {
        p.sample_rate = rate;
        let mut reference = p.compile().unwrap();
        let mut expected = vec![0.0; 4096];
        reference.process_block(&mut expected);
        assert!(expected.iter().all(|v| v.is_finite() && v.abs() < 2.0));
        // Independent DC loop equation y = 1 + 0.1*y, allowing f32 and FIR error.
        assert!((f64::from(*expected.last().unwrap()) - 1.0 / 0.9).abs() < 2e-5);
        let mut reordered = p.clone();
        reordered.nodes.reverse();
        reordered.cables.reverse();
        for partition in [1, 17, 128, 257] {
            let mut before = p.compile().unwrap();
            let mut after = reordered.compile().unwrap();
            let mut actual = vec![0.0; expected.len()];
            for chunk in actual[..333].chunks_mut(partition) {
                before.process_block(chunk);
            }
            after.adopt_state_from(&before);
            for chunk in actual[333..].chunks_mut(partition) {
                after.process_block(chunk);
            }
            assert_eq!(actual, expected, "rate={rate}, partition={partition}");
        }
    }
}
