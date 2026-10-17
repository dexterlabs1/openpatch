# Existing ARM plugin binary feasibility

This deliberately separate executable tests an **existing installed** Bogaudio
2.6.47 `plugin.dylib` with the genuine Rack SDK 2.6.6 runtime. It compiles only
OpenPatch's adapter, never Bogaudio source. It is not linked into OpenPatch,
included in the application bundle, or an available playback/import backend.

On macOS ARM, supply explicit local directories:

```sh
python3 scripts/rack_binary_host/run.py \
  --sdk /path/to/Rack-SDK \
  --plugin /path/to/Bogaudio \
  --output native-test-output/rack-control-import/binary-host
```

Each execution creates an append-only attempt directory containing input
hashes, compiler/dependency/signature diagnostics, commands, child outputs,
serialized module state and a machine-readable result. The local installed
binary, manifest, SDK headers and runtime are hashed before/after execution.
The probe checks `dladdr(init)` and all loaded `libRack.dylib` images, requiring
the selected plugin and exactly the selected runtime. This identifies the bytes
tested; it does not authenticate their official distribution/build provenance.

## Tested behavior

The child loads the binary with `dlopen`, calls its actual exported `init`,
registers the plugin's actual models/manifest and instantiates `Bogaudio-Offset`.
One genuine Rack Engine owns signal routing through source → plugin → sink
cables. Its ordinary connection bookkeeping handles polyphony; no manual
connection-field writes or substitute OFFSET implementation are used.

The first child sets `.25/.25` parameters and both nondefault custom flags,
serializes via Rack's module API, then checks six settled observations. A fresh
second child restores that JSON using Rack's module loader and repeats them:

| Observation | Expected voltage |
|---|---|
| Saved state, 4 V mono input | 4.0625 V |
| Saved state, 16 channels with inputs −5…10 V | −1.5625…7.8125 V in 0.625 V steps |
| Saved state, 100 V input, limiter disabled | 64.0625 V |
| Change operation order, 4 V input | 5 V |
| Enable limiter, 100 V input | 12 V |
| Change scale to .5, 1 V input | 5 V |

Full saved JSON and all observations must match between children. Persistent
parameter/custom-state restoration is tested; transient DSP history continuation
and active-audio checkpointing are not. Each observation allows 2,048 engine
frames for modulation cadence and cable propagation, so this is not a timing or
latency measurement.

Expected behavior comes from the exact official [`v.2.6.47` source tag](https://github.com/bogaudio/BogaudioModules/tree/656eaae458e045602dc974bae82e15a11e104958),
particularly [`Offset.cpp`](https://github.com/bogaudio/BogaudioModules/blob/656eaae458e045602dc974bae82e15a11e104958/src/Offset.cpp)
and its [`base module lifecycle`](https://github.com/bogaudio/BogaudioModules/blob/656eaae458e045602dc974bae82e15a11e104958/src/module.cpp).
Those sources explain the anchors; they are not compiled into the probe.

## Ownership and limits

Context is set on the sole processing thread before Engine construction. All
stepping and state operations are quiescent/serial. Engine cleanup destroys
registered modules/cables before Context disappears. The small plugin registry,
models and library handle intentionally remain loaded until child exit; unload,
reload and repeated-session memory behavior are unqualified. Internal Engine
construction/sample-rate APIs make this a version-pinned companion experiment,
not proof of a stable public hosting ABI.

Private temporary Rack user/settings paths are set before plugin initialization.
No real user settings are loaded/saved; no plugin scanning, fallback processing,
ModuleWidget, scene, window, patch manager, MIDI or audio driver is started.
The chosen model's source-backed constructor/process/restore path works without
those UI objects. Other models may require them. The plugin directory remains
read-only from the adapter. **Child process isolation is not a filesystem or
security sandbox against arbitrary plugin code.**

The installed plugin requests `/tmp/Rack2/libRack.dylib`. A child-scoped
`DYLD_LIBRARY_PATH` selects the genuine SDK runtime without patching the binary
or creating global symlinks. Inherited `DYLD_*` overrides are removed.

This clears only the bounded actual-binary load/process/persistent-restore
feasibility gate. OpenPatch still needs a versioned companion protocol, bounded
audio/control transfer, crash/stall containment, explicit hosted identities and
state ownership, mixed-graph/feedback scheduling, UI policy and licensing/
redistribution review. No performance, general plugin, GUI, device or hardware
accuracy claim follows from these observations.

## Rust/shared-memory transport foundation

The next implementation is `companion.cpp` plus the macOS ARM
`openpatch::binary_host` Rust module. It is still separate from application
playback. Run its actual-binary gate with the complete `saved-module.json` from
the first probe:

```sh
python3 scripts/rack_binary_host/run_transport.py \
  --sdk /path/to/Rack-SDK \
  --plugin /path/to/Bogaudio \
  --state /path/to/first-attempt/saved-module.json \
  --output native-test-output/rack-control-import/binary-host
```

The runner builds the optimized C++ companion and debug Rust control driver
serially, hashes their sources/SDK/plugin before and after execution, and retains
toolchains, exact Cargo artifact selection, loaded-image receipts, binary hashes,
child logs, saved state, WAV and result/artifact manifests. It never launches an
audio device or installed app. Signal injection/termination affects only children
spawned by this gate.

Protocol v1 (`wire.h`) has four owned slots, 128 frames at 48 kHz, three inputs,
one output, up to 16 channels per frame and 64 ordered parameter events per block.
The fixed plan's input connection mask preserves disconnected-port semantics.
Acquire/release ownership publishes complete payloads across processes; all
concurrent state words are aligned lock-free u32 atomics. Incarnation, document
generation, revision and sequence fence every block. Bounds, finite values,
reserved fields, event order and unused payloads are validated before execution.
Each region supplies exactly one parent endpoint and is never reused after fault.

Endpoint submit/receive functions have no locks, loops waiting on a peer,
allocation, process operations or formatted errors. A deadline failure latches
silence; a later result cannot resume execution or replay stale CV. Mapping
creation/destruction, process supervision and JSON stay on control threads. The
companion detects supervising-parent death, cleans up its engine and exits.

The actual gate covers an impulse, settled mono/poly16, within-block width
changes at the pinned source modulation cadence, mono offset CV/poly16 bipolar
scale CV, sample-offset and simultaneous ordered events, quiescent state capture
and fresh-process restoration. It checks 48,000 samples of a native Rust
oscillator through the existing binary against the expected gain/routing and
audible RMS, then writes a WAV. SIGSTOP, late results, SIGKILL, malformed requests
and supervising-parent death exercise real failure paths. Instrumented parent
endpoint calls perform zero heap allocations/deallocations. Eight focused Rust
boundary tests also exercise stale fences, malformed output, full queues and
retired endpoints.

The measured Rack source-input → observed-sink path adds **one frame** in this
harness. The WAV explicitly prepends the **proposed** 128-frame transport delay;
its control driver waits outside endpoint calls. This does **not** demonstrate
asynchronous callback scheduling, device latency, deadline sustainability or a
feedback contract. Those measurements, shared ProjectDocument/UI/MCP/history
integration and application packaging remain open in
[the application-host milestone](../../docs/BINARY_HOST_V1.md).

## Process ownership and authored state preparation

The same gate now also exercises the production Rust process owner: explicit
pinned launch, loaded-image receipt, endpoint movement independently of the
control-thread supervisor, acknowledged capture/fresh restore and refusal of
wrong fingerprints, incomplete capture or an unrelated endpoint even with the
same public fences. Failed ownership/capture paths reap the child and remove
private staging. These are control-thread lifecycle checks, not device deadline
measurements.

The portable project model additionally retains hosted identity, ports and full
immutable snapshots, with shared revision-fenced parameter/option edits,
history and save/reopen checks through Session, stdio MCP and the native headless
App. Hosted nodes remain explicitly inactive. This preparation does not connect
the companion to the application audio graph; see the milestone for remaining
execution, live-state and active UI work.

## Production mixed-graph runner gate

Use the same command with `--gate mixed-graph` to build `binary_host_graph`.
It constructs authored native/external graphs, uses the production Core boundary
and hosted runner, and paces callbacks outside tracked processing. Three fresh
real-plugin children compare every sample of a one-frame native impulse,
declared delayed feedback and simultaneous ordered parameter events. Retained
artifacts include full callback timing/heap records, sample traces, non-silent
WAVs, actual identity receipts and acknowledged full state captures.

The provisional profile uses 48 kHz, Q128, delay384 and actual callbacks no larger
than128. The measured native impulse path is387 frames, its additional-mixer
feedback topology cycles every388 frames, and events at input657 change native
output1042. All tracked processing allocates/deallocates zero heap memory. These
are short debug correctness tests; sustained load and audio-device qualification
remain open. Wire v1 currently refuses stacked hosted inputs rather than changing
their normaling; their routes/state remain saved.

The native desktop additionally has an explicitly ignored real-companion test,
`hosted_callback_actual_plugin_native_impulse_and_control_capture`. Supply
`OPENPATCH_HOSTED_COMPANION`, `OPENPATCH_HOSTED_SDK`, `OPENPATCH_HOSTED_PLUGIN`
and `OPENPATCH_HOSTED_CALLBACK_REVIEW`, then run it with `--ignored`. It exercises
the actual desktop Callback, native event queue, fault/priming publication and
control-side capture, without opening a window/device. Its two one-frame
callbacks exceeded their processing budgets; a small event callback also overran
in the runner gate. Neither test certifies tiny callbacks or application playback. Activation and
live document/UI/MCP/capture integration remain in the application milestone.
