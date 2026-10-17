# Current work plan

Updated 2026-10-17. This is the compact execution plan; [ROADMAP.md](ROADMAP.md) and [MODULE_COVERAGE.md](MODULE_COVERAGE.md) retain the full objective and parity inventory.

## Stable release

**OpenPatch 0.2.6 build 8 is installed at `~/Applications/OpenPatch.app`.** The user explicitly requested this upgrade. It includes the 34-panel detailed Signal Atlas pass, stronger lights-off glow, value bubbles, undo/toolbar fixes, Ferrofluid and Studio Mixer. The optimized ARM64 desktop/CLI bundle is locally ad-hoc signed, with installed hashes/signature verified; 0.2.5 build 7 was removed. Keep this build stable during continued development. User priority is macOS ARM; Linux/Windows validation can wait. MIDI/live performance remain last. No native-window/audio-device qualification is claimed.

## Execution changes

The recent process duplicated feature-push/PR CI, required two full capture reviews, ran benchmarks across every platform for routine UI edits, copied large manifests into chat and fragmented related work across small releases. These consume time without a matching product benefit.

Now use one milestone branch, focused local checks, one PR CI run and one appropriate review. Default CI targets macOS ARM, with Rust build/dependency caching and cancellation of superseded runs. Full Linux/Windows validation and benchmarks are available through the manual `full_validation` input. Documentation-only changes skip application CI. The installed app is independent of CI artifacts.

## Active milestone: complete the approved module design

Follow the approved [01 Signal Atlas reference](../design/concepts/module-panels-20261016-v1/01-signal-atlas.png), [two-panel study](../design/reviews/signal-atlas-two-modules-v1/index.html) and detailed native ADSR/Ladder implementation. **76 panels now have detailed native layouts.** Studio Mixer retains its existing bespoke panel; the three MIDI panels remain deferred. The non-MIDI design inventory has been audited, with final Mac candidate qualification pending; this count does not establish VCV parity. [Family previews and validation notes](../design/reviews/signal-atlas-two-modules-v1/README.md) retain the historical checkpoints instead of duplicating them here.

Preserve larger tactile knobs, readable titles/values, meaningful source-backed markings and identical shared rack/browser geometry. Keep existing widths, named sockets, editing, undo and MCP construction behavior. Configured diagrams remain explicitly separate from live measurements. The user rejected broad neon glow: use restrained local halos, readable lenses and honest zero/unavailable state.

Latest: **Scope, Stereo Audio, Poly Viz, Notes and Blank**. Scope retains calibrated acquisition/AUTO behavior with four new rotaries and inline exact entry. Stereo Audio exposes its true polyphonic sum, right-input normal and two-channel output. Poly Viz uses separated polarity lenses and measured channel state; Notes/Blank preserve editing and resizing. [Native family](../design/reviews/signal-atlas-two-modules-v1/native-views.png) · [dim-room snapshots](../design/reviews/signal-atlas-two-modules-v1/native-views-dark.png).

**Validation:** 75 focused checks passed across actual Session edits/undo, scope calibration/AUTO/weak signals, annotation Unicode/resize/history, rotary/inline precision entry, visible MCP motion and human interruption, restrained lighting, all-catalog default rack/browser drawing equality, and overview text/socket/mount clearance. Strict desktop Clippy/formatting passed; actual light/dim CPU meshes were reviewed. Source `2dbb1f6` passed the preceding full Mac/visualizer CI. No DSP/frozen-reference changes, package/install or native-window/audio-device qualification.

Keep [PR #28](https://github.com/dexterlabs1/openpatch/pull/28) draft and the installed app stable. The final inventory audit accounts for all 77 non-MIDI panels, including Studio Mixer’s existing console. Its knobs, faders and saved switches now have actual MCP control cues, fixing the audit’s remaining gap. Thirty-two focused audit checks passed; its 1–16-channel workflow, stereo FX returns and Custom/Mixing categories remain intact. [Audit evidence](../design/reviews/signal-atlas-two-modules-v1/README.md#non-midi-design-candidate-audit). Final CI `7784f53` found three stale caption fixtures and a real Notes guide clipping regression. The fixture targets are corrected; all 14 shipped guides now fit the selected larger font at five zoom levels, with graph/parameter/asset data unchanged. All 211 affected workflow checks, strict desktop Clippy/formatting and actual guide captures passed before one new full run. Next: pass full workspace/MCP CI, then inspect/reuse its Mac bundle artifact for the milestone candidate. The three MIDI panels and live performance remain last. Stock Scope remains checkpointed until that design milestone is ready.

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
| 1. Signal Atlas module design | Complete the approved non-MIDI catalog rollout with larger knobs/ports, meaningful markings, readable titles and identical rack/browser rendering. Review representative panels early; qualify one Mac release candidate. |
| 2. Scope integration | Connect, adjust and inspect Stock Scope reliably, including successive TIME drags, native AUTO view and real Callback → MCP publication. Focused safety/lifecycle/realtime and one Mac regression pass. |
| 3. Real Rack patch workflows | Open, repair, play, save and reopen a declared collection of actual `.vcv` patches with routing, parameters, layout and assets preserved. Unsupported modules remain explicit, recoverable placeholders. Publish the supported mapping. |
| 4. Complete non-MIDI Mac instrument | Close module gaps in coherent families, including hidden VCA-2/Unity compatibility; complete editing/history, project recovery, audio configuration and Audio 2/8/16 requirements. Qualify shared physical audio paths once at the host milestone. |
| 5. Demonstrated performance/reliability | Comparable representative small, polyphonic, feedback, delay-heavy and large patch measurements support specific CPU/latency claims. Fix measured bottlenecks; verify callback deadlines, live graph changes, overload, installation/upgrade and recovery. |
| 6. Distinctive creative workflow | MCP completes useful musical editing/project tasks with visible changes, coherent undo and conflict recovery; GLSL visuals persist assets and contain failures during audio use. General-purpose live DSP authoring stays a separate later feature. |
| 7. Deferred portability, then MIDI/live | When authorized, qualify Linux/Windows against the finished baseline. Complete remaining MIDI conversion, mapping, timing, reconnect and stuck-note recovery last. These gates remain open until actually verified. |

Keep frozen numerical contracts, state/event and realtime invariants, persistence/migration, representative native interactions and actual device qualification. Add cases for concrete uncovered behavior; do not multiply every parameter, platform, sample rate and gesture into an exhaustive Cartesian matrix. Numerical agreement alone is not application completion.

The immediate architecture decision is the [compatibility boundary](COMPATIBILITY_DECISION.md): stable native module identities, versioned state and recoverable source payloads are shared foundations for import and extensions. Existing VCV binary hosting is a distinct unresolved deliverable; a Rust extension API does not silently satisfy it.

The final goal still requires the pinned 49-module baseline, tracked legacy compatibility, the host matrix, supported platforms and bounded improvement claims. Until deferred requirements pass, call intermediate releases macOS/non-MIDI milestones, not full Rack parity. Third-party coverage is a versioned support matrix, not an unfinishable promise to support every future plugin.
