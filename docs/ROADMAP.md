# Roadmap: Rack functionality, then measurable improvements

Baseline checked **2026-10-12**. This is an acceptance plan, not a completed-feature list. OpenPatch is currently a native Rust instrument prototype with 42 built-in module kinds, a tested reference analog voice, a versioned polyphonic engine, rack editing, and shared-session stdio MCP. It has not reached VCV Rack parity.

## Define what “parity” includes

The first target is **VCV Rack 2 Free standalone host functionality plus its 49 Core/Free modules**. The [Rack product page](https://vcvrack.com/Rack) lists Free as the standalone edition for macOS, Windows, and Linux. DAW plugin operation belongs to the Pro follow-on. Hosting other VST plugins through the separately purchased VCV Host product is another distinct scope.

The bounded stock inventory is 12 Core modules and 37 Free modules, individually tracked in [MODULE_COVERAGE.md](MODULE_COVERAGE.md), using the [Core library](https://library.vcvrack.com/Core) and [Free manuals](https://vcvrack.com/Free). None should count as complete merely because an OpenPatch module has a similar name or musical role. The seven legacy utilities, six reference-voice kinds, five functional utilities, four polyphonic routing/source utilities, MIDI input, stereo output, four additional stereo/pitch/routing processors, a four-output clocked LFO, a three-row sequencer, two sequential switches, three voltage/mute banks, four gate/voltage processors, two CV-controlled audio mixers, and a four-output audio oscillator do not constitute 42 completed VCV equivalents.

The Free manifest also retains two hidden legacy models, VCA-2 and Unity.
They are tracked separately for existing-patch compatibility in the coverage
matrix; the visible 49-module baseline does not erase those requirements.

The third-party ecosystem is an ongoing compatibility program, not a finite “all modules” milestone. A Rust extension API, a Rust implementation of selected module behavior, a `.vcv` importer, and a loader for existing VCV C++ binaries are four different deliverables. Each must state which plugins and versions it supports. Neither a generic filter nor a circuit-derived reference model establishes the behavior of a particular commercial module.

## Architecture audit

This plan starts from the code inspected on the baseline date:

- `openpatch-core`: version 1 preserves mono, same-sample DAG routing and cycle rejection. Version 2 implements named source ports, up to 16 input/output ports per current module, 0–16 channels per port, and a synchronous scheduler with one sample of delay per cable, including channel counts. Feedback cycles are supported in version 2. Limits remain 256 nodes and 1024 cables. See [ENGINE_V2.md](ENGINE_V2.md).
- Audio oscillator: four simultaneous outputs, FM/PWM, hard/soft sync and direct Fundamental 2.6.4 voltage comparisons. Reconstruction and retirement gaps remain; see [AUDIO_OSCILLATOR.md](AUDIO_OSCILLATOR.md).
- Reference DSP: voltage-based VCO/ladder/OTA VCA/RC ADSR/gate/audio conversion, with documented idealizations and numerical tests. Ladder and VCA each add 64 samples of oversampling latency; parallel paths are not compensated. Legacy patches retain normalized semantics. See [ANALOG_MODELS.md](ANALOG_MODELS.md).
- Functional utilities: quantizer, sample-and-hold, seeded Gaussian white noise, attenuverter and crossfade retain mono version-1 behavior and declared version-2 voice policies. Four additional version-2 kinds provide polyphonic constants, Split, Merge and Sum. Complete corresponding VCV module contracts remain open.
- Native host: rack browser, module movement, rotary controls, cable creation/removal, single-file native projects with Save/Save As/Revert, recovery copies, and legacy JSON/sidecar import, shared multi-step patch undo/redo, and optional GLSL visuals. Studio Rack chrome provides direct transport/save/history controls and a bounded, tabbed Scope/Visuals dock beside the module library. Consecutive parameter edits can share an explicit gesture group. Stable history identities restore layouts for states observed by the GUI, with at most 129 retained layout snapshots. Single/group module moves now create one checkpoint in that shared history; drag previews are excluded from saved geometry. Selectable-device stereo output, one hardware MIDI input, polyphonic MIDI → CV, and a performance keyboard are implemented. Hardware audio input and hands-on MIDI/device qualification remain open. See [MIDI_AUDIO.md](MIDI_AUDIO.md).
- Control: UI and MCP share revision-checked transactions and bounded history. `preview_edits` validates a candidate without committing; `apply_edits` validates the final graph and commits up to 256 edit operations as one revision/history action. History retains at most 128 prior/future patch snapshots combined. Rendering/export clone the active patch without cloning history. Audio swaps compiled graphs through bounded queues and carries matching DSP state forward. Stdio starts with one application instance; it cannot attach to an arbitrary already-running window.
- Validation: repository tests and platform CI exist. Independent engine-v2 contract tests pass, covering timing/counts, feedback, normalization, routing, retirement/state adoption, invalid data, legacy rendering and allocation/deallocation checks. Those do not establish an exhaustive hardware/device qualification matrix or a measured performance advantage over Rack. See [PERFORMANCE.md](PERFORMANCE.md).

“Partial” below means useful behavior exists but its acceptance gate is open. “Missing” means the capability is not implemented. Gates describe proposed OpenPatch tests, not claims about current results.

## Free host parity matrix

| Area | Current status | Acceptance gate |
|---|---|---|
| Named ports and multiple outputs | Foundation implemented in v2; Split provides 16 named outputs | Extend native interaction and persistence coverage across every multi-output module; retain invalid-port/direction and legacy-file tests as the catalog grows. |
| Polyphonic engine | Foundation implemented; 0–16-channel frames, voice policies and retirement tested | Broaden cross-talk and shrink/regrowth tests across every stateful model; measure 16-voice callback deadlines and memory; qualify actual multichannel device sinks separately. |
| Stacked input cables | Foundation implemented in v2: summed delayed frames, stable IDs, per-cable editing and incarnation-aware history | Qualify native device workflows and broader large-rack performance; complete Rack plug-order behavior and duplication placement parity. [Contract and controls](STACKED_CABLES.md). |
| Cable timing and feedback | Foundation implemented; per-cable value/count delay and deterministic loops tested | Retain node-order/callback invariance; add clock/reset and longer modulated feedback fixtures as those modules arrive; test feedback during live graph replacement. |
| Voltage and event contracts | Partial; reference modules use volts | Catalog every port's units, normaling, thresholds and channel policy; test gate/trigger edges, clock/reset ordering, pitch calibration and legacy conversions. |
| DSP state and parameters | Partial; compatible per-voice/edge adoption, retirement resets and crossfade | Define sample-timed parameter events; expand edit tests through envelope release, circuit transients, feedback and future delay tails; keep sample-rate and incompatible-model reset rules explicit. |
| Engine rate, diagnostics and scheduling | Partial; device-rate compilation, offline benchmark | User-selected engine/device rates, rate conversion, per-module timing and overload reporting; concurrency and SIMD changes must preserve the timing contract. |
| Stock module coverage | Partial; 49-entry inventory remains open | Every inventory row has complete port/parameter/state behavior, persistence and functional tests; publish differences rather than claiming equivalence for a substitute. |
| Rack construction | Partial; group selection, selected-group and hovered-module duplication with explicit cable policies, group movement/deletion, Initialize/Randomize, a native module preset library and saved bypass | Rack preset conversion/templates, exact duplication placement and complete layout history remain open; [editing contract](RACK_EDITING.md). |
| Parameter/cable interaction | Partial | Precise entry, fine control, reset, reconnect/delete/fan-out, keyboard shortcuts and cable presentation work together; no swallowed gestures or accidental edits behind dialogs. |
| History | Partial; shared 128-step patch/movement undo/redo, explicit parameter grouping and observed-state layout restoration implemented | Complete geometry history for states the GUI has not observed; broaden device/native gesture qualification and project-history coverage. |
| Browser/navigation | Partial; docked library with search, role filters, saved favorites, keyboard access, contextual help and drag-to-add previews | Zoom/pan and selection reveal are implemented; broader large-rack navigation and full Rack browser equivalence remain open. |
| Project lifecycle | Partial; coherent native projects, atomic Save/Save As, Revert, current SEQ-3 Run capture, Run-aware content dirty state and per-instance recovery implemented | Qualify physical interruption/durability across filesystems, standard file dialogs, project-level MCP operations, factory templates, assets and VCV preset conversion. Native module presets are available separately. See [PROJECTS.md](PROJECTS.md) for guarantees and remaining boundaries. |
| Hardware audio | Partial; selectable device and explicit stereo sink | Select backend/device/rate/buffer; stereo and 8/16-channel input/output; loopback channel-isolation and latency checks; hot unplug/reconnect without callback blockage. |
| MIDI and keyboard performance | Partial; hardware input, keyboard, voice allocation and sample-timed traces | Input/output notes, gates, CC, pitch bend, pressure, clock/transport, learn/mapping, poly allocation, and keyboard performance; timestamp/order tests and stuck-note recovery. |
| Plugin extensibility | Missing | Versioned Rust module API/manifests, state migration, install/discovery/update and missing-module recovery; publish compatibility boundaries. Existing VCV binary support remains a separate unfulfilled gate. |
| Existing Rack projects | Missing | Supported `.vcv` imports preserve routings, parameters and layout; unknown modules/assets remain explicit and recoverable; publish a version/module compatibility matrix. |
| Platform release quality | Partial; macOS ARM first | Repeatable macOS ARM builds and actual input/audio/UI tests, then Linux/Windows device qualification; clean-machine install, upgrade, recovery and accessibility checks. |

The engine targets derive from VCV's [voltage/timing contract](https://vcvrack.com/manual/VoltageStandards) and [polyphony documentation](https://vcvrack.com/manual/Polyphony). In particular, 0-channel ports behave as unpatched, mono signals are a special case of polyphony, and Rack's documented cable delay matters for feedback and event ordering. Our version-1 scheduler must not silently change to those semantics.

Host workflow targets are grounded in VCV's [menu reference](https://vcvrack.com/manual/MenuBar), [key commands](https://vcvrack.com/manual/KeyCommands), and [getting-started workflow](https://vcvrack.com/manual/GettingStarted). Rack includes persistent patch operations, history, configurable engine/view behavior, direct module/parameter interaction, and cable editing. Equivalent task outcomes matter; identical artwork is not required. Audio and MIDI scope follows the [Core manual](https://vcvrack.com/manual/Core).

## Implemented engine foundation and next gates

The [version-2 contract](ENGINE_V2.md) is implemented and tested. The existing
`Patch` wire model gains `output_port`, and `Cable` gains source `output`, both
defaulting to `out`. `Patch::default()` remains version 1; `Patch::new_v2()`
explicitly selects version 2. Patch version chooses timing; there is no separate
`PatchV2` type or independently selectable timing field. Old files retain their
same-sample mono behavior. Explicitly changing their version changes timing.

Compilation resolves stable node/port names into routing indices and allocates
voice storage. `PortFrame` carries `[f32; 16]` plus a channel count.
`process_poly_block` returns full frames; the existing mono API returns channel
0 rather than mixing voices. `poly_sum` supplies explicit unity mixing. All
version-2 cables read the previous completed frame, including its channel
count; newly connected edges start empty. Compatible unchanged edges and active
voices retain state across graph replacement. Retired voices reset, and
ordinary modules execute only their active preallocated states.

Independent tests in
`crates/openpatch-core/tests/engine_v2_contract.rs` cover old JSON/render
compatibility, named outputs, zero/one/sixteen channels, sparse routing,
normalization and broadcast, exact self/two-node recurrences, callback
partitioning, state adoption and retirement. A 16-voice VCO/ladder graph passes
the first-callback and adoption allocation/deallocation check. This proves the
tested contracts, not a 16-voice live performance guarantee.

The next gates are:

1. **Complete rack construction workflows.** Selected-group and hovered-single
   duplication now use distinct cable policies, including outgoing stacked
   connections for a single-module copy. Initialize, Randomize and native presets
   share the UI/MCP transaction path. Add Rack preset conversion/templates,
   exact duplication placement and geometry history for unseen intermediate MCP
   states. Single/group movement is now undoable, and canceled drag previews
   never enter saved geometry.
2. **Profile representative polyphonic workloads.** Measure callback deadlines,
   graph-compilation cost and memory at one and sixteen voices on supported
   hardware. Preserve the reference timing when introducing SIMD or scheduling
   optimizations. The first circuit optimization retains exact checked results
   and reduces the paired M1 Max reference-workload processing time by about
   33%; the sixteen-voice workload remains slower than realtime. Live callback
   tails, crossfade cost and broader hardware results remain open. See
   [PERFORMANCE.md](PERFORMANCE.md).
3. **Broaden live state/event tests.** Exercise graph edits during envelope
   release, circuit transients and feedback; add direct/indirect clock-reset
   fixtures with sequencing modules. Keep crossfades a host policy rather than
   claiming them as circuit behavior.
4. **Connect the foundation to complete instrument workflows.** Qualify native
   multi-output patching and persistence, then stereo/multichannel device I/O
   and MIDI allocation. Every new module still needs its own complete port,
   parameter, voice-policy and state contract.

## Three-row sequencer: implemented contract, partial parity

The 3×8 editor, all five inputs and sixteen outputs, pattern rotation, live
step/run indicators, and bounded Clock/Run/Reset actions are implemented.
Offline MCP renders/captures accept ordered module events alongside MIDI.
[SEQUENCER.md](SEQUENCER.md) specifies timing, state, persistence, and explicit
differences from the pinned reference. The connected example uses all three
rows for pitch, filter movement, and accents.

Acceptance tests cover reset/clock ordering, one-step patterns, dynamic length,
stopped clocks, rewiring, exact pulses, adoption, chained sequences, callback
heap activity, and module removal/recreation across coalesced graph updates.
The queue distinguishes accepted actions from audio-applied actions and reports
expiry, obsolete identities and backpressure.

Remaining gates are direct differential traces against the pinned VCV version,
runtime state persistence, parameter/phase mapping alignment, and actual native
playback/device qualification. Current policy differences remain explicit;
passing OpenPatch tests does not promote SEQ 3 to complete VCV equivalence.

## Clocked routing

Two sequential switches now supply the documented 1-to-4 and 4-to-1 signal
roles, including step count, clock/reset priority, polyphony, de-click ramps
and actual engine route indicators. The [switch contract](SWITCHING.md) records
mode-specific mono behavior, detector initialization and gain-state rules.
A connected stereo example exercises both signal directions. Complete parity
still requires reference traces, legacy import/state mapping and
native-device qualification.

## Voltage banks and host bypass

8vert, Mutes, and CV Mix now implement their specified signal roles with dedicated
panels. Mutes preserves the pinned source's mono inherited rows; 8vert inherits
the whole upstream input frame. CV Mix sums independent 10 V normals with bipolar
levels. See [the utility contract](VOLTAGE_UTILITIES.md).

Saved bypass works in both engine versions through the shared UI/MCP history.
Routes are declared per kind; ordinary DSP freezes, outgoing cable samples clear
on transitions, and a separate action epoch cancels stale sequencer requests.
MIDI keeps its note/pulse state current while emitting silence. These policies
are explicit where they differ from Rack. Direct reference comparisons, old
Mutes state migration, Rack patch import, and native qualification remain open.

## Gate and voltage processors

Logic, Compare, Process, and Rescale now provide their specified signal roles,
including saved reflection settings, retained per-voice sampling histories,
momentary buttons, and actual DSP indicator snapshots. A bounded emergency
release path prevents queued presses from defeating a release when the event
queue fills. UI focus and pointer cleanup share the typed MCP action interface.
The [signal processor contract](SIGNAL_PROCESSING.md) records explicit numerical
and timing differences. Direct Rack traces, legacy state import, and native
device tests remain open; implemented ports do not establish full host parity.

## Voltage-controlled audio mixing

VCA Mix and Fade add independent channel taps, per-channel and master CV,
complementary crossfading, saved response settings, and live mixer meters.
The connected **Layered stereo morph** example exercises both processors.
[Voltage mixing](VOLTAGE_MIXING.md) records the exact gain/voice/meter contracts,
including source-specific padded meter lanes. Direct reference execution,
Rack parameter import and actual native playback remain open acceptance gates.

## MCP and visuals beyond the baseline

These are OpenPatch differentiators and do not substitute for missing host or module functionality.

| Capability | Current status | Acceptance gate |
|---|---|---|
| Machine-readable instrument contract | Partial: catalog/patch inspection includes versions, named outputs and voice policies | Complete per-port units/normaling, model latency and bounded resource-cost metadata; no UI-only meaning required to construct a valid patch. |
| Atomic multi-edit transactions | Implemented: candidate preview and atomic apply, final validation, one revision/history action | Structured change summary included; keep rejection/rollback and stale-revision tests; preview/apply clients must carry the preview revision into commit. |
| Agent discovery and attachment | Missing | Attach to an existing chosen desktop session, with session identity and explicit transport lifetime; demonstrate simultaneous UI/two-client edits without lost updates. |
| Subscriptions and diagnostics | Missing | Bounded revision/transport/meter/solver-event subscriptions, backpressure and reconnect/resync tests; no JSON or network work in DSP callbacks. |
| Render/analysis jobs | Partial: synchronous offline render and benchmark | Cancellable background jobs, progress, reproducible patch/seed/configuration records, and artifact results; GUI and other control requests remain responsive. |
| Musical operations | Partial: graph tools, shared undo/redo, grouped parameter edits, atomic sequence rotation and live/offline module actions | Sample-timed automation, audition, transport control and reusable subpatch operations share the same validation/history as human edits. |
| Objective evaluation | Partial: numerical tests, per-channel statistics and peak-preserving MCP offline waveform capture | Agents can request calibrated spectra, pitch, clipping, latency and CPU diagnostics from explicit taps; tests use known signals and declared error bounds. |
| Native visual instruments | Partial: GLSL dock/editor plus calibrated two-source live scope with arbitrary ports/voices, trigger, freeze and X/Y | Arbitrary-port and per-voice taps, calibrated scope/spectrum, saved shader/preset state and synchronized controls; GPU failures preserve the instrument and last valid shader. |
| Better performance at equal quality | Unproven | Fixed workload/quality comparisons, reproducible CPU and callback-tail measurements, memory and power observations; publish improvements and regressions separately. |

Local controls, remote commands and project loading should use one command/history layer. A future control service should not depend on the GUI repaint loop to acknowledge whether the audio engine has applied a revision. Distinguish committed revision from audio-applied revision in diagnostics.

## Pro and ecosystem follow-ons

Rack Pro adds DAW plugin formats, host audio/MIDI, parameter automation, offline bounce and its own modules. Its documented formats are VST2, VST3, Audio Unit and CLAP, with up to 16 audio input/output channels. These belong to a separate milestone; no OpenPatch DAW plugin exists today. [Official Pro feature reference](https://vcvrack.com/manual/RackPro)

Before claiming this follow-on, test instrument/effect operation, DAW transport, sample-offset MIDI/automation, state recall, plugin latency reporting, offline/realtime consistency and multiple independent instances in named hosts. Choose supported formats explicitly; do not imply every format is delivered by adding one wrapper. Inventory Pro modules separately. Hosting external plugins inside OpenPatch is another host/bridge project.

For third-party breadth, prioritize concrete user patches and publish per-module compatibility status. A module port needs behavior, state/assets, signal quality and performance validation. A native extension ecosystem needs stable APIs and discoverable distribution. A C++ compatibility layer additionally needs a versioned host ABI/UI/lifecycle contract. No finite stock-module checklist demonstrates the entire ecosystem.

## Permissively licensed DSP candidates

These are researched candidates, not newly added dependencies or endorsements of their numerical equivalence to VCV. Licenses below were checked in the upstream projects on the baseline date. Pin reviewed versions, preserve required notices and recheck individual files/dependencies when reusing implementation code. VCV's own source and assets are not a blanket permissive code source; its [licensing documentation](https://vcvrack.com/manual/PluginLicensing) distinguishes their terms.

| Primary project | Upstream license | Practical use in OpenPatch | Integration gate |
|---|---|---|---|
| [FunDSP](https://github.com/SamiPerttu/fundsp) | MIT or Apache-2.0 | Rust oscillators, filters, noise and delay building blocks; compare components before writing every generic utility | Preallocate; adapt its units/buffering explicitly; test each selected component and required quality. Do not replace the graph contract blindly. |
| [Rubato](https://github.com/HEnquist/rubato) | MIT or Apache-2.0 | Rust engine/device rate conversion and independently clocked streams | Preallocated processing, bounded buffering, measured group delay and clock-drift behavior. |
| [RustFFT](https://github.com/ejmahler/RustFFT) | MIT or Apache-2.0 | Rust spectrum analysis, validation and future convolution | Plan and allocate scratch outside real-time processing; verify scaling/window calibration and ARM/x86 results. |
| [chowdsp_wdf](https://github.com/Chowdhury-DSP/chowdsp_wdf) | BSD-3-Clause, with credited component notices | C++ wave-digital circuit algorithms and tests to study/port for an original Rust circuit library | Preserve provenance; validate the Rust port against circuit equations and independent references; this is not a ready-made Rust dependency. |
| Original Mutable Instruments firmware: [Plaits oscillator](https://github.com/pichenettes/eurorack/blob/master/plaits/dsp/oscillator/oscillator.h), [Rings resonator](https://github.com/pichenettes/eurorack/blob/master/rings/dsp/resonator.cc) | MIT headers in these specific DSP files | Concrete oscillator and resonator algorithms to port to Rust after the engine foundation; full Plaits/Rings module ports are not implemented | Pin revisions; audit every copied file, dependency and asset separately; preserve notices and validate reference renders, state behavior and performance. The [VCV Audible Instruments wrapper](https://github.com/VCVRack/AudibleInstruments/blob/master/LICENSE.md) has separate GPL licensing; these file-level findings do not license that wrapper or all firmware/assets. |

## Release gates

Advance by evidence, not module count alone:

1. **Engine foundation:** implemented named multi-output/polyphonic/timed routing and backward-compatible files pass the independent contract tests; remaining performance, broader state/event and native/device qualification gates are listed above.
2. **Useful instrument:** shared history, reliable projects, stereo audio/MIDI and a playable polyphonic voice pass native macOS tests; core concurrency and device-failure cases are exercised.
3. **Stock Free coverage:** all 49 inventory rows meet their contracts; remaining behavioral differences and host/plugin compatibility gaps are listed openly.
4. **Free host parity:** every host-matrix gate has a test artifact and supported-platform result. An unsupported plugin/file workflow remains a gap, not an implied success.
5. **Beyond parity:** MCP/visual features have end-to-end tests and measurable benefits; quality/performance claims cite matched experiments. Pro and third-party work retain their separate matrices.

Each milestone should link its patch fixtures, reference version, test results, known differences and representative recordings/screenshots. Refresh this baseline when upstream functionality or OpenPatch's contracts change.
