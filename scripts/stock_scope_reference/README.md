# Stock Scope independent reference — draft source-only declaration

This directory declares a new `stock_scope` qualification gate. The older
OpenPatch `scope` kind and saved patches remain independent. **Do not generate
inputs, compile, run the SDK, run Core DSP, or edit these seven files after a
freeze until Root and Astra review the full declaration and Root completes an
input-only preflight.** The author has performed none of those executions.

## Pinned genuine source and host

The target is Fundamental `src/Scope.cpp` at commit
`10dd0160c664770910e5584b7b00498cc48d9ddd`, independently retrieved
SHA-256 `948a5034696084bd894d256a9fdecd620799e611c689e6ae5b0e19ff0b73c54f`.
The SDK runner takes its actual `Scope` class through the closing `};` before
`Scope::Point DEMO_POINT_BUFFER[BUFFER_SIZE];`. It only replaces
`#include "plugin.hpp"` with genuine `<rack.hpp>` and `using namespace rack;`.
The demo buffer, widget, display, artwork and measurement GUI are excluded.
The actual Rack 2.6.6 ARM Engine owns source, Scope and sink modules, cables,
bypass, reset, sample-rate events and raw ports. The host adapter never copies
Scope's acquisition expression or writes FPCR.

Root must stage and hash the pinned Fundamental GPL license and plugin metadata,
Rack Engine/Module/system sources, Rack license, installed SDK headers and
library, and the prior genuine SDK attestation. The runner checks their exact
hashes against a separately reviewed lock, retains their bytes, the transformed
prefix, compiler command, binary, full traces, diagnostics and every failed
attempt. Source-level qualification does not claim an official distributed
Fundamental plugin binary.

## Matrix and boundary

Rates are 44.1, 48 and 96 kHz. Each of **1,056 cases** begins with an event -1
constructor snapshot and emits a complete snapshot after every declared event.
The **3,186 events** yield **4,242 full snapshots**. Input-only preflight must
establish those exact counts and all 46 separate safety records before the
first SDK execution.

Per rate, 272 cases pair every width `0..16`
independently across X, Y and external trigger using four asymmetric rotations,
then cross each pair with all four output consumer masks `0..3`. Another 80
explicit cases cover configured TIME minimum/default/maximum, exact integer
counter anchors, a neighboring-binary32 ceil crossing, fractional boundaries,
a complete 50-second sweep at each rate,
near-threshold bit neighbors at `-10,0,+10 V`, asymmetric and highest-lane
trigger profiles, first-high arming, sample inclusion, signed-zero extrema,
finite maximum-magnitude extrema, subnormal raw copy and same-bin signed ties,
FTZ-aware XY/TRIG signed subnormal/minimum-normal controls, display-control Params,
lane retirement/regrowth, Initialize while partial and
waiting, live param/rate edits, same-incarnation adoption, bypass without
routes, fresh recreation, physical cable replacement while partial and waiting,
fanout first/middle/last-consumer removal, and six legacy JSON variants. All advances are fixed
nonnegative frame counts declared in `fixtures.py`; none depend on observed
SDK/Core state. The full-sweep case advances `rate * 50 + 1024` frames with
trigger detection enabled so the completed buffer remains waiting. These
three long cases are intentionally included; they are not inferred from a
short partial capture.

Ordinary comparisons use finite raw binary32 source controls and Params. The
TIME range is conservatively restricted to `[-6,8]` at construction, adoption,
and every parameter edit so
`exp2_taylor5(-TIME)/256 * sampleRate`, `ceil` and int conversion stay in the
defined practical band. Fractional/negative raw XY controls deliberately
exercise `bool(nonzero)` processing; upstream's separate GUI
`isLissajous()>0` is outside this gate. Engine FZ makes signed subnormal
XY/TRIG controls false in float-to-bool comparisons while raw Param bits remain;
signed minimum normals are true. Raw subnormal ports, completed bins and
same-bin min/max operand order are explicit anchors. The 46 separate
nonfinite, extreme-finite and out-of-band TIME records are **local Rust safety
inputs only**; selected finite X/Y amplitudes also have ordinary exact anchors. An
undefined SDK-domain value can never be used to excuse an ordinary-domain
failure or to fabricate an SDK result. Unbounded arbitrary graph topology,
stacking beyond the declared cable cases, GUI rendering/measurements and
sample-accurate hardware timing are outside this first gate.

## Commands and full observations

Each JSONL case has `id`, `sample_rate`, `initial`, `events`. Initial contains
three source control arrays of `0..16` unsigned binary32 bits, physical input
mask `0..7`, output consumer mask `0..3`, eight raw Params in source order,
and legacy custom data. Identity cables are `in0..in2` and `out0..out1`.
Events are `advance {frames}`, `source`, `parameter`, `data_load`, `save`,
`reset`, `bypass`, `connect`, `disconnect`, `adopt`, `recreate` and `rate`.
`adopt` sets receiving raw Params on the same SDK instance; the production
counterpart must rebuild/adopt the actual Core incarnation. `rate` is a live
Engine event, not a cross-rate cold graph rebuild. `recreate` removes incident
cables and constructs a fresh Scope while source/sink and frame remain.

Every snapshot includes **all 8,192 completed Points** (`256 × 2 × 16`) and
**all 32 partial Points** (`2 × 16`), both raw min/max bits for each, buffer and
frame indices, X/Y metadata, all 16 exact Schmitt enum values
(`LOW=0,HIGH=1,UNINITIALIZED=2`), all eight Params, both lights, source controls,
actual custom `dataToJson` output, sample rate/time, bypass, all 16 raw slots
of every source/Scope/sink port, cable counts and sorted actual Engine registry.
No hash-only comparison, tolerance, dropped warmup, inferred detector state,
or filtered inactive payload is permitted. Rack's ordinary parameter JSON
clamp/snap/load and OpenPatch saved-project policy are separate application
tests; this gate observes current raw Params and legacy `dataFromJson`.

The production probe must be separately built from real Core sources. It
accepts the same commands and emits actual `stock_scope` Engine/host state.
No sidecar acquisition mirror may supply points, detector states, lights or
raw ports. `run_paired.py` verifies the release binary's complete source
closure and then compares every full JSON object against the retained genuine
SDK trace.

## Root-only preflight and freeze

After Root/Astra source review, Root generates inputs into new paths and runs
input-only validation:

```sh
python3 scripts/stock_scope_reference/fixtures.py --output INPUTS --safety-output SAFETY_INPUTS
python3 scripts/stock_scope_reference/verify.py --inputs INPUTS --safety-inputs SAFETY_INPUTS --fixtures-only
```

Root then creates an **external** declaration lock with
`reviewed_before_first_execution:true`, SHA-256 for all seven declaration files,
both input streams, successful exact preflight, all pinned source hashes, SDK
header/library hashes and prior genuine SDK attestation hash. Runners refuse
an absent or mismatched lock. Attempts are append-only. A failed attempt stays
retained; any changed declaration requires a superseding reviewed freeze.

The SDK runner takes `--inputs --safety-inputs --lock [--sdk]`. The paired
runner takes `--probe --build-manifest --sdk-evidence --lock`, with default
production source path
`crates/openpatch-core/examples/stock_scope_host_reference_probe.rs`.
