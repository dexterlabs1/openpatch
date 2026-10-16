//! Tests the production transform independently of any module or SDK adapter.
use openpatch_core::{OutputLifecycleEntry, OutputLifecycleTransform as Transform};
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

const EDGES: [Transform; 4] = [
    Transform::identity(),
    Transform::connect(),
    Transform::disconnect(),
    Transform::bypass_edge(),
];

fn raw_values() -> [f32; 16] {
    [
        -0.0,
        0.0,
        f32::from_bits(0x7fc1_2345),
        f32::from_bits(0xffc5_4321),
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::MAX,
        -f32::MAX,
        1.0,
        -1.0,
        f32::MIN_POSITIVE,
        -f32::MIN_POSITIVE,
        f32::from_bits(1),
        f32::from_bits(0x8000_0001),
        42.0,
        -42.0,
    ]
}

fn bits(values: [f32; 16]) -> [u32; 16] {
    values.map(f32::to_bits)
}

#[test]
fn elementary_entries_cover_all_seventeen_counts() {
    assert_eq!(Transform::default(), Transform::identity());
    for count in 0..=16_u8 {
        assert_eq!(
            EDGES[0].entry(count).unwrap(),
            OutputLifecycleEntry {
                final_count: count,
                clear_mask: 0,
            }
        );
        assert_eq!(
            EDGES[1].entry(count).unwrap(),
            OutputLifecycleEntry {
                final_count: 1,
                clear_mask: 0,
            }
        );
        assert_eq!(
            EDGES[2].entry(count).unwrap(),
            OutputLifecycleEntry {
                final_count: 0,
                clear_mask: 0,
            }
        );
        assert_eq!(
            EDGES[3].entry(count).unwrap(),
            OutputLifecycleEntry {
                final_count: u8::from(count != 0),
                clear_mask: ((1_u32 << count) - 1) as u16,
            }
        );
    }
    assert_eq!(EDGES[3].entries()[16].clear_mask, u16::MAX);
}

#[test]
fn composition_matches_sequential_edges_for_all_counts_and_five_edge_words() {
    // Identity padding includes every shorter word, too. The reference applies
    // the edge laws directly to storage instead of reconstructing a table.
    for word in 0..4_usize.pow(5) {
        for initial_count in 0..=16_u8 {
            let mut transform = Transform::identity();
            let mut expected_count = initial_count;
            let mut expected = raw_values();
            let mut remaining = word;
            for _ in 0..5 {
                let edge = remaining % 4;
                remaining /= 4;
                transform = transform.then(EDGES[edge]);
                match edge {
                    0 => {}
                    1 => expected_count = 1,
                    2 => expected_count = 0,
                    3 if expected_count != 0 => {
                        expected[..usize::from(expected_count)].fill(0.0);
                        expected_count = 1;
                    }
                    3 => {}
                    _ => unreachable!(),
                }
            }
            let mut actual_count = initial_count;
            let mut actual = raw_values();
            transform.apply(&mut actual_count, &mut actual).unwrap();
            assert_eq!(actual_count, expected_count, "word {word}");
            assert_eq!(bits(actual), bits(expected), "word {word}");
        }
    }
}

#[test]
fn composition_is_associative_with_two_sided_identity() {
    let mut transforms = vec![Transform::identity()];
    transforms.extend(EDGES);
    for first in EDGES {
        for second in EDGES {
            transforms.push(first.then(second));
        }
    }
    for &first in &transforms {
        assert_eq!(Transform::identity().then(first), first);
        assert_eq!(first.then(Transform::identity()), first);
        for &second in &transforms {
            for &third in &transforms {
                assert_eq!(
                    first.then(second).then(third),
                    first.then(second.then(third))
                );
            }
        }
    }
}

#[test]
fn reconnect_and_bypass_order_preserve_irreversible_clears() {
    let disconnected_then_bypass = Transform::disconnect().then(Transform::bypass_edge());
    let bypass_then_disconnected = Transform::bypass_edge().then(Transform::disconnect());
    assert_eq!(disconnected_then_bypass.entry(16).unwrap().clear_mask, 0);
    assert_eq!(
        bypass_then_disconnected.entry(16).unwrap().clear_mask,
        u16::MAX
    );
    assert_ne!(disconnected_then_bypass, bypass_then_disconnected);

    // Both end connected with count one, but reconnect-before-bypass clears
    // just lane zero; bypass-before-reconnect clears all initially active lanes.
    let reconnect = Transform::disconnect().then(Transform::connect());
    assert_eq!(
        reconnect.then(Transform::bypass_edge()).entry(16).unwrap(),
        OutputLifecycleEntry {
            final_count: 1,
            clear_mask: 1,
        }
    );
    assert_eq!(
        Transform::bypass_edge().then(reconnect).entry(16).unwrap(),
        OutputLifecycleEntry {
            final_count: 1,
            clear_mask: u16::MAX,
        }
    );
    // Crossing bypass twice cannot restore voltages cleared on the first edge.
    assert_eq!(
        Transform::bypass_edge().then(Transform::bypass_edge()),
        Transform::bypass_edge()
    );
}

#[test]
fn clears_are_positive_zero_and_other_raw_bits_remain_untouched() {
    for count in 0..=16_u8 {
        let before = raw_values();
        let mut values = before;
        let mut actual_count = count;
        Transform::bypass_edge()
            .apply(&mut actual_count, &mut values)
            .unwrap();
        for lane in 0..16 {
            assert_eq!(
                values[lane].to_bits(),
                if lane < usize::from(count) {
                    0
                } else {
                    before[lane].to_bits()
                }
            );
        }
    }
}

#[test]
fn every_invalid_external_count_is_rejected_before_mutation() {
    for invalid in 17..=u8::MAX {
        for transform in EDGES {
            assert_eq!(transform.entry(invalid).unwrap_err().count, invalid);
            let mut count = invalid;
            let mut values = raw_values();
            assert_eq!(
                transform.apply(&mut count, &mut values).unwrap_err().count,
                invalid
            );
            assert_eq!(count, invalid);
            assert_eq!(bits(values), bits(raw_values()));
        }
    }
}

#[test]
fn first_construction_composition_application_and_retirement_touch_no_heap() {
    MEMORY_OPERATIONS.with(|count| count.set(0));
    TRACKING.with(|tracking| tracking.set(true));
    let mut accepted = true;
    for initial_count in 0..=16 {
        let transform = Transform::default()
            .then(Transform::bypass_edge())
            .then(Transform::disconnect())
            .then(Transform::connect());
        let mut values = raw_values();
        let mut count = initial_count;
        accepted &= transform.apply(&mut count, &mut values).is_ok();
        accepted &= transform.entry(255).is_err();
        std::hint::black_box((transform, count, values));
    }
    TRACKING.with(|tracking| tracking.set(false));
    assert!(accepted);
    assert_eq!(MEMORY_OPERATIONS.with(Cell::get), 0);
    assert!(!std::mem::needs_drop::<Transform>());
}
