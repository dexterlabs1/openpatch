# Current work plan

Updated 2026-10-17. This is the compact execution plan; [ROADMAP.md](ROADMAP.md) and [MODULE_COVERAGE.md](MODULE_COVERAGE.md) retain the full objective and parity inventory.

## Stable release

**OpenPatch 0.2.6 build 8 is installed at `~/Applications/OpenPatch.app`.** The user explicitly requested this upgrade. It includes the 34-panel detailed Signal Atlas pass, stronger lights-off glow, value bubbles, undo/toolbar fixes, Ferrofluid and Studio Mixer. The optimized ARM64 desktop/CLI bundle is locally ad-hoc signed, with installed hashes/signature verified; 0.2.5 build 7 was removed. Keep this build stable during continued development. User priority is macOS ARM; Linux/Windows validation can wait. MIDI/live performance remain last. No native-window/audio-device qualification is claimed.

## Execution changes

The recent process duplicated feature-push/PR CI, required two full capture reviews, ran benchmarks across every platform for routine UI edits, copied large manifests into chat and fragmented related work across small releases. These consume time without a matching product benefit.

Now use one milestone branch, focused local checks, one PR CI run and one appropriate review. Default CI targets macOS ARM, with Rust build/dependency caching and cancellation of superseded runs. Full Linux/Windows validation and benchmarks are available through the manual `full_validation` input. Documentation-only changes skip application CI. The installed app is independent of CI artifacts.

## Delivered design checkpoint: Signal Atlas

The approved [01 reference](../design/concepts/module-panels-20261016-v1/01-signal-atlas.png) now covers the **77 non-MIDI panels**: 76 detailed conversions plus the existing bespoke Studio Mixer. Three MIDI panels remain deferred. Larger tactile controls, meaningful guides, readable values, matching rack/browser rendering, real MCP control motion and restrained local glow are preserved. This count does not establish VCV parity. [Families, captures and audit](../design/reviews/signal-atlas-two-modules-v1/README.md#non-midi-design-candidate-audit).

**[PR #28](https://github.com/dexterlabs1/openpatch/pull/28) is merged.** Its [final Mac CI](https://github.com/dexterlabs1/openpatch/actions/runs/39007243527) passed the complete Rust workspace/correctness/allocation suite, **1,175 desktop tests** (seven intentionally ignored), **114 Python/MCP tests**, optimized build, packaging and visualizer integration. The four preceding desktop failures were fixed: three obsolete caption fixtures and real guide clipping. All 14 shipped guides fit the selected larger font at five zoom levels; their graph/parameter/asset data is unchanged. 211 affected local checks and strict Clippy/formatting passed.

The existing CI app artifact was reused rather than rebuilt. Its GitHub SHA-256 digest, ARM64 binaries, deep/strict ad-hoc signature, 80-kind catalog, Studio Mixer Custom/Mixing tags, icon/licenses, selected font and embedded GLSL/Ferrofluid passed inspection. Read-only early-exit CLI flags passed; installed app hashes stayed unchanged. This internal candidate is identified by source **`c4f9509`** and run, with base bundle metadata **0.2.6/build 8**; it is not a newly versioned release or installation. No native-window/audio-device/driver qualification is claimed.

## Active next: Stock Scope integration

The design checkpoint is delivered; resume the existing Stock Scope implementation. Its branch/worktree is `work/stock-scope-integration` / `openpatch-stock-scope-integration`.

- **Proven:** the optimized actual Core probe matched all 4,242 frozen genuine-Rack snapshots across 1,056 cases, including full 50-second sweeps at three rates. The reference declaration remains unchanged. This establishes the bounded source/host numerical result, not complete application or hardware qualification.
- **Focused application checks passed:** seven Core tests, 66 affected desktop Scope checks, actual Callback → Session lifecycle/publication under heap checks, source-correct initialize/recreation and native AUTO separation. TIME uses the current Session revision during successive drags while typed entry retains its opening revision; one gesture remains one undo/redo. Reduced-zoom controls clear real jack targets. [Qualification and captures](STOCK_SCOPE.md#application-evidence).
- **Safety passed:** all 46 unchanged local inputs ran through the real Engine: 12 accepted finite updates and 34 atomic rejections. No SDK-equivalence claim applies to this safety set. Frozen numerical evidence is reused after binary, source, toolchain and artifact audits; executable model/host/probe logic is unchanged.
- **UI checks passed:** all 81 catalog kinds share matching rack/browser default drawing; workspace strict Clippy and formatting pass. The user's latest glow feedback further reduces halo intensity/spread and screen spill; normal/dim captures and 16 lighting checks pass. Installed 0.2.6 build 8 remains unchanged.
- **Next verification:** one complete macOS regression on the settled milestone PR, then review the result. No Linux/Windows, repeated oracle generation or packaging/install during this check.
- **Done:** source controls/lifecycle remain truthful; the stock panel, weak-signal AUTO, cable interactions and live inspection work together; affected focused tests and one macOS regression pass. Then review one milestone PR. Package/install only at a useful release milestone, with the user's session preserved.

The authoritative local evidence is `native-test-output/stock-scope/STATE.json` in the integration worktree. Full logs/traces stay outside chat and Git. Existing `scope` and user patches keep their separate behavior.

## Course to completion

One focused Astra advisory reviewed the remaining scope and this execution policy. The user's subsequent priority puts the approved module redesign first. Resume the checkpointed Scope integration now, then prioritize making music with real imported patches. A blocking regression in the current instrument takes priority immediately.

| Milestone | User-visible exit |
|---|---|
| 1. Signal Atlas module design | Delivered: audited non-MIDI design, full Mac CI, inspected internal app candidate and merged source. Installed app remains stable. |
| 2. Scope integration | Connect, adjust and inspect Stock Scope reliably, including successive TIME drags, native AUTO view and real Callback → MCP publication. Focused safety/lifecycle/realtime and one Mac regression pass. |
| 3. Real Rack patch workflows | Open, repair, play, save and reopen a declared collection of actual `.vcv` patches with routing, parameters, layout and assets preserved. Unsupported modules remain explicit, recoverable placeholders. Publish the supported mapping. |
| 4. Complete non-MIDI Mac instrument | Close module gaps in coherent families, including hidden VCA-2/Unity compatibility; complete editing/history, project recovery, audio configuration and Audio 2/8/16 requirements. Qualify shared physical audio paths once at the host milestone. |
| 5. Demonstrated performance/reliability | Comparable representative small, polyphonic, feedback, delay-heavy and large patch measurements support specific CPU/latency claims. Fix measured bottlenecks; verify callback deadlines, live graph changes, overload, installation/upgrade and recovery. |
| 6. Distinctive creative workflow | MCP completes useful musical editing/project tasks with visible changes, coherent undo and conflict recovery; GLSL visuals persist assets and contain failures during audio use. General-purpose live DSP authoring stays a separate later feature. |
| 7. Deferred portability, then MIDI/live | When authorized, qualify Linux/Windows against the finished baseline. Complete remaining MIDI conversion, mapping, timing, reconnect and stuck-note recovery last. These gates remain open until actually verified. |

Keep frozen numerical contracts, state/event and realtime invariants, persistence/migration, representative native interactions and actual device qualification. Add cases for concrete uncovered behavior; do not multiply every parameter, platform, sample rate and gesture into an exhaustive Cartesian matrix. Numerical agreement alone is not application completion.

The immediate architecture decision is the [compatibility boundary](COMPATIBILITY_DECISION.md): stable native module identities, versioned state and recoverable source payloads are shared foundations for import and extensions. Existing VCV binary hosting is a distinct unresolved deliverable; a Rust extension API does not silently satisfy it.

The final goal still requires the pinned 49-module baseline, tracked legacy compatibility, the host matrix, supported platforms and bounded improvement claims. Until deferred requirements pass, call intermediate releases macOS/non-MIDI milestones, not full Rack parity. Third-party coverage is a versioned support matrix, not an unfinishable promise to support every future plugin.
