//! Run with `cargo bench -p openpatch-core --bench engine`.
//! Measures this engine only; results are not a comparison with another synth.
use openpatch_core::{Cable, Node, Patch, BLOCK_SIZE};
use std::collections::BTreeMap;
use std::hint::black_box;
use std::time::{Duration, Instant};

fn voice_patch(voices: usize) -> Patch {
    let mut patch = Patch::default();
    let node = |id: String, kind: &str, params: BTreeMap<String, f32>| Node { id, kind: kind.into(), params };
    for i in 0..voices {
        patch.nodes.push(node(format!("osc{i}"), "oscillator", BTreeMap::from([("frequency".into(), 110.0 + i as f32 * 7.0), ("amplitude".into(), 0.8)])));
    }
    let mut sum = "osc0".to_string();
    for i in 1..voices {
        let id = format!("mix{i}");
        patch.nodes.push(node(id.clone(), "mixer", BTreeMap::new()));
        patch.cables.push(Cable { from: sum, to: id.clone(), input: "a".into() });
        patch.cables.push(Cable { from: format!("osc{i}"), to: id.clone(), input: "b".into() });
        sum = id;
    }
    patch.nodes.push(node("master".into(), "gain", BTreeMap::from([("gain".into(), 1.0 / voices as f32)])));
    patch.cables.push(Cable { from: sum, to: "master".into(), input: "signal".into() });
    patch.output = Some("master".into());
    patch
}

fn command_output(program: &str, args: &[&str]) -> Option<String> {
    std::process::Command::new(program).args(args).output().ok().filter(|output| output.status.success()).map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn cpu_name() -> String {
    if let Ok(cpu) = std::env::var("PROCESSOR_IDENTIFIER") { return cpu; }
    if let Some(cpu) = command_output("sysctl", &["-n", "machdep.cpu.brand_string"]) { return cpu; }
    if let Ok(cpuinfo) = std::fs::read_to_string("/proc/cpuinfo") {
        if let Some(line) = cpuinfo.lines().find(|line| line.starts_with("model name") || line.starts_with("Hardware")) {
            return line.split_once(':').map(|(_, value)| value.trim().to_string()).unwrap_or_else(|| line.to_string());
        }
    }
    "unavailable (record hardware separately)".into()
}

fn main() {
    assert!(!cfg!(debug_assertions), "run cargo bench, using optimized release settings");
    println!("OpenPatch original DSP benchmark; no external synth comparison");
    println!("os={} arch={} cpu={} logical_cpus={}", std::env::consts::OS, std::env::consts::ARCH, cpu_name(), std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1));
    println!("compiler={}", command_output("rustc", &["--version"]).unwrap_or_else(|| "unavailable".into()));
    println!("profile=bench sample_rate=48000 block_size={BLOCK_SIZE} duration_per_case>=2s warmup_blocks=1000 single_thread=true");
    println!("voices,nodes,blocks,wall_seconds,audio_seconds,realtime_factor,ns_per_sample");
    for voices in [1, 16, 64] {
        let patch = voice_patch(voices);
        let mut engine = patch.compile().unwrap();
        let mut output = [0.0; BLOCK_SIZE];
        for _ in 0..1000 { engine.process_block(black_box(&mut output)); black_box(&output); }
        let start = Instant::now();
        let mut blocks = 0u64;
        loop {
            // Amortize timer overhead; black_box keeps rendered work observable.
            for _ in 0..256 { engine.process_block(black_box(&mut output)); black_box(&output); }
            blocks += 256;
            if start.elapsed() >= Duration::from_secs(2) { break; }
        }
        let elapsed = start.elapsed().as_secs_f64();
        let samples = blocks * BLOCK_SIZE as u64;
        let audio_seconds = samples as f64 / patch.sample_rate as f64;
        println!("{voices},{},{blocks},{elapsed:.6},{audio_seconds:.6},{:.2},{:.2}", patch.nodes.len(), audio_seconds / elapsed, elapsed * 1e9 / samples as f64);
    }
}
