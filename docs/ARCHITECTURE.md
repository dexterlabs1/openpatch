# Architecture and roadmap

## Current implementation

`openpatch-core` owns the serializable patch, validation, graph compilation,
stateful DSP engine, offline rendering, and WAV encoding. It has no dependence
on a model, UI framework, audio device, or MCP client.

`openpatch` exposes the shared control library and stdio MCP adapter, plus a CLI
for catalog/demo/offline rendering and the standalone browser visualizer export.
The CLI shares the core rendering API; it does not expose every MCP edit as a
shell command.

`openpatch-desktop` owns the native eframe/egui window, OpenGL visualizer, and CPAL
audio output. The rack includes a module browser, parameter sliders, cable
editing, output selection, and workspace JSON save/load. The desktop starts with
the demo patch and muted transport. Play/pause and master volume are local UI
controls. The `--mcp` option serves stdio on a background control thread, sharing
the window's `Arc<Mutex<Session>>`. Audio never acquires that mutex. Separate
processes have separate patches; there is no listener for attaching to an
existing application window.

An edit clones the control graph, applies the command, compiles it to validate
the complete candidate, then commits patch plus revision. Compilation occurs
outside DSP processing. GUI and MCP edits use the same transaction and revision
checks. Offline rendering, benchmarking, and export operations clone a coherent
session snapshot, then release the mutex before doing their work. They do not
advance the live patch revision.

## Native audio and rendering

The UI polls the session revision and compiles changed patches for the device's
sample rate, without rewriting the stored patch rate. CPAL uses the default
output device with a supported f32, i16, or u16 format. Mono output is duplicated
to the device's channels and bounded to [-1,1]. Device failures are surfaced in
the window with a retry control; editing remains available without an audio device.

Two bounded single-producer/single-consumer queues transfer compiled engine
ownership into the callback and retired engines back to the UI for destruction.
Updates are checked at callback boundaries. A full queue retains ownership and
defers the update or retirement. The callback does not allocate, free engines,
lock a mutex, log, or perform file/GPU work during rendering. Replacement engines
start from reset and crossfade for approximately 5 ms. Transport and volume use
an approximately 10 ms gain smoother. Pause fades the output to silence while
the engine continues advancing; it is not a rewind.

The audio callback publishes a decimated waveform and peak through atomics.
Native visual analysis and OpenGL work run on the UI/render thread. The display
uses best-effort recent samples, and does not require the audio callback to wait
for a complete visual frame. See [visualizers](VISUALIZERS.md) for the shader
interface and analysis limitations.

The engine sorts dependencies into a stable order, stores integer input indices,
and processes nodes across fixed-capacity blocks. Audio-rate modulation still
varies per sample. Arbitrary caller buffer lengths are split into chunks without
allocation. There is no runtime graph traversal, JSON, filesystem access,
mutex, or trait-object plugin dispatch in the sample loop.

## Signal contract

This initial engine uses normalized mono signals rather than Eurorack voltages.
Oscillator FM is additive Hz. Gain has signal and optional multiplicative CV;
an unconnected CV has unity gain. Other unconnected inputs are zero. WAV export
clips to [-1,1]; the graph can carry signals outside that range within its numeric
safety limits. Consult `catalog` for exact limits. These conventions are versioned
and intentionally do not promise VCV patch or plugin compatibility.

Cycles are rejected. A future feedback design must define an explicit sample
delay and strongly connected component scheduling; simply accepting cycles in
the current block graph would change feedback timing and pitch.

## Next milestones

1. **Live instrument refinement:** state-preserving parameter/graph edits,
   device selection, latency controls, xrun and callback percentile measurements
   on Linux, Windows, and macOS ARM. Current crossfades do not preserve DSP state.
2. **Rack workflow:** undo/redo, freely positioned modules, keyboard navigation,
   and richer project persistence. Current rack positions are automatic.
3. **Musical breadth:** envelopes, bandlimited oscillators, filters, sequencers,
   MIDI, polyphony, explicit feedback, project persistence and migration.
4. **Additional control transports:** multiple agents attached to one existing
   application session, subscriptions, and cancellable background rendering.
   Current native stdio already shares one agent connection with its own GUI.
5. **Measured optimization:** equivalent-workload VCV Rack baselines, profiling,
   dead-node elimination, SIMD and parallelism where they improve measured
   latency/throughput without changing the signal contract.

A VCV C++ plugin compatibility layer would be a separate substantial project.
Platform CI validates the portable headless core; hardware audio and native UI
require actual device testing before a platform can be called production-ready.
