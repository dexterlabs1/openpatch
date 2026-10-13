# Roadmap: Rack functionality, then measurable improvements

Baseline checked **2026-10-12**. This is an acceptance plan, not a completed-feature list. OpenPatch is currently a native Rust instrument prototype with 18 built-in module kinds, a tested reference analog voice, rack editing, and shared-session stdio MCP. It has not reached VCV Rack parity.

## Define what “parity” includes

The first target is **VCV Rack 2 Free standalone host functionality plus its 49 Core/Free modules**. The [Rack product page](https://vcvrack.com/Rack) lists Free as the standalone edition for macOS, Windows, and Linux. DAW plugin operation belongs to the Pro follow-on. Hosting other VST plugins through the separately purchased VCV Host product is another distinct scope.

The bounded stock inventory is 12 Core modules and 37 Free modules, individually tracked in [MODULE_COVERAGE.md](MODULE_COVERAGE.md), using the [Core library](https://library.vcvrack.com/Core) and [Free manuals](https://vcvrack.com/Free). None should count as complete merely because an OpenPatch module has a similar name or musical role. The seven legacy utilities, six reference-voice kinds and five functional utilities do not constitute 18 completed VCV equivalents.

The third-party ecosystem is an ongoing compatibility program, not a finite “all modules” milestone. A Rust extension API, a Rust implementation of selected module behavior, a `.vcv` importer, and a loader for existing VCV C++ binaries are four different deliverables. Each must state which plugins and versions it supports. Neither a generic filter nor a circuit-derived reference model establishes the behavior of a particular commercial module.

## Architecture audit

This plan starts from the code inspected on the baseline date:

- `openpatch-core`: version-1 JSON patches; one unnamed source output per node; at most two compiled inputs; mono buffers; deterministic dependency ordering; all cycles rejected. Source dependencies are evaluated within the same sample, without a cable delay. The limits are 256 nodes and 1024 cables.
- Reference DSP: voltage-based VCO/ladder/OTA VCA/RC ADSR/gate/audio conversion, with documented idealizations and numerical tests. Ladder and VCA each add 64 samples of oversampling latency; parallel paths are not compensated. Legacy patches retain normalized semantics. See [ANALOG_MODELS.md](ANALOG_MODELS.md).
- Functional utilities: quantizer, sample-and-hold, seeded Gaussian white noise, attenuverter and crossfade. These implement declared mono behavior; full corresponding VCV port and polyphony contracts remain open.
- Native host: rack browser, module movement, rotary controls, cable creation/removal, workspace save/load with layout sidecars, shared multi-step patch undo/redo, and optional GLSL visuals. Consecutive parameter edits can share an explicit gesture group. Stable history identities restore layouts for states observed by the GUI, with at most 129 retained layout snapshots. Layout movement is not yet an undoable project action. Default-device mono output is duplicated across device channels; there is no hardware audio input or MIDI integration.
- Control: UI and MCP share revision-checked transactions and bounded history. `preview_edits` validates a candidate without committing; `apply_edits` validates the final graph and commits up to 256 edit operations as one revision/history action. History retains at most 128 prior/future patch snapshots combined. Rendering/export clone the active patch without cloning history. Audio swaps compiled graphs through bounded queues and carries matching DSP state forward. Stdio starts with one application instance; it cannot attach to an arbitrary already-running window.
- Validation: repository tests and platform CI exist. Those do not establish an exhaustive hardware/device qualification matrix or a measured performance advantage over Rack. See [PERFORMANCE.md](PERFORMANCE.md).

“Partial” below means useful behavior exists but its acceptance gate is open. “Missing” means the capability is not implemented. Gates describe proposed OpenPatch tests, not claims about current results.

## Free host parity matrix

| Area | Current status | Acceptance gate |
|---|---|---|
| Named ports and multiple outputs | Missing; only destination inputs are named | A versioned patch can independently route every declared source output; validate names, direction, channel limits and normaling; preserve old files and render fixtures. |
| Polyphonic engine | Missing; mono only | Per-port 0–16 channels; primary-input voice count, mono broadcast and missing-channel policies tested; add/remove voices without allocation, cross-talk or stale samples. |
| Cable timing and feedback | Missing; same-sample DAG | Versioned Rack-style timing with one sample per cable; self-loops and multi-node loops have deterministic impulse responses independent of node order and callback partition. No implicit block-sized feedback delay. |
| Voltage and event contracts | Partial; reference modules use volts | Catalog every port's units, normaling, thresholds and channel policy; test gate/trigger edges, clock/reset ordering, pitch calibration and legacy conversions. |
| DSP state and parameters | Partial; state adoption plus crossfade | Preserve per-voice state through graph edits; define sample-timed parameter events and reset behavior; test edits during oscillation, release, delay tails and feedback. |
| Engine rate, diagnostics and scheduling | Partial; device-rate compilation, offline benchmark | User-selected engine/device rates, rate conversion, per-module timing and overload reporting; concurrency and SIMD changes must preserve the timing contract. |
| Stock module coverage | Partial; 49-entry inventory remains open | Every inventory row has complete port/parameter/state behavior, persistence and functional tests; publish differences rather than claiming equivalence for a substitute. |
| Rack construction | Partial | Module selection/duplication, presets, initialization/randomization, bypass, grouping and placement workflows have interaction and persistence tests. |
| Parameter/cable interaction | Partial | Precise entry, fine control, reset, reconnect/delete/fan-out, keyboard shortcuts and cable presentation work together; no swallowed gestures or accidental edits behind dialogs. |
| History | Partial; shared 128-step patch undo/redo, explicit parameter grouping and observed-state layout restoration implemented | Complete project history including layout movement; verify restored geometry, gesture boundaries, branching and concurrent MCP edits without lost updates, including states the GUI has not observed. |
| Browser/navigation | Partial | Search, filters/favorites, keyboard access, contextual help, zoom/pan and large-rack navigation; selections and newly created modules remain visible. |
| Project lifecycle | Partial; new-name-only JSON and sidecar | Save/update/Save As, revert, templates, autosave/recovery, asset handling and module presets; simulate interruption during writes and recover a coherent project. |
| Hardware audio | Partial; default mono output | Select backend/device/rate/buffer; stereo and 8/16-channel input/output; loopback channel-isolation and latency checks; hot unplug/reconnect without callback blockage. |
| MIDI and keyboard performance | Missing | Input/output notes, gates, CC, pitch bend, pressure, clock/transport, learn/mapping, poly allocation, and keyboard performance; timestamp/order tests and stuck-note recovery. |
| Plugin extensibility | Missing | Versioned Rust module API/manifests, state migration, install/discovery/update and missing-module recovery; publish compatibility boundaries. Existing VCV binary support remains a separate unfulfilled gate. |
| Existing Rack projects | Missing | Supported `.vcv` imports preserve routings, parameters and layout; unknown modules/assets remain explicit and recoverable; publish a version/module compatibility matrix. |
| Platform release quality | Partial; macOS ARM first | Repeatable macOS ARM builds and actual input/audio/UI tests, then Linux/Windows device qualification; clean-machine install, upgrade, recovery and accessibility checks. |

The engine targets derive from VCV's [voltage/timing contract](https://vcvrack.com/manual/VoltageStandards) and [polyphony documentation](https://vcvrack.com/manual/Polyphony). In particular, 0-channel ports behave as unpatched, mono signals are a special case of polyphony, and Rack's documented cable delay matters for feedback and event ordering. Our version-1 scheduler must not silently change to those semantics.

Host workflow targets are grounded in VCV's [menu reference](https://vcvrack.com/manual/MenuBar), [key commands](https://vcvrack.com/manual/KeyCommands), and [getting-started workflow](https://vcvrack.com/manual/GettingStarted). Rack includes persistent patch operations, history, configurable engine/view behavior, direct module/parameter interaction, and cable editing. Equivalent task outcomes matter; identical artwork is not required. Audio and MIDI scope follows the [Core manual](https://vcvrack.com/manual/Core).

## Next architecture step: versioned ports and timing

Implement the port/graph contract before expanding the module catalog substantially. Otherwise every multi-output oscillator, stereo utility, polyphonic module and MIDI interface will require another incompatible graph redesign.

1. **Introduce a version-2 wire schema.** Represent both cable endpoints as stable module ID plus port ID. Declare port metadata, normaling, channel policy and output channel counts. Keep version-1 parsing/rendering fixtures intact; migration assigns its implicit output explicitly and retains its original timing mode.
2. **Separate control data from compiled signal storage.** Resolve strings and allocate port buffers, voice states and lookup tables on the control thread. Runtime uses dense port indices and fixed-capacity 16-channel storage with an active-channel count. Change that count without allocating or leaving old voice values audible.
3. **Add a simple, correct timing reference.** For Rack-style mode, every sample reads connected values from a completed previous-sample bank, computes current outputs, then publishes them together. This permits arbitrary loops with defined edge delays. Validate it before introducing block/SCC scheduling optimizations; adding a single delay only to a feedback edge is not equivalent to delaying every cable.
4. **Carry routing history as well as module history.** Preserve compatible per-port delay values and voice states across live edits. Specify zeroing for newly connected ports, voice retirement, sample-rate changes and intentional reset. Keep crossfades an explicit host policy rather than a claimed circuit property.
5. **Ship one end-to-end proof.** A multi-output source drives distinct destinations; a 16-channel source exercises split/merge; a delayed feedback loop produces a known recurrence. Test direct/indirect clock paths, zero/one/sixteen channels, arbitrary callback lengths, old-file regression, and callback allocation/deallocation. Expose port metadata and routing through both GUI and MCP.

This is the next implementation recommendation, not an implemented engine description. Profile the reference scheduler before replacing it with a faster one. The existing expensive circuit voice is not evidence that sixteen such voices will meet a live deadline.

### Proposed API boundary for the next engine pass

Keep deserialization/version migration separate from the compiled engine. Add an explicit `PatchV2` wire type instead of silently interpreting new fields in version 1. Its cables use `source: {node, port}` and `destination: {node, port}`; its selected output also identifies a port. Stable string IDs belong in patch files and control APIs; dense `NodeIndex`/`PortIndex` values belong only in compiled storage. The catalog should expose ordered input/output descriptors and their channel/normaling policy.

Require an explicit version-2 timing mode: `legacy_dag` preserves migrated version-1 evaluation, while `rack_delayed` uses the completed-previous-sample bank described above. The latter may accept cycles; the former must retain cycle rejection. Migration maps today's implicit output to `out` without changing the module kind, voltage convention or selected waveform. A source-port rename needs a module-version migration, not an index-based guess.

Define a runtime port frame as fixed `[f32; 16]` storage plus a validated 0–16 channel count. Delay the count together with its samples in Rack timing mode, and clear newly inactive voices. Compile per-module voice policy rather than imposing mono broadcast on every input. Expose a channel-aware render API for new device sinks while retaining `process_block(&mut [f32])` as the legacy mono adapter. Do not add device handles, JSON values or dynamic allocation to module processing.

The first change can introduce schema/types, validation and compatibility fixtures without replacing the scheduler in the same commit. Follow with named outputs, then the reference timed/polyphonic engine. Review serialization, history snapshots and MCP endpoint schemas together so one layer cannot refer to a port the others cannot represent.

## MCP and visuals beyond the baseline

These are OpenPatch differentiators and do not substitute for missing host or module functionality.

| Capability | Current status | Acceptance gate |
|---|---|---|
| Machine-readable instrument contract | Partial: catalog and patch inspection | Versioned schemas include named ports, units, polyphony, model class, latency and bounded resource costs; no UI-only meaning required to construct a valid patch. |
| Atomic multi-edit transactions | Implemented: candidate preview and atomic apply, final validation, one revision/history action | Structured change summary included; keep rejection/rollback and stale-revision tests; preview/apply clients must carry the preview revision into commit. |
| Agent discovery and attachment | Missing | Attach to an existing chosen desktop session, with session identity and explicit transport lifetime; demonstrate simultaneous UI/two-client edits without lost updates. |
| Subscriptions and diagnostics | Missing | Bounded revision/transport/meter/solver-event subscriptions, backpressure and reconnect/resync tests; no JSON or network work in DSP callbacks. |
| Render/analysis jobs | Partial: synchronous offline render and benchmark | Cancellable background jobs, progress, reproducible patch/seed/configuration records, and artifact results; GUI and other control requests remain responsive. |
| Musical operations | Partial: graph tools, shared undo/redo and grouped parameter edits | Sample-timed automation, audition, transport control and reusable subpatch operations share the same validation/history as human edits. |
| Objective evaluation | Partial: numerical tests and peak/RMS | Agents can request calibrated spectra, pitch, clipping, latency and CPU diagnostics from explicit taps; tests use known signals and declared error bounds. |
| Native visual instruments | Partial: optional GLSL dock/editor | Arbitrary-port and per-voice taps, calibrated scope/spectrum, saved shader/preset state and synchronized controls; GPU failures preserve the instrument and last valid shader. |
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

1. **Engine foundation:** named multi-output/polyphonic/timed routing and backward-compatible files pass the reference tests above.
2. **Useful instrument:** shared history, reliable projects, stereo audio/MIDI and a playable polyphonic voice pass native macOS tests; core concurrency and device-failure cases are exercised.
3. **Stock Free coverage:** all 49 inventory rows meet their contracts; remaining behavioral differences and host/plugin compatibility gaps are listed openly.
4. **Free host parity:** every host-matrix gate has a test artifact and supported-platform result. An unsupported plugin/file workflow remains a gap, not an implied success.
5. **Beyond parity:** MCP/visual features have end-to-end tests and measurable benefits; quality/performance claims cite matched experiments. Pro and third-party work retain their separate matrices.

Each milestone should link its patch fixtures, reference version, test results, known differences and representative recordings/screenshots. Refresh this baseline when upstream functionality or OpenPatch's contracts change.
