//! Allocation tracking is thread-local so parallel test harness work is ignored.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct TrackingAllocator;
thread_local! {
    static TRACKING: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
    static DEALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

fn record() {
    let _ = TRACKING.try_with(|enabled| {
        if enabled.get() {
            let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
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
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        record();
        unsafe { System.realloc(ptr, layout, size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let _ = TRACKING.try_with(|enabled| {
            if enabled.get() {
                let _ = DEALLOCATIONS.try_with(|count| count.set(count.get() + 1));
            }
        });
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: TrackingAllocator = TrackingAllocator;

#[test]
fn live_parameters_write_render_and_cross_rate_handoff_use_no_heap() {
    let mut patch: openpatch_core::Patch = serde_json::from_value(serde_json::json!({
        "version":2,"sample_rate":48000,
        "nodes":[{"id":"source","kind":"poly_constant","params":{"v1":5}},
                 {"id":"vca","kind":"stock_vca","params":{}},
                 {"id":"filter","kind":"rack264_vcf","params":{}}],
        "cables":[{"from":"source","to":"vca","input":"signal"},
                  {"from":"vca","to":"filter","input":"signal"}],
        "output":"filter","output_port":"lowpass"
    }))
    .unwrap();
    let mut a = patch.compile().unwrap();
    let gain = a.resolve_parameter("vca", "level").unwrap();
    let cutoff = a.resolve_parameter("filter", "frequency").unwrap();
    patch.sample_rate = 96000;
    let mut b = patch.compile().unwrap();
    let receiving_gain = b.resolve_parameter("vca", "level").unwrap();
    let receiving_cutoff = b.resolve_parameter("filter", "frequency").unwrap();
    let mut out = [openpatch_core::PortFrame::default(); 1];
    ALLOCATIONS.with(|n| n.set(0));
    DEALLOCATIONS.with(|n| n.set(0));
    TRACKING.with(|enabled| enabled.set(true));
    for frame in 0..4096 {
        if frame % 32 == 0 {
            a.set_parameter_value(gain, (frame % 128) as f32 / 128.0);
            a.set_parameter_value(cutoff, 0.1 + (frame % 127) as f32 / 200.0);
        }
        a.process_poly_block(&mut out);
        b.adopt_state_from(&a);
        b.inherit_parameter_values(&a, &[], &[]);
        b.parameter_value(receiving_gain);
        b.parameter_descriptor(receiving_cutoff);
        b.process_poly_block(&mut out);
    }
    // Explicit assignment/fresh incarnation filtering is stack-only as well.
    b.inherit_parameter_values(&a, &[receiving_gain.module()], &[receiving_cutoff]);
    TRACKING.with(|enabled| enabled.set(false));
    assert!(out[0].values.iter().all(|v| v.is_finite()));
    assert_eq!(ALLOCATIONS.with(Cell::get), 0);
    assert_eq!(DEALLOCATIONS.with(Cell::get), 0);
}

#[test]
fn rackmcp_sequence_transport_data_adoption_and_lanes_use_no_heap() {
    use openpatch_core::rackmcp_sequence::{Player, SequenceData};
    let first: SequenceData = serde_json::from_value(serde_json::json!({
        "patterns":[{"name":"A","length":2,"tracks":[{"voices":4,"steps":[
            {"step":0,"notes":[0,4,7,12],"ratchet":8,"slide":true,"accent":true},
            {"step":1,"notes":[12,7,4,0],"ratchet":4}]}]},
            {"name":"B","length":1,"tracks":[{"steps":[{"step":0}]}]}],
        "lanes":[{"moduleId":"123","paramId":0,"mode":"raw","points":[
            {"bar":0,"value":-5},{"bar":1,"value":5,"curve":"easeInOut"}]}]
    }))
    .unwrap();
    let mut second = first.clone();
    second.selected_pattern = Some("B".into());
    let a = first.compile().unwrap();
    let b = second.compile().unwrap();
    let mut player = Player::default();
    let mut sum = 0.0;
    ALLOCATIONS.with(|count| count.set(0));
    DEALLOCATIONS.with(|count| count.set(0));
    TRACKING.with(|enabled| enabled.set(true));
    player.adopt_data(&a);
    for frame in 0..4096 {
        if frame == 128 {
            player.adopt_data(&b);
        }
        if frame == 512 {
            player.reset();
        }
        if frame == 1024 {
            player.seek(2.25);
        }
        if frame == 2048 {
            player.request_pattern(&a, 0, true);
        }
        let data = if frame < 128 { &a } else { &b };
        let out = player.process(
            data,
            48000.0,
            120.0,
            62.5,
            frame % 127 != 0,
            true,
            frame % 97 == 0,
        );
        sum += out.pitch[0][0]
            + data.lanes()[0]
                .as_ref()
                .unwrap()
                .value_at(player.position().4 / 4.0);
    }
    TRACKING.with(|enabled| enabled.set(false));
    assert!(sum.is_finite());
    assert_eq!(
        ALLOCATIONS.with(Cell::get),
        0,
        "sequencer primitives allocated"
    );
    assert_eq!(
        DEALLOCATIONS.with(Cell::get),
        0,
        "sequencer primitives deallocated"
    );
}

#[test]
fn process_block_allocates_nothing_even_on_first_callback() {
    let mut engine = openpatch_core::demo_patch().compile().unwrap();
    let mut samples = [0.0; 1025];
    ALLOCATIONS.with(|count| count.set(0));
    TRACKING.with(|enabled| enabled.set(true));
    for length in [0, 1, 3, 64, 127, 128, 129, 1025] {
        for _ in 0..20 {
            engine.process_block(&mut samples[..length]);
        }
    }
    TRACKING.with(|enabled| enabled.set(false));
    assert_eq!(ALLOCATIONS.with(Cell::get), 0, "audio callback allocated");
    assert!(samples.iter().all(|value| value.is_finite()));
}

#[test]
fn stock_process_sixteen_voice_first_callback_actions_adoption_bypass_and_resets_use_no_heap() {
    use openpatch_core::{ModuleAction, Patch, PortFrame};

    let patch: Patch = serde_json::from_value(serde_json::json!({
        "version":2,"sample_rate":48000,
        "nodes":[
            {"id":"source","kind":"poly_constant","params":{"channels":16,"v1":5,"v16":-5}},
            {"id":"process","kind":"stock_process","params":{"slew":-10}}
        ],
        "cables":[{"from":"source","to":"process","input":"signal"}],
        "output":"process","output_port":"sh1"
    }))
    .unwrap();
    let mut old = patch.compile().unwrap();
    let mut receiving = patch.compile().unwrap();
    let mut bypass_patch = patch;
    bypass_patch.nodes[1].bypassed = true;
    let mut bypassed = bypass_patch.compile().unwrap();
    let old_handle = old.resolve_module_control("process").unwrap();
    let receiving_handle = receiving.resolve_module_control("process").unwrap();
    let bypass_handle = bypassed.resolve_module_state("process").unwrap();
    let mut frames = [PortFrame::default(); 129];

    ALLOCATIONS.with(|count| count.set(0));
    DEALLOCATIONS.with(|count| count.set(0));
    TRACKING.with(|enabled| enabled.set(true));
    for length in [0, 1, 3, 64, 129] {
        assert!(old.handle_module_event(old_handle, ModuleAction::GatePress));
        old.process_poly_block(&mut frames[..length]);
        assert!(old.handle_module_event(old_handle, ModuleAction::GateRelease));
        old.process_poly_block(&mut frames[..length]);

        receiving.adopt_state_from(&old);
        assert!(receiving.handle_module_event(receiving_handle, ModuleAction::GatePress));
        receiving.process_poly_block(&mut frames[..length]);
        std::hint::black_box(receiving.stock_process_snapshot_bits(receiving_handle));
        std::hint::black_box(receiving.module_lights(receiving_handle));

        bypassed.adopt_state_from(&receiving);
        bypassed.process_poly_block(&mut frames[..length]);
        std::hint::black_box(bypassed.stock_process_snapshot_bits(bypass_handle));
        assert!(receiving.handle_module_event(receiving_handle, ModuleAction::GateRelease));
        assert!(receiving.initialize_module_state(receiving_handle));
        assert!(receiving.reset_module_state(receiving_handle));
        receiving.process_poly_block(&mut frames[..length]);
        assert!(receiving.release_module_controls(receiving_handle));
    }
    TRACKING.with(|enabled| enabled.set(false));
    assert_eq!(ALLOCATIONS.with(Cell::get), 0);
    assert_eq!(DEALLOCATIONS.with(Cell::get), 0);
    assert!(frames.iter().all(|frame| frame.channels <= 16));
}

#[test]
fn studio_mixer_first_process_adoption_recreation_and_reset_use_no_heap() {
    use openpatch_core::{Patch, PortDirection, PortFrame};
    let mut patch: Patch = serde_json::from_value(serde_json::json!({
        "version": 2,
        "sample_rate": 48000,
        "nodes": [
            {"id": "source", "kind": "poly_constant", "params": {"channels": 2, "v1": 5, "v2": -3}},
            {"id": "mix", "kind": "studio_mixer", "params": {"channels": 16, "send_a16": 1}}
        ],
        "cables": [],
        "output": "mix",
        "output_port": "left"
    }))
    .unwrap();
    for input in (1..=16)
        .map(|i| format!("in{i}"))
        .chain(["return_a", "return_b", "send_a_cv", "send_b_cv"].map(str::to_owned))
    {
        patch.cables.push(
            serde_json::from_value(serde_json::json!({
                "from": "source", "output": "out", "to": "mix", "input": input
            }))
            .unwrap(),
        );
    }
    let mut old = patch.compile().unwrap();
    let mut adopted = patch.compile().unwrap();
    let mut recreated = patch.compile().unwrap();
    let handle = adopted.resolve_module_lights("mix").unwrap();
    let recreated_handle = recreated.resolve_module_lights("mix").unwrap();
    assert!(!adopted.enable_host_input_observer(handle, 19));
    assert!(adopted.host_port_layout(handle).is_none());
    adopted.set_port_lights_enabled(true);
    recreated.set_port_lights_enabled(true);
    let port_handles = ["return_a", "return_b", "send_a_cv", "send_b_cv"].map(|port| {
        adopted
            .resolve_port_light("mix", PortDirection::Input, port)
            .unwrap()
    });
    let mut frames = [PortFrame::default(); 129];

    ALLOCATIONS.with(|count| count.set(0));
    DEALLOCATIONS.with(|count| count.set(0));
    TRACKING.with(|enabled| enabled.set(true));
    for length in [0, 1, 3, 64, 128, 129] {
        old.process_poly_block(&mut frames[..length]);
        adopted.adopt_state_from(&old);
        adopted.process_poly_block(&mut frames[..length]);
        std::hint::black_box(adopted.module_lights(handle));
        for port in port_handles {
            std::hint::black_box(adopted.port_light(port));
        }
        assert!(adopted.reset_module_state(handle));
        adopted.process_poly_block(&mut frames[..length]);
        recreated.process_poly_block(&mut frames[..length]);
        std::hint::black_box(recreated.module_lights(recreated_handle));
    }
    TRACKING.with(|enabled| enabled.set(false));
    assert_eq!(ALLOCATIONS.with(Cell::get), 0);
    assert_eq!(DEALLOCATIONS.with(Cell::get), 0);
    assert!(frames
        .iter()
        .all(|frame| frame.values.iter().all(|value| value.is_finite())));
    assert!(port_handles.into_iter().all(|port| adopted
        .port_light(port)
        .is_some_and(|light| light.connected)));
}

#[test]
fn merge_first_callback_adoption_initialization_and_telemetry_allocate_nothing() {
    use openpatch_core::{Patch, PortFrame};
    let patch: Patch = serde_json::from_value(serde_json::json!({
        "version":2,"sample_rate":48000,
        "nodes":[{"id":"source","kind":"poly_constant","params":{"channels":16,"v1":5}},
                 {"id":"merge","kind":"poly_merge"}],
        "cables":[{"from":"source","to":"merge","input":"in16"}],"output":"merge"
    }))
    .unwrap();
    let mut zero_patch = patch.clone();
    zero_patch.nodes[1].params.insert("channels".into(), -1.0);
    let mut bypass_patch = zero_patch.clone();
    bypass_patch.nodes[1].bypassed = true;
    let mut old = patch.compile().unwrap();
    let mut receiving = zero_patch.compile().unwrap();
    let mut bypassed = bypass_patch.compile().unwrap();
    let handle = receiving.resolve_module_lights("merge").unwrap();
    let bypass_handle = bypassed.resolve_module_lights("merge").unwrap();
    let mut frames = [PortFrame::default(); 1025];
    ALLOCATIONS.with(|count| count.set(0));
    TRACKING.with(|enabled| enabled.set(true));
    for length in [0, 1, 3, 64, 127, 128, 129, 1025] {
        old.process_poly_block(&mut frames[..length]);
        receiving.adopt_state_from(&old);
        receiving.process_poly_block(&mut frames[..length]);
        std::hint::black_box(receiving.module_lights(handle));
        bypassed.adopt_state_from(&receiving);
        bypassed.process_poly_block(&mut frames[..length]);
        std::hint::black_box(bypassed.module_lights(bypass_handle));
    }
    let initialized = receiving.initialize_module_state(handle);
    receiving.process_poly_block(&mut frames);
    TRACKING.with(|enabled| enabled.set(false));
    assert!(initialized);
    assert_eq!(ALLOCATIONS.with(Cell::get), 0);
    assert_eq!(
        receiving.module_lights(handle).unwrap().output_channels,
        Some(16)
    );
    assert_eq!(
        receiving.module_lights(handle).unwrap().automatic_channels,
        Some(16)
    );
    assert_eq!(
        bypassed
            .module_lights(bypass_handle)
            .unwrap()
            .output_channels,
        Some(0)
    );
}

#[test]
fn sum_meter_and_channel_telemetry_allocate_nothing() {
    use openpatch_core::{Patch, PortFrame};
    let patch: Patch = serde_json::from_value(serde_json::json!({
        "version":2,"sample_rate":48000,
        "nodes":[{"id":"source","kind":"poly_constant","params":{"channels":16}},
                 {"id":"sum","kind":"poly_sum","params":{"level":0.5}}],
        "cables":[{"from":"source","to":"sum","input":"signal","output":"out"}],
        "output":"sum","output_port":"out"
    }))
    .unwrap();
    let mut engine = patch.compile().unwrap();
    let handle = engine.resolve_module_lights("sum").unwrap();
    let mut frames = [PortFrame::default(); 1025];
    ALLOCATIONS.with(|count| count.set(0));
    TRACKING.with(|enabled| enabled.set(true));
    for _ in 0..20 {
        engine.process_poly_block(&mut frames);
        std::hint::black_box(engine.module_lights(handle));
    }
    TRACKING.with(|enabled| enabled.set(false));
    assert_eq!(ALLOCATIONS.with(Cell::get), 0);
    assert_eq!(
        engine.module_lights(handle).unwrap().input_channels,
        Some(16)
    );
}

#[test]
fn viz_light_callback_allocates_nothing() {
    use openpatch_core::{Patch, PortFrame};
    let patch: Patch = serde_json::from_value(serde_json::json!({"version":2,"sample_rate":48000,"nodes":[{"id":"source","kind":"poly_constant","params":{"channels":16}},{"id":"viz","kind":"poly_viz"}],"cables":[{"from":"source","to":"viz","input":"signal"}]})).unwrap();
    let mut engine = patch.compile().unwrap();
    let handle = engine.resolve_module_lights("viz").unwrap();
    let mut frames = [PortFrame::default(); 1025];
    ALLOCATIONS.with(|count| count.set(0));
    TRACKING.with(|enabled| enabled.set(true));
    for _ in 0..20 {
        engine.process_poly_block(&mut frames);
        std::hint::black_box(engine.module_lights(handle));
    }
    TRACKING.with(|enabled| enabled.set(false));
    assert_eq!(ALLOCATIONS.with(Cell::get), 0);
    assert_eq!(
        engine.module_lights(handle).unwrap().input_channels,
        Some(16)
    );
}

#[test]
fn scope_capture_clone_adoption_and_initialization_allocate_nothing() {
    use openpatch_core::{Patch, PortFrame, ScopeSnapshot};
    let p:Patch=serde_json::from_value(serde_json::json!({"version":2,"sample_rate":48000,"nodes":[{"id":"x","kind":"poly_constant","params":{"channels":16}},{"id":"s","kind":"scope"}],"cables":[{"from":"x","to":"s","input":"x"}]})).unwrap();
    let mut old = p.compile().unwrap();
    let mut new = p.compile().unwrap();
    let handle = new.resolve_module_scope("s").unwrap();
    let mut snapshot = Box::new(ScopeSnapshot::default());
    let mut frames = [PortFrame::default(); 1025];
    ALLOCATIONS.with(|c| c.set(0));
    TRACKING.with(|c| c.set(true));
    for _ in 0..20 {
        old.process_poly_block(&mut frames);
        new.adopt_state_from(&old);
        snapshot
            .as_mut()
            .clone_from(new.module_scope(handle).unwrap());
        new.process_poly_block(&mut frames);
    }
    assert!(new.initialize_module_state(handle));
    TRACKING.with(|c| c.set(false));
    assert_eq!(ALLOCATIONS.with(Cell::get), 0);
    assert_eq!(new.module_scope(handle).unwrap().buffer_index, 0);
}

#[test]
fn colored_noise_first_callback_adoption_initialize_and_bypass_allocate_nothing() {
    use openpatch_core::{Patch, PortFrame};
    let patch: Patch = serde_json::from_value(serde_json::json!({"version":2,"sample_rate":48000,"nodes":[{"id":"n","kind":"colored_noise"}],"output":"n","output_port":"gray"})).unwrap();
    let mut old = patch.compile().unwrap();
    let mut new = patch.compile().unwrap();
    let mut bypass = patch.clone();
    bypass.nodes[0].bypassed = true;
    let mut paused = bypass.compile().unwrap();
    let handle = new.resolve_module_state("n").unwrap();
    let mut samples = [PortFrame::default(); 1025];
    ALLOCATIONS.with(|c| c.set(0));
    TRACKING.with(|c| c.set(true));
    for _ in 0..8 {
        old.process_poly_block(&mut samples);
        new.adopt_state_from(&old);
        new.process_poly_block(&mut samples);
        paused.adopt_state_from(&new);
        paused.process_poly_block(&mut samples);
        new.adopt_state_from(&paused);
        assert!(new.initialize_module_state(handle));
        assert!(new.reset_module_state(handle));
        new.process_poly_block(&mut samples);
    }
    TRACKING.with(|c| c.set(false));
    assert_eq!(ALLOCATIONS.with(Cell::get), 0);
    assert!(samples
        .iter()
        .all(|f| f.channels == 1 && f.values.iter().all(|v| v.is_finite())));
}

#[test]
fn two_stock_noise_gray_boundaries_and_compatible_adoption_use_no_callback_heap() {
    use openpatch_core::{Patch, PortFrame};
    let patch: Patch = serde_json::from_value(serde_json::json!({
        "version":2,"sample_rate":48000,
        "nodes":[{"id":"a","kind":"stock_noise"},
                 {"id":"b","kind":"stock_noise"},
                 {"id":"sink","kind":"poly_viz"}],
        "cables":[{"from":"b","output":"gray","to":"sink","input":"signal"}],
        "output":"a","output_port":"gray"
    }))
    .unwrap();
    let mut old = patch.compile().unwrap();
    let mut receiving = patch.compile().unwrap();
    let mut bypass = patch.clone();
    bypass.nodes[0].bypassed = true;
    bypass.nodes[1].bypassed = true;
    let mut paused = bypass.compile().unwrap();
    let normals = [0x3f80_0000; 64];
    let uniforms = [0x3f00_0000; 64];
    for engine in [&mut old, &mut receiving, &mut paused] {
        for id in ["a", "b"] {
            let handle = engine.resolve_module_state(id).unwrap();
            assert!(engine.set_stock_noise_controlled_draws(handle, &normals, &uniforms));
        }
        let sink = engine.resolve_module_state("sink").unwrap();
        assert!(engine.enable_host_input_observer(sink, 0));
    }
    let handle_a = receiving.resolve_module_state("a").unwrap();
    let handle_b = receiving.resolve_module_state("b").unwrap();
    let sink = receiving.resolve_module_state("sink").unwrap();
    let mut frames = [PortFrame::default(); 1025];
    ALLOCATIONS.with(|count| count.set(0));
    DEALLOCATIONS.with(|count| count.set(0));
    TRACKING.with(|enabled| enabled.set(true));
    old.process_poly_block(&mut frames);
    receiving.adopt_state_from(&old);
    receiving.process_poly_block(&mut frames);
    paused.adopt_state_from(&receiving);
    paused.process_poly_block(&mut frames[..1]);
    receiving.adopt_state_from(&paused);
    std::hint::black_box(receiving.observed_host_input(sink, 0));
    assert!(receiving.initialize_module_state(handle_a));
    assert!(receiving.initialize_module_state(handle_b));
    receiving.process_poly_block(&mut frames);
    TRACKING.with(|enabled| enabled.set(false));
    assert_eq!(ALLOCATIONS.with(Cell::get), 0);
    assert_eq!(DEALLOCATIONS.with(Cell::get), 0);
    assert!(frames.iter().all(|frame| frame.values[0].is_finite()));
    assert_eq!(
        receiving.stock_noise_snapshot(handle_a).unwrap().gray_frame,
        3
    );
    assert_eq!(
        receiving.stock_noise_snapshot(handle_b).unwrap().gray_frame,
        3
    );
}

#[test]
fn random_values_actions_capture_restore_and_adoption_allocate_nothing() {
    use openpatch_core::{ModuleAction, Patch, PortFrame};
    let p:Patch=serde_json::from_value(serde_json::json!({"version":2,"sample_rate":48000,"nodes":[{"id":"r","kind":"random_values"}],"output":"r","output_port":"out7"})).unwrap();
    let mut a = p.compile().unwrap();
    let mut b = p.compile().unwrap();
    let h = a.resolve_module_control("r").unwrap();
    let hb = b.resolve_module_state("r").unwrap();
    let mut frames = [PortFrame::default(); 128];
    ALLOCATIONS.with(|count| count.set(0));
    TRACKING.with(|enabled| enabled.set(true));
    for _ in 0..50 {
        a.handle_module_event(h, ModuleAction::PushPress);
        a.handle_module_event(h, ModuleAction::PushRelease);
        a.process_poly_block(&mut frames);
        let state = a.module_saved_state(h).unwrap();
        b.set_module_saved_state(hb, state);
        b.adopt_state_from(&a);
        a.initialize_module_state(h);
        std::hint::black_box(a.module_control_state(h));
        std::hint::black_box(a.module_lights(h));
    }
    TRACKING.with(|enabled| enabled.set(false));
    assert_eq!(ALLOCATIONS.with(Cell::get), 0);
}

#[test]
fn shift_register_first_callback_actions_snapshot_initialize_and_adoption_allocate_nothing() {
    use openpatch_core::{ModuleAction, Patch, PortFrame};
    let p:Patch=serde_json::from_value(serde_json::json!({"version":2,"sample_rate":48000,"nodes":[{"id":"s","kind":"sh_asr","params":{"random":1}}],"output":"s","output_port":"out8"})).unwrap();
    let mut a = p.compile().unwrap();
    let mut b = p.compile().unwrap();
    let mut bp = p.clone();
    bp.nodes[0].bypassed = true;
    let mut paused = bp.compile().unwrap();
    let ha = a.resolve_module_control("s").unwrap();
    let hb = b.resolve_module_state("s").unwrap();
    let mut frames = [PortFrame::default(); 129];
    ALLOCATIONS.with(|c| c.set(0));
    TRACKING.with(|c| c.set(true));
    for length in [0, 1, 3, 64, 129] {
        a.handle_module_event(ha, ModuleAction::PushPress);
        a.handle_module_event(ha, ModuleAction::PushRelease);
        a.handle_module_event(ha, ModuleAction::ResetPress);
        a.handle_module_event(ha, ModuleAction::ResetRelease);
        a.process_poly_block(&mut frames[..length]);
        let state = a.module_saved_state(ha).unwrap();
        b.set_module_saved_state(hb, state);
        b.adopt_state_from(&a);
        b.process_poly_block(&mut frames[..length]);
        paused.adopt_state_from(&b);
        paused.process_poly_block(&mut frames[..length]);
        a.adopt_state_from(&paused);
        a.initialize_module_state(ha);
        a.reset_module_state(ha);
        std::hint::black_box(a.module_control_state(ha));
    }
    TRACKING.with(|c| c.set(false));
    assert_eq!(ALLOCATIONS.with(Cell::get), 0);
    assert!(frames
        .iter()
        .all(|f| f.values.iter().all(|v| v.is_finite())));
}

#[test]
fn adsr_eg_first_callback_actions_lamps_initialize_reset_and_adoption_allocate_nothing() {
    use openpatch_core::{ModuleAction, Patch, PortFrame};
    let p: Patch=serde_json::from_value(serde_json::json!({"version":2,"sample_rate":48000,"nodes":[{"id":"g","kind":"poly_constant","params":{"channels":16}},{"id":"e","kind":"adsr_eg"}],"cables":[{"from":"g","to":"e","input":"gate"}],"output":"e"})).unwrap();
    let mut old = p.compile().unwrap();
    let mut receiving = p.compile().unwrap();
    let h = old.resolve_module_control("e").unwrap();
    let hr = receiving.resolve_module_control("e").unwrap();
    let mut frames = [PortFrame::default(); 1025];
    ALLOCATIONS.with(|count| count.set(0));
    TRACKING.with(|enabled| enabled.set(true));
    assert!(old.handle_module_event(h, ModuleAction::PushPress));
    old.process_poly_block(&mut frames);
    std::hint::black_box(old.module_lights(h));
    std::hint::black_box(old.module_control_state(h));
    receiving.adopt_state_from(&old);
    receiving.process_poly_block(&mut frames);
    assert!(receiving.release_module_controls(hr));
    receiving.process_poly_block(&mut frames);
    assert!(receiving.initialize_module_state(hr));
    assert!(receiving.reset_module_state(hr));
    TRACKING.with(|enabled| enabled.set(false));
    assert_eq!(ALLOCATIONS.with(Cell::get), 0);
}

#[test]
fn midi_cc_first_callback_taps_learn_capture_and_adoption_allocate_nothing() {
    use openpatch_core::{MidiEvent, Patch, PortFrame, TapSample};
    let p:Patch=serde_json::from_value(serde_json::json!({"version":2,"sample_rate":48000,"nodes":[{"id":"cc","kind":"midi_cc","params":{"mpe":1,"high_resolution":1}}]})).unwrap();
    let mut old = p.compile().unwrap();
    let mut new = p.compile().unwrap();
    let old_h = old.resolve_module_state("cc").unwrap();
    let new_h = new.resolve_module_state("cc").unwrap();
    let taps = [
        Some(new.resolve_tap("cc", "out1", 0).unwrap()),
        Some(new.resolve_tap("cc", "out16", 15).unwrap()),
    ];
    let mut audio = [[0.0; 2]; 129];
    let mut samples = [[TapSample::default(); 2]; 129];
    let mut frames = [PortFrame::default(); 129];
    ALLOCATIONS.with(|count| count.set(0));
    TRACKING.with(|enabled| enabled.set(true));
    old.arm_midi_cc_learn(old_h, 1, 0);
    old.handle_midi(MidiEvent::ControlChange {
        channel: 15,
        controller: 16,
        value: 100,
    });
    new.adopt_state_from(&old);
    std::hint::black_box(new.take_midi_cc_learn_completion(new_h));
    new.arm_midi_cc_learn(new_h, 2, 2);
    for channel in 0..16 {
        new.handle_midi(MidiEvent::ControlChange {
            channel,
            controller: 1,
            value: 64,
        });
        new.handle_midi(MidiEvent::ControlChange {
            channel,
            controller: 33,
            value: 3,
        });
    }
    new.process_audio_block_with_taps(&mut audio, &taps, &mut samples);
    new.process_poly_block(&mut frames);
    std::hint::black_box(new.module_saved_state(new_h));
    new.initialize_module_state(new_h);
    new.reset_module_state(new_h);
    TRACKING.with(|enabled| enabled.set(false));
    assert_eq!(ALLOCATIONS.with(Cell::get), 0);
    assert!(samples[0][0].active);
    assert!(samples[0][1].active);
}

#[test]
fn stock_vcf_first_processing_taps_adoption_and_initialize_allocate_nothing() {
    use openpatch_core::{Patch, PortFrame, TapSample};
    let p:Patch=serde_json::from_value(serde_json::json!({"version":2,"sample_rate":48000,
      "nodes":[{"id":"s","kind":"poly_constant","params":{"channels":16}}, {"id":"f","kind":"stock_vcf"}],
      "cables":[{"from":"s","to":"f","input":"signal"}],"output":"f","output_port":"lowpass"})).unwrap();
    let mut old = p.compile().unwrap();
    let mut next = p.compile().unwrap();
    let h = next.resolve_module_state("f").unwrap();
    let taps = [
        Some(next.resolve_tap("f", "lowpass", 0).unwrap()),
        Some(next.resolve_tap("f", "highpass", 15).unwrap()),
    ];
    let mut frames = [PortFrame::default(); 129];
    let mut audio = [[0.0; 2]; 129];
    let mut samples = [[TapSample::default(); 2]; 129];
    ALLOCATIONS.with(|c| c.set(0));
    TRACKING.with(|c| c.set(true));
    old.process_poly_block(&mut frames);
    next.adopt_state_from(&old);
    next.process_audio_block_with_taps(&mut audio, &taps, &mut samples);
    assert!(next.initialize_module_state(h));
    assert!(next.reset_module_state(h));
    next.process_poly_block(&mut frames);
    TRACKING.with(|c| c.set(false));
    assert_eq!(ALLOCATIONS.with(Cell::get), 0);
    assert!(samples[0][0].active && samples[0][1].active);
}

#[test]
fn stock_vca_first_processing_taps_lights_adoption_and_reset_allocate_nothing() {
    use openpatch_core::{Patch, PortFrame, TapSample};
    let p: Patch = serde_json::from_value(serde_json::json!({
        "version":2,"sample_rate":48000,
        "nodes":[{"id":"s","kind":"poly_constant","params":{"channels":1,"v1":2}},
                 {"id":"cv","kind":"poly_constant","params":{"channels":16,"v1":5,"v16":10}},
                 {"id":"vca","kind":"stock_vca","params":{"response":0}}],
        "cables":[{"from":"s","to":"vca","input":"signal"},
                  {"from":"cv","to":"vca","input":"cv"}],"output":"vca"
    }))
    .unwrap();
    let mut old = p.compile().unwrap();
    let mut next = p.compile().unwrap();
    let state = next.resolve_module_state("vca").unwrap();
    let lights = next.resolve_module_lights("vca").unwrap();
    let taps = [
        Some(next.resolve_tap("vca", "out", 0).unwrap()),
        Some(next.resolve_tap("vca", "out", 15).unwrap()),
    ];
    let mut frames = [PortFrame::default(); 129];
    let mut audio = [[0.0; 2]; 129];
    let mut samples = [[TapSample::default(); 2]; 129];
    ALLOCATIONS.with(|c| c.set(0));
    TRACKING.with(|c| c.set(true));
    old.process_poly_block(&mut frames);
    next.adopt_state_from(&old);
    next.process_audio_block_with_taps(&mut audio, &taps, &mut samples);
    let published = next.module_lights(lights).unwrap();
    let initialized = next.initialize_module_state(state);
    let cleared = next.module_lights(lights).unwrap();
    let reset = next.reset_module_state(state);
    next.process_poly_block(&mut frames);
    TRACKING.with(|c| c.set(false));
    assert_eq!(ALLOCATIONS.with(Cell::get), 0);
    assert!(initialized && reset);
    assert_eq!(published.input_channels, Some(16));
    assert_eq!(published.brightness[0], [0.0625, 0.0]);
    assert_eq!(published.brightness[15], [1.0, 0.0]);
    assert_eq!(cleared.input_channels, Some(1));
    assert_eq!(
        cleared.brightness,
        [[0.0; 2]; openpatch_core::MAX_MODULE_LIGHTS]
    );
    assert!(samples[0][0].active && samples[0][1].active);
    assert_eq!(samples[0][0].value, 0.125);
    assert_eq!(samples[0][1].value, 2.0);
}
