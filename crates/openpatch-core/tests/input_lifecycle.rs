//! Tests the production input edge effect, without a DSP or SDK mirror.
use openpatch_core::{InputLifecycleTransform as Transform, OutputLifecycleTransform};
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

const EDGES: [Transform; 3] = [
    Transform::identity(),
    Transform::connect(),
    Transform::disconnect(),
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
fn elementary_effects_cover_all_seventeen_incoming_counts() {
    assert_eq!(Transform::default(), Transform::identity());
    assert_eq!(EDGES[0].effect(), (false, None));
    assert_eq!(EDGES[1].effect(), (false, Some(1)));
    assert_eq!(EDGES[2].effect(), (true, Some(0)));
    for initial_count in 0..=16_u8 {
        for (edge, expected_count) in [(EDGES[0], initial_count), (EDGES[1], 1), (EDGES[2], 0)] {
            let mut count = initial_count;
            let mut values = raw_values();
            edge.apply(&mut count, &mut values).unwrap();
            assert_eq!(count, expected_count);
        }
    }
}

#[test]
fn composed_six_edge_words_match_sequential_effects_for_every_count() {
    // Identity padding includes all shorter words. Apply edge laws directly to
    // the reference storage, independently of the production effect fields.
    for word in 0..3_usize.pow(6) {
        for initial_count in 0..=16_u8 {
            let mut transform = Transform::identity();
            let mut expected_count = initial_count;
            let mut expected = raw_values();
            let mut remaining = word;
            for _ in 0..6 {
                let edge = remaining % 3;
                remaining /= 3;
                transform = transform.then(EDGES[edge]);
                match edge {
                    0 => {}
                    1 => expected_count = 1,
                    2 => {
                        expected.fill(0.0);
                        expected_count = 0;
                    }
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
fn reconnect_seeds_mono_without_erasing_the_previous_reset() {
    let reconnect = Transform::disconnect().then(Transform::connect());
    let connect_then_disconnect = Transform::connect().then(Transform::disconnect());
    assert_eq!(reconnect.effect(), (true, Some(1)));
    assert_eq!(connect_then_disconnect.effect(), (true, Some(0)));
    assert_ne!(reconnect, connect_then_disconnect);
    assert_eq!(reconnect.then(Transform::identity()), reconnect);
    assert_eq!(
        Transform::disconnect().then(Transform::disconnect()),
        Transform::disconnect()
    );
    for initial_count in 0..=16_u8 {
        let mut count = initial_count;
        let mut values = raw_values();
        reconnect.apply(&mut count, &mut values).unwrap();
        assert_eq!(count, 1);
        assert_eq!(bits(values), [0; 16]);
    }
}

#[test]
fn input_disconnect_clears_all_storage_unlike_output_active_prefix_reset() {
    for initial_count in 0..=16_u8 {
        let mut input_count = initial_count;
        let mut input = [7.0; 16];
        Transform::disconnect()
            .apply(&mut input_count, &mut input)
            .unwrap();
        assert_eq!(input_count, 0);
        assert_eq!(bits(input), [0; 16]);

        let mut output_count = initial_count;
        let mut output = [7.0; 16];
        OutputLifecycleTransform::bypass_edge()
            .apply(&mut output_count, &mut output)
            .unwrap();
        assert!(output[..usize::from(initial_count)]
            .iter()
            .all(|value| value.to_bits() == 0));
        assert!(output[usize::from(initial_count)..]
            .iter()
            .all(|value| value.to_bits() == 7.0_f32.to_bits()));
        assert_eq!(output_count, u8::from(initial_count != 0));
    }
}

#[test]
fn identity_and_connect_preserve_all_raw_bits_disconnect_clears_to_positive_zero() {
    for initial_count in 0..=16_u8 {
        for transform in [Transform::identity(), Transform::connect()] {
            let mut count = initial_count;
            let mut values = raw_values();
            transform.apply(&mut count, &mut values).unwrap();
            assert_eq!(bits(values), bits(raw_values()));
        }
        let mut count = initial_count;
        let mut values = raw_values();
        Transform::disconnect()
            .apply(&mut count, &mut values)
            .unwrap();
        assert_eq!(bits(values), [0; 16]);
    }
}

#[test]
fn every_invalid_count_is_rejected_before_raw_storage_or_count_changes() {
    for invalid in 17..=u8::MAX {
        for transform in EDGES
            .into_iter()
            .chain([Transform::disconnect().then(Transform::connect())])
        {
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
            .then(Transform::disconnect())
            .then(Transform::connect());
        let mut values = raw_values();
        let mut count = initial_count;
        accepted &= transform.apply(&mut count, &mut values).is_ok();
        count = 255;
        accepted &= transform.apply(&mut count, &mut values).is_err();
        std::hint::black_box((transform, transform.effect(), count, values));
    }
    TRACKING.with(|tracking| tracking.set(false));
    assert!(accepted);
    assert_eq!(MEMORY_OPERATIONS.with(Cell::get), 0);
    assert!(!std::mem::needs_drop::<Transform>());
}
