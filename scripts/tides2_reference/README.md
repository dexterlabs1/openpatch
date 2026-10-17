# Tides 2018 portable kernel reference foundation

Prepared 2026-10-17. **Root compiled and exercised the reference on macOS ARM; a clock-start boundary remains failed.** This folder prepares a genuine source oracle; it does not implement a new OpenPatch module, compare a Rust backend, or establish hardware fidelity.

## Sources and domain

`source_manifest.json` freezes 17 unmodified source/header/resource files by SHA-256 from `pichenettes/eurorack` revision `f4cfa7697095a28d292bff758beb51deb49b5a86` and its `stmlib` gitlink revision `e3bd7c9cc00e4364166f9905c0509b6ffd0535ec`. All fetched files have MIT license notices, retained byte-for-byte. `NOTICE.md` records attribution and boundaries. No DSP implementation or generated table is replaced with a handwritten equivalent.

The original `PolySlopeGenerator::Init`, `Render`, `Reset` and `RampExtractor::Init/Process/Reset` run directly. `Reset` resets the generator's smoothing filter only; it is **not** a reinitialization of oscillator phase. Each case starts a fresh process containing zero-initialized static source instances, as the firmware does. Parameter interpolation, source ratio tables, wave tables, template dispatch, BLEP/shaper/filter arithmetic and extractor prediction are unchanged. Gate edges use upstream `ExtractGateFlags`.

The independent input protocol carries kernel-normalized frequency (`frequency_hz / sample_rate`), scalar block controls and explicit gate/clock pulse patterns. These are kernel inputs, not panel ADC codes. Modes use upstream numeric enums: ramp AD=0, LOOPING=1, AR=2; output GATES=0, AMPLITUDE=1, SLOPE_PHASE=2, FREQUENCY=3; range CONTROL=0, AUDIO=1. The `slope` input is the source Render `pw` argument. A patched clock drives the actual extractor; its phase-ramp buffer reaches Render only when trigger is unpatched, mirroring the firmware's priority rule. Ratio and denominator are direct source `Ratio` fields; their declared pairs are pinned in cases.

The 28-case JSON matrix covers every ramp/output mode combination, clocked looping and audio-range AR, trigger-over-ramp priority, extreme/stepped slope/shape/smoothness/shift, separate filter/extractor resets, and half/double clock ratios. Every sample retains four output float32 bit patterns, gate/clock flags, normalized effective frequency bits and extracted ramp/phase bits (null without clock). Internal private generator state is not fabricated or exposed. Gate/clock periods are expressed in processing frames.

The primary portable domain is 48 kHz/block 6, matching the upstream desktop test's declared rate/block size. This is **not the hardware processing cadence**. Firmware declares 62,500 Hz/block 8 and halves effective processing for slope-phase/frequency modes; two additional cases exercise 62,500/block 8 amplitude and 31,250/block 4 frequency. They test the kernel at those rates, not the entire firmware's missing-sample interpolation, DAC duplication, ADC smoothing, transposition quantizer, output calibration, drivers or interrupt scheduling.

Pinned provenance context: [firmware `tides.cc`](https://github.com/pichenettes/eurorack/blob/f4cfa7697095a28d292bff758beb51deb49b5a86/tides2/tides.cc), [`io_buffer.h`](https://github.com/pichenettes/eurorack/blob/f4cfa7697095a28d292bff758beb51deb49b5a86/tides2/io_buffer.h), [desktop test](https://github.com/pichenettes/eurorack/blob/f4cfa7697095a28d292bff758beb51deb49b5a86/tides2/test/tides_test.cc). Those context files are not compiled into the oracle. The desktop test has a GPL notice; its test implementation is not copied here.

## Reproduction

From repository root, fetch/verify source into ignored cache without compiling:

```sh
python3 scripts/tides2_reference/fetch.py
python3 scripts/tides2_reference/run.py --validate-only
```

Root owns reference execution. When root is ready to compile and execute:

```sh
python3 scripts/tides2_reference/run.py
```

Requires existing Python 3, curl and clang++; no installs, Rack SDK, audio device, network access beyond source retrieval, or Cargo build. Use `--cache PATH` or `--compiler PATH` if needed. Fetch checks existing file bytes and refetches a mismatch against the pinned hash; sources never come from moving branches.

Each execution creates a new `native-test-output/tides2-reference/attempt-NNN` with exact runner/harness/input snapshots, retained upstream sources, compiler version/command/stdout/stderr, per-case input protocol/raw output, SHA-256 artifact manifest and result report. Failures retain their stage/logs. Compile profile is C++11 `-O2 -DTEST -ffp-contract=off -fno-fast-math`. `TEST` selects upstream's own desktop branches: it removes the embedded RAM-section attribute and uses upstream C++ saturation helpers/`sqrtf` instead of Cortex-M assembly. The adapter does not supply substitute helpers. This is the portable-source domain, not proof of ARM instruction-level identity. No global floating-point mode changes are made. Cross-platform/compiler bit identity has not been established.

A successful reference run means only that these source traces were produced with finite samples and the expected record counts. It does not establish source correctness against hardware or Rust equivalence. Before writing a backend, root should review/freeze this case/harness manifest, retain the first successful traces, define numerical/event acceptance independently and compare the actual production path. Do not update reference outputs merely to agree with a candidate.

## First root execution (2026-10-17)

`attempt-001` retained a compiler-environment failure (missing libc++ headers).
The runner now resolves the macOS SDK through `xcrun`, as the existing reference
runners do, and records the explicit sysroot/header flags in the compile command.
`attempt-002` compiled the unmodified source successfully, produced the twelve
unclocked mode-combination traces, then correctly refused the first clocked case
with `nonfinite frequency`. That case presents a rising clock edge on frame zero,
when the extractor's current pulse duration is still zero. The raw source and
failed evidence are preserved; no source patch, accepted nonfinite output, Rust
comparison or completed module is claimed.

Next isolate this startup boundary with explicit clock timing in the input
protocol while retaining the frame-zero case. Define the native initialization
and safety policy before adapting the kernel; do not drop the failing case or
change the source oracle merely to obtain a green comparison. Evidence lives at
`native-test-output/tides2-reference/attempt-002`.
