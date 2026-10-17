# Current work plan

Updated 2026-10-16. This is the compact execution plan; [ROADMAP.md](ROADMAP.md) and [MODULE_COVERAGE.md](MODULE_COVERAGE.md) retain the full objective and parity inventory.

## Stable release

**OpenPatch 0.2.6 build 8 is installed at `~/Applications/OpenPatch.app`.** The user explicitly requested this upgrade. It includes the 34-panel detailed Signal Atlas pass, stronger lights-off glow, value bubbles, undo/toolbar fixes, Ferrofluid and Studio Mixer. The optimized ARM64 desktop/CLI bundle is locally ad-hoc signed, with installed hashes/signature verified; 0.2.5 build 7 was removed. Keep this build stable during continued development. User priority is macOS ARM; Linux/Windows validation can wait. MIDI/live performance remain last. No native-window/audio-device qualification is claimed.

## Execution changes

The recent process duplicated feature-push/PR CI, required two full capture reviews, ran benchmarks across every platform for routine UI edits, copied large manifests into chat and fragmented related work across small releases. These consume time without a matching product benefit.

Now use one milestone branch, focused local checks, one PR CI run and one appropriate review. Default CI targets macOS ARM, with Rust build/dependency caching and cancellation of superseded runs. Full Linux/Windows validation and benchmarks are available through the manual `full_validation` input. Documentation-only changes skip application CI. The installed app is independent of CI artifacts.

### Lights-off refinement

Active LEDs, ports and meters now cast wider, stronger additive halos as ambient light falls; the daytime lamp profile stays restrained. Detailed-panel displays gain a bounded soft green spill, and Studio Mixer position rings have broader additive glow. Static display illumination remains separate from engine-driven lamps; zero or unavailable signal does not create activity.

[Updated dim-room preview](../design/reviews/signal-atlas-two-modules-v1/native-ux-dark-room.png). **32 focused checks passed**, covering lighting, unchanged rack/browser geometry and cable targets, port signals, Studio Mixer behavior and eight oscillator acceptance fixtures corrected after the preceding CI run. Strict desktop Clippy and formatting passed. Actual-engine CPU captures were inspected at full, dim and zero ambient light; no native window/device or installed-release claim. The noise/stochastic design family resumes next.

### Noise and stochastic controls

White Noise, Stock Noise, Colored Noise, Random and Random Values bring the detailed native pass to **34 panels**. White Noise states its Gaussian/RMS path, Stock Noise shows seven actual process roles, and Colored Noise pairs its level knob and exact seed field with the core's configured response curves. Random has eight larger knobs, a clock/probability/target/curve path and a visible full-precision seed button. Random Values shows actual held voltages in illuminated readouts, retaining all sixteen lanes in hover details and guarded PUSH behavior.

[Native family and matching thumbnails](../design/reviews/signal-atlas-two-modules-v1/native-stochastic.png), [dark view from offline engine snapshots](../design/reviews/signal-atlas-two-modules-v1/native-stochastic-dark.png). **54 focused checks passed**, including successive Session gestures/undo for every new rotary, exact values, all stochastic MCP anchors, real lamps, held-state provenance, PUSH lifecycle, shared rack/browser geometry and low-zoom label/target clearance. Strict desktop Clippy and formatting passed. A preceding socket selection also passed 34 checks. No DSP or frozen-reference changes. The lighting checkpoint `9e9a111` passed full Mac and visualizer CI. Next: sample/hold and routing utilities, then dense sequencers.

### Capture and routing continuation

S&H, Shift Register, Mult and both sequential switches extend the detailed native pass to **39 panels**. S&H has a larger initial-voltage knob and its actual trigger/rearm thresholds; the initial setting is explicitly separate from a measured held voltage. Shift Register shows normalled old→next captures and eight actual saved voltages, with guarded PUSH/CLEAR and full-precision seed. Mult shows complete-frame fanout. Switches separate configured length, observed route and de-click tails, with truthful offline/bypass provenance and a shared glowing selected-route lamp.

[Native panels and matching thumbnails](../design/reviews/signal-atlas-two-modules-v1/native-capture-routing.png), [offline-engine snapshots in a dim room](../design/reviews/signal-atlas-two-modules-v1/native-capture-routing-dark.png). **30 focused checks passed**, including actual Session edits/undo, precision units, every named S&H/Mult socket, eight parameter MCP anchors, switch keyboard/guards, saved-value provenance/manual lifecycle and text clearance against enlarged rims down to 25% zoom. Strict desktop Clippy and formatting passed. Socket coordinates, DSP and frozen references remain unchanged. Installed checkpoint `340c77b` passed full Mac and visualizer CI; 0.2.6 build 8 stays unchanged. Next: voltage processors and quantizers, then dense sequencers.

## Active milestone: extend the approved native design

The user rejected the material/control rollout as too far from the [01 Signal Atlas reference](../design/concepts/module-panels-20261016-v1/01-signal-atlas.png). The user approved the two-panel preview (“Looks good”) and resumed work with “continue”. Extend that native design across the catalog in coherent families, retaining the finished ADSR/Ladder panels as the reference.

The [interactive design review](../design/reviews/signal-atlas-two-modules-v1/index.html) contains analog ADSR and Ladder VCF studies. They use the existing modules' controls, ranges and ports, large tactile knobs, functional stage/feedback diagrams, metal sockets and identical scaled thumbnails. The HTML/SVG artifact records the approved direction; native implementation lives in `atlas_voice_panel.rs` and uses the real shared rack/browser renderer. Configured plots are explicitly separate from live audio. No DSP code changed.

- **Native scope:** tactile knobs, warm material, metal sockets, diagrams, static configured RC/linearized filter plots, scaled thumbnails, existing drag/fine/keyboard/numeric/context editing, undo and MCP motion anchors. Existing panel widths and port identities remain stable; the three sockets now share the approved bottom row.
- **Validation:** actual Session gestures and numeric entry for all seven controls, cable target/label clearance, rack/browser geometry, response-guide contracts, native mesh previews and macOS desktop regression. No native window or physical audio-device qualification is claimed.
- **Verified:** 1,081 macOS ARM desktop tests passed, 4 ignored; strict desktop Clippy and formatting passed. Native light/dark mesh captures were inspected. [Native panels and thumbnails](../design/reviews/signal-atlas-two-modules-v1/native-preview.png).
- **Delivery:** keep one development PR and publish useful family checkpoints. Keep the user-requested 0.2.6 build 8 installation stable; avoid new releases and repeated full CI runs for individual layout edits.

### Analog voice continuation

The capacitor-ramp VCO and OTA VCA now extend the same native material, hardware and functional diagrams. The VCO shows its nominal ±5 V shape, cycle period, exponential pitch/linear FM path and a pulse-only width control. The VCA shows a configured static saturation curve at CV 10 V, with the 0–10 V CV clamp and unpatched 10 V normal stated on the panel. These are configured model guides, not live waveforms. Existing HP widths, identities, numeric edits, choices, gesture undo and cable/MCP behavior are retained.

[Four-module native voice and browser thumbnails](../design/reviews/signal-atlas-two-modules-v1/native-analog-voice.png). **56 focused checks passed**, covering the updated gestures/units, choices, jack targets, text/geometry, rack/browser identity and OTA DC guide against the actual engine. Strict desktop Clippy and formatting passed. No DSP sources or frozen references changed. Next: modulation/filter utilities, then stock voice/envelope panels, before dense effect/sequencer/mixer families. Other catalog panels retain intermediate styling until their individual pass.

### Modulation and utility continuation

Sine LFO, Low-pass and Gain now use the detailed native treatment, bringing the completed layout family to seven panels. The LFO explicitly fits weak configured cycles and retains its zero-Hz hold and original logarithmic travel through zero. The one-pole plot uses the actual discrete coefficient; Gain shows its signed transfer with CV normalled to 1. Diagrams and markings describe the existing processors. The former VCA utility is labeled GAIN and remains searchable as VCA.

[Native utility panels and matching thumbnails](../design/reviews/signal-atlas-two-modules-v1/native-modulation-utilities.png). **82 focused checks passed**, including actual Session edits/undo, numeric entry, keyboard zero-Hz recovery, socket picking, text clearance, library search, shared rack/browser rendering and configured guides compared with the engine. Strict desktop Clippy, formatting and light/dark mesh review passed. The preceding `703c26b` analog-voice checkpoint passed GitHub CI. No DSP sources, frozen references or installed app changed. Next: Stock VCO and ADSR EG, preserving real phase/stage telemetry and PUSH behavior.

### Stock oscillator and envelope continuation

Stock VCO and ADSR EG now extend the detailed design to nine native panels. The VCO has larger tuning/width knobs around four nominal wave shapes, separate signed modulation controls and engine-driven phase/mode lights. ADSR EG retains all eight controls and guarded PUSH, with larger real stage/gate lamps, a configured normalized envelope guide and the actual CV → depth → base → clamp path. Both use the approved faceplate, rotary hardware, beige readouts and metallic sockets; widths, socket rows and port identities remain stable.

MCP construction cues follow all six VCO controls and all eight ADSR controls at their new positions, with human-unit readouts and grouped editing history. Display guides are explicitly configured, not live samples: the oscillator omits reconstruction/DC-blocker dynamics; the envelope shows a continuous-time stage shape with normalized time. Existing actual telemetry stays separate.

**84 focused checks passed**, including all fourteen MCP control anchors and five cable workflows; strict desktop Clippy and formatting passed. Final light/dark native meshes were inspected. The preceding utility CI run exposed four stale jack-coordinate tests; those workflows and the related cancellation fixture now select actual named ports. Detailed results and the native capture are recorded in the [review notes](../design/reviews/signal-atlas-two-modules-v1/README.md). Next: Stock LFO and Stock VCF, then the remaining catalog families. Installed 0.2.5 build 7 remains unchanged; no DSP sources or frozen references changed.

### Stock modulation and filter continuation

Stock LFO now shares the VCO's larger main controls and four-wave display, with its own source-accurate offset/invert/PWM shapes, observed clock units and real phase/mode lamps. Stock VCF pairs three larger main controls with three smaller CV-depth controls and a drive/ladder/negative-feedback diagram. Both keep their widths, socket rows, identities, numeric editing and undo; all twelve MCP anchors follow the new controls. The filter's highpass tap mix and exact CV laws are explained in tooltips, without inventing a measured response.

[Native LFO/filter pair and thumbnails](../design/reviews/signal-atlas-two-modules-v1/native-stock-modulation.png). **85 focused checks passed**, including the actual LFO engine in all offset/invert modes at three widths, clock-aware editing, MCP anchors, real lamps, geometry, shared rendering and isolated simultaneous filter previews. Strict desktop Clippy and formatting passed; light/dark meshes were inspected. Prior `dc748c7` CI passed 1,090 desktop tests and failed one stale ADSR label expectation; that exact fixture is corrected and passes locally. No DSP sources/frozen references or installed app changed. Eleven native panels now have detailed layouts; next: Stock VCA, Process and Octave as a utility family.

### Stock utility continuation

Stock VCA, Process and Octave bring the detailed native design to fourteen panels. VCA has a larger central level knob and sixteen grouped actual-gain lanes, with a separate MCP cue for EXP/LIN response. Process adds sample/hold, track/hold and step/slew diagrams while keeping PRESS safety and actual button telemetry. Its step guide distinguishes the instantaneous detent from a linear limiting ramp. Octave keeps its nine authored selectors and exact-value behavior, with stronger key contrast and a pitch-plus-rounded-shift diagram; the effective shift remains actual engine state.

[Native utility trio and thumbnails](../design/reviews/signal-atlas-two-modules-v1/native-stock-utilities.png). **42 focused checks passed**, including real Session edits/undo, actual gain lanes/dark-room telemetry, octave source/raw-value behavior, manual gate release, precise units, all affected MCP anchors and simultaneous isolated previews. A real-engine check confirms +1 V slew CV halves rate and doubles time. Strict desktop Clippy and formatting passed; light/dark native meshes were inspected. Preceding `92c4e0c` passed GitHub CI. No DSP/frozen-reference changes or installation. Next: the mixer family, starting with Stock VCA Mix and the two six-input mixers.


### Mixer continuation

Mix 6, Stock Mix and Stock VCA Mix now use the detailed native treatment, bringing the catalog pass to seventeen panels. Six-input mixers gain larger calibrated level knobs, numeric wells and a configured sum/average/level/polarity path. Stock VCA Mix keeps its four actual strip VUs and five rotaries, with separate strip/master CV laws and a compact four-output-to-master diagram. Socket identities, existing HP widths, raw/dB entry and editing/history remain intact; MCP cues follow the relocated controls, including the six-input flags.

[Native mixer panels and matching thumbnails](../design/reviews/signal-atlas-two-modules-v1/native-mixers.png). **24 focused checks passed**, covering real Session edits/undo, precision values, actual factory meters, all seven Stock VCA Mix MCP anchors, six-input flag cues, socket/text clearance, natural browser zoom and shared rack/browser drawing. Strict desktop Clippy and formatting passed; light/dim-room CPU meshes were inspected. The prior `dc9dbe8` CI passed 1,096 desktop checks and exposed two old primitive expectations; those are corrected with size/alignment assertions retained. No DSP/frozen-reference changes or installation. Next: detailed effect panels, preserving truthful configured guides and real telemetry.


### Delay and space continuation

Stock Delay, Stereo Delay and Space extend the detailed native treatment to twenty panels. Stock Delay keeps its actual clock lamp, eight controls and signed CV depth, with larger primary knobs and a mono delay/tone/feedback/dry-wet guide. Stereo Delay groups ten controls into two rows and shows the filtered cross-feedback loop. Space shows its four-line feedback delay network and configured nominal RT60, updated with DECAY. Independent dry/wet gains and unfiltered stereo-delay wet taps are explained in the guide tooltips. These are configured diagrams, not measured waveforms or impulse responses.

[Native delay/reverb panels and matching thumbnails](../design/reviews/signal-atlas-two-modules-v1/native-effects.png). **19 focused checks passed**, including real Session successive edits with one undo for all 24 controls, their construction-motion anchors, precision units/navigation, actual clock emission, graph-edit gesture identity, all generic control geometry, socket/text clearance and shared rack/browser drawing. Strict desktop Clippy and formatting passed; light/dim-room CPU meshes were inspected. The prior mixer CI passed 1,099 desktop checks and found one reduced-zoom endpoint label crossing its knob hit target; that actual layout issue is corrected with the target preserved. No DSP/frozen-reference changes or installation. Next: dynamics and macro/drum voices.


### Dynamics and creative voices

DUCK, Drum and Macro Voice bring the detailed native pass to twenty-three panels. DUCK pairs its seven controls with a configured soft-knee transfer plot including makeup, explicit dB axes and a separate explanation of detector/attack/release behavior. Drum retains three direct mode choices, with a source/drive/envelope/level path and the envelope's configured T60. Macro Voice retains all eight engines, six continuous controls and its original pitch/trigger polyphony, showing the engine/LPG/DC/output path. Existing widths, sockets, real units, precision editing and grouped history remain intact; both mode choices and all eighteen rotaries have MCP cues.

[Native dynamics/voice panels and matching thumbnails](../design/reviews/signal-atlas-two-modules-v1/native-creative-voices.png). **16 focused checks passed** across the interaction, geometry, choices and configured-guide targets; this includes real Session successive edits/undo for every rotary, twenty MCP anchors, mode/envelope legend changes, and shared rack/browser drawing. The compressor plot agrees within 0.001 dB with the actual settled Core engine across 45 input/ratio/knee combinations. This bounded static check is not a transient, detector or hardware equivalence claim. Strict desktop Clippy and formatting passed; light/dim-room CPU meshes were inspected. Creative checkpoint `1ce9099` passed complete GitHub CI, including the Mac build and visualizer job. No DSP/frozen-reference changes or installation. Next: the SVF filter and physical resonator.

### Rack editing, toolbar and dark-room polish

The user-requested UX pass adds a large human-unit value bubble above a rotary while dragging, resetting or using keyboard adjustment. It eases in and fades after release, remains readable at overview zoom and never owns pointer input. Cmd/Ctrl+Z and Shift+Cmd/Ctrl+Z now work while a knob retains keyboard focus; actual text editors retain their own undo. A history action cancels the cached pointer value for that frame so releasing a knob cannot replay the edit immediately after undo.

The toolbar uses bounded project/status and action groups, aligned controls and a compact second band. Long titles cannot cover the Save, transport, MCP or settings actions. Existing animated anchored settings/device dropdowns remain. Dark-room LEDs, port lights and meter segments now reveal brighter, broader soft halos as ambient light falls; daytime bloom stays unchanged, zero/missing signals remain dark, and the mesh budget and cable occlusion order are preserved.

**44 focused macOS ARM behavior checks passed**, including actual Session gestures and one-step history across the control families, same-frame release/undo/redo, text focus, modal guards, five toolbar widths and dark-room emission/geometry. Strict all-target desktop Clippy and formatting passed. CPU meshes were inspected for a real knob drag, the toolbar at 700/850/1100/1140/1440 pixels and its settings dropdown, and actual engine-observed lamp/port/meter states at full, dim and zero room brightness. No native window or physical audio-device qualification is claimed. Installed **0.2.5 build 7 remains unchanged**.

[Dark-room preview](../design/reviews/signal-atlas-two-modules-v1/native-ux-dark-room.png) · [Toolbar](../design/reviews/signal-atlas-two-modules-v1/native-ux-toolbar.png) · [Value bubble](../design/reviews/signal-atlas-two-modules-v1/native-ux-value-bubble.png).

### SVF and physical resonator continuation

The SVF and Modal Resonator bring the detailed native design to **twenty-five panels**. Both now use the same warm faceplate, hardware, tactile knobs and readouts as the approved voice family, with identical browser thumbnails. The SVF has a larger cutoff control and a configured logarithmic low/band/high core-response guide with explicit dB limits and Q. Its signed CV depth reads in octaves per volt and the input is labeled FREQ CV to reflect adjustable sensitivity. The plot explicitly excludes the upstream nonlinear drive/antialias stage and fixed delay; no live spectrum is invented.

The resonator retains its eight controls and the actual core's natural frequencies, signed force/pickup coupling and mode RT60 profile. The configured display remains readable in darkness. Its force/exciter → modes/loss → velocity output diagram explains the physical signal path, with saved strike/pickup positions below. Existing widths, socket coordinates, raw parameters, precision editing, gesture history and MCP motion remain intact.

[Native panels and matching thumbnails](../design/reviews/signal-atlas-two-modules-v1/native-modal-and-filter.png). **50 focused checks passed**, including real Session successive edits/one-step undo for every rotary, percentage/signed exact entry, all twelve MCP anchors with human units, shared rack/browser geometry, minimum-zoom jack captions and actual named patching. The SVF guide agrees within 0.02 dB with the actual zero-drive engine at three frequencies and three resonance settings across all three outputs. This bounded passband comparison does not establish nonlinear/AA/high-frequency or hardware equivalence. Strict all-target desktop Clippy and formatting passed; light/dim CPU meshes were inspected.

Broader CI for the preceding UX checkpoint passed 1,109 desktop tests and the visualizer job, but exposed two oscillator lamp fixtures that counted the newly brighter additive glow core as an extra lamp. The fixtures now explicitly compare the three actual opaque measured lenses, preserving their unchanged light/dark contract; both pass locally. Checkpoint `8cfca10` subsequently passed complete Mac and visualizer CI. No DSP/frozen-reference changes, release or installation. The oscillator family follows below.

### Four-output and wavetable oscillator continuation

Multi VCO, Multi LFO, Wavetable VCO and Wavetable LFO bring the detailed design to **twenty-nine native panels**. Their warm faceplates, hardware, larger tactile knobs, readouts and browser thumbnails now match the approved family. Multi LFO has an explicit MULTI LFO title. The four-output modules show phase-domain guides: the VCO's actual curved nominal core shapes before reconstruction/filtering, and the LFO's configured voltage curves with saved range/invert settings. Their original functional processors are not circuit models; the plots are explicitly distinct from live audio.

Wavetable panels retain actual source-bank interpolation, asset recovery, six controls and five measured lamps. The display fits quiet source tables and reports its scale; zero stays flat and missing assets remain unavailable. SET POS / LIVE POS distinguishes the saved selection from observed position telemetry. The shape is source-table geometry, not a sampled post-filter output. Existing raw parameter laws, clock-ratio entry, widths, named sockets, exact editing and grouped undo stay intact. All twenty-four controls/modes have MCP cues, including the previously unanchored multi-oscillator mode buttons.

[Native oscillator panels and matching thumbnails](../design/reviews/signal-atlas-two-modules-v1/native-four-oscillators.png). **72 focused checks passed**, covering real Session edits/one-step undo, the sixteen rotaries, all twenty-four MCP anchors, guarded mode buttons, exact/clock-ratio entry and precision navigation, quiet/zero/missing table display, actual named jacks, reduced-zoom geometry and shared browser rendering. Configured Multi LFO curves agree with actual Core output within 0.00003 V over a complete cycle for three widths and both range/polarity settings; discontinuity comparisons exclude a tiny floating-point seam neighborhood. This bounded check does not establish VCO reconstruction/filter or physical-hardware equivalence.

Strict all-target desktop Clippy and formatting passed; light/dim CPU meshes were inspected. Previous checkpoint `8cfca10` passed complete Mac/visualizer CI; the new head receives the same CI. No production DSP/frozen-reference changes, native-window/device qualification, package or installation. Installed **0.2.5 build 7 stays unchanged**. Next: noise and stochastic control families.

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
