use openpatch_core::{Patch, TapSample};
use serde_json::{json, Value};
use std::{hint::black_box, io::Write, time::Instant};

const FRAMES: usize = 128;
const WARMUP: usize = 512;
const MEASURED: usize = 4096;

fn graph(rate: u32, voices: usize, drive: f32, modulation: bool) -> Value {
    let mut nodes = vec![
        json!({"id":"osc", "kind":"oscillator", "params":{"frequency":1000.0,"amplitude":1.0}}),
        json!({"id":"gain", "kind":"gain", "params":{"gain":5.0}}),
        json!({"id":"merge", "kind":"poly_merge", "params":{"channels":voices}}),
        json!({"id":"filter", "kind":"svf_filter", "params":{"cutoff":12000.0,"resonance":0.2,"drive":drive,"cutoff_cv_amount":if modulation {1.0} else {0.0}}}),
    ];
    let mut cables = vec![json!({"from":"osc","to":"gain","input":"signal"}),
        json!({"from":"merge","to":"filter","input":"signal"})];
    for voice in 1..=voices {
        cables.push(json!({"from":"gain","to":"merge","input":format!("in{voice}")}));
    }
    if modulation {
        nodes.push(json!({"id":"lfo","kind":"lfo","params":{"frequency":13.0,"amplitude":0.5,"offset":0.0}}));
        cables.push(json!({"from":"lfo","to":"filter","input":"cutoff_cv"}));
    }
    json!({"version":2,"sample_rate":rate,"nodes":nodes,"cables":cables,"output":"filter","output_port":"low"})
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    sorted[((sorted.len() as f64 * p).ceil() as usize).saturating_sub(1).min(sorted.len()-1)]
}

fn summary(sorted: &[u64], deadline_ns: f64) -> Value {
    let p50 = percentile(sorted, 0.5);
    let p95 = percentile(sorted, 0.95);
    let p99 = percentile(sorted, 0.99);
    let max = *sorted.last().unwrap();
    json!({"blocks":sorted.len(), "p50_us":p50 as f64/1000.0,"p95_us":p95 as f64/1000.0,
        "p99_us":p99 as f64/1000.0,"max_us":max as f64/1000.0,
        "mean_us":sorted.iter().map(|&v|v as f64).sum::<f64>()/sorted.len() as f64/1000.0,
        "deadline_us":deadline_ns/1000.0,
        "p50_deadline_ratio":p50 as f64/deadline_ns,"p95_deadline_ratio":p95 as f64/deadline_ns,
        "p99_deadline_ratio":p99 as f64/deadline_ns,"max_deadline_ratio":max as f64/deadline_ns,
        "blocks_exceeding_nominal_deadline":sorted.iter().filter(|&&v|v as f64>deadline_ns).count()})
}

fn main() {
    assert!(!cfg!(debug_assertions), "use release builds");
    let args: Vec<String> = std::env::args().collect();
    let variant = &args[1];
    let output_path = &args[2];
    // Reserve the report before timing; refuse accidental replacement.
    let mut output_file = std::fs::OpenOptions::new().write(true).create_new(true)
        .open(output_path).expect("performance report must be a new file");
    let mut empty_timer_ns = Vec::with_capacity(MEASURED);
    for _ in 0..MEASURED {let now=Instant::now();black_box(());empty_timer_ns.push(now.elapsed().as_nanos() as u64);}
    empty_timer_ns.sort_unstable();
    let start = Instant::now();
    let mut cases = Vec::new();
    for rate in [44100,48000,96000] {
        for voices in [1,16] {
            for modulation in [false,true] {
                for drive in [0.0,0.1,1.0] {
                    let patch_json = graph(rate,voices,drive,modulation);
                    let patch: Patch = serde_json::from_value(patch_json.clone()).unwrap();
                    let mut engine = patch.compile().unwrap();
                    let taps = [Some(engine.resolve_tap("filter","low",0).unwrap()),
                        Some(engine.resolve_tap("filter","low",if voices==16 {15} else {1}).unwrap())];
                    let mut audio = [[0.0;2];FRAMES];
                    let mut capture = [[TapSample::default();2];FRAMES];
                    engine.process_audio_block_with_taps(&mut audio,&taps,&mut capture);
                    // Version-2 cables publish synchronous frames; allow startup propagation.
                    assert!(capture[FRAMES/2..].iter().all(|s|s[0].active && s[0].value.is_finite()
                        && s[1].active==(voices==16) && s[1].value.is_finite()));
                    for _ in 0..WARMUP {engine.process_audio_block(black_box(&mut audio));black_box(&audio);}
                    let mut timings = Vec::with_capacity(MEASURED);
                    let case_start = Instant::now();
                    for _ in 0..MEASURED {
                        let block_start = Instant::now();
                        engine.process_audio_block(black_box(&mut audio));
                        black_box(&audio);
                        timings.push(block_start.elapsed().as_nanos() as u64);
                    }
                    let measured_wall_seconds = case_start.elapsed().as_secs_f64();
                    assert!(audio.iter().flatten().all(|v|v.is_finite()));
                    let mut sorted = timings.clone(); sorted.sort_unstable();
                    let stats = summary(&sorted,FRAMES as f64/rate as f64*1e9);
                    eprintln!("{variant}: rate={rate} voices={voices} drive={drive} modulation={modulation} p50={}us p99={}us max={}us",stats["p50_us"],stats["p99_us"],stats["max_us"]);
                    cases.push(json!({"sample_rate":rate,"voices":voices,"drive":drive,"modulation":modulation,
                        "warmup_blocks":WARMUP,"statistics":stats,"measured_wall_seconds":measured_wall_seconds,
                        "last_output":audio[FRAMES-1],"patch":patch_json,"raw_block_ns":timings}));
                }
            }
        }
    }
    let report = json!({"variant":variant,"frames_per_block":FRAMES,"warmup_blocks":WARMUP,
        "measured_blocks_per_case":MEASURED,"case_order":"rate, voices, modulation, drive",
        "profile":"release; thin LTO; codegen-units=1","total_wall_seconds":start.elapsed().as_secs_f64(),
        "timer_overhead_ns":{"p50":percentile(&empty_timer_ns,0.5),"p95":percentile(&empty_timer_ns,0.95),"p99":percentile(&empty_timer_ns,0.99),"max":empty_timer_ns.last()},
        "method":"Single-thread sequential headless wall-clock timing of public Engine::process_audio_block; no audio device or UI; nearest-rank percentiles; timer overhead retained. One prewarm tap block validates active channel 0 and active channel 15 / inactive channel 1. All patches version 2.",
        "limitations":["Headless processing cost does not qualify device underruns or callback scheduling.","Wall-clock tails include OS scheduling; no realtime priority or core pinning.","One run per case; same oscillator broadcasts identical voltage to all active lanes; independent pitches are not covered.","Nominal block deadline ratios are processing budgets, not measured audio deadline misses."],
        "cases":cases});
    output_file.write_all(serde_json::to_string_pretty(&report).unwrap().as_bytes()).unwrap();
}
