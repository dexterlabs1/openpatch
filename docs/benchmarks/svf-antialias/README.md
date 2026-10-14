# SVF drive antialiasing study

These are **isolated experiments, not the installed OpenPatch filter**. They
investigate the nonlinear-drive aliasing defect measured in the
[baseline](../svf-spectral/README.md). They do not establish VCV Rack equivalence,
hardware fidelity, or release readiness.

The baseline is pinned to `147c8300b7dab2ab58569ee186d0e8fe4a58f1ad`.
The experiments retain its host-rate topology-preserving state-variable filter
and oversample only the preceding memoryless drive. The existing Blackman-windowed
sinc resampler is tested at 4×, 8× and 16×, using 257/513/1025 taps respectively.
Each interpolation and decimation filter contributes 32 host samples of group
delay: 64 total, in addition to graph cable delays. Cutoff CV receives the same
64-sample delay. This is additional module latency, including at zero drive.

The current drive has a discontinuity at zero. Experimental drive values below
0.1 use `x + smoothstep(drive / 0.1) * (legacy_curve(x, drive) - x)`;
values at or above 0.1 retain the previous static curve. This changes patch sound
below 0.1. No hardware model is implied.

## Acceptance criteria declared before selection

- Off-harmonic residual at most −60 dB relative to 5 V peak on every output;
  at least 40 dB improvement where the baseline was worse than −40 dB.
- Linear magnitude within 0.1 dB through 18 kHz; driven fundamental/retained
  harmonics within 0.25 dB where the reference component exceeds −60 dB/5 V peak.
- Independent reference convergence better than −80 dB/5 V peak. Report phase,
  full-waveform residual and residual on legal harmonic bins as well as magnitude.
- Separately report wanted-signal attenuation above 18 kHz; removing wanted
  signal must not be counted as an unqualified aliasing improvement.
- Measure actual-engine processing cost before selecting a production design.
  Audio-device operation, parameter transitions, voice-state handling, allocation
  behavior and broader modulation still require qualification.

The spectral thresholds are engineering gates for this study, not a claim that
these are universal audibility limits or that passing establishes complete DSP
accuracy. In particular, off-harmonic energy omits aliases landing on wanted
harmonics; the independent reference tests those bins too.

## Measurement design

The original matrix has 144 actual-engine cases: three sample rates
(44.1/48/96 kHz), four coherent frequencies, three amplitudes and four drive
settings. It records raw low/band/high voltages before device clipping.

The extended matrix has 198 direct-component cases. An analytic sine, quantized
to f32 and delayed by two explicit samples, feeds the actual `VoiceTools` filter
implementation. This permits input frequencies beyond the engine oscillator's
20 kHz parameter limit without changing that oscillator. It includes approximately
18/20 kHz and 0.42/0.45/0.48/0.49 times the host rate, amplitudes 0.1/5 V,
and drive 0/0.1/1. Every case runs eight 8192-sample periods and retains the final
two. This component probe does not exercise graph scheduling.

The independent oracle generates continuous sine/drive spectra at 32×/64×/128×,
removes frequencies at and above host Nyquist, then applies an independently
derived host-rate trapezoidal SVF transfer. It does not reuse the candidate FIR
or filter recurrence. The declared delay is applied without phase fitting.
Reference convergence and baseline linear calibration are checked. Source f32
quantization remains in traces but is excluded from the continuous-source oracle.

## Initial and extended results

Worst driven off-harmonic residual RMS, dB relative to 5 V peak, across all three outputs
and the entire host spectrum; more negative is less unwanted energy. Here the
oracle sums residual outside the wanted odd-harmonic bins, including unintended
DC. Legal-bin residual is reported separately. The original baseline analyzer
excludes DC and all harmonic bins, so its lower-bound definition is slightly
different; the headline values agree to the shown precision in this matrix:

| Candidate | Original 144-case matrix | Extended 198-case matrix |
| --- | ---: | ---: |
| Baseline host-rate drive | −9.96 | −8.99 |
| 4× | −41.10 | Not run |
| 8× | −74.33 | −39.13 |
| 16× | −101.44 | −69.14 |

**The wider matrix changes the decision.** The 8× version passes the original
matrix but fails the extended one, including a 48 kHz / 18 kHz / 5 V / maximum
drive case. Its lowpass output has about −42.59 dB/5 V of off-harmonic energy.
Passing a few frequencies would have hidden this defect.

The 16× version passes these two matrices' alias and magnitude gates. Its minimum
improvement over baseline cases worse than −40 dB is 90.62 dB in the original
matrix and 54.58 dB in the extended matrix. The extended maximum gated linear
magnitude error is 0.001509 dB and driven error 0.001508 dB; the maximum gated
phase error is under 0.000080 degrees. The extended reference's worst peak
convergence error is −105.85 dB/5 V. These are bounded measurements, not general
proofs for arbitrary signals.

All prototypes substantially attenuate wanted frequencies in their FIR transition
band. At about 0.42/0.45/0.48/0.49 times the host rate, the 8× linear fundamental
loses approximately 0.25/12.11/74.13/126.15 dB. The 16× filter has essentially the
same transition. Full-Nyquist waveform error therefore remains large even when
through-18-kHz magnitude and alias checks pass. At 44.1 kHz the nearest coherent
18 kHz probe is 18,001.76 Hz; it is outside the strict gate and is separately
reported (approximately 0.00568 dB linear attenuation).

## Denser upper-band sweep

A further **234 cases per baseline/16× variant** sweep coherent bins at intervals
of 16/8192 of the host rate from approximately 0.35 to 0.49, plus selected rational
frequencies. All use 5 V peak and maximum drive at the three sample rates. These
were generated, built and analyzed using the public reproducer below.

The 16× candidate's worst off-harmonic residual is −67.19 dB/5 V; minimum
improvement over eligible baseline cases is 50.02 dB. All measured driven
magnitude gates pass (maximum 0.001681 dB), with reference convergence better than
−100.58 dB/5 V. The worst total residual through 18 kHz is −67.46 dB/5 V.
There are no linear controls in this drive-1-only matrix; they are explicitly
unmeasured here and covered in the original/extended matrices.

[results.json](results.json) contains per-matrix summaries, failures, worst cases,
reference-report hashes and every trace hash. [source-provenance.json](source-provenance.json)
identifies prepared source. These sweeps still do not cover arbitrary amplitudes,
two-tone intermodulation, rapid modulation, feedback or all possible frequencies.

## Processing cost on Apple M1 Max

[performance-summary.json](performance-summary.json) records all 36 paired
workloads per candidate, p50/p95/p99/max, source hashes and compiler metadata.
Four release builds completed before sequential runs on 2026-10-14, using
Rust 1.99.0, thin LTO and one codegen unit. Total measured blocks: 589,824.

Selected workload: 16 voices, maximum drive, cutoff modulation enabled;
128-frame processing times in milliseconds:

| Candidate | 48 kHz p50 | 48 kHz p99 | 96 kHz p50 | 96 kHz p99 |
| --- | ---: | ---: | ---: | ---: |
| Baseline | 0.087 | 0.096 | 0.082 | 0.086 |
| 4× | 0.756 | 0.814 | 0.755 | 0.801 |
| 8× | 1.360 | 1.433 | 1.339 | 1.401 |
| 16× | 2.569 | 2.827 | 2.494 | 2.603 |
| Whole block budget | 2.667 | 2.667 | 1.333 | 1.333 |

The current 8× prototype exceeds the 96 kHz budget; 16× exceeds it at both
48 and 96 kHz at p99. These are rejected as production implementations for the
measured workload. The baseline has substantially worse aliasing, so its lower
cost is not an equal-quality performance comparison. The largest observed 16×
48 kHz/modulated block was 11.10 ms; wall-clock tails include OS scheduling and
must not be attributed solely to arithmetic cost.

The public performance harness preserves the measured timed workload. Its only
subsequent change reserves a new report file before timing instead of overwriting
at the end. Both measured and reproducer hashes are retained in the summary.
No application DSP or installed bundle changed in this study.

## Grouped 16× bounded comparison

The grouped prototype processes active quartets through the existing
`Oversampler::process_four`, with scalar processing for one to three remaining
voices. Each lane retains its FIR addition order, ring heads and separate
64-host-sample cutoff-CV delay. Gain, blend and `tanh(gain)` are compiled once;
division and drive expression order are preserved. Zero blend uses an identity
closure while advancing the FIR histories. State adoption retains the newly
compiled constants, and reset/voice retirement clear the histories.

[grouped-performance.json](grouped-performance.json) records a **bounded eight-case
screen per variant**, with 256 warmup and 1024 measured 128-frame blocks per case:
48/96 kHz × 1/16 voices × drive 0/1, with cutoff modulation enabled throughout.
The scalar and grouped release binaries were built before sequential timing on
the same Apple M1 Max. Total measured blocks: 16,384. This has fewer cases and
shorter runs than the previous 36-case/4096-block study.

Selected 16-voice block times, milliseconds:

| Rate | Drive | Scalar p50 | Grouped p50 | p50 speedup | Scalar p99 | Grouped p99 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 48 kHz | 0 | 2.540 | 0.796 | 3.19× | 2.786 | 0.883 |
| 48 kHz | 1 | 2.565 | 0.981 | 2.61× | 2.766 | 1.143 |
| 96 kHz | 0 | 2.490 | 0.774 | 3.22× | 2.694 | 0.903 |
| 96 kHz | 1 | 2.528 | 0.978 | 2.58× | 2.696 | 1.147 |

Maximum-drive mono p50 was essentially unchanged: 0.168/0.172 ms scalar versus
0.168/0.168 ms grouped at 48/96 kHz, respectively. Zero-drive mono improved by
approximately 1.13×. The measured 16-voice maximum-drive speedup is about 2.6×;
it does not establish production readiness. At 96 kHz, two of the grouped
maximum-drive case's 1024 blocks exceeded the nominal 1.333 ms whole-block budget;
the maximum was 1.966 ms. OS scheduling still affects tails, and this screen does
not test audio callbacks or headroom for a full patch.

Six release unit tests passed both in the isolated candidate and in a freshly
prepared public candidate using `--locked`. They compare all three output ports
with `to_bits()` against an unchanged scalar 16× source copy, across counts 1–16,
44.1/48/96 kHz, and drive 0/1e−12/0.1/1. They also cover changing counts, differing
ring heads, CV broadcast/modulation and exact 64-frame delay, reset, retirement,
state adoption and drive changes, f64 drive outputs, retained FIR histories and
subsequent scalar continuation. Signed-zero checks found no bit mismatch. These
are bounded equivalence checks, not qualification for arbitrary signals.

## Remaining work

The subsequent [four-stage resampler study](multistage/README.md) changes the
filter kernels and adds 76 samples of latency. It passes all three measured
spectral matrices and reduces both mono and polyphonic median cost. Its initial
96 kHz p99 and bounded repeat results remain explicit; this is still an
experimental candidate, not an installed fix.

The production decision remains open. Preserve the 16× candidate as a spectral
reference while measuring and reducing its cost. The grouped experiment improves
the bounded screen while retaining scalar bit equality in the tests above.
It still needs the full timing matrix, parameter-continuity and allocation/state
adoption checks, broader amplitude/intermodulation/modulation/feedback tests, and
eventual audio-device qualification before any production decision.

The experimental factor changes affect the copied `ladder.rs` shared resampler.
Only SVF is tested here. **Do not copy that constants change into production**:
SVF needs its own configuration so ladder/VCA behavior and cost remain controlled.

## Reproduce without modifying the application

Requirements: a Git checkout containing the pinned baseline commit, Rust/Cargo,
and Python with NumPy. Start from the repository root. Output directories and
reference reports must be new; generation refuses overwrites. No command opens
a window or audio device.

For example, prepare and analyze the 16× extended matrix:

```sh
python3 docs/benchmarks/svf-antialias/prepare.py --factor 16 --matrix highband --output native-test-output/svf-study-16x
cargo run --release --locked --manifest-path native-test-output/svf-study-16x/probe/Cargo.toml -- native-test-output/svf-study-16x/traces
python3 docs/benchmarks/svf-antialias/reference.py --traces native-test-output/svf-study-16x/traces --delay 64 --output native-test-output/svf-study-16x/reference.json
```

The oracle prints `max_other_bin_residual_rms_db5` and
`alias_floor_failed_port_cases` for the headline alias threshold. To reproduce the
improvement gate, prepare/analyze the matching baseline first, then add
`--baseline-reference path/to/baseline-reference.json` to the candidate oracle
command. Its JSON records the minimum improvement for baseline port/cases above
−40 dB, the eligible count and failures below 40 dB. Unlike the original
baseline-only analyzer, this oracle supports a dense matrix without linear
control cases; those controls are checked in the other matrices.

Use factor `0` and reference delay `0` for baseline, or factors `4`/`8` and delay
`64` for those prototypes. Matrices `engine`, `highband`, and `dense` use the
same spectral invocation. The dense matrix sweeps coherent bins from approximately
0.35 to 0.49 times host rate at 5 V/maximum drive, plus selected rational-frequency
cases. Each generated `provenance.json` identifies every input and prepared file
by SHA-256; [source-provenance.json](source-provenance.json) records all combinations.

For timing, prepare each factor with matrix `performance`, then **build every
binary before running any of them**. Run variants sequentially with other heavy
work stopped. The local probe uses thin LTO and one codegen unit:

```sh
python3 docs/benchmarks/svf-antialias/prepare.py --factor 16 --matrix performance --output native-test-output/svf-cost-16x
cargo build --release --locked --manifest-path native-test-output/svf-cost-16x/probe/Cargo.toml
native-test-output/svf-cost-16x/probe/target/release/openpatch-svf-performance-probe 16x native-test-output/svf-cost-16x/performance.json
```

The binary takes a variant label and a new JSON output path. It retains raw
per-block times, complete patch descriptions and summary statistics. The workload
uses the public engine with 1 or 16 active voices, 128-frame blocks, three rates,
three drive settings, and optional 13 Hz cutoff modulation. Each case has 512
warmup and 4096 measured blocks. Taps verify voice activity after cable startup;
only public block processing and timer/black-box overhead are timed.

To reproduce the bounded scalar/grouped screen, prepare both candidates and
build both binaries before timing either one:

```sh
python3 docs/benchmarks/svf-antialias/prepare.py --factor 16 --matrix performance --screen --output native-test-output/svf-screen-scalar
python3 docs/benchmarks/svf-antialias/prepare.py --factor 16 --matrix performance --grouped --screen --output native-test-output/svf-screen-grouped
cargo build --release --locked --manifest-path native-test-output/svf-screen-scalar/probe/Cargo.toml
cargo build --release --locked --manifest-path native-test-output/svf-screen-grouped/probe/Cargo.toml
native-test-output/svf-screen-scalar/probe/target/release/openpatch-svf-performance-probe scalar16x native-test-output/svf-screen-scalar/performance.json
native-test-output/svf-screen-grouped/probe/target/release/openpatch-svf-performance-probe grouped16x native-test-output/svf-screen-grouped/performance.json
```

Run these sequentially with other heavy work stopped. `--screen` requires matrix
`performance`; `--grouped` requires factor `16`. Each output directory and JSON
report must be new. The generated screen main is byte-identical to the measured
screen harness. Public grouped DSP/test source is byte-identical to the frozen
candidate, and prepared provenance includes the unchanged scalar test oracle and
core/probe lockfiles. [source-provenance.json](source-provenance.json) records the
16 original combinations, four grouped matrices and two performance screens.

Run the six equivalence checks separately from timing:

```sh
cargo test --release --locked --lib --manifest-path native-test-output/svf-screen-grouped/core/Cargo.toml voice_tools::grouped_tests::
```

Use this selected test group: copied ladder tests include assertions for the
original 8× configuration and are not a valid full-suite gate for these 16× SVF
experiments. `--grouped` also works with `engine`, `highband` and `dense`; their
spectral probe/oracle invocation and 64-sample delay are unchanged.

This is an offline wall-clock screen. OS scheduling affects tails; there is no
realtime priority, core pinning, audio device, or callback scheduling test. All
voices share one oscillator, and each case is measured once. Nominal block-budget
ratios must not be described as measured audio-device underruns. A single filter
exceeding the whole block budget is enough to reject that workload for realtime
use; falling below it does not establish sufficient headroom for a full patch.
