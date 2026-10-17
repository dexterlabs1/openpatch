//! The counter is thread-local so parallel harness allocations are not counted.
use openpatch_core::{Engine, Patch, PortFrame};
use serde_json::json;
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

fn voice(rate: u32, width: u8) -> Patch {
    let mut params = serde_json::Map::from_iter([("channels".into(), json!(width))]);
    for lane in 1..=16 {
        params.insert(format!("v{lane}"), json!((lane as f32 - 8.) / 12.));
    }
    let mut patch: Patch = serde_json::from_value(json!({"version":2,"sample_rate":rate,"nodes":[
        {"id":"pitch","kind":"poly_constant","params":params},
        {"id":"vco","kind":"rack264_vco"},
        {"id":"vcf","kind":"rack264_vcf","params":{"frequency":0.65,"resonance":0.2,"drive":0.1}},
        {"id":"gate","kind":"constant","params":{"value":10}},
        {"id":"envelope","kind":"adsr_eg"},
        {"id":"vca","kind":"stock_vca","params":{"level":0.7}}
    ],"cables":[
        {"from":"pitch","to":"vco","input":"pitch"},
        {"from":"vco","output":"saw","to":"vcf","input":"signal"},
        {"from":"gate","to":"envelope","input":"gate"},
        {"from":"envelope","output":"out","to":"vca","input":"cv"},
        {"from":"vcf","output":"lowpass","to":"vca","input":"signal"}
    ],"output":"vca","output_port":"out"}))
    .unwrap();
    patch.assign_missing_cable_ids().unwrap();
    patch
}
fn render(engine: &mut Engine) -> [PortFrame; 257] {
    let mut frames = [PortFrame::default(); 257];
    engine.process_poly_block(&mut frames);
    frames
}
#[test]
fn actual_engine_renders_the_versioned_voice_with_mono_and_sixteen_distinct_lanes() {
    for rate in [44100, 48000, 96000] {
        for width in [1, 16] {
            let mut engine = voice(rate, width).compile().unwrap();
            let mut frames = [PortFrame::default(); 4096];
            engine.process_poly_block(&mut frames);
            assert!(frames
                .iter()
                .all(|frame| frame.values.iter().all(|sample| sample.is_finite())));
            // Engine-v2 carries channel width across each cable with the same
            // one-sample causal delay as voltage: pitch → VCO → VCF → VCA.
            assert!(
                frames[3..].iter().all(|frame| frame.channels == width),
                "rate{rate}, width{width}; widths {:?}",
                frames[..12]
                    .iter()
                    .map(|frame| frame.channels)
                    .collect::<Vec<_>>()
            );
            for lane in 0..usize::from(width) {
                let rms = (frames
                    .iter()
                    .map(|frame| frame.values[lane].powi(2))
                    .sum::<f32>()
                    / 4096.)
                    .sqrt();
                assert!(rms > 0.001, "rate{rate},lane{lane}: silent voice ({rms})");
            }
            if width == 16 {
                assert!(frames
                    .iter()
                    .any(|frame| frame.values[0] != frame.values[15]));
            }
        }
    }
}
#[test]
fn same_identity_adoption_keeps_both_processors_noise_history_and_voice_timing() {
    let patch = voice(48000, 16);
    let mut engine = patch.compile().unwrap();
    render(&mut engine);
    let mut adopted = patch.compile().unwrap();
    adopted.adopt_state_from(&engine);
    assert_eq!(render(&mut engine), render(&mut adopted));
    let encoded = serde_json::to_vec(&patch).unwrap();
    let reopened: Patch = serde_json::from_slice(&encoded).unwrap();
    let mut fresh = reopened.compile().unwrap();
    let mut original = patch.compile().unwrap();
    assert_eq!(render(&mut fresh), render(&mut original));
}
#[test]
fn voice_callback_and_state_adoption_do_not_allocate_or_deallocate() {
    let patch = voice(48000, 16);
    let mut engine = patch.compile().unwrap();
    let mut adopted = patch.compile().unwrap();
    let mut frames = [PortFrame::default(); 257];
    MEMORY_OPERATIONS.with(|value| value.set(0));
    TRACKING.with(|value| value.set(true));
    engine.process_poly_block(&mut frames);
    adopted.adopt_state_from(&engine);
    adopted.process_poly_block(&mut frames);
    TRACKING.with(|value| value.set(false));
    let operations = MEMORY_OPERATIONS.with(Cell::get);
    assert_eq!(operations, 0);
}
