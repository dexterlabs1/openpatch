# openpatch-core

Original Rust mono DSP engine for OpenPatch, with no Rack source or dependencies.

`Patch::compile()` validates and schedules a directed acyclic graph on the control
thread. `Engine::process_block()` advances its state into any output slice, using
preallocated 128-sample buffers and no allocation, locks, or I/O. Callback lengths
do not alter the result. Replace compiled engines outside the audio callback.

`catalog()` describes exact kinds, ports, defaults and parameter bounds.
`demo_patch()` supplies the legacy sine/tremolo study; `analog_demo_patch()` supplies
the six-module voltage-based reference voice. `render()`
creates up to 30 seconds of mono audio offline. `write_wav()` exports PCM16 with
clipping at ±1 and refuses to overwrite an existing path.

The graph has at most 256 nodes and 1,024 cables. IDs contain 1–64 ASCII letters,
digits, periods, underscores or hyphens and remain stable across array reordering.
Only one cable can feed a particular input. Cycles are rejected; feedback support
requires a future explicit delay module. Node state starts at zero on compilation;
`Engine::adopt_state_from()` preserves matching states for live replacements.
An omitted output produces silence; a selected output may be any existing node.
Unconnected inputs are zero, except legacy `gain.cv` (1) and `analog_vca.cv` (10 V).

Legacy oscillator FM is additive Hz, evaluated each sample, with instantaneous frequency
limited to 0–0.45 times the sample rate. These are simple sine oscillators and a
one-pole filter, not analog circuit or VCV module emulations. Gain and mixer
arithmetic is bounded at ±1,000,000 to keep extreme graphs finite; normal signals
are not clipped. Very small filter state is flushed to zero to avoid subnormal
tails. Sample rates span 8,000–192,000 Hz. Unknown kinds, ports, parameters and JSON
fields fail validation rather than silently changing a patch's meaning.

The six reference kinds add an ideal ramp VCO, matched-transistor ladder,
differential-pair VCA, RC ADSR, gate clock, and voltage/audio adapter. The ladder
uses a bounded coupled nonlinear solve; filter and VCA use paired signal/CV
oversampling. See [model equations, assumptions, and validation](../../docs/ANALOG_MODELS.md).
They are not calibrated commercial hardware emulations or complete VCV equivalents.

Tests cover oscillator frequency/amplitude, sample-accurate modulation, filter
response, order independence, callback size independence, bounded extreme gain,
validation, WAV correctness and overwrite refusal. An integration test tracks
allocations during the very first and later callbacks.

Run `cargo test -p openpatch-core` and
`cargo bench -p openpatch-core --bench engine`. The release benchmark warms the
engine, then measures at least two seconds each for 1, 16 and 64 mixed oscillators.
It reports OS, architecture, CPU, compiler, wall time, rendered audio time, realtime
factor and nanoseconds per sample. This measures offline single-thread throughput;
it does not prove device callback deadlines, full application latency or superiority
over another synthesizer. Record the complete output and power settings alongside
any published results. No comparative performance claim is made here.
