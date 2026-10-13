# Architecture and roadmap

## First milestone

`openpatch-core` owns the serializable patch, validation, graph compilation,
stateful DSP engine, offline rendering, and WAV encoding. It has no dependence
on a model, UI framework, audio device, or MCP client.

`openpatch` owns the application control API and adapters: a stdio MCP service
for editing/session operations and a small CLI for catalog/demo/offline rendering.
The CLI shares the core rendering API; it does not yet expose every MCP edit.

An edit clones the control graph, applies the command, compiles it to validate
the complete candidate, then commits patch plus revision. Compilation occurs
outside DSP processing. The current server creates fresh engines for offline
rendering and benchmarking; it does not retain live oscillator state across edits.

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

1. **Live instrument:** platform audio devices, bounded command queue, parameter
   smoothing, state-preserving graph edits, off-thread memory reclamation, xrun
   and callback percentile measurements on Linux, Windows, and macOS ARM.
2. **Visual rack:** native patch canvas, cables, module controls, undo/redo, module
   browser. All editing goes through the same application command interface as MCP.
3. **Musical breadth:** envelopes, bandlimited oscillators, filters, sequencers,
   MIDI, polyphony, explicit feedback, project persistence and migration.
4. **Shared control:** multiple agent/UI clients attached to one application
   session, subscriptions, transport state, cancellable background rendering.
5. **Measured optimization:** equivalent-workload VCV Rack baselines, profiling,
   dead-node elimination, SIMD and parallelism where they improve measured
   latency/throughput without changing the signal contract.

A VCV C++ plugin compatibility layer would be a separate substantial project.
Platform CI validates the portable headless core; hardware audio and native UI
require actual device testing before a platform can be called production-ready.
