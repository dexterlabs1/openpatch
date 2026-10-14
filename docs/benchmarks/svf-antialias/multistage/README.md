# Original multistage SVF drive experiment

This is an isolated, repository-owned MIT experiment. The installed application
is unchanged. It oversamples only the SVF's preceding memoryless drive; the
host-rate topology-preserving state-variable filter remains unchanged. This
study does not establish hardware fidelity or production release readiness.

Four 2x FIR stages provide 16x drive evaluation. The host-adjacent stage is a
129-tap normalized Blackman-windowed sinc with cutoff .45 times the host rate.
Higher stages are exact-zero halfbands with lengths 33, 17, 33 and cutoffs 1, 2, 4
in host-rate units. Symmetric polyphase interpolation and selected phase-zero
decimator outputs reduce the estimated coefficient multiplies from 2050 to
350 per host sample. Exactly sixteen chronological scalar nonlinear callbacks
retain the same static curve as the parent scalar study. This arithmetic count
is separate from measured CPU cost.

Interpolation plus decimation adds **76 host samples of latency**, including at
zero drive: approximately 1.72/1.58/.79 ms at 44.1/48/96 kHz. Each voice's cutoff
CV receives an independent 76-sample delay. Low drive uses the parent's
`smoothstep(drive/.1)` blend; drive at or above .1 retains its static saturation
curve. Both latency and low-drive blending change production patch behavior.
Parallel paths and graph feedback have no automatic latency compensation.

The candidate was copied from the pinned parent scalar-16 experiment. Its copied
`ladder.rs` is byte-identical to that parent's 16x/1025-tap source. Ladder and VCA
therefore retain that copied resampler; only the SVF imports the new module.
Their copied settings differ from the pinned production baseline's 8x/513 taps.
The measured workloads contain SVF, rather than ladder/VCA, and these unrelated
copied settings are disclosed instead of presented as production changes.

The [results](results.json) use an independent analytic continuous-source oracle:
32x/64x/128x periodic drive FFTs, strict host-Nyquist bandlimiting and an
independently derived host-rate trapezoidal SVF transfer. Declared delay is
applied without phase fitting. Actual traces retain f32 source quantization;
the continuous-source reference excludes it. Residual RMS is measured relative
to fixed 5 V peak, across all three raw voltage outputs. It is not THD.

| Matrix | Cases | Worst driven off-harmonic residual, dB/5 V | Maximum gated linear magnitude error | Maximum gated driven magnitude error | Minimum eligible baseline improvement |
| --- | ---: | ---: | ---: | ---: | ---: |
| Original public Engine | 144 | -98.6305 | .005063 dB | .007650 dB | 85.912 dB |
| Extended direct-component highband | 198 | -69.1366 | .015169 dB | .015164 dB | 54.616 dB |
| Dense upper-band direct-component | 234 | -67.2108 | Not measured | .008533 dB | 49.983 dB |

These bounded matrices pass the parent's declared alias and magnitude gates:
off-harmonic RMS at most -60 dB/5 V; at least 40 dB improvement where baseline
residual was worse than -40 dB/5 V; linear magnitude within .1 dB through 18 kHz;
driven magnitude within .25 dB for significant retained components through 18
kHz; independent-reference peak convergence better than -80 dB/5 V. Extended
and dense reference convergence is -105.85 and -100.59 dB/5 V respectively.
The dense matrix contains maximum-drive 5 V tones, without linear controls.
Phase, complex legal-bin error and full-waveform error are reported without
inventing additional retrospective acceptance thresholds.

Wanted attenuation above the declared band remains substantial. For the 48 kHz
linear 5 V highband controls near .42/.45/.48/.49 times host rate, fundamental
loss is approximately .27/12.12/74.14/126.16 dB. Removing wanted signal must not be
counted as unrestricted alias improvement. Worst driven total residual through
18 kHz is -71.19/-61.09/-61.11 dB/5 V across the three matrices; this includes
wanted-bin response error as well as off-harmonic residual. Full-Nyquist waveform
error can be large because of transition-band attenuation.

The independent NumPy [frequency screen](source_artifacts/frequency-screen.json)
measures individual protected folds and the complete cascade's desired-passband
images and decimator aliases. The implemented 76-sample design predicts at most
.01912 dB roundtrip loss through 18 kHz at 44.1 kHz; its worst complete image-band
amplitude is -75.44 dB. The provisional 90 dB stopband aim was not reached.
The screen also records an unimplemented 78-sample alternative. These filter
screens do not replace the actual nonlinear spectral measurements.

[Performance](performance.json) is a bounded eight-case offline public-Engine
screen on Apple M1 Max with Rust 1.99.0, thin LTO and one codegen unit: 48/96 kHz,
1/16 voices, drive 0/1, cutoff CV modulation enabled, 128 frames, 256 warmup and
1024 timed blocks per case. All binaries were built before sequential timings,
with no intentionally competing heavy tasks. There was no real-time priority,
core pinning, thermal isolation or audio device measurement. The full 36-case
performance matrix is available to prepare but has not been measured for this
candidate.

| 16 voices, drive 1, CV enabled | 48 kHz p50 / p99 | 96 kHz p50 / p99 |
| --- | ---: | ---: |
| Parent scalar16 | 2.558 / 2.815 ms | 2.557 / 2.782 ms |
| Parent grouped16 | .986 / 1.179 ms | .997 / 1.263 ms |
| Multistage16 | .705 / .874 ms | .702 / 1.679 ms |
| Entire 128-frame nominal budget | 2.667 ms | 1.333 ms |

Multistage median processing is about 3.6x faster than scalar16 for 16 voices and
about 3x faster for mono. The initial 96 kHz 16-voice p99 exceeds the whole-block
budget, so this is **not a production CPU pass**. The initial over-budget samples
clustered at indices 439..469. A [bounded alternating-order diagnostic](tail-repeat.json)
reused unchanged binaries for three repeats per multistage/grouped variant.
Multistage 96 kHz 16-voice drive 1 p99 was .862/.873/.870 ms, with 0/2/0 blocks above
budget out of 1024 and maximum 1.924 ms. These repeats establish neither an OS
cause nor a guarantee; they do not erase the original result. Broader cost,
silence/decay behavior and actual audio-device operation remain unqualified.

Source is frozen under [frozen_src](frozen_src/), with measured-source hashes in
[frozen-manifest.json](frozen-manifest.json). [prepare.py](prepare.py) imports the
parent pinned baseline preparer, verifies the measured SVF and copied ladder,
and writes only a new isolated destination. It supports `engine`, `highband`,
`dense` and `performance`, with `--screen` only for performance. Each prepared
destination receives provenance covering baseline files, parent scripts,
frozen source, original generator/screen and all prepared files.
[source-provenance.json](source-provenance.json) records source-only validation
of all five variants, exact kernel regeneration and invalid-output guards.

From the repository root, prepare and run the extended matrix with a Python
containing NumPy for the oracle:

```sh
python3 docs/benchmarks/svf-antialias/multistage/prepare.py --matrix highband --output native-test-output/svf-multistage-reproduce
cargo run --release --locked --manifest-path native-test-output/svf-multistage-reproduce/probe/Cargo.toml -- native-test-output/svf-multistage-reproduce/traces
python3 docs/benchmarks/svf-antialias/reference.py --traces native-test-output/svf-multistage-reproduce/traces --delay 76 --output native-test-output/svf-multistage-reproduce/reference.json
```

Use `engine` or `dense` with the same spectral invocation and declared delay 76.
For baseline improvement, separately prepare the parent's `--factor 0` using
the same matrix and analyze it with `--delay 0`; pass its reference report using
`--baseline-reference`. Each output directory/report must be new. The source
adds two graph-cable or explicit probe delay samples, separate from 76.

To reproduce the bounded timing screen, prepare all comparison binaries before
running any timing. Use the parent's `--factor 16 --matrix performance --screen`
for scalar and add `--grouped` for grouped. For multistage:

```sh
python3 docs/benchmarks/svf-antialias/multistage/prepare.py --matrix performance --screen --output native-test-output/svf-multistage-cost
cargo build --release --locked --manifest-path native-test-output/svf-multistage-cost/probe/Cargo.toml
native-test-output/svf-multistage-cost/probe/target/release/openpatch-svf-performance-probe multistage16 native-test-output/svf-multistage-cost/performance.json
```

Run the three variants sequentially with other heavy work stopped. Omit
`--screen` to prepare the unmeasured full timing matrix. Performance sources
contain three samples of graph-source delay, irrelevant to the cost gate.

Three original unit tests passed again in a freshly prepared public candidate
with a locked release build. They verify phase-zero impulse latency, DC and linear
phase/gain through 18 kHz, state adoption, and exactly 16 chronological callbacks.
An independent direct high-rate FIR uses runtime equation-generated kernels,
literal zero insertion and full-rate filtering before sample selection. Its
nonlinear stateful output agrees within 2e-12 absolute tolerance. In any prepared
candidate, run:

```sh
cargo test --release --locked --manifest-path native-test-output/svf-multistage-reproduce/core/Cargo.toml --lib svf_resampler::tests::
python3 docs/benchmarks/svf-antialias/multistage/verify_prepare.py
```

The byte-preserved stdlib kernel generator and NumPy screen are copied to each
prepared root, where their relative paths remain valid. Run its
`generate_kernels.py` to regenerate `core/src/svf_resampler_kernels.rs` and
`screen.py` to regenerate its local `frequency-screen.json`.

Before runtime integration, dedicated qualification still needs 16-voice
allocation/deallocation instrumentation including grow/shrink, reset, adoption
and bypass; voice retirement/regrowth and mono/short-poly cutoff-CV alignment;
output-level audio-rate modulation; full filter continuation with nonzero CV;
parameter transitions near drive 0/.1 and maximum; large/nonfinite input and
silence recovery; feedback and parallel-path latency behavior. Arbitrary
amplitudes, two-tone intermodulation and rapid modulation are not established
by these single-tone sweeps. Any eventual runtime patch should stay confined
to SVF drive, its lifecycle/tests and accurate latency documentation, retaining
ladder/VCA and graph semantics. No production implementation is included here.
