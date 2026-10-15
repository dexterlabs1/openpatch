//! Offline wall-clock profile for the native OpenPatch MACRO VOICE graph.
//!
//! Run with `cargo run --release -p openpatch-core --example macro_voice_profile`.
//! This measures public Engine::process_poly_block calls in-process. It does not
//! qualify an audio-device callback deadline and does not compare against Rack.
use openpatch_core::{Patch, PortFrame};
use serde_json::{json, Value};
use std::{hint::black_box, time::Instant};

const RATE: u32 = 48_000;
const WARMUP_BLOCKS: usize = 8;
const TIMED_BLOCKS: usize = 64;
const ENGINES: [&str; 8] = [
    "Analog",
    "Wavefold",
    "FM",
    "Harmonic",
    "Wavetable",
    "Chord",
    "Swarm",
    "Metallic",
];

fn patch(engine: usize, voices: usize, profile: &str) -> Patch {
    let mut nodes = Vec::new();
    let mut cables = Vec::new();
    let mut pitch = json!({"id":"pitch","kind":"poly_constant","params":{"channels":voices}});
    for voice in 0..voices {
        // Evenly spaced pitch offsets across the requested +/-1 V/oct range.
        let volts = if voices == 1 {
            0.0
        } else {
            -1.0 + 2.0 * voice as f32 / (voices - 1) as f32
        };
        pitch["params"][format!("v{}", voice + 1)] = json!(volts);
    }
    nodes.push(pitch);
    let stress = profile == "high_drive";
    let voice_params = if stress {
        json!({"engine":engine,"frequency":110.0,"harmonics":1.0,"timbre":1.0,"morph":1.0,"decay":0.5,"level":0.8})
    } else {
        json!({"engine":engine,"frequency":220.0,"harmonics":0.35,"timbre":0.55,"morph":0.4,"decay":0.5,"level":0.8})
    };
    nodes.push(json!({"id":"voice","kind":"macro_voice","params":voice_params}));
    cables.push(json!({"from":"pitch","to":"voice","input":"pitch"}));
    // Fixed, mono CVs exercise the public cable path and documented CV scaling.
    for (id, input, volts) in [
        ("harm", "harmonics", 1.0),
        ("timbre", "timbre", -1.0),
        ("morph", "morph", 0.5),
    ] {
        nodes.push(json!({"id":id,"kind":"poly_constant","params":{"channels":1,"v1":volts}}));
        cables.push(json!({"from":id,"to":"voice","input":input}));
    }
    serde_json::from_value(json!({"version":2,"sample_rate":RATE,"nodes":nodes,"cables":cables,"output":"voice","output_port":"out"})).unwrap()
}

fn baseline_patch(voices: usize) -> Patch {
    let mut params = json!({"channels":voices});
    for voice in 0..voices {
        let volts = if voices == 1 {
            0.0
        } else {
            -1.0 + 2.0 * voice as f32 / (voices - 1) as f32
        };
        params[format!("v{}", voice + 1)] = json!(volts);
    }
    serde_json::from_value(json!({"version":2,"sample_rate":RATE,"nodes":[{"id":"pitch","kind":"poly_constant","params":params}],"output":"pitch"})).unwrap()
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    sorted[((sorted.len() - 1) as f64 * p).ceil() as usize]
}

fn measure(
    patch: &Patch,
    block: usize,
    voices: usize,
    engine_name: &str,
    profile: &str,
    baseline: bool,
) -> Value {
    let mut engine = patch.compile().unwrap();
    let mut frames = vec![PortFrame::default(); block];
    for _ in 0..WARMUP_BLOCKS {
        engine.process_poly_block(black_box(&mut frames));
        black_box(&frames);
    }
    let mut durations = Vec::with_capacity(TIMED_BLOCKS);
    let mut checksum = 0.0_f64;
    let mut all_finite = true;
    let mut output_channels = 0_u8;
    let mut channels_match = true;
    for _ in 0..TIMED_BLOCKS {
        let start = Instant::now();
        engine.process_poly_block(black_box(&mut frames));
        let elapsed = start.elapsed().as_secs_f64() * 1e6;
        black_box(&frames);
        durations.push(elapsed);
        for frame in &frames {
            output_channels = output_channels.max(frame.channels);
            channels_match &= usize::from(frame.channels) == voices;
            for &sample in &frame.values[..usize::from(frame.channels).min(16)] {
                all_finite &= sample.is_finite();
                checksum += f64::from(sample);
            }
        }
    }
    let observations = durations.clone();
    durations.sort_by(f64::total_cmp);
    let mean = durations.iter().sum::<f64>() / durations.len() as f64;
    let budget = block as f64 / f64::from(RATE) * 1e6;
    json!({
        "patch": patch,
        "observations_us": observations,
        "sample_rate": RATE,
        "block_size": block,
        "voice_count": voices,
        "engine": engine_name,
        "profile": profile,
        "baseline": baseline,
        "iterations": TIMED_BLOCKS,
        "warmup_blocks": WARMUP_BLOCKS,
        "mean_processing_us": mean,
        "mean_us_per_frame": mean / block as f64,
        "p50_processing_us": percentile(&durations, 0.50),
        "p95_processing_us": percentile(&durations, 0.95),
        "p99_processing_us": percentile(&durations, 0.99),
        "max_processing_us": durations[durations.len()-1],
        "nominal_block_budget_us": budget,
        "mean_utilization": mean / budget,
        "p95_utilization": percentile(&durations, 0.95) / budget,
        "max_utilization": durations[durations.len()-1] / budget,
        "checksum": checksum,
        "all_finite": all_finite,
        "output_channels": output_channels,
        "channels_match": channels_match
    })
}

fn main() {
    if cfg!(debug_assertions) {
        panic!("run with --release for useful timings");
    }
    let mut rows = Vec::new();
    for voices in [1, 16] {
        for block in [64, 256] {
            rows.push(measure(
                &baseline_patch(voices),
                block,
                voices,
                "poly_constant",
                "baseline",
                true,
            ));
            for (engine, name) in ENGINES.iter().enumerate() {
                let p = patch(engine, voices, "representative");
                rows.push(measure(&p, block, voices, name, "representative", false));
            }
            for engine in [1, 2] {
                let p = patch(engine, voices, "high_drive");
                rows.push(measure(
                    &p,
                    block,
                    voices,
                    ENGINES[engine],
                    "high_drive",
                    false,
                ));
            }
        }
    }
    println!(
        "{}",
        json!({
            "benchmark": "OpenPatch native MACRO VOICE offline wall-clock profile",
            "model_note": "Eight native OpenPatch engines; FM/additive algorithms are adapted from Plaits, not a bit-identical Plaits port.",
            "qualification": "Offline process_poly_block wall-clock measurements only; not audio-device callback qualification and no Rack or physical-device comparison.",
            "cv_note": "Pitch poly_constant values span -1..+1 V; three mono poly_constant CVs feed harmonics/timbre/morph. Trigger is unpatched, so the voice remains continuous.",
            "method": "Separate single-engine workloads measured sequentially; 8 warmup blocks and 64 timed blocks per workload. Table creation, compilation and output inspection are outside the timed calls. No baseline subtraction. high_drive is a selected high-control setting, not an exhaustive worst case. With 64 observations and this nearest-rank estimator p99 equals max; these are short-run observations, not stable tail-capacity bounds. Baseline has only the pitch source and is not a matched support graph.",
            "rows": rows
        })
    );
}
