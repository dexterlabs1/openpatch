//! Public Engine integration and fixed-storage callback guards for Random.
//! Graph preparation/destruction and assertion formatting stay outside tracking.
use openpatch_core::{Engine, Patch, PortDirection, PortFrame, TapSample};
use serde_json::json;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
};

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
const PORTS: [&str; 5] = ["stepped", "linear", "smooth", "exponential", "trigger"];
fn patch(width: u8, stereo: bool, feedback: bool) -> Patch {
    let mut p:Patch=serde_json::from_value(json!({"version":2,"sample_rate":48000,
        "nodes":[{"id":"source","kind":"poly_constant","params":{"channels":width}},
                 {"id":"random","kind":"random","params":{"rate":10,"shape":0.65}},
                 {"id":"sink","kind":"audio_stereo"}],
        "cables":[{"id":"external","from":"source","to":"random","input":"external"},
                  {"id":"left","from":"random","output":"smooth","to":"sink","input":"left"},
                  {"id":"right","from":"random","output":"exponential","to":"sink","input":"right"}],
        "output":if stereo {"sink"} else {"random"},"output_port":if stereo {"out"} else {"stepped"}})).unwrap();
    for lane in 1..=width {
        p.nodes[0]
            .params
            .insert(format!("v{lane}"), f32::from(lane) / 8.0);
    }
    if feedback {
        p.nodes[1].params.insert("shape".into(), 0.0);
        p.cables.push(serde_json::from_value(json!({"id":"feedback","from":"random","output":"stepped","to":"random","input":"external"})).unwrap());
    }
    p
}
fn lights(e: &Engine) -> openpatch_core::ModuleLightState {
    e.module_lights(e.resolve_module_lights("random").unwrap())
        .unwrap()
}

#[test]
fn sixteen_lane_feedback_obeys_delayed_capture_and_node_order_and_partition_invariance() {
    let p = patch(16, false, true);
    let mut whole = p.compile().unwrap();
    let mut partitioned = p.compile().unwrap();
    let mut reverse = p.clone();
    reverse.nodes.reverse();
    let mut reordered = reverse.compile().unwrap();
    let mut expected = [PortFrame::default(); 2048];
    let mut actual = [PortFrame::default(); 2048];
    let mut other = [PortFrame::default(); 2048];
    whole.process_poly_block(&mut expected);
    let sizes = [1, 3, 0, 17, 64, 127, 128, 129];
    let mut position = 0;
    let mut index = 0;
    while position < actual.len() {
        let count = sizes[index % sizes.len()].min(actual.len() - position);
        partitioned.process_poly_block(&mut actual[position..position + count]);
        position += count;
        index += 1;
    }
    reordered.process_poly_block(&mut other);
    assert_eq!(actual, expected);
    assert_eq!(other, expected);
    // The external cable has no source frame at frame zero. Its width and
    // samples arrive together on the next frame, independent of node ordering.
    assert_eq!(expected[0].channels, 1);
    assert_eq!(expected[1].channels, 16);
    assert_eq!(expected[1].values, [0.0; 16]);
    let mut captures = 0;
    let mut previous = [0.0; 16];
    for frame in expected.iter().skip(1) {
        assert_eq!(frame.channels, 16);
        if frame.values != previous {
            captures += 1;
            for (lane, value) in frame.values.iter().enumerate() {
                // Every capture adds exactly one incoming constant to the
                // previous held output. No same-sample recursive growth occurs.
                assert_eq!(*value, previous[lane] + (lane + 1) as f32 / 8.0);
            }
            previous = frame.values;
        }
    }
    assert!(captures > 30);
}

#[test]
fn all_outputs_all_lanes_taps_stereo_and_actual_lights_are_coherent() {
    let p = patch(16, true, false);
    let mut engine = p.compile().unwrap();
    engine.set_port_lights_enabled(true);
    let taps: [Option<openpatch_core::ResolvedTap>; 65] = std::array::from_fn(|index| {
        let (port, lane) = if index < 64 {
            (PORTS[index / 16], index % 16)
        } else {
            ("trigger", 0)
        };
        Some(engine.resolve_tap("random", port, lane).unwrap())
    });
    let external = engine
        .resolve_port_light("random", PortDirection::Input, "external")
        .unwrap();
    let smooth = engine
        .resolve_port_light("random", PortDirection::Output, "smooth")
        .unwrap();
    let mut audio = [[0.0; 2]; 257];
    let mut observed = [[TapSample::default(); 65]; 257];
    engine.process_audio_block_with_taps(&mut audio, &taps, &mut observed);
    for index in 2..audio.len() {
        let left: f32 = observed[index - 1][32..48]
            .iter()
            .map(|sample| sample.value)
            .sum();
        let right: f32 = observed[index - 1][48..64]
            .iter()
            .map(|sample| sample.value)
            .sum();
        assert_eq!(audio[index], [left / 10.0, right / 10.0]);
    }
    assert!(observed[2..]
        .iter()
        .flatten()
        .all(|sample| sample.active && sample.value.is_finite()));
    assert!(observed.iter().any(|row| row[64].value == 10.0));
    for row in &observed {
        assert!(row[64].value == 0.0 || row[64].value == 10.0);
    }
    let state = lights(&engine);
    assert_eq!(state.count, 5);
    assert_eq!(state.brightness[1][0], 0.65);
    assert_eq!(
        state.brightness[3][0], 0.0,
        "external sample mode ignores spread"
    );
    assert_eq!(state.brightness[4][0], 0.0);
    assert!(state.brightness[0][0] > 0.0 && state.brightness[2][0] > 0.0);
    for handle in [external, smooth] {
        let light = engine.port_light(handle).unwrap();
        assert!(light.connected && !light.invalid);
        assert_eq!(light.channels, 16);
        assert!(light.raw_rgb.iter().any(|value| *value > 0.0));
    }
}

#[test]
fn adoption_bypass_resume_initialize_and_fresh_reset_have_distinct_temporal_contracts() {
    let p = patch(16, false, false);
    let mut old = p.compile().unwrap();
    old.process_poly_block(&mut [PortFrame::default(); 83]);
    let mut adopted = p.compile().unwrap();
    adopted.adopt_state_from(&old);
    let mut initialized = p.compile().unwrap();
    initialized.adopt_state_from(&old);
    let initialized_handle = initialized.resolve_module_state("random").unwrap();
    assert!(initialized.initialize_module_state(initialized_handle));
    assert_eq!(lights(&initialized), lights(&old));
    let mut muted_patch = p.clone();
    muted_patch.nodes[1].bypassed = true;
    let mut muted = muted_patch.compile().unwrap();
    muted.adopt_state_from(&old);
    let before = lights(&muted);
    let mut silence = [PortFrame::default(); 513];
    muted.process_poly_block(&mut silence);
    assert!(silence.iter().all(|frame| *frame == PortFrame::mono(0.0)));
    assert_eq!(lights(&muted), before);
    let mut resumed = p.compile().unwrap();
    resumed.adopt_state_from(&muted);
    let mut expected = [PortFrame::default(); 129];
    let mut actual = [PortFrame::default(); 129];
    old.process_poly_block(&mut expected);
    for engine in [&mut adopted, &mut initialized, &mut resumed] {
        engine.process_poly_block(&mut actual);
        assert_eq!(actual, expected);
    }
    let mut reset = p.compile().unwrap();
    reset.adopt_state_from(&old);
    assert!(reset.reset_module_state(reset.resolve_module_state("random").unwrap()));
    assert_eq!(lights(&reset).brightness, [[0.0; 2]; 16]);
    reset.process_poly_block(&mut actual);
    // Fresh runtime loses the held target and restarts the internal clock. It
    // must not produce the continued sample from the adopted envelope.
    assert_ne!(actual, expected);
    assert_eq!(actual[0].values, [0.0; 16]);
    // Excluding the reincarnated module also clears all its incident cables,
    // which must give the same full trace as a freshly compiled graph.
    let mut reincarnated = p.compile().unwrap();
    let handle = reincarnated.resolve_module_state("random").unwrap();
    reincarnated.adopt_state_from_excluding(&old, &[handle]);
    let mut fresh = p.compile().unwrap();
    fresh.process_poly_block(&mut expected);
    reincarnated.process_poly_block(&mut actual);
    assert_eq!(actual, expected);
}

#[test]
fn changing_seed_keeps_existing_curves_and_trigger_times_until_a_new_random_capture() {
    let mut p = patch(1, false, false);
    p.cables
        .retain(|cable| cable.id.as_deref() != Some("external"));
    p.nodes[1].params.insert("shape".into(), 0.65);
    p.nodes[1].params.insert("rate".into(), 8.0);
    let mut old = p.compile().unwrap();
    old.process_poly_block(&mut [PortFrame::default(); 213]);
    p.nodes[1].params.insert("seed".into(), 27.0);
    let mut changed = p.compile().unwrap();
    changed.adopt_state_from(&old);
    assert_eq!(lights(&changed), lights(&old));
    let old_taps = [
        Some(old.resolve_tap("random", "linear", 0).unwrap()),
        Some(old.resolve_tap("random", "trigger", 0).unwrap()),
    ];
    let new_taps = [
        Some(changed.resolve_tap("random", "linear", 0).unwrap()),
        Some(changed.resolve_tap("random", "trigger", 0).unwrap()),
    ];
    let mut audio = [[0.0; 2]; 512];
    let mut baseline = [[TapSample::default(); 2]; 512];
    let mut actual = baseline;
    old.process_audio_block_with_taps(&mut audio, &old_taps, &mut baseline);
    changed.process_audio_block_with_taps(&mut audio, &new_taps, &mut actual);
    let mut saw_low = false;
    let mut first_new_trigger = None;
    for (index, (before, after)) in baseline.iter().zip(&actual).enumerate() {
        assert_eq!(before[1], after[1], "seed changed clock/pulse timing");
        if before[1].value == 0.0 {
            saw_low = true;
        } else if saw_low && first_new_trigger.is_none() {
            first_new_trigger = Some(index);
        }
    }
    let boundary = first_new_trigger.expect("next capture was not reached");
    assert!(boundary > 0);
    assert_eq!(baseline[..boundary], actual[..boundary]);
    assert!(baseline[boundary..]
        .iter()
        .zip(&actual[boundary..])
        .any(|(a, b)| a[0].value != b[0].value));
}

#[test]
fn first_callbacks_taps_prepared_handoffs_resets_bypass_and_rewind_have_no_heap_activity() {
    for width in [1, 16] {
        let p = patch(width, false, false);
        let stereo = patch(width, true, false);
        let mut first = p.compile().unwrap();
        let mut receiver = stereo.compile().unwrap();
        let mut bypass_patch = stereo.clone();
        bypass_patch.nodes[1].bypassed = true;
        let mut bypass = bypass_patch.compile().unwrap();
        let mut resumed = stereo.compile().unwrap();
        let mut fresh = stereo.compile().unwrap();
        let taps = std::array::from_fn::<_, 5, _>(|index| {
            Some(
                receiver
                    .resolve_tap(
                        "random",
                        PORTS[index],
                        if index == 4 {
                            0
                        } else {
                            usize::from(width - 1)
                        },
                    )
                    .unwrap(),
            )
        });
        let resumed_taps = std::array::from_fn::<_, 5, _>(|index| {
            Some(
                resumed
                    .resolve_tap(
                        "random",
                        PORTS[index],
                        if index == 4 {
                            0
                        } else {
                            usize::from(width - 1)
                        },
                    )
                    .unwrap(),
            )
        });
        let handle = resumed.resolve_module_state("random").unwrap();
        let external = resumed
            .resolve_port_light("random", PortDirection::Input, "external")
            .unwrap();
        resumed.set_port_lights_enabled(true);
        let mut frames = [PortFrame::default(); 129];
        let mut muted = [PortFrame::default(); 129];
        let mut audio = [[0.0; 2]; 129];
        let mut samples = [[TapSample::default(); 5]; 129];
        let mut accepted = true;
        MEMORY_OPERATIONS.with(|count| count.set(0));
        TRACKING.with(|flag| flag.set(true));
        for count in [0, 1, 3, 64, 127, 128, 129] {
            first.process_poly_block(&mut frames[..count]);
        }
        let _ = receiver.handoff_state_from(&mut first);
        receiver.process_audio_block_with_taps(&mut audio, &taps, &mut samples);
        bypass.adopt_state_from(&receiver);
        bypass.process_poly_block(&mut muted);
        let _ = resumed.handoff_state_from(&mut bypass);
        resumed.process_audio_block_with_taps(&mut audio, &resumed_taps, &mut samples);
        let observed_external = resumed.port_light(external);
        accepted &= resumed.initialize_module_state(handle);
        resumed.process_poly_block(&mut frames);
        accepted &= resumed.reset_module_state(handle);
        resumed.process_audio_block_with_taps(&mut audio, &resumed_taps, &mut samples);
        // Engine has no whole-engine rewind API. The host can publish an
        // already prepared fresh graph; retiring its predecessor stays outside
        // the callback. This swap tests that concrete rewind mechanism.
        std::mem::swap(&mut resumed, &mut fresh);
        resumed.process_poly_block(&mut frames);
        TRACKING.with(|flag| flag.set(false));
        assert!(accepted);
        assert_eq!(
            MEMORY_OPERATIONS.with(Cell::get),
            0,
            "callback allocated/reallocated/freed at width {width}"
        );
        assert!(frames
            .iter()
            .skip(1)
            .all(|frame| frame.channels == 2 && frame.values.iter().all(|v| v.is_finite())));
        // Output selects audio_stereo, whose bypassed Random inputs receive
        // zero after cable propagation. The first device frame can contain the
        // previously delayed source sample, so only the settled suffix is zero.
        assert!(muted[1..].iter().all(|frame| frame.values == [0.0; 16]));
        assert!(samples
            .iter()
            .skip(1)
            .flatten()
            .all(|sample| sample.active && sample.value.is_finite()));
        assert_eq!(observed_external.unwrap().channels, width);
    }
}
