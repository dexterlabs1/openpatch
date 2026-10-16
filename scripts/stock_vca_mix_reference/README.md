# Stock VCA Mix independent reference — draft declaration

This source-only declaration was written without inspecting the new candidate
production implementation. Its author ran no fixtures, validators, compilers,
SDK/Core DSP, or tests. Manual counts remain draft until Root's retained
input-only preflight. This preparation is **not parity proof**.

The oracle compiles the actual `VCMixer` class prefix from Fundamental commit
`10dd0160c664770910e5584b7b00498cc48d9ddd`, `src/VCMixer.cpp`, SHA256
`ca4d75289d517d18ec1f758e9bcb9407cd188dc4079616dc1f3f793f546a94c4`.
The extraction ends immediately before `struct VCMixerWidget :`; only the
`plugin.hpp` include becomes genuine `<rack.hpp>` plus the Rack namespace.
No module DSP or bypass formula is reimplemented in the SDK adapter.

Root supplies unmodified source, `LICENSE-GPLv3.txt`, `plugin.json`, pinned Rack
`Engine.cpp`, `Module.cpp`, `system.cpp`, and `Rack-LICENSE.md` under ignored
`native-test-output/stock-vca-mix/reference-research/`. Upstream class code is
GPL-3.0-or-later reference evidence, not production implementation material.
Every source hash plus genuine Rack 2.6.6 library, complete SDK header map and
SDK attestation hash is frozen before execution and retained per attempt.
An ARM64 `mrs fpcr` assertion runs passively inside actual HostSource processing:
FZbit24 must be set and rounding bits23:22 must select nearest. The first
observation is retained on SDK stderr; it is not a paired snapshot field. The
adapter never writes FPCR. Per-row Engine getter sample rate/time is checked
against an independent integer nearest-even encoding of1/rate. JSON-domain
preflight recursively rejects invalid types, nonfinite and overflowing reals.
Compiler flags `-O2 -ffp-contract=off` define strict pinned-source-plus-SDK
behavior; this is not distributed official plugin-binary equivalence.

## Frozen schema and matrix to review

Native parameter order is `[mix,level1,level2,level3,level4]`, input order is
`[mix_cv,in1,in2,in3,in4,cv1,cv2,cv3,cv4]`, output order is
`[mix,out1,out2,out3,out4]`. A single actual source module has nine independent
held outputs; an actual sink has five inputs. Engine frame/cable/control paths
own processing. No direct candidate module port writes are permitted.

At each of 44.1, 48 and 96 kHz the fresh matrix contains:

| Family | Cases per rate |
|---|---:|
| Each of nine port widths 0…16 × four flags × three output masks | 1,836 |
| Twelve asymmetric width vectors × four flags × all 32 consumer masks | 1,536 |
| Five parameters × twelve raw gain boundaries × four flags | 240 |
| Signal/CV/parameter roles × every finite exponent × both signs | 1,530 |
| Five independent signal/CV width-pair roles × 17 × 17 × four flags | 5,780 |
| All 512 input connection masks × four flags | 2,048 |
| Explicit arithmetic, lifecycle, topology, JSON and meter histories | 37 |

Manual source totals: **39,021 cases, 130,743 snapshots, 91,722 events**.
Input-only preflight must confirm these before freeze; mismatches are retained
preflight failures to correct before executing any compiler or DSP. Every fresh
case includes constructor and two samples. Every explicit event emits one row,
including all source/parameter/topology/state changes that process no sample.
Default CV lane zero is nominal10V; later lanes include20V,5V,0V,−5V,
15V and2.5V. Signal strip amplitudes/signs are distinct `[1,−2,4,8]` times
lane variation, with unity lane-zero mix11. Independent exact positive strip
and nominal mix/count anchors prevent a pair of muted implementations from
passing on empty equality. Five pair roles independently sweep every17×17
strip signal/CV width combination and mix-CV/global-signal witness. Nine
isolated connected-zero histories retain physical input_mask511 with the
selected held array empty, compare muted/active witnesses, then restore and
empty it again. All five raw parameter event ports and all twelve raw boundary
receiving controls roundtrip through actual getters/save/adoption.

Four sole-signal histories with consumer masks0,1,30,31 drive the actual global
output width16→1→3→requested0→5→16 with two retained samples per source change.
All CV and other signals are absent. Exact16→1 raw anchors distinguish consumed
strip retirement from disconnected strip padded stores and retained tails;
unconsumed mix storage stays untouched. Four matching fresh-mono cases expose
all four padded copies before any shrink, including consumed mix/strip ports.

A simultaneous four-strip meter history processes1,026 active frames per rate.
Mono signals `[1.25,.3125,.078125,.625]` create padded meter targets
`[1,.25,.0625,.5]`; frame512 independently anchors all four meter bits, first
lamp1 and third lamp0. Four retained source events change those signals to
`[0,.125,1.25,0]`, followed by513 more frames of distinct attack/release states.
Frame1024 anchors third lamp1. All four lamps therefore become active across
actual refreshes, including non-saturated second/fourth lights.

Long meter histories observe frames 511/512/513 and exceed 1,024 active samples
at every rate; they are not repeated for every fresh width case.

The ordinary fidelity domain includes **all finite binary32 raw controls**,
including gains outside UI bounds, signed zeros, subnormals and values producing
internal overflow/Inf/NaN. Resulting internal NaN payloads/signs remain exact
comparison fields. Only nonfinite input/control descriptors belong to the
separate safety stream, which the SDK never executes. There are no integer
conversion exclusions in VCMixer. This bounded matrix is not exhaustive over
all width/control combinations.

Source-defined anchors include mono padding into four physical SIMD lanes,
signed meter cancellation, strip accumulation order, and raw subnormal cable
copies. Independent reconstruction checks every row's controls, serialized
flags, physical cable registry/multiplicities, actual output consumer port mask,
constructor raw state, output widths, meter defaults, reset/adopt/no-sample
history preservation, bypass freeze and divider phase. Paired comparison then
requires exact equality of **all fields and all raw 16-lane storage**. No light,
libm, meter, inactive-tail, first-frame or internally nonfinite tolerance exists.

## Input and event protocol

Each line has exactly `id,sample_rate,initial,events`. Initial state has exactly
`sources` (nine arrays of 0…16 unsigned binary32 words), `input_mask` (nine bits),
`output_mask` (five bits), `parameter_bits` (five finite binary32 words), and
`data` (arbitrary Jansson-domain JSON). Controls and params are assigned first;
actual `dataFromJson` runs second; initial physical cables connect third.
Input mask bit p creates `inP` from source p to module p. Output mask bit p creates
`outP` from module p to sink p. Constructor row is event −1, op `initial`, frame0.

| Operation | Exact fields beyond `op` |
|---|---|
| `step` | none; exactly one actual Engine frame |
| `source` | `port` 0…8, `values_bits`; changes held controls only |
| `parameter` | `port` 0…4, `value_bits`; immediate raw Param assignment |
| `data_load` | `value`; actual `dataFromJson` |
| `save` | none; actual `dataToJson` |
| `reset` | none; actual Engine reset |
| `bypass` | Boolean `value`; actual Engine bypass transition |
| `connect` | `id,source,port,destination,input`; real physical cable |
| `disconnect` | existing `id`; actual physical cable removal |
| `adopt` | five `parameter_bits`, `data`; receiving params and flags win |
| `recreate` | none; removes incident cables and creates fresh module |

Optional `expect` dot paths are validator metadata only. Allowed routes are
source→module and module→sink; multiple input cables and output consumers are
covered. `output_consumer_mask` refers to module output ports, independently of
sink destinations. Every registry is sorted by external cable identity; entries
are `{id,source,port,destination,input}`. Actual Engine registry pointers/counts
are queried and checked against external identities in the SDK adapter.

Each port snapshot is exactly `{channels:0…16,values_bits:[16 unsigned words]}`.
The declaration's `snapshot_fields` additionally bind five actual parameters,
actual flags and `dataToJson`, four meter objects `{v_bits,mode,lambda_bits}`,
actual `divider_clock/divider_division`, four actual `light_bits`, authoritative
Engine `sample_rate` and `sample_time_bits`, held source
controls and physical count arrays for all ports. No raw output echo or fixture
metadata can substitute for actual runtime getters.

SDK sources set desired width then overwrite all sixteen held/zero-padded lanes
on each sample. Connected requested-zero therefore follows actual Rack port
semantics. Candidate Core must use existing `poly_constant` sources and existing
`set_host_reference_source` generation-fenced finite-control setters/getters;
no direct current raw-port writes. Source, transfer, retirement and candidate
processing remain genuine actual Core execution.

The module writes all four padded strip lanes unconditionally, writes mix lanes
only with a consumer, then sets all output counts. Signal widths alone set
max(1,in1…4). Mono broadcast occurs iff actual input count1; short poly CVs do
not broadcast. Channel gain is float `pow(level,2.f)`; CV divides by10 and clamps
only below0, then optional `(cv*cv)*(cv*cv)`. Strip sums follow source order.
Meters use the signed padded sum/5 before peak absolute value, immediate attack,
and ordered binary32 release. Actual divider512 refreshes first on active
frame512. Bypass has no configured routes; actual Engine/Module behavior owns
output lifecycle while meters/lights/divider freeze. Reset restores parameters1
and flagsfalse but preserves ports, meter/light history and divider.

`dataFromJson` changes present keys `chExp`/`mixExp` using
`json_boolean_value`: only JSONtrue sets true; other present types set false;
absent keys and nonobject roots retain. `dataToJson` always emits both actual
Booleans. Adoption uses the same SDK module, applies all receiving raw params
then actual JSON flags; candidate must rebuild/adopt its actual compatible
same-incarnation Core graph with runtime/ports/history retained. Recreation
removes incident cables and creates fresh defaults while sources/sinks/frame
survive. No event implicitly warms up or processes a frame.

## Root review and execution

Root first independently reviews the entire seven-file contract. Only then may
Root run input-only preflight (no compiler or DSP):

```sh
python3 scripts/stock_vca_mix_reference/fixtures.py --output INPUTS --safety-output SAFETY
python3 scripts/stock_vca_mix_reference/verify.py --inputs INPUTS --safety-inputs SAFETY --fixtures-only
```

Before any compiler or DSP, Root retains an external declaration lock with:
`reviewed_before_first_execution:true`; `files` containing SHA256 of exactly all
seven files; `inputs_sha256`; `safety_inputs_sha256`; `fixture_validation` equal
to full successful preflight JSON; `reference_source_sha256` mapping all seven
primary evidence files; `sdk_header_sha256` complete SDK header mapping;
`sdk_library_sha256`; `sdk_attestation_sha256`. The author cannot approve a lock.
Runners refuse absent/unreviewed/mismatched provenance and retain every failure.

SDK runner: `run_sdk.py --inputs INPUTS --safety-inputs SAFETY --lock LOCK`
(optionally `--sdk SDK_DIRECTORY`). Paired runner:
`run_paired.py --probe BINARY --build-manifest MANIFEST --sdk-evidence ATTEMPT --lock LOCK`.
The separately built release production probe manifest binds actual binary,
toolchain, command and complete Core source closure. Default probe source is
`crates/openpatch-core/examples/stock_vca_mix_host_reference_probe.rs`.

Attempts are append-only and isolated, retaining all seven frozen files, both
input streams, source/SDK provenance, commands, diagnostics, complete SDK/Core
traces, binary/build manifests, validator reports and artifact hashes. Failed
attempts and preflights must survive. Any source/contract change requires an
explicit superseding independent freeze with previous failures retained; no
post-failure case edits, field filtering, skipped rows or tolerance widening.
Module four-lamp cadence is covered; host plug-light cadence remains a separate
open host-parity topic. Legacy `vca_mix`, general Rack import, UI/device behavior and hardware/callback
deadlines are separate application requirements.
