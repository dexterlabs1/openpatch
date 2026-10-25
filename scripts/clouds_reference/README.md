# Clouds portable granular processor reference

Prepared 2026-10-24 for Grain Weather (`grain_weather`). This folder builds a genuine C++ oracle from the pinned Clouds source (original firmware only; no Parasites or other alternate firmware) and compares the production Rust kernel against it. It does not qualify the OpenPatch host adapter's resampling, CV mapping or hardware fidelity.

## Sources and domain

`source_manifest.json` freezes 42 unmodified source/header/resource files by SHA-256 from `pichenettes/eurorack` revision `f4cfa7697095a28d292bff758beb51deb49b5a86` and its `stmlib` gitlink revision `e3bd7c9cc00e4364166f9905c0509b6ffd0535ec`. All have MIT notices, retained byte-for-byte. `NOTICE.md` records attribution and boundaries.

The original `clouds::GranularProcessor` runs directly: recording buffers (16-bit and 8-bit mu-law, stereo and mono, with the write crossfade), the feedback path, the 2x sample-rate converters of the low-fidelity setting, all four playback modes (granular sample player with the source RNG, WSOLA time stretcher with its 1-bit correlator, looping delay with tap tempo and frozen loops, and the STFT phase vocoder with the ShyFFT and the frame transformation including glitch algorithms), the diffuser, the looping-delay pitch shifter, the texture filters, the reverb and the dry/wet output stage. Each case starts a fresh process holding a zero-initialized static processor and the firmware's two zeroed static memory blocks of 118,784 and 65,408 bytes, as `clouds.cc` declares them; several members are only zeroed by that static storage.

The protocol drives `Parameters` directly (kernel inputs, not panel ADC codes): playback mode, quality, freeze, trigger, gate, then position, size, pitch, density, texture, dry/wet, stereo spread, feedback and reverb, followed by 32 interleaved int16 codec frames per block. Controls are rounded to binary32 before printing, so both probes parse identical values. Per block the adapter calls `Process` once (the codec interrupt) and then `Prepare` `prepare_calls` times (the firmware main loop; 8 by default, 1 in two cases). Outputs are the int16 frames the codec would receive.

`cases.json` (schema 1) holds 39 cases, 126,920 blocks (4,061,440 frames, 8,122,880 output samples) at 32 kHz; each case states its purpose. Test signals are deterministic integer sequences retained in each case's `input.txt`. Two `host_golden_*` cases use controls exactly as the native host produces them for fixed knobs and are frozen into a native fixture by `make_fixture.py`.

## Random generator

The stmlib PRNG is a process-global LCG whose static initial state is `0x21`. The probe seeds it with the case's `seed` (default `0x21`, one case uses 123456789) before `Init`; the Rust probe seeds its per-instance generator identically. The oracle has no other consumer, so this covers every draw the DSP makes: granular seeding (one draw per sample, whether or not it is used), grain panning, spectral random-bin refresh, phase randomization and glitch selection/patterns. In the firmware nothing else draws from it in the audio path (the CV scaler does not), but its state at any moment depends on how long the module has been running, so hardware sequences are not claimed.

## The window.h variant

At the pinned revision `Window::Start` no longer clears `done_`: two 2023 static-analysis commits (`fbb53ba`, `0e3756f`, merged in `d1d8839`) each removed one of the two duplicated `done_ = false` lines. No WSOLA window ever starts, and the stretch mode outputs only its effects tail. The `window_2019` variant compiles the same pinned tree with `clouds/dsp/window.h` from `2a46873` (hash-pinned under `variants`), which restores the behaviour of every earlier release. `run.py` builds and runs both oracles on every case and records whether they agree. `compare.py` uses `pinned` wherever they agree and `window_2019` only where they differ, and also reports the Rust kernel's divergence from `pinned` for those cases.

## Undefined-behaviour boundary

`Correlator::EvaluateNextCandidate` shifts a 32-bit word right by 32 when the candidate offset is word-aligned, which is undefined in C++. Cortex-M yields zero; the oracle's clang arm64 code computes `(x >> 1) >> (31 - offset)`, also zero (checked by disassembly in both the vector and scalar loops). The Rust kernel implements zero explicitly. Other reads that could leave an object (recording-buffer reads outside their region, look-up-table reads past the end with a nonzero fraction) are guarded: the Rust kernel counts them and `compare.py` excludes the block and the rest of the case from the exact domain. Table reads that land exactly on the last entry, where the source multiplies the out-of-range neighbour by zero, return the last entry without counting.

## Reproduction

From the repository root:

```sh
python3 scripts/clouds_reference/fetch.py
python3 scripts/clouds_reference/run.py --validate-only
python3 scripts/clouds_reference/run.py
python3 scripts/clouds_reference/compare.py --reference native-test-output/clouds-reference/attempt-NNN
python3 scripts/clouds_reference/make_fixture.py --reference native-test-output/clouds-reference/attempt-NNN
```

Requires Python 3, curl, clang++ and rustc. Each run creates a new `native-test-output/clouds-reference/attempt-NNN` (or `rust-comparison-NNN`) with the exact harness/input snapshots, retained upstream sources, compiler version/commands/logs, per-case input and raw outputs of both oracles, a SHA-256 artifact manifest and `result.json`. The compile profile is C++11 `-O2 -DTEST -ffp-contract=off -fno-fast-math`; `TEST` selects stmlib's portable `Clip16` instead of the Cortex-M `ssat` instruction. The Rust probe is built with `rustc -O` from the production files via `#[path]`. Acceptance is bit-identical int16 output. This is the portable-source domain on macOS ARM, not proof of Cortex-M instruction-level identity.

## Results

See `docs/GRAIN_WEATHER.md` for the frozen attempt and comparison numbers.
