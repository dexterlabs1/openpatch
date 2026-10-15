# MIDI Map: implementation and acceptance design

Status: **DESIGN — not implemented and not frozen.** This document proposes the
implementation boundary and acceptance work for Core MIDI-Map. Root review and
actual-class probes must settle the provisional decisions below before fixtures,
numerical gates or lifecycle behavior are frozen. No passing result is claimed.

The completion target is at least the Core MIDI-Map function: hardware CC controls
eligible parameters throughout the current OpenPatch catalog, with learning,
persistence, effective-value feedback and coherent save/capture authority. A
parameter foundation or a small target allowlist is an intermediate step only.
It must remain explicitly incomplete until every eligible current control and
the authority/capture path meet this contract. Existing `midi_cc` and `midi_gate`
keep their contracts.
The inventory must also cover bounded controls in the source that OpenPatch
currently represents as runtime actions. Representation as an action does not
remove a source-mappable button from the completion target.

## Pinned reference and observed source behavior

The proposed source baseline is Rack commit
`8ea470f343058ade6f75f16d503a1eb330e1700c`, also used for the existing MIDI CC/Gate
comparisons. Core belongs to Rack, not Fundamental. The
[Core manual](https://vcvrack.com/manual/Core#MIDI-Map) describes selecting a slot,
a destination control and a hardware CC, with the two selections possible in
either order. The following are source observations, not OpenPatch test results.

[Pinned MIDIMap.cpp](https://github.com/VCVRack/Rack/blob/8ea470f343058ade6f75f16d503a1eb330e1700c/src/core/MIDIMap.cpp)
has no ports or ordinary parameters, up to 128 slots and a 32-sample divider.
Controller values begin unknown. Eligible targets have bounded quantities. The
first valid slot tick initializes its filter from the destination and skips a
write. Later ticks use CC/127, optional smoothing, destination scaling and
integer snapping. Smooth CC defaults on; a normalized difference strictly below
one smooths, otherwise it jumps. Direct Engine writes avoid a second smoother.
Latched controllers continue driving after messages stop, including over manual
edits. JSON saves mappings, smoothing and MIDI configuration, not CC values or
filter state. Learn requires a changed CC and a target, then advances to an
incomplete slot. Repeated CC assignments are permitted. Clearing resets filters
without clearing `filterInitialized`; reset/remap behavior needs direct probes.

The base [Quantity scaling](https://github.com/VCVRack/Rack/blob/8ea470f343058ade6f75f16d503a1eb330e1700c/src/Quantity.cpp)
is affine in the raw bounded range; display conversions are separate in
[ParamQuantity](https://github.com/VCVRack/Rack/blob/8ea470f343058ade6f75f16d503a1eb330e1700c/src/engine/ParamQuantity.cpp).
Quantity subclasses may override scaling. The
[exponential filter](https://github.com/VCVRack/Rack/blob/8ea470f343058ade6f75f16d503a1eb330e1700c/include/dsp/filter.hpp)
uses forward Euler and snaps to its input when f32 arithmetic makes no progress.
[Engine parameter handles](https://github.com/VCVRack/Rack/blob/8ea470f343058ade6f75f16d503a1eb330e1700c/src/engine/Engine.cpp)
weakly identify destinations. Interactive assignment can displace another target
owner; non-overwriting loading rejects a later ownership claim.
The source quantity getter can read the Engine's pending smoothing target rather
than its instantaneous DSP parameter. Probe initialization during manual Engine
smoothing explicitly; substituting the instantaneous value is a host adaptation.

Recorded source SHA-256 values from the design research:

| File at the pin | SHA-256 |
|---|---|
| `src/core/MIDIMap.cpp` | `52f51951643eeccf4c65d4917221c74c8c74f69f7263d488aa16eb4f1d152a5c` |
| `src/Quantity.cpp` | `bdcc0c812f756d07c4ee1a8a0a02926cc3dbc1a3dde5760d9b1b705abbc9da86` |
| `src/engine/ParamQuantity.cpp` | `ca43a344b4bd9971c7288826ab947864f0804baee30d6bf66ccde4bac09d0b0b` |
| `src/engine/Engine.cpp` | `d404fd4521c90bc5c52b77f6e38d2e132812879734589cdb223c721099c04fcf` |
| `include/dsp/filter.hpp` | `18d4d490eb754259c6c3b335445f9f5369c4a1edfe03ba7f110cf48d0665d8e1` |

## Intended module scope

Proposed version-2 kind: `midi_map`, without signal ports. Preserve 128 mapping
slots, ordinary seven-bit absolute CC, a 32-engine-sample control tick, Smooth CC
on by default, and continuously latched absolute overwrite as the default
behavior. These are the source-style scope decisions for this design. Exact
first-tick phase, queue consumption, initialization, rounding and reset details
remain provisional until genuine-class probes establish them.

Channel configuration uses the host's human numbering: 0/omni or 1–16. CC values
are 0–127; unassigned is a separate mapping state. Unknown controller state must
be distinguishable from received value zero. Accepted wire channels merge into
one controller bank; Map does not create MPE lanes or CV outputs. 14-bit pairing,
relative encoders, pickup/touch modes and the nonstandard signed gamepad extension
are separate extensions, not substitutes for the stated target.

OpenPatch currently selects a hardware input globally. Reusing that ingress is a
proposed first host adaptation and must be labeled global. It does not establish
Rack's per-module endpoint behavior; that difference remains a named gap until
implemented or explicitly bounded in the eventual qualification claim. No claim
of full Core or host parity follows from adding the module kind.

## Parameter authority foundation

Existing Session `set_parameter` validates an authored graph and commits shared
history. MIDI CC generates voltages; SCORE generates voltage automation lanes.
Neither supplies a generic audio-thread setter for arbitrary parameter values.
Existing durable Learn, frame-stamped ingress, revision-coherent telemetry and
project saved-state capture are foundations to extend, not complete Map support.

The implementation must distinguish three authorities:

| Domain | Owner and required behavior |
|---|---|
| Authored parameter/configuration | Session owns validated GUI/MCP edits, revisions, mapping configuration and undo history. |
| Effective parameter value | Audio owns the actual value and resulting DSP coefficients used for processing, including MIDI movement and application of authored edits. |
| Saved parameter value | Capture clones the authored graph and overlays effective destination values from one coherent audio frame, without changing history. |

Controller movement changes effective values without a graph rebuild, graph
revision or undo entry. A manual/MCP value edit is still valid and undoable, but
the next mapping tick may overwrite it with the latched controller value even
without a new MIDI message. Mapping ownership must be explicit in inspection and
native controls. The display must distinguish authored/pending value, actual
value, mapping owner, audio-applied frame and unavailable state.

Coherent project/preset capture, exports that capture current settings, recovery
and dirty observation must use effective values where mappings have changed
them. Hardware motion must not become invisible to an unsaved-change check. A
bounded effective-value/state epoch can support observation, but is not itself
a substitute for a coherent value snapshot. Without an active backend, offline
capture uses the authored initial state and labels that provenance. A pending
or stale live snapshot must cause a retry/unavailable result, not fabricated
values or a mixture of graph revisions and callback frames.
Compare coherent saved value contents against the saved baseline: moving away
and back can restore saved contents despite a newer motion epoch. Deliberate
backend detach must retain the last coherent effective values, including unbound
ones, instead of falling back to stale authored settings. Define the detach frame
and pending-revision behavior explicitly before implementing this transition.

An unrelated graph edit must preserve effective target values and compatible
controller/filter state. An explicit edit to a target parameter carries a control
epoch so that its authored assignment applies after adoption, then follows the
next-tick overwrite rule. Equal-value Initialize, preset/load and restoration
must remain explicit assignments, not disappear as apparent graph no-ops.
Assignment epochs are per parameter and survive coalesced graph publications:
an A→B→A edit sequence must retain its final explicit assignment even when the
callback never observes B. Unrelated edits to another control cannot replace
that assignment fence or reset the whole module's performance values.
Ordinary parameter values also survive Clear, ownership displacement, mapper removal and
mapping Undo. Stopping a controller must not restore a stale authored value.
Keep effective state and assignment epochs for previously controlled parameters
until an explicit value assignment or destination retirement, or materialize the
last coherent value through an equally tested authority transition. Capture,
dirty observation and unrelated graph adoption include these unbound values.
Mapping Undo/Redo changes configuration/ownership while retaining current live
performance values; manual parameter Undo restores the authored value and can
again be overwritten at a later tick. Final ordering needs dedicated tests.

## Eligible controls and normalized destination values

Before implementation is called complete, inventory every current catalog
parameter and its native control. Record eligibility, normalized conversion and
inverse, finite bounds, step/enum snapping, setter opcode and coefficient/state
side effects. Exclusions require a semantic reason such as hidden persistent
state or absence of a parameter; lack of an implemented setter is unfinished
work, not an eligibility rule. All bounded ordinary knobs, sliders and parameter
switches in the current catalog must be covered. Text/content fields and
cable/patch identities are not ordinary numeric controls.

Use a broader `ControlTarget` inventory for source-mappable buttons. Source
[`configButton` quantities](https://github.com/VCVRack/Rack/blob/8ea470f343058ade6f75f16d503a1eb330e1700c/include/engine/Module.hpp#L149)
are bounded and snapped, even when randomization is
disabled. ADSR PUSH, ten PULSES buttons and SEQ3 Clock/Run/Reset are examples
that a `Node.params`-only inventory would miss. Require audited adapters for
these controls, with stable mapping identity, normalized/snap semantics and
their source level/edge behavior. Separate a button's raw control level from
the resulting Run latch, Clock event or envelope state. A latched high CC must
not dispatch a fresh Run/Clock action on every Map tick. Include interactions
with physical/MCP press and release, clear/remap, bypass and graph adoption;
declare each control's capture/load policy after source probes. A scalar-setter
foundation alone does not fulfill the full Map control target.
The unbind-retains-value rule for ordinary parameters does not silently decide
transient held-button cleanup. Explicitly probe and choose clear/disconnect
behavior, shared GUI/MIDI control ownership and stale release fencing; record
any host release policy that differs from the source. Preserve source smoothing
and snapping before adapting a button into a level or edge, rather than treating
every nonzero CC as pressed.

Do not infer mapping eligibility from randomization policy or from the presence
of numeric bounds alone. The DSP setter must update the actual control and all
necessary coefficients without corrupting state. Controls affecting voice count
or configuration require an audited safe path rather than silently being omitted
from the final target. A foundation-stage allowlist may prevent unsupported
writes during development, but cannot satisfy milestone acceptance.

Target transforms are **provisional**. Rack's raw parameter coordinate and
OpenPatch's serialized coordinate may differ: a frequency can be stored in Hz
while the native knob moves logarithmically. One candidate is explicit normalized
control travel shared by UI and engine; another is affine raw-domain mapping.
Probe representative source targets and choose per-parameter descriptors before
freezing fixtures. Do not derive this decision from a display unit, a randomizer
or an incidental UI helper. State every deliberate source-domain adaptation.

For the selected descriptor: normalize CC, smooth in the normalized domain,
convert to the target raw domain, apply declared snapping, and clamp/validate
against the valid target range. Boolean/integer controls must never receive an
invalid fractional state. Sparse enums require explicit ordering and boundaries.
Constant ranges are constant; unbounded or unavailable targets report a reason
and receive no write. Tests must cover endpoints, midpoint and rounding ties,
negative ranges, Hz/log/encoded controls, enum transitions and actual signal or
state consequences for every eligible control.

## Mapping persistence and target identity

Use typed saved mapping data with a versioned schema. The following is only a
conceptual representation; field placement in `Node` and the serialized schema
remain design decisions, not a wire-format promise:

```json
{
  "schema": 1,
  "smooth": true,
  "channel": 0,
  "slots": [
    {"cc": 74, "target": {"module_id": "filter", "kind": "stock_vcf", "parameter": "frequency"}}
  ]
}
```

Saved targets use stable module IDs and parameter names, guarded by module kind;
never compiler indices, widget labels, pointers or UI positions. Runtime handles
add document/backend identity, source and destination incarnations, mapping/control
epochs and compiled-engine generation. Runtime tokens must not be serialized.
Preserve slot holes and partial assignments. Validate lengths, CC/channel/options,
target shape and unknown fields before any live mutation.

A CC may drive several different destinations. A target has one owning mapping
slot across all mapper instances. Proposed interactive assignment atomically
displaces the former target owner and records one shared undo operation, including
the displaced binding. Reusing MIDI CC's unique-CC-number policy would be wrong.
Proposed load collision resolution is deterministic first valid target owner in
saved module/slot order, with rejected later claims reported visibly; exact
source load behavior and host order must be probed.

Deletion immediately invalidates runtime writes. An unresolved record may remain
for history/inspection, but accidental same-ID replacement cannot inherit a stale
binding. The lifetime of unresolved targets across save/reload is provisional:
a persisted suspended/bind-on-load policy or equivalent identity mechanism is
needed if unresolved records survive. Undo restoration of an original destination
and explicit reassignment must be distinguished from unrelated ID reuse.
This protection must survive save/reload: losing a runtime incarnation token
cannot reactivate a suspended reference merely because an ID exists again.

Duplication and module presets need explicit cross-module reference policies.
Candidate behavior remaps internal references when mapper and destination are
duplicated together, leaves external preset references unresolved, and avoids
Map-only duplication silently taking ownership of existing targets. These are
host adaptations under review, not established Rack behavior. Project save/load,
recovery, Initialize, duplication, preset application and Undo/Redo must preserve
or clear references consistently under the eventual policy.

Controller values and smoothing histories are transient for Map. Save the actual
controlled destination values through parameter capture; do not copy MIDI CC's
hidden committed-controller persistence into Map. On a fresh load, unknown CCs
must not force destinations to zero before input arrives.

## Learn, manual selection and MCP transactions

Learn has two halves: a target parameter selection and a changed, accepted CC.
Either order must work. Repeated equal messages, wrong channels, unrelated MIDI
and unavailable/nonparameter destinations cannot falsely complete it. Selecting
a target is explicit intent; ordinary editing must not silently become a target
selection outside an armed session. Native UI must offer the full slot set,
clear/assign operations, current target names, channel/Smooth options, Learn and
Cancel, with actual callback state rather than a simulated learning indicator.

The source applies and saves each accepted partial assignment immediately;
pair completion controls Learn advancement, not whether those assignments
exist. A target selection can displace its former owner before a new CC half
arrives, and rearming an existing row does not clear its old CC. A proposed
atomic paired-commit policy therefore differs observably from the source.
Probe both empty and populated rows, cancellation, save during partial Learn
and ownership changes before choosing this policy. Do not present deferred
paired configuration as source-equivalent behavior.

Arming and cancelling are transient commands. Carry backend, mapper incarnation,
target incarnation, configuration epoch, request serial and slot. Reserve a
durable completion location before arming; the completion includes the pairing,
displaced ownership and applied audio frame. Session reconciles one atomic mapping
history operation independently of GUI visibility. How accepted partial updates
are durably reconciled and grouped remains provisional; completed-pair receipts
alone cannot preserve the source's partial behavior. Existing CC/Gate receipt
payloads are not sufficient unchanged for cross-module 128-slot ownership.

Only the active crossfade graph may learn or publish a decision. An unrelated
MCP edit preserves compatible learning. A conflicting mapping/target edit,
Initialize, preset, Undo, bypass, removal or backend replacement must resolve a
completed decision or cancel an armed request before proceeding. No completed
decision may be lost to queue overflow or a cancellation race, and no stale
receipt may bind a replacement incarnation.

Proposed MCP surface provides atomic assign/clear, arm/cancel and inspection of
mapping ownership/effective values. Exact tool names and schemas are not frozen.
It must preserve expected-revision checks and atomic preview/apply behavior.
Responses distinguish accepted intent from audio application, including actual
frame and unavailable/conflict reasons. Streaming controller values must not be
implemented as repeated Session `set_parameter` calls.

## Audio scheduling, resource bounds and lifecycle

Resolve strings, descriptors, ownership and targets during control-thread
compilation. The callback receives bounded storage, numeric handles and audited
setter opcodes. No allocation, deallocation, locks, JSON/catalog work, module-string
routing, recompilation, file I/O or UI calls may occur during MIDI processing,
mapping writes, initialization, reset or graph adoption.

Retain a source-style 32-sample engine tick independent of device buffer length.
Exact divider phase and event-to-tick latency are provisional until probes cover
frame zero and events immediately before/on/after a boundary. Preserve due event
order for Learn even when several updates to one CC coalesce to the last value
used by a mapping tick. Rendering must split at relevant control deadlines or
provide equivalent internal sample scheduling; it cannot wait for the next
callback merely because no new MIDI event is due.

Candidate host ordering applies explicit control-boundary assignments, consumes
due events under existing FIFO/equal-frame rules, evaluates mapping ticks, then
executes destination DSP and ordinary delayed cables. Performing mapping writes
before destination processing would make behavior independent of compiled module
order. This is a proposed host adaptation, not a claim about Rack's scheduling;
source write/observation frames and crossfade behavior must be measured first.

Resource limits are provisional. Keep 128 slots per module. A possible global
bound is 1024 resolved bindings, validated on the control thread, with explicit
rejection rather than silent truncation. This number is not a source limit or
a frozen acceptance value. Choose storage, global limits and worst-case work
budgets through capacity probes and callback deadline measurements. Existing MIDI
ingress budgets must remain bounded; no overflow recovery may allocate or wait
for Session reconciliation.
Bound mapper count, stored/dormant/partial slots, target-state publication and
ownership reconciliation as well as resolved writes. A resolved-binding limit
alone does not bound scanning work or memory for many empty or unresolved maps.

Compatible adoption preserves controller state, slot filters, initialization and
divider phase plus effective destination values, subject to explicit edit/load
epochs. Fresh incarnation/rate changes require a documented initialization rule.
Each crossfade branch uses handles resolved against its own engine; both receive
compatible due input, but only the active branch owns Learn, publication and save
state. Removed/replaced destinations must be untouchable through old handles.

Bypass, reset, Initialize, device disconnect/Panic and overflow policies remain
provisional pending source lifecycle probes. Candidate host behavior freezes a
bypassed mapper and cancels armed Learn, allows a bypassed destination to receive
parameter writes, and holds effective values on disconnect/Panic instead of
inventing a return to zero. Source filter-initialization retention on clear/reset
must be tested explicitly; an intentional host deviation must be named rather
than silently replacing it with an assumed reset rule.

## Acceptance work before completion

Nothing in this section is frozen or reported as passing. Before the first
production comparison, resolve the provisional decisions, declare deterministic
fixtures and numerical gates, and record their hashes. Do not tune tolerances,
alignment or exclusions to comparison output.

| Gate | Required evidence |
|---|---|
| Genuine source oracle | Compile the pinned class with actual Engine/ParamQuantity targets and genuine smoothing, not just a formula adapter. Probe all CCs/channels, slot capacity, CC fan-out, target collisions, two-order Learn, first ticks, manual overwrite, snapping, unresolved targets, clear/remap/reset and JSON restoration. |
| Numerical/timing | At 44.1/48/96 kHz compare normalized/raw values, filter state, write frames, target identity and Learn/ownership outcomes from frame zero. Discrete decisions must match exactly. Freeze numerical tolerances before comparison; report source/SDK/driver/toolchain/production/input hashes and explicit adapter differences. |
| Complete current controls | Publish an audited inventory of every current ordinary parameter with eligibility reasons. Exercise each eligible setter and its actual signal/state effects, including coefficient, enum and voice/configuration changes. No partial allowlist can close this gate. |
| Engine scheduling | Prove chosen module-order and callback-partition behavior, boundary events, source-style tick cadence, reset/rate adoption, unconsumed targets and deterministic offline MIDI rendering. |
| Session and persistence | Prove actual-value save/capture/recovery/dirty state, mapping/preset persistence, two-sided Learn without GUI reconciliation, manual/MCP/history ordering, duplicate/delete/restore identity, stale backend/epoch rejection and no controller-motion history spam. |
| Real callback | First processing, full binding capacity, continuous movement, reset/overflow/close and graph crossfade must have no heap activity or locks. Compare audio, write frames and coherent owner/value publication across callback partitions; benchmark against the callback deadline with machine and limits recorded. |
| Native UI | Original shared rack/browser representation, all slots and options, eligible target selection, clear/assign/Learn/Cancel, actual owner/value/unavailable states, keyboard focus and readable normal/darkroom captures. Headless UI checks do not qualify native input. |
| Hardware/platform | Cross-platform CI, actual native captures and separate macOS window/input/audio/physical MIDI qualification. Global endpoint reuse and other host differences stay explicit until resolved. |

## Review decisions still open

- Per-target normalized transforms and snap boundaries, including Hz-stored and
  source-encoded controls; the complete eligibility inventory cannot be replaced
  by an implementation convenience list.
- Exact 32-sample divider phase, queue/tick ordering, initialization and repeated
  clear/load/reset semantics after genuine-class probes.
- Mapping-before-DSP host ordering, crossfade write visibility and equal-time
  authored/MIDI authority ordering.
- Global binding/capacity bounds and overload behavior after measurement.
- Saved representation, unresolved reference lifetime, collisions on load,
  duplication and external-reference preset semantics.
- Coherent effective-value save/dirty capture and explicit assignment epochs,
  including manual/MCP Undo while latched control remains active.
- Bypass/disconnect/Panic policy and the remaining per-module endpoint gap.

The implementation can proceed foundation first, but MIDI Map remains incomplete
until all eligible current controls, authority/capture, persistence, learning and
acceptance gates are addressed. Increasing the kind count or reusing MIDI CC
voltage outputs does not complete this Core target.
