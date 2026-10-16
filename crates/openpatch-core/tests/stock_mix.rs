//! Actual Engine tests for native Stock Mix, independent of the SDK oracle.
use openpatch_core::{
    catalog, Engine, HostModuleSnapshot, HostPortLifecycle, InputLifecycleTransform,
    ModuleParameterActions, OutputLifecycleTransform, Patch, PortFrame, StockMixSnapshot,
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
            {"id":"b","kind":"poly_constant","params":{"channels":3,"v1":4,"v2":6,"v3":8}},
            {"id":"m","kind":"stock_mix","params":{"level":1,"invert":0,"average":0}}
        ],
        "cables":[{"id":"a_m","from":"a","to":"m","input":"in1"},
            {"id":"b_m","from":"b","to":"m","input":"in2"}],
        "output":"m","output_port":"out"
    }))
    .unwrap()
}
fn step(engine: &mut Engine) -> PortFrame {
    let mut output = [PortFrame::default(); 1];
    engine.process_poly_block(&mut output);
    output[0]
}
fn ports(engine: &Engine) -> HostModuleSnapshot {
    engine
        .host_module_snapshot(engine.resolve_module_state("m").unwrap())
        .unwrap()
}
fn state(engine: &Engine) -> StockMixSnapshot {
    engine
        .stock_mix_snapshot(engine.resolve_module_state("m").unwrap())
        .unwrap()
}
fn delta(engine: &Engine) -> HostPortLifecycle {
    let (_, _, input_presence, output_consumers, bypassed) = engine
        .host_port_layout(engine.resolve_module_state("m").unwrap())
        .unwrap();
    HostPortLifecycle {
        input_presence,
        output_consumers,
        bypassed,
        ..Default::default()
    }
}
fn apply(engine: &mut Engine, effect: &HostPortLifecycle) {
    let handle = engine.resolve_module_state("m").unwrap();
    assert!(engine.apply_host_port_lifecycle(handle, effect));
}

#[test]
fn strict_catalog_flags_and_defaults_do_not_invent_lights_or_actions() {
    let c = catalog();
    let module = c["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|module| module["kind"] == "stock_mix")
        .unwrap();
    assert_eq!(
        module["inputs"],
        json!(["in1", "in2", "in3", "in4", "in5", "in6"])
    );
    assert_eq!(module["outputs"], json!(["out"]));
    assert_eq!(module["light_names"], json!([]));
    assert_eq!(module["bypass"]["routes"], json!([]));
    assert_eq!(module["min_patch_version"], 2);
    for name in ["invert", "average"] {
        assert_eq!(module["params"][name]["values"], json!([0, 1]));
        assert_eq!(module["params"][name]["initialize"], false);
        assert_eq!(module["params"][name]["randomization"], "disabled");
    }
    let mut original = patch();
    original.nodes[2].params.clear();
    let engine = original.compile().unwrap();
    assert_eq!(
        state(&engine),
        StockMixSnapshot {
            level: 1.0,
            invert: false,
            average: false
        }
    );
    assert!(engine.resolve_module_lights("m").is_err());
    assert!(engine.resolve_module_control("m").is_err());
    assert!(engine.resolve_module_event("m").is_err());
    for name in ["invert", "average"] {
        for invalid in [
            -1.0,
            0.25,
            0.5,
            1.5,
            f32::from_bits(1),
            -f32::from_bits(1),
            f32::NAN,
            f32::INFINITY,
        ] {
            let mut bad = original.clone();
            bad.nodes[2].params.insert(name.into(), invalid);
            assert!(bad.compile().is_err());
        }
    }
    for invalid in [-0.01, 1.01, -f32::from_bits(1), f32::NEG_INFINITY] {
        let mut bad = original.clone();
        bad.nodes[2].params.insert("level".into(), invalid);
        assert!(bad.compile().is_err());
    }
    original.nodes[2].params.insert("unknown".into(), 1.0);
    assert!(original.compile().is_err());
    let mut legacy = patch();
    legacy.version = 1;
    assert!(legacy.compile().is_err());
}

#[test]
fn retained_inputs_produce_raw_lane_mix_after_exactly_one_cable_frame() {
    let mut engine = patch().compile().unwrap();
    assert_eq!(ports(&engine).inputs[0].channels, 1);
    assert_eq!(ports(&engine).inputs[1].channels, 1);
    assert_eq!(step(&mut engine), PortFrame::mono(0.0));
    assert_eq!(ports(&engine).inputs[1].channels, 3);
    let frame = step(&mut engine);
    assert_eq!(frame.channels, 3);
    assert_eq!(&frame.values[..3], &[6.0, 6.0, 8.0]);
    assert_eq!(&frame.values[3..], &[0.0; 13]);
}

#[test]
fn connected_requested_zero_source_counts_as_mono_zero_in_average() {
    let mut patch = patch();
    patch.nodes[2].params.insert("average".into(), 1.0);
    patch.nodes.push(
        serde_json::from_value(json!({"id":"zero","kind":"poly_constant","params":{"channels":0}}))
            .unwrap(),
    );
    patch.cables.push(
        serde_json::from_value(json!({"id":"z_m","from":"zero","to":"m","input":"in3"})).unwrap(),
    );
    let mut engine = patch.compile().unwrap();
    step(&mut engine);
    let frame = step(&mut engine);
    assert_eq!(ports(&engine).inputs[2].channels, 1);
    assert_eq!(&frame.values[..3], &[2.0, 2.0, 8.0 * (1.0_f32 / 3.0)]);
}

#[test]
fn initialize_and_seeded_randomize_preserve_durable_flags_and_ports() {
    let mut original = patch();
    original.nodes[2].params.extend([
        ("level".into(), 0.2),
        ("invert".into(), 1.0),
        ("average".into(), 1.0),
    ]);
    let mut engine = original.compile().unwrap();
    for _ in 0..3 {
        step(&mut engine);
    }
    let before = ports(&engine);
    let actions = ModuleParameterActions::new(original.sample_rate);
    actions.initialize(&mut original.nodes[2]).unwrap();
    assert_eq!(original.nodes[2].params["level"], 1.0);
    assert_eq!(original.nodes[2].params["invert"], 1.0);
    assert_eq!(original.nodes[2].params["average"], 1.0);
    let mut receiving = original.compile().unwrap();
    receiving.adopt_state_from(&engine);
    let handle = receiving.resolve_module_state("m").unwrap();
    assert!(receiving.initialize_module_state(handle));
    assert_eq!(ports(&receiving), before);
    assert_eq!(
        state(&receiving),
        StockMixSnapshot {
            level: 1.0,
            invert: true,
            average: true
        }
    );
    let mut same = original.nodes[2].clone();
    actions.randomize(&mut original.nodes[2], 1234).unwrap();
    actions.randomize(&mut same, 1234).unwrap();
    assert_eq!(original.nodes[2].params, same.params);
    assert_eq!(same.params["invert"], 1.0);
    assert_eq!(same.params["average"], 1.0);
    assert!((0.0..=1.0).contains(&same.params["level"]));
}

#[test]
fn legacy_loader_and_serializer_access_actual_model_and_reject_stale_or_wrong_kind() {
    let mut engine = patch().compile().unwrap();
    let handle = engine.resolve_module_state("m").unwrap();
    let source = engine.resolve_module_state("a").unwrap();
    assert!(engine.load_stock_mix_legacy_data(handle, &json!({"invert":true,"average":true})));
    assert!(state(&engine).invert && state(&engine).average);
    assert!(engine.load_stock_mix_legacy_data(handle, &json!({"average":1})));
    assert_eq!(
        engine.stock_mix_legacy_data(handle).unwrap(),
        json!({"invert":true,"average":false})
    );
    assert!(engine.load_stock_mix_legacy_data(handle, &json!([false, false])));
    assert_eq!(
        engine.stock_mix_legacy_data(handle).unwrap(),
        json!({"invert":true,"average":false})
    );
    assert!(!engine.load_stock_mix_legacy_data(source, &json!({"invert":true})));
    assert!(engine.stock_mix_snapshot(source).is_none());
    assert!(engine.stock_mix_legacy_data(source).is_none());
    let mut other = patch().compile().unwrap();
    assert!(!other.load_stock_mix_legacy_data(handle, &json!({"invert":false})));
    assert!(other.stock_mix_snapshot(handle).is_none());
    assert!(other.stock_mix_legacy_data(handle).is_none());
    step(&mut engine);
    assert_eq!(step(&mut engine).values[0], -6.0);
}

#[test]
fn receiving_controls_win_copy_and_move_adoption_without_touching_retained_ports() {
    let original = patch();
    let mut previous = original.compile().unwrap();
    for _ in 0..3 {
        step(&mut previous);
    }
    let prior = ports(&previous);
    let mut changed = original.clone();
    changed.nodes[2].params.extend([
        ("level".into(), 0.5),
        ("invert".into(), 1.0),
        ("average".into(), 1.0),
    ]);
    changed.nodes.swap(0, 2);
    let mut receiving = changed.compile().unwrap();
    receiving.adopt_state_from(&previous);
    assert_eq!(ports(&receiving), prior);
    assert_eq!(
        state(&receiving),
        StockMixSnapshot {
            level: 0.5,
            invert: true,
            average: true
        }
    );
    assert_eq!(&step(&mut receiving).values[..3], &[-1.5, -1.5, -2.0]);
    let mut moved = changed.compile().unwrap();
    assert!(!moved.handoff_state_from(&mut previous));
    assert_eq!(ports(&moved), prior);
    assert_eq!(
        state(&moved),
        StockMixSnapshot {
            level: 0.5,
            invert: true,
            average: true
        }
    );
}

#[test]
fn partial_stack_removal_retains_previous_sum_until_transfer_and_reconnect_clears() {
    let mut original = patch();
    original.cables[1].input = "in1".into();
    let mut previous = original.compile().unwrap();
    for _ in 0..3 {
        step(&mut previous);
    }
    let mut changed = original;
    changed
        .cables
        .retain(|cable| cable.id.as_deref() != Some("a_m"));
    let mut receiving = changed.compile().unwrap();
    receiving.adopt_state_from(&previous);
    let effect = delta(&receiving);
    apply(&mut receiving, &effect);
    assert_eq!(&step(&mut receiving).values[..3], &[6.0, 8.0, 10.0]);
    assert_eq!(&step(&mut receiving).values[..3], &[4.0, 6.0, 8.0]);
    let mut reconnect = delta(&receiving);
    reconnect.inputs[0] =
        InputLifecycleTransform::disconnect().then(InputLifecycleTransform::connect());
    apply(&mut receiving, &reconnect);
    assert_eq!(ports(&receiving).inputs[0].values, [0.0; 16]);
    assert_eq!(step(&mut receiving), PortFrame::mono(0.0));
    assert_eq!(&step(&mut receiving).values[..3], &[4.0, 6.0, 8.0]);
}

#[test]
fn bypass_order_can_expose_disconnected_raw_history_without_processing() {
    let mut original = patch();
    original.output = None;
    original.output_port = "out".into();
    let mut previous = original.compile().unwrap();
    for _ in 0..3 {
        step(&mut previous);
    }
    assert_eq!(ports(&previous).outputs[0].channels, 0);
    assert_eq!(ports(&previous).outputs[0].values[0], 6.0);
    let mut changed = original;
    changed.nodes[2].bypassed = true;
    changed.output = Some("m".into());
    let mut receiving = changed.compile().unwrap();
    receiving.adopt_state_from(&previous);
    let mut effect = delta(&receiving);
    effect.outputs[0] =
        OutputLifecycleTransform::bypass_edge().then(OutputLifecycleTransform::connect());
    apply(&mut receiving, &effect);
    assert_eq!(ports(&receiving).outputs[0].channels, 1);
    assert_eq!(ports(&receiving).outputs[0].values[0], 6.0);
    assert_eq!(step(&mut receiving), PortFrame::mono(6.0));
}

#[test]
fn old_mix6_broadcast_behavior_remains_independent() {
    let mut old = patch();
    old.nodes[2].kind = "mix6".into();
    let mut engine = old.compile().unwrap();
    step(&mut engine);
    let frame = step(&mut engine);
    assert_eq!(&frame.values[..3], &[6.0, 8.0, 10.0]);
}

#[test]
fn node_permutation_and_callback_partitioning_do_not_change_native_schedule() {
    let mut original = patch();
    original.nodes[2].params.insert("invert".into(), 1.0);
    let mut first = original.compile().unwrap();
    original.nodes.reverse();
    let mut reordered = original.compile().unwrap();
    let mut expected = [PortFrame::default(); 32];
    let mut actual = [PortFrame::default(); 32];
    first.process_poly_block(&mut expected);
    let mut cursor = 0;
    for length in [0, 1, 3, 7, 21] {
        reordered.process_poly_block(&mut actual[cursor..cursor + length]);
        cursor += length;
    }
    for (actual, expected) in actual.iter().zip(&expected) {
        assert_eq!(actual.channels, expected.channels);
        assert_eq!(
            actual.values.map(f32::to_bits),
            expected.values.map(f32::to_bits)
        );
    }
    let first = ports(&first);
    let reordered = ports(&reordered);
    for (actual, expected) in reordered
        .inputs
        .iter()
        .chain(&reordered.outputs)
        .zip(first.inputs.iter().chain(&first.outputs))
    {
        assert_eq!(actual.channels, expected.channels);
        assert_eq!(
            actual.values.map(f32::to_bits),
            expected.values.map(f32::to_bits)
        );
    }
}

#[test]
fn temporary_tap_consumes_real_output_without_persisting_a_consumer() {
    let mut original = patch();
    original.output = None;
    let mut engine = original.compile().unwrap();
    for _ in 0..3 {
        step(&mut engine);
    }
    assert_eq!(ports(&engine).outputs[0].channels, 0);
    let tap = engine.resolve_tap("m", "out", 2).unwrap();
    let mut observed = [[openpatch_core::TapSample::default(); 1]; 1];
    engine.process_audio_block_with_taps(&mut [[0.0; 2]; 1], &[Some(tap)], &mut observed);
    assert!(observed[0][0].active);
    assert_eq!(observed[0][0].value, 8.0);
    assert_eq!(ports(&engine).outputs[0].channels, 0);
    assert_eq!(ports(&engine).output_consumers, 0);
    assert_eq!(ports(&engine).outputs[0].values[2], 8.0);
}

#[test]
fn output_retirement_happens_after_quartet_stores_on_actual_engine() {
    let mut original = patch();
    original.nodes[0].params.insert("channels".into(), 16.0);
    for lane in 1..=16 {
        original.nodes[0].params.insert(format!("v{lane}"), 2.0);
    }
    original.cables.retain(|cable| cable.from == "a");
    let mut previous = original.compile().unwrap();
    for _ in 0..3 {
        step(&mut previous);
    }
    assert_eq!(ports(&previous).outputs[0].channels, 16);
    original.cables.clear();
    original.nodes[2].params.insert("invert".into(), 1.0);
    let mut receiving = original.compile().unwrap();
    receiving.adopt_state_from(&previous);
    let mut effect = delta(&receiving);
    effect.inputs[0] = InputLifecycleTransform::disconnect();
    apply(&mut receiving, &effect);
    assert_eq!(ports(&receiving).outputs[0].values, [2.0; 16]);
    let frame = step(&mut receiving);
    assert_eq!(frame.channels, 1);
    assert_eq!(frame.values[0].to_bits(), (-0.0_f32).to_bits());
    assert!(ports(&receiving).outputs[0].values[1..]
        .iter()
        .all(|value| value.to_bits() == 0));
}

#[test]
fn first_processing_adoption_loader_and_observation_touch_no_heap() {
    let patch = patch();
    let mut previous = patch.compile().unwrap();
    let mut receiving = patch.compile().unwrap();
    let handle = receiving.resolve_module_state("m").unwrap();
    let data = json!({"invert":true});
    let mut frames = [PortFrame::default(); 17];
    MEMORY_OPERATIONS.with(|count| count.set(0));
    TRACKING.with(|tracking| tracking.set(true));
    previous.process_poly_block(&mut frames);
    let moved = receiving.handoff_state_from(&mut previous);
    let loaded = receiving.load_stock_mix_legacy_data(handle, &data);
    receiving.process_poly_block(&mut frames);
    let observation = receiving.stock_mix_snapshot(handle);
    let raw = receiving.host_module_snapshot(handle);
    TRACKING.with(|tracking| tracking.set(false));
    assert!(!moved);
    assert!(loaded && observation.is_some() && raw.is_some());
    assert_eq!(MEMORY_OPERATIONS.with(Cell::get), 0);
}
