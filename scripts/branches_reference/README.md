# Branches raw firmware scan reference

This probe executes the genuine pinned `branches/branches.cc` translation unit,
including its original `Read`, `GateOn`, `GateOff`, input decisions, LED handling
and shared LFSR. It does not substitute a handwritten decision simulator.
`source_manifest.json` identifies the primary GitHub source and SHA-256.

The adapter renames `main`, then redirects its single `while (1)` condition to
a finite scan scheduler through a preprocessor macro. The original file remains
byte-for-byte unchanged. At EOF the scheduler throws a private completion
exception, caught by the host main, avoiding a missing-return path in the renamed
firmware main. The preprocessed translation unit is retained for inspection.
AVR/avrlib stubs capture actual GPIO writes. The original `Init` executes; the
first scan injects the fixture seed after the firmware's fixed seed-1 assignment.

Before each original loop scan, the adapter injects logical input gates (active
low GPIO), both source thresholds and both toggle/latch flags. ADC `ready` stays
false, switches stay unpressed and the timer stays zero. This deliberately
excludes ADC conversion scheduling, button gestures/debounce/long-press/EEPROM,
analog circuitry and CPU/wall-clock timing. The native host's probability-to-ADC,
Schmitt voltage inputs, sample rate, lifecycle and polyphonic routing are separate
contracts. One kernel call represents one complete scan, including idle scans.

The source consumes the low/high 16-bit words of the same pre-scan RNG state for
channels 0/1. Every scan advances the LFSR once, even without edges. Threshold
65535 forbids B before optional toggle XOR; threshold zero permits B. The whole
256-entry source ADC table is compared independently on every fixture. The raw
Rust API accepts any `u16` threshold. Its zero seed remains an absorbing state.

Fixtures cover all ADC entries and paired toggle combinations, simultaneous and
staggered edges, startup high, equality at the random threshold, zero/high seeds,
held gates, mode changes at high/low levels, latch retention after disabling
while already low, and the exact LED tail. Source behavior is retained even when
it differs from a convenient native interpretation. In particular, disabling
latch after the input has already fallen does not clear the existing output.

Run from the repository root, serially with other DSP qualification:

```sh
python3 scripts/branches_reference/run.py --validate-only
python3 scripts/branches_reference/fetch.py
python3 scripts/branches_reference/run.py
```

The runner compiles the original firmware and a UBSan/bounds variant with
`clang++`, compiles the production kernel through the direct-path Rust probe with
`rustc`, and compares every integer record byte for byte. Each execution writes
a fresh `native-test-output/branches-reference/attempt-NNN` directory containing
source, stubs, cases, production kernel, toolchain details, exact commands,
preprocessed source, traces, result and artifact hashes. Failed attempts remain.
The runner never updates fixtures or source hashes to obtain a match.

Trace rows first contain `["adc", index, threshold]` for all 256 table entries.
Scan rows are `[frame, out0A, out0B, out1A, out1B, previous0, previous1,
input0, input1, led0, led1, delay0, delay1, post_scan_rng]`. Outputs are logical
GPIO highs, LED states are the source's numeric 0/off, 1/red/B, 2/green/A.

Done checklist: primary source hash verified; unmodified original scan executed;
all raw Rust records equal; sanitizer traces equal; adaptation bounds disclosed.
Passing this harness establishes that bounded raw scan contract only.


The additional `native_graph_cases.json` fixture has 513 scans (seed17,
ADC127/191, normalled paired gate, toggle1/latch2). Run it separately with
`python3 scripts/branches_reference/run.py --cases scripts/branches_reference/native_graph_cases.json`.
Its unchanged original-source output is frozen in
`crates/openpatch-core/tests/fixtures/event-fork-native-graph.json`; the native
Engine test supplies each gate one frame early to account for the existing
host cable delay, without shifting or rewriting expected source records.
