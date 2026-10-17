# RackMCP SequencerModule wrapper reference

This additional bounded comparison uses Zachary Scheffler's unchanged RackMCP
2.0.0 `SequencerModule.cpp/.hpp` at commit
`285e84c5ff2d3ecef25890da07f578af6a0c5ca6` (GPL-3.0-or-later), alongside its
unchanged `sequencer.cpp` and `plan.cpp`, linked to genuine Rack SDK **2.6.6**.
The previous [18-case primitive proof](../rackmcp_sequence_reference/README.md)
is frozen and unchanged. This harness adds the wrapper boundary rather than
replacing that proof.

```sh
python3 scripts/rackmcp_module_reference/run.py \
  --source native-test-output/rack-control-import/reference/rackmcp \
  --sdk /Users/dbrewer/Library/Caches/VCVRackMCP/build/ck-sdk/vendor/Rack-SDK \
  --target-dir native-test-output/rackmcp-build \
  --output native-test-output/rack-control-import/module-reference-new
```

Root runs the builds and numerical comparison serially. `--case NAME` can be
repeated for diagnosis. Output must be a new directory. The runner uses no
network, app window, device, project-private assets or installation. Full build
logs and traces stay in the supplied ignored output directory. The manifest,
fixture declarations and driver copies are frozen before sample execution.
Pinned source hashes are verified; upstream source is never patched or trimmed.
SDK headers/library, actual production Rust source, linked libraries and driver
binaries are hashed. The native probe links actual `openpatch-core` and uses
public `Patch::compile`, `Engine` adoption, parameter, reload, producer and tap
APIs; it never calls `Player` or other sequencing internals.
Build subprocesses time out after 180 seconds and each numerical subprocess
after 60 seconds; a timeout retains partial logs/traces and records failure
without restarting. The source driver calls genuine `rack::random::init()` and
assigns stable cable IDs (1000–1017, clock 2000, reset 2001).

The source driver creates the real headless `rack::Context` and
`rack::engine::Engine`. Actual `Engine::addModule/addCable/removeModule` manage
real SDK modules, ports and the ParamHandle registry. Actual
`Engine::moduleFromJson` calls the unchanged `dataFromJson` under the engine
lock. Loads start with actual `Module::toJson()` and replace only `data`, retaining
the SDK's complete plugin/model/version/ID/parameter envelope; the real plugin
and model are registered in the SDK plugin registry. All 18 outputs have actual
SDK sink cables. Two parameters of an original
minimal real SDK `Module` are configured with genuine `ParamQuantity` objects.
Their ranges/defaults come directly from the native public descriptors for
`stock_vca.level` and `rack264_vcf.frequency`; source IDs are explicitly
`42:0` and `42:1`, while native destination modules are `gain` and `filter`.
Binding order deliberately differs from source lane order. These are SDK target
quantity/write checks, **not** Fundamental target DSP equivalence. The sole
link-only definition is the unused plugin UI asset pointer `pluginInstance`;
all source widget code is compiled unchanged, and no widget is constructed.
No detector, port, quantity, DSP, engine or registry implementation is stubbed.
Load diagnostics in private stderr retain actual data, module IDs and genuine
ParamHandle registry state before/after each load. They do not repair SDK handle
updates or enter the compared trace.

Every frame compares all 18 channel counts, all 16 canonical voltage bits per
output, both actual destination parameter bits and destination presence. Hidden
inactive lanes are canonical positive zero. No tolerance or startup exemption
is allowed. Nine deterministic 48 kHz cases cover polyphony, internal pulses,
selected nonzero pattern's first advance, Schmitt startup and thresholds,
clock disconnect/reconnect, reset input/button, stopped 32-sample divider,
raw clamp, normalized mapping, manual override/cache, explicit equal-data load
without transport restart, changed data/channel counts, missing initial target,
and written-target disappearance/return followed by an explicit load. The bare
same-source-ID recreation case intentionally exposes lifecycle differences; it
must not be removed merely because native behavior differs. Both currently
supported native capabilities have `snap=false`; snapped target equivalence is
outside this comparison. Production's snap arithmetic has separate unit checks.

Scheduling is explicitly bounded. The driver calls the actual source module's
`process` directly and reads real destinations after that call. It does not
claim to duplicate Rack's engine module execution order. Desired input producer
sample *n* is delivered to the source receiver at *n+1*, matching the declared
native one-sample cable schedule; a newly connected route starts at positive
zero. Connection state itself changes before that frame. Real SDK cables set
connected port state, while the driver installs this declared receive frame.
Native input paths really traverse compiled producer/cable processing, and
native taps read the actual outputs before host audio gain/crossfade. Native
node-order destination DSP visibility remains a separate declared host check.
No status-atomics, UI, audio/device, Bridge, Listen or whole-patch claim follows.

## Separate source ownership audit

`ownership.cpp`/`ownership.py` independently audit two unchanged source modules,
IDs 100 (A) and 200 (B), writing real SDK target 42:0 with raw values .25/.75.
The target is a genuine SDK Module/ParamQuantity, range 0–1/default 1. There is
no Cargo build or native comparison in this audit:

```sh
python3 scripts/rackmcp_module_reference/ownership.py \
  --source native-test-output/rack-control-import/reference/rackmcp \
  --sdk /Users/dbrewer/Library/Caches/VCVRackMCP/build/ck-sdk/vendor/Rack-SDK \
  --output native-test-output/rack-control-import/ownership-reference-new
```

Six original stages expect A acquisition, B takeover, incumbent B's equal-load
self-unbind, displaced A's load/reacquisition, B takeover again, and deletion of
B without automatically restoring A. These expectations follow the pinned
source's `overwrite=true` calls and genuine SDK 2.6.6's observed self-unbind;
they are verified by the audit, rather than presented as completed evidence
before execution. Each stage records immediate ownership and both live saved
handle identities, then processes A followed by B for 32 frames and records
effective target bits/ownership again. The deleted B pointer is cleared before
later observations and is never dereferenced after destruction. This measures
192 deterministic frames, without claiming Rack scheduler equivalence.
Declaration, source/SDK/adapter hashes and executable identity are frozen before
execution; build/case timeouts and private log retention match the wrapper
runner. The existing nine-case declaration/comparator/fixtures are independent.

## Recorded comparison and native binding correction

The recorded nine-case 48 kHz comparison remains **failed** for target writes
following source same-target data reloads. SDK registry diagnostics confirm the
pinned module's second update self-unbinds its handles; the third rebinds them.
All 18 output counts and canonical samples remain exact. The native host keeps
binding idempotent so explicit pattern edits and undo preserve automation.

`audit_binding_correction.py` verifies every word against explicit correction
windows and source/native expected values, while requiring equality everywhere
else. It requires the raw report to remain failed and reports that separately.
It does not ignore a target column or claim whole-module equivalence.

```sh
python3 scripts/rackmcp_module_reference/audit_binding_correction.py \
  --reference-dir /absolute/path/to/frozen-comparison \
  --output /absolute/path/to/new-correction-audit.json
```

See the [source-derived host contract](../../docs/RACKMCP_SEQUENCE_CONTRACT.md#genuine-wrapper-finding-and-deliberate-host-correction).

## Separate transport and status comparison

`transport.py` builds `transport.cpp` against the same unchanged source TUs and
SDK flags, and `transport.rs` against public native Patch/Engine APIs. The three
original fixtures contain **no automation lanes or targets**; the existing SDK
binding discrepancy/correction does not enter this comparison.

```sh
python3 scripts/rackmcp_module_reference/transport.py \
  --source native-test-output/rack-control-import/reference/rackmcp \
  --sdk /Users/dbrewer/Library/Caches/VCVRackMCP/build/ck-sdk/vendor/Rack-SDK \
  --target-dir native-test-output/rackmcp-build \
  --output native-test-output/rack-control-import/transport-reference-new
```

Root runs this command serially; a new output directory is required. The default
three cases contain **17,584 frames**: internal now/wrap selection and stop/resume;
stopped fractional seek, reset/selection/seek coalescing and source consumption
order; and external edges, stalls, thresholds, stop/seek/reset and reconnect.
Reset input/button and changes to BPM/swing also cross the genuine module wrapper.
`--case NAME` selects a diagnostic subset. Fixtures, source/SDK/Core manifests,
drivers, toolchains and both binaries are frozen before either driver executes.
Build/case timeouts and private logs follow the existing runner.

Every frame compares all 306 output words (18 channel counts plus sixteen
canonical f32 words each), together with every actual `statusJson` field **both
after requests/before process and after process**. BPM/swing/published bar use
exact IEEE words; published beat is recovered exactly from the source's bar by
multiplication by four. Desired pattern name comes from actual `dataToJson`.
Names, pending request overlay, running, clock source, step, loop and counts
compare exactly, including the source's 64-sample publication lag. No tolerance,
startup exemption or ignored field is allowed. A successful run prints one JSON
summary with `passed:true`, `cases:3`, `frames:17584` and the report path; this is an
expected result. Recorded `transport-reference-001` passes all three cases with
17,584 complete frames; the current Core production manifest matches its frozen
96 entries. See the [transport evidence](../../docs/benchmarks/rackmcp-sequence-transport.json).

The source still uses a genuine Context/Engine, module/plugin/model registry,
SDK load envelope and real output/reset/clock cables. Its declared receive
boundary matches the existing driver. Native parameter/topology rebuilds use
ordinary equal-data state adoption and live parameter inheritance; source data
is loaded only once. Graph adoption/rate-change behavior has separate Core
contracts because Rack has no corresponding graph replacement operation here.
This bounded transport/status comparison makes no UI/device, Bridge, Listen or
whole-patch claim and does not alter the frozen failed target-write comparison.


The original `native.rs` target-wrapper driver explicitly marks each incoming
packet's running control as an assignment during effective-parameter inheritance.
Running joined that public capability set with transport integration; omitting
this assignment would silently inherit the previous run value despite the source
packet setting it. The initial failed requalification (`module-reference-006`)
is retained. The corrected driver (`module-reference-007`) retains every original
fixture/output comparison and only the declared SDK binding differences; no
production DSP or expected source trace was changed to obtain the audit pass.
