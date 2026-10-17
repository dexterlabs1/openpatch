# Current work plan

Updated 2026-10-16. This is the compact execution plan; [ROADMAP.md](ROADMAP.md) and [MODULE_COVERAGE.md](MODULE_COVERAGE.md) retain the full objective and parity inventory.

## Stable release

**OpenPatch 0.2.5 build 7 stays installed.** It includes the Signal Atlas knob/socket rollout, Ferrofluid and the Studio Mixer with FX sends. PR25 merged at `ab3b2a280e9affdd55015ba5fd0a4664a32df6aa`. Do not create another release for routine development changes. User priority is macOS ARM; Linux/Windows validation can wait. MIDI/live performance remain last.

## Execution changes

The recent process duplicated feature-push/PR CI, required two full capture reviews, ran benchmarks across every platform for routine UI edits, copied large manifests into chat and fragmented related work across small releases. These consume time without a matching product benefit.

Now use one milestone branch, focused local checks, one PR CI run and one appropriate review. Default CI targets macOS ARM, with Rust build/dependency caching and cancellation of superseded runs. Full Linux/Windows validation and benchmarks are available through the manual `full_validation` input. Documentation-only changes skip application CI. The installed app is independent of CI artifacts.

## Active milestone: extend the approved native design

The user rejected the material/control rollout as too far from the [01 Signal Atlas reference](../design/concepts/module-panels-20261016-v1/01-signal-atlas.png). The user approved the two-panel preview (“Looks good”) and resumed work with “continue”. Extend that native design across the catalog in coherent families, retaining the finished ADSR/Ladder panels as the reference.

The [interactive design review](../design/reviews/signal-atlas-two-modules-v1/index.html) contains analog ADSR and Ladder VCF studies. They use the existing modules' controls, ranges and ports, large tactile knobs, functional stage/feedback diagrams, metal sockets and identical scaled thumbnails. The HTML/SVG artifact records the approved direction; native implementation lives in `atlas_voice_panel.rs` and uses the real shared rack/browser renderer. Configured plots are explicitly separate from live audio. No DSP code changed.

- **Native scope:** tactile knobs, warm material, metal sockets, diagrams, static configured RC/linearized filter plots, scaled thumbnails, existing drag/fine/keyboard/numeric/context editing, undo and MCP motion anchors. Existing panel widths and port identities remain stable; the three sockets now share the approved bottom row.
- **Validation:** actual Session gestures and numeric entry for all seven controls, cable target/label clearance, rack/browser geometry, response-guide contracts, native mesh previews and macOS desktop regression. No native window or physical audio-device qualification is claimed.
- **Verified:** 1,081 macOS ARM desktop tests passed, 4 ignored; strict desktop Clippy and formatting passed. Native light/dark mesh captures were inspected. [Native panels and thumbnails](../design/reviews/signal-atlas-two-modules-v1/native-preview.png).
- **Delivery:** keep one development PR and publish useful family checkpoints. Keep 0.2.5 build 7 installed during development; avoid new releases and repeated full CI runs for individual layout edits.

### Analog voice continuation

The capacitor-ramp VCO and OTA VCA now extend the same native material, hardware and functional diagrams. The VCO shows its nominal ±5 V shape, cycle period, exponential pitch/linear FM path and a pulse-only width control. The VCA shows a configured static saturation curve at CV 10 V, with the 0–10 V CV clamp and unpatched 10 V normal stated on the panel. These are configured model guides, not live waveforms. Existing HP widths, identities, numeric edits, choices, gesture undo and cable/MCP behavior are retained.

[Four-module native voice and browser thumbnails](../design/reviews/signal-atlas-two-modules-v1/native-analog-voice.png). **56 focused checks passed**, covering the updated gestures/units, choices, jack targets, text/geometry, rack/browser identity and OTA DC guide against the actual engine. Strict desktop Clippy and formatting passed. No DSP sources or frozen references changed. Next: modulation/filter utilities, then stock voice/envelope panels, before dense effect/sequencer/mixer families. Other catalog panels retain intermediate styling until their individual pass.

### Modulation and utility continuation

Sine LFO, Low-pass and Gain now use the detailed native treatment, bringing the completed layout family to seven panels. The LFO explicitly fits weak configured cycles and retains its zero-Hz hold and original logarithmic travel through zero. The one-pole plot uses the actual discrete coefficient; Gain shows its signed transfer with CV normalled to 1. Diagrams and markings describe the existing processors. The former VCA utility is labeled GAIN and remains searchable as VCA.

[Native utility panels and matching thumbnails](../design/reviews/signal-atlas-two-modules-v1/native-modulation-utilities.png). **82 focused checks passed**, including actual Session edits/undo, numeric entry, keyboard zero-Hz recovery, socket picking, text clearance, library search, shared rack/browser rendering and configured guides compared with the engine. Strict desktop Clippy, formatting and light/dark mesh review passed. The preceding `703c26b` analog-voice checkpoint passed GitHub CI. No DSP sources, frozen references or installed app changed. Next: Stock VCO and ADSR EG, preserving real phase/stage telemetry and PUSH behavior.

The following catalog batches describe existing development source, not an approved final design.

### First catalog batch in progress

Branch `work/signal-atlas-catalog` extends rotary controls and larger sockets to Sine LFO, Low-pass, Gain, OTA VCA, analog ADSR and State Variable Filter. ADSR EG receives four main rotaries and four smaller bipolar CV-depth rotaries. Its range legends describe real time constants, sustain voltage and modulation depth; stage lamps remain engine-driven. All catalog faces now share the approved warm ivory material through the rack/browser renderer.

Focused checks cover successive real-Session drag edits with one undo, enlarged jack identity/caption clearance, envelope precision entry/lights, and identical rack/browser control geometry. An opt-in CPU panel preview renders actual egui meshes without opening a window or audio device. This batch is development source, not an installed release. Remaining dense effect, sequencer, mixer and utility layouts still need individual control/spacing passes; the shared faceplate change alone does not complete their redesign.

### Effects and macro voice batch

Stock Delay, Stereo Delay, Space, Duck, Drum and Macro Voice now have rotary control layouts. Dense effects use compact three-row layouts; the reverb/drum use two larger rows. Macro Voice has separate numeric wells for all six continuous controls. Stock Delay keeps its time, signed tone and CV-depth units, precision entry, real clock lamp and construction-motion cues.

Effect output sockets move from y382 to y374 to fit the larger rims inside the bottom rail; all cable port identities and panel widths remain stable. Stock Delay retains its original y380 anchors with a bounded 20% socket enlargement; other new sockets use 30%. Captions clear neighboring sockets and controls at reduced zoom. The actual shared renderer supplies both rack and thumbnail views.

The full reference also calls for more tactile hardware detail and functional panel graphics. The current material/control rollout is an intermediate step; remaining work must address those details with source-backed markings rather than arbitrary diagrams.

### Resonator, utilities and shared hardware

The physical resonator now uses eight rotary controls alongside its configured modal/decay plot and strike/pickup guide. It retains geometric frequency/RT60 adjustment, exact entry, fine/keyboard adjustment, grouped undo and construction cues. Its printed frequency range reflects the effective sample-rate limit; the configured plot is explicitly separate from live signal telemetry.

Crossfade, both quantizers, Sum, both mid/side processors and both six-input mixers now use rotary parameter controls with larger sockets. Quantizer keyboards, mixer flags, mid/side routing and measured lights/meters keep their original behavior. Knob grip flutes and socket hex-nut/bevel details stay within existing bounds; subpixel grip detail is omitted in small thumbnails. The same renderer supplies rack and browser views.

Focused checks cover real Session edits/undo, new jack caption clearance, matching control geometry, quantizer/mixer/mid-side workflows, the resonator's physical units/rate-limited range and static configured plot. Full-size and thumbnail CPU renders are reviewed without opening a native window or audio device. Remaining sequencer and legacy control layouts still need their design pass before the milestone candidate; installed 0.2.5 remains unchanged.

## Checkpointed: Stock Scope integration

Stock Scope implementation is checkpointed while the design milestone proceeds. Its branch/worktree is `work/stock-scope-integration` / `openpatch-stock-scope-integration`.

- **Proven:** the optimized actual Core probe matched all 4,242 frozen genuine-Rack snapshots across 1,056 cases, including full 50-second sweeps at three rates. The reference declaration remains unchanged. This establishes the bounded source/host numerical result, not complete application or hardware qualification.
- **Written, not yet qualified:** the shared stock panel, native AUTO view, real callback/MCP transport, seven Core lifecycle/realtime tests, panel tests and separate 46-record safety probe.
- **Known next fixes:** two new test comparisons must use `Cable.id.as_deref()` because IDs are optional; the Stock Scope TIME drag must use the current observed Session revision on successive moves, retaining opening revision only for text-entry stale-editor rejection.
- **Next verification:** check those focused targets, run the existing safety probe, verify actual Callback → Session inspection and affected stock panel interactions, then one complete macOS regression. Reuse the unchanged numerical evidence unless relevant production DSP changes.
- **Done:** source controls/lifecycle remain truthful; the stock panel, weak-signal AUTO, cable interactions and live inspection work together; affected focused tests and one macOS regression pass. Then review one milestone PR. Package/install only at a useful release milestone, with the user's session preserved.

The authoritative local evidence is `native-test-output/stock-scope/STATE.json` in the integration worktree. Full logs/traces stay outside chat and Git. Existing `scope` and user patches keep their separate behavior.

## Course to completion

One focused Astra advisory reviewed the remaining scope and this execution policy. The user's subsequent priority puts the approved module redesign first. Resume the checkpointed Scope integration afterward, then prioritize making music with real imported patches. A blocking regression in the current instrument takes priority immediately.

| Milestone | User-visible exit |
|---|---|
| 1. Signal Atlas module design | Complete the approved catalog rollout with larger knobs/ports, meaningful markings, readable titles and identical rack/browser rendering. Review representative panels early; qualify one Mac release candidate. |
| 2. Scope integration | Connect, adjust and inspect Stock Scope reliably, including successive TIME drags, native AUTO view and real Callback → MCP publication. Focused safety/lifecycle/realtime and one Mac regression pass. |
| 3. Real Rack patch workflows | Open, repair, play, save and reopen a declared collection of actual `.vcv` patches with routing, parameters, layout and assets preserved. Unsupported modules remain explicit, recoverable placeholders. Publish the supported mapping. |
| 4. Complete non-MIDI Mac instrument | Close module gaps in coherent families, including hidden VCA-2/Unity compatibility; complete editing/history, project recovery, audio configuration and Audio 2/8/16 requirements. Qualify shared physical audio paths once at the host milestone. |
| 5. Demonstrated performance/reliability | Comparable representative small, polyphonic, feedback, delay-heavy and large patch measurements support specific CPU/latency claims. Fix measured bottlenecks; verify callback deadlines, live graph changes, overload, installation/upgrade and recovery. |
| 6. Distinctive creative workflow | MCP completes useful musical editing/project tasks with visible changes, coherent undo and conflict recovery; GLSL visuals persist assets and contain failures during audio use. General-purpose live DSP authoring stays a separate later feature. |
| 7. Deferred portability, then MIDI/live | When authorized, qualify Linux/Windows against the finished baseline. Complete remaining MIDI conversion, mapping, timing, reconnect and stuck-note recovery last. These gates remain open until actually verified. |

Keep frozen numerical contracts, state/event and realtime invariants, persistence/migration, representative native interactions and actual device qualification. Add cases for concrete uncovered behavior; do not multiply every parameter, platform, sample rate and gesture into an exhaustive Cartesian matrix. Numerical agreement alone is not application completion.

The immediate architecture decision is the [compatibility boundary](COMPATIBILITY_DECISION.md): stable native module identities, versioned state and recoverable source payloads are shared foundations for import and extensions. Existing VCV binary hosting is a distinct unresolved deliverable; a Rust extension API does not silently satisfy it.

The final goal still requires the pinned 49-module baseline, tracked legacy compatibility, the host matrix, supported platforms and bounded improvement claims. Until deferred requirements pass, call intermediate releases macOS/non-MIDI milestones, not full Rack parity. Third-party coverage is a versioned support matrix, not an unfinishable promise to support every future plugin.
