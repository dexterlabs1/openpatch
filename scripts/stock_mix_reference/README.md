# Stock Mix independent reference — reviewed declaration

Root reviewed this declaration, generator, hand anchors, validator and adapter
and verified fixture cardinality before freezing their hashes and before either
oracle/production DSP execution.
This directory contains independently authored reference infrastructure, not a
Rust DSP implementation. No new Stock Mix Rust implementation/tests were read
while authoring it. No result or parity claim follows from source preparation.

## Pinned primary material

- Actual `Mixer` class from Fundamental commit
  `10dd0160c664770910e5584b7b00498cc48d9ddd`, `src/Mixer.cpp`, SHA-256
  `7cebae980040e2d6bd0df03456e6952144c9ab2b5ceddf66dc256608ad69d0d6`.
- Genuine Rack SDK/Engine 2.6.6, including `Engine.cpp`, `Module.cpp`, all SDK
  headers and `libRack.dylib`, bound to the existing genuine SDK attestation.
- Rack `system.cpp` at commit `8ea470f343058ade6f75f16d503a1eb330e1700c`, SHA-256
  `cc10706acb8029b2769618a09a44233be449d73c59b6f1d2d2e4ed248db85c18`.
  `Engine::stepBlock` installs nearest rounding and FTZ/DAZ. This affects both
  Mixer arithmetic and genuine Engine stacked-input additions. Single-cable
  transfer copies finite bits, including subnormals, without flushing them.

The runner retains original source and Rack/Fundamental license notices in the
ignored attempt directory. It extracts only the actual Mixer class prefix,
substituting `plugin.hpp` with the SDK include/namespace preamble. It does not
copy widget/artwork code into the application. This reference prefix remains
GPL-licensed upstream material; the original host adapter invokes it directly.

## Matrix and boundaries

At **each** of 44.1, 48 and 96 kHz:

| Family | Dimensions | Cases |
|---|---|---:|
| Isolated width/position | 17 counts × 6 jacks × 4 flag combinations × 3 levels | 1,224 |
| Connection masks | 64 masks × 4 flag combinations × 5 levels | 1,280 |
| Asymmetric pairs | 17² counts × 3 jack pairs × 4 flags × 2 levels | 6,936 |
| Explicit anchors/state/host/precision | Declared ordered histories | 21 |

Declared matrix: **28,383 cases and 86,355 snapshots**, including one constructor
snapshot per case and every declared event. Cardinalities must pass the fixture
validator before oracle compilation. Fresh cases have two one-frame steps:
the first captures real initial-input latency, the second the requested signals.
No warmup sample or snapshot is discarded.

All fresh width-zero inputs are **uncabled**. Connected requested-zero sources
are separate histories: SDK output bookkeeping makes them effective mono zero.
Source controls remain within real production `poly_constant` bounds [-16,16].
The six-input +96 V anchor demonstrates absence of module clipping. All finite
normal/subnormal/underflow boundaries remain in the ordinary exact-bit gate.
NaN/infinite source injection and finite overflow are outside this real-source
matrix, not silently replaced with fake production source writes.

The sparse hand anchors independently cover no mono broadcast, every flag
combination, connected-zero averaging, gain order, fixed f32 addition order,
signed zero, write-before-retirement, disconnected raw tails, partial/nonboolean
Jansson loads, Reset, no-DSP adoption, stacked-input history, and FTZ/DAZ.
All other expected DSP values come only from executing the genuine class/Engine.
Randomize's application policy is an integration gate, not a Rack RNG claim.

## Exact event protocol

Input is JSONL, one case per line:

```json
{"id":"unique","sample_rate":48000,"initial":{"sources":[[],[],[],[],[],[]],"input_mask":0,"output_mask":0,"level_bits":1065353216,"data":{}},"events":[]}
```

`sources` contains six arrays of **unsigned integer f32 bit patterns**. Array
length is requested source count. Source configuration changes held controls
only. During genuine source processing, each output calls `setChannels` then
writes all 16 lanes: held active values and +0 inactive values. Values and trace
fields use memcpy/u32 conversion, never f32→double or double→f32 conversion after
Engine changes floating-point flags.

Construction creates fresh source/Mixer/sink instances, assigns source controls,
level and actual `dataFromJson`, then connects input mask bit p as source p to
Mixer input p (`in0`…`in5`), followed by output mask bits 0/1 to sink inputs 0/1
(`out0`/`out1`). No DSP executes. Emit `event=-1,op="initial",frame=0`.
Constructor actions are a declared initial condition, not hidden transition
events; transition streams explicitly name every subsequent operation.

| op | Fields/meaning |
|---|---|
| `source` | `port` 0..5, `values_bits` variable array; configure held source only |
| `level` | `value_bits`; actual immediate level assignment |
| `data_load` | `value` arbitrary JSON; actual Mixer `dataFromJson` |
| `save` | invoke actual Mixer `dataToJson`, no mutation |
| `reset` | actual Engine reset; level resets, data flags and raw ports retained |
| `bypass` | Boolean `value`; actual Engine bypass operation, unchanged assignment is no edge |
| `connect` | `id`, `source`, `port`, `destination`, `input`; roles source/module/sink |
| `disconnect` | existing `id`; actual Engine cable removal |
| `adopt` | `level_bits`, Boolean `invert`,`average`; receiving controls win, retain same live instance/ports |
| `recreate` | remove cables touching Mixer, delete actual Mixer, create fresh defaults, keep source/sink/frame |
| `step` | exactly one real Engine frame |

Every event emits a snapshot immediately afterward, even if no sample runs.
Optional `expect` is validator metadata and has no effect on either processor.
It maps dot paths to exact expected scalar/array values.

The genuine `adopt` operation assigns actual controls to the existing Mixer.
The production counterpart must perform real Core graph rebuild/adoption and
preserve its incarnation, not substitute a test DSP processor. Recreate requires
an actual incarnation fence. Frame count is total processed frames in the case.

## Snapshot protocol and production counterpart

Each row has exactly the `snapshot_fields` in `acceptance.json`:

- `case,event,op,frame,bypassed,level_bits,average,invert,saved_data`.
- `source_outputs[6],inputs[6],outputs[1],sink_inputs[2]`, each port exactly
  `{channels:0..16,values_bits:[16 unsigned u32 bit patterns]}`.
- `source_config[6]`: actual held source controls, variable-length bit arrays.
- `input_cables[6],output_cables[1],source_cables[6],sink_cables[2]`: actual
  physical cable multiplicities. SDK reads the Engine registry, not fixture
  expectations. Production reads its actual compiled/routed graph context.

`saved_data` is actual `dataToJson`/production serialization with both Boolean
keys. Actual level/flags must be read from the model, not rebuilt expectations.
The declared production APIs are `stock_mix_snapshot`,
`load_stock_mix_legacy_data`, and `stock_mix_legacy_data` with generation-checked
module handles. Use six actual `poly_constant` sources, actual `stock_mix`, and
two actual `mult` sink inputs. Inspect canonical ports passively. A selected
render output may belong to a sink; do not add an unmatched Mixer/source
consumer. Existing logical-frame DSP contracts remain unchanged.

All raw lanes and all fields are compared, including disconnected/inactive
storage, source outputs and sink inputs. The validator independently checks
control semantics and no-DSP port invariance. It never recomputes Mixer DSP.
No field filtering, tolerances or post-failure fixture pruning is permitted.

## Review, freeze and later Root-only execution

The independent author did not execute a compiler, SDK, production DSP, or these scripts.
Root generated and structurally checked inputs before the first reviewed freeze.
That preflight corrected a draft event-count typo (300 snapshots); no fixture,
event or hand anchor was removed or changed. The prior draft and failure are
retained in `native-test-output/stock-mix/declaration-review-001`.
Root may first generate inputs and check declaration cardinalities without DSP:

```sh
python3 scripts/stock_mix_reference/fixtures.py --output native-test-output/stock-mix/inputs.jsonl
python3 scripts/stock_mix_reference/verify.py --inputs native-test-output/stock-mix/inputs.jsonl --fixtures-only
```

After independent review, Root retains an external `declaration-lock.json`:

```json
{"reviewed_before_first_execution":true,"files":{"acceptance.json":"SHA256","fixtures.py":"SHA256","verify.py":"SHA256","sdk_probe.cpp":"SHA256","README.md":"SHA256","run_sdk.py":"SHA256","run_paired.py":"SHA256"},"inputs_sha256":"SHA256"}
```

Change draft status text before computing the reviewed hashes. Both runners
refuse a missing/unreviewed/mismatched lock; neither creates or approves one.
SDK runner accepts `--inputs PATH --lock PATH` (optional explicit `--sdk`).
Each attempt retains sources/licenses, full SDK headers/library, compiler command,
actual executable, inputs, full JSONL trace, validator output and hash manifests.
Failures retain the same artifacts and are never overwritten.

Paired runner accepts `--probe PATH --build-manifest PATH --sdk-evidence PATH
--lock PATH`; optional `--probe-source` defaults to
`crates/openpatch-core/examples/stock_mix_host_reference_probe.rs`.
Its build manifest must record `profile:"release"`, `toolchain`, `build_command`,
`binary_sha256`, and `source_sha256` mapping all Core Rust files, Cargo manifests,
Cargo.lock and actual probe source to hashes. It does not build Rust or infer
build provenance from the current compiler. Use isolated target directories.
It verifies retained genuine SDK artifacts, executes the real production probe,
and applies the unchanged validator to both complete traces. Reports stay under
ignored `native-test-output/stock-mix`; publication is a separate reviewed step.

New/load/preset/randomize/UI/audio-callback/device behavior beyond the declared
histories remains an application integration responsibility. Neither this matrix
nor its source preparation establishes universal Rack parity or performance.
