# Stock Octave independent reference — frozen declaration

This source-only declaration was authored without reading the new production
Stock Octave implementation. No generator, validator, compiler, SDK, Core DSP
or tests were executed by its author. Root reviewed the source and the input-only preflight passed all declared counts
and 225,207 conversion-domain checks. An external hash lock freezes all seven
files plus both input streams before the first compiler or DSP execution.
No preparation here constitutes a parity result.

## Actual reference and licensing

The oracle compiles the actual `Octave` class prefix from Fundamental commit
`10dd0160c664770910e5584b7b00498cc48d9ddd`, `src/Octave.cpp`, SHA-256
`d0ee41f787180db1c223dc15018619798797e875d86a73e62e53748b0ea835a6`.
The prefix ends immediately before `struct OctaveButton :`; only the
`plugin.hpp` include is replaced with the genuine Rack SDK include/namespace
preamble. The adapter does not mirror the Octave DSP or bypass equation.

The ignored `native-test-output/stock-octave/reference-research/` directory
supplies the unmodified source, `LICENSE-GPLv3.txt`, `plugin.json`, pinned Rack
`Engine.cpp`, `Module.cpp`, `system.cpp`, and `Rack-LICENSE.md`. The runner
retains these files with every attempt. The copied actual source remains
GPL-3.0-or-later upstream material; it is reference evidence, not application
implementation code. All SDK headers and `libRack.dylib` are verified against
the retained genuine Rack 2.6.6 output-lifecycle attestation and copied into
each SDK attempt.

## Declared matrix and domain

At each of 44.1, 48 and 96 kHz:

| Family | Cases |
|---|---:|
| Pitch/CV widths 0…16, five knobs, three output masks | 4,335 |
| Signed CV and knob half-ties, each adjacent float neighbor | 360 |
| Every finite pitch exponent, both signs, four mantissa lanes, three knobs | 1,530 |
| CV exponents 0…157, both signs, four mantissas, three knobs | 3,792 |
| Explicit arithmetic, raw-state, bypass, topology and persistence histories | 23 |

Manual source cardinality is **30,120 cases, 91,479 snapshots and 61,359 events**.
The input-only validator must confirm these counts before freezing; a mismatch
is a draft preflight failure to retain and resolve before execution, never a
reason to omit a case. Every fresh case includes its constructor and two
single-frame steps. Every explicit event, including no-sample events, emits a
snapshot. No warmup or first-sample observation is discarded.

Finite pitch values span the full binary32 exponent range. The conversion
domain requires both source conversions to be defined: `roundf(parameter)` to
`int`, and `float(float(knob_int) + roundf(CV))` to `int`. The second expression
performs **float addition before integer conversion**, not integer addition.
Rounded CV alone need not fit an integer: explicit valid anchors cancel +2³¹
with a −2³¹ knob and cancel +4,294,967,040 with that knob. Other anchors expose
the lost ±1 term at ±2²⁴. Independent integer/rational helpers check conversion
domains and calculate selected exact anchors without hardware float arithmetic.

The declared host topology uses source1 as the only CV source, with no stacked
CV or feedback; pitch stacking is covered. Input-only validation conservatively
checks every CV control that could be retained in a case against every parameter
used during active processing. This prevents an undefined SDK conversion before
execution. Out-of-domain descriptors are in a separate frozen safety stream;
the SDK never processes them. Production may qualify its chosen saturation/NaN
safety separately, without inventing a Rack result or pruning defined failures.

The actual ARM Engine supplies nearest rounding and FTZ/DAZ. Single cables
preserve finite raw bits; arithmetic may flush subnormals. The declared gate
allows no ULP tolerance or platform-rounding exception. Source copies and bypass
signed-zero/subnormal behavior are checked separately from arithmetic.

## Shared case and event protocol

Each input line has exactly `id`, `sample_rate`, `initial`, and `events`.
Initial state is:

```json
{"sources":[[],[]],"input_mask":0,"output_mask":0,"parameter_bits":0,"data":{}}
```

Each source is a variable array of 0…16 unsigned raw binary32 patterns.
Source0 supplies pitch; source1 supplies CV. Initial mask bit p connects
source output p to module input p under identity `in0` or `in1`. Output mask
bit p connects module output0 to sink input p under identity `out0` or `out1`.
Held controls and immediate parameter are assigned first, actual `dataFromJson`
is invoked second (legacy data wins), then cables connect. Constructor snapshot
is `event=-1`, `op="initial"`, `frame=0`.

| Operation | Fields and semantics |
|---|---|
| `source` | `port` 0/1, `values_bits`; only held source controls change |
| `parameter` | `value_bits`; immediate raw Param assignment, without UI clamping |
| `data_load` | `value`; actual `dataFromJson` with arbitrary JSON |
| `save` | actual `dataToJson`; no state change |
| `reset` | actual Engine reset; parameter resets, runtime/ports remain |
| `bypass` | Boolean `value`; actual Engine bypass transition |
| `connect` | `id,source,port,destination,input`; actual physical cable creation |
| `disconnect` | existing `id`; actual physical cable removal |
| `adopt` | `parameter_bits`; receiving parameter wins, same incarnation/runtime |
| `recreate` | remove incident cables and actual module; fresh default module |
| `step` | exactly one actual Engine frame |

Source/destination roles are `source`, `module`, `sink`; local port indices are
integers. Optional `expect` maps dot paths to exact values; it is validator
metadata and cannot influence either processor.

SDK sources write desired count then all sixteen held/zero-padded slots each
sample. The actual Core counterpart uses existing `poly_constant` modules with
a generation-fenced reference control setter/getter: it changes desired finite
controls only, never current raw ports. Actual source producer adaptation,
retirement, cable transfer and actual Stock Octave processing remain in use.
Production snapshots read the actual getter, not fixture echoes.

SDK adoption assigns the receiving parameter on the same actual module.
Production must perform a real same-incarnation Core graph rebuild/adoption,
preserving `lastOctave` and raw host history. Recreation fences the incarnation,
removes incident cables, and resets parameter/runtime while preserving other
modules and total frame count. No operation secretly processes a sample.

## Exact snapshot protocol

The keys are frozen in `acceptance.json`. Every port is exactly
`{"channels":0..16,"values_bits":[16 unsigned raw bits]}`. Capture both source
outputs, both module inputs, one module output and both sink inputs, including
all inactive/disconnected storage. No blanket inactive-padding assumption is
allowed. Capture actual `parameter_bits`, actual `last_octave`, actual bypass
state, actual held `source_config`, and actual default `dataToJson` as `saved_data`
(`nullptr` normalized to JSON null). Octave has no engine lights.

`input_cables[2]`, `output_cables[1]`, `source_cables[2]`, and `sink_cables[2]`
are physical multiplicities. `output_consumer_mask` is the output-port bitmask,
0 or 1, **not** the sink destination mask. `cable_registry` is sorted by external
identity and contains exact entries:

```json
{"id":"in0","source":"source","port":0,"destination":"module","input":0}
```

SDK registry records are derived from actual Engine registry cable pointers,
with an independently checked external-identity association. Production must
observe its actual graph registry. The validator independently reconstructs
expected topology and checks every row, controls, constructor state, no-DSP raw
invariance, channel-zero runtime arithmetic and bypass runtime freeze.

## Root-only review and execution

Input-only preflight, before compiler/DSP execution:

```sh
python3 scripts/stock_octave_reference/fixtures.py --output INPUTS --safety-output SAFETY_INPUTS
python3 scripts/stock_octave_reference/verify.py --inputs INPUTS --safety-inputs SAFETY_INPUTS --fixtures-only
```

After independent review, Root changes the draft status if appropriate and
retains an external lock with `reviewed_before_first_execution:true`, SHA256
`files` mapping all seven files, `inputs_sha256`, `safety_inputs_sha256`, and
`fixture_validation` equal to the complete successful input-only validator JSON.
Runners refuse an absent, mismatched or unreviewed lock. Neither approves one.

SDK runner arguments: `--inputs INPUTS --safety-inputs SAFETY_INPUTS --lock LOCK`
and optional `--sdk SDK_DIRECTORY`. Paired runner arguments: `--probe BINARY
--build-manifest MANIFEST --sdk-evidence ATTEMPT --lock LOCK`. The production
manifest binds a separately built release probe, toolchain, command, binary
hash and complete Core source closure. Default probe source is
`crates/openpatch-core/examples/stock_octave_host_reference_probe.rs`.

Every append-only attempt retains frozen files, both input streams, commands,
full traces, manifests, compiler/probe diagnostics and failures. The unchanged
validator compares complete traces without field filtering, tolerances or
post-failure exclusions. Root must preserve failed preflights/executions and
obtain a superseding reviewed freeze for any declaration revision.

General Rack import, current-parameter project persistence, UI behavior, native
devices, callback deadlines and hardware accuracy remain separate application
gates. The matrix is sampled finite-domain evidence, not universal Rack parity.
