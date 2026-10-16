//! Public graph/lifecycle coverage. The thread-local allocator measures only
//! prepared callback work; graph construction and retirement stay outside it.
use openpatch_core::{
    Engine, ModuleContent, Patch, PortDirection, PortFrame, PreparedWaveBanks, TapSample,
    WaveAsset, WavePreparationCache,
};
use serde_json::json;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    sync::Arc,
};

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

fn sine() -> Vec<f32> {
    (0..64)
        .map(|point| (std::f32::consts::TAU * point as f32 / 64.0).sin())
        .collect()
}
fn install(patch: &mut Patch, id: &str, samples: &[f32], points: u32) {
    let asset = Arc::new(WaveAsset::new(samples).unwrap());
    patch
        .nodes
        .iter_mut()
        .find(|node| node.id == id)
        .unwrap()
        .content = Some(ModuleContent::Wavetable {
        asset_id: asset.id().into(),
        wave_points: points,
        name: "Host test bank".into(),
    });
    patch.assets.insert(asset.id().into(), asset);
}
fn simple(kind: &str) -> Patch {
    let mut patch: Patch = serde_json::from_value(json!({"version":2,"sample_rate":48000,
        "nodes":[{"id":"wave","kind":kind,"params":{"offset":0}}],"output":"wave"}))
    .unwrap();
    if kind == "wavetable_vco" {
        patch.nodes[0].params.clear();
    }
    install(&mut patch, "wave", &sine(), 64);
    patch
}
fn rich() -> Patch {
    let mut patch: Patch = serde_json::from_value(json!({"version":2,"sample_rate":48000,
        "nodes":[
            {"id":"pitch","kind":"poly_constant","params":{"channels":16,"v1":-0.5,"v16":0.5}},
            {"id":"fm","kind":"poly_constant","params":{"channels":16,"v1":0.1,"v16":-0.1}},
            {"id":"position","kind":"poly_constant","params":{"channels":1,"v1":5}},
            {"id":"sync_source","kind":"lfo","params":{"frequency":73,"amplitude":1,"offset":0}},
            {"id":"sync","kind":"gain","params":{"gain":5}},
            {"id":"clock_source","kind":"lfo","params":{"frequency":47,"amplitude":1,"offset":0}},
            {"id":"clock","kind":"gain","params":{"gain":5}},
            {"id":"vco","kind":"wavetable_vco","params":{"position":0.25,"fm_amount":0.1,"position_cv_amount":0.5}},
            {"id":"lfo","kind":"wavetable_lfo","params":{"rate":3,"position":0.25,"fm_amount":0.1,"position_cv_amount":0.5,"offset":0}},
            {"id":"vco_meter","kind":"poly_viz"},
            {"id":"lfo_meter","kind":"poly_viz"}
        ],"cables":[
            {"id":"sync_gain","from":"sync_source","to":"sync","input":"signal"},
            {"id":"clock_gain","from":"clock_source","to":"clock","input":"signal"},
            {"id":"pitch_vco","from":"pitch","to":"vco","input":"pitch"},
            {"id":"fm_vco","from":"fm","to":"vco","input":"fm"},
            {"id":"fm_lfo","from":"fm","to":"lfo","input":"fm"},
            {"id":"position_vco","from":"position","to":"vco","input":"position_cv"},
            {"id":"position_lfo","from":"position","to":"lfo","input":"position_cv"},
            {"id":"sync_vco","from":"sync","to":"vco","input":"sync"},
            {"id":"reset_lfo","from":"sync","to":"lfo","input":"reset"},
            {"id":"clock_lfo","from":"clock","to":"lfo","input":"clock"},
            {"id":"vco_meter","from":"vco","to":"vco_meter","input":"signal"},
            {"id":"lfo_meter","from":"lfo","to":"lfo_meter","input":"signal"}
        ],"output":"vco"})).unwrap();
    let waves = [sine(), sine().iter().map(|sample| sample * -0.5).collect()].concat();
    install(&mut patch, "vco", &waves, 64);
    install(&mut patch, "lfo", &waves, 64);
    patch
}
fn prepared(patch: &Patch, cache: &WavePreparationCache) -> Engine {
    let ticket = patch.prepare_wave_banks(cache).unwrap();
    patch.compile_with_wave_banks(&ticket).unwrap()
}

#[test]
fn first_polyphonic_callback_taps_handoffs_initialize_and_rewires_do_not_touch_heap() {
    let cache = WavePreparationCache::default();
    let patch = rich();
    let mut first = prepared(&patch, &cache);
    let mut second = prepared(&patch, &cache);
    let mut bypass_patch = patch.clone();
    bypass_patch
        .nodes
        .iter_mut()
        .find(|node| node.id == "vco")
        .unwrap()
        .bypassed = true;
    let mut bypass = prepared(&bypass_patch, &cache);
    let mut rewire_patch = patch.clone();
    rewire_patch
        .cables
        .iter_mut()
        .find(|cable| cable.id.as_deref() == Some("sync_vco"))
        .unwrap()
        .from = "clock".into();
    rewire_patch
        .nodes
        .iter_mut()
        .find(|node| node.id == "vco")
        .unwrap()
        .params
        .insert("tune".into(), 7.0);
    let mut rewired = prepared(&rewire_patch, &cache);
    let taps = [
        Some(second.resolve_tap("vco", "out", 15).unwrap()),
        Some(second.resolve_tap("lfo", "out", 15).unwrap()),
    ];
    let vco = rewired.resolve_module_state("vco").unwrap();
    let lfo = rewired.resolve_module_state("lfo").unwrap();
    let changed_cable = rewired.resolve_cable_state("sync_vco").unwrap();
    let input_light = rewired
        .resolve_port_light("vco", PortDirection::Input, "pitch")
        .unwrap();
    let output_light = rewired
        .resolve_port_light("lfo", PortDirection::Output, "out")
        .unwrap();
    rewired.set_port_lights_enabled(true);
    let mut frames = [PortFrame::default(); 129];
    let mut muted = [PortFrame::default(); 129];
    let mut audio = [[0.0; 2]; 129];
    let mut samples = [[TapSample::default(); 2]; 129];
    let mut accepted = true;

    MEMORY_OPERATIONS.with(|count| count.set(0));
    TRACKING.with(|tracking| tracking.set(true));
    for length in [0, 1, 3, 64, 127, 128, 129] {
        first.process_poly_block(&mut frames[..length]);
    }
    first.process_poly_block(&mut frames[..128]);
    first.process_poly_block(&mut frames[..96]);
    // Wavetable handoff copies bounded histories and retains the receiving
    // immutable bank; no large mutable allocation needs to change ownership.
    let _ = second.handoff_state_from(&mut first);
    second.process_audio_block_with_taps(&mut audio, &taps, &mut samples);
    bypass.adopt_state_from(&second);
    bypass.process_poly_block(&mut muted);
    let _ = rewired.handoff_state_from_excluding_routes(&mut bypass, &[], &[changed_cable]);
    accepted &= rewired.initialize_module_state(vco);
    accepted &= rewired.initialize_module_state(lfo);
    for length in [0, 1, 3, 64, 127, 128, 129] {
        rewired.process_poly_block(&mut frames[..length]);
    }
    let observed_input = rewired.port_light(input_light);
    let observed_output = rewired.port_light(output_light);
    accepted &= rewired.reset_module_state(lfo);
    TRACKING.with(|tracking| tracking.set(false));

    assert!(accepted);
    assert_eq!(
        MEMORY_OPERATIONS.with(Cell::get),
        0,
        "callback allocated, reallocated or deallocated"
    );
    assert!(frames
        .iter()
        .all(|frame| frame.channels == 16 && frame.values.iter().all(|value| value.is_finite())));
    assert!(muted
        .iter()
        .all(|frame| frame.channels == 1 && frame.values == [0.0; 16]));
    assert!(samples
        .iter()
        .flatten()
        .all(|sample| sample.active && sample.value.is_finite()));
    assert_eq!(observed_input.unwrap().channels, 16);
    assert_eq!(observed_output.unwrap().channels, 16);
    assert!(!observed_output.unwrap().invalid);
    // All donor/receiver graph destruction occurs after the allocator guard.
}

#[test]
fn graph_sync_changes_voltage_and_position_cv_and_polyphony_are_observable() {
    let patch = rich();
    let mut donor = patch.compile().unwrap();
    let mut adopted = patch.compile().unwrap();
    // Advance just beyond a source zero crossing so subsequent frames include
    // a pending hard-sync correction, in addition to ordinary phase history.
    donor.process_poly_block(&mut [PortFrame::default(); 676]);
    adopted.adopt_state_from(&donor);
    let mut continued = [PortFrame::default(); 129];
    let mut after_adoption = [PortFrame::default(); 129];
    donor.process_poly_block(&mut continued);
    adopted.process_poly_block(&mut after_adoption);
    assert_eq!(after_adoption, continued);
    let mut no_sync = patch.clone();
    no_sync
        .cables
        .retain(|cable| cable.id.as_deref() != Some("sync_vco"));
    let mut synced = patch.compile().unwrap();
    let mut free = no_sync.compile().unwrap();
    synced.set_port_lights_enabled(true);
    let lfo_tap = [Some(synced.resolve_tap("lfo", "out", 15).unwrap())];
    let mut audio = [[0.0; 2]; 4096];
    let mut lfo = [[TapSample::default(); 1]; 4096];
    synced.process_audio_block_with_taps(&mut audio, &lfo_tap, &mut lfo);
    let mut unsynced = [PortFrame::default(); 4096];
    free.process_poly_block(&mut unsynced);
    assert!(audio
        .iter()
        .zip(&unsynced)
        .skip(256)
        .any(|(synced, free)| (synced[0] - free.values[0]).abs() > 0.01));
    assert!(audio
        .iter()
        .all(|frame| frame[0].is_finite() && frame[0].abs() < 100.0));
    assert!(lfo
        .iter()
        .skip(1)
        .all(|frame| frame[0].active && frame[0].value.is_finite()));
    for id in ["vco", "lfo"] {
        let lights = synced
            .module_lights(synced.resolve_module_lights(id).unwrap())
            .unwrap();
        assert_eq!(lights.count, 5);
        assert_eq!(lights.waveform_position, Some(0.5));
        let output = synced
            .port_light(
                synced
                    .resolve_port_light(id, PortDirection::Output, "out")
                    .unwrap(),
            )
            .unwrap();
        assert!(output.connected && !output.invalid);
        assert_eq!(output.channels, 16);
    }
}

#[test]
fn adoption_and_bypass_resume_the_same_wave_and_initialize_has_model_specific_phase_policy() {
    for kind in ["wavetable_vco", "wavetable_lfo"] {
        let patch = simple(kind);
        let mut old = patch.compile().unwrap();
        old.process_poly_block(&mut [PortFrame::default(); 513]);
        let mut resumed = patch.compile().unwrap();
        resumed.adopt_state_from(&old);
        let mut bypass_patch = patch.clone();
        bypass_patch.nodes[0].bypassed = true;
        let mut bypassed = bypass_patch.compile().unwrap();
        bypassed.adopt_state_from(&old);
        let before_lights = bypassed.module_lights(bypassed.resolve_module_lights("wave").unwrap());
        let mut muted = [PortFrame::default(); 1024];
        bypassed.process_poly_block(&mut muted);
        assert!(muted
            .iter()
            .all(|frame| frame.channels == 1 && frame.values == [0.0; 16]));
        assert_eq!(
            bypassed.module_lights(bypassed.resolve_module_lights("wave").unwrap()),
            before_lights
        );
        let mut unmuted = patch.compile().unwrap();
        unmuted.adopt_state_from(&bypassed);
        let mut expected = [PortFrame::default(); 257];
        let mut adopted = [PortFrame::default(); 257];
        let mut after_bypass = [PortFrame::default(); 257];
        old.process_poly_block(&mut expected);
        resumed.process_poly_block(&mut adopted);
        unmuted.process_poly_block(&mut after_bypass);
        assert_eq!(adopted, expected, "{kind} adoption lost phase/history");
        assert_eq!(
            after_bypass, expected,
            "{kind} bypass advanced oscillator history"
        );

        let mut initialized = patch.compile().unwrap();
        initialized.adopt_state_from(&old);
        assert!(
            initialized.initialize_module_state(initialized.resolve_module_state("wave").unwrap())
        );
        let mut baseline = if kind == "wavetable_lfo" {
            patch.compile().unwrap()
        } else {
            let mut next = patch.compile().unwrap();
            next.adopt_state_from(&old);
            next
        };
        initialized.process_poly_block(&mut adopted);
        baseline.process_poly_block(&mut expected);
        assert_eq!(
            adopted, expected,
            "{kind} Initialize violated its published phase policy"
        );
    }
}

#[test]
fn receiving_bank_and_controls_remain_authoritative_and_cache_ownership_survives_handoff() {
    for kind in ["wavetable_vco", "wavetable_lfo"] {
        let cache = WavePreparationCache::default();
        let patch = simple(kind);
        let ticket = patch.prepare_wave_banks(&cache).unwrap();
        let mut old = patch.compile_with_wave_banks(&ticket).unwrap();
        old.process_poly_block(&mut [PortFrame::default(); 513]);
        let mut changed = patch.clone();
        changed.nodes[0].params.insert("position".into(), 0.75);
        install(
            &mut changed,
            "wave",
            &[vec![-0.25; 64], vec![0.75; 64]].concat(),
            64,
        );
        let replacement = changed.prepare_wave_banks(&cache).unwrap();
        let mut receiving = changed.compile_with_wave_banks(&replacement).unwrap();
        let _ = receiving.handoff_state_from(&mut old);
        let mut output = [PortFrame::default(); 64];
        receiving.process_poly_block(&mut output);
        assert!(output
            .iter()
            .all(|frame| (frame.values[0] - 2.5).abs() <= 0.002));
        assert_eq!(
            receiving
                .module_lights(receiving.resolve_module_lights("wave").unwrap())
                .unwrap()
                .waveform_position,
            Some(0.75)
        );
        drop(ticket);
        drop(replacement);
        cache.clear_unused();
        assert_eq!(cache.stats().live_banks, 2);
        drop(old);
        assert_eq!(cache.stats().live_banks, 1);
        assert!(cache.stats().admitted_bytes > 0);
        drop(receiving);
        assert_eq!(cache.stats().live_banks, 0);
        assert_eq!(cache.stats().admitted_bytes, 0);

        // Persist the graph itself, including sample payloads. The compact
        // inspection representation must never be mistaken for this document.
        let saved = serde_json::to_vec(&changed).unwrap();
        let restored: Patch = serde_json::from_slice(&saved).unwrap();
        assert_eq!(restored.assets, changed.assets);
        let mut saved_engine = restored.compile().unwrap();
        saved_engine.process_poly_block(&mut output);
        assert!(output
            .iter()
            .all(|frame| (frame.values[0] - 2.5).abs() <= 0.002));
        assert!(serde_json::from_value::<Patch>(changed.reference_view()).is_err());
    }
}

#[test]
fn stale_preparation_tickets_and_missing_assets_cannot_replace_a_working_engine() {
    let cache = WavePreparationCache::default();
    let patch = simple("wavetable_vco");
    let ticket = patch.prepare_wave_banks(&cache).unwrap();
    let mut engine = patch.compile_with_wave_banks(&ticket).unwrap();
    let mut baseline = patch.compile_with_wave_banks(&ticket).unwrap();
    assert!(patch
        .compile_with_wave_banks(&PreparedWaveBanks::default())
        .is_err());
    let stats = cache.stats();
    let mut missing = patch.clone();
    missing.assets.clear();
    assert!(missing.prepare_wave_banks(&cache).is_err());
    assert!(missing.compile_with_wave_banks(&ticket).is_err());
    let mut geometry = patch.clone();
    if let Some(ModuleContent::Wavetable { wave_points, .. }) = &mut geometry.nodes[0].content {
        *wave_points = 32;
    }
    assert!(geometry.compile_with_wave_banks(&ticket).is_err());
    let mut model = patch.clone();
    model.nodes[0].kind = "wavetable_lfo".into();
    assert!(model.compile_with_wave_banks(&ticket).is_err());
    let mut defaults = patch.clone();
    defaults.nodes[0].content = None;
    assert!(defaults.compile_with_wave_banks(&ticket).is_err());
    let mut parameters = patch.clone();
    parameters.nodes[0].params.insert("position".into(), 0.25);
    parameters.sample_rate = 96000;
    assert!(parameters.compile_with_wave_banks(&ticket).is_ok());
    assert_eq!(cache.stats(), stats);
    let mut actual = [PortFrame::default(); 257];
    let mut expected = [PortFrame::default(); 257];
    engine.process_poly_block(&mut actual);
    baseline.process_poly_block(&mut expected);
    assert_eq!(actual, expected);
}

fn linear(fm: f32, sync: bool) -> Patch {
    let mut patch = simple("wavetable_vco");
    patch.nodes[0].params.insert("fm_mode".into(), 1.0);
    patch.nodes[0].params.insert("fm_amount".into(), 1.0);
    patch.nodes.push(
        serde_json::from_value(json!({"id":"fm","kind":"poly_constant",
        "params":{"channels":1,"v1":fm}}))
        .unwrap(),
    );
    patch.cables.push(
        serde_json::from_value(json!({"id":"fm_wave","from":"fm","to":"wave","input":"fm"}))
            .unwrap(),
    );
    if sync {
        patch.nodes.push(
            serde_json::from_value(json!({"id":"sync_source","kind":"lfo",
            "params":{"frequency":73,"amplitude":1,"offset":0}}))
            .unwrap(),
        );
        patch.nodes.push(
            serde_json::from_value(json!({"id":"sync","kind":"gain",
            "params":{"gain":5}}))
            .unwrap(),
        );
        patch.cables.push(
            serde_json::from_value(
                json!({"id":"sync_gain","from":"sync_source","to":"sync","input":"signal"}),
            )
            .unwrap(),
        );
        patch.cables.push(
            serde_json::from_value(
                json!({"id":"sync_wave","from":"sync","to":"wave","input":"sync"}),
            )
            .unwrap(),
        );
    }
    patch
}

#[test]
fn through_zero_linear_fm_reverses_holds_crosses_signs_and_accepts_sync_at_zero() {
    let mut forward = linear(0.0, false).compile().unwrap();
    let mut backward = linear(-2.0, false).compile().unwrap();
    let mut stopped = linear(-1.0, false).compile().unwrap();
    let mut synced = linear(-1.0, true).compile().unwrap();
    let mut positive = [PortFrame::default(); 2048];
    let mut negative = [PortFrame::default(); 2048];
    let mut held = [PortFrame::default(); 2048];
    let mut reset = [PortFrame::default(); 2048];
    forward.process_poly_block(&mut positive);
    backward.process_poly_block(&mut negative);
    stopped.process_poly_block(&mut held);
    synced.process_poly_block(&mut reset);
    // The cable publishes after frame zero. Subsequent negative FM reverses
    // the sine trajectory; exact cancellation holds the first advanced phase.
    assert!(positive[2].values[0] > 0.0);
    assert!(negative[2].values[0] < 0.0);
    assert!(held[0].values[0].abs() > 0.01);
    assert!(held[1..].iter().all(|frame| frame.values == held[0].values));
    assert!(reset[1900..]
        .iter()
        .all(|frame| frame.values[0].abs() < 0.002));
    assert!(positive
        .iter()
        .chain(&negative)
        .chain(&held)
        .chain(&reset)
        .all(|frame| frame.values.iter().all(|value| value.is_finite())));
    assert!(negative.iter().any(|frame| frame.values[0] > 1.0));
    assert!(negative.iter().any(|frame| frame.values[0] < -1.0));
    let before = positive.last().unwrap().values[0];
    backward.adopt_state_from(&forward);
    backward.process_poly_block(&mut negative[..1]);
    assert!((negative[0].values[0] - before).abs() < 1.0);
    forward.adopt_state_from(&backward);
    forward.process_poly_block(&mut positive);
    assert!(positive.iter().all(|frame| frame.values[0].is_finite()));
}
