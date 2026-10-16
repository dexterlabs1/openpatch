# Performance measurement

The goal is lower CPU use and predictable audio deadlines on large patches.
Rust alone is not evidence of a speedup. No VCV Rack comparison is available yet.

## Native macro voice observation

The [MACRO VOICE contract](MACRO_VOICE.md) includes a public Engine profile for
all eight engines, separately at one and sixteen voices, 48 kHz and 64/256-frame
blocks. A local Apple M1 Max / Rust 1.99.0 release run on 2026-10-15 produced
these sixteen-voice, 256-frame representative observations. Each block has
5,333.33 µs of nominal frame time. Each row is a separate graph, not eight
engines running simultaneously.

| Engine | Median µs/block | Maximum µs/block | Mean frame-time budget use |
|---|---:|---:|---:|
| Analog | 233.92 | 307.29 | 4.40% |
| Wavefold | 1,098.33 | 3,359.79 | 23.26% |
| FM | 1,873.75 | 2,740.67 | 36.25% |
| Harmonic | 423.29 | 462.79 | 7.96% |
| Wavetable | 420.83 | 478.88 | 7.95% |
| Chord | 261.04 | 310.71 | 4.92% |
| Swarm | 477.79 | 789.42 | 9.11% |
| Metallic | 275.04 | 313.17 | 5.20% |

The selected high-drive Wavefold/FM graphs had mean budget use of 24.15% and
35.28%. They are additional fixtures, not exhaustive worst-case bounds. The
source-only baseline median was 3.38 µs/block; it has fewer sources and cables,
and no subtraction is used to claim isolated oscillator cost. All 44 workloads
reported finite output and the requested channel count throughout measured
blocks. [Raw observations, actual patches, source hashes, binary hash, toolchain
and hardware provenance](benchmarks/macro-voice-m1max.json).

Each workload uses eight warmup blocks and 64 timed blocks, excluding compilation,
table creation and output inspection. With this short sample count the reported
p99 equals the maximum; neither is a stable long-run tail bound. Graphs run in a
fixed sequential order without affinity, thermal control or background-load
isolation. These are offline graph timings, not live audio callback measurements,
dropout qualification or VCV Rack speed comparisons. FM and Wavefold are the
largest costs in this measured matrix and remain optimization candidates.

## Live callback diagnostics

![Native Audio performance window with buffer timing and overrun counters](images/openpatch-audio-performance.png)

*Native Linux / Mesa CI render using clearly labeled simulated data for layout
validation. These displayed numbers are not a live device measurement.*

Open **View → Audio performance**, or click **DSP** in the footer. The native
app shows current, peak and frame-weighted mean processing budget use, actual
buffer size/rate, interval callback counts and processing-budget overruns.
**Reset statistics** starts a new interval at the next callback; until then it
shows a pending reset. Reconnecting audio creates fresh statistics. Transport
pause still runs the callback and is measured.

Two monotonic clock reads measure elapsed wall time around graph adoption, state transfer, crossfade,
DSP, device sample conversion and existing telemetry publication. The timing
accumulator and its own atomic publication happen afterward and are excluded
from the reported duration. Preemption during processing is included; scheduling
lateness before callback entry is not. Storage is fixed; this adds no callback allocations,
locks, logging or per-sample clock reads. Readers make at most four attempts at
an independently coherent timing snapshot. Missing, closed or contended data
is shown as unavailable, never as zero budget use.

A callback's budget is its actual complete frames divided by the device sample
rate. A processing-budget overrun means elapsed nanoseconds × sample rate is
strictly greater than frames × 1,000,000,000, using integer arithmetic. This is
**not a hardware xrun counter**: it does not measure late delivery by the audio
driver or OS before callback entry. Zero-frame/channel callbacks have no utilization and do not
enter valid interval totals or peaks. Counters saturate rather than wrap.

MCP `inspect_performance.performance.audio_timing` exposes the same read-only
snapshot with its own completed-callback sequence, graph revision and reset
epoch. It is independent of module-state publication; consumers must not assume
those snapshots describe the same instant. See [the MCP contract](MCP.md).

For repeatable **headless callback** observations (no device is opened):

```sh
OPENPATCH_CALLBACK_PROFILE=/tmp/openpatch-callbacks.json \
  cargo test --release -p openpatch-desktop callback_timing_profile -- --ignored --nocapture
```

This harness covers one/sixteen functional oscillator, circuit-voice and full
Gates-history workloads at 48 kHz with 256-frame stereo buffers. It alternates
instrumented/uninstrumented timing order (comparing each batch's final buffer),
checks every warmup/steady/transition buffer for exact output equality, and
records graph checkpoint, cable edit and fresh-replacement adoption buffers.
Compilation is excluded. Timing results are observations, not CI pass/fail
thresholds or proof of hardware deadlines. The callback's 5 ms crossfade fits
inside each measured 256-frame adoption buffer. Cable edits can reduce the
active voice width. Initial Gates graphs and checkpoint transitions have full
histories; repeated fresh replacements start new histories.

### Headless callback observation — Apple M1 Max

A local release run of `058d3dd` on 2026-10-13 (Rust 1.99.0), before the
paired-circuit optimization below, produced these results.
Each buffer has **5.333 ms** of frame-time budget. Steady observations use 256
callbacks per workload; checkpoint transitions use 16 observations and include
state adoption plus both graphs during the crossfade.
[Raw timings, patches, source hashes and method](benchmarks/callback-timing-m1max.json).

| Workload | Steady median ms | Steady p99 ms | Checkpoint median ms | Steady overruns |
|---|---:|---:|---:|---:|
| Functional, 1 voice | 0.026 | 0.028 | 0.061 | 0/256 |
| Functional, 16 voices | 0.312 | 0.373 | 0.637 | 0/256 |
| Circuit, 1 voice | 0.597 | 0.663 | 1.205 | 0/256 |
| Circuit, 16 voices | 8.755 | 9.254 | 17.570 | 256/256 |
| Gates, 1 voice | 0.060 | 0.061 | 0.129 | 0/256 |
| Gates, 16 voices | 0.689 | 0.768 | 1.384 | 0/256 |

The sixteen-voice circuit patch exceeds this processing budget even without an
edit, and its checkpoint crossfade roughly doubles the cost. This makes circuit
processing and edit transitions the next optimization targets. These are
headless callback observations, not live audio-device distributions.

Five alternating enabled/raw batches per workload also measured the full
instrumented call, including statistics publication. Differences crossed zero
and varied with background activity (the one-voice functional median difference
was about 0.143 µs per callback). This run does **not** resolve a reliable precise
instrumentation overhead. Checked output buffers matched exactly. It establishes
neither faster-than-Rack performance nor glitch-free hardware playback.

## Paired circuit voices

The version-2 engine now processes independent ladder and OTA VCA voices in
pairs. Each FIR coefficient is shared across two independent accumulation
chains. Mono and odd final voices retain scalar processing. The equations,
f64 precision, tap order, separate multiply/add operations, eight circuit steps,
solver limits and 64-sample latency are unchanged. Each voice retains its own
history, parameters and diagnostics; persistent state size is unchanged.

The source expresses independent scalar lanes, but the final M1 Max release
executable with ThinLTO already vectorizes most interpolation arithmetic.
Its inspected interpolation loops do not spill accumulators to the stack.
Paired decimation uses packed adjacent-tap products followed by ordered scalar
additions. Earlier inspection of intermediate, pre-LTO assembly understated
this optimization; the actual timed executable is the relevant evidence.
New tests compare every sample and
internal solver state against independent scalar instances and frozen reference
code, including unequal histories/parameters, control-history draining, invalid
inputs, state adoption and voice shrink/regrowth. Public Engine tests cover odd
and even widths, CV normaling/broadcast/zero-padding, bypass and callback splits.
Existing response, alias rejection, latency and callback allocation tests remain
in place. These tests do not constitute commercial-hardware calibration.

A local Apple M1 Max / Rust 1.99.0 release comparison used five randomized,
interleaved pairs per workload at 48 kHz and 128-frame blocks. Each timed call
renders one audio second after warmup; compilation/RPC are excluded.
[Raw patches, timings, compiler details, binary and source hashes](benchmarks/paired-circuits-m1max.json).

| Workload | Before µs/frame | After µs/frame | Processing time reduction |
|---|---:|---:|---:|
| Ladder, one voice | 1.603 | 1.596 | 0.4% |
| Ladder, sixteen voices | 25.183 | 21.064 | 16.4% |
| OTA VCA, one voice | 0.724 | 0.732 | -1.1% |
| OTA VCA, sixteen voices | 10.898 | 6.736 | 38.2% |
| Full circuit voice, one voice | 2.319 | 2.294 | 1.1% |
| Full circuit voice, sixteen voices | 35.927 | 28.079 | 21.8% |

With audio-rate CV, sixteen-voice reductions were **14.4% for the ladder** and
**36.7% for the VCA**. Single-voice VCA median time increased by about 1.1%
(0.8% with audio-rate CV); those paired timing ranges crossed no change. The
single-voice full patch improved about 1.1%. These small changes should be read
alongside the raw ranges, not as universal speed guarantees. The sixteen-voice
full patch improved about **21.8%**, from approximately 0.580× to **0.742×
realtime**: it still runs slower than realtime on this machine.

All 50 paired benchmark checksums matched; all ten per-channel analyses matched
exactly and reported zero solver failures over their 0.5-second windows. This is
a comparison with the previous OpenPatch implementation, **not VCV Rack**.
Background activity, thermal state and lack of core affinity limit the result.

A separate headless callback profile of the final build recorded a sixteen-voice
circuit steady median of **6.885 ms**, versus **9.117 ms** in a fresh baseline
profile. Checkpoint medians were **13.718 ms** versus **18.286 ms**. Both builds
exceeded the 5.333 ms budget on all 256 steady circuit buffers. One-voice circuit
medians were 0.604 ms versus 0.575 ms; the functional and Gates workloads remained
well below budget at their medians. These are single sequential profiles per
build, so they do not isolate small regressions or establish a precise callback
speedup. The randomized offline comparison above provides stronger evidence of
the processing improvement.
[Complete callback observations and binary provenance](benchmarks/paired-circuit-callbacks-m1max.json).
Within each build, all checked instrumented/raw buffers matched exactly. This
comparison does not measure audio-device behavior or scheduling lateness.

## Four-voice circuit traversal

`a1ef5a4` extends the paired implementation by processing circuit voices in
groups of four. Interpolation still runs as two pairs; decimation traverses
the coefficients once for four independent sums. Smaller remainders use the
existing pair/scalar paths. No precision, equation, tap order, solver limit,
oversampling factor, latency, or persistent-state layout changes are involved.
The code uses portable Rust; inspection of the actual ARM64 release CLI after
ThinLTO confirms two packed accumulation chains with separate multiply/add.

Five randomized, interleaved baseline/candidate pairs per workload on the same
M1 Max (Rust 1.99.0, 48 kHz, 128-frame blocks, one audio second per call) gave:

| Workload | Paired baseline µs/frame | Quartet µs/frame | Processing time reduction |
|---|---:|---:|---:|
| Ladder, sixteen voices | 21.032 | 19.170 | 8.9% |
| OTA VCA, sixteen voices | 6.714 | 4.691 | 30.1% |
| Full circuit voice, sixteen voices | 27.957 | 24.064 | 13.9% |
| Ladder with audio-rate CV, sixteen voices | 22.389 | 20.472 | 8.6% |
| OTA VCA with audio-rate CV, sixteen voices | 7.070 | 5.064 | 28.4% |

Mono median changes were between a 0.56% reduction and a 0.91% increase. All
mono paired ranges except the full-voice patch crossed no change; its median
improved 0.32%, too small to treat as a universal speed guarantee. All 50 benchmark checksums and ten channel
analyses matched exactly, with zero reported solver failures. The full sixteen-
voice patch improved from about 0.745× to **0.866× realtime**, still below
realtime on this machine. These figures compare OpenPatch builds, not VCV Rack.
[Raw workloads, timings, output checks and provenance](benchmarks/quartet-circuits-m1max.json).

New tests compare quartet phases, full ring histories, solver state and
diagnostics with independent scalar trajectories. Public-engine coverage includes
widths 1/2/3/4/5/7/8/15/16, quartet/pair/scalar tails, parameter adoption,
regrowth, and a 4→3→4 transition. Response, alias rejection, latency and callback
allocation checks remain in the suite.

A final headless callback profile recorded a **5.845 ms** steady median for the
sixteen-voice circuit patch, versus 6.841 ms in the fresh paired baseline.
Both profiles exceeded the 5.333 ms budget on all 256 measured steady buffers.
The candidate checkpoint median was 11.756 ms. This is progress toward the
budget, not realtime qualification: there is still no headroom, and edits are
more expensive. These sequential profiles are observations, not controlled
estimates of small timing changes or audio-device stability.
[Full callback profiles and provenance](benchmarks/quartet-circuit-callbacks-m1max.json).

## Offline engine throughput

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

## Circuit implementation comparison

The `a9144a0` optimization preserves the circuit equations, coefficients,
8× oversampling, 64-sample latency per circuit module, and solver tolerances.
It removes repeated kernel copies, uses mirrored histories and independent
interpolation phase lanes, and reuses already evaluated Newton currents. Four
frozen-reference tests compare exact floating-point outputs, callback order,
solver state and diagnostics against the previous scalar implementation;
independent circuit-response and allocation tests remain in the suite.

On an Apple M1 Max, five randomized/interleaved pairs per workload at 48 kHz
and 128-frame blocks produced these **median offline processing times**. Each
call renders two audio seconds after warmup; compilation and RPC are excluded.
Both binaries were built with Rust 1.99.0 in release mode. See the
[full report](benchmarks/circuit-optimization-m1max.md) and
[patches, binary hashes and raw results](benchmarks/circuit-optimization-m1max.json).

| Workload | Before µs/frame | After µs/frame | Processing time reduction |
|---|---:|---:|---:|
| Ladder, one voice | 2.267 | 1.582 | 30.2% |
| Ladder, sixteen voices | 35.831 | 24.871 | 30.6% |
| OTA VCA, one voice | 1.083 | 0.701 | 35.2% |
| OTA VCA, sixteen voices | 16.919 | 10.833 | 36.0% |
| Full reference voice, one voice | 3.339 | 2.260 | 32.3% |
| Full reference voice, sixteen voices | 53.354 | 35.775 | 33.0% |

Audio-rate-CV cases reduced processing time by 29.1–29.7% for the ladder and
34.6–35.4% for the VCA. All 50 paired benchmark checksums matched; all ten
per-channel analyses matched exactly and observed zero solver failures during
their 0.5-second windows. These checks are evidence for the stated workloads,
not an exhaustive proof for every patch.

The full reference workloads achieved **9.22× realtime for one voice** and
**0.582× for sixteen**. The sixteen-voice case therefore remains slower than
realtime. These are approximately 1.48–1.49× throughput improvements over the
previous OpenPatch implementation, not over VCV Rack. Background system activity,
thermal state and lack of core affinity limit this single-machine comparison.

Mirrored storage adds **5,144 bytes per oversampler**. A compiled version-2 graph
with one ladder and one VCA preallocates 16 voices plus a template for each, so
its histories grow by **174,896 bytes (about 171 KiB)** even with one active voice.
Live state transfer copies these larger histories. Process RSS and callback-tail
cost during a live crossfade have not been measured.

Reproduce a paired comparison using two separately built release CLI binaries:

```sh
python3 scripts/compare_engines.py \
  --baseline /path/to/baseline/openpatch \
  --candidate target/release/openpatch \
  --baseline-source 98edb73 --candidate-source a9144a0 \
  --runs 5 --seconds 2 --output comparison.json
```

The standard-library script includes all ten patches, binary hashes and every
raw timing. It shuffles workload order and interleaves baseline/candidate calls,
checks benchmark checksums and per-channel analysis, and exits unsuccessfully
if the checked signals differ. It does not build either binary or infer source
identity: provide source IDs that match the binaries you built. These offline
measurements do not establish live device deadlines or a VCV Rack speedup.

## Fair comparison protocol

To include stacked-input routing workloads, run
`python3 scripts/benchmark_engines.py --stacked --seconds 1 --runs 3` after
building the release CLI. This adds fixed two-module graphs with 1, 16, and
1,024 cables at both one and sixteen voices, plus the stacked pitch example.
The fixed module count helps isolate routing cost. These remain offline
throughput measurements; they do not measure live callback deadlines or Rack.

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
speedup claim should be based on one CI run. Broader SIMD work, multicore
scheduling and feedback optimization remain future work.

## SVF nonlinear-drive integration and prototype screen

The [SVF antialiasing study](benchmarks/svf-antialias/README.md) compares isolated
4×/8×/16× drive preprocessing against a pinned historical host-rate baseline. The broader
spectral tests reject 4×/8×; unoptimized 16× passes the measured sparse spectral
matrices but exceeds the 48/96 kHz 128-frame block budget with sixteen voices.
The study retains source hashes, independent
spectral references and 589,824 measured actual-engine blocks to guide an
optimization that preserves sound quality. Offline cost is not device stability
or a faster-than-Rack result.

Two subsequent bounded screens compare implementations at 16×. Processing voice
quartets retains scalar bit equality in the tested cases and reduces the
16-voice maximum-drive median by about 2.6×; mono cost is essentially unchanged.
An original [four-stage resampler](benchmarks/svf-antialias/multistage/README.md)
passes the 144-case engine, 198-case extended and 234-case dense spectral gates,
with 76 samples of additional latency instead of 64. Its paired median is about
3× faster for mono and 3.6× for sixteen voices than the unoptimized 16× prototype.
The initial 96 kHz/16-voice maximum-drive p99 still exceeds the nominal block
budget. These are experimental alternatives, with public source and measured
tails; they do not establish callback reliability or installed-build behavior.

The [SVF source integration](SVF_FILTER.md) uses the original multistage design
with gain, blend and normalization denominator calculated during graph
construction and shared across voices. Signal and per-voice cutoff CV receive 76 samples of delay,
including at zero drive. The host-rate TPT recurrence, ladder/VCA processing
and graph scheduling remain separate. The tables above measure frozen
prototypes. A separate integrated-source screen is recorded below; actual
audio-device behavior remains unqualified.

A [bounded alternating-order repeat](benchmarks/svf-antialias/multistage/tail-repeat.json)
reused unchanged prototype binaries. Multistage 96 kHz/16-voice maximum-drive
p99 was .862/.873/.870 ms across three repeats, with 0/2/0 over-budget blocks
out of 1024 and maximum 1.924 ms. The initial p99 was 1.679 ms against a
1.333 ms budget; its over-budget samples clustered at indices 439–469.
Repeats do not establish the cause or erase the initial result. The full cost
matrix, voice transitions, graph swaps, silence/decay and device deadlines
remain separate qualification work. No VCV performance superiority is claimed.

The [integrated-source screen](benchmarks/svf-integration-m1max.json) uses the
same eight-case harness and thin-LTO profile, with the original multistage and
integrated binaries built before sequential runs. At 16 voices and maximum
drive, integrated 48/96 kHz p50 was .728/.744 ms and p99 .817/.820 ms. Zero-drive
16-voice p50 was .450/.458 ms versus .672/.678 ms in the unhoisted comparison.
Maximum-drive cost is broadly unchanged; the zero-drive identity path avoids
unneeded nonlinear evaluation while maintaining FIR/CV histories. This is a
bounded screen with 1024 timed blocks per case, not the earlier full matrix.
The integrated 96 kHz maximum-drive maximum was 1.692 ms, above the 1.333 ms
budget. Raw report hashes, source hashes and all summary statistics are retained.
Numerical, state and allocation tests support integration; device deadlines,
larger mixed patches and broader hardware remain open.

### Stock VCF callback observation

The separate stock-style digital VCF has a [real callback report](benchmarks/stock-vcf-callback-profile.json)
for a VCO → VCF → Sum patch at 48 kHz/256 frames on an Apple M1 Max. Steady
p99 was 0.128 ms at one voice and 0.688 ms at sixteen voices; the largest
observed sixteen-voice adoption/crossfade buffer was 1.367 ms against a 5.333 ms
block deadline. Audio matched the uninstrumented callback exactly. These
headless observations do not qualify device latency/xruns, hard real-time
scheduling, or a performance advantage over Rack. See [model contract](STOCK_VCF.md).


### Stock VCA voice callbacks

The [stock VCA report](benchmarks/stock-vca-callback-profile.json) measures a
connected VCO → Stock VCF → Stock VCA → Sum patch with shared mono ADSR EG at
48 kHz/256 stereo frames on the same M1 Max. Steady p99 was 0.156 ms for one
voice and 0.729 ms for sixteen voices. The largest observed sixteen-voice
adoption/crossfade buffer was 1.413 ms against the 5.333 ms block deadline.
The actual callback matched its uninstrumented audio exactly, and separate
first-buffer/adoption tests found no heap activity. This measures the complete
headless workload rather than isolated amplification or physical device behavior;
it does not demonstrate a VCV Rack speed advantage.


### Retained host-port transport: measured incremental cost

Stock Mid/Side requires retained raw input/output storage and ordered connection
history. The version-2 host now maintains that storage even before a native
consumer is added. Legacy patches therefore incur the transport cost too.
The [four-attempt report](benchmarks/host-transport-overhead.json) retains every
paired measurement, workload, binary/source identity, signal-statistics check,
and summary, including the reverted publication-fusion experiment.

The selected final candidate was measured against main `5844d26` using separate
release artifacts and an isolated candidate target directory. On one Apple M1
Max at 48 kHz, five alternating-order pairs per workload gave:

| Workload | Main median ns/frame | Candidate median ns/frame | Median paired time increase |
|---|---:|---:|---:|
| Routing, mono | 261.01 | 342.41 | 30.11% |
| Routing, 16 channels | 1204.43 | 1325.78 | 10.24% |
| Mixed stock/analog voice, mono | 1693.69 | 1728.39 | 2.01% |
| Mixed stock/analog voice, 16 voices | 19595.71 | 19659.45 | 0.24% |

This is a correctness foundation with a measurable routing regression, not a
performance-neutral change. The mixed sixteen-voice difference is near observed
within-run variation. The final small mono branch does not establish a material
speedup over the earlier candidates. Exact reported signal statistics and all
warmup/timed checksums match the baseline. The harness's completed status means
those experiment guards passed; it imposes no performance-regression threshold.

Each timed call renders five seconds of audio; that is not five seconds of
wall-clock measurement. Four offline workloads on one host do not qualify
compilation, adoption, memory, callback deadlines, physical devices, UI load or
VCV Rack performance. A shared-target build that returned the old baseline
executable was rejected before timing and retained in the report. Broader CPU
optimization and callback qualification remain open.

## Stock Mix and explicit FTZ/DAZ: measured incremental cost

The Stock Mix milestone also makes canonical stacked-cable addition follow
Rack's signed FTZ/DAZ behavior without changing thread-wide floating-point
flags. This applies to retained host transport in existing version-2 patches,
including patches with no Stock Mix. It adds measurable work.

The final retained experiment compares the prior qualified PR15 release CLI
(catalog 74) with the Stock Mix release CLI (catalog 75) on the same M1 Max.
The unchanged four workloads run at 48 kHz, five seconds per measurement and
five alternating pairs. Before timing, reported signal statistics match exactly;
all warmup and timed checksums match. Actual catalogs and binary hashes are
checked before any analysis or timing. Separate source/build manifests and
both complete wire logs are retained in the
[report](benchmarks/stock-mix-transport-overhead.json). The report keeps all raw
timing pairs readable and binds the [complete RPC/catalog archive](benchmarks/stock-mix-transport-overhead-raw.json.gz)
with compressed and uncompressed SHA-256 hashes. The archive preserves the
original report byte-for-byte.

| Workload | Paired median elapsed-time change from PR15 |
|---|---:|
| Sixteen-stage mono routing | +5.34% |
| Sixteen-stage 16-voice routing | +0.80% |
| Mixed stock/analog mono voice | +0.32% |
| Mixed stock/analog 16-voice voice | −0.34% |

The mono-routing increase appears in all five pairs (+5.06% to +5.72%).
The 16-voice routing pairs range from +0.43% to +0.98%. The mixed-voice paired
ranges cross parity, so their small medians do not establish a speed change. This incremental experiment neither
replaces the earlier retained-host transport cost nor establishes an advantage
over VCV Rack. It does not measure compile/adoption latency, memory, UI, audio
devices or live callback deadlines. An initial attempt is also retained (+4.85% mono routing). It predates the
separate pre-rounding correctness fix and is not the final-source measurement.
The report retains both attempts, source manifests and wire records; no timing
attempt was discarded and no optimization search was performed. The benchmark measures existing patches,
not a direct throughput comparison of Stock Mix with Rack's Mixer.
