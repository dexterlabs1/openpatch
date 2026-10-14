//! Allocation tracking is thread-local so parallel test harness work is ignored.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct TrackingAllocator;
thread_local! {
    static TRACKING: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
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
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: TrackingAllocator = TrackingAllocator;

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
