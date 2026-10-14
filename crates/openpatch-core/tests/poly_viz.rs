use openpatch_core::*;
use serde_json::json;
fn patch(channels: u8, value: f32) -> Patch {
    serde_json::from_value(json!({"version":2,"sample_rate":48000,"nodes":[
        {"id":"source","kind":"poly_constant","params":{"channels":channels,"v1":value,"v16":-10}},
        {"id":"viz","kind":"poly_viz"}],"cables":[{"from":"source","to":"viz","input":"signal"}]}))
    .unwrap()
}
fn lights(e: &Engine) -> ModuleLightState {
    e.module_lights(e.resolve_module_lights("viz").unwrap())
        .unwrap()
}
#[test]
fn true_zero_outputs_and_validation() {
    let p = patch(1, 10.0);
    for position in 0..=2 {
        let mut p = p.clone();
        p.nodes.push(
            serde_json::from_value(
                json!({"id":"other","kind":"poly_constant","params":{"channels":1,"v1":3}}),
            )
            .unwrap(),
        );
        let viz = p.nodes.remove(1);
        p.nodes.insert(position, viz);
        p.output = Some("other".into());
        let mut e = p.compile().unwrap();
        assert!(e.resolve_tap("viz", "out", 0).is_err());
        assert!(e.resolve_tap("viz", "missing", 0).is_err());
        let tap = e.resolve_tap("source", "out", 0).unwrap();
        let mut out = [PortFrame::default(); 16];
        e.process_poly_block(&mut out);
        assert!(out.iter().all(|f| f.values[0] == 3.0));
        assert_eq!(lights(&e).input_channels, Some(1));
        std::hint::black_box(tap);
    }
    let mut p = p.clone();
    p.output = Some("viz".into());
    assert!(p.compile().is_err());
    p.output = None;
    p.cables[0].from = "viz".into();
    assert!(p.compile().is_err());
    p.cables.clear();
    p.nodes[1].params.insert("level".into(), 1.0);
    assert!(p.compile().is_err());
    let mut empty = Patch::new_v2().compile().unwrap();
    let mut out = [PortFrame::mono(99.0); 16];
    empty.process_poly_block(&mut out);
    assert_eq!(out, [PortFrame::default(); 16]);
    let mut only = patch(0, 0.0);
    only.nodes.remove(0);
    only.cables.clear();
    let mut e = only.compile().unwrap();
    e.process_poly_block(&mut out);
    assert_eq!(out, [PortFrame::default(); 16]);
}
#[test]
fn divider_signed_targets_partition_and_retirement() {
    for channels in [0, 1, 16] {
        let p = patch(channels, 10.0);
        let mut a = p.compile().unwrap();
        let mut b = p.compile().unwrap();
        a.process_poly_block(&mut [PortFrame::default(); 15]);
        assert_eq!(lights(&a).input_channels, Some(0));
        a.process_poly_block(&mut [PortFrame::default(); 1]);
        for _ in 0..16 {
            b.process_poly_block(&mut [PortFrame::default(); 1]);
        }
        assert_eq!(lights(&a), lights(&b));
        assert_eq!(lights(&a).input_channels, Some(channels));
        if channels > 0 {
            assert_eq!(lights(&a).brightness[0], [1.0, -0.01]);
        }
        if channels == 16 {
            assert_eq!(lights(&a).brightness[15], [-0.01, 1.0]);
        }
        let mut retired = patch(0, 0.0).compile().unwrap();
        retired.adopt_state_from(&a);
        retired.process_poly_block(&mut [PortFrame::default(); 16]);
        assert_eq!(lights(&retired).input_channels, Some(0));
        if channels > 0 {
            assert_eq!(lights(&retired).brightness[0], [0.99, 0.0]);
        }
    }
}
#[test]
fn adoption_freeze_and_rate_reset() {
    let p = patch(1, 10.0);
    let mut old = p.compile().unwrap();
    old.process_poly_block(&mut [PortFrame::default(); 15]);
    let mut next = p.compile().unwrap();
    next.adopt_state_from(&old);
    next.process_poly_block(&mut [PortFrame::default(); 1]);
    assert_eq!(lights(&next).brightness[0], [1.0, -0.01]);
    let mut muted = p.clone();
    muted.nodes[1].bypassed = true;
    let mut muted = muted.compile().unwrap();
    muted.adopt_state_from(&next);
    muted.process_poly_block(&mut [PortFrame::default(); 1024]);
    assert_eq!(lights(&muted), lights(&next));
    let mut rate = p.clone();
    rate.sample_rate = 96000;
    let mut rate = rate.compile().unwrap();
    rate.adopt_state_from(&next);
    assert_eq!(lights(&rate).input_channels, Some(0));
    assert_eq!(lights(&rate).brightness, [[0.0; 2]; 16]);
    let catalog = catalog();
    let viz = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["kind"] == "poly_viz")
        .unwrap();
    assert_eq!(viz["outputs"], json!([]));
    assert_eq!(viz["params"], json!({}));
    assert_eq!(viz["min_patch_version"], 2);
}

#[test]
fn delayed_cable_polarity_and_fresh_identity() {
    let p = patch(1, 10.0);
    let mut old = p.compile().unwrap();
    old.process_poly_block(&mut [PortFrame::default(); 15]);
    let mut negative = patch(1, -10.0).compile().unwrap();
    negative.adopt_state_from(&old);
    // The adopted cable still carries the previous source frame at sample 16.
    negative.process_poly_block(&mut [PortFrame::default(); 1]);
    assert_eq!(lights(&negative).brightness[0], [1.0, -0.01]);
    negative.process_poly_block(&mut [PortFrame::default(); 16]);
    assert!((lights(&negative).brightness[0][0] - 0.98).abs() < 1e-7);
    assert_eq!(lights(&negative).brightness[0][1], 1.0);
    let mut fresh = p.clone();
    fresh.nodes[1].id = "new_viz".into();
    fresh.cables[0].to = "new_viz".into();
    let mut fresh = fresh.compile().unwrap();
    fresh.adopt_state_from(&negative);
    let state = fresh
        .module_lights(fresh.resolve_module_lights("new_viz").unwrap())
        .unwrap();
    assert_eq!(state.input_channels, Some(0));
    assert_eq!(state.brightness, [[0.0; 2]; 16]);
}
