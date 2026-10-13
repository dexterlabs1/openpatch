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
audio output. The rack includes a module browser, rotary controls, cable
editing, output selection, single-file native projects, legacy JSON import, and
per-session recovery. See [PROJECTS.md](PROJECTS.md) for persistence and close
concurrency contracts. The desktop starts with
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
sample rate, without rewriting the stored patch rate. CPAL uses the selected or default
output device with a supported f32, i16, or u16 format. Explicit stereo sinks
map L/R to the first two channels (averaged on mono devices); other patches
duplicate mono across device channels. Samples are bounded to [-1,1]. Device failures are surfaced in
the window with a retry control; editing remains available without an audio device.

Two bounded single-producer/single-consumer queues transfer compiled engine
ownership into the callback and retired engines back to the UI for destruction.
Updates are checked at callback boundaries. A full queue retains ownership and
defers the update or retirement. The callback does not allocate, free engines,
lock a mutex, log, or perform file/GPU work during rendering. Replacement engines
adopt matching module state by stable ID and exact kind, then crossfade for
approximately 5 ms. The precompiled ID lookup avoids allocation during adoption.
Oscillator phases, capacitor states, and resampler histories survive edits.
Transport and volume use
an approximately 10 ms gain smoother. Pause fades the output to silence while
the engine continues advancing; it is not a rewind. Pause and resume reset MIDI
performance state. Separate bounded hardware and UI MIDI queues deliver events
at sample boundaries to both crossfade branches. Timestamp mapping, reset
generations, keyboard behavior, and offline traces are documented in
[MIDI and audio](MIDI_AUDIO.md).

The audio callback publishes a decimated waveform and peak through atomics.
Native visual analysis and OpenGL work run on the UI/render thread. The display
uses best-effort recent samples, and does not require the audio callback to wait
for a complete visual frame. See [visualizers](VISUALIZERS.md) for the shader
interface and analysis limitations.

The version-1 engine sorts dependencies into a stable order, stores integer
input indices, and processes nodes across fixed-capacity blocks. Audio-rate
modulation still varies per sample. The version-2 foundation adds named source
ports and fixed-capacity 16-channel frames, with every cable carrying a completed
previous-sample frame. This allows feedback without evaluation-order-dependent
timing. Arbitrary caller buffer lengths must preserve the same sample sequence.
There is no runtime graph search, JSON, filesystem access, mutex, or trait-object
plugin dispatch in the sample loop. The precise API, migration rules and
acceptance gates are specified in [Engine version 2](ENGINE_V2.md).

## Signal contract

The reference voice uses volts, 1 V/oct pitch, 0–10 V envelope/gates, and explicit
voltage/audio conversion. The seven legacy utilities preserve their original
normalized signal semantics. Legacy oscillator FM is additive Hz. Legacy gain
has direct multiplicative CV, normalled to 1; the OTA VCA's CV is normalled to
10 V. Other unconnected inputs are zero. WAV export
clips to [-1,1]; the graph can carry signals outside that range within its numeric
safety limits. Consult `catalog` for exact limits. These conventions are versioned
and intentionally do not promise VCV patch or plugin compatibility.

Patch version selects engine semantics: version 1 remains mono, uses same-sample
DAG routing and rejects cycles. Version 2 uses named ports, 0–16 active channels
and one sample of delay per cable, including its channel count. New cable frames
start empty. Version-1 files must never acquire version-2 timing implicitly.
`Patch::default()` stays version 1; `Patch::new_v2()` explicitly requests the new
contract. The mono render API returns channel 0; explicit `poly_sum` performs
voice mixing. Polyphonic processing does not itself add hardware channels.

## Next milestones

1. **Model validation:** expand the documented reference circuits, spectral and
   numerical error measurements, hardware comparisons, and latency alignment.
   See [analog models](ANALOG_MODELS.md) and [module coverage](MODULE_COVERAGE.md).
2. **Live instrument refinement:** device selection, latency controls, xrun and
   callback percentile measurements, with macOS ARM as the first interactive target.
3. **Rack workflow and breadth:** complete the named-port/polyphonic/feedback
   foundation's acceptance gates, then layout undo/redo, sequencing, MIDI and
   richer project persistence.
4. **Additional control transports:** multiple agents attached to one existing
   application session, subscriptions, and cancellable background rendering.
   Current native stdio already shares one agent connection with its own GUI.
5. **Measured optimization:** equivalent-workload VCV Rack baselines, profiling,
   dead-node elimination, SIMD and parallelism where they improve measured
   latency/throughput without changing the signal contract.

A VCV C++ plugin compatibility layer would be a separate substantial project.
Platform CI validates the portable headless core; hardware audio and native UI
require actual device testing before a platform can be called production-ready.

The current cross-cutting parity target and acceptance gates are tracked in the
[parity roadmap](ROADMAP.md). The shared control session supports atomic batch
edits, preview, and 128 graph undo/redo steps; GUI and MCP use the same revision
boundary. Layout history is a separate remaining gap.
