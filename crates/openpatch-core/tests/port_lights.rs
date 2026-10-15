//! Public-engine signal provenance, lifecycle and noninterference regressions.
use openpatch_core::port_lights::step_port_light;
use openpatch_core::*;
use serde_json::{json, Value};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct Allocator;
thread_local! {
    static TRACK: Cell<bool> = const { Cell::new(false) };
    static HEAP: Cell<(usize, usize)> = const { Cell::new((0, 0)) };
}
fn record(drop: bool) {
    let _ = TRACK.try_with(|t| {
        if t.get() {
            let _ = HEAP.try_with(|h| {
                let (a, d) = h.get();
                h.set((a + usize::from(!drop), d + usize::from(drop)));
            });
        }
    });
}
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        record(false);
        unsafe { System.alloc(l) }
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        record(false);
        unsafe { System.alloc_zeroed(l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        record(false);
        unsafe { System.realloc(p, l, n) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        record(true);
        unsafe { System.dealloc(p, l) }
    }
}
#[global_allocator]
static ALLOCATOR: Allocator = Allocator;
fn patch(v: Value) -> Patch {
    serde_json::from_value(v).unwrap()
}
fn step(e: &mut Engine) -> PortFrame {
    let mut f = [PortFrame::default()];
    e.process_poly_block(&mut f);
    f[0]
}
fn light(e: &Engine, id: &str, direction: PortDirection, port: &str) -> Option<PortLightState> {
    e.port_light(e.resolve_port_light(id, direction, port).unwrap())
}
fn connected(channels: u8, rgb: [f32; 3]) -> PortLightState {
    PortLightState {
        connected: true,
        channels,
        raw_rgb: rgb,
        invalid: false,
    }
}
fn pair(version: u32) -> Patch {
    patch(json!({"version":version,"sample_rate":48000,
        "nodes":[{"id":"s","kind":"constant","params":{"value":5}}, {"id":"v","kind":"gain"}],
        "cables":[{"id":"route","from":"s","to":"v","input":"signal"}],"output":"v"}))
}

#[test]
fn detector_preserves_signed_targets_overrange_norm_and_source_decay() {
    for rate in [44100, 48000, 96000] {
        let zero = PortLightState::default();
        let positive = step_port_light(zero, true, 1, &[10.0], rate);
        let dt = 7.0 * (1.0 / rate as f32);
        assert_eq!(positive, connected(1, [-30.0 * dt, 1.0, 0.0]));
        let reversed = step_port_light(positive, true, 1, &[-10.0], rate);
        assert_eq!(reversed.raw_rgb, [1.0, 1.0 + (-1.0 - 1.0) * 30.0 * dt, 0.0]);
        let quiet = step_port_light(positive, true, 1, &[0.0], rate);
        assert_eq!(quiet.raw_rgb[1], 1.0 - 30.0 * dt);
        assert_eq!(
            step_port_light(zero, true, 2, &[3.0, 4.0], rate),
            connected(2, [0.0, 0.0, 0.5])
        );
        assert_eq!(
            step_port_light(zero, true, 3, &[3.0, 4.0, 0.0], rate).raw_rgb[2],
            0.5
        );
        assert!(step_port_light(zero, true, 2, &[10.0, -10.0], rate).raw_rgb[2] > 1.0);
        assert_eq!(
            step_port_light(zero, true, 16, &[0.0; 16], rate),
            connected(16, [0.0; 3])
        );
        assert_eq!(
            step_port_light(positive, true, 0, &[], rate),
            connected(0, [0.0; 3])
        );
        assert_eq!(
            step_port_light(positive, false, 16, &[f32::NAN; 16], rate),
            zero
        );
    }
}

#[test]
fn invalid_measurements_reset_only_observer_and_inactive_values_are_ignored() {
    let zero = PortLightState::default();
    for (count, values) in [
        (1, vec![f32::NAN]),
        (1, vec![f32::INFINITY]),
        (2, vec![f32::MAX; 2]),
        (17, vec![0.0; 17]),
        (2, vec![1.0]),
    ] {
        let invalid = step_port_light(connected(1, [1.0; 3]), true, count, &values, 48000);
        assert!(invalid.invalid);
        assert_eq!(invalid.raw_rgb, [0.0; 3]);
        assert_eq!(
            step_port_light(invalid, true, 1, &[5.0], 48000),
            step_port_light(zero, true, 1, &[5.0], 48000)
        );
    }
    assert!(!step_port_light(zero, true, 1, &[5.0, f32::NAN], 48000).invalid);
    assert_eq!(
        step_port_light(zero, true, 0, &[f32::NAN], 48000),
        connected(0, [0.0; 3])
    );
}

#[test]
fn physical_connectivity_owner_binding_and_opt_in_are_separate_from_consumers() {
    for version in [1, 2] {
        let mut e = pair(version).compile().unwrap();
        let h = e
            .resolve_port_light("s", PortDirection::Output, "out")
            .unwrap();
        let unused = e
            .resolve_port_light("v", PortDirection::Input, "cv")
            .unwrap();
        // Selected out is a DSP consumer but there is no physical output plug.
        let selected = e
            .resolve_port_light("v", PortDirection::Output, "out")
            .unwrap();
        assert!(e
            .resolve_port_light("missing", PortDirection::Output, "out")
            .is_err());
        assert!(e
            .resolve_port_light("s", PortDirection::Input, "out")
            .is_err());
        assert!(e
            .resolve_port_light("v", PortDirection::Input, "wrong")
            .is_err());
        assert!(e.port_light(h).is_none());
        step(&mut e);
        assert_eq!(e.port_light_processed_frames(), 1);
        assert!(e.port_light_observation_frame().is_none());
        e.set_port_lights_enabled(true);
        assert!(e.port_light(h).is_none());
        e.process_block(&mut [0.0; 6]); // frames 1..6, not an acquisition
        assert!(e.port_light(h).is_none());
        step(&mut e); // frame 7
        assert_eq!(e.port_light_observation_frame(), Some(7));
        assert_eq!(e.port_light(h).unwrap().raw_rgb[1], 0.5);
        assert_eq!(e.port_light(unused), Some(PortLightState::default()));
        assert_eq!(e.port_light(selected), Some(PortLightState::default()));
        let mut copy = [None; 3];
        assert!(e.copy_port_lights(&[h, unused, selected], &mut copy));
        assert_eq!(copy[0], e.port_light(h));
        let before = copy;
        assert!(!e.copy_port_lights(&[h], &mut copy));
        assert_eq!(copy, before);
        let mut foreign = pair(version).compile().unwrap();
        foreign.set_port_lights_enabled(true);
        step(&mut foreign);
        assert!(foreign.port_light(h).is_none());
        assert!(foreign.copy_port_lights(&[h], &mut copy[..1]));
        assert_eq!(copy[0], None);
        e.set_port_lights_enabled(false);
        assert!(e.port_light(h).is_none());
        e.process_block(&mut [0.0; 7]);
        e.set_port_lights_enabled(true);
        assert!(e.port_light(h).is_none());
        e.process_block(&mut [0.0; 6]); // next tick is frame21
        assert!(e.port_light(h).is_none());
        step(&mut e);
        assert_eq!(e.port_light_observation_frame(), Some(21));
    }
}

#[test]
fn input_observation_uses_actual_same_sample_or_delayed_summed_frame() {
    for version in [1, 2] {
        let mut e = pair(version).compile().unwrap();
        e.set_port_lights_enabled(true);
        step(&mut e);
        assert_eq!(
            light(&e, "s", PortDirection::Output, "out")
                .unwrap()
                .raw_rgb[1],
            0.5
        );
        let input = light(&e, "v", PortDirection::Input, "signal").unwrap();
        assert_eq!(input.channels, if version == 1 { 1 } else { 0 });
        assert_eq!(input.raw_rgb[1], if version == 1 { 0.5 } else { 0.0 });
        e.process_block(&mut [0.0; 7]);
        assert_eq!(
            light(&e, "v", PortDirection::Input, "signal")
                .unwrap()
                .raw_rgb[1],
            0.5
        );
    }
    let mut e = patch(json!({"version":2,"nodes":[
        {"id":"p","kind":"constant","params":{"value":5}}, {"id":"n","kind":"constant","params":{"value":-5}}, {"id":"v","kind":"gain"}],
        "cables":[{"from":"p","to":"v","input":"signal"},{"from":"n","to":"v","input":"signal"}]})).compile().unwrap();
    e.set_port_lights_enabled(true);
    e.process_block(&mut [0.0; 8]);
    assert_eq!(
        light(&e, "p", PortDirection::Output, "out")
            .unwrap()
            .raw_rgb[1],
        0.5
    );
    assert_eq!(
        light(&e, "n", PortDirection::Output, "out")
            .unwrap()
            .raw_rgb[0],
        0.5
    );
    assert_eq!(
        light(&e, "v", PortDirection::Input, "signal"),
        Some(connected(1, [0.0; 3]))
    );
}

#[test]
fn stacked_poly_inputs_measure_sum_and_width_without_broadcasting_the_detector() {
    let p = patch(json!({"version":2,"nodes":[
        {"id":"m","kind":"constant","params":{"value":2}},
        {"id":"p","kind":"poly_constant","params":{"channels":3,"v1":1,"v2":2,"v3":3}},
        {"id":"v","kind":"poly_viz"}],"cables":[{"from":"m","to":"v","input":"signal"},{"from":"p","to":"v","input":"signal"}]}));
    let mut e = p.compile().unwrap();
    e.set_port_lights_enabled(true);
    e.process_block(&mut [0.0; 8]);
    assert_eq!(
        light(&e, "v", PortDirection::Input, "signal"),
        Some(connected(3, [0.0, 0.0, 50.0_f32.sqrt() / 10.0]))
    );
    assert_eq!(
        light(&e, "m", PortDirection::Output, "out")
            .unwrap()
            .channels,
        1
    );
    let mut zero = p.clone();
    zero.nodes[1].params.insert("channels".into(), 0.0);
    zero.cables.remove(0);
    let mut z = zero.compile().unwrap();
    z.set_port_lights_enabled(true);
    z.process_block(&mut [0.0; 8]);
    assert_eq!(
        light(&z, "p", PortDirection::Output, "out"),
        Some(connected(0, [0.0; 3]))
    );
    assert_eq!(
        light(&z, "v", PortDirection::Input, "signal"),
        Some(connected(0, [0.0; 3]))
    );
}

#[test]
fn all_named_catalog_endpoints_resolve_including_input_only_and_twenty_output_modules() {
    for descriptor in catalog()["modules"].as_array().unwrap() {
        let kind = descriptor["kind"].as_str().unwrap();
        let p = patch(json!({"version":2,"nodes":[{"id":"n","kind":kind}]}));
        let mut e = p
            .compile()
            .unwrap_or_else(|error| panic!("{kind}: {error}"));
        e.set_port_lights_enabled(true);
        let mut handles = Vec::new();
        for (direction, key) in [
            (PortDirection::Input, "inputs"),
            (PortDirection::Output, "outputs"),
        ] {
            for name in descriptor[key].as_array().unwrap() {
                handles.push(
                    e.resolve_port_light("n", direction, name.as_str().unwrap())
                        .unwrap_or_else(|error| panic!("{kind}/{name}: {error}")),
                );
            }
        }
        step(&mut e);
        assert!(
            handles
                .into_iter()
                .all(|h| e.port_light(h) == Some(PortLightState::default())),
            "{kind}"
        );
    }
    let mut p = patch(
        json!({"version":2,"nodes":[{"id":"p","kind":"pulses"},{"id":"a","kind":"poly_merge"},{"id":"b","kind":"poly_merge"}]}),
    );
    for index in 0..20 {
        p.cables.push(serde_json::from_value(json!({"from":"p","output":format!("{}{}", if index<10 {"trig"} else {"gate"}, index%10+1),"to":if index<16 {"a"} else {"b"},"input":format!("in{}",index%16+1)})).unwrap());
    }
    let mut e = p.compile().unwrap();
    e.set_port_lights_enabled(true);
    let h = e.resolve_module_control("p").unwrap();
    assert!(e.handle_module_event(h, ModuleAction::PulsePress(10)));
    step(&mut e);
    assert_eq!(
        light(&e, "p", PortDirection::Output, "gate10")
            .unwrap()
            .raw_rgb[1],
        1.0
    );
    assert_eq!(
        light(&e, "p", PortDirection::Output, "trig10")
            .unwrap()
            .raw_rgb[1],
        1.0
    );
}

#[test]
fn seven_sample_clock_and_audio_are_partition_independent_across_every_render_api() {
    for version in [1, 2] {
        let mut p = pair(version);
        p.nodes[0].kind = "oscillator".into();
        p.nodes[0].params = [("frequency".into(), 73.0), ("amplitude".into(), 0.7)]
            .into_iter()
            .collect();
        let mut whole = p.compile().unwrap();
        let mut mixed = p.compile().unwrap();
        whole.set_port_lights_enabled(true);
        mixed.set_port_lights_enabled(true);
        let handles = [
            whole
                .resolve_port_light("s", PortDirection::Output, "out")
                .unwrap(),
            mixed
                .resolve_port_light("s", PortDirection::Output, "out")
                .unwrap(),
        ];
        let mut expected = [0.0; 513];
        whole.process_block(&mut expected);
        let mut actual = [0.0; 513];
        let mut offset = 0;
        for (api, length) in [0, 1, 3, 127, 2, 129, 7, 0, 116, 128]
            .into_iter()
            .enumerate()
        {
            match api % 4 {
                0 => mixed.process_block(&mut actual[offset..offset + length]),
                1 => {
                    let mut f = vec![PortFrame::default(); length];
                    mixed.process_poly_block(&mut f);
                    for (out, f) in actual[offset..offset + length].iter_mut().zip(f) {
                        *out = f.channel(0);
                    }
                }
                2 => {
                    let mut f = vec![[0.0; 2]; length];
                    mixed.process_audio_block(&mut f);
                    for (out, f) in actual[offset..offset + length].iter_mut().zip(f) {
                        *out = f[0];
                    }
                }
                _ => {
                    let mut f = vec![[0.0; 2]; length];
                    let taps = [Some(mixed.resolve_tap("s", "out", 0).unwrap()), None];
                    let mut samples = vec![[TapSample::default(); 2]; length];
                    mixed.process_audio_block_with_taps(&mut f, &taps, &mut samples);
                    for (out, f) in actual[offset..offset + length].iter_mut().zip(f) {
                        *out = f[0];
                    }
                }
            }
            offset += length;
        }
        assert_eq!(offset, 513);
        assert_eq!(actual, expected);
        assert_eq!(mixed.port_light_processed_frames(), 513);
        assert_eq!(mixed.port_light_observation_frame(), Some(511));
        assert_eq!(whole.port_light(handles[0]), mixed.port_light(handles[1]));
    }
}

#[test]
fn compatible_adoption_retains_phase_and_decay_but_exclusions_reset_only_visual_targets() {
    for version in [1, 2] {
        let p = pair(version);
        let mut old = p.compile().unwrap();
        old.set_port_lights_enabled(true);
        old.process_block(&mut [0.0; 10]);
        let h = old
            .resolve_port_light("s", PortDirection::Output, "out")
            .unwrap();
        let before = old.port_light(h);
        let mut new = p.compile().unwrap();
        new.set_port_lights_enabled(true);
        new.adopt_state_from(&old);
        assert_eq!(new.port_light_processed_frames(), 10);
        assert_eq!(new.port_light_observation_frame(), Some(7));
        assert_eq!(light(&new, "s", PortDirection::Output, "out"), before);
        assert!(new.port_light(h).is_none());
        let mut quiet = p.clone();
        quiet.nodes[0].params.insert("value".into(), 0.0);
        let mut quiet = quiet.compile().unwrap();
        quiet.set_port_lights_enabled(true);
        quiet.adopt_state_from(&old);
        quiet.process_block(&mut [0.0; 4]);
        assert_eq!(quiet.port_light_observation_frame(), Some(7));
        step(&mut quiet);
        assert_eq!(quiet.port_light_observation_frame(), Some(14));
        assert!(
            (light(&quiet, "s", PortDirection::Output, "out")
                .unwrap()
                .raw_rgb[1]
                - 0.5 * (1.0 - 30.0 * (7.0 / 48000.0)))
                .abs()
                < 1e-7
        );
        let mut fresh = p.compile().unwrap();
        fresh.set_port_lights_enabled(true);
        let excluded = fresh.resolve_module_state("s").unwrap();
        fresh.adopt_state_from_excluding(&old, &[excluded]);
        assert_eq!(fresh.port_light_processed_frames(), 10);
        assert!(light(&fresh, "s", PortDirection::Output, "out").is_none());
        assert!(light(&fresh, "v", PortDirection::Input, "signal").is_none());
        fresh.process_block(&mut [0.0; 5]);
        assert_eq!(fresh.port_light_observation_frame(), Some(14));
        let module = fresh.resolve_module_state("v").unwrap();
        assert!(fresh.initialize_module_state(module));
        assert_eq!(fresh.port_light_processed_frames(), 15);
        assert!(light(&fresh, "v", PortDirection::Input, "signal").is_none());
        fresh.process_block(&mut [0.0; 7]);
        assert_eq!(fresh.port_light_observation_frame(), Some(21));
    }
}

#[test]
fn route_incarnation_changes_reset_inputs_and_output_connections_but_color_does_not() {
    let p = pair(2);
    let mut old = p.compile().unwrap();
    old.set_port_lights_enabled(true);
    old.process_block(&mut [0.0; 15]);
    let mut presentation = p.clone();
    presentation.cables[0].color = Some([1, 2, 3]);
    presentation.cables[0].input_order = Some(4);
    presentation.cables[0].output_order = Some(5);
    let mut new = presentation.compile().unwrap();
    new.set_port_lights_enabled(true);
    new.adopt_state_from(&old);
    assert_eq!(
        light(&new, "s", PortDirection::Output, "out"),
        light(&old, "s", PortDirection::Output, "out")
    );
    let route = new.resolve_cable_state("route").unwrap();
    new.adopt_state_from_excluding_routes(&old, &[], &[route]);
    assert!(light(&new, "s", PortDirection::Output, "out").is_none());
    assert!(light(&new, "v", PortDirection::Input, "signal").is_none());
    assert_eq!(new.port_light_processed_frames(), 15);
    let mut replacement = p.clone();
    replacement.cables[0].id = Some("replacement".into());
    let mut new = replacement.compile().unwrap();
    new.set_port_lights_enabled(true);
    new.adopt_state_from(&old);
    assert!(light(&new, "s", PortDirection::Output, "out").is_none());
    assert!(light(&new, "v", PortDirection::Input, "signal").is_none());
    replacement.cables.clear();
    let mut removed = replacement.compile().unwrap();
    removed.set_port_lights_enabled(true);
    removed.adopt_state_from(&old);
    assert_eq!(
        light(&removed, "s", PortDirection::Output, "out"),
        Some(PortLightState::default())
    );
    replacement = p.clone();
    replacement
        .nodes
        .push(serde_json::from_value(json!({"id":"extra","kind":"gain"})).unwrap());
    replacement.cables.push(
        serde_json::from_value(json!({"id":"extra","from":"s","to":"extra","input":"signal"}))
            .unwrap(),
    );
    let mut added = replacement.compile().unwrap();
    added.set_port_lights_enabled(true);
    added.adopt_state_from(&old);
    assert!(light(&added, "s", PortDirection::Output, "out").is_none());
    assert_eq!(
        light(&added, "v", PortDirection::Input, "signal"),
        light(&old, "v", PortDirection::Input, "signal")
    );
}

#[test]
fn incompatible_rate_or_version_adoption_starts_clock_zero_even_after_processing() {
    let mut old = pair(2).compile().unwrap();
    old.set_port_lights_enabled(true);
    old.process_block(&mut [0.0; 17]);
    for (version, rate) in [(1, 48000), (2, 96000)] {
        let mut p = pair(version);
        p.sample_rate = rate;
        let mut new = p.compile().unwrap();
        new.set_port_lights_enabled(true);
        step(&mut new);
        new.adopt_state_from(&old);
        assert_eq!(new.port_light_processed_frames(), 0);
        assert_eq!(new.port_light_observation_frame(), None);
        assert!(light(&new, "s", PortDirection::Output, "out").is_none());
        step(&mut new);
        assert_eq!(new.port_light_observation_frame(), Some(0));
    }
}

#[test]
fn bypass_plug_signal_changes_while_stock_vca_gain_stays_frozen_and_silence_is_dark() {
    let mut p = patch(
        json!({"version":2,"nodes":[{"id":"s","kind":"poly_constant","params":{"channels":1,"v1":2}},{"id":"cv","kind":"poly_constant","params":{"channels":4,"v1":5,"v2":5,"v3":5,"v4":5}},{"id":"v","kind":"stock_vca"},{"id":"sink","kind":"poly_viz"}],"cables":[{"from":"s","to":"v","input":"signal"},{"from":"cv","to":"v","input":"cv"},{"from":"v","to":"sink","input":"signal"}]}),
    );
    let mut old = p.compile().unwrap();
    old.set_port_lights_enabled(true);
    old.process_block(&mut [0.0; 15]);
    let gain = old
        .module_lights(old.resolve_module_lights("v").unwrap())
        .unwrap();
    assert_eq!(gain.input_channels, Some(4));
    p.nodes[2].bypassed = true;
    p.nodes[0].params.insert("v1".into(), -10.0);
    let mut bypass = p.compile().unwrap();
    bypass.set_port_lights_enabled(true);
    bypass.adopt_state_from(&old);
    bypass.process_block(&mut [0.0; 7]);
    assert_eq!(
        bypass
            .module_lights(bypass.resolve_module_lights("v").unwrap())
            .unwrap(),
        gain
    );
    assert_eq!(
        light(&bypass, "v", PortDirection::Output, "out")
            .unwrap()
            .channels,
        1
    );
    assert_eq!(
        light(&bypass, "v", PortDirection::Output, "out")
            .unwrap()
            .raw_rgb[0],
        1.0
    );
    p.nodes[2].bypassed = false;
    p.nodes[0].params.insert("v1".into(), 0.0);
    let mut silent = p.compile().unwrap();
    silent.set_port_lights_enabled(true);
    silent.process_block(&mut [0.0; 15]);
    assert_eq!(
        light(&silent, "v", PortDirection::Output, "out"),
        Some(connected(4, [0.0; 3]))
    );
    assert_eq!(
        silent
            .module_lights(silent.resolve_module_lights("v").unwrap())
            .unwrap()
            .brightness[0],
        [0.5, 0.0]
    );
}

// Paired engines receive identical actions, taps, graph changes and captures;
// only one runs the passive observer. Later reconnects reveal hidden histories.
#[test]
fn observers_do_not_change_consumer_gated_dsp_rng_saved_state_taps_or_reconnects() {
    for kind in ["stock_vcf", "midi_cc", "random_values", "stock_vca"] {
        let ports: &[&str] = match kind {
            "stock_vcf" => &["lowpass", "highpass"],
            "midi_cc" => &["out1", "out2"],
            "random_values" => &["out1", "out7"],
            _ => &["out"],
        };
        let mut p = patch(
            json!({"version":2,"nodes":[{"id":"s","kind":"poly_constant","params":{"channels":16,"v1":2,"v2":-3}},{"id":"m","kind":kind},{"id":"sink","kind":"poly_viz"}]}),
        );
        if kind == "stock_vcf" || kind == "stock_vca" {
            p.cables.push(
                serde_json::from_value(json!({"id":"in","from":"s","to":"m","input":"signal"}))
                    .unwrap(),
            );
        }
        let mut off = p.compile().unwrap();
        let mut on = p.compile().unwrap();
        on.set_port_lights_enabled(true);
        for phase in 0..6 {
            if phase > 0 {
                p.cables.retain(|c| c.id.as_deref() != Some("consumer"));
                if phase != 3 {
                    p.cables.push(serde_json::from_value(json!({"id":"consumer","from":"m","output":ports[phase%ports.len()],"to":"sink","input":"signal"})).unwrap());
                }
                p.nodes[1].bypassed = phase == 4;
                let mut a = p.compile().unwrap();
                let mut b = p.compile().unwrap();
                b.set_port_lights_enabled(true);
                a.adopt_state_from(&off);
                b.adopt_state_from(&on);
                off = a;
                on = b;
            }
            for round in 0..9 {
                for e in [&mut off, &mut on] {
                    e.handle_midi(MidiEvent::ControlChange {
                        channel: 0,
                        controller: 1,
                        value: if round % 2 == 0 { 64 } else { 127 },
                    });
                    if kind == "random_values" && phase != 4 {
                        let h = e.resolve_module_control("m").unwrap();
                        assert!(e.handle_module_event(h, ModuleAction::PushPress));
                        assert!(e.handle_module_event(h, ModuleAction::PushRelease));
                    }
                }
                let mut a = [PortFrame::default(); 37];
                let mut b = a;
                off.process_poly_block(&mut a);
                on.process_poly_block(&mut b);
                assert_eq!(a, b, "{kind} phase{phase}");
                // Reads of every advertised output must themselves remain passive.
                for port in ports {
                    std::hint::black_box(light(&on, "m", PortDirection::Output, port));
                }
                if round % 3 == 0 {
                    let ta = [
                        Some(off.resolve_tap("m", ports[0], 0).unwrap()),
                        Some(off.resolve_tap("m", ports[ports.len() - 1], 15).unwrap()),
                    ];
                    let tb = [
                        Some(on.resolve_tap("m", ports[0], 0).unwrap()),
                        Some(on.resolve_tap("m", ports[ports.len() - 1], 15).unwrap()),
                    ];
                    let mut audio_a = [[0.0; 2]; 19];
                    let mut audio_b = audio_a;
                    let mut sa = [[TapSample::default(); 2]; 19];
                    let mut sb = sa;
                    off.process_audio_block_with_taps(&mut audio_a, &ta, &mut sa);
                    on.process_audio_block_with_taps(&mut audio_b, &tb, &mut sb);
                    assert_eq!(audio_a, audio_b);
                    assert_eq!(sa, sb, "{kind} taps phase{phase}");
                }
                let ha = off.resolve_module_state("m").unwrap();
                let hb = on.resolve_module_state("m").unwrap();
                assert_eq!(
                    off.module_saved_state(ha),
                    on.module_saved_state(hb),
                    "{kind} state phase{phase}"
                );
                if kind == "stock_vca" {
                    assert_eq!(off.module_lights(ha), on.module_lights(hb));
                }
            }
        }
    }
}

#[test]
fn maximum_graph_and_first_process_adoption_toggles_and_reads_allocate_and_free_nothing() {
    let mut p = Patch::new_v2();
    for index in 0..64 {
        p.nodes.push(
            serde_json::from_value(json!({"id":format!("s{index}"),"kind":"poly_split"})).unwrap(),
        );
        p.nodes.push(
            serde_json::from_value(json!({"id":format!("m{index}"),"kind":"poly_merge"})).unwrap(),
        );
        for port in 1..=16 {
            p.cables.push(serde_json::from_value(json!({"id":format!("c{index}_{port}"),"from":format!("s{index}"),"output":format!("out{port}"),"to":format!("m{index}"),"input":format!("in{port}")})).unwrap());
        }
    }
    let mut a = p.compile().unwrap();
    let mut b = p.compile().unwrap();
    let handles: Vec<_> = p
        .cables
        .iter()
        .flat_map(|c| {
            [
                b.resolve_port_light(&c.from, PortDirection::Output, &c.output)
                    .unwrap(),
                b.resolve_port_light(&c.to, PortDirection::Input, &c.input)
                    .unwrap(),
            ]
        })
        .collect();
    assert_eq!(handles.len(), 2 * MAX_CABLES);
    let mut states = vec![None; handles.len()];
    let mut frames = [PortFrame::default(); 129];
    HEAP.with(|c| c.set((0, 0)));
    TRACK.with(|c| c.set(true));
    a.set_port_lights_enabled(true);
    b.set_port_lights_enabled(true);
    a.process_poly_block(&mut frames);
    b.adopt_state_from(&a);
    b.process_poly_block(&mut frames);
    let copied = b.copy_port_lights(&handles, &mut states);
    b.set_port_lights_enabled(false);
    b.process_poly_block(&mut frames);
    b.set_port_lights_enabled(true);
    b.process_poly_block(&mut frames);
    b.copy_port_lights(&handles, &mut states);
    TRACK.with(|c| c.set(false));
    assert_eq!(HEAP.with(Cell::get), (0, 0));
    assert!(copied);
    assert!(states
        .iter()
        .all(|s| s.is_some_and(|s| s.connected && s.channels == 1)));
    for version in [1, 2] {
        let p = pair(version);
        let mut old = p.compile().unwrap();
        let mut new = p.compile().unwrap();
        let h = new
            .resolve_port_light("s", PortDirection::Output, "out")
            .unwrap();
        let mh = new.resolve_module_state("v").unwrap();
        let mut audio = [[0.0; 2]; 129];
        let taps = [Some(new.resolve_tap("s", "out", 0).unwrap()), None];
        let mut samples = [[TapSample::default(); 2]; 129];
        HEAP.with(|c| c.set((0, 0)));
        TRACK.with(|c| c.set(true));
        old.set_port_lights_enabled(true);
        new.set_port_lights_enabled(true);
        old.process_block(&mut [0.0; 129]);
        new.adopt_state_from(&old);
        new.process_audio_block_with_taps(&mut audio, &taps, &mut samples);
        new.initialize_module_state(mh);
        new.set_port_lights_enabled(false);
        new.set_port_lights_enabled(true);
        std::hint::black_box(new.port_light(h));
        TRACK.with(|c| c.set(false));
        assert_eq!(HEAP.with(Cell::get), (0, 0));
    }
}
