//! Public graph behavior: inactive Merge storage cannot reach Split over cables.
use openpatch_core::{Engine, Patch, ResolvedTap, TapSample};
use serde_json::json;

fn patch(selection: f32) -> Patch {
    let mut nodes: Vec<_> = (1..=16)
        .map(|i| {
            json!({"id":format!("source{i}"),"kind":"poly_constant",
            "params":{"channels":1,"v1":i}})
        })
        .collect();
    nodes.push(json!({"id":"merge","kind":"poly_merge","params":{"channels":selection}}));
    nodes.push(json!({"id":"split","kind":"poly_split"}));
    let mut cables: Vec<_> = (1..=16)
        .map(|i| json!({"from":format!("source{i}"),"to":"merge","input":format!("in{i}")}))
        .collect();
    cables.push(json!({"from":"merge","to":"split","input":"signal"}));
    serde_json::from_value(json!({"version":2,"sample_rate":48000,"nodes":nodes,
        "cables":cables,"output":"split","output_port":"out16"}))
    .unwrap()
}

fn taps(engine: &Engine) -> [Option<ResolvedTap>; 16] {
    std::array::from_fn(|i| {
        Some(
            engine
                .resolve_tap("split", &format!("out{}", i + 1), 0)
                .unwrap(),
        )
    })
}

fn sample(engine: &mut Engine, taps: &[Option<ResolvedTap>; 16]) -> [TapSample; 16] {
    let mut audio = [[0.0; 2]; 1];
    let mut samples = [[TapSample::default(); 16]; 1];
    engine.process_audio_block_with_taps(&mut audio, taps, &mut samples);
    samples[0]
}

fn count(engine: &Engine) -> u8 {
    engine
        .module_lights(engine.resolve_module_lights("split").unwrap())
        .unwrap()
        .input_channels
        .unwrap()
}

#[test]
fn merge_split_retirement_zero_width_and_regrowth_follow_actual_delayed_counts() {
    let mut engine = patch(16.0).compile().unwrap();
    let handles = taps(&engine);
    for _ in 0..4 {
        sample(&mut engine, &handles);
    }
    assert_eq!(count(&engine), 16);
    let mut previous_width = 16;
    for (selection, expected_width) in [
        (1.0, 1),
        (-1.0, 0),
        (16.0, 16),
        (0.0, 16),
        (-1.0, 0),
        (1.0, 1),
    ] {
        let mut next = patch(selection).compile().unwrap();
        next.adopt_state_from(&engine);
        assert_eq!(count(&next), previous_width);
        let handles = taps(&next);
        sample(&mut next, &handles);
        assert_eq!(
            count(&next),
            previous_width,
            "Split reads the previously published Merge frame"
        );
        let outputs = sample(&mut next, &handles);
        assert_eq!(count(&next), expected_width);
        for (lane, output) in outputs.iter().enumerate() {
            assert!(
                output.active,
                "even missing Split lanes are independent mono outputs"
            );
            assert_eq!(
                output.value.to_bits(),
                if lane < expected_width as usize {
                    (lane as f32 + 1.0).to_bits()
                } else {
                    0.0_f32.to_bits()
                }
            );
        }
        engine = next;
        previous_width = expected_width;
    }
}
