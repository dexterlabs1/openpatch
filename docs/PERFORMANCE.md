# Performance measurement

The goal is lower CPU use and predictable audio deadlines on large patches.
Rust alone is not evidence of a speedup. No VCV Rack comparison is available yet.

Run release benchmarks using `cargo bench -p openpatch-core`, or MCP `benchmark`
on the current patch. The MCP result records sample rate, block size, node count,
OS/architecture, build mode, elapsed time, frames/s and realtime factor.
Compilation is excluded. Output is consumed to prevent the compiler discarding
the workload. A realtime factor of 20 means the offline run produced 20 seconds
of audio per wall-clock second; it does not guarantee live glitch-free playback.

The benchmark also measures the six-module reference voice, including the 8×
oversampled ladder and VCA. On an Apple M1 Max, Rust 1.99.0 release build at
48 kHz/128-frame blocks, a local 2026-10-12 run rendered 12.619 seconds in
2.000 seconds: **6.31× realtime**. This is one offline throughput observation,
not a live deadline test or VCV comparison. The simple legacy oscillator results
are not representative of the cost of these circuit models.

## Version-2 foundation measurements

Repeat the four included workloads with:

```sh
cargo build --locked --release -p openpatch
python3 scripts/benchmark_engines.py --seconds 1 --runs 3
```

A local Apple M1 Max / Rust 1.99.0 release run at 48 kHz and 128-frame blocks,
using three repetitions per workload, produced the following offline factors.
The script includes compilation-free processing of **all active voices**, even
though the mono output adapter selects one channel. The polyphonic voice uses
SUM before the audio interface. [Raw patches and results](benchmarks/engine-v2-m1max.json).

| Workload | Median × realtime | Observed range |
|---|---:|---:|
| Version 1 reference voice, one voice | 6.369 | 5.867–6.369 |
| Version 2 reference voice, one voice | 6.275 | 6.268–6.295 |
| Version 2 chord, three circuit voices | 2.113 | 2.095–2.116 |
| Version 2 chord, sixteen circuit voices | 0.390 | 0.387–0.391 |

The sixteen-voice oversampled circuit workload is **slower than realtime on this
machine**. Routing sixteen channels is supported; this measurement does not
promise that every sixteen-voice patch can play without underruns. SIMD and
circuit-processing optimization remain necessary. Keep these results distinct
from simpler polyphonic routing/utility workloads, callback deadline tests, and
any future VCV comparison. The version-1 and version-2 graphs also intentionally
have different cable timing, so these figures are diagnostic overhead/scaling
observations, not an identical-signal comparison or claimed speedup.

## Fair comparison protocol

- Same machine, audio rate, channel/voice count, oscillator/filter algorithms,
  modulation and feedback semantics, output quality, and active signal paths.
- Release builds, documented compiler and application versions, power mode,
  thread count, audio backend, and device buffer size.
- Separate headless DSP measurements from UI/GPU overhead and device latency.
- Warm up, repeat, report median and spread. Live tests additionally report p95,
  p99, worst callback duration, xruns and memory use.
- Keep benchmark patches, commands, raw results and audio correctness checks.
- Do not compare a simple OpenPatch sine with an oversampled or bandlimited
  Rack oscillator and label the difference an engine speedup.

CI timing is diagnostic: shared runners are noisy. No hard speed threshold or
speedup claim should be based on one CI run. SIMD, multicore scheduling and
feedback optimization remain future work.
