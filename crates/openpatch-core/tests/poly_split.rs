use openpatch_core::*;
use serde_json::json;

fn patch(channels: u8) -> Patch {
    let mut patch: Patch = serde_json::from_value(json!({
        "version":2,"sample_rate":48000,
        "nodes":[{"id":"source","kind":"poly_constant","params":{"channels":channels}},
                 {"id":"split","kind":"poly_split"}],
        "cables":[{"from":"source","to":"split","input":"signal"}],
        "output":"split","output_port":"out1"
    }))
    .unwrap();
    for channel in 0..16 {
        patch.nodes[0].params.insert(
            format!("v{}", channel + 1),
            match channel {
                0 => -0.0,
                1 => 0.0,
                _ => channel as f32 - 8.0,
            },
        );
    }
    patch
}

fn lights(engine: &Engine) -> ModuleLightState {
    engine
        .module_lights(engine.resolve_module_lights("split").unwrap())
        .unwrap()
}

fn step(engine: &mut Engine) -> PortFrame {
    let mut frame = [PortFrame::default()];
    engine.process_poly_block(&mut frame);
    frame[0]
}

#[test]
fn every_input_width_and_named_output_preserves_active_bits_and_observes_the_delayed_count() {
    for channels in 0..=16 {
        for channel in 0..16 {
            let mut p = patch(channels);
            p.output_port = format!("out{}", channel + 1);
            let expected = if channel < usize::from(channels) {
                p.nodes[0].params[&format!("v{}", channel + 1)]
            } else {
                0.0
            };
            let mut engine = p.compile().unwrap();
            assert_eq!(lights(&engine).input_channels, Some(0));
            assert_eq!(step(&mut engine), PortFrame::mono(0.0));
            assert_eq!(lights(&engine).input_channels, Some(0));
            let frame = step(&mut engine);
            assert_eq!(frame.channels, 1);
            assert_eq!(frame.values[0].to_bits(), expected.to_bits());
            assert!(frame.values[1..].iter().all(|v| v.to_bits() == 0));
            let observed = lights(&engine);
            assert_eq!(observed.input_channels, Some(channels));
            assert_eq!(observed.count, 0);
            assert_eq!(observed.brightness, [[0.0; 2]; 16]);
            assert_eq!(observed.output_channels, None);
            assert_eq!(observed.automatic_channels, None);
        }
    }
}

#[test]
fn unplugged_and_unconsumed_split_reports_actual_zero_or_input_count_without_lamps() {
    let mut p = patch(16);
    p.output = None;
    p.output_port = "out".into();
    let mut engine = p.compile().unwrap();
    step(&mut engine);
    step(&mut engine);
    assert_eq!(lights(&engine).input_channels, Some(16));
    let value = serde_json::to_value(lights(&engine)).unwrap();
    assert_eq!(value["input_channels"], 16);
    assert_eq!(value["count"], 0);
    assert!(value.get("output_channels").is_none());
    assert!(value.get("automatic_channels").is_none());
    p.cables.clear();
    let mut unplugged = p.compile().unwrap();
    unplugged.adopt_state_from(&engine);
    assert_eq!(lights(&unplugged).input_channels, Some(16));
    step(&mut unplugged);
    assert_eq!(lights(&unplugged).input_channels, Some(0));
    assert!(unplugged.resolve_module_control("split").is_err());
    assert!(unplugged.resolve_module_event("split").is_err());
}

#[test]
fn compatible_adoption_initialize_bypass_and_fresh_or_rate_replacement_have_explicit_observation_lifetimes(
) {
    let mut old = patch(16).compile().unwrap();
    step(&mut old);
    step(&mut old);
    let observed = lights(&old);
    let mut edited_patch = patch(1);
    edited_patch.nodes.reverse();
    let mut edited = edited_patch.compile().unwrap();
    edited.adopt_state_from(&old);
    assert_eq!(lights(&edited), observed);
    let handle = edited.resolve_module_state("split").unwrap();
    assert!(edited.initialize_module_state(handle));
    assert_eq!(
        lights(&edited),
        observed,
        "Initialize has no Split reset callback"
    );
    step(&mut edited);
    assert_eq!(
        lights(&edited).input_channels,
        Some(16),
        "adopted cable frame is still the first input"
    );
    step(&mut edited);
    assert_eq!(lights(&edited).input_channels, Some(1));

    let mut bypass_patch = patch(0);
    bypass_patch.nodes[1].bypassed = true;
    let mut bypassed = bypass_patch.compile().unwrap();
    bypassed.adopt_state_from(&old);
    let bypass_handle = bypassed.resolve_module_state("split").unwrap();
    assert!(bypassed.initialize_module_state(bypass_handle));
    for _ in 0..20 {
        assert_eq!(step(&mut bypassed), PortFrame::mono(0.0));
        assert_eq!(lights(&bypassed), observed);
    }

    let mut fresh = patch(16).compile().unwrap();
    let fresh_handle = fresh.resolve_module_state("split").unwrap();
    fresh.adopt_state_from_excluding(&old, &[fresh_handle]);
    assert_eq!(lights(&fresh).input_channels, Some(0));
    let mut rate_patch = patch(16);
    rate_patch.sample_rate = 96000;
    let mut changed_rate = rate_patch.compile().unwrap();
    changed_rate.adopt_state_from(&old);
    assert_eq!(lights(&changed_rate).input_channels, Some(0));

    let mut changed_kind_patch = patch(16);
    changed_kind_patch.nodes[1].kind = "poly_viz".into();
    changed_kind_patch.output = None;
    changed_kind_patch.output_port = "out".into();
    let mut changed_kind = changed_kind_patch.compile().unwrap();
    changed_kind.adopt_state_from(&old);
    let viz = changed_kind
        .module_lights(changed_kind.resolve_module_lights("split").unwrap())
        .unwrap();
    assert_eq!(viz.input_channels, Some(0));
}

#[test]
fn shrinking_and_regrowing_count_and_values_remain_independent_of_callback_partition() {
    let mut full = patch(16).compile().unwrap();
    let mut pieces = patch(16).compile().unwrap();
    for channels in [16, 0, 1, 7, 16, 0] {
        let p = patch(channels);
        let mut next_full = p.compile().unwrap();
        let mut next_pieces = p.compile().unwrap();
        next_full.adopt_state_from(&full);
        next_pieces.adopt_state_from(&pieces);
        let mut expected = [PortFrame::default(); 257];
        let mut actual = [PortFrame::default(); 257];
        next_full.process_poly_block(&mut expected);
        for chunk in actual.chunks_mut(17) {
            next_pieces.process_poly_block(chunk);
        }
        assert_eq!(expected, actual);
        assert_eq!(lights(&next_full), lights(&next_pieces));
        assert_eq!(lights(&next_full).input_channels, Some(channels));
        let value = if channels == 0 { 0.0 } else { -0.0_f32 };
        assert_eq!(
            expected.last().unwrap().values[0].to_bits(),
            value.to_bits()
        );
        full = next_full;
        pieces = next_pieces;
    }
}

#[test]
fn count_catalog_has_no_saved_controls_and_actions_preserve_the_node() {
    let catalog = catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "poly_split")
        .unwrap();
    assert_eq!(module["params"], json!({}));
    assert_eq!(module["inputs"], json!(["signal"]));
    assert_eq!(
        module["outputs"],
        json!((1..=16).map(|n| format!("out{n}")).collect::<Vec<_>>())
    );
    assert_eq!(module["light_names"], json!([]));
    assert_eq!(module["min_patch_version"], 2);
    assert!(module["light_policy"]["input_channels"]
        .as_str()
        .unwrap()
        .contains("every processed frame"));
    assert!(module["functional_reference"]
        .as_str()
        .unwrap()
        .ends_with("/Split.cpp"));
    let actions = ModuleParameterActions::new(48000);
    let mut node = patch(1).nodes.remove(1);
    let before = serde_json::to_value(&node).unwrap();
    actions.initialize(&mut node).unwrap();
    actions.randomize(&mut node, 42).unwrap();
    assert_eq!(serde_json::to_value(&node).unwrap(), before);
    let mut unsupported = patch(1);
    unsupported.nodes[1].params.insert("channels".into(), 3.0);
    assert!(unsupported.compile().is_err());
}
