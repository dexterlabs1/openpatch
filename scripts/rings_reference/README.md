# Rings resonator reference

Prepared 2026-10-24 for String Field (`string_field`). This folder builds a genuine C++ oracle from the pinned Rings source and compares the production Rust kernel against it. It does not qualify the OpenPatch host adapter's resampling, CV mapping or hardware fidelity.

## Sources and domain

`source_manifest.json` freezes 31 unmodified source/header/resource files by SHA-256 from `pichenettes/eurorack` revision `f4cfa7697095a28d292bff758beb51deb49b5a86` and its `stmlib` gitlink revision `e3bd7c9cc00e4364166f9905c0509b6ffd0535ec`, plus four reference-only files that are fetched and checked but not compiled. All have MIT notices, retained byte-for-byte. `NOTICE.md` records attribution and boundaries.

The original `rings::Strummer` and `rings::Part` run directly: note filtering and voice allocation (including the three-voice ping pattern reached by a long press), the excitation filter, the internal exciter (pulse, filtered noise burst or envelope trigger, by model), all six resonator models (modal, sympathetic strings, inharmonic string, FM voice, quantized sympathetic strings, string and reverb) with their resonator, string, FM voice, follower and reverb code, the onset detector and the output limiter. The easter-egg string synthesizer is not part of the domain.

The probe reproduces the DSP part of `FillBuffer` in `rings/rings.cc`: the input is cleared while the exciter input is normalized (as `CvScaler::DetectAudioNormalization` does), passes the input noise gate, then `Strummer::Process` (always with the input block, so the onset detector runs every block) and `Part::Process`, and the out/aux buffers are converted to int16 with `Clip16`. Each case starts a fresh process holding zero-initialized static `Part`, `Strummer` and 32768-word reverb memory, as `rings.cc` declares them; several members (`Part::step_counter_`, `FMVoice::previous_sample_`, `String::src_phase_`, `Plucker::comb_filter_gain_`, the reverb low-pass states) are only zeroed by that static storage and survive later re-initialization.

The protocol drives `PerformanceState` and `Patch` directly (kernel inputs, not panel ADC codes): polyphony, model, strum, the three normalization flags, chord, then tonic, note, fm, structure, brightness, damping and position, followed by 24 int16 exciter samples (the codec's right channel, the only one Rings reads). Controls are rounded to binary32 before printing, so both probes parse identical values. Polyphony is applied with `set_polyphony` only when it changes and the model with `set_model` every block, matching `Ui::Init` and the buttons (polyphony before model). Outputs are the int16 odd/even frames the codec would receive, followed by the strum flag as the Strummer left it.

`cases.json` (schema 1) holds 71 cases, 163,800 blocks (3,931,200 frames, 7,862,400 output samples) at 48 kHz; each case states its purpose. They cover every model at polyphony 1–4, the internal and external exciter, external strums (including retriggers inside the inhibit window), strums derived from V/oct changes and from input onsets, the silent all-normalized state, all eleven chords at each polyphony, structure 0.9995 and damping 1.0, the crossfade to infinite decay, both dispersion signs, notes below the delay line, FM feedback of both signs, and polyphony and model changes mid-case. Test signals are deterministic integer sequences retained in each case's `input.txt`. Two `host_golden_*` cases use controls exactly as the native host produces them for fixed knobs and are frozen into a native fixture by `make_fixture.py`.

## Random generator

The stmlib PRNG is a process-global LCG whose static initial state is `0x21`. The probe seeds it with the case's `seed` (default `0x21`, one case uses 123456789) before `Init`; the Rust kernel seeds its per-instance generator identically. Its only consumers in the domain are the plucker's noise burst and the dispersion noise of the inharmonic string models. In the firmware, `CvScaler::DetectNormalization` also draws one word per block for the normalization probe, so hardware noise sequences are not claimed.

## Source revision and boundaries

No commit after the 2019 rename touches `rings/dsp` except `43b9c9f` (2020), which initializes `Resonator::previous_position_` in `Init`; the pinned revision includes it. No source bug of the `Window::Start` kind was found: no deliberate deviation is needed. Defined but notable behaviour reproduced as is: `rings/dsp/dsp.h` defines `BRYAN_CHORDS`, so the quantized model uses Bryan Noll's chord table; `lfo_frequencies` has seven initializers for eight strings, so the eighth string's LFO runs at zero frequency; a modal bank with an odd number of audible modes also processes the next (clamped) filter.

Undefined behaviour boundary: out-of-range float-to-integer conversions (`SineFm` phase offsets, reverb `Compress`, output `Clip16` arguments) saturate on Cortex-M, on the oracle's clang arm64 code and in Rust. Table reads that would leave an object (`SemitonesToRatio` beyond ±128 semitones, `Interpolate` past the end with a nonzero fraction) are guarded: the Rust kernel counts them and `compare.py` excludes the block and the rest of the case from the exact domain. `Interpolate(lut_4_decades, 1.0)` lands exactly on the last entry, where the source multiplies the out-of-range neighbour by zero; it returns the last entry without counting. Structure is limited to 0.9995, as the firmware's CV scaler constrains it, because the stiffness table read leaves the table at 1.0.

## Reproduction

From the repository root:

```sh
python3 scripts/rings_reference/fetch.py
python3 scripts/rings_reference/run.py --validate-only
python3 scripts/rings_reference/run.py
python3 scripts/rings_reference/compare.py --reference native-test-output/rings-reference/attempt-NNN
python3 scripts/rings_reference/make_fixture.py --reference native-test-output/rings-reference/attempt-NNN
```

Requires Python 3, curl, clang++ and rustc. Each run creates a new `native-test-output/rings-reference/attempt-NNN` (or `rust-comparison-NNN`) with the exact harness/input snapshots, retained upstream sources, compiler version/commands/logs, per-case input and raw outputs, a SHA-256 artifact manifest and `result.json`. The compile profile is C++11 `-O2 -DTEST -ffp-contract=off -fno-fast-math`; `TEST` selects stmlib's portable `Clip16` and `Sqrt` instead of the Cortex-M `ssat`/`vsqrt` instructions. The Rust probe is built with `rustc -O` from the production files via `#[path]`. Acceptance is bit-identical int16 output and an identical strum flag. This is the portable-source domain on macOS ARM, not proof of Cortex-M instruction-level identity.

## Results

See `docs/STRING_FIELD.md` for the frozen attempt and comparison numbers.
