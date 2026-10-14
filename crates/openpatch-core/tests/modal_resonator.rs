use openpatch_core::{
    catalog, modal_resonator_modes, Engine, ModuleParameterActions, Node, Patch, PortFrame,
};
use serde_json::json;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
struct Allocator;
thread_local! {static TRACK:Cell<bool>=const {Cell::new(false)};static OPS:Cell<usize>=const {Cell::new(0)};}
fn record() {
    let _ = TRACK.try_with(|t| {
        if t.get() {
            let _ = OPS.try_with(|n| n.set(n.get() + 1));
        }
    });
}
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        record();
        unsafe { System.alloc(l) }
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        record();
        unsafe { System.alloc_zeroed(l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, s: usize) -> *mut u8 {
        record();
        unsafe { System.realloc(p, l, s) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        record();
        unsafe { System.dealloc(p, l) }
    }
}
#[global_allocator]
static ALLOCATOR: Allocator = Allocator;
fn patch() -> Patch {
    serde_json::from_value(json!({"version":2,"sample_rate":48000,
    "nodes":[{"id":"gate","kind":"constant","params":{"value":10}},
    {"id":"force","kind":"poly_constant","params":{"channels":16,"v1":2,"v2":-3,"v16":1}},
    {"id":"body","kind":"modal_resonator"}],
    "cables":[{"id":"hit","from":"gate","to":"body","input":"strike"},{"id":"push","from":"force","to":"body","input":"excite"}],"output":"body"})).unwrap()
}
fn one(e: &mut Engine) -> PortFrame {
    let mut frames = [PortFrame::default()];
    e.process_poly_block(&mut frames);
    frames[0]
}

#[test]
fn wire_roundtrip_catalog_and_validation_agree_with_helper() {
    let p = patch();
    let restored: Patch = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
    restored.compile().unwrap();
    let modules = catalog();
    let spec = modules["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "modal_resonator")
        .unwrap();
    assert_eq!(spec["model"], "physical");
    assert_eq!(spec["inputs"], json!(["pitch", "strike", "excite"]));
    assert_eq!(spec["outputs"], json!(["out"]));
    assert_eq!(spec["min_patch_version"], 2);
    assert_eq!(
        spec["bypass"]["routes"],
        json!([{"input":"excite","output":"out"}])
    );
    assert!(spec["polyphony"]["primary_input"].is_null());
    let modes = modal_resonator_modes(&p.nodes[2], 48000).unwrap();
    assert_eq!(modes.len(), 16);
    assert!(modes[0].active);
    assert!(!modes[15].active);
    let mut legacy = p.clone();
    legacy.version = 1;
    assert!(legacy.compile().is_err());
    for (name, spec) in spec["params"].as_object().unwrap() {
        for value in [
            spec["min"].as_f64().unwrap() as f32 - 1.0,
            spec["max"].as_f64().unwrap() as f32 + 1.0,
            f32::NAN,
            f32::INFINITY,
        ] {
            let mut bad = p.clone();
            bad.nodes[2].params.insert(name.clone(), value);
            assert!(bad.compile().is_err(), "accepted {name}={value}");
            assert!(modal_resonator_modes(&bad.nodes[2], 48000).is_err());
        }
    }
    let mut bad = p.clone();
    bad.nodes[2].params.insert("unknown".into(), 0.0);
    assert!(bad.compile().is_err());
    bad = p.clone();
    bad.sample_rate = 8000;
    bad.nodes[2].params.insert("frequency".into(), 4000.0);
    assert!(bad.compile().is_err());
    for port in ["signal", "velocity", "real"] {
        let mut bad = p.clone();
        bad.output_port = port.into();
        assert!(bad.compile().is_err());
    }
    let invalid: Result<Node, _> =
        serde_json::from_value(json!({"id":"body","kind":"modal_resonator","modes":[1,2]}));
    assert!(invalid.is_err());
}

#[test]
fn unchanged_graph_adoption_matches_continuation_and_replacement_strike_retriggers() {
    let mut p = patch();
    p.cables.truncate(1);
    let mut old = p.compile().unwrap();
    for _ in 0..128 {
        one(&mut old);
    }
    let mut same = p.compile().unwrap();
    same.adopt_state_from(&old);
    for _ in 0..128 {
        assert_eq!(one(&mut same), one(&mut old));
    }
    let mut p_changed = p.clone();
    p_changed.cables[0].id = Some("new-hit".into());
    let mut changed = p_changed.compile().unwrap();
    changed.adopt_state_from(&old);
    let mut normal = p.compile().unwrap();
    normal.adopt_state_from(&old);
    one(&mut changed);
    one(&mut normal);
    assert_ne!(one(&mut changed), one(&mut normal));
    // Excluded module incarnations reset physical state instead of adopting it.
    let mut fresh = p.compile().unwrap();
    let handle = fresh.resolve_module_state("body").unwrap();
    fresh.adopt_state_from_excluding(&old, &[handle]);
    assert_eq!(one(&mut fresh), PortFrame::mono(0.0));
}

#[test]
fn initialize_clears_ring_and_bypass_copies_exact_force_frames() {
    let mut p = patch();
    p.cables.truncate(1);
    let mut e = p.compile().unwrap();
    for _ in 0..20 {
        one(&mut e);
    }
    p.nodes[0].params.insert("value".into(), 0.0);
    let mut released = p.compile().unwrap();
    released.adopt_state_from(&e);
    one(&mut released);
    one(&mut released);
    assert!(one(&mut released).values[0].abs() > 0.001);
    let h = released.resolve_module_state("body").unwrap();
    assert!(released.initialize_module_state(h));
    assert_eq!(one(&mut released), PortFrame::mono(0.0));
    p = patch();
    p.nodes[2].bypassed = true;
    p.nodes[2].params.insert("level".into(), 0.0);
    let mut bypass = p.compile().unwrap();
    assert_eq!(one(&mut bypass), PortFrame::mono(0.0));
    let frame = one(&mut bypass);
    assert_eq!(frame.channels, 16);
    assert_eq!(frame.values[0], 2.0);
    assert_eq!(frame.values[1], -3.0);
    assert_eq!(frame.values[15], 1.0);
    let actions = ModuleParameterActions::new(8000);
    let mut body = p.nodes[2].clone();
    for seed in 0..64 {
        actions.randomize(&mut body, seed).unwrap();
        let mut candidate = Patch::new_v2();
        candidate.sample_rate = 8000;
        candidate.nodes.push(body.clone());
        candidate.compile().unwrap();
    }
    actions.initialize(&mut body).unwrap();
    assert_eq!(body.params["frequency"], 220.0);
    assert_eq!(body.params["level"], 1.0);
}

#[test]
fn first_callback_pitch_recache_adoption_initialize_and_retirement_use_no_heap() {
    let mut p = patch();
    p.nodes.push(
        serde_json::from_value(
            json!({"id":"pitch","kind":"lfo","params":{"frequency":33,"amplitude":1,"offset":0}}),
        )
        .unwrap(),
    );
    p.cables
        .push(serde_json::from_value(json!({"from":"pitch","to":"body","input":"pitch"})).unwrap());
    let mut old = p.compile().unwrap();
    let mut replacement = p.compile().unwrap();
    let mut changed = p.clone();
    changed.nodes[1].params.insert("channels".into(), 1.0);
    changed.nodes[2].params.insert("stiffness".into(), 1.0);
    let mut mono = changed.compile().unwrap();
    let mut frames = [PortFrame::default(); 129];
    let handle = replacement.resolve_module_state("body").unwrap();
    OPS.with(|n| n.set(0));
    TRACK.with(|t| t.set(true));
    for length in [0, 1, 3, 64, 128, 129] {
        old.process_poly_block(&mut frames[..length]);
        replacement.adopt_state_from(&old);
        replacement.process_poly_block(&mut frames[..length]);
        mono.adopt_state_from(&replacement);
        mono.process_poly_block(&mut frames[..length]);
    }
    let initialized = replacement.initialize_module_state(handle);
    TRACK.with(|t| t.set(false));
    assert!(initialized);
    assert_eq!(
        OPS.with(Cell::get),
        0,
        "callback allocated, reallocated or deallocated"
    );
    assert!(frames
        .iter()
        .all(|f| f.values.iter().all(|v| v.is_finite())));
}

#[test]
fn bypass_freezes_mechanical_state_and_resume_matches_uninterrupted_next_sample() {
    let mut p = patch();
    p.cables.truncate(1);
    let mut old = p.compile().unwrap();
    for _ in 0..127 {
        one(&mut old);
    }
    let mut expected = p.compile().unwrap();
    expected.adopt_state_from(&old);
    let mut bypassed = p.clone();
    bypassed.nodes[2].bypassed = true;
    let mut frozen = bypassed.compile().unwrap();
    frozen.adopt_state_from(&old);
    for _ in 0..257 {
        assert_eq!(one(&mut frozen), PortFrame::mono(0.0));
    }
    let mut resumed = p.compile().unwrap();
    resumed.adopt_state_from(&frozen);
    for _ in 0..64 {
        assert_eq!(one(&mut resumed), one(&mut expected));
    }
}
