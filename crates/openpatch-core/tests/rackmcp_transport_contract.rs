//! Source-wrapper transport/publication contracts, distinct from DSP oracle qualification.
use openpatch_core::{
    runtime_parameter_descriptor, runtime_parameter_names, Engine, FixedPatternName, ModuleHandle,
    Node, Patch, PortFrame, RackMcpSequenceStatus, RackMcpTransport as Transport, SavedModuleState,
};
use serde_json::{json, Value};

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct CallbackAllocator;
thread_local! {
    static TRACKING: Cell<bool> = const { Cell::new(false) };
    static MEMORY_OPERATIONS: Cell<usize> = const { Cell::new(0) };
}

fn record() {
    let _ = TRACKING.try_with(|tracking| {
        if tracking.get() {
            let _ = MEMORY_OPERATIONS.try_with(|count| count.set(count.get() + 1));
        }
    });
}

unsafe impl GlobalAlloc for CallbackAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record();
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record();
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        record();
        unsafe { System.realloc(pointer, layout, size) }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        record();
        unsafe { System.dealloc(pointer, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CallbackAllocator = CallbackAllocator;

fn node(value: Value) -> Node {
    serde_json::from_value(value).unwrap()
}

fn patch() -> Patch {
    Patch {
        sample_rate: 8000,
        nodes: vec![node(json!({"id":"seq","kind":"rackmcp_sequencer",
        "rackmcp_sequence":{"data":{"patterns":[
            {"name":"A","length":4,"tracks":[{"steps":[
                {"step":0,"notes":[0]}, {"step":1,"notes":[12]}]}]},
            {"name":"B","length":3,"stepsPerBeat":2,"tracks":[{"steps":[
                {"step":0,"notes":[24]}, {"step":1,"notes":[36]}]}]}
        ]}}}))],
        output: Some("seq".into()),
        output_port: "pitch1".into(),
        ..Patch::new_v2()
    }
}

fn tick(engine: &mut Engine, frames: usize) -> Vec<PortFrame> {
    let mut output = vec![PortFrame::default(); frames];
    engine.process_poly_block(&mut output);
    output
}

fn handle(engine: &Engine) -> ModuleHandle {
    engine.resolve_module_state("seq").unwrap()
}

fn status(engine: &Engine) -> RackMcpSequenceStatus {
    engine.rackmcp_sequence_status(handle(engine)).unwrap()
}

fn request(engine: &mut Engine, request: Transport) {
    assert!(engine.rackmcp_sequence_transport(handle(engine), request));
}

#[test]
fn source_defaults_and_current_controls_with_64_sample_position_publication() {
    let mut engine = patch().compile().unwrap();
    let initial = status(&engine);
    assert_eq!(
        (initial.running, initial.bpm, initial.swing),
        (false, 120.0, 50.0)
    );
    assert_eq!(
        (
            initial.active_pattern,
            initial.pending_pattern,
            initial.step
        ),
        (0, None, 0)
    );
    assert_eq!(
        (initial.loop_count, initial.beat, initial.external_clock),
        (0, 0.0, false)
    );
    assert_eq!(
        (
            initial.selected_pattern,
            initial.pattern_count,
            initial.lane_count
        ),
        (0, 2, 0)
    );
    request(&mut engine, Transport::Play);
    assert!(status(&engine).running);
    tick(&mut engine, 63);
    assert_eq!(status(&engine).beat, 0.0);
    tick(&mut engine, 1);
    assert_eq!(status(&engine).beat, 63.0 / 4000.0);
    let published = status(&engine).beat;
    request(&mut engine, Transport::Stop);
    assert!(!status(&engine).running);
    assert_eq!(status(&engine).beat, published);
    tick(&mut engine, 64);
    assert_eq!(status(&engine).beat, 0.0); // source drops stopped fractional beat
    request(&mut engine, Transport::Toggle);
    assert!(status(&engine).running);
    request(&mut engine, Transport::Toggle);
    assert!(!status(&engine).running);
    assert!(serde_json::to_value(status(&engine))
        .unwrap()
        .get("selected_pattern")
        .is_some());
}

#[test]
fn select_is_persisted_immediately_but_now_waits_for_advance_and_wrap_waits_for_wrap() {
    for (now, advance_frames) in [(true, 1024), (false, 4032)] {
        let mut engine = patch().compile().unwrap();
        request(&mut engine, Transport::Select { pattern: 1, now });
        assert_eq!(
            (
                status(&engine).selected_pattern,
                status(&engine).pending_pattern
            ),
            (1, Some(1))
        );
        request(&mut engine, Transport::Play);
        assert_eq!(tick(&mut engine, 1)[0].channel(0), 0.0);
        tick(&mut engine, 63);
        assert_eq!(
            (
                status(&engine).active_pattern,
                status(&engine).pending_pattern
            ),
            (0, Some(1))
        );
        tick(&mut engine, advance_frames - 64);
        assert_eq!(
            (
                status(&engine).active_pattern,
                status(&engine).pending_pattern
            ),
            (1, None)
        );
        assert_eq!(status(&engine).step, if now { 1 } else { 0 });
    }
}

#[test]
fn queued_requests_coalesce_and_source_reset_seek_select_order_is_independent_of_call_order() {
    for reverse in [false, true] {
        let mut p = patch();
        p.output_port = "reset".into();
        let mut engine = p.compile().unwrap();
        let mut commands = vec![
            Transport::Reset,
            Transport::Seek { bar: 0.875 },
            Transport::Select {
                pattern: 1,
                now: false,
            },
        ];
        if reverse {
            commands.reverse();
        }
        for command in commands {
            request(&mut engine, command);
        }
        // Source applies reset, seek and requested pattern before engine.process:
        // reset lands B, then seek uses B's division/length and cancels reset pulse.
        assert_eq!(tick(&mut engine, 64)[0].channel(0), 0.0);
        let s = status(&engine);
        assert_eq!(
            (s.active_pattern, s.step, s.loop_count, s.beat),
            (1, 1, 2, 3.5)
        );
    }
    let mut engine = patch().compile().unwrap();
    request(&mut engine, Transport::Seek { bar: 99.0 });
    request(&mut engine, Transport::Seek { bar: 0.375 });
    request(
        &mut engine,
        Transport::Select {
            pattern: 1,
            now: true,
        },
    );
    request(
        &mut engine,
        Transport::Select {
            pattern: 0,
            now: false,
        },
    );
    request(&mut engine, Transport::Reset);
    request(&mut engine, Transport::Reset);
    tick(&mut engine, 64);
    let s = status(&engine);
    assert_eq!(
        (s.active_pattern, s.selected_pattern, s.pending_pattern),
        (0, 0, None)
    );
    assert_eq!((s.step, s.loop_count, s.beat), (2, 1, 1.5));
}

#[test]
fn fractional_seek_while_stopped_and_external_waits_for_an_edge() {
    let mut p = patch();
    p.nodes.push(node(
        json!({"id":"clock","kind":"poly_constant","params":{"v1":0}}),
    ));
    p.cables
        .push(serde_json::from_value(json!({"from":"clock","to":"seq","input":"clock"})).unwrap());
    let mut engine = p.compile().unwrap();
    request(&mut engine, Transport::Seek { bar: 0.390625 }); // 1.5625 beats floors to 1.5
    tick(&mut engine, 63);
    assert!(!status(&engine).external_clock);
    tick(&mut engine, 1);
    let s = status(&engine);
    assert_eq!(
        (s.step, s.loop_count, s.beat, s.external_clock),
        (2, 1, 1.5, true)
    );
    request(&mut engine, Transport::Play);
    tick(&mut engine, 128);
    assert_eq!((status(&engine).step, status(&engine).beat), (2, 1.5));
    request(&mut engine, Transport::Reset);
    tick(&mut engine, 64);
    assert_eq!(
        (
            status(&engine).step,
            status(&engine).loop_count,
            status(&engine).beat
        ),
        (0, 0, 0.0)
    );
}

#[test]
fn stop_does_not_synthesize_reset_or_change_the_current_step() {
    let mut p = patch();
    p.output_port = "reset".into();
    let mut engine = p.compile().unwrap();
    request(&mut engine, Transport::Play);
    tick(&mut engine, 1100);
    request(&mut engine, Transport::Stop);
    assert!(tick(&mut engine, 64)
        .iter()
        .all(|frame| frame.channel(0) == 0.0));
    assert_eq!((status(&engine).step, status(&engine).beat), (1, 0.25));
    request(&mut engine, Transport::Play);
    tick(&mut engine, 64);
    assert_eq!(status(&engine).step, 1);
}

#[test]
fn invalid_requests_and_stale_or_wrong_kind_handles_are_atomic() {
    let p = patch();
    let mut engine = p.compile().unwrap();
    request(
        &mut engine,
        Transport::Select {
            pattern: 1,
            now: false,
        },
    );
    request(&mut engine, Transport::Seek { bar: 0.375 });
    let before = status(&engine);
    for command in [
        Transport::Select {
            pattern: 2,
            now: true,
        },
        Transport::Select {
            pattern: u8::MAX,
            now: true,
        },
        Transport::Seek { bar: -1.0 },
        Transport::Seek { bar: 100000.001 },
        Transport::Seek { bar: f64::NAN },
        Transport::Seek { bar: f64::INFINITY },
    ] {
        assert!(!engine.rackmcp_sequence_transport(handle(&engine), command));
        assert_eq!(status(&engine), before);
    }
    let foreign = handle(&p.compile().unwrap());
    assert!(!engine.rackmcp_sequence_transport(foreign, Transport::Play));
    assert_eq!(engine.rackmcp_sequence_status(foreign), None);
    tick(&mut engine, 64);
    assert_eq!((status(&engine).step, status(&engine).beat), (2, 1.5));
    let mut other = p;
    other
        .nodes
        .push(node(json!({"id":"other","kind":"stock_vca"})));
    let mut engine = other.compile().unwrap();
    let wrong = engine.resolve_module_state("other").unwrap();
    assert_eq!(engine.rackmcp_sequence_status(wrong), None);
    assert!(!engine.rackmcp_sequence_transport(wrong, Transport::Play));
    request(&mut engine, Transport::Seek { bar: 100000.0 });
}

#[test]
fn adoption_preserves_requests_selected_and_publish_phase_but_reloads_changed_data_selection() {
    let p = patch();
    let mut donor = p.compile().unwrap();
    tick(&mut donor, 63);
    request(
        &mut donor,
        Transport::Select {
            pattern: 1,
            now: true,
        },
    );
    request(&mut donor, Transport::Seek { bar: 0.375 });
    request(&mut donor, Transport::Reset);
    request(&mut donor, Transport::Play);
    let mut same = p.compile().unwrap();
    same.adopt_state_from(&donor);
    assert_eq!(status(&same).selected_pattern, 1);
    assert_eq!(status(&same).pending_pattern, Some(1));
    assert!(!status(&same).running); // receiver controls survive DSP-only adoption
    tick(&mut same, 1);
    assert_eq!(
        (
            status(&same).active_pattern,
            status(&same).step,
            status(&same).beat
        ),
        (1, 0, 1.5)
    );
    assert_eq!(same.inherit_parameter_values(&donor, &[], &[]), 1);
    assert!(status(&same).running);
    let mut changed_patch = p;
    changed_patch.nodes[0]
        .rackmcp_sequence
        .as_mut()
        .unwrap()
        .data
        .seed = 19;
    let mut changed = changed_patch.compile().unwrap();
    changed.adopt_state_from(&same);
    assert_eq!(status(&changed).selected_pattern, 0);
    assert_eq!(status(&changed).active_pattern, 1); // publication held until next 64
    tick(&mut changed, 64);
    assert_eq!(status(&changed).pending_pattern, Some(0));
    assert!(same.rackmcp_sequence_data_matches(handle(&same), &donor, handle(&donor)));
    assert!(!changed.rackmcp_sequence_data_matches(handle(&changed), &same, handle(&same)));
}

#[test]
fn running_is_the_only_live_sequence_parameter_and_rejects_fractional_values() {
    assert_eq!(runtime_parameter_names("rackmcp_sequencer"), &["running"]);
    let descriptor = runtime_parameter_descriptor("rackmcp_sequencer", "running").unwrap();
    assert_eq!(
        (
            descriptor.default,
            descriptor.min,
            descriptor.max,
            descriptor.snap
        ),
        (0.0, 0.0, 1.0, true)
    );
    let mut engine = patch().compile().unwrap();
    let running = engine.resolve_parameter("seq", "running").unwrap();
    for invalid in [-1.0, 0.5, 1.1, f32::NAN] {
        assert!(!engine.set_parameter_value(running, invalid));
        assert_eq!(engine.parameter_value(running), Some(0.0));
    }
    assert!(engine.set_parameter_value(running, 1.0));
    assert!(status(&engine).running);
    request(&mut engine, Transport::Toggle);
    assert_eq!(engine.parameter_value(running), Some(0.0));
    assert!(engine.resolve_parameter("seq", "bpm").is_err());
    assert!(engine.resolve_parameter("seq", "swing").is_err());
}

fn automation(value: f32) -> Value {
    json!({"data":{"patterns":[{"name":"A","length":4}],
        "lanes":[{"moduleId":"42","paramId":0,"points":[{"bar":0,"value":value}]}]},
        "targets":[{"source_module_id":"42","source_param_id":0,"module":"vca","parameter":"level"}]})
}

#[test]
fn transport_neither_rearms_lane_cache_nor_takes_competing_target_ownership() {
    let mut p = patch();
    p.nodes[0].rackmcp_sequence = Some(Box::new(serde_json::from_value(automation(0.2)).unwrap()));
    p.nodes.push(node(
        json!({"id":"second","kind":"rackmcp_sequencer","params":{"running":1},
        "rackmcp_sequence":automation(0.7)}),
    ));
    p.nodes.push(node(json!({"id":"vca","kind":"stock_vca"})));
    let mut engine = p.compile().unwrap();
    let vca = engine.resolve_parameter("vca", "level").unwrap();
    tick(&mut engine, 32);
    assert_eq!(engine.parameter_value(vca), Some(0.7));
    request(&mut engine, Transport::Play); // A has never written: claiming would overwrite B.
    request(&mut engine, Transport::Reset);
    request(&mut engine, Transport::Seek { bar: 0.25 });
    request(
        &mut engine,
        Transport::Select {
            pattern: 0,
            now: true,
        },
    );
    tick(&mut engine, 32);
    assert_eq!(engine.parameter_value(vca), Some(0.7));
    assert!(engine.set_parameter_value(vca, 0.4));
    let second = engine.resolve_module_state("second").unwrap();
    for command in [
        Transport::Reset,
        Transport::Seek { bar: 0.5 },
        Transport::Select {
            pattern: 0,
            now: false,
        },
        Transport::Stop,
        Transport::Play,
    ] {
        assert!(engine.rackmcp_sequence_transport(second, command));
    }
    tick(&mut engine, 64);
    assert_eq!(engine.parameter_value(vca), Some(0.4));
}

#[test]
fn queued_selection_surviving_a_shrinking_load_is_sanitized_after_the_load_request() {
    let mut p = patch();
    p.nodes[0]
        .rackmcp_sequence
        .as_mut()
        .unwrap()
        .data
        .patterns
        .push(serde_json::from_value(json!({"name":"C","length":4})).unwrap());
    let mut donor = p.compile().unwrap();
    request(
        &mut donor,
        Transport::Select {
            pattern: 2,
            now: true,
        },
    );
    let mut receiving = patch();
    receiving.nodes[0]
        .rackmcp_sequence
        .as_mut()
        .unwrap()
        .data
        .selected_pattern = Some("B".into());
    let mut engine = receiving.compile().unwrap();
    engine.adopt_state_from(&donor);
    assert_eq!(status(&engine).selected_pattern, 1);
    tick(&mut engine, 64);
    assert_eq!(
        (
            status(&engine).active_pattern,
            status(&engine).pending_pattern
        ),
        (0, None)
    );
}

#[test]
fn fixed_selection_snapshots_write_typed_data_and_resolve_names_after_reordering() {
    let unicode = "節奏🎹";
    let name = FixedPatternName::new(unicode).unwrap();
    assert_eq!(name.as_str(), unicode);
    assert_eq!(
        serde_json::from_str::<FixedPatternName>(&serde_json::to_string(&name).unwrap()).unwrap(),
        name
    );
    for invalid in ["", "a\0b", "123456789012345678901234567890123"] {
        assert!(FixedPatternName::new(invalid).is_none());
    }
    let mut p = patch();
    let saved = SavedModuleState::RackMcpSelection(FixedPatternName::new("B").unwrap());
    let before_params = p.nodes[0].params.clone();
    saved.write_params(&mut p.nodes[0].params);
    assert_eq!(p.nodes[0].params, before_params);
    assert_eq!(saved.boolean(), None);
    assert_eq!(saved.parameter(), "");
    assert_eq!(saved.with_value(true), saved);
    saved.write_node(&mut p.nodes[0]);
    assert_eq!(
        p.nodes[0]
            .rackmcp_sequence
            .as_ref()
            .unwrap()
            .data
            .selected_pattern
            .as_deref(),
        Some("B")
    );
    assert_eq!(SavedModuleState::from_node(&p.nodes[0]), Some(saved));
    p.nodes[0]
        .rackmcp_sequence
        .as_mut()
        .unwrap()
        .data
        .patterns
        .swap(0, 1);
    let mut engine = p.compile().unwrap();
    let seq = handle(&engine);
    assert_eq!(engine.module_saved_state(seq), Some(saved));
    assert_eq!(status(&engine).selected_pattern, 0);
    assert!(engine.set_module_saved_state(
        seq,
        SavedModuleState::RackMcpSelection(FixedPatternName::new("A").unwrap())
    ));
    assert_eq!(status(&engine).selected_pattern, 1);
    let before = status(&engine);
    assert!(!engine.set_module_saved_state(
        seq,
        SavedModuleState::RackMcpSelection(FixedPatternName::new("missing").unwrap())
    ));
    assert_eq!(status(&engine), before);
}

#[test]
fn selection_only_overlay_preserves_pending_now_but_default_adoption_keeps_source_reload_behavior()
{
    let mut p = patch();
    p.nodes[0].params.insert("running".into(), 1.0);
    let mut donor = p.compile().unwrap();
    request(
        &mut donor,
        Transport::Select {
            pattern: 1,
            now: true,
        },
    );
    tick(&mut donor, 64); // queued request consumed; engine's pending-now must survive
    donor
        .module_saved_state(handle(&donor))
        .unwrap()
        .write_node(&mut p.nodes[0]);
    let mut preserved = p.compile().unwrap();
    let seq = handle(&preserved);
    let data = &p.nodes[0].rackmcp_sequence.as_ref().unwrap().data;
    assert_eq!(
        preserved.rackmcp_sequence_content_fingerprint(seq),
        Some(openpatch_core::rackmcp_sequence::content_fingerprint(data).unwrap())
    );
    assert!(preserved.rackmcp_sequence_content_matches(seq, &donor, handle(&donor)));
    assert!(!preserved.rackmcp_sequence_data_matches(seq, &donor, handle(&donor)));
    preserved.adopt_state_from_excluding_routes_preserving_sequences(&donor, &[], &[], &[seq]);
    let mut default = p.compile().unwrap();
    default.adopt_state_from(&donor);
    tick(&mut preserved, 960);
    tick(&mut default, 960);
    assert_eq!(status(&preserved).active_pattern, 1);
    assert_eq!(status(&default).active_pattern, 0);
    // Stale preserve handles cannot suppress source reload.
    let mut stale = p.compile().unwrap();
    stale.adopt_state_from_excluding_routes_preserving_sequences(&donor, &[], &[], &[seq]);
    tick(&mut stale, 960);
    assert_eq!(status(&stale).active_pattern, 0);
    // Content changes must reload despite the explicit preserve flag.
    p.nodes[0].rackmcp_sequence.as_mut().unwrap().data.seed += 1;
    let mut changed = p.compile().unwrap();
    let seq = handle(&changed);
    assert!(!changed.rackmcp_sequence_content_matches(seq, &donor, handle(&donor)));
    changed.adopt_state_from_excluding_routes_preserving_sequences(&donor, &[], &[], &[seq]);
    tick(&mut changed, 960);
    assert_eq!(status(&changed).active_pattern, 0);
}

fn two_pattern_automation(value: f32) -> Value {
    let mut state = automation(value);
    state["data"]["patterns"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name":"B","length":4}));
    state
}

#[test]
fn selection_overlay_preserves_lane_cache_and_explicit_reload_rearms_it() {
    let mut p = patch();
    p.nodes[0] = node(
        json!({"id":"seq","kind":"rackmcp_sequencer","params":{"running":1},
        "rackmcp_sequence":two_pattern_automation(0.2)}),
    );
    p.nodes.push(node(json!({"id":"vca","kind":"stock_vca"})));
    let mut donor = p.compile().unwrap();
    tick(&mut donor, 32);
    let vca = donor.resolve_parameter("vca", "level").unwrap();
    assert!(donor.set_parameter_value(vca, 0.4));
    request(
        &mut donor,
        Transport::Select {
            pattern: 1,
            now: true,
        },
    );
    donor
        .module_saved_state(handle(&donor))
        .unwrap()
        .write_node(&mut p.nodes[0]);
    let mut receiver = p.compile().unwrap();
    let seq = handle(&receiver);
    receiver.handoff_state_from_excluding_routes_preserving_sequences(&mut donor, &[], &[], &[seq]);
    receiver.inherit_parameter_values(&donor, &[], &[]);
    let vca = receiver.resolve_parameter("vca", "level").unwrap();
    tick(&mut receiver, 32);
    assert_eq!(receiver.parameter_value(vca), Some(0.4));
    assert!(receiver.reload_rackmcp_sequence_data(seq));
    tick(&mut receiver, 32);
    assert_eq!(receiver.parameter_value(vca), Some(0.2));
}

#[test]
fn selection_overlay_retains_competing_owner_and_explicit_reload_acquires_targets() {
    let mut p = patch();
    p.nodes[0] = node(
        json!({"id":"seq","kind":"rackmcp_sequencer","params":{"running":1},
        "rackmcp_sequence":two_pattern_automation(0.2)}),
    );
    p.nodes.push(node(
        json!({"id":"second","kind":"rackmcp_sequencer","params":{"running":1},
        "rackmcp_sequence":automation(0.7)}),
    ));
    p.nodes.push(node(json!({"id":"vca","kind":"stock_vca"})));
    let mut donor = p.compile().unwrap();
    tick(&mut donor, 32);
    request(
        &mut donor,
        Transport::Select {
            pattern: 1,
            now: false,
        },
    );
    donor
        .module_saved_state(handle(&donor))
        .unwrap()
        .write_node(&mut p.nodes[0]);
    let mut receiver = p.compile().unwrap();
    let seq = handle(&receiver);
    receiver.adopt_state_from_excluding_routes_preserving_sequences(&donor, &[], &[], &[seq]);
    receiver.inherit_parameter_values(&donor, &[], &[]);
    let vca = receiver.resolve_parameter("vca", "level").unwrap();
    tick(&mut receiver, 32);
    assert_eq!(receiver.parameter_value(vca), Some(0.7));
    assert!(receiver.reload_rackmcp_sequence_data(seq));
    tick(&mut receiver, 32);
    assert_eq!(receiver.parameter_value(vca), Some(0.2));
}

#[test]
fn fixed_selection_capture_restore_and_preserving_handoff_do_not_allocate_or_deallocate() {
    let p = patch();
    let mut donor = p.compile().unwrap();
    let mut receiver = p.compile().unwrap();
    let donor_seq = handle(&donor);
    let receiver_seq = handle(&receiver);
    let mut frames = [PortFrame::default(); 64];
    MEMORY_OPERATIONS.with(|count| count.set(0));
    TRACKING.with(|tracking| tracking.set(true));
    let mut valid = true;
    for _ in 0..8 {
        valid &= donor.rackmcp_sequence_transport(
            donor_seq,
            Transport::Select {
                pattern: 1,
                now: true,
            },
        );
        let saved = donor.module_saved_state(donor_seq).unwrap();
        valid &= donor.set_module_saved_state(donor_seq, saved);
        valid &= donor.rackmcp_sequence_status(donor_seq).is_some();
        donor.process_poly_block(&mut frames);
    }
    receiver.handoff_state_from_excluding_routes_preserving_sequences(
        &mut donor,
        &[],
        &[],
        &[receiver_seq],
    );
    receiver.process_poly_block(&mut frames);
    TRACKING.with(|tracking| tracking.set(false));
    assert!(valid);
    assert_eq!(MEMORY_OPERATIONS.with(Cell::get), 0);
}
