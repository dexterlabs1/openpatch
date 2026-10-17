# Tides 2018 portable kernel reference foundation

Prepared 2026-10-17. **Root compiled and exercised the reference on macOS ARM; the original clock-start failure is retained as an explicit negative boundary. The revised finite input domain awaits root execution.** This folder prepares a genuine source oracle; it does not itself compare a Rust backend or establish hardware fidelity.

## Sources and domain

`source_manifest.json` freezes 17 unmodified source/header/resource files by SHA-256 from `pichenettes/eurorack` revision `f4cfa7697095a28d292bff758beb51deb49b5a86` and its `stmlib` gitlink revision `e3bd7c9cc00e4364166f9905c0509b6ffd0535ec`. All fetched files have MIT license notices, retained byte-for-byte. `NOTICE.md` records attribution and boundaries. No DSP implementation or generated table is replaced with a handwritten equivalent.

The original `PolySlopeGenerator::Init`, `Render`, `Reset` and `RampExtractor::Init/Process/Reset` run directly. `Reset` resets the generator's smoothing filter only; it is **not** a reinitialization of oscillator phase. Each case starts a fresh process containing zero-initialized static source instances, as the firmware does. Parameter interpolation, source ratio tables, wave tables, template dispatch, BLEP/shaper/filter arithmetic and extractor prediction are unchanged. Gate edges use upstream `ExtractGateFlags`.

The independent input protocol carries kernel-normalized frequency (`frequency_hz / sample_rate`), scalar block controls and explicit gate/clock pulse patterns. These are kernel inputs, not panel ADC codes. Modes use upstream numeric enums: ramp AD=0, LOOPING=1, AR=2; output GATES=0, AMPLITUDE=1, SLOPE_PHASE=2, FREQUENCY=3; range CONTROL=0, AUDIO=1. The `slope` input is the source Render `pw` argument. A patched clock drives the actual extractor; its phase-ramp buffer reaches Render only when trigger is unpatched, mirroring the firmware's priority rule. Ratio and denominator are direct source `Ratio` fields; their declared pairs are pinned in cases.

Schema 2 appends `clock_start` (nonnegative frame offset) and `initial_clock_high` (0/1 previous sampled level) to the protocol header after ratio/denominator. Clock level is LOW before `clock_start`; afterwards it is HIGH when `(frame - clock_start) % clock_period < clock_high`. `initial_clock_high` initializes only the caller's previous gate flags; it does not modify extractor state. All 34 positive cases explicitly start from previous LOW, with clocked cases starting at frame one. Thus a complete elapsed input sample precedes every first rising edge after construction or the declared extractor reset. Input validation independently rejects a positive case whose construction/reset coincides with a rising edge. This is an explicitly selected finite source-kernel domain, not a claim that firmware masks its first edge.

The 28-case positive JSON matrix covers every ramp/output mode combination, clocked looping and audio-range AR, trigger-over-ramp priority, extreme/stepped slope/shape/smoothness/shift, separate filter/extractor resets, and half/double clock ratios. A separate negative case preserves the complete original `clocked_gates` controls/pulse timing with frame-zero rising edge and previous LOW. The runner executes it independently and requires a nonzero exit, exactly `nonfinite frequency` on stderr and no accepted trace records; other failures do not count as a pass. Every positive sample retains four output float32 bit patterns, gate/clock flags, normalized effective frequency bits and extracted ramp/phase bits (null without clock). Internal private generator state is not fabricated or exposed. Gate/clock periods are expressed in processing frames.

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

A successful reference run means only that these source traces were produced with finite samples and the expected record counts, and the known negative boundary was rejected for its exact expected reason. Every subprocess is bounded to 30 seconds; in particular a zero-period AUDIO PLL can loop indefinitely in the source's phase compensation, so the negative case deliberately uses the existing CONTROL failure. It does not establish source correctness against hardware or Rust equivalence. Root must review/freeze this case/harness manifest, retain the first successful traces, define numerical/event acceptance independently and compare the actual production path. Do not update reference outputs merely to agree with a candidate.

## Native startup boundary

The pinned firmware [`tides.cc`](https://github.com/pichenettes/eurorack/blob/f4cfa7697095a28d292bff758beb51deb49b5a86/tides2/tides.cc#L169-L191) resets the extractor on the next clocked block after an unpatched block. It forwards that block's existing clock flags directly. The generator reset on output-mode change affects its smoothing filter only. No explicit first-edge mask appears at this call site, so a native mask is a documented safety adaptation, not firmware equivalence.

The raw Rust `RampExtractor` preserves the source arithmetic and zero-initialized static construction domain. `Reset` intentionally retains `max_train_phase`, which upstream does not assign there. Its caller must provide at least one elapsed sample before any first rising edge after construction/reset. The native module adapter primes that first sampled clock level: it suppresses only that sample's RISING flag, retains HIGH and records the actual previous level for subsequent edge extraction. All later flags reach the raw tracker unchanged. This prevents a zero-duration period, including reconnect/reset while clock is high, without clamping or replacing nonfinite source results. Source comparison applies to the declared finite input domain; startup safety is a separately tested native contract.

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

Schema 2 isolates this boundary with explicit clock timing and retains the
frame-zero case as an independently asserted negative. The harness now rejects
nonfinite frequency immediately after extractor processing, before forwarding it
to Render. Neither extractor nor generator source is patched. Original failed
input/trace evidence remains at `native-test-output/tides2-reference/attempt-002`.


## Qualified Rust port (2026-10-17)

`attempt-003` produced the first complete 28 positive traces (124,800 frames)
and rejected the original zero-duration CONTROL boundary. Rust comparison
`rust-comparison-002` matched all observed fields exactly. Review then found
that centered Shift makes the Amplitude outputs legitimately silent. Those
unchanged traces remain frozen; six additional AD/Loop/AR × Control/Audio cases
scan Shift through 0, 0.25, 0.5, 0.6, 0.75 and 1 to exercise nonzero gain paths.
`attempt-004` retains all 34 cases (129,840 frames) plus the negative boundary.
`rust-comparison-005` matches every output, extracted ramp, frequency and flag
bit exactly, with source/toolchain/executable hashes and mismatch reports.

Reproduce the comparison against the retained C++ evidence (no oracle rebuild):

```sh
python3 scripts/tides2_reference/compare.py --reference native-test-output/tides2-reference/attempt-004
```

The probe imports the actual production Rust files, not a parallel model. The
runner snapshots them before compiling, verifies every retained C++ artifact
hash, and records all candidate artifacts. Failed comparison001 (Rust submodule
path resolution) remains retained. Correcting explicit module paths did not
alter numerical behavior. Native six-frame scheduling, CV mapping, startup,
Engine adoption/bypass/realtime behavior, shared panel and MCP/history are
separate checks documented in [Tidal Functions](../../docs/TIDAL_FUNCTIONS.md).
