//! The counter is thread-local so parallel harness allocations are not counted.
use openpatch_core::{Patch, PortFrame, TapSample};
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

fn patch() -> Patch {
    serde_json::from_value(json!({
        "version":2,"sample_rate":48000,
        "nodes":[
            {"id":"audio","kind":"poly_constant","params":{"channels":16,"v1":2,"v2":-1,"v5":3,"v16":-2}},
            {"id":"cv","kind":"lfo","params":{"frequency":7,"amplitude":0.8,"offset":0}},
            {"id":"clock","kind":"gate","params":{"frequency":13,"duty":0.2}},
            {"id":"delay","kind":"stock_delay","params":{"time":0.3,"feedback":0.8,"time_cv_amount":0.8,"tone_cv_amount":-0.7}},
            {"id":"other","kind":"stock_delay","params":{"time":0.6,"feedback":0.7,"mix":0.3}}
        ],
        "cables":[
            {"id":"audio_delay","from":"audio","to":"delay","input":"signal"},
            {"id":"audio_other","from":"audio","to":"other","input":"signal"},
            {"id":"cv_delay","from":"cv","to":"delay","input":"time_cv"},
            {"id":"tone_delay","from":"cv","to":"delay","input":"tone_cv"},
            {"id":"clock_delay","from":"clock","to":"delay","input":"clock"},
            {"id":"clock_other","from":"clock","to":"other","input":"clock"}
        ],
        "output":"delay","output_port":"mix"
    })).unwrap()
}

#[test]
fn first_processing_taps_initialization_and_ownership_handoffs_do_not_touch_heap() {
    let patch = patch();
    let mut first = patch.compile().unwrap();
    let mut second = patch.compile().unwrap();
    let mut third = patch.compile().unwrap();
    let mut changed = patch.clone();
    changed.nodes[0].params.insert("channels".into(), 1.0);
    changed.nodes[3].params.insert("mix".into(), 0.0);
    let mut fourth = changed.compile().unwrap();
    let second_taps = [
        Some(second.resolve_tap("delay", "mix", 0).unwrap()),
        Some(second.resolve_tap("other", "wet", 0).unwrap()),
    ];
    let third_module = third.resolve_module_state("delay").unwrap();
    let third_cable = third.resolve_cable_state("audio_delay").unwrap();
    let fourth_module = fourth.resolve_module_state("delay").unwrap();
    let mut frames = [PortFrame::default(); 129];
    let mut audio = [[0.0; 2]; 129];
    let mut taps = [[TapSample::default(); 2]; 129];
    let mut accepted = true;

    // Every graph, handle, tap, exclusion and output buffer exists before the
    // counter starts. Do not format assertions or retire graphs inside the guard.
    MEMORY_OPERATIONS.with(|count| count.set(0));
    TRACKING.with(|tracking| tracking.set(true));
    for length in [0, 1, 3, 64, 127, 128, 129] {
        first.process_poly_block(&mut frames[..length]);
    }
    accepted &= second.handoff_state_from(&mut first);
    second.process_audio_block_with_taps(&mut audio, &second_taps, &mut taps);
    accepted &= third.handoff_state_from_excluding_routes(&mut second, &[], &[third_cable]);
    third.process_poly_block(&mut frames);
    accepted &= third.initialize_module_state(third_module);
    // A fresh first Delay must not prevent the other Delay's ownership transfer.
    accepted &= fourth.handoff_state_from_excluding_routes(&mut third, &[fourth_module], &[]);
    for length in [0, 1, 3, 64, 127, 128, 129] {
        fourth.process_poly_block(&mut frames[..length]);
    }
    TRACKING.with(|tracking| tracking.set(false));

    assert!(accepted);
    assert_eq!(
        MEMORY_OPERATIONS.with(Cell::get),
        0,
        "callback allocated, reallocated or deallocated"
    );
    assert!(frames
        .iter()
        .all(|frame| frame.values.iter().all(|value| value.is_finite())));
    assert!(taps
        .iter()
        .all(|frame| frame.iter().all(|tap| tap.active && tap.value.is_finite())));
    // The donor graphs remain alive until after tracking is disabled. Their
    // destruction is worker-side retirement, outside the audio callback contract.
}
