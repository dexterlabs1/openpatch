use openpatch_core::{catalog, Cable, Node, Patch};
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

fn node(id: &str, kind: &str, params: &[(&str, f32)]) -> Node {
    Node {
        content: None,
        score: None,
        bypassed: false,
        id: id.into(),
        kind: kind.into(),
        params: params.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
    }
}
fn cable(from: &str, to: &str, input: &str) -> Cable {
    Cable {
        input_order: None,
        output_order: None,
        color: None,
        id: None,
        from: from.into(),
        to: to.into(),
        input: input.into(),
        output: "out".into(),
    }
}
fn utilities_patch() -> Patch {
    Patch {
        sample_rate: 8000,
        nodes: vec![
            node("noise", "white_noise", &[("seed", 23.0)]),
            node("clock", "gate", &[("frequency", 100.0)]),
            node("held", "sample_hold", &[]),
            node("quantized", "quantizer", &[("scale_mask", 2741.0)]),
            node(
                "attenuated",
                "attenuverter",
                &[("gain", -0.5), ("offset", 0.2)],
            ),
            node("reference", "constant", &[("value", 2.0)]),
            node("mixed", "crossfade", &[("mix", 0.2), ("law", 1.0)]),
        ],
        cables: vec![
            cable("noise", "held", "signal"),
            cable("clock", "held", "trigger"),
            cable("held", "quantized", "pitch"),
            cable("quantized", "attenuated", "signal"),
            cable("attenuated", "mixed", "a"),
            cable("reference", "mixed", "b"),
        ],
        output: Some("mixed".into()),
        ..Patch::default()
    }
}
#[test]
fn utility_catalog_defaults_compile_and_parameter_contracts_reject_invalid_data() {
    let kinds = [
        "quantizer",
        "sample_hold",
        "white_noise",
        "attenuverter",
        "crossfade",
    ];
    let catalog = catalog();
    for kind in kinds {
        let entry = catalog["modules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["kind"] == kind)
            .unwrap();
        assert_eq!(entry["model"], "functional");
        let mut patch = Patch {
            nodes: vec![node("unit", kind, &[])],
            output: Some("unit".into()),
            ..Patch::default()
        };
        let encoded = serde_json::to_string(&patch).unwrap();
        patch = serde_json::from_str(&encoded).unwrap();
        let mut output = [0.0; 257];
        patch.compile().unwrap().process_block(&mut output);
        assert!(output.iter().all(|sample| sample.is_finite()));
    }
    for (kind, key, value) in [
        ("quantizer", "scale_mask", 0.5),
        ("quantizer", "scale_mask", 4096.0),
        ("quantizer", "offset", 4.1),
        ("sample_hold", "initial", 17.0),
        ("white_noise", "seed", 1.5),
        ("white_noise", "seed", 16777216.0),
        ("white_noise", "level", -1.0),
        ("attenuverter", "gain", 1.1),
        ("attenuverter", "offset", f32::NAN),
        ("crossfade", "mix", 1.1),
        ("crossfade", "law", 0.5),
    ] {
        let patch = Patch {
            nodes: vec![node("unit", kind, &[(key, value)])],
            ..Patch::default()
        };
        assert!(patch.validate().is_err(), "{kind}.{key}={value}");
    }
    let unknown = Patch {
        nodes: vec![node("unit", "quantizer", &[("unknown", 1.0)])],
        ..Patch::default()
    };
    assert!(unknown
        .validate()
        .unwrap_err()
        .contains("unknown parameter"));
}
#[test]
fn utilities_are_independent_of_callback_boundaries_and_graph_order() {
    let patch = utilities_patch();
    let mut baseline = patch.compile().unwrap();
    let mut expected = vec![0.0; 4099];
    baseline.process_block(&mut expected);
    for length in [1, 3, 64, 128, 129, 513] {
        let mut reordered = patch.clone();
        reordered.nodes.reverse();
        reordered.cables.reverse();
        let mut engine = reordered.compile().unwrap();
        let mut actual = vec![0.0; expected.len()];
        for chunk in actual.chunks_mut(length) {
            engine.process_block(chunk);
        }
        assert_eq!(actual, expected, "callback length={length}");
    }
}
#[test]
fn utility_state_adoption_preserves_sequence_and_hold_but_new_seed_restarts() {
    let patch = utilities_patch();
    let mut old = patch.compile().unwrap();
    old.process_block(&mut [0.0; 113]);
    let mut same = patch.compile().unwrap();
    same.adopt_state_from(&old);
    let mut expected = [0.0; 391];
    let mut actual = [0.0; 391];
    old.process_block(&mut expected);
    same.process_block(&mut actual);
    assert_eq!(actual, expected);

    let noise_patch = Patch {
        nodes: vec![node("noise", "white_noise", &[("seed", 3.0)])],
        output: Some("noise".into()),
        ..Patch::default()
    };
    let mut previous = noise_patch.compile().unwrap();
    previous.process_block(&mut [0.0; 17]);
    let mut changed = noise_patch.clone();
    changed.nodes[0].params.insert("seed".into(), 4.0);
    let mut adopted = changed.compile().unwrap();
    let mut fresh = changed.compile().unwrap();
    adopted.adopt_state_from(&previous);
    adopted.process_block(&mut actual);
    fresh.process_block(&mut expected);
    assert_eq!(actual, expected);
}
#[test]
fn unpatched_attenuverter_and_empty_quantizer_have_documented_normalizations() {
    let patch = Patch {
        nodes: vec![node(
            "unit",
            "attenuverter",
            &[("gain", 0.5), ("offset", -2.0)],
        )],
        output: Some("unit".into()),
        ..Patch::default()
    };
    let mut output = [0.0; 32];
    patch.compile().unwrap().process_block(&mut output);
    assert_eq!(output, [3.0; 32]);
    let mut empty = patch.clone();
    empty.nodes[0] = node(
        "unit",
        "quantizer",
        &[("scale_mask", 0.0), ("offset", 0.25)],
    );
    empty.compile().unwrap().process_block(&mut output);
    assert_eq!(output, [0.25; 32]);
}
#[test]
fn first_utility_callback_and_state_adoption_allocate_nothing() {
    let patch = utilities_patch();
    let mut old = patch.compile().unwrap();
    let mut replacement = patch.compile().unwrap();
    let mut samples = [0.0; 1025];
    ALLOCATIONS.with(|count| count.set(0));
    TRACKING.with(|enabled| enabled.set(true));
    for length in [0, 1, 3, 64, 127, 128, 129, 1025] {
        old.process_block(&mut samples[..length]);
    }
    replacement.adopt_state_from(&old);
    replacement.process_block(&mut samples);
    TRACKING.with(|enabled| enabled.set(false));
    assert_eq!(ALLOCATIONS.with(Cell::get), 0);
    assert!(samples.iter().all(|sample| sample.is_finite()));
}
