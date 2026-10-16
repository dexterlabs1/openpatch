//! Actual Engine integration of canonical ports and the native stock model.
use openpatch_core::{
    Engine, HostModuleSnapshot, HostPortLifecycle, InputLifecycleTransform,
    OutputLifecycleTransform, Patch, PortFrame,
};
use serde_json::json;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct TrackingAllocator;
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

unsafe impl GlobalAlloc for TrackingAllocator {
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
static ALLOCATOR: TrackingAllocator = TrackingAllocator;

fn patch() -> Patch {
    serde_json::from_value(json!({
        "version":2,"sample_rate":48000,
        "nodes":[
            {"id":"a","kind":"poly_constant","params":{"channels":1,"v1":2}},
            {"id":"b","kind":"poly_constant","params":{"channels":1,"v1":6}},
            {"id":"m","kind":"stock_mid_side"},
            {"id":"sink","kind":"mult"}
        ],
        "cables":[
            {"id":"a_m","from":"a","to":"m","input":"left"},
            {"id":"b_m","from":"b","to":"m","input":"left"},
            {"id":"m_sink","from":"m","output":"left","to":"sink","input":"signal"}
        ],"output":"sink","output_port":"out1"
    }))
    .unwrap()
}

fn snapshot(engine: &Engine, id: &str) -> HostModuleSnapshot {
    engine
        .host_module_snapshot(engine.resolve_module_state(id).unwrap())
        .unwrap()
}

fn delta(engine: &Engine, id: &str) -> HostPortLifecycle {
    let (_, _, input_presence, output_consumers, bypassed) = engine
        .host_port_layout(engine.resolve_module_state(id).unwrap())
        .unwrap();
    HostPortLifecycle {
        input_presence,
        output_consumers,
        bypassed,
        ..HostPortLifecycle::default()
    }
}

fn apply(engine: &mut Engine, id: &str, effect: &HostPortLifecycle) {
    let handle = engine.resolve_module_state(id).unwrap();
    assert!(engine.apply_host_port_lifecycle(handle, effect));
}

fn step(engine: &mut Engine) {
    engine.process_poly_block(&mut [PortFrame::default(); 1]);
}

#[test]
fn first_sample_reads_seeded_inputs_then_transfers_after_all_models() {
    let mut engine = patch().compile().unwrap();
    let initial = snapshot(&engine, "m");
    assert_eq!((initial.input_count, initial.output_count), (6, 4));
    assert_eq!(initial.inputs[1].channels, 1);
    assert_eq!(initial.inputs[1].values, [0.0; 16]);
    assert_eq!(snapshot(&engine, "sink").input_count, 1);
    assert_eq!(snapshot(&engine, "sink").output_count, 8);
    step(&mut engine);
    let first = snapshot(&engine, "m");
    assert_eq!(first.inputs[1].values[0], 8.0);
    assert_eq!(first.outputs[2].values[0], 0.0);
    assert_eq!(snapshot(&engine, "sink").inputs[0].values[0], 0.0);
    step(&mut engine);
    assert_eq!(snapshot(&engine, "m").outputs[2].values[0], 8.0);
    assert_eq!(snapshot(&engine, "sink").inputs[0].values[0], 8.0);
}

#[test]
fn partial_stack_removal_keeps_prior_combined_frame_until_end_transfer() {
    let original = patch();
    let mut engine = original.compile().unwrap();
    for _ in 0..3 {
        step(&mut engine);
    }
    let mut changed = original;
    changed
        .cables
        .retain(|cable| cable.id.as_deref() != Some("a_m"));
    let mut receiving = changed.compile().unwrap();
    receiving.adopt_state_from(&engine);
    assert_eq!(snapshot(&receiving, "m").inputs[1].values[0], 8.0);
    let effect = delta(&receiving, "m");
    apply(&mut receiving, "m", &effect);
    step(&mut receiving);
    assert_eq!(snapshot(&receiving, "m").outputs[2].values[0], 8.0);
    assert_eq!(snapshot(&receiving, "m").inputs[1].values[0], 6.0);
    step(&mut receiving);
    assert_eq!(snapshot(&receiving, "m").outputs[2].values[0], 6.0);
}

#[test]
fn native_input_port_observer_reads_retained_frame_used_by_processor() {
    let original = patch();
    let mut previous = original.compile().unwrap();
    previous.set_port_lights_enabled(true);
    for _ in 0..7 {
        step(&mut previous);
    }
    let mut changed = original;
    changed
        .cables
        .retain(|cable| cable.id.as_deref() != Some("a_m"));
    let mut receiving = changed.compile().unwrap();
    receiving.set_port_lights_enabled(true);
    receiving.adopt_state_from(&previous);
    let effect = delta(&receiving, "m");
    apply(&mut receiving, "m", &effect);
    let handle = receiving
        .resolve_port_light("m", openpatch_core::PortDirection::Input, "left")
        .unwrap();
    step(&mut receiving);
    let observed = receiving.port_light(handle).unwrap();
    let expected =
        openpatch_core::port_lights::step_port_light(Default::default(), true, 1, &[8.0], 48000);
    assert_eq!(observed, expected);
    assert_eq!(snapshot(&receiving, "m").inputs[1].values[0], 6.0);
}

#[test]
fn same_route_reconnect_clears_input_immediately_without_net_fallback_replay() {
    let patch = patch();
    let mut previous = patch.compile().unwrap();
    for _ in 0..3 {
        step(&mut previous);
    }
    let mut receiving = patch.compile().unwrap();
    receiving.adopt_state_from(&previous);
    let mut effect = delta(&receiving, "m");
    effect.inputs[1] =
        InputLifecycleTransform::disconnect().then(InputLifecycleTransform::connect());
    apply(&mut receiving, "m", &effect);
    assert_eq!(snapshot(&receiving, "m").inputs[1].channels, 1);
    assert_eq!(snapshot(&receiving, "m").inputs[1].values, [0.0; 16]);
    step(&mut receiving);
    assert_eq!(snapshot(&receiving, "m").outputs[2].values[0], 0.0);
    assert_eq!(snapshot(&receiving, "m").inputs[1].values[0], 8.0);
    step(&mut receiving);
    assert_eq!(snapshot(&receiving, "m").outputs[2].values[0], 8.0);
}

#[test]
fn producer_parameters_write_only_during_process_and_private_history_adopts() {
    let mut original = patch();
    original.nodes[0].params.insert("channels".into(), 16.0);
    let mut previous = original.compile().unwrap();
    step(&mut previous);
    let prior = snapshot(&previous, "a");
    assert_eq!(prior.outputs[0].channels, 16);
    assert_eq!(prior.outputs[0].values[15], 16.0);
    original.nodes[0].params.insert("channels".into(), 0.0);
    original.nodes[0].params.insert("v1".into(), 12.0);
    original.nodes.swap(0, 3); // Raw input identity cannot be an input-group index.
    let mut receiving = original.compile().unwrap();
    receiving.adopt_state_from(&previous);
    assert_eq!(snapshot(&receiving, "a").outputs, prior.outputs);
    step(&mut receiving);
    let current = snapshot(&receiving, "a");
    assert_eq!(current.outputs[0].channels, 1);
    assert_eq!(current.outputs[0].values, [0.0; 16]);
}

#[test]
fn disconnected_producer_writes_real_raw_slots_and_observation_never_connects() {
    let mut patch = patch();
    patch.cables.retain(|cable| cable.from != "a");
    let mut engine = patch.compile().unwrap();
    let before = snapshot(&engine, "a");
    assert_eq!(before.outputs[0].channels, 0);
    assert_eq!(before.output_consumers, 0);
    step(&mut engine);
    let after = snapshot(&engine, "a");
    assert_eq!(after.outputs[0].channels, 0);
    assert_eq!(after.outputs[0].values[0], 2.0);
    assert_eq!(after.outputs[0].values[1..], [0.0; 15]);
    assert_eq!(snapshot(&engine, "a"), after);
    assert_eq!(snapshot(&engine, "a").output_consumers, 0);
}

#[test]
fn bypass_edges_clear_active_prefix_and_reconnect_while_bypassed_retains_raw() {
    let original = patch();
    let mut engine = original.compile().unwrap();
    for _ in 0..3 {
        step(&mut engine);
    }
    // Encoder outputs have no consumer but contain real raw internal values.
    assert_eq!(snapshot(&engine, "m").outputs[0].values[0], 4.0);
    let mut changed = original;
    changed.nodes[2].bypassed = true;
    changed.cables.push(
        serde_json::from_value(json!({
            "id":"new_mid","from":"m","output":"mid","to":"sink","input":"signal"
        }))
        .unwrap(),
    );
    let mut receiving = changed.compile().unwrap();
    receiving.adopt_state_from(&engine);
    let mut effect = delta(&receiving, "m");
    // Bypass happens first while mid is disconnected, then a cable connects it.
    for port in 0..4 {
        effect.outputs[port] = OutputLifecycleTransform::bypass_edge();
    }
    effect.outputs[0] = effect.outputs[0].then(OutputLifecycleTransform::connect());
    apply(&mut receiving, "m", &effect);
    let state = snapshot(&receiving, "m");
    assert_eq!(state.outputs[0].channels, 1);
    assert_eq!(state.outputs[0].values[0], 4.0);
    assert_eq!(state.outputs[2].values[0].to_bits(), 0);
    step(&mut receiving);
    assert_eq!(snapshot(&receiving, "m").outputs[0], state.outputs[0]);
    assert_eq!(snapshot(&receiving, "sink").inputs[0].values[0], 4.0);
}

#[test]
fn invalid_context_and_stale_handles_reject_without_raw_mutation() {
    let mut engine = patch().compile().unwrap();
    step(&mut engine);
    let before = snapshot(&engine, "m");
    let mut effect = delta(&engine, "m");
    effect.inputs[1] = InputLifecycleTransform::disconnect();
    effect.output_consumers ^= 1;
    let handle = engine.resolve_module_state("m").unwrap();
    assert!(!engine.apply_host_port_lifecycle(handle, &effect));
    assert_eq!(snapshot(&engine, "m"), before);
    let other = patch().compile().unwrap();
    assert!(other.host_module_snapshot(handle).is_none());
}

#[test]
fn initialize_preserves_ports_and_both_adoption_paths_respect_incarnation_fences() {
    let patch = patch();
    let mut previous = patch.compile().unwrap();
    for _ in 0..3 {
        step(&mut previous);
    }
    let old_handle = previous.resolve_module_state("m").unwrap();
    let before = snapshot(&previous, "m");
    assert!(previous.initialize_module_state(old_handle));
    assert_eq!(snapshot(&previous, "m"), before);
    let mut copied = patch.compile().unwrap();
    let fresh = copied.resolve_module_state("m").unwrap();
    copied.adopt_state_from_excluding_routes(&previous, &[fresh], &[]);
    assert_eq!(snapshot(&copied, "m").inputs[1].values, [0.0; 16]);
    assert_eq!(snapshot(&copied, "m").outputs[0].values, [0.0; 16]);
    let mut moved = patch.compile().unwrap();
    let fresh = moved.resolve_module_state("m").unwrap();
    assert!(!moved.handoff_state_from_excluding_routes(&mut previous, &[fresh], &[]));
    assert_eq!(snapshot(&moved, "m").inputs[1].values, [0.0; 16]);
    assert_eq!(snapshot(&moved, "m").outputs[0].values, [0.0; 16]);
}

#[test]
fn actual_tap_consumes_temporarily_and_can_read_retained_bypass_storage() {
    let mut original = patch();
    let mut engine = original.compile().unwrap();
    for _ in 0..3 {
        step(&mut engine);
    }
    original.nodes[2].bypassed = true;
    let mut receiving = original.compile().unwrap();
    receiving.adopt_state_from(&engine);
    let mut effect = delta(&receiving, "m");
    for output in &mut effect.outputs[..4] {
        *output = OutputLifecycleTransform::bypass_edge();
    }
    apply(&mut receiving, "m", &effect);
    let tap = receiving.resolve_tap("m", "mid", 0).unwrap();
    let mut samples = [[openpatch_core::TapSample::default(); 1]; 1];
    receiving.process_audio_block_with_taps(&mut [[0.0; 2]; 1], &[Some(tap)], &mut samples);
    assert!(samples[0][0].active);
    assert_eq!(samples[0][0].value, 4.0);
    let after = snapshot(&receiving, "m");
    assert_eq!(after.outputs[0].channels, 0);
    assert_eq!(after.outputs[0].values[0], 4.0);
    assert_eq!(after.output_consumers, 4);
}

#[test]
fn canonical_producer_history_exists_before_first_native_model_is_added() {
    let mut original = patch();
    original.nodes.retain(|node| node.id != "m");
    original.cables.clear();
    let mut previous = original.compile().unwrap();
    step(&mut previous);
    let prior = snapshot(&previous, "a").outputs[0];
    assert_eq!(prior.channels, 0);
    assert_eq!(prior.values[0], 2.0);
    let changed = patch();
    let mut receiving = changed.compile().unwrap();
    receiving.adopt_state_from(&previous);
    assert_eq!(snapshot(&receiving, "a").outputs[0], prior);
    let mut effect = delta(&receiving, "a");
    effect.outputs[0] = OutputLifecycleTransform::connect();
    apply(&mut receiving, "a", &effect);
    assert_eq!(snapshot(&receiving, "a").outputs[0].channels, 1);
    assert_eq!(snapshot(&receiving, "a").outputs[0].values[0], 2.0);
}

#[test]
fn rate_change_fences_raw_history_and_old_mid_side_policy_is_preserved() {
    let mut original = patch();
    let mut previous = original.compile().unwrap();
    for _ in 0..3 {
        step(&mut previous);
    }
    original.sample_rate = 96000;
    let mut receiving = original.compile().unwrap();
    receiving.adopt_state_from(&previous);
    assert_eq!(snapshot(&receiving, "m").inputs[1].values, [0.0; 16]);
    assert_eq!(snapshot(&receiving, "m").outputs[0].values, [0.0; 16]);
    let old: Patch = serde_json::from_value(json!({
        "version":2,"sample_rate":48000,
        "nodes":[{"id":"left","kind":"constant","params":{"value":2}},
            {"id":"cv","kind":"constant","params":{"value":5}},
            {"id":"old","kind":"mid_side"}],
        "cables":[{"from":"left","to":"old","input":"left"},
            {"from":"cv","to":"old","input":"encode_cv"}],
        "output":"old","output_port":"side"
    }))
    .unwrap();
    let mut old = old.compile().unwrap();
    let mut frames = [PortFrame::default(); 3];
    old.process_poly_block(&mut frames);
    assert_eq!(frames[2].values[0], 0.5);
}

#[test]
fn first_processing_adoption_explicit_effects_and_snapshots_touch_no_heap() {
    let patch = patch();
    let mut previous = patch.compile().unwrap();
    let mut receiving = patch.compile().unwrap();
    let handle = receiving.resolve_module_state("m").unwrap();
    let mut effect = delta(&receiving, "m");
    effect.inputs[1] =
        InputLifecycleTransform::disconnect().then(InputLifecycleTransform::connect());
    let mut frames = [PortFrame::default(); 17];
    MEMORY_OPERATIONS.with(|count| count.set(0));
    TRACKING.with(|tracking| tracking.set(true));
    previous.process_poly_block(&mut frames);
    let moved = receiving.handoff_state_from(&mut previous);
    let accepted = receiving.apply_host_port_lifecycle(handle, &effect);
    receiving.process_poly_block(&mut frames);
    let observed = receiving.host_module_snapshot(handle);
    TRACKING.with(|tracking| tracking.set(false));
    assert!(!moved);
    assert!(accepted);
    assert!(observed.is_some());
    assert_eq!(MEMORY_OPERATIONS.with(Cell::get), 0);
}

#[test]
fn twenty_output_pulses_last_port_renders_and_has_real_canonical_storage() {
    let patch: Patch = serde_json::from_value(json!({
        "version":2,"sample_rate":48000,
        "nodes":[{"id":"p","kind":"pulses"},{"id":"sink","kind":"mult"}],
        "cables":[{"id":"last","from":"p","output":"gate10","to":"sink","input":"signal"}],
        "output":"p","output_port":"gate10"
    }))
    .unwrap();
    let mut engine = patch.compile().unwrap();
    let handle = engine.resolve_module_state("p").unwrap();
    let (_, count, _, consumers, _) = engine.host_port_layout(handle).unwrap();
    assert_eq!(count, 20);
    assert_eq!(consumers, 1_u32 << 19);
    let action = openpatch_core::ModuleAction::PulsePress(10);
    let control = engine.resolve_module_action("p", action).unwrap();
    assert!(engine.handle_module_event(control, action));
    let mut output = [PortFrame::default(); 1];
    engine.process_poly_block(&mut output);
    assert_eq!(output[0], PortFrame::mono(10.0));
    let state = engine.host_module_snapshot(handle).unwrap();
    assert_eq!(state.output_count, 20);
    assert_eq!(state.outputs[19].channels, 1);
    assert_eq!(state.outputs[19].values[0], 10.0);
    assert_eq!(state.outputs[19].values[1..], [0.0; 15]);
    assert_eq!(snapshot(&engine, "sink").inputs[0].values[0], 10.0);
    let effect = delta(&engine, "p");
    apply(&mut engine, "p", &effect);
    assert_eq!(engine.host_module_snapshot(handle).unwrap(), state);
    assert_eq!(
        engine.resolve_host_port(handle, openpatch_core::PortDirection::Output, "gate10"),
        Some(19)
    );
}
