use openpatch_core::*;
use std::collections::BTreeMap;
fn node(id: &str, kind: &str, params: &[(&str, f32)]) -> Node {
    Node {
        id: id.into(),
        kind: kind.into(),
        params: params.iter().map(|(k, v)| ((*k).into(), *v)).collect(),
    }
}
fn seq(port: &str, params: &[(&str, f32)]) -> Patch {
    Patch {
        sample_rate: 8000,
        nodes: vec![node("s", "seq3", params)],
        output: Some("s".into()),
        output_port: port.into(),
        ..Patch::new_v2()
    }
}
fn event(frame: u64, action: ModuleAction) -> TimedModuleEvent {
    TimedModuleEvent {
        frame,
        id: "s".into(),
        action,
    }
}
fn one(e: &mut Engine) -> f32 {
    let mut out = [0.0];
    e.process_block(&mut out);
    out[0]
}
fn action(e: &mut Engine, h: ModuleHandle, a: ModuleAction) -> f32 {
    assert!(e.handle_module_event(h, a));
    one(e)
}
fn cable(from: &str, output: &str, to: &str, input: &str) -> Cable {
    Cable {
        from: from.into(),
        output: output.into(),
        to: to.into(),
        input: input.into(),
    }
}
#[test]
fn internal_tempo_first_tick_sample_and_callback_partitions_are_exact() {
    let p = seq("cv1", &[("row1_1", 1.0), ("row1_2", 2.0)]);
    let mut e = p.compile().unwrap();
    let mut output = [0.0; 4001];
    e.process_block(&mut output);
    assert!(output[..3999].iter().all(|v| *v == 1.0));
    assert_eq!(&output[3999..], &[2.0, 2.0]);
    let mut partitioned = p.compile().unwrap();
    let mut actual = Vec::new();
    for count in [1, 3, 127, 1025, 2845] {
        let mut out = vec![0.0; count];
        partitioned.process_block(&mut out);
        actual.extend(out);
    }
    assert_eq!(actual, output);
}
#[test]
fn actions_preserve_order_multiple_clocks_and_run_toggle_parity() {
    let mut e = seq(
        "cv1",
        &[
            ("tempo", 15.0),
            ("row1_1", 1.0),
            ("row1_2", 2.0),
            ("row1_3", 3.0),
        ],
    )
    .compile()
    .unwrap();
    let h = e.resolve_module_event("s").unwrap();
    for a in [
        ModuleAction::Run,
        ModuleAction::Clock,
        ModuleAction::Run,
        ModuleAction::Clock,
    ] {
        assert!(e.handle_module_event(h, a));
    }
    assert_eq!(one(&mut e), 2.0);
    assert_eq!(
        e.module_telemetry(h),
        Some(ModuleTelemetry {
            step: 2,
            running: true,
            length: 8
        })
    );
    e.handle_module_event(h, ModuleAction::Clock);
    e.handle_module_event(h, ModuleAction::Clock);
    one(&mut e);
    assert_eq!(e.module_telemetry(h).unwrap().step, 4);
    for _ in 0..MAX_MODULE_ACTIONS_PER_SAMPLE {
        assert!(e.handle_module_event(h, ModuleAction::Run));
    }
    assert!(!e.handle_module_event(h, ModuleAction::Run));
    one(&mut e);
    assert!(e.module_telemetry(h).unwrap().running);
}
#[test]
fn reset_guard_includes_reset_sample_and_manual_clock_obeys_it() {
    let mut e = seq("cv1", &[("tempo", 15.0), ("row1_1", 1.0), ("row1_2", 2.0)])
        .compile()
        .unwrap();
    let h = e.resolve_module_event("s").unwrap();
    assert_eq!(action(&mut e, h, ModuleAction::Clock), 2.0);
    e.handle_module_event(h, ModuleAction::Reset);
    e.handle_module_event(h, ModuleAction::Clock);
    assert_eq!(one(&mut e), 1.0);
    for _ in 1..8 {
        assert_eq!(action(&mut e, h, ModuleAction::Clock), 1.0);
    }
    assert_eq!(action(&mut e, h, ModuleAction::Clock), 2.0);
}
#[test]
fn every_length_one_tick_pulses_clock_and_enabled_trigger_without_wrap_reset() {
    for port in ["clock", "trigger", "reset"] {
        let mut e = seq(port, &[("tempo", 15.0), ("steps", 1.0)])
            .compile()
            .unwrap();
        let h = e.resolve_module_event("s").unwrap();
        assert_eq!(
            action(&mut e, h, ModuleAction::Clock),
            if port == "reset" { 0.0 } else { 10.0 }
        );
        let mut tail = [0.0; 8];
        e.process_block(&mut tail);
        if port != "reset" {
            assert_eq!(&tail[..7], &[10.0; 7]);
        }
        assert_eq!(tail[7], 0.0);
        assert_eq!(
            action(&mut e, h, ModuleAction::Clock),
            if port == "reset" { 0.0 } else { 10.0 }
        );
        assert_eq!(e.module_telemetry(h).unwrap().step, 1);
    }
    let mut e = seq("trigger", &[("steps", 1.0), ("gate1", 0.0)])
        .compile()
        .unwrap();
    let h = e.resolve_module_event("s").unwrap();
    assert_eq!(action(&mut e, h, ModuleAction::Clock), 0.0);
}
#[test]
fn length_shrink_holds_position_until_next_clock_and_steps_voltage_is_length_minus_one() {
    let p = seq("cv1", &[("tempo", 15.0), ("row1_1", 1.0), ("row1_8", 8.0)]);
    let mut old = p.compile().unwrap();
    let h = old.resolve_module_event("s").unwrap();
    for _ in 0..7 {
        old.handle_module_event(h, ModuleAction::Clock);
    }
    assert_eq!(one(&mut old), 8.0);
    let mut smaller = p.clone();
    smaller.nodes[0].params.insert("steps".into(), 1.0);
    let mut new = smaller.compile().unwrap();
    new.adopt_state_from(&old);
    let h = new.resolve_module_event("s").unwrap();
    assert_eq!(one(&mut new), 8.0);
    assert_eq!(
        new.module_telemetry(h).unwrap(),
        ModuleTelemetry {
            step: 8,
            running: true,
            length: 1
        }
    );
    assert_eq!(action(&mut new, h, ModuleAction::Clock), 1.0);
    smaller.output_port = "steps".into();
    assert_eq!(one(&mut smaller.compile().unwrap()), 0.0);
    assert_eq!(one(&mut seq("steps", &[]).compile().unwrap()), 7.0);
}
#[test]
fn pause_holds_step_and_same_identity_parameter_edits_preserve_runtime_run() {
    let p = seq("cv1", &[("tempo", 15.0), ("row1_1", 1.0), ("row1_2", 2.0)]);
    let mut old = p.compile().unwrap();
    let h = old.resolve_module_event("s").unwrap();
    action(&mut old, h, ModuleAction::Clock);
    action(&mut old, h, ModuleAction::Run);
    assert_eq!(action(&mut old, h, ModuleAction::Clock), 2.0);
    assert!(!old.module_telemetry(h).unwrap().running);
    let mut changed = p.clone();
    changed.nodes[0].params.insert("row1_2".into(), 7.0);
    let mut new = changed.compile().unwrap();
    new.adopt_state_from(&old);
    let h = new.resolve_module_event("s").unwrap();
    assert!(!new.module_telemetry(h).unwrap().running);
    assert_eq!(one(&mut new), 7.0);
    changed.nodes[0].params.insert("running".into(), 1.0);
    let mut explicit = changed.compile().unwrap();
    explicit.adopt_state_from(&new);
    assert!(
        explicit
            .module_telemetry(explicit.resolve_module_event("s").unwrap())
            .unwrap()
            .running
    );
}
#[test]
fn handles_are_engine_bound_and_identity_reset_clears_incoming_and_outgoing_delays() {
    let mut p = seq("cv1", &[("tempo", 15.0), ("row1_1", 1.0), ("row1_2", 2.0)]);
    p.nodes.push(node("clock", "constant", &[("value", 10.0)]));
    p.nodes.push(node("out", "output", &[]));
    p.cables = vec![
        cable("clock", "out", "s", "clock"),
        cable("s", "cv1", "out", "signal"),
    ];
    let mut old = p.compile().unwrap();
    let old_h = old.resolve_module_event("s").unwrap();
    one(&mut old);
    assert_eq!(one(&mut old), 2.0);
    let mut recreated = p.compile().unwrap();
    let h = recreated.resolve_module_event("s").unwrap();
    recreated.adopt_state_from(&old);
    assert!(!recreated.handle_module_event(old_h, ModuleAction::Clock));
    assert_eq!(recreated.module_telemetry(old_h), None);
    assert!(!recreated.reset_module_state(old_h));
    assert!(recreated.reset_module_state(h));
    assert_eq!(one(&mut recreated), 1.0); // Incoming old high must not clock a new instance immediately.
    assert_eq!(one(&mut recreated), 2.0); // Fresh one-sample cable delivers source high normally.
    p.output = Some("out".into());
    p.output_port = "out".into();
    let mut fresh = p.compile().unwrap();
    let h = fresh.resolve_module_event("s").unwrap();
    fresh.adopt_state_from(&old);
    fresh.reset_module_state(h);
    assert_eq!(one(&mut fresh), 0.0); // Outgoing old CV is cleared too.
    assert!(fresh.resolve_module_event("clock").is_err());
    assert!(fresh.resolve_module_event("missing").is_err());
}
#[test]
fn unchanged_high_detectors_survive_adoption_rewiring_resets_only_relevant_detector() {
    let mut p = seq(
        "cv1",
        &[
            ("tempo", 15.0),
            ("row1_1", 1.0),
            ("row1_2", 2.0),
            ("row1_3", 3.0),
        ],
    );
    p.nodes.push(node("high", "constant", &[("value", 10.0)]));
    p.nodes.push(node("other", "constant", &[("value", 10.0)]));
    p.cables = vec![cable("high", "out", "s", "clock")];
    let mut old = p.compile().unwrap();
    one(&mut old);
    assert_eq!(one(&mut old), 2.0);
    let mut same = p.compile().unwrap();
    same.adopt_state_from(&old);
    assert_eq!(one(&mut same), 2.0);
    p.cables[0].from = "other".into();
    let mut rewired = p.compile().unwrap();
    rewired.adopt_state_from(&same);
    assert_eq!(one(&mut rewired), 2.0);
    assert_eq!(one(&mut rewired), 3.0);
    p.nodes[2].kind = "gate".into();
    p.nodes[2].params = BTreeMap::new();
    let mut retyped = p.compile().unwrap();
    retyped.adopt_state_from(&rewired);
    assert_eq!(one(&mut retyped), 3.0);
    assert_eq!(one(&mut retyped), 0.0); // Gate is high initially; step4 default zero.
}
#[test]
fn all_ports_are_mono_and_cv_inputs_read_only_the_first_lane() {
    let mut p = seq("steps", &[("steps", 4.0)]);
    p.nodes.push(node(
        "cv",
        "poly_constant",
        &[("channels", 16.0), ("v1", -2.5), ("v2", 10.0)],
    ));
    p.cables = vec![cable("cv", "out", "s", "steps")];
    let mut e = p.compile().unwrap();
    one(&mut e);
    assert_eq!(one(&mut e), 1.0); // 4-2.5 rounds to2; output is length-1.
    for port in [
        "cv1", "cv2", "cv3", "trigger", "step1", "step2", "step3", "step4", "step5", "step6",
        "step7", "step8", "steps", "clock", "run", "reset",
    ] {
        p.output_port = port.into();
        let mut out = [PortFrame::default(); 2];
        p.compile().unwrap().process_poly_block(&mut out);
        assert!(out
            .iter()
            .all(|f| f.channels == 1 && f.values[1..].iter().all(|v| *v == 0.0)));
    }
}
#[test]
fn clock_chaining_is_synchronous_and_independent_of_node_cable_order() {
    let mut p = seq("cv1", &[("tempo", 15.0), ("row1_1", 1.0), ("row1_2", 2.0)]);
    p.nodes.push(node("leader", "seq3", &[("tempo", 15.0)]));
    p.cables = vec![cable("leader", "clock", "s", "clock")];
    let events = [TimedModuleEvent {
        frame: 10,
        id: "leader".into(),
        action: ModuleAction::Clock,
    }];
    let a = render_audio_with_performance_events(&p, 0.004, &[], &events).unwrap();
    assert!(a.samples[..11].iter().all(|v| *v == 1.0));
    assert!(a.samples[11..].iter().all(|v| *v == 2.0));
    p.nodes.reverse();
    p.cables.reverse();
    let b = render_audio_with_performance_events(&p, 0.004, &[], &events).unwrap();
    assert_eq!(a, b);
}
#[test]
fn offline_performance_scheduling_preserves_ties_and_matches_single_sample_partition() {
    let p = seq(
        "cv1",
        &[
            ("tempo", 15.0),
            ("row1_1", 1.0),
            ("row1_2", 2.0),
            ("row1_3", 3.0),
        ],
    );
    let events = [
        event(2, ModuleAction::Clock),
        event(3, ModuleAction::Clock),
        event(5, ModuleAction::Reset),
        event(5, ModuleAction::Clock),
        event(13, ModuleAction::Clock),
    ];
    let rendered = render_audio_with_performance_events(&p, 0.002, &[], &events).unwrap();
    assert_eq!(
        rendered.samples,
        vec![1.0, 1.0, 2.0, 3.0, 3.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 2.0, 2.0, 2.0]
    );
    let mut e = p.compile().unwrap();
    let h = e.resolve_module_event("s").unwrap();
    let mut index = 0;
    let mut manual = Vec::new();
    for frame in 0..16 {
        while index < events.len() && events[index].frame == frame {
            assert!(e.handle_module_event(h, events[index].action));
            index += 1;
        }
        manual.push(one(&mut e));
    }
    assert_eq!(manual, rendered.samples);
    assert_eq!(
        render_audio_with_events(&p, 0.002, &[]).unwrap(),
        render_audio_with_performance_events(&p, 0.002, &[], &[]).unwrap()
    );
}
#[test]
fn invalid_event_traces_targets_parameters_and_versions_reject_before_render() {
    assert!(serde_json::from_str::<ModuleAction>(r#"{"type":"clock"}"#).is_err());
    assert!(serde_json::from_str::<ModuleAction>(r#"{"clock":null}"#).is_err());
    assert!(serde_json::from_str::<ModuleAction>(r#""unknown""#).is_err());
    assert!(serde_json::from_str::<TimedModuleEvent>(
        r#"{"frame":0,"id":"s","action":"clock","extra":1}"#
    )
    .is_err());
    let p = seq("cv1", &[]);
    for events in [
        vec![event(16, ModuleAction::Clock)],
        vec![event(2, ModuleAction::Clock), event(1, ModuleAction::Clock)],
        vec![event(0, ModuleAction::Clock); 65],
        vec![TimedModuleEvent {
            frame: 0,
            id: "missing".into(),
            action: ModuleAction::Clock,
        }],
        vec![TimedModuleEvent {
            frame: 0,
            id: "../bad".into(),
            action: ModuleAction::Clock,
        }],
    ] {
        assert!(render_audio_with_performance_events(&p, 0.002, &[], &events).is_err());
    }
    assert!(validate_module_events(
        &vec![event(0, ModuleAction::Clock); MAX_MODULE_EVENTS + 1],
        16
    )
    .is_err());
    assert!(validate_module_events(&[event(0, ModuleAction::Run)], 0).is_err());
    for (name, value) in [
        ("tempo", 14.0),
        ("tempo", 961.0),
        ("steps", 0.0),
        ("steps", 2.5),
        ("steps_amount", 1.1),
        ("tempo_amount", -0.1),
        ("running", 0.5),
        ("passthrough", 0.5),
        ("row3_8", 10.1),
        ("gate8", 0.5),
        ("row4_1", 0.0),
        ("gate9", 1.0),
    ] {
        assert!(seq("cv1", &[(name, value)]).validate().is_err(), "{name}");
    }
    let mut legacy = p;
    legacy.version = 1;
    assert!(legacy.validate().is_err());
}

#[test]
fn combined_midi_and_module_events_share_timing_without_bypassing_reset_guard() {
    let mut p = seq("cv1", &[("tempo", 15.0), ("row1_1", 1.0), ("row1_2", 2.0)]);
    p.sample_rate = 48000;
    p.nodes.push(node("m", "midi_cv", &[("channels", 1.0)]));
    p.cables.push(cable("m", "clock", "s", "clock"));
    let midi = [
        TimedMidiEvent {
            frame: 100,
            event: MidiEvent::Clock,
        },
        TimedMidiEvent {
            frame: 200,
            event: MidiEvent::Clock,
        },
    ];
    let modules = [
        event(99, ModuleAction::Clock),
        event(100, ModuleAction::Reset),
    ];
    let audio = render_audio_with_performance_events(&p, 0.01, &midi, &modules).unwrap();
    assert!(audio.samples[..99].iter().all(|v| *v == 1.0));
    assert_eq!(audio.samples[99], 2.0);
    assert!(audio.samples[100..201].iter().all(|v| *v == 1.0));
    assert!(audio.samples[201..].iter().all(|v| *v == 2.0));
}
