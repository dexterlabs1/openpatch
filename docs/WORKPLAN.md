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

## Delivered: Rack patch import and repair

[PR #30](https://github.com/dexterlabs1/openpatch/pull/30) is merged at `b03ced7`, with runtime source `cc14da0`. Desktop and MCP share the complete project document, dependency repair, undo/history and saved layout. Verified mappings become native modules; unsupported modules retain their source data, assets and incident routes as visible placeholders. Exact playback stays suspended; Partial playback and substitutions are explicit. Source-only routes carry no invented signal activity. Native v1/legacy JSON migration, recovery and templates remain supported. [Support and limits](VCV_IMPORT.md).

The user's real fixture preserves **55 modules and 83 cables**, with **24 verified native mappings and 31 unresolved dependencies**, through native import/save/reopen. The declared synthetic collection exercises real 16-channel native processing, delayed feedback startup/steady-state and unsupported effects. Six focused host regressions cover repair, saved camera/placement, authored MCP loads, same-package reimport, unseen history geometry and early startup fitting. This proves the supported subset and preservation, not sonic equivalence of the whole patch.

[Full macOS ARM CI](https://github.com/dexterlabs1/openpatch/actions/runs/39016180431) passed **2,378 Rust tests**, including **1,206 desktop cases** (eight intentionally ignored), **117 Python/MCP tests**, strict workspace lint/formatting, optimized binaries, app packaging and visualizer integration. The downloaded artifact's GitHub SHA-256 digest, ARM64 executables and deep/strict ad-hoc signature were verified. This internal candidate retains base metadata 0.2.6/build 8; the installed app remains unchanged. No native-window/audio-device qualification is claimed.

## Active next: version-correct Rack voice playback

Implement the Fundamental 2.6.4 **VCO → VCF** voice chain using separate source-version identities, preserving existing Stock and Multi VCO/VCF patches. The real fixture contains one such VCO and four VCFs. Their source behavior differs substantially from the currently qualified newer Stock implementations; do not silently map them as equivalents.

Reuse the existing frozen Stock reference harness architecture without changing its pins or declarations. Qualify actual 2.6.4 C++ class behavior, mono/16-channel routing, sync/PWM, regrowth, LP/HP connectivity, driven filtering and self-oscillation. Audit the old VCF's previous-input initialization before claiming frame-zero equivalence. Then import a real voice chain through VCA/ADSR, verify finite audible output and edit/history/save/reopen behavior, and add the pinned mappings. Third-party plugins and binary hosting remain tracked parity work; MIDI/live and other platforms remain deferred.

**Processor checkpoint:** distinct `rack264_vco` / `rack264_vcf` now pass the
[pinned-class reference suite](../scripts/rack264_voice_reference/README.md):
45 oscillator cases, 78 filter cases and ten genuine SDK loading/reset cases
on macOS ARM with Rack SDK 2.6.6. All compared finite voltages and observed
states matched exactly in these fixtures; this is a bounded source-version
result, not hardware accuracy or whole-patch equivalence. VCF explicitly uses
recorded initial history because the C++ constructor leaves it uninitialized;
Rust starts fresh histories at zero. Its legacy loader distinguishes absent
`params` from a present array missing newer CV-depth entries. The 19 focused
processor cases, three actual Core voice/adoption/no-heap checks, strict Core
Clippy and desktop compilation pass. Source-adaptation GPL notices and combined
binary licensing are included; original OpenPatch code remains MIT. Existing
Stock oracles are unchanged. **Shared panels, catalog fixtures, pinned import
mappings and application workflows are next; no full milestone CI/package or
installation yet.** Installed 0.2.6 build 8 is unchanged.

Reuse this integration checkout; do not create another worktree for a small slice. The import milestone's local evidence is `native-test-output/rack-import/STATE.json`; Stock Scope's frozen evidence remains under `native-test-output/stock-scope/`. Start one compact current state for the version-correct voice work, referencing those completed records rather than copying their histories.

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
