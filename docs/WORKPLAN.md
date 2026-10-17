# Current work plan

Updated 2026-10-17. This is the compact execution plan; [ROADMAP.md](ROADMAP.md) and [MODULE_COVERAGE.md](MODULE_COVERAGE.md) retain the full objective and parity inventory.

## Stable release

**OpenPatch 0.2.7 build 9 is installed at `~/Applications/OpenPatch.app`.**
The user explicitly requested this upgrade on 2026-10-17. It includes the 77-panel
Signal Atlas design, softer local glow, Stock Scope, pinned 2.6.4 voice models,
source-preserving Rack import and native RackMCP Sequencer/Listen/Bridge controls.
Ferrofluid, Studio Mixer and Stock Process are present. The optimized ARM64
bundle's installed binary hashes and deep/strict ad-hoc signature match the
verified build. The previous 0.2.6 app and an older extracted candidate were
removed; patches/settings were untouched. Local family validation passed 2,616
Rust tests and 123 Python/MCP tests, plus four bundled-release CLI smoke tests.
[PR #32](https://github.com/dexterlabs1/openpatch/pull/32) is merged; its [full Mac CI](https://github.com/dexterlabs1/openpatch/actions/runs/39040128343) passed the same 2,616 Rust/123 Python tests, optimized build, packaging and visualizer integration. This is not a notarized public release or a
native-window/audio-device qualification. Keep the installed app stable; macOS
ARM remains first, Windows/Linux and MIDI/live performance remain deferred.

## User-requested rack workflow and creative expansion (2026-10-17)

The first creative family is merged through [PR #34](https://github.com/dexterlabs1/openpatch/pull/34)
at `7248095`: three source-derived instruments and their shared seated-black-screw
rendering. [Full macOS CI](https://github.com/dexterlabs1/openpatch/actions/runs/39069352032) passed **2,693 Rust tests and 127 Python/MCP tests**, strict lint/format checks, optimized ARM binaries, application packaging and browser visualizer integration. Other rack/toolbar work and experimental
binary hosting remain preserved on `work/rack-binary-feasibility` at `71c0cba`;
[recorded rack tools](https://github.com/dexterlabs1/openpatch/tree/71c0cba/design/reviews/rack-tools-v1)
and [host tools](https://github.com/dexterlabs1/openpatch/tree/71c0cba/design/reviews/host-tools-v1)
are separate pending integration work. No source change is installed; keep
0.2.7 build 9 stable. Claude's Wordpatch work is independent.

The user's next creative-library milestone is [twenty genuinely new instruments](CREATIVE_MODULES_20.md),
with independent contracts rather than renamed existing kinds. **Tidal Functions,
Frame Scene and Segment Garden passed the first family gate. Event Fork and Rhythm Terrain are the
fourth and fifth implemented development candidates; fifteen remain.** Tidal
Functions’ native Rust generator/extractor matches 129,840 frames in 34 pinned C++ reference cases
bit for bit, including nonzero amplitude scans, while preserving the original
frame-zero clock failure as a negative and explicitly adapting native startup.
Shared Signal Atlas controls, actual output lamps, matching browser thumbnails,
MCP/history/presets/reopen and callback no-heap checks have focused Mac evidence.
[Contract and bounds](TIDAL_FUNCTIONS.md), [actual panel capture](../design/reviews/tidal-functions-v1/README.md).
Frame Scene adds 64 persistent stable-ID scenes, source-integer keyframing,
explicit ideal VCA normaling, real UI telemetry and shared revision/history/MCP
editing. Its 25 source cases match 163,013 integer records and pass an independent
UBSan/bounds audit; source-undefined endpoints have explicit native extensions.
[Contract and example](FRAME_SCENE.md), [actual shared panel review](../design/reviews/frame-scene-v1/README.md).
Segment Garden adds six persistent source kernels, physical-gate grouping, all standard
segment modes, source-clock buffering and ordered edge capture. Its shared panel, real
telemetry, seeded controls, MCP/presets/reopen and callback no-heap paths have focused Mac
evidence; raw source qualification covers 952,896 records across 69 cases, with guarded
endpoints and the internal zero-division boundary kept separate.
[Contract and source bounds](SEGMENT_GARDEN.md), [shared panel review](../design/reviews/segment-garden-v1/README.md).
The focused family cut passes local Mac validation: **2,693 Rust tests and 127 Python/MCP tests**, strict workspace Clippy, formatting and doc tests. The three normal-room panel/thumbnail captures are byte-identical to their reviewed checkpoints; all 55 module/reference files match the qualified source. Three regression issues were corrected: content schema tests now select by type, the spacer geometry test recognizes seated screws, and two example guides fit their panels without changing graph or parameter data. The same family-level macOS CI gate passed in PR #34. Source fidelity, shared UI/MCP/history, Custom plus functional tags for original instruments and Mac validation remain done requirements. Binary-host
MCP save integration, active panels and actual device qualification remain open
parallel goal requirements; no hosting-completion or new installation is claimed.

**Next family, `work/creative-rhythm`:** [Event Fork](EVENT_FORK.md) adds two source-derived Bernoulli routers with physical input normaling, shared seeded RNG, toggle and latch memory. Eighteen source cases cover 12,190 exact raw scans plus the complete ADC table; 513 genuine frames also pass through the actual native Engine. Six adapter checks, four Engine contracts, callback allocation/deallocation checks at five rates, six UI checks, the 90-kind shared-renderer guard, readable factory guides, actual stdio MCP/history/preset/reopen, strict Clippy and reviewed normal/dim panels pass. Its quiet routed-duet example renders offline without clipping. Family-level CI remains pending; no install or hardware/device qualification is claimed. [Rhythm Terrain](RHYTHM_TERRAIN.md) adds the pinned Grids drum maps and Euclidean engine, accents, swing, source clock/reset and runtime tap authority. Its genuine source/audited/Rust comparison passes 49 cases / 377,402 operations; the actual Engine matches 29,400 source frames. Focused native callback/no-heap, shared UI/history, stdio MCP/presets/reopen, normal/dim panels and a quiet offline groove have evidence. Fast TAP clicks preserve ordered HIGH/LOW snapshots, and emergency release cancels stale presses. The shared default rack/browser renderer now covers 91 kinds. Continue Recall Random, then run one full Mac family gate. Retain all broader parity/host/rack-workflow requirements above.

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

**Further lighting refinement (2026-10-17, installed in 0.2.7):** following
feedback about unnatural glow, local halos, diffusion, display spill and knob-ring
haze are substantially reduced while lens brightness and weak-signal response
remain visible. Sixteen focused shared-control/lighting checks and normal/dim/off
CPU calibration pass. This also passes the complete local desktop suite; it does
not establish physical-light or GPU-driver accuracy.

## Delivered: Rack patch import and repair

[PR #30](https://github.com/dexterlabs1/openpatch/pull/30) is merged at `b03ced7`, with runtime source `cc14da0`. Desktop and MCP share the complete project document, dependency repair, undo/history and saved layout. Verified mappings become native modules; unsupported modules retain their source data, assets and incident routes as visible placeholders. Exact playback stays suspended; Partial playback and substitutions are explicit. Source-only routes carry no invented signal activity. Native v1/legacy JSON migration, recovery and templates remain supported. [Support and limits](VCV_IMPORT.md).

At the PR #30 checkpoint, the user's real fixture preserved **55 modules and 83 cables**, with **24 verified native mappings and 31 unresolved dependencies**, through native import/save/reopen. The declared synthetic collection exercises real 16-channel native processing, delayed feedback startup/steady-state and unsupported effects. Six focused host regressions cover repair, saved camera/placement, authored MCP loads, same-package reimport, unseen history geometry and early startup fitting. This proves the supported subset and preservation, not sonic equivalence of the whole patch.

[Full macOS ARM CI](https://github.com/dexterlabs1/openpatch/actions/runs/39016180431) passed **2,378 Rust tests**, including **1,206 desktop cases** (eight intentionally ignored), **117 Python/MCP tests**, strict workspace lint/formatting, optimized binaries, app packaging and visualizer integration. The downloaded artifact's GitHub SHA-256 digest, ARM64 executables and deep/strict ad-hoc signature were verified. This internal candidate retains base metadata 0.2.6/build 8; the installed app remains unchanged. No native-window/audio-device qualification is claimed.

## Delivered: version-correct Rack voice playback

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
processor cases, four actual Core voice/bypass/adoption/no-heap checks, strict Core
Clippy and desktop compilation pass. Source-adaptation GPL notices and combined
binary licensing are included; original OpenPatch code remains MIT. Existing
Stock oracles are unchanged.

**[PR #31](https://github.com/dexterlabs1/openpatch/pull/31) is merged at `c020cef`:** the two models now use the shared detailed rack/browser
panels with source-specific bounds, guides and lights. All 83 catalog previews
match default rack drawing. Five new UI checks and targeted CPU-only normal/dark
and browser renders pass. Import transforms preserve sync encoding, removed
source slots and legacy CV-depth defaults. The actual user fixture now has
**29 mapped / 26 unresolved** modules; all 55 modules, 83 cables, source JSON and
assets survive save/reopen. A real shared-session mono/16-channel voice passes
parameter edit/undo/redo and fresh saved reopen; the MCP stdio test renders a
non-silent WAV after the same workflow. The older filter bypasses raw signal to
both outputs and freezes its integrator/RNG history; an actual Core regression
covers empty, mono and 16-channel routes.

[Full macOS ARM CI](https://github.com/dexterlabs1/openpatch/actions/runs/39020770229)
passed **2,411 Rust tests**, including **1,211 desktop cases** (nine intentionally
ignored), **118 Python/MCP tests**, strict workspace lint/formatting, optimized
binaries, Mac packaging and visualizer integration. The downloaded artifact's
GitHub digest, ARM64 executables, deep/strict app signature, icon, source/GPL/font
notices, 83-kind catalog and embedded Ferrofluid shader passed inspection.
The temporary extracted app was removed; the existing CI archive is retained
for reuse. This internal candidate keeps base bundle metadata 0.2.6/build 8 and
runtime source `597ebce`; it is not a newly versioned release or installation.
Installed 0.2.6 build 8 is unchanged. No native-window/audio-device qualification
is claimed.

Reuse this integration checkout. The import evidence is
`native-test-output/rack-import/STATE.json`; the completed voice milestone is
`native-test-output/rack-264-voice/STATE.json`. Keep their frozen evidence rather
than copying or regenerating it for a later module family.

## Current candidate: Core text and host boundary

Exact Core Notes 2.6.6 now converts into editable native Notes, including legacy
loading, Unicode/whitespace and original-source retention. The actual full patch
preserves 55 modules/83 cables with 37 mapped and 18 unresolved. Source and native
App checks cover edits, initialization/history, saved baseline, fresh reopen and
no import side effects. Actual stdio checks exercise text editing/export/reload.
Core AudioInterface2 stays unresolved, retains saved hardware/mute metadata and
validates its known source port shape; import reports explain a saved zero level.
[Source contract and boundaries](CORE_TEXT_IMPORT.md).

This is a development follow-up, not installed in 0.2.7. No version bump, package
or installation is needed. Finish focused checks and one follow-up PR. The next
compatibility gate remains actual existing ARM plugin load/process/state restore
in a separate companion, as scoped in the compatibility decision; that is not
an already proven binary host or a substitute for native Rust module development.

## Delivered: native sequencing/control workflow

Bridge now has real app-owned connection lifecycle, shared native save status and
explicit panel controls. The Sequencer panel displays actual active/pending
patterns and playhead and sends pattern/seek requests through shared transport.
The actual source fixture preserves 55 modules/83 cables with 33 mapped and 22
unresolved. Remaining third-party/Core host/text boundaries stay explicit; this
is not whole-patch or Rack-host equivalence. Native save concurrency, listener
failure/discovery, import boundaries and shared panel checks pass. The complete
local Mac workspace/MCP regression and PR #32 Mac CI pass. [Behavior and boundaries](RACKMCP_CONTROL_CONTRACT.md#native-bridge-and-live-sequencer-checkpoint-2026-10-17).
Installed 0.2.7 build 9 includes this merged checkpoint. Earlier checkpoints below record the
incremental evidence and do not override this current status.

## Active next: imported sequencing and control

**Primitives checkpoint:** the separate RackMCP engine and bounded typed pattern/
target-lane compilation are implemented, preserving existing Score. The genuine
pinned C++ comparison passes 18 cases / 620,730 frames at 44.1/48/96 kHz with
all compared output/position/curve bits equal. Both actual saved sequencers and
their 13 lanes compile; ten targets are supported VCAs/filters and three are
unresolved Surge modules. Nine focused unit tests, a zero-allocation/deallocation
playback/adoption check and strict Core Clippy pass. Source contracts cover [sequencing](RACKMCP_SEQUENCE_CONTRACT.md)
and [Bridge/Listen](RACKMCP_CONTROL_CONTRACT.md). This is not yet a catalog model,
import mapping or usable imported musical workflow. Next implement native target
parameter authority/scheduling, typed document adoption and actual live capture;
then shared panels/import and the end-to-end excerpt. Installed app is unchanged.
The concrete [parameter authority decision](RACKMCP_PARAMETER_AUTHORITY.md) covers
typed target handles, native scheduling, explicit assignment intent and coherent
effective-value capture; it is the next implementation boundary.

**Effective-value checkpoint:** generation-bound native parameter handles now
write the real VCA/filter controls, with an explicit bounded inheritance API
covering assignment/fresh-instance/rate rules. Desktop publication and project
save share one coherent observation; saved copies retain the actual values and
fresh reopen reproduces rendered gain. The assignment/history checkpoint below
now activates this API in prepared callback plans. Source sequencer target
writes remain next. No new catalog kind, import mapping or installation is
included in these checkpoints.

**Assignment/history checkpoint:** shared transactions and undo/redo now carry
explicit per-field parameter intent, including equal values and lifecycle loads.
Prepared callback plans freeze assignment epochs; unrelated graph edits inherit
effective values, including across rate changes. Real shared-session edit/history/
save/reopen, removal undo and suspended execution/resume pass without callback
heap activity. All 280 control tests and the 26 performance tests pass. Late stale
parameter plans are refused; already-prepared plans keep their original meaning.
Next add the dedicated typed RackMCP state/adapter and source-provenance target
bindings, then source-style lane dispatch/ownership and the musical excerpt.
The installed app and catalog remained unchanged at that checkpoint.

**Native adapter checkpoint (in progress):** the separate `rackmcp_sequencer`
now has typed persisted state, all 18 source outputs and prepared eight-lane
native target dispatch. VCA/filter values feed the actual callback publication,
MCP save, manual edit/undo/redo and saved reopen. Unrelated graph edits, node
reorder and rate changes preserve transport/cache; target recreation preserves
the source lane cache. The shared Signal Atlas rack/browser panel shows authored patterns,
voices, drum hits and accents without inventing live telemetry. Native catalog
now has 84 kinds on this milestone branch; source import remains unchanged.
See the [adapter boundary and remaining work](RACKMCP_SEQUENCE_CONTRACT.md#native-adapter-checkpoint)
and [actual panel captures](../design/reviews/rackmcp-sequencer-v1/README.md).
**Data-load checkpoint:** equal/changed sequence edits, presets and undo/redo now
carry explicit load intent through frozen graph plans. Late older plans cannot
replay already consumed loads; unrelated edits preserve playhead and lane cache.
Actual callback and stdio MCP workflows pass. The genuine SDK wrapper comparison
found a self-unbinding defect on repeated source loads: native rebinding remains
idempotent, and the raw comparison remains honestly failed for those target
writes. All 18 output fields match throughout 25,104 frames; a strict separate
audit verifies the exact native correction. [Evidence and remaining ownership boundary](RACKMCP_SEQUENCE_CONTRACT.md#genuine-wrapper-finding-and-deliberate-host-correction).

**Ownership checkpoint:** competing sequencers now acquire exclusively in the
order of their final document actions, including creation, duplication, presets
and undo/redo. Reorder/rate changes retain the winner; deleting it leaves the
target unclaimed. A displaced writer only returns through explicit acquisition.
Late sequencing graphs below the callback revision high-water mark are rejected
before adoption, even after sequence removal, and retained
safely if retirement is full. Focused native/callback checks cover the policy;
the genuine single-module comparison was rerun after the Core change and retains
the same declared SDK self-unbinding differences. No whole-module equivalence
or new installation is claimed.

[Ownership checks and reference result](benchmarks/rackmcp-sequence-ownership.json):
8 ownership, 9 adapter and 3 Core heap checks; 287 control, 28 performance and
220 audio regressions (one intentionally ignored), including shared-session
creation/duplication/history and persistent rollback fencing; one real MCP stdio roundtrip.
Strict Core/CLI/desktop all-target Clippy and workspace formatting pass. The
initial audio run exposed an obsolete VCA fixture that omitted explicit knob
assignment intent; its gain/no-heap assertions pass after correcting that input.

**Imported automation checkpoint:** RackMCP Sequencer 2.0.0 now imports with all
typed source lanes retained. Bindings require the exact source package, original
numeric destination ID, registered mapping and supported native control.
Unrelated same-ID replacements reject atomically; missing targets remain dormant.
Exact waits on unbound automation even with no unresolved modules; explicit
Partial playback discloses omissions in the native inspector and MCP.

The user's complete patch retains all 55 modules/83 cables and source bytes,
now with **31 mapped / 24 unresolved** modules. Both sequencers and all 13 lanes
are retained; ten VCA/filter bindings qualify and three Surge targets remain
unbound. The strict mapping is labeled MusicalRole because native scheduling and
healthy rebinding corrections are disclosed, not whole-module equivalence.
Shared UI/MCP edit/history/save/reopen and an actual allocation-free callback
workflow cover live levels, explicit reload, suspension, Partial resume and
captured effective values. Import starts neither a listener nor an audio device;
the installed app is unchanged.

[Import evidence](benchmarks/rackmcp-sequence-import.json): 351 CLI-library
checks (two ignored), 221 audio-callback checks (one ignored), seven native import
UI checks, actual user-fixture import and desktop save/reopen, plus five real
MCP stdio workflows pass. Strict CLI/desktop all-target Clippy and formatting
pass. [Reviewed UI](../design/reviews/rackmcp-import-v1/README.md) shows Exact
suspension and explicit Partial without invented telemetry. The frozen 96-file
Core manifest is unchanged, so the existing numerical proof is reused.

Transport/status now has direct MCP commands, coherent source-cadence status and
selected-name/running save/history capture. Source transport comparison passes
3 cases / 17,584 frames exactly. The actual shared-session import workflow reaches
audio and source-preserving save/reopen; focused callback checks cover queue
fences and selection-only adoption without reclaiming automation.
[Transport contract and checks](benchmarks/rackmcp-sequence-transport.json): all 71
Core unit/integration targets (927 passed, three ignored), 355 CLI-library
checks (two ignored), 229 audio-callback checks (one ignored), 37 publication
checks, nine document-state checks and five real MCP stdio workflows pass.
Strict Core/CLI/Desktop all-target Clippy and formatting pass.

**Listen checkpoint (not installed):** raw L/R passthrough, host-owned armed
capture, source analysis/WAV and shared rack/browser controls now work through
the same Session MCP authority. The pinned source comparison passes 821 cases /
9,873 complete observations exactly; all 17 analysis fixtures and both WAV export
policies match exactly. [Evidence and declared host corrections](benchmarks/rackmcp-listen.json).
The real shared-session musical fixture passes live capture, octave edit/undo/redo
and source-preserving save/reopen. The full user collection now preserves all
55 modules/83 cables with 32 mapped and 23 unresolved dependencies. Focused Core,
CLI, callback, UI, real stdio and actual-fixture checks plus strict all-target
Clippy/formatting pass. The installed app remains 0.2.6 build 8.

Next close Bridge and live Sequencer panel transport presentation, then one
complete macOS family milestone validation/PR. No release, installation or
whole-module/whole-patch equivalence is claimed.

Close one imported musical workflow using the actual RackMCP 2.0.0 sequencers
and supported voice chain. The reference checkout is pinned at
`285e84c5ff2d3ecef25890da07f578af6a0c5ca6`; its plugin source is clean.
The current read-only import finds 22 unresolved modules in
the full user patch: third-party instruments/effects and Core
Notes/audio I/O. Preserve that complete fixture; a supported excerpt or explicit
Partial playback is not whole-patch equivalence.

Define the source-backed contract before choosing a translation or new model.
The source sequencer includes pattern switching, accent outputs and target
parameter automation; the existing native Score has a different contract and
must not be silently aliased. Treat Sequencer, Bridge and Listen as one coherent
family, including real transport, status/control and capture semantics. Include
the Core text/host boundaries needed by the musical workflow. Do not replace
Bridge/Listen with inert compatibility labels or bind audio hardware on import.

Done checks: one pinned sequence/control timing and saved-state contract; one
imported musical excerpt edited through native UI and MCP with real gate/audio
observations and save/reopen; whole-fixture preservation plus focused changed
lifecycle/realtime checks; one full macOS candidate CI. Reuse unchanged DSP and
rendering proofs. Complete the remaining Fundamental baseline later in coherent
families rather than 49 independent mini-projects. The binary-host feasibility
direction is recorded in [the compatibility decision](COMPATIBILITY_DECISION.md#extensions-and-existing-vcv-binaries);
that open gate must not postpone this native musical workflow.

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
