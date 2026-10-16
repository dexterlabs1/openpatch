# Roadmap: Rack functionality, then measurable improvements

Baseline updated **2026-10-15**, with a [manual and source review on 2026-10-13](VCV_MANUAL_REVIEW.md). This is an acceptance plan, not a completed-feature list. OpenPatch is currently a native Rust instrument prototype with 69 built-in module kinds, a tested reference analog voice, a versioned polyphonic engine, rack editing, and shared-session stdio MCP. It has not reached VCV Rack parity.

## Delivery order

Updated by user direction on **2026-10-15**: MIDI and live-performance work move
to the final delivery phase. MIDI Map and its controller-driven live parameter
foundation are deferred; their research remains available for that phase.

| Order | Focus | Main work |
|---|---|---|
| 1 | Design and patching experience | Consistent module/browser rendering, direct controls, typography, cable dragging, navigation and room lighting. |
| 2 | Synthesis and module fidelity | Complete non-MIDI stock synthesis/effects, validate existing modules against their references, and improve physical models. |
| 3 | Projects and compatibility | Reliable editing/history, presets/templates, `.vcv` import, missing-module recovery and selected open-source ports/extension foundations. |
| 4 | MCP and creative tools | Refine visible agent construction, inspection/analysis, instrument-authoring foundations and GLSL audio/visual workflows. |
| 5 — final | MIDI and live performance | MIDI Map, controller-driven parameter authority, remaining MIDI inputs/outputs, keyboard/stage workflows, expanded hardware I/O and latency/reconnect/device qualification. |

DSP correctness, CPU optimization and ordinary playback reliability remain part
of every phase. Existing MIDI features remain supported. Full stock/host parity
cannot be signed off until the final phase meets its acceptance gates.

## Current product design priority

The feedback shared on 2026-10-14 makes the next design pass a priority before
another batch of modules. Visible agent construction, a toolbar MCP session
control and room lighting are now implemented. The dark-room preset preserves
actual LED/display emission while dimming reflective rack surfaces.
Keep familiar rack patching and a single shared
module/browser renderer, while improving controls for a computer: ordinary
click and keyboard value entry, direct sliders or other appropriate editors,
clear signal flow, readable typography and fewer decorative hardware cues.
Shared generic controls now use direct vertical faders with numeric entry,
plus catalog-defined labeled choices for waveform, law, DC filtering, Score run
state and drum type.
Signal-driven top-plug lights now observe actual input summation and mono/poly
signals, with dark-room emission and bounded audio observation.
See [the detector and transport contract](PORT_LIGHTS.md).
Persistent View controls add cable opacity/tension and independent plug-light
brightness, including observation disabled at zero; [behavior and validation](CABLE_PRESENTATION.md).
Specialized panels keep their dedicated editors. Held cable drags pan at visible
rack edges. The broader module layout and material review remains open.
Any changes must preserve cable targets, explicit values, undo and MCP edits.
The latest shared rack/browser header pass removes brand/description headers
and makes module names prominent and left-aligned. Local layout checks and the
Mac ARM candidate build pass; capture review and native window/input
qualification remain separate gates.

The product direction also includes visible incremental agent work and eventual
creation of genuinely new modules. Existing module insertion and progressive
construction are distinct from authoring new DSP; live module authoring remains
unimplemented. Physical acoustic modeling means excitation, resonators and
damping; it is separate from FFT analysis or analog circuit emulation. The modal
resonator is an initial acoustic model, with its validation boundaries documented.

## Define what “parity” includes

The first target is **VCV Rack 2 Free standalone host functionality plus its 49 Core/Free modules**. The [Rack product page](https://vcvrack.com/Rack) lists Free as the standalone edition for macOS, Windows, and Linux. DAW plugin operation belongs to the Pro follow-on. Hosting other VST plugins through the separately purchased VCV Host product is another distinct scope.

The bounded stock inventory is 12 Core modules and 37 Free modules, individually tracked in [MODULE_COVERAGE.md](MODULE_COVERAGE.md), using the [Core library](https://library.vcvrack.com/Core) and [Free manuals](https://vcvrack.com/Free). None should count as complete merely because an OpenPatch module has a similar name or musical role. The current 69 built-in kinds include reference circuits, original utilities, and partial functional equivalents; this count does not represent 69 completed VCV equivalents.

The Free manifest also retains two hidden legacy models, VCA-2 and Unity.
They are tracked separately for existing-patch compatibility in the coverage
matrix; the visible 49-module baseline does not erase those requirements.

The third-party ecosystem is an ongoing compatibility program, not a finite “all modules” milestone. A Rust extension API, a Rust implementation of selected module behavior, a `.vcv` importer, and a loader for existing VCV C++ binaries are four different deliverables. Each must state which plugins and versions it supports. Neither a generic filter nor a circuit-derived reference model establishes the behavior of a particular commercial module.

## Architecture audit

This plan starts from the code inspected on the baseline date:

- `openpatch-core`: version 1 preserves mono, same-sample DAG routing and cycle rejection. Version 2 implements named input/output ports and 0–16 channels per port, and a synchronous scheduler with one sample of delay per cable, including channel counts. Feedback cycles are supported in version 2. Limits remain 256 nodes and 1024 cables. See [ENGINE_V2.md](ENGINE_V2.md).
- Audio oscillator: four simultaneous outputs, FM/PWM, hard/soft sync and direct Fundamental 2.6.4 voltage comparisons. Reconstruction and retirement gaps remain; see [AUDIO_OSCILLATOR.md](AUDIO_OSCILLATOR.md).
- Wavetable family: separate audio/modulation oscillators, immutable custom assets, worker preparation and revision-checked publication, portable projects/presets and shared panels. The checked 153-case actual-class run qualifies bounded digital behavior; audio geometry, nonpositive-frequency safety policy and remaining platform/UI gates are explicit in [WAVETABLES.md](WAVETABLES.md).
- Reference DSP: voltage-based VCO/ladder/OTA VCA/RC ADSR/gate/audio conversion, with documented idealizations and numerical tests. Ladder and VCA each add 64 samples of oversampling latency; parallel paths are not compensated. Legacy patches retain normalized semantics. See [ANALOG_MODELS.md](ANALOG_MODELS.md).
- Functional utilities: quantizer, sample-and-hold, seeded Gaussian white noise, attenuverter and crossfade retain mono version-1 behavior and declared version-2 voice policies. Four additional version-2 kinds provide polyphonic constants, Split, Merge and Sum. SUM now adds output attenuation and revision-coherent six-band level/channel telemetry; [contract](POLYPHONIC_SUM.md). [VIZ](POLYPHONIC_VIZ.md) adds 16-channel signed indicators and genuine input-only modules, with primitive reference comparisons at three sample rates. Complete corresponding VCV module contracts remain open.
- Native host: rack browser, module movement, direct parameter controls, cable creation/removal, single-file native projects with Save/Save As/Revert, recovery copies, and legacy JSON/sidecar import, shared multi-step patch undo/redo, and optional GLSL visuals. Studio Rack chrome provides direct transport/save/history controls and a bounded, tabbed Scope/Visuals dock beside the module library. Consecutive parameter edits can share an explicit gesture group. Stable history identities restore layouts for states observed by the GUI, with at most 129 retained layout snapshots. Single/group module moves now create one checkpoint in that shared history; drag previews are excluded from saved geometry. Selectable-device stereo output, one hardware MIDI input, polyphonic MIDI → CV, sixteen-output MIDI CC and MIDI Gate with Learn, and a performance keyboard are implemented. Hardware audio input and hands-on MIDI/device qualification remain open. See [MIDI_AUDIO.md](MIDI_AUDIO.md).
- Control: UI and MCP share revision-checked transactions and bounded history. `preview_edits` validates a candidate without committing; `apply_edits` validates the final graph and commits up to 256 edit operations as one revision/history action. History retains at most 128 prior/future patch snapshots combined. Rendering/export clone the active patch without cloning history. Audio swaps compiled graphs through bounded queues and carries matching DSP state forward. Desktop stdio starts with one application instance. On macOS/Linux, the CLI can also discover and attach to an existing window over a private local socket; Windows retains stdio.
- Validation: repository tests and platform CI exist. Independent engine-v2 contract tests pass, covering timing/counts, feedback, normalization, routing, retirement/state adoption, invalid data, legacy rendering and allocation/deallocation checks. Those do not establish an exhaustive hardware/device qualification matrix or a measured performance advantage over Rack. See [PERFORMANCE.md](PERFORMANCE.md).

“Partial” below means useful behavior exists but its acceptance gate is open. “Missing” means the capability is not implemented. Gates describe proposed OpenPatch tests, not claims about current results.

## Free host parity matrix

| Area | Current status | Acceptance gate |
|---|---|---|
| Named ports and multiple outputs | Foundation implemented in v2; Split provides 16 named outputs | Extend native interaction and persistence coverage across every multi-output module; retain invalid-port/direction and legacy-file tests as the catalog grows. |
| Polyphonic engine | Foundation implemented; 0–16-channel frames, voice policies and retirement tested | Broaden cross-talk and shrink/regrowth tests across every stateful model; measure 16-voice callback deadlines and memory; qualify actual multichannel device sinks separately. |
| Stacked input cables | Foundation implemented in v2: summed delayed frames, stable IDs, per-cable editing and incarnation-aware history | Qualify native device workflows and broader large-rack performance; qualify independent plug ordering on physical devices and complete duplication placement parity. [Contract and controls](STACKED_CABLES.md). |
| Cable timing and feedback | Foundation implemented; per-cable value/count delay and deterministic loops tested | Retain node-order/callback invariance; add clock/reset and longer modulated feedback fixtures as those modules arrive; test feedback during live graph replacement. |
| Voltage and event contracts | Partial; reference modules use volts | Catalog every port's units, normaling, thresholds and channel policy; test gate/trigger edges, clock/reset ordering, pitch calibration and legacy conversions. |
| DSP state and parameters | Partial; compatible per-voice/edge adoption, retirement resets and crossfade; Stock Delay runtime moves without copying its large history, using cached-output fade for the retired graph | Qualify live Delay tails and callback deadlines, expand effects coverage and sample-timed parameter events; borrowed Delay adoption and existing StudioFx can still copy large state. Keep rate/incarnation rules explicit. |
| Engine rate, diagnostics and scheduling | Partial; device-rate compilation, offline benchmark | User-selected engine/device rates, rate conversion, per-module timing and overload reporting; concurrency and SIMD changes must preserve the timing contract. |
| Stock module coverage | Partial; 49-entry inventory remains open | Every inventory row has complete port/parameter/state behavior, persistence and functional tests; publish differences rather than claiming equivalence for a substitute. |
| Rack construction | Partial; group selection, selected-group and hovered-module duplication with explicit cable policies, group movement/deletion, Initialize/Randomize, a native module preset library and saved bypass | Rack preset conversion/templates, exact duplication placement and complete layout history remain open; [editing contract](RACK_EDITING.md). |
| Parameter/cable interaction | Partial; anchored precision entry with same-module Apply & Next/Previous, cable-action guidance, and core Rack cable gestures, both-end menus, grouped moves/deletion, independent plug ordering, saved cable palettes and next-colour cycling implemented with guarded history | Precise entry, fine control, reset, reconnect/delete/fan-out, keyboard shortcuts and cable presentation work together; no swallowed gestures or accidental edits behind dialogs. |
| History | Partial; shared 128-step patch/movement undo/redo, explicit parameter grouping and observed-state layout restoration implemented | Complete geometry history for states the GUI has not observed; broaden device/native gesture qualification and project-history coverage. |
| Browser/navigation | Partial; docked library with saved 25–200% preview zoom, proportionate wrapping cards, pinned natural-size inspection, search, role filters, saved favorites, keyboard access, contextual help and drag-to-add previews | Rack zoom/pan, selection reveal and optional [Follow agent edits](AGENT_FOLLOW.md) are implemented; broader large-rack navigation and full Rack browser equivalence remain open. [Preview behavior](LIBRARY_ZOOM.md). |
| Project lifecycle | Partial; coherent native projects, atomic Save/Save As, Revert, current SEQ-3 Run, Push HOLD, Random Values, S&H/ASR and MIDI CC saved-value capture, content dirty tracking for persistent state and per-instance recovery implemented | Qualify physical interruption/durability across filesystems, standard file dialogs, project-level MCP operations, factory templates, assets and VCV preset conversion. Native module presets are available separately. See [PROJECTS.md](PROJECTS.md) for guarantees and remaining boundaries. |
| Hardware audio | Partial; selectable device and explicit stereo sink | Select backend/device/rate/buffer; stereo and 8/16-channel input/output; loopback channel-isolation and latency checks; hot unplug/reconnect without callback blockage. |
| MIDI and keyboard performance | Partial; hardware input, 32-pitch QWERTY/mouse keyboard with guarded note ownership, voice allocation, MIDI CC and note-gate conversion/Learn, paired 14-bit CC, MPE and sample-timed traces | Input/output notes, gates, CC, pitch bend, pressure, clock/transport, learn/mapping, poly allocation, and keyboard performance; timestamp/order tests and stuck-note recovery. |
| Plugin extensibility | Missing | Versioned Rust module API/manifests, state migration, install/discovery/update and missing-module recovery; publish compatibility boundaries. Existing VCV binary support remains a separate unfulfilled gate. |
| Existing Rack projects | General import missing; a [bounded reconstruction helper](STIW_RECONSTRUCTION.md) preserves 55 known source roles and 83 cable routes, with matching stock VCA/ADSR/VCF models and explicit substitutes | Supported `.vcv` imports preserve routings, parameters and layout; unknown modules/assets remain explicit and recoverable; publish a version/module compatibility matrix. |
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
   multi-output patching and persistence. Expanded hardware I/O and MIDI
   allocation belong to the final delivery phase. Every new module still needs its own complete port,
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
| Agent discovery and attachment | Partial: macOS/Linux local sockets, explicit window/workspace selection, independent client handshakes and shared revisions/history | Qualify simultaneous native UI/two-client edits and device behavior; implement Windows attachment. Headless bridge tests cover two-client conflicts, shared undo and transport lifetime. |
| Visible agent construction | Partial: bounded preflighted plans, paced real transactions, per-step undo, native progress/Pause/Resume/Cancel, module/parameter cues, new-cable reveals, existing-cable motion for paced and direct MCP edits, and external-revision conflicts; [contract](CONSTRUCTION.md) | Qualify native window/input behavior and add audio-application feedback. Headless tests cover lifecycle, conflicts, MCP dispatch, rendered controls and cable interaction; committed state is explicitly distinct from audio-applied state. |
| Live instrument authoring | Missing: clients can assemble existing kinds, not define new DSP modules | Validate new instrument definitions, bound execution and memory, isolate failures, preserve provenance and versioned state, and make installation/replacement undoable without interrupting unrelated voices. |
| Subscriptions and diagnostics | Missing | Bounded revision/transport/meter/solver-event subscriptions, backpressure and reconnect/resync tests; no JSON or network work in DSP callbacks. |
| Render/analysis jobs | Partial: synchronous offline render and benchmark | Cancellable background jobs, progress, reproducible patch/seed/configuration records, and artifact results; GUI and other control requests remain responsive. |
| Musical operations | Partial: graph tools, shared undo/redo, grouped parameter edits, atomic sequence rotation and live/offline module actions | Sample-timed automation, audition, transport control and reusable subpatch operations share the same validation/history as human edits. |
| Objective evaluation | Partial: numerical tests, per-channel statistics, peak-preserving MCP offline waveform capture and live rack Scope extrema | Agents can request calibrated spectra, pitch, clipping, latency and CPU diagnostics from explicit taps; tests use known signals and declared error bounds. |
| Native visual instruments | Partial: GLSL dock/editor, calibrated two-source dock, and rack Scope with polyphonic X/Y, voltage-range controls, extrema/trigger readouts and active-graph live inspection; [contract](RACK_SCOPE.md) | Arbitrary-port and per-voice taps, calibrated scope/spectrum, saved shader/preset state and synchronized controls; GPU failures preserve the instrument and last valid shader. |
| Better performance at equal quality | Unproven | Fixed workload/quality comparisons, reproducible CPU and callback-tail measurements, memory and power observations; publish improvements and regressions separately. |

Local controls, remote commands and project loading should use one command/history layer. A future control service should not depend on the GUI repaint loop to acknowledge whether the audio engine has applied a revision. Distinguish committed revision from audio-applied revision in diagnostics.

## Product direction: build instruments alongside the musician

The 2026-10-14 product discussion reinforces the parity baseline while adding
three explicit design gates. These are intended capabilities, not completion
claims:

- **Computer-native editing:** editable values, meaningful sliders, direct
  envelope/response editing and readable modulation. Evaluate task completion,
  fine adjustment, keyboard access and matching rack/browser presentation;
  hardware resemblance alone is not an acceptance criterion.
- **Acoustic physical modeling:** separate excitation, resonance, coupling and
  damping from analog circuit emulation. The first [struck resonator](MODAL_RESONATOR.md) implements a linear
  string / beam with sixteen modes, explicit impulse/force excitation and a
  velocity pickup. The shared panel previews its configured modes; live motion,
  coupled bodies, nonlinear contact and measured hardware calibration remain
  open. Retained-mode analytical validation is distinct from full-body accuracy. See Julius O.
  Smith's [Physical Audio Signal Processing](https://ccrma.stanford.edu/~jos/pasp/).
- **Shared audio/visual instruments:** connect CV to visual parameters and show
  how changes affect both sound and image. Keep rendering outside the audio
  callback and measure graphics load alongside audio deadlines.

Visible construction and live DSP authoring retain their separate acceptance
gates above. The musician must be able to interrupt and undo edits; cable
animation must follow real committed edits, and audio-applied state must remain
observable. Performance superiority requires comparable measured workloads,
not an inference from implementation language.

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

Advance by evidence, not module count alone. These are acceptance criteria;
implementation follows the delivery order above:

1. **Engine foundation:** implemented named multi-output/polyphonic/timed routing and backward-compatible files pass the independent contract tests; remaining performance, broader state/event and native/device qualification gates are listed above.
2. **Editing and synthesis:** shared history, reliable projects, stereo playback and a playable polyphonic voice pass native macOS tests; non-MIDI module contracts and core concurrency cases are exercised.
3. **Creative tools and compatibility:** delivered MCP/visual/import/extension capabilities have end-to-end tests and explicit supported-version boundaries; quality/performance claims cite matched experiments.
4. **Final MIDI/live-performance phase:** mapping and controller authority, remaining MIDI interfaces, stage workflows and expanded device I/O pass timing, persistence, recovery and native/hardware qualification.
5. **Stock Free and host parity sign-off:** all 49 inventory rows and every host-matrix gate have test artifacts and supported-platform results. Remaining behavior, plugin and file gaps stay explicit. Pro and third-party work retain their separate matrices.

Each milestone should link its patch fixtures, reference version, test results, known differences and representative recordings/screenshots. Refresh this baseline when upstream functionality or OpenPatch's contracts change.

The stock-style [ADSR EG](ADSR_EG.md) adds four CV depths, retrigger, PUSH and real stage lamps without changing the original RC `adsr` kind. Its pinned DSP comparison is separate from macOS device/window qualification and hardware calibration. Stock VCF and VCA have since added their separate interfaces and bounded comparisons; remaining MIDI work follows the final delivery phase above.


## MIDI CC: bounded DSP comparison and remaining host qualification

The separate version-2 `midi_cc` kind adds sixteen output cells with exact CC
assignment entry, explicit Learn/Cancel, smoothing, channel filtering, MPE and
paired 14-bit controllers. Learn follows actual audio decisions and creates a
mapping-only history operation; controller motion preserves current values
without adding graph revisions or undo entries. Saved state captures all 128
committed channel-zero values. The existing `midi_cv` behavior is unchanged.

The [pinned actual-class comparison](benchmarks/midi-cc-reference.json) records
45 passing cases at 44.1, 48 and 96 kHz: 221,184 frames and 36,284,064 active lane
samples, zero measured voltage error, and exact mappings, committed values,
Learn completion frames and output widths. Its artifact identifies source,
input, driver and toolchain hashes. See the [fixed contract](MIDI_CC.md) and
[reference method](../scripts/midi_cc_reference/README.md). This qualifies the
recorded DSP scope and platform; it does not prove native Mac input/window,
hardware MIDI, audio-device behavior, or complete Core/host parity. Hardware
endpoint selection is global, and the nonstandard signed gamepad extension is
excluded.


## MIDI Gate: functional coverage and explicit state behavior

The version-2 `midi_gate` kind adds sixteen note assignments, minimum-duration
note gates, velocity voltage, MPE, channel filtering, Learn/Cancel and an explicit
module Panic. CC and Gate share one original 4×4 panel/precision-entry renderer
and the kind-tagged durable Learn transport. MIDI Gate brought the catalog to 60 kinds;
this does not complete the Core inventory.

The source's state belongs to output cells. A mapping edit, Learn, or mapping
Undo preserves that cell's gates/timers/velocities; an old note-off may no longer
find a held cell after reassignment. Panic clears gates/timers and cancels armed
Learn. Completed durable decisions still reconcile. Same-sample NoteOn/Off emits
a minimum pulse: the exact f32 timer produces 45, 48 and 97 samples at
44.1, 48 and 96 kHz, respectively. Velocity mode scales the active voltage to
velocity/127 × 10 V. Mono merges accepted channels with last-event semantics;
MPE preserves sixteen separate lanes. Only mappings/options persist.

The [actual-class report](benchmarks/midi-gate-reference.json) passes 63 cases
(21 fixtures × 44.1/48/96 kHz), 258,048 frames and 56,513,088 active lane samples.
Binary voltage traces are bit-identical with zero voltage error and no mapping,
Learn, width or edge mismatches. This is the recorded pinned DSP/SDK queue/source
JSON comparison, with an explicit driver channel rule and every output probed;
it does not qualify disconnected SDK port metadata or native hardware behavior.
The [locked Gate contract](MIDI_GATE.md) keeps host transport/Session/UI tests
separate. Native Mac window/audio, physical MIDI, endpoint timing/reconnect and
broader platform acceptance remain open.
The selected hardware input is global rather than per module. The connected
[MIDI drum pads](../examples/midi-gate-drums.json) example and
[two-second note trace](../examples/midi-gate-phrase.events.json) exercise the
existing global keyboard and offline MCP path without replacing the CC module.


## Notes and Blank: readable, portable instruments

The catalog reached 62 kinds at that milestone. Original `notes` and `blank` close two missing
Core roles with typed module content, revision-checked `set_module_content`,
atomic mixed edits, preset/duplicate/project/recovery persistence and shared
history. Notes has a multiline draft editor with explicit Apply/Cancel and a
retained conflict draft. Blank has left/right HP-snapped preview and direct width
entry; geometry-aware GUI resize blocks collisions and commits width plus left
position as one Undo boundary. Width belongs to module content, so unseen MCP
width revisions restore from graph history rather than a renderer cache.

[The annotation contract](ANNOTATIONS.md) specifies limits and remaining gates.
Direct MCP/preset width changes retain the left position; the Session does not
own rack positions and cannot enforce layout collision constraints. Existing
observed-state position-history limitations and native macOS IME/clipboard/
interaction qualification remain open. Notes' screen is emissive in lights-out;
Blank and faceplates are passive. Stock VCF and VCA subsequently added their
interfaces and actual-class DSP qualification. MIDI Map is deferred to the final
MIDI/live-performance phase.


## Stock VCF milestone

A separate `stock_vcf` supplies six controls, four inputs and LP/HP outputs
without changing the existing circuit ladder. The [frozen contract](STOCK_VCF.md)
requires genuine noise excitation in the actual-source oracle, per-output
reconnect histories, rounded polyphonic groups, real graph consumers and no
callback allocation. The original shared panel supplies Hz entry and signed
CV depths; the connected voice pairs it with ADSR EG and Scope. This advances
one stock signal role, not complete Rack/plugin/host parity.


## Stock VCA milestone

A separate `stock_vca` adds ideal polyphonic amplification, direct percentage
level entry, linear/fourth-power response controls and per-channel effective
gain columns. **File → Stock VCA voice** connects oscillator, Stock VCF, ADSR EG,
amplifier and a before/after Scope. Silent audio can still show nonzero gain;
bypass preserves raw signal width and freezes the meter. The catalog now has
64 kinds; the reference OTA VCA and legacy `gain` retain their contracts.

The [frozen contract](STOCK_VCA.md) and
[actual-class report](benchmarks/stock-vca-reference.json) record 72 passing
traces at 44.1/48/96 kHz, 55,872 frames and 800,328 active output/gain comparisons.
Maximum absolute error is 9.54e-7 under the unchanged
`2e-6 + 2e-6 * abs(reference)` gate from frame zero, with exact actual SDK channel
metadata and visible gains. Fresh partial-CV traces distinguish both response
modes, and their endpoint gains agree exactly.

Six DSP unit tests, five public Engine tests and one core allocation
guard pass. Fifteen desktop callback/panel tests pass. Actual Linux captures
and platform CI are release gates; native macOS window/input/playback qualification remains
separate outstanding work. This finite digital result is not measured hardware
fidelity, a completed VCV inventory row or release qualification.

## Stock Delay milestone

`stock_delay` brings the catalog to 66 kinds with a separate clock-synchronized
mono delay, filtered feedback, MIX/WET outputs and four signed CV depths.
Its original 24 HP rack/browser faceplate provides direct faders, ms/s entry
and an actual engine CLK lamp. **File → Clocked delay voice** connects the
delay to an envelope/VCA voice and Scope. The original stereo delay retains
its independent contract.

The [frozen numerical contract](STOCK_DELAY.md) pins Fundamental's Delay class
and BSD-2-Clause libsamplerate 0.1.9. Source-defined whole-module traces must
retain the 2 mV + 10 ppm gate from frame zero; defined SRC calls require exact
counts and 2e-6 + 2 ppm samples. An undefined upstream ratio-1→0.25 history
read receives native zero-history safety behavior, never a source-fidelity
pass. The final [guarded report](benchmarks/stock-delay-reference.json) passes
all 43 whole-Delay cases over 11,134,750 frames at 44.1/48/96 kHz, with zero
worst absolute error and no source-undefined whole-Delay cases. Six defined
direct SRC scenarios match counts/samples exactly over 8,192 calls each.
The separate analytic source-undefined startup case passes native safety
checks with null fidelity. Production hashes remained stable and were rechecked.

Each instance owns an 8 MiB history plus approximately 49 KB of SRC state.
Live compatible handoff swaps runtime ownership and retires the whole donor
graph with a cached raw-output fade; borrowed adoption and existing StudioFx
can still copy large state. The [release adoption microprofile](benchmarks/stock-delay-handoff.json)
records one/eight-Delay Engine transfer costs without claiming a full callback
deadline or Rack CPU comparison. Actual callback tests check no heap activity.
The Rust checks pass 1,580 tests with six existing ignored tests, strict Clippy
passes, the release workspace builds, and all 79 Python/MCP tests pass.
The macOS ARM bundle's ad-hoc signature and notices are verified; the bundle
has not been installed or launched. [PR #4](https://github.com/dexterlabs1/openpatch/pull/4)
merged after three-platform CI and all 97 native captures passed, including
visual review of the three Delay views. Full callback-budget and macOS ARM window/input/playback
acceptance remain open.

## Wavetable family milestone

`wavetable_vco` and `wavetable_lfo` bring the catalog to **68 kinds**. Custom
tables, position CV, audio FM/sync and modulation clock/reset share immutable
assets and one rack/browser presentation. Workspace import prepares a complete
candidate on a cancellable worker and rechecks authorization/revision before
publication; ordinary inspection avoids large payload echoes. Portable presets
contain their referenced samples and support digest-checked file application.
History keeps durable assets and weak derived tickets, rebuilding cold banks
on the control thread within explicit payload budgets.

The checked actual-class SDK run passes 153 cases over 626,688 frames at
44.1/48/96 kHz. Maximum error is 1.9073486328125e-6 V under the frozen
2 mV + 10 ppm contract, with exact LFO voltages. Audio wave lengths are powers
of two >=32; LFO lengths are integers >=2. Source Nyquist/rounded-endpoint
compatibility and the independently tested safe through-zero extension have
explicit boundaries in the [contract](WAVETABLES.md). The
[production-hash proof](benchmarks/wavetable-reference.json) and local Mac ARM
release build pass. [PR #5](https://github.com/dexterlabs1/openpatch/pull/5)
merged after three-platform CI passed (81 Python/MCP tests per platform),
all 100 native captures passed integrity checks and six normal/dark/browser/compact
scenes were visually reviewed. Native Mac window/input/device qualification
remains open.
Neither this digital result nor local host tests qualify hardware behavior,
an installed native app or complete Rack host/plugin parity. The full 49-module
and host objective, with MIDI/live-performance work last, is unchanged.

## Random modulation milestone

`random` brings the catalog to **69 native kinds** with a separate, shared-clock
modulation instrument: probability/spread, four interpolation outputs, external
polyphonic targets and a mono accepted-event trigger. A native seed makes fresh
renders reproducible without claiming Rack global-PRNG sequence identity.
The checked actual-class run passes all 75 cases and 484,548 frames across
44.1/48/96 kHz, with zero measured voltage/state error under the frozen gate.
All 225 retained trace artifacts and 74 source hashes verify. Local host/heap
and 82 MCP tests, strict Clippy and a separate Mac ARM release build pass.
Platform CI/capture review and native interaction remain separate gates in
the [Random contract](RANDOM.md). This does not complete
all 49 stock module/host targets. MIDI/live-performance work remains last.
