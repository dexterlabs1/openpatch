# RackMCP sequence engine reference

This compares the real pinned C++ `seq::Engine`, `validateData`, `parseData`
and `laneValue` against the compiled Rust sequencing primitives. The C++
adapter links the untouched `sequencer.cpp` and `plan.cpp`, using Jansson from
the genuine Rack SDK. The Rust adapter links `openpatch-core`. Neither adapter
reimplements playback or curve calculation.

Source: Zachary Scheffler's RackMCP 2.0.0,
[`285e84c`](https://github.com/dexterlabs1/vcv-rack-mcp/tree/285e84c5ff2d3ecef25890da07f578af6a0c5ca6/plugins/RackMCP),
GPL-3.0-or-later. Source is supplied separately, never fetched from mutable HEAD.
The frozen manifest must include the original tracked `src/gen` protocol header.

```sh
python3 scripts/rackmcp_sequence_reference/run.py \
  --source native-test-output/rack-control-import/reference/rackmcp \
  --sdk /path/to/Rack-SDK \
  --target /path/to/shared-cargo-target \
  --output native-test-output/rack-control-import/sequence-reference-new
```

On macOS ARM, the initial completed run has 18 cases / **620,730 frames** at
44.1/48/96 kHz. Every output float, channel count, transport index/count,
double beat position and automation curve value matches bit for bit. The six
declared scenarios cover swing/polyphony/probability, slides/ratchets/stops,
pattern requests, external stalls/recovery, seek/reset, and data adoption.
Each run freezes its declaration and adapters before numerical execution and
retains both traces, build identity and a JSON report. The first adapter launch
failed to locate `libRack.dylib`; explicit SDK dynamic-library search fixed the
harness. No model or tolerance change was needed.

This is a bounded engine/parser comparison. It does **not** establish native
graph scheduling, parameter-target writes/cache, strict validation equivalence
for every possible JSON object, Rack module Schmitt/divider/light behavior,
serialization trimming, live Listen capture, Bridge protocol, device behavior
or whole-patch equivalence. Rust compilation explicitly rejects NUL names and
raw lane values/deltas that overflow f32; those unsafe domains are not compared.
The existing native Score is a distinct model. No new import mapping is enabled
by this primitives checkpoint.
