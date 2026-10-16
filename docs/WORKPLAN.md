# Current work plan

Updated 2026-10-16. This is the compact execution plan; [ROADMAP.md](ROADMAP.md) and [MODULE_COVERAGE.md](MODULE_COVERAGE.md) retain the full objective and parity inventory.

## Stable release

**OpenPatch 0.2.5 build 7 stays installed.** It includes the Signal Atlas knob/socket rollout, Ferrofluid and the Studio Mixer with FX sends. PR25 merged at `ab3b2a280e9affdd55015ba5fd0a4664a32df6aa`. Do not create another release for routine development changes. User priority is macOS ARM; Linux/Windows validation can wait. MIDI/live performance remain last.

## Execution changes

The recent process duplicated feature-push/PR CI, required two full capture reviews, ran benchmarks across every platform for routine UI edits, copied large manifests into chat and fragmented related work across small releases. These consume time without a matching product benefit.

Now use one milestone branch, focused local checks, one PR CI run and one appropriate review. Default CI targets macOS ARM, with Rust build/dependency caching and cancellation of superseded runs. Full Linux/Windows validation and benchmarks are available through the manual `full_validation` input. Documentation-only changes skip application CI. The installed app is independent of CI artifacts.

## Active product milestone: finish the approved module design

The user wants the new module design soon. Extend the approved **01 Signal Atlas** direction across the remaining catalog before adding more DSP modules or patch-import features. Sixteen module types already use the new rotary controls; this is a rollout of that direction, not another round of concepts.

- Warm ivory panels, black rotary knobs and larger ports; prominent top-left titles in the selected Rajdhani SemiBold font. No OpenPatch module branding, descriptions or colored header bands.
- Markings describe actual ranges, units, routing and control behavior. Preserve meaningful differences between modules without decorative pseudo-circuitry or invented signal displays.
- Rack panels and browser thumbnails share the same renderer. Keep titles, controls and jack labels legible at working zoom levels; retain useful weak-signal displays and genuine signal lights, including dark-room behavior.
- Preserve numeric entry, fine adjustment, keyboard access, undo, cable hit areas and existing parameter behavior. Keep Ferrofluid, Studio Mixer FX sends/dynamic channels and Custom tags intact.
- **Delivery:** inventory the remaining panels, apply shared controls first, then convert coherent layout families on one milestone branch. Review a representative changed panel early; avoid waiting for every catalog detail before making the design reviewable. Use focused UI/geometry checks and one Mac regression for the release candidate; unchanged DSP references do not need repeating.
- **Done:** remaining supported catalog panels follow the approved design, browser previews match them, controls/labels remain usable, affected checks pass and the candidate is ready for one installation. Keep 0.2.5 installed during development.

### First catalog batch in progress

Branch `work/signal-atlas-catalog` extends rotary controls and larger sockets to Sine LFO, Low-pass, Gain, OTA VCA, analog ADSR and State Variable Filter. ADSR EG receives four main rotaries and four smaller bipolar CV-depth rotaries. Its range legends describe real time constants, sustain voltage and modulation depth; stage lamps remain engine-driven. All catalog faces now share the approved warm ivory material through the rack/browser renderer.

Focused checks cover successive real-Session drag edits with one undo, enlarged jack identity/caption clearance, envelope precision entry/lights, and identical rack/browser control geometry. An opt-in CPU panel preview renders actual egui meshes without opening a window or audio device. This batch is development source, not an installed release. Remaining dense effect, sequencer, mixer and utility layouts still need individual control/spacing passes; the shared faceplate change alone does not complete their redesign.

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
