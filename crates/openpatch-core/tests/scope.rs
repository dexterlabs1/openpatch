use openpatch_core::{catalog, Engine, Patch, PortFrame};
use serde_json::json;
fn patch(bypassed: bool, channels: u8) -> Patch {
    serde_json::from_value(json!({"version":2,"sample_rate":51200,"nodes":[{"id":"x","kind":"poly_constant","params":{"channels":channels,"v1":12.0,"v16":-14.0}},{"id":"s","kind":"scope","bypassed":bypassed,"params":{"time":0.005}}],"cables":[{"from":"x","to":"s","input":"x"}],"output":"s","output_port":"x"})).unwrap()
}
fn tick(e: &mut Engine) -> PortFrame {
    let mut f = [PortFrame::default()];
    e.process_poly_block(&mut f);
    f[0]
}
#[test]
fn public_scope_raw_passthrough_bypass_and_stale_handles() {
    for bypassed in [false, true] {
        for channels in [0, 1, 16] {
            let mut e = patch(bypassed, channels).compile().unwrap();
            let h = e.resolve_module_scope("s").unwrap();
            tick(&mut e);
            let out = tick(&mut e);
            assert_eq!(out.channels, channels);
            if channels > 0 {
                assert_eq!(out.values[0], 12.0);
            }
            if channels == 16 {
                assert_eq!(out.values[15], -14.0);
            }
            let s = e.module_scope(h).unwrap();
            assert_eq!(s.buffer_index, if bypassed { 0 } else { 2 });
            assert!(patch(false, 1).compile().unwrap().module_scope(h).is_none());
        }
    }
}
#[test]
fn catalog_contract_and_validation() {
    let c = catalog();
    let s = c["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "scope")
        .unwrap();
    assert_eq!(s["min_patch_version"], 2);
    assert_eq!(s["outputs"], json!(["x", "y"]));
    assert_eq!(s["inputs"], json!(["x", "y", "trigger"]));
    for (name, value) in [
        ("wat", 0.0),
        ("x_scale", 0.5),
        ("xy", 0.5),
        ("time", 0.0),
        ("threshold", 11.0),
    ] {
        let mut p = patch(false, 1);
        p.nodes[1].params.insert(name.into(), value);
        assert!(p.compile().is_err());
    }
    let mut p = patch(false, 1);
    p.version = 1;
    assert!(p.compile().is_err());
}
#[test]
fn processing_partition_preserves_capture() {
    let p = patch(false, 16);
    let mut a = p.compile().unwrap();
    let mut b = p.compile().unwrap();
    let ah = a.resolve_module_scope("s").unwrap();
    let bh = b.resolve_module_scope("s").unwrap();
    let mut out = vec![PortFrame::default(); 1025];
    a.process_poly_block(&mut out);
    for width in [1, 256, 17, 751] {
        b.process_poly_block(&mut out[..width]);
    }
    let a = a.module_scope(ah).unwrap();
    let b = b.module_scope(bh).unwrap();
    assert_eq!(a.x, b.x);
    assert_eq!(a.y, b.y);
    assert_eq!(a.buffer_index, b.buffer_index);
    assert_eq!(a.sample_index, b.sample_index);
}

#[test]
fn adoption_resets_for_acquisition_routes_rate_and_initialization() {
    let p = patch(false, 1);
    let mut old = p.compile().unwrap();
    tick(&mut old);
    tick(&mut old);
    tick(&mut old);
    for mode in 0..7 {
        let mut p = p.clone();
        match mode {
            0 => {
                p.nodes[1].params.insert("x_scale".into(), 3.0);
            }
            1 => {
                p.nodes[1].params.insert("time".into(), 0.01);
            }
            2 => {
                p.nodes[1].params.insert("threshold".into(), 1.0);
            }
            3 => {
                p.nodes[1].params.insert("xy".into(), 1.0);
            }
            4 => {
                p.nodes[1].params.insert("trigger_enabled".into(), 1.0);
            }
            5 => {
                p.cables.clear();
            }
            _ => p.sample_rate = 48000,
        };
        let mut new = p.compile().unwrap();
        let h = new.resolve_module_scope("s").unwrap();
        new.adopt_state_from(&old);
        assert_eq!(
            new.module_scope(h).unwrap().buffer_index,
            if mode == 0 { 3 } else { 0 }
        );
        assert!(new.initialize_module_state(h));
        assert_eq!(new.module_scope(h).unwrap().buffer_index, 0);
    }
}
#[test]
fn bypass_adoption_freezes_and_resume_preserves_capture() {
    let mut old = patch(false, 1).compile().unwrap();
    for _ in 0..3 {
        tick(&mut old);
    }
    let mut bypassed = patch(true, 1).compile().unwrap();
    bypassed.adopt_state_from(&old);
    let h = bypassed.resolve_module_scope("s").unwrap();
    for _ in 0..20 {
        tick(&mut bypassed);
    }
    assert_eq!(bypassed.module_scope(h).unwrap().buffer_index, 3);
    let mut resumed = patch(false, 1).compile().unwrap();
    resumed.adopt_state_from(&bypassed);
    let h = resumed.resolve_module_scope("s").unwrap();
    tick(&mut resumed);
    assert_eq!(resumed.module_scope(h).unwrap().buffer_index, 4);
}
