# Warps portable modulator reference

Prepared 2026-10-24 for Cross Matter (`cross_matter`). This folder builds a genuine C++ oracle from the pinned Warps source and compares the production Rust kernel against it. It does not qualify the OpenPatch host adapter's resampling or hardware fidelity.

## Sources and domain

`source_manifest.json` freezes 25 unmodified source/header/resource files by SHA-256 from `pichenettes/eurorack` revision `f4cfa7697095a28d292bff758beb51deb49b5a86` and its `stmlib` gitlink revision `e3bd7c9cc00e4364166f9905c0509b6ffd0535ec`. All have MIT notices, retained byte-for-byte. `NOTICE.md` records attribution and boundaries.

The original `warps::Modulator::Init(96000)` and `Process` run directly: the input amplifiers and noise gate, the built-in carrier oscillators (sine, triangle, saw, pulse, filtered noise with ducking), the 6x polyphase up/down converters, all six cross-modulation pairs with algorithm/parameter interpolation, the 20-band decimated filter-bank vocoder with envelope followers and limiter, the transition crossfade, and the frequency shifter (`ProcessEasterEgg`, with quadrature transform and quadrature oscillator). Each case starts a fresh process holding a zero-initialized static `Modulator`, as the firmware does; several members are only zeroed by that static storage.

The protocol drives `Parameters` directly (kernel inputs, not panel ADC codes): `easter_egg`, `carrier_shape` (0 external, 1..3 internal), `channel_drive[2]`, `modulation_algorithm`, `modulation_parameter`, `frequency_shift_pot`, `frequency_shift_cv`, `phase_shift`, `note`, then 60 interleaved int16 codec frames per block. Controls are rounded to binary32 before printing, so both probes parse identical values. Blocks are fixed at the firmware's 60 frames at 96 kHz; this keeps the source on its circular-buffer down-sampling branch. Outputs are the int16 main/aux frames the codec would receive.

`cases.json` (schema 1) holds 30 cases, 603,600 frames: silence from construction; full algorithm sweeps up and down; timbre sweeps at each of the six algorithm points; integral algorithm jumps; the cross-modulation/vocoder transition; vocoder formant and release/freeze; drive extremes; clipped inputs; noise-gate tails; each internal carrier across every algorithm (including the dual-oscillator transition and the noise carrier); deep internal FM; pitch-table clipping; the frequency shifter with external and each internal carrier; live mode/carrier switching; a fold case with overdriven inputs; and two cases whose controls are exactly what the native host produces for fixed knob settings (frozen into a native fixture). Test signals are deterministic integer sequences retained in each case's `input.txt`.

The stmlib PRNG is a process global seeded with `0x21`. The oracle has no other consumer, so the noise carrier matches the Rust kernel's per-instance copy. In the firmware, the CV scaler also draws from it every block, so firmware noise sequences differ; noise is not claimed to match hardware.

## Undefined-behaviour boundary

A few source table reads can leave their arrays: the frequency shifter's phase rotation reads `lut_sin[1281]` when the interpolated phase overshoots 1.0, and the wavefolder indexes past `lut_bipolar_fold` when oversampling overshoot pushes `|sum|` above about 2.3. C++ reads neighbouring memory; the Rust kernel clamps to the table and counts a guarded read. `rust_probe.rs` reports the blocks where that happens. `compare.py` excludes such a block and the rest of its case from the exact domain and reports those samples separately; they never count as matched.

## Reproduction

From the repository root:

```sh
python3 scripts/warps_reference/fetch.py
python3 scripts/warps_reference/run.py --validate-only
python3 scripts/warps_reference/run.py
python3 scripts/warps_reference/compare.py --reference native-test-output/warps-reference/attempt-NNN
python3 scripts/warps_reference/make_fixture.py --reference native-test-output/warps-reference/attempt-NNN
```

Requires Python 3, curl, clang++ and rustc. Each run creates a new `native-test-output/warps-reference/attempt-NNN` (or `rust-comparison-NNN`) with the exact harness/input snapshots, retained upstream sources, compiler version/command/logs, per-case input and raw output, a SHA-256 artifact manifest and `result.json`. The compile profile is C++11 `-O2 -DTEST -ffp-contract=off -fno-fast-math`; `TEST` selects stmlib's portable `Clip16` instead of the Cortex-M `ssat` instruction. The Rust probe is built with `rustc -O` from the production files via `#[path]`. Acceptance is bit-identical int16 output. This is the portable-source domain on macOS ARM, not proof of Cortex-M instruction-level identity.

## Results

See `docs/CROSS_MATTER.md` for the frozen attempt and comparison numbers.
