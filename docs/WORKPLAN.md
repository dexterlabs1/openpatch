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

## Delivered: Stock Scope integration

[PR #29](https://github.com/dexterlabs1/openpatch/pull/29) is merged at `903021c`. The separate Stock Scope uses the shared Signal Atlas panel, source-correct lifecycle, nominal millisecond TIME controls, native AUTO fitting and real Callback → MCP publication. Existing Scope patches retain their model. Successive TIME edits stay one undoable gesture; stale typed edits cannot overwrite newer revisions. The user's softer, tighter glow is included. [Contract and actual normal/dim captures](STOCK_SCOPE.md).

The frozen actual Core/genuine Rack result covers **4,242 complete snapshots across 1,056 cases**, including full 50-second sweeps. Binary/source/toolchain audits establish unchanged executable model/host/probe logic, so the proof was reused. All **46 local safety records** passed: 12 accepted finite updates and 34 atomic rejections; no SDK-equivalence claim applies to that set.

[Full macOS CI](https://github.com/dexterlabs1/openpatch/actions/runs/39009839579) passed **2,312 Rust tests**, including **1,188 desktop cases** (seven intentionally ignored), **115 Python/MCP tests**, strict workspace Clippy/formatting, optimized binaries, application packaging and visualizer integration. The first CI attempt stopped on an obsolete global catalog-count assertion; all five unrelated count fixtures were corrected and their affected checks passed before the successful run. The shared renderer retains its exhaustive 81-kind coverage guard. Installed 0.2.6 build 8 is unchanged; no native-window/audio-device qualification is claimed.

## Active next: real Rack patch workflows

Implement the existing [compatibility decision](COMPATIBILITY_DECISION.md) with a **shared control-owned project document**, used by desktop and MCP transactions, history and persistence. A focused Astra review confirmed that desktop-only provenance would be lost through the current MCP patch-only save/replace and undo paths. Core Patch remains an executable projection; source payloads never become hidden DSP parameters.

The shared control foundation is implemented on `work/rack-import`, with native desktop import still pending. Session edits, duplication, deletion, undo/redo and full-document MCP save/load preserve unresolved content and immutable source assets. Exact playback stays suspended while dependencies remain; partial playback is explicit and reports omitted routes. Revision/generation fences freeze DSP and observations without losing delay/scope state. Pending runnable graph updates preserve held MIDI and current-revision actions. Suspension and history crossings capture surviving persistent fields before fencing; unavailable live capture fails atomically. Legacy patch export/replacement refuses source loss.

The bounded converter and native project workflow are now implemented in the working tree. The real 55-module/83-cable patch retains all source data/assets through native save/reopen: 24 modules map to pinned Fundamental 2.6.4 models; 31 remain visibly unresolved. Shared native/MCP repair, Exact/Partial review, placeholder rendering and width-aware import placement are connected. Session owns saved camera/placement with revision/state/generation guards; camera adds no undo step and pending placement blocks full capture. [Current import contract and limits](VCV_IMPORT.md). Focused qualification passed: 320 control tests, 245 native project-workflow cases, six final import/review cases and two actual stdio/MCP workflows; strict all-target lint/formatting passes. The declared synthetic collection exercises real16-channel native processing, retained feedback startup/steady-state and explicit Partial handling for unknown effects. Full milestone macOS CI is next; no new package or installation has been made.

Focused validation passed **302 control-library tests**, **189 actual-callback tests**, two stdio/MCP workflows and strict all-target control/desktop Clippy. These are foundation checks, not evidence of the full import acceptance collection. No new app version, package or installation was made; full milestone CI waits for the usable workflow.

- Finish regression qualification of the connected native store/editor, placeholder review and shared repair workflow, preserving native v1 and legacy Patch JSON behavior.
- Confirm the declared polyphony/feedback/effects acceptance collection in addition to the user's real “So There I Was” fixture. Preserve opaque state and routes through open, repair, edit/undo, explicit playback, save and reopen. The existing curated reconstruction does not prove general import compatibility.
- Resolve review findings, run one settled macOS milestone CI and open a PR. No automatic installation. Existing VCV binary hosting and additional plugin mappings remain tracked parity requirements.


Reuse the existing integration worktree on branch `work/rack-import`; do not create another checkout for each subtask. The active state moves to `native-test-output/rack-import/STATE.json`. Stock Scope's completed numerical/application evidence remains under `native-test-output/stock-scope/`.

## Course to completion

One focused Astra advisory reviewed the remaining scope and this execution policy. The user's subsequent priority puts the approved module redesign first. Stock Scope is now delivered; prioritize making music with real imported patches. A blocking regression in the current instrument takes priority immediately.

| Milestone | User-visible exit |
|---|---|
| 1. Signal Atlas module design | Delivered: audited non-MIDI design, full Mac CI, inspected internal app candidate and merged source. Installed app remains stable. |
| 2. Scope integration | Delivered: source/acquisition, shared panel, native AUTO, live MCP lifecycle, local safety and full Mac CI; PR29 merged. |
| 3. Real Rack patch workflows | Open, repair, play, save and reopen a declared collection of actual `.vcv` patches with routing, parameters, layout and assets preserved. Unsupported modules remain explicit, recoverable placeholders. Publish the supported mapping. |
| 4. Complete non-MIDI Mac instrument | Close module gaps in coherent families, including hidden VCA-2/Unity compatibility; complete editing/history, project recovery, audio configuration and Audio 2/8/16 requirements. Qualify shared physical audio paths once at the host milestone. |
| 5. Demonstrated performance/reliability | Comparable representative small, polyphonic, feedback, delay-heavy and large patch measurements support specific CPU/latency claims. Fix measured bottlenecks; verify callback deadlines, live graph changes, overload, installation/upgrade and recovery. |
| 6. Distinctive creative workflow | MCP completes useful musical editing/project tasks with visible changes, coherent undo and conflict recovery; GLSL visuals persist assets and contain failures during audio use. General-purpose live DSP authoring stays a separate later feature. |
| 7. Deferred portability, then MIDI/live | When authorized, qualify Linux/Windows against the finished baseline. Complete remaining MIDI conversion, mapping, timing, reconnect and stuck-note recovery last. These gates remain open until actually verified. |

Keep frozen numerical contracts, state/event and realtime invariants, persistence/migration, representative native interactions and actual device qualification. Add cases for concrete uncovered behavior; do not multiply every parameter, platform, sample rate and gesture into an exhaustive Cartesian matrix. Numerical agreement alone is not application completion.

The immediate architecture decision is the [compatibility boundary](COMPATIBILITY_DECISION.md): stable native module identities, versioned state and recoverable source payloads are shared foundations for import and extensions. Existing VCV binary hosting is a distinct unresolved deliverable; a Rust extension API does not silently satisfy it.

The final goal still requires the pinned 49-module baseline, tracked legacy compatibility, the host matrix, supported platforms and bounded improvement claims. Until deferred requirements pass, call intermediate releases macOS/non-MIDI milestones, not full Rack parity. Third-party coverage is a versioned support matrix, not an unfinishable promise to support every future plugin.
