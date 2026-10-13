//! Headless callback profiling; no audio device is opened.
//! cargo test --release -p openpatch-desktop callback_timing_profile -- --ignored --nocapture
use super::tests::{sequence_callback, sequence_graph};
use openpatch::control::LiveModuleEvents;
use openpatch_core::Patch;
use serde_json::{json, Value};
use std::hint::black_box;
use std::time::Instant;

fn patch(family: &str, voices: usize) -> Patch {
    let mut value = match family {
        "circuit" => {
            serde_json::from_str(include_str!("../../../examples/polyphonic-voice.json")).unwrap()
        }
        "functional" => json!({"version":2,"sample_rate":48000,
            "nodes":[{"id":"chord","kind":"poly_constant"},
                {"id":"source","kind":"multi_vco","params":{"frequency":130.8128}},
                {"id":"sum","kind":"poly_sum"}],
            "cables":[{"from":"chord","to":"source","input":"pitch"},
                {"from":"source","output":"saw","to":"sum","input":"signal"}],
            "output":"sum"}),
        "gates" => json!({"version":2,"sample_rate":48000,
            "nodes":[{"id":"chord","kind":"poly_constant"},
                {"id":"source","kind":"multi_vco","params":{"frequency":1000}},
                {"id":"timing","kind":"gates","params":{"length":0.1}}],
            "cables":[{"from":"chord","to":"source","input":"pitch"},
                {"from":"source","output":"square","to":"timing","input":"in"}],
            "output":"timing","output_port":"delay"}),
        _ => unreachable!(),
    };
    let nodes = value["nodes"].as_array_mut().unwrap();
    let chord = nodes.iter_mut().find(|node| node["id"] == "chord").unwrap();
    chord["params"] = json!({"channels":voices});
    let mut patch: Patch = serde_json::from_value(value).unwrap();
    patch.assign_missing_cable_ids().unwrap();
    patch
}

fn distribution(mut values: Vec<u64>) -> Value {
    values.sort_unstable();
    let quantile = |q: f64| values[((values.len() - 1) as f64 * q).ceil() as usize];
    json!({"count":values.len(),"median_ns":quantile(0.5),"p95_ns":quantile(0.95),
        "p99_ns":quantile(0.99),"max_ns":values.last().unwrap(),"durations_ns":values})
}

#[test]
#[ignore = "manual release timing observation; no CI timing threshold or device playback"]
fn callback_timing_profile() {
    if cfg!(debug_assertions) {
        panic!("run with --release");
    }
    let mut workloads = Vec::new();
    for family in ["functional", "circuit", "gates"] {
        for voices in [1, 16] {
            let initial = patch(family, voices);
            let (mut measured, mut updates, mut retired) = sequence_callback(&initial);
            let (mut reference, mut raw_updates, mut raw_retired) = sequence_callback(&initial);
            let mut actual = [0.0_f32; 512];
            let mut expected = actual;
            // More than 1,023 transitions per Gates lane before collecting timings.
            for _ in 0..188 {
                measured.render(&mut actual, 2);
                reference.render_inner(&mut expected, 2);
                assert_eq!(actual, expected);
            }
            let mut pairs = Vec::new();
            let mut steady = Vec::new();
            for repetition in 0..5 {
                let mut run = |instrumented: bool| {
                    let start = Instant::now();
                    for _ in 0..128 {
                        if instrumented {
                            measured.render(black_box(&mut actual), 2);
                            black_box(&actual);
                        } else {
                            reference.render_inner(black_box(&mut expected), 2);
                            black_box(&expected);
                        }
                    }
                    start.elapsed().as_nanos() as u64
                };
                let (enabled, disabled) = if repetition % 2 == 0 {
                    let enabled = run(true);
                    (enabled, run(false))
                } else {
                    let disabled = run(false);
                    (run(true), disabled)
                };
                assert_eq!(actual, expected);
                pairs.push(
                    json!({"instrumented_ns":enabled,"uninstrumented_ns":disabled,"callbacks":128}),
                );
            }
            measured.performance.diagnostics.request_reset();
            for _ in 0..256 {
                measured.render(&mut actual, 2);
                reference.render_inner(&mut expected, 2);
                assert_eq!(actual, expected);
                steady.push(measured.performance.diagnostics.snapshot().duration_ns);
            }
            let steady_snapshot = measured.performance.diagnostics.snapshot();
            let mut edits = Vec::new();
            let mut revision = 1;
            // Every entry renders the first adoption buffer then the remaining crossfade.
            // A movement checkpoint uses identical graph data, as the native host does.
            for operation in ["layout_checkpoint", "cable_edit", "replacement"] {
                let mut times = Vec::new();
                for iteration in 0..16 {
                    revision += 1;
                    let mut current = initial.clone();
                    if operation == "cable_edit" && iteration % 2 == 0 {
                        current.cables.remove(0);
                    }
                    if operation == "replacement" {
                        let modules: Vec<_> = current.nodes.iter().map(|n| n.id.clone()).collect();
                        let cables: Vec<_> =
                            current.cables.iter().filter_map(|c| c.id.clone()).collect();
                        for performance in [&measured.performance, &reference.performance] {
                            performance.retire_modules(&modules);
                            performance.retire_cables(&cables);
                        }
                    }
                    measured.performance.sync_patch(&current, revision);
                    reference.performance.sync_patch(&current, revision);
                    let next = sequence_graph(&measured, &current, revision);
                    let raw_next = sequence_graph(&reference, &current, revision);
                    updates.push(next).ok().unwrap();
                    raw_updates.push(raw_next).ok().unwrap();
                    measured.render(&mut actual, 2);
                    reference.render_inner(&mut expected, 2);
                    assert_eq!(actual, expected);
                    let timing = measured.performance.diagnostics.snapshot();
                    assert!(timing.adopted_graph && timing.crossfade);
                    times.push(timing.duration_ns);
                    while retired.pop().is_ok() {}
                    while raw_retired.pop().is_ok() {}
                }
                edits.push(json!({"operation":operation,"first_buffer":distribution(times)}));
            }
            workloads.push(json!({"family":family,"voices":voices,"patch":initial,
                "instrumentation_pairs":pairs,"steady":distribution(steady),
                "steady_snapshot":steady_snapshot,"edits":edits,"audio_exact":true}));
        }
    }
    let report = json!({"format_version":1,"sample_rate":48000,"frames":256,"channels":2,
        "method":"headless callback with f32 conversion; no device, UI, scheduling or hardware xruns; compilation excluded; alternating enabled/disabled order; exact audio checked", "workloads":workloads});
    if let Some(path) = std::env::var_os("OPENPATCH_CALLBACK_PROFILE") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    } else {
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
    }
}
