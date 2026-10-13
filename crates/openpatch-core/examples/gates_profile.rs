//! `cargo run --release -p openpatch-core --example gates_profile`
//! Measures an entire three-node source + Gates graph, not Gates alone or Rack.
use openpatch_core::{Patch, PortFrame, BLOCK_SIZE};
use serde_json::json;
use std::hint::black_box;
use std::time::{Duration, Instant};

fn main() {
    if cfg!(debug_assertions) {
        panic!("use --release");
    }
    println!("OpenPatch Gates graph; rate=48000 block={BLOCK_SIZE}; source=poly_constant+multi_vco(square,1000Hz); length=.1s; warmed 48000frames; no device playback or Rack comparison");
    println!("voices,ns_per_frame,realtime_factor,adoption_mean_us,adoption_max_us");
    for channels in [1, 16] {
        let patch: Patch = serde_json::from_value(json!({"version":2,"sample_rate":48000,
            "nodes":[{"id":"pitch","kind":"poly_constant","params":{"channels":channels}},
                {"id":"source","kind":"multi_vco","params":{"frequency":1000}},
                {"id":"timing","kind":"gates","params":{"length":0.1}}],
            "cables":[{"from":"pitch","to":"source","input":"pitch"},
                {"from":"source","output":"square","to":"timing","input":"in"}],
            "output":"timing","output_port":"delay"}))
        .unwrap();
        let mut engine = patch.compile().unwrap();
        let mut successor = patch.compile().unwrap();
        let mut frames = [PortFrame::default(); BLOCK_SIZE];
        // 2,000 edges/voice fills the 1,023-entry history before timing.
        for _ in 0..48_000_usize.div_ceil(BLOCK_SIZE) {
            engine.process_poly_block(&mut frames);
        }
        assert_eq!(frames[0].channels, channels as u8);
        let start = Instant::now();
        let mut count = 0_u64;
        while start.elapsed() < Duration::from_secs(2) {
            for _ in 0..128 {
                engine.process_poly_block(black_box(&mut frames));
                black_box(&frames);
            }
            count += 128 * BLOCK_SIZE as u64;
        }
        let elapsed = start.elapsed().as_secs_f64();
        let mut sum = 0.0_f64;
        let mut max = 0.0_f64;
        for _ in 0..1000 {
            let start = Instant::now();
            successor.adopt_state_from(black_box(&engine));
            black_box(&successor);
            let micros = start.elapsed().as_secs_f64() * 1e6;
            sum += micros;
            max = max.max(micros);
        }
        println!(
            "{channels},{:.2},{:.2},{:.3},{max:.3}",
            elapsed * 1e9 / count as f64,
            count as f64 / 48_000.0 / elapsed,
            sum / 1000.0
        );
    }
}
