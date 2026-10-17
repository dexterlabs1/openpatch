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
