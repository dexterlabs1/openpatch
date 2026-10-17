//! Host-facing Stock Scope regressions. Full acquisition parity belongs to the
//! independently frozen genuine-SDK reference, not to these Rust expectations.
use crate::{catalog, Engine, Patch, PortFrame, StockScopeDiagnosticBits};
use serde_json::json;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct CountingAllocator;
thread_local! {
    static TRACKING: Cell<bool> = const { Cell::new(false) };
    static ALLOCS: Cell<usize> = const { Cell::new(0) };
    static DEALLOCS: Cell<usize> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = TRACKING.try_with(|tracking| {
            if tracking.get() {
                let _ = ALLOCS.try_with(|count| count.set(count.get() + 1));
            }
        });
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let _ = TRACKING.try_with(|tracking| {
            if tracking.get() {
                let _ = ALLOCS.try_with(|count| count.set(count.get() + 1));
            }
        });
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let _ = TRACKING.try_with(|tracking| {
            if tracking.get() {
                let _ = ALLOCS.try_with(|count| count.set(count.get() + 1));
            }
        });
        unsafe { System.realloc(ptr, layout, size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let _ = TRACKING.try_with(|tracking| {
            if tracking.get() {
                let _ = DEALLOCS.try_with(|count| count.set(count.get() + 1));
            }
        });
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn patch(rate: u32) -> Patch {
    serde_json::from_value(json!({
        "version":2,"sample_rate":rate,
        "nodes":[
            {"id":"x_source","kind":"poly_constant","params":{"channels":16,"v1":2,"v16":-8}},
            {"id":"y_source","kind":"poly_constant","params":{"channels":7,"v1":-3,"v7":6}},
            {"id":"y_alternate","kind":"poly_constant","params":{"channels":3,"v1":4}},
            {"id":"scope","kind":"stock_scope","params":{}},
            {"id":"y_sink","kind":"mult"}
        ],
        "cables":[
            {"id":"x_in","from":"x_source","to":"scope","input":"x"},
            {"id":"y_in","from":"y_source","to":"scope","input":"y"},
            {"id":"y_out","from":"scope","output":"y","to":"y_sink","input":"signal"}
        ],
        "output":"scope","output_port":"x"
    }))
    .unwrap()
}

fn frames(engine: &mut Engine, count: usize) {
    let mut output = [PortFrame::default(); 300];
    engine.process_poly_block(&mut output[..count]);
}

fn source_frame(channels: u8, first: f32, last: f32) -> PortFrame {
    let mut frame = PortFrame {
        channels,
        ..PortFrame::default()
    };
    if channels > 0 {
        frame.values[0] = first;
        frame.values[usize::from(channels - 1)] = last;
    }
    frame
}

fn state(engine: &Engine) -> StockScopeDiagnosticBits {
    let scope = engine.resolve_module_state("scope").unwrap();
    engine.stock_scope_diagnostic_bits(scope).unwrap()
}

#[test]
fn catalog_and_normal_compile_keep_stock_scope_separate_and_enforce_authored_controls() {
    let modules = catalog()["modules"].as_array().unwrap().clone();
    let stock = modules
        .iter()
        .find(|module| module["kind"] == "stock_scope")
        .unwrap();
    assert!(modules.iter().any(|module| module["kind"] == "scope"));
    assert_eq!(stock["inputs"], json!(["x", "y", "trigger"]));
    assert_eq!(stock["outputs"], json!(["x", "y"]));
    assert_eq!(stock["light_names"], json!(["xy", "trigger"]));
    assert_eq!(stock["model"], "functional");

    for (key, value) in [
        ("x_scale", -1.0),
        ("x_scale", 0.5),
        ("x_scale", 9.0),
        ("x_offset", 10.01),
        ("y_scale", 7.5),
        ("y_offset", -10.01),
        ("time", -6.0),
        ("time", 8.0),
        ("xy", -0.25),
        ("xy", 0.5),
        ("threshold", 10.01),
        ("trigger_disabled", 0.5),
    ] {
        let mut invalid = patch(48_000);
        invalid.nodes[3].params.insert(key.into(), value);
        assert!(
            invalid.compile().is_err(),
            "{key}={value} must reject on normal compile"
        );
    }
    for (key, value) in [
        ("x_scale", 8.0),
        ("y_scale", 0.0),
        ("x_offset", -10.0),
        ("y_offset", 10.0),
        ("time", -50.0_f32.log2()),
        ("time", -0.005_f32.log2()),
        ("xy", 1.0),
        ("threshold", -10.0),
        ("trigger_disabled", 0.0),
    ] {
        let mut valid = patch(48_000);
        valid.nodes[3].params.insert(key.into(), value);
        assert!(valid.compile().is_ok(), "{key}={value} must compile");
    }

    let mut engine = patch(48_000).compile().unwrap();
    let scope = engine.resolve_module_state("scope").unwrap();
    for (index, raw) in [(0, -1.25), (4, -6.0), (5, -0.25), (6, f32::from_bits(1))] {
        assert!(engine.set_stock_scope_parameter_raw(scope, index, raw));
        assert_eq!(state(&engine).parameter_bits[index], raw.to_bits());
    }
    let before = state(&engine).parameter_bits;
    for (index, raw) in [(4, -7.0), (4, 9.0), (2, f32::NAN), (8, 0.0)] {
        assert!(!engine.set_stock_scope_parameter_raw(scope, index, raw));
    }
    assert_eq!(state(&engine).parameter_bits, before);
}

#[test]
fn independent_sixteen_and_seven_lane_inputs_copy_only_connected_output_widths() {
    let mut engine = patch(48_000).compile().unwrap();
    let scope = engine.resolve_module_state("scope").unwrap();
    let x = engine.resolve_module_state("x_source").unwrap();
    let y = engine.resolve_module_state("y_source").unwrap();
    assert!(engine.set_host_reference_source(x, source_frame(16, 2.0, -8.0)));
    assert!(engine.set_host_reference_source(y, source_frame(7, -3.0, 6.0)));
    frames(&mut engine, 3);
    let raw = engine.host_module_snapshot(scope).unwrap();
    assert_eq!((raw.inputs[0].channels, raw.inputs[1].channels), (16, 7));
    assert_eq!((raw.outputs[0].channels, raw.outputs[1].channels), (16, 7));
    assert_eq!(raw.outputs[0].values[15].to_bits(), (-8.0_f32).to_bits());
    assert_eq!(raw.outputs[1].values[6].to_bits(), 6.0_f32.to_bits());
    assert_eq!(
        (state(&engine).channels_x, state(&engine).channels_y),
        (16, 7)
    );
    assert_eq!(
        engine
            .module_scope(scope)
            .unwrap()
            .stock
            .unwrap()
            .sample_rate,
        48_000
    );

    let mut unconsumed = patch(48_000);
    unconsumed.cables.retain(|cable| cable.id != "y_out");
    let mut engine = unconsumed.compile().unwrap();
    let scope = engine.resolve_module_state("scope").unwrap();
    frames(&mut engine, 3);
    let raw = engine.host_module_snapshot(scope).unwrap();
    assert_eq!(raw.inputs[1].channels, 7);
    assert_eq!(raw.outputs[1].channels, 0);
    assert_eq!(state(&engine).channels_y, 7);
}

#[test]
fn initialize_clears_only_completed_bins_and_retains_partial_counter_and_lights() {
    let mut engine = patch(48_000).compile().unwrap();
    let scope = engine.resolve_module_state("scope").unwrap();
    assert!(engine.set_stock_scope_parameter_raw(scope, 4, 8.0));
    frames(&mut engine, 3);
    assert!(engine.set_stock_scope_parameter_raw(scope, 4, 5.0));
    frames(&mut engine, 2);
    let before = state(&engine);
    assert!(before.buffer_index > 0);
    assert!(before.frame_index > 0);
    assert_ne!(
        before.current_points[0][0].min_bits,
        f32::INFINITY.to_bits()
    );
    assert_ne!(
        before.point_buffer[0][0][0].min_bits,
        f32::INFINITY.to_bits()
    );
    assert_eq!(before.light_bits[1], 0.0_f32.to_bits());
    assert!(engine.initialize_module_state(scope));
    let after = state(&engine);
    assert!(after.point_buffer.iter().flatten().flatten().all(|point| {
        point.min_bits == f32::INFINITY.to_bits() && point.max_bits == f32::NEG_INFINITY.to_bits()
    }));
    assert_eq!(after.current_points, before.current_points);
    assert_eq!(after.buffer_index, before.buffer_index);
    assert_eq!(after.frame_index, before.frame_index);
    assert_eq!(after.triggers, before.triggers);
    assert_eq!(after.light_bits, before.light_bits);
    assert_eq!(after.parameter_bits, before.parameter_bits);
}

#[test]
fn initialize_keeps_nontrivial_waiting_detector_but_recreation_cools_every_history() {
    let mut engine = patch(48_000).compile().unwrap();
    let scope = engine.resolve_module_state("scope").unwrap();
    let x = engine.resolve_module_state("x_source").unwrap();
    assert!(engine.set_stock_scope_parameter_raw(scope, 4, 8.0));
    assert!(engine.set_stock_scope_parameter_raw(scope, 7, 0.0));
    frames(&mut engine, 260);
    assert_eq!(state(&engine).buffer_index, 256);
    assert_eq!(state(&engine).triggers[0], 1);
    assert!(engine.set_host_reference_source(x, source_frame(16, -2.0, -8.0)));
    frames(&mut engine, 2);
    let waiting = state(&engine);
    assert_eq!(waiting.triggers[0], 0);
    assert_eq!(waiting.light_bits[1], 1.0_f32.to_bits());
    assert!(engine.initialize_module_state(scope));
    let initialized = state(&engine);
    assert_eq!(initialized.triggers, waiting.triggers);
    assert_eq!(initialized.buffer_index, waiting.buffer_index);
    assert_eq!(initialized.light_bits, waiting.light_bits);

    assert!(engine.reset_module_state(scope));
    let cold = state(&engine);
    assert_eq!(cold.triggers, [2; 16]);
    assert_eq!(
        (
            cold.channels_x,
            cold.channels_y,
            cold.buffer_index,
            cold.frame_index
        ),
        (0, 0, 0, 0)
    );
    assert_eq!(cold.light_bits, [0; 2]);
    assert_eq!(cold.parameter_bits, waiting.parameter_bits);
    assert!(cold.point_buffer.iter().flatten().flatten().all(|point| {
        point.min_bits == f32::INFINITY.to_bits() && point.max_bits == f32::NEG_INFINITY.to_bits()
    }));
}

#[test]
fn receiving_controls_win_same_identity_route_adoption_while_acquisition_and_bypass_hold() {
    let original = patch(48_000);
    let mut old = original.compile().unwrap();
    let old_scope = old.resolve_module_state("scope").unwrap();
    assert!(old.set_stock_scope_parameter_raw(old_scope, 4, 8.0));
    frames(&mut old, 5);
    let before = state(&old);

    let mut changed = patch(48_000);
    changed.nodes[3].params.insert("time".into(), 2.0);
    changed.nodes[3].params.insert("xy".into(), 1.0);
    changed
        .cables
        .iter_mut()
        .find(|cable| cable.id == "y_in")
        .unwrap()
        .from = "y_alternate".into();
    let mut receiving = changed.compile().unwrap();
    receiving.adopt_state_from(&old);
    let adopted = state(&receiving);
    assert_eq!(adopted.point_buffer, before.point_buffer);
    assert_eq!(adopted.current_points, before.current_points);
    assert_eq!(adopted.triggers, before.triggers);
    assert_eq!(
        (adopted.buffer_index, adopted.frame_index),
        (before.buffer_index, before.frame_index)
    );
    assert_eq!(adopted.light_bits, before.light_bits);
    assert_eq!(adopted.parameter_bits[4], 2.0_f32.to_bits());
    assert_eq!(adopted.parameter_bits[5], 1.0_f32.to_bits());

    let mut bypass_patch = changed;
    bypass_patch.nodes[3].bypassed = true;
    let mut bypassed = bypass_patch.compile().unwrap();
    bypassed.adopt_state_from(&receiving);
    let held = state(&bypassed);
    frames(&mut bypassed, 4);
    assert_eq!(state(&bypassed), held);
    let scope = bypassed.resolve_module_state("scope").unwrap();
    let output = bypassed.host_module_snapshot(scope).unwrap().outputs[0];
    assert!(output.values[..usize::from(output.channels)]
        .iter()
        .all(|value| *value == 0.0));
}

#[test]
fn actual_rate_event_preserves_acquisition_and_host_storage_and_rejects_unsupported_graphs() {
    let mut engine = patch(48_000).compile().unwrap();
    let scope = engine.resolve_module_state("scope").unwrap();
    let x = engine.resolve_module_state("x_source").unwrap();
    frames(&mut engine, 5);
    let before = state(&engine);
    let raw = engine.host_module_snapshot(scope).unwrap();
    let processed = engine.port_light_processed_frames();
    assert!(!engine.stock_scope_sample_rate_event(0));
    assert_eq!(state(&engine), before);
    assert!(engine.stock_scope_sample_rate_event(96_000));
    let after = state(&engine);
    assert_eq!(after.point_buffer, before.point_buffer);
    assert_eq!(after.current_points, before.current_points);
    assert_eq!(after.triggers, before.triggers);
    assert_eq!(
        (after.buffer_index, after.frame_index),
        (before.buffer_index, before.frame_index)
    );
    assert_eq!(after.parameter_bits, before.parameter_bits);
    assert_eq!(after.light_bits, before.light_bits);
    assert_eq!(after.sample_rate, 96_000);
    assert_eq!(after.sample_time_bits, (1.0_f32 / 96_000.0).to_bits());
    assert_eq!(engine.host_module_snapshot(scope).unwrap(), raw);
    assert_eq!(engine.port_light_processed_frames(), processed);
    assert_eq!(engine.get_host_reference_source(x).unwrap().channels, 16);
    let metadata = engine.module_scope(scope).unwrap().stock.unwrap();
    assert_eq!(metadata.sample_rate, 96_000);
    assert_eq!(metadata.sample_time_bits, after.sample_time_bits);

    let mut mixed = patch(48_000);
    mixed
        .nodes
        .push(serde_json::from_value(json!({"id":"noise","kind":"stock_noise"})).unwrap());
    let mut mixed = mixed.compile().unwrap();
    let mixed_before = state(&mixed);
    assert!(!mixed.stock_scope_sample_rate_event(96_000));
    assert_eq!(state(&mixed), mixed_before);
    assert_eq!(mixed.host_reference_timing().unwrap().0, 48_000);

    let mut doubled = patch(48_000);
    doubled
        .nodes
        .push(serde_json::from_value(json!({"id":"second","kind":"stock_scope"})).unwrap());
    let mut doubled = doubled.compile().unwrap();
    let double_before = state(&doubled);
    assert!(!doubled.stock_scope_sample_rate_event(96_000));
    assert_eq!(state(&doubled), double_before);
    assert_eq!(doubled.host_reference_timing().unwrap().0, 48_000);

    let stale = mixed.resolve_module_state("scope").unwrap();
    assert!(engine.stock_scope_diagnostic_bits(stale).is_none());
    assert!(!engine.set_stock_scope_parameter_raw(stale, 4, 2.0));
}

#[test]
fn first_and_steady_callbacks_adoption_bypass_and_initialize_use_no_heap_activity() {
    let graph = patch(48_000);
    let mut old = graph.compile().unwrap();
    let mut receiving = graph.compile().unwrap();
    let mut bypass_graph = graph;
    bypass_graph.nodes[3].bypassed = true;
    let mut bypassed = bypass_graph.compile().unwrap();
    let receiving_scope = receiving.resolve_module_state("scope").unwrap();
    let mut output = [PortFrame::default(); 129];
    ALLOCS.with(|count| count.set(0));
    DEALLOCS.with(|count| count.set(0));
    TRACKING.with(|tracking| tracking.set(true));
    for length in [1, 3, 64, 129] {
        old.process_poly_block(&mut output[..length]);
        receiving.adopt_state_from(&old);
        receiving.process_poly_block(&mut output[..length]);
        std::hint::black_box(receiving.module_scope(receiving_scope));
        std::hint::black_box(receiving.module_lights(receiving_scope));
        bypassed.adopt_state_from(&receiving);
        bypassed.process_poly_block(&mut output[..length]);
        assert!(receiving.initialize_module_state(receiving_scope));
        receiving.process_poly_block(&mut output[..length]);
    }
    TRACKING.with(|tracking| tracking.set(false));
    assert_eq!(ALLOCS.with(Cell::get), 0);
    assert_eq!(DEALLOCS.with(Cell::get), 0);
}
