# Stock Process independent reference — draft source-only declaration

This directory declares a pinned, independent oracle for Fundamental Process. It
does not run DSP until Root independently reviews and freezes all seven files and
both generated input streams. Its author has not compiled, executed the SDK,
executed production DSP, or edited the production implementation. The exact
whole-snapshot comparison is predeclared before either implementation is run.

## Genuine source and Engine

The oracle compiles the actual `SlewFilter` and `Process` class prefix from
Fundamental commit `10dd0160c664770910e5584b7b00498cc48d9ddd`,
`src/Process.cpp`, SHA256
`17e9ae4d61c226db0893bd059d179622ed86a997f9bebcc643a6cb8f371692c1`.
The boundary is immediately before `struct ProcessWidget :`. The only source
transformation replaces `#include "plugin.hpp"` with the genuine Rack SDK
`<rack.hpp>` include and namespace preamble. The widget prefix is excluded,
so no mock plugin instance, copied DSP equation or altered source class is used.
The genuine Rack 2.6.6 ARM Engine schedules the actual source, Process and sink,
owns physical cables, bypass, reset, sample-rate changes and raw port storage.
The adapter observes FPCR in its source callback; it never changes the flags.

Root retains the actual source, GPL license, Rack Engine/Module/system sources,
Rack license, SDK headers and library, the pinned source hashes, transformed
prefix, compiler command, full traces and diagnostics with each append-only
attempt. This is source-level qualification, not an official distributed plugin
binary claim.

## Declared matrix

At 44.1, 48 and 96 kHz, each case starts with a constructor snapshot before
DSP. Every event emits another snapshot, including no-sample topology/control
events. Fresh families take exactly two Engine frames. Source bus 0 is slew CV,
bus 1 signal, bus 2 gate. Outputs 0–5 are SH1, SH2, TH, HT, Slew and Glide.

| Family per rate | Cases |
|---|---:|
| Signal widths 0–16 × independent gate widths 0/1/3/16 × slew widths 0/1/5/16 × output masks 0/1/63 | 816 |
| All 64 output-consumer masks × three asymmetric width profiles | 192 |
| Explicit edges, histories, topology, retained tails, finite slew controls and safety anchors | 140 |

The predeclared total is **3,444 cases, 10,155 events, 13,599 snapshots**.
The input-only validator must establish these exact counts before freezing.
The six-output mask is exhaustive at 0…63. Histories cover every engine lane,
inactive raw payloads, disconnected outputs and retained slots, signal
shrink/zero/regrow freeze, source and cable changes, gate CV and manual button
independence, exact neighbors of 0.1f and 2f, positive raw button fractions,
1 ms cumulative binary32 time at all rates, trigger/release order, bypass,
reset, same-incarnation adoption, recreation, and rate change without a sample.

All ordinary source controls are finite binary32. The active slew samples keep
`-slewPitch+30` within the source-defined normal exponent band `[-126,127]` for
`dsp::exp2_taylor5`; the input-only validator conservatively checks every held
slew CV against every used slew Param, including controls during no-sample
events. `-10` exercises the hard-left infinite slew branch. Raw `+8` slew and
raw fractional button values intentionally exercise immediate Param assignment
beyond UI configuration within the defined processing domain. Separate frozen
nonfinite/extreme-exponent records are Rust safety cases: no SDK result is
invented for them and they cannot be used to discard a defined-domain failure.

## Case and event protocol

Each input JSONL line has exact keys `id`, `sample_rate`, `initial`, `events`.
Initial has `sources` (three arrays of 0–16 unsigned binary32 bits),
`input_mask` (0–7), `output_mask` (0–63), `parameter_bits` (slew, manual gate),
and empty custom `data`. Identity input cables are `in0`…`in2`, output cables
`out0`…`out5`. The adapter sets source controls and immediate raw Params, invokes
actual `dataFromJson`, connects cables, then snapshots `event=-1`,
`op="initial"`, `frame=0`.

Events are `source {port,values_bits}`, `parameter {port,value_bits}`,
`data_load {value}`, `save`, `reset`, `bypass {value}`, `connect
{id,source,port,destination,input}`, `disconnect {id}`, `adopt
{parameter_bits}`, `recreate`, `rate {sample_rate}`, and `step`. `step` is
exactly one actual Engine frame. Adoption only writes receiving raw Params to
the same live SDK module; production must rebuild/adopt its same incarnation.
Recreation removes incident cables and constructs a fresh module without
resetting sources, sinks or frame count. The optional `expect` map is immutable
validator metadata and cannot influence either processor.

Every snapshot has the exact `snapshot_fields` in `acceptance.json`. Each port
is `{"channels":0..16,"values_bits":[16 unsigned raw bits]}`. Capture all
source outputs, module inputs and outputs, sink inputs, all 16 engines (state,
on-time, both samples, both sample filters, hold, slew and glide filters), both
raw Params, gate light, actual `dataToJson` normalized to null, raw source
controls, rate/time, bypass, physical cable multiplicities and a sorted actual
Engine cable registry. `output_consumer_mask` is output-port connectivity,
not the sink destination mask. No inactive lane or first frame is filtered.

The source has no custom JSON data. This gate observes live Param bits and
default `dataToJson`; project `paramsToJson`/`paramsFromJson` persistence,
including Rack clamp/snap behavior, is separate application policy.

## Root-only preflight, lock and execution

First generate input streams and run **input-only** verification:

```sh
python3 scripts/stock_process_reference/fixtures.py --output INPUTS --safety-output SAFETY_INPUTS
python3 scripts/stock_process_reference/verify.py --inputs INPUTS --safety-inputs SAFETY_INPUTS --fixtures-only
```

After independent review, Root stores an external declaration lock with
`reviewed_before_first_execution:true`, SHA256 `files` mapping every one of
the seven files, `inputs_sha256`, `safety_inputs_sha256`, exact successful
`fixture_validation`, complete pinned source hashes, SDK headers/library hashes
and prior genuine SDK attestation hash. Runners refuse absent or mismatched
locks. The SDK runner takes `--inputs`, `--safety-inputs`, `--lock`, optional
`--sdk`; the paired runner takes `--probe`, `--build-manifest`, `--sdk-evidence`,
`--lock`. The production source defaults to
`crates/openpatch-core/examples/stock_process_host_reference_probe.rs`.

Every attempt is append-only and preserves full failures. Revisions after an
attempt need a new superseding reviewed freeze; previous evidence remains.
This finite sampled matrix does not imply universal binary32 parity, Rack patch
import, native-device timing, UI or hardware accuracy.
