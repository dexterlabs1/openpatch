# Stock Mix: before-rounding underflow supplement

Source-only independent declaration. No supplement SDK or production execution
has occurred during authorship. Root must review these inputs and hand anchors,
verify input-only cardinality, and retain an external hash lock **before** any
supplementary SDK/Core processing. The original reference directory and its
freeze remain unchanged. This supplements the first passing bounded gate; it
does not retrospectively expand what that gate tested.

## Primary rule and hand derivation

The [Arm Compiler armasm User Guide, Version 6.6](https://documentation-service.arm.com/static/63eb50c09567172d4e2aa777)
documents A64 FPCR.FZ selection and single-precision result flushing before
rounding in its flush-to-zero sections (PDF pages 173–174). Signed zero retains
the result sign. This is the conventional Arm FZ environment of the retained
SDK executable, not a claim about every architecture or alternative FP mode.
The actual genuine Engine result remains the execution oracle.

Let M = 2^-126 (minimum normal) and u = 2^-149 (one subnormal spacing).
`0x3f7fffff` is 1 − 2^-24. Thus M times that level is M − u/2:
ties-to-even without FZ would round to M, but before-rounding FZ gives +0.
Negating the exact product gives −0. Conversely, (M + u) times that level is
M + u/2 − u*2^-24, which rounds down to M but is not tiny before rounding.
This control detects an incorrect rule that flushes every rounded M result.

For averaging, `0x00ffffff` is 2M − u. Division by two gives M − u/2,
which must flush before rounding. Two connected same-sign 1 V inputs then
multiply that zero gain by +2 or −2. Levels `0x01000000` and `0x01000001`
instead divide exactly to M and M + u; their outputs have magnitudes 2M and
2M + 2u. The immediately lower level `0x00fffffe` divides to M − u and also
flushes. With Average off, all these gains remain normal and give exact
nonzero doubled outputs.

`fixtures.py` records only eight fixed magnitude rows. Source sign, Invert,
Average and rate expand those rows; there is no general arithmetic reference
implementation. For multiplication with Average enabled, the first source is
doubled and a second connected zero source supplies denominator two. This
preserves the exact boundary product. Division cases use two same-sign unit
sources. All operands are normal finite values supported by actual production
sources, and both output consumers are connected.

## Cardinality and complete comparison

At each of 44.1, 48 and 96 kHz, two families each contain four magnitude rows
times two source signs times two Invert settings times two Average settings:
32 multiplication cases and 32 division cases. Total: **192 cases, 384 step
events, 576 snapshots and 1,728 scalar hand anchors**. Every case retains its
constructor snapshot and two single-frame samples; no warmup is discarded.
First-sample anchors check latency and zero sign. Settled anchors check output
count, the computed quartet and both sink lane-zero values.

The same original event/snapshot protocol and existing genuine SDK/Core
executables apply. The validator imports the frozen original shape, control,
topology and trace checks, but does not invoke its large-matrix cardinality
check. Both traces must separately satisfy all hand anchors and counts before
whole-object equality compares every field and every raw lane, including
source ports, inactive storage and sinks. No filtering or tolerance is allowed.

Root may generate and validate inputs without running DSP:

```sh
python3 scripts/stock_mix_rounding_reference/fixtures.py --output INPUTS.jsonl
python3 scripts/stock_mix_rounding_reference/verify.py --inputs INPUTS.jsonl --fixtures-only
```

The generator refuses to overwrite existing inputs. After independent review,
retain an external lock with this structure (real hashes replace placeholders):

```json
{"reviewed_before_first_execution":true,"files":{"acceptance.json":"SHA256","fixtures.py":"SHA256","verify.py":"SHA256","README.md":"SHA256"},"dependencies":{"scripts/stock_mix_reference/acceptance.json":"SHA256","scripts/stock_mix_reference/verify.py":"SHA256"},"inputs_sha256":"SHA256"}
```

Retain the reused SDK and production executable hashes and their original
build/source attestations before processing the supplement. Retain every
attempt's lock, inputs, binaries, complete traces and validation result.
After Root executes the probes, validate with:

```sh
python3 scripts/stock_mix_rounding_reference/verify.py --inputs INPUTS.jsonl --lock LOCK.json --sdk-trace SDK.jsonl --paired CORE.jsonl
```

A failed production result stays retained. Any production fix requires an
unchanged rerun of both this supplement and the original complete gate. Prior
passed reports, performance and bundle artifacts describe the prior binary;
retain them as superseded evidence rather than attributing them to the fix.
No anchor or input may be changed after observing a difference without a new,
explicitly reviewed superseding declaration preserving the earlier failure.
