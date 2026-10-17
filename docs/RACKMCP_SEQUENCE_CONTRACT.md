# RackMCP 2.0.0 sequence contract

Source contract; the bounded engine comparison below is separate evidence.
Reference: Zachary Scheffler's
RackMCP 2.0.0, commit `285e84c5ff2d3ecef25890da07f578af6a0c5ca6`, frozen
75-tracked-file checkout at `native-test-output/rack-control-import/reference/rackmcp`.
`plugin.json` declares **GPL-3.0-or-later**; retain source attribution/licensing
for adaptations. Audited `src/core/sequencer.{hpp,cpp}`,
`src/rackside/SequencerModule.{hpp,cpp}`, generated protocol limits, and the
Listen/Bridge process and volatile-target boundaries. These distinguish pure
sequence calculation, Rack module effects, and host scheduling/capture effects.

## Saved data and strict preview validation

- `data` permits only `version` (optional integer 1), `seed` (u32, default 1),
  `patterns` (required 1–16), `selectedPattern` (optional existing name; default
  first), `lanes` (optional 0–8). Unknown keys are rejected at every object level.
- Pattern: required unique `name` (1–32 UTF-8 bytes), required integer `length`
  1–64; `stepsPerBeat` integer 1–8/default 4; `tracks` optional array of at most 4.
  Track: `voices` 1–4/default 1; `glideMs` 0–5000/default 80; optional sparse
  `steps` array, at most 64, with unique required integer `step` in pattern range.
- Step: optional `notes` array of 0–4 numbers, semitones from C4 in −60…60;
  `vel` 0…1/default .8, `gate` 0…1/default .5, `prob` 0…1/default 1,
  integer `ratchet` 1–8/default 1, boolean `accent`/`slide` default false.
  Presence enables a step; missing/empty notes means a pitch-holding drum hit.
  Missing steps are silent. Empty store is one silent 16-step pattern `A`.
- Lane: required canonical decimal string `moduleId` (0…INT64_MAX, no leading
  zero except `0`), integer `paramId` 0–65535, `mode` normalized/default or raw,
  boolean `loop` default true, `lengthBars` >0 (minimum 1e-6), maximum 4096.
  Default length is last positive point bar, otherwise 1. Required 1–32 `points`:
  `bar` 0…4096, nondecreasing (equal allowed); finite `value` (normalized 0…1);
  optional `curve`: linear/default, easeIn, easeOut, easeInOut, hold.
  Duplicate target pairs are rejected; points need not fit within lane length.
- Strict validation is the transaction preview contract. Patch loading uses
  defensive `parseData`, not this validator; do not describe malformed legacy
  loads as strict rejections. Raw finite double values can overflow on float
  conversion. Serialization writes defaults and enabled steps, but omits
  trailing tracks without enabled steps, including their voices/glide settings.
  It stores selected name, never active/pending playhead, clock history or RNG state.

## Engine timing and outputs

- Output IDs: pitch tracks 0–3, gate 4–7, velocity 8–11, accent 12–15,
  clock 16, reset 17. Each track group has the track's configured 1–4 channels;
  pitch repeats notes modulo note count, `/12` V (0=C4); gate/accent are 0/10 V,
  velocity is `vel*10` held across rests/stops. Accent follows actual gate.
  Clock/reset are mono 10 V pulses lasting `sampleRate*.001` countdown samples.
- Internal first running sample starts current step; later elapsed samples
  advance when `>= stepLen`. Base length is `rate*60/(max(1,bpm)*stepsPerBeat)`;
  swing clamps to 50–75%, multiplying even/odd steps by `2*s`/`2*(1-s)`.
  Fractional lengths are compared against integer elapsed samples per step.
- External clock overrides internal advance and swing: one Schmitt rising edge
  per step, channel 0, 0.1 V rearm/1 V trigger. First edge begins current step.
  First measured gap is ignored; tempo supplies its length. Later edge periods
  determine gate/ratchet length. A gap >2.5 times accepted period is ignored
  unless two consecutive gaps agree within 5% of the new gap. Clock tracking
  continues while stopped; disconnect resets edge count, not trigger state.
- Stop clears active gates and started flag but holds step/elapsed position;
  resume replays current step. Glide increments still run per sample even
  while stopped/resting, so held pitch can finish moving. Beat fraction clamps
  to 1 during external stalls; stopped publication drops the fractional beat.
- Reset is consumed next process sample: pending pattern lands, step/loop/beats
  become zero, tracks deactivate, 1 ms reset emits; running internal clock
  starts step 0 that sample, external waits for edge. Existing clock-period
  history is retained. Reset does not explicitly clear an existing clock pulse.
- Seek floors beats to a step (epsilon 1e-9), computes loop and step in active
  pattern, rearms start, clears gates/ties and clock/reset pulses; no reset
  pulse. Pending selection remains queued. Engine clamps beats to 0…5e6;
  module seek clamps bars to 0…1e6, public seq.transport accepts 0…100000 bars.
- Probability is stateless SplitMix64 hashing of seed, active pattern index,
  loop, step and track; top 24 bits /2^24, fire iff draw < prob. Ratchets divide
  step length evenly. Gate starts after 1 ms low gap, width
  `max(.01,gate)*subLen`; widths are not capped to available substep remainder.
  A previous successful slide removes first-subhit gap and enables linear
  per-sample pitch glide; slide holds final subhit high through step boundary.
  A failed/rest step breaks the tie. Zero notes retain prior pitch/glide state.

## Selection, edits and module/host boundaries

- Requests queue by index. `now` means next **advance**, not same sample or
  initial begin; otherwise switch at old pattern wrap. Changed pattern resets
  loop count but retains advanced step if it fits new length and continuous
  beat total. Reset consumes pending immediately. Only upper bounds are
  sanitized by process; callers must prevent negative invalid indices.
- `dataFromJson` runs with DSP paused by Rack's exclusive engine lock, replaces
  fixed data and resolves lane ParamHandles, preserving engine transport/RT.
  Edits can change channel count immediately; new step content is latched at
  next start. Out-of-range active/step values are repaired on next process.
  First load queues differing selected pattern with now=true, subsequent loads
  queue at wrap. Despite the source comment, fresh selected pattern !=0 still
  plays pattern 0's first step until advance unless reset consumes the queue.
- Params: BPM 20–300/default120; swing50–75/default50; run0/1/default0;
  reset button. Process consumes reset, seek, pattern requests, runs engine,
  writes CV, then evaluates lanes every32 samples only while running.
  Lane position is engine beats/4; negative clamps0, looping uses fmod length,
  endpoints hold; destination point's curve shapes each segment (cubic easing).
  Equal bars retain order with endpoint behavior determined by source comparisons.
- Handles skip missing targets/invalid parameters. Normalized maps linearly
  through ParamQuantity min/max (fallback0…1); raw still clamps; snapped params
  round after clamp. Direct `Param::setValue`, no smoothing; writes only when
  change from cached lane value is >=1e-7 (cache invalidated on data load).
  External edits may persist when lane's cached value is unchanged. Assigned
  lane targets are fingerprint-masked even stopped. Target DSP visibility
  depends on Rack module scheduling; pure engine does not establish same-frame
  destination execution, competing writers, or native host automation policy.
- Status atomics publish every64 samples; transport responses can precede DSP
  consumption. Listen passes every input channel unchanged each sample; armed
  capture sums polyphony into stereo ring (unconnected right=0), counts drops,
  and is independent of sequencer run. This is not the host WAV conversion or
  wall-clock arm/drain boundary. Bridge DSP only raises pairing-reset request
  and publishes lights every512 samples; host owns real control/session work.

## Native Score divergence and bounded acceptance

`crates/openpatch-core/src/score.rs` has one pattern, default length64/run1,
velocity1/glide0, no accent, 16 CV-output automation lanes rather than 8 target
lanes, snake_case schema/curves and different lane bounds/defaults. Changed
compiled Score data resets transport; equal-data adoption preserves it.
Score lacks source pattern-index probability and source seek/selection state;
glide arithmetic and clock detector lifecycle also differ. It is not an alias.

Acceptance fixtures (execute separately against pinned source and actual host):
1. Sparse/default save/reopen, strict invalid transaction atomicity, and omitted
   trailing-track settings; distinguish permissive patch parse from preview.
2. Two named patterns: initial nonzero selection, now/wrap/reset switching,
   midstep edit/undo and shorter-length adoption with gate/pitch observations.
3. Internal swing plus stop/resume/reset/seek; external first edge, stall and
   two-gap tempo recovery, including reset/seek coincident with an edge.
4. Fixed-seed loops across pattern indices, poly note cycling, accent, drum
   steps, failed slide probability and ratchet/glide boundaries.
5. Lane duplicate bars/curves/loop endpoint, raw/normalized/snap/missing target,
   unchanged-cache external edit, every32 scheduling and every64 status lag.
6. Real sequence→voice→Listen excerpt: target DSP ordering, poly stereo sum,
   arm/drop/drain boundaries and saved reopen; preserve full unsupported patch.

Implementation checkpoint: separate fixed-capacity Rust primitives now pass
the [actual C++ engine comparison](../scripts/rackmcp_sequence_reference/README.md):
18 cases / 620,730 frames, all compared bits equal. Native strict compilation
also preserves all 13 lanes in the two actual saved sequencers. That frozen
proof covers the engine/parser/curve primitives, not the module wrapper or
whole-patch playback.

## Native adapter checkpoint

`rackmcp_sequencer` is a separate version-2 native model. Its optional
`Node.rackmcp_sequence` contains strict source `data` and native `targets`.
Absent state creates a stopped, silent 16-step pattern A. Native bindings name
`source_module_id` / `source_param_id` plus a native `module` / `parameter`;
matching uses the source pair rather than array order. Compilation resolves
bounded handles once. Missing destinations retain their metadata and skip
writes; a present destination without the declared live capability rejects
compilation. RackMCP 2.0.0 import now enables this model with the source-package
checks described below; unsupported lanes remain authored but unbound.

The adapter implements source Schmitt startup, connected-clock detection,
polyphonic pitch/gate/velocity/accent groups and mono clock/reset outputs. Its
32-sample lane divider runs while stopped; assignments occur only while running,
with the source normalize/clamp/snap arithmetic and acknowledged-write cache.
Native node order determines whether a destination sees an assignment this
sample or next sample; cable signals retain native delayed scheduling. This is
a declared host boundary rather than Rack scheduling equivalence.

Compatible graph adoption preserves transport/detectors/divider/cache, including
across sample rates and node reorder. Receiving controls/data remain owned by
the receiver. Changed source data reloads selection and invalidates lane cache;
equal unrelated rebuilds do not. Target disappearance/recreation retains the
last successful lane value, matching the actual source; an initially missing
target still has an unwritten NaN cache. Initialize does not invent an onReset
event: the pinned module has no onReset/onSampleRateChange override.

`set_rackmcp_sequence`, explicit sequence presets and their undo/redo transitions
now carry data-load intent through graph activation, including equal data.
Parameter-only presets and unrelated edits do not reload. One atomic native
transaction loads its final state once per module; it does not replay intermediate
JSON states. Final loads retain their action order, including creation and
duplication: loading A, then B, then A publishes B followed by A. Undo restores
these final module states in reverse order; redo restores the forward order.
This is native atomic-transaction behavior, not replay of intermediate Rack JSON.
Prepared plans freeze requested `(revision, order)` epochs; a separate per-incarnation
applied high-water mark prevents stale plans from replaying consumed events.
Fresh constructors and changed-data adoption consume a load once. Compatible
equal-data adoption applies the explicit event without resetting transport.

Focused native tests cover all 18 outputs, destination values and process-order
visibility, bypass freeze/resume, edits, adoption, publication, save and history.
Actual callback processing/adoption is allocation/deallocation-free in the tested
workflows. The dedicated Signal Atlas rack/browser panel displays authored
selection/notes/drum hits/accents, never an invented live playhead. Direct
transport/seek/status publication,
Bridge and Listen capture remain open; this is not whole-module or whole-patch
qualification.

## Genuine wrapper finding and deliberate host correction

The additional [unchanged module/SDK harness](../scripts/rackmcp_module_reference/README.md)
compares all 18 output widths and canonical samples plus actual SDK parameter
writes over nine cases / 25,104 frames. All 7,681,824 output words match exactly.
The **raw full-module comparison fails** in three target-write cases: repeated
equal data, changed data and reload after target recreation. It remains failed.

The real SDK 2.6.6 registry audit confirms that a second same-target load leaves
the target module registered and serialized source data intact, but changes the
existing ParamHandles to module ID -1. A third load rebinds them. This is an
observed interaction of the pinned module and SDK, not a substitute registry.
OpenPatch deliberately keeps same-target rebinding idempotent so pattern edits,
presets and undo do not detach automation. It remains a source-derived native
sequencer with documented host binding semantics, not a whole-module equivalent.

A separate strict correction audit checks every trace word. Every unaffected
word must remain equal; all 1,964 differing words must occur in declared frame
windows and equal explicitly specified source/native values. It never masks
columns, changes tolerances, modifies the source, or turns the raw report green.
See [machine-readable evidence](benchmarks/rackmcp-sequence-loads.json).

A separate real two-sequencer probe passes six stages / 192 frames: the later
claim displaces the former owner, reloading a displaced writer can reclaim it,
and deleting the winner does not automatically restore the former writer.
Native exclusive ownership now uses prepared native module/parameter keys and
fixed lane slots. Only the owner may write or acknowledge a lane's cached value.
Cold standalone graphs acquire in node/lane order; later loads acquire in
document action order, independent of DSP traversal. Fresh constructors and
changed data already perform their load; the host's subsequent ordered acquisition
does not reload them again or alter first-advance pattern selection.

Compatible node reorder, rate changes and unrelated edits preserve ownership.
Missing targets retain claims and caches. Deleted or retargeted winners leave
vacant claims; displaced writers do not automatically recover. An explicit
load can reclaim a target. Healthy incumbent reloads retain ownership as the
declared correction to SDK self-unbinding.

Before adoption, the desktop rejects graphs below its persistent revision
high-water mark when either graph contains a RackMCP sequencer. That fence
survives sequence removal and intervening older non-sequence graphs.
Already-prepared graphs remain valid in forward
order; a late plan cannot restore old data, claims or a deleted incarnation.
Rejected graphs use the existing retirement queue, retaining ownership locally
if it is full rather than freeing memory in the callback. Equal-revision
rebuilds remain supported. Focused Core and actual callback tests cover these
native policies, including allocation/deallocation checks; they do not establish
general equivalence to Rack's scheduler.

## Source import and automation authority

The registered `vcv/RackMCP/Sequencer/2.0.0` mapping, translation version 1,
is labeled `musical_role`: its bounded source-derived timing and outputs retain
the declared native scheduling and healthy-binding corrections. It is not a
whole-module equivalence claim. Bridge and Listen remain separate unresolved
dependencies; import never starts a listener or audio device.

Import retains every typed source lane and creates native bindings only for
qualified Fundamental 2.6.4 VCA-1 parameter 0 (`stock_vca.level`) and VCF
parameter 0 (`rack264_vcf.frequency`). Other source targets remain unbound,
with their original IDs, points and curves intact.

Every accepted document enforces imported automation authority. An existing
native destination must belong to the sequencer's exact source package, retain
the lane's original numeric module ID, and have the registered source mapping,
native kind and exact parameter conversion. Native IDs are not parsed as source
identities. An explicitly named qualified duplicate can be bound; validation
never searches for another matching module. Pure native sequences (`source:
None`) retain their explicit native bindings.

Missing destinations remain dormant with their bindings preserved. Reusing the
destination's native ID for an unrelated module rejects the entire edit/load,
including a remove/recreate batch. Clear the binding before that replacement,
or retain the target and give the new module a fresh ID. Clearing a binding
leaves an unbound lane until it is explicitly resolved or removed. Arbitrary
native authoring from an imported sequence is not granted by a caller-supplied
fidelity label.

`omitted_automation` lists every unbound, missing or unresolved imported lane in
the state carrier, projection and shared execution status. Exact playback waits
even when no module is unresolved; explicit Partial playback runs the supported
graph with these omissions disclosed in the UI and MCP. The same complete-document
validation protects audio tickets, offline rendering, history, replacement and
save/reopen without filtering or rewriting authoritative sequence state.
