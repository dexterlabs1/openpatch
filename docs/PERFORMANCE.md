# Performance measurement

The goal is lower CPU use and predictable audio deadlines on large patches.
Rust alone is not evidence of a speedup. No VCV Rack comparison is available yet.

Run release benchmarks using `cargo bench -p openpatch-core`, or MCP `benchmark`
on the current patch. The MCP result records sample rate, block size, node count,
OS/architecture, build mode, elapsed time, frames/s and realtime factor.
Compilation is excluded. Output is consumed to prevent the compiler discarding
the workload. A realtime factor of 20 means the offline run produced 20 seconds
of audio per wall-clock second; it does not guarantee live glitch-free playback.

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
