# RackMCP parameter automation: implementation decision

Concrete follow-on to the [sequence contract](RACKMCP_SEQUENCE_CONTRACT.md).
The source engine primitives are qualified. Core target capabilities, coherent
publication, project-save overlays and explicit parameter history/adoption are
implemented for the two current capabilities. Native sequence dispatch and explicit
data-load intent are implemented; exclusive lane ownership below remains open. Preserve
the [shared document authority](COMPATIBILITY_DECISION.md).

The control document records authored intent. The active engine owns effective
parameter values. Automation submits no patch, revision or undo entry. Graph
recompilation inherits effective values unless an operation explicitly assigns
those parameters. Never implement automation as repeated control-thread edits.

## State and capability boundary

Add dedicated `Node::rackmcp_sequence: Option<SequenceData>`, restricted to the new
model; leave annotations and native Score alone. Compile off-thread. Carry data
assignment intent separately: explicit data loads/edits, including equal data,
adopt the data and invalidate lane caches. Unrelated graph adoption preserves
transport, clock detectors, divider phases, pending actions and caches. Retire
compiled allocations through existing graph retirement, never in the callback.

Add typed parameter descriptors and model getter/setter dispatch. Begin with
`StockVca.level` and `Rack264Vcf.parameters[0]`: eight and two actual fixture
targets respectively. More native models extend the same interface. Source
parameter IDs map to native parameter keys in the pinned import mapping. Keep
one mutable effective value owner, preferably the existing model field.
Register persistent capability slots even when no lane currently targets them;
removing the final lane must not restore an old authored value.

Resolve each sequencer's at most eight target plans off-thread to target index,
incarnation, typed handle, bounds and snap policy. Resolve source IDs using
retained provenance, not coincidental native ID reuse. Unresolved targets retain
their source data and have no invented writes. Three actual lanes target Surge
models which remain unresolved.

## Processing and authority

Process a sequencer at its normal native node position. End that node borrow,
then apply a fixed stack buffer of at most eight writes before processing the
next node. Later nodes see the new parameter that sample; earlier nodes see it
next sample. Competing writers follow native traversal/lane order. This is a
declared deterministic native policy, **not proof of Rack scheduler equivalence**;
native cables already deliver their previous sample.

Advance the lane divider while stopped; evaluate only while running. Preserve
source float mapping/clamp/snap and per-lane `1e-7` cache. Update that cache only
when a valid target is written. A manual edit does not invalidate it: an
unchanged source lane intentionally allows that manual value to persist.

Extend coherent performance/project capture with effective values, ownership,
incarnation, revision and frame fences. UI/inspection read that publication.
Save overlays a clone at one coherent boundary; it does not mutate live history.
Only the active graph publishes authority, never a crossfade donor. Publish
independently of the source sequencer's slower position packet.

Carry explicit per-parameter assignment intent, following `control/running.rs`'s
state assignment precedent. Inherit donor effective values first, then explicit
set/preset/initialize/load/history assignments win, including equal values.
Parameter handoff survives sample-rate changes even when DSP history resets.
Capture effective values for history/suspension; undo restores assigned fields
while unrelated fields continue. Full document load assigns saved values.
Bypass/stop/suspension and lane removal never restore stale defaults.

Mask assigned target values in observational dirty fingerprints even stopped.
Keep authored lane edits and explicit user transactions in history. The real
Bridge session and Listen capture contracts remain separate requirements.

## First acceptance workflow

Use the shared session for a real sequencer→VCA/VCF excerpt. Check 32-sample
writes in both node orders, unchanged-cache manual edits, unrelated recompiles,
lane removal, edit/undo/redo, coherent save/reopen, suspension and target
deletion/recreation. Observe actual rendered audio and effective readback,
unchanged automation history and unresolved-target preservation. Check zero
callback allocation/deallocation through processing, adoption and retirement
saturation. Then integrate panels/import and live Listen, before milestone CI.

## Implemented checkpoint

Core resolves generation-bound handles and native raw descriptors for VCA level
and the older VCF's frequency. Direct writes change actual DSP fields. Explicit
effective-value inheritance supports receiving assignments, fresh exclusions and
rate changes independently of DSP history; callers must still supply assignment
intent. The desktop now invokes effective-value inheritance after DSP handoff.
Core's ordinary DSP-only adoption API retains its existing receiving-parameter
policy; the desktop supplies the separate assignment-aware step.

Desktop parameter values share the module meter's serial, incarnation, revision
and callback frame. Capability bindings are prepared for untargeted modules too.
The current publication transport supports eight values per model and refuses
larger capability sets explicitly. No second mutable DSP value bank is created.

Native project save uses one coherent backend observation for saved state and
effective parameters, validates identity/revision/frame/raw bounds, and overlays
only a cloned document. Legacy backends retain their explicit authored-value
behavior. Lightweight dirty observation remains unchanged. A real headless
callback→shared Session→file→fresh reopen test preserves the effective values and
rendered gain without changing live revision/history. Callback publication and
saturated graph retirement use no heap activity.

Shared transactions now capture only the effective fields they intentionally
assign. Equal values, presets, initialization, creation, full loads and removals
carry explicit intent; previews and rejected edits change no backend authority.
Undo restores the affected fields while unrelated parameters continue. Grouped
gestures retain their original effective undo value and final authored redo.
Coherent current-revision publication is required when capturing live history;
unavailable values are never guessed from stale authored parameters.

Patch/assignment synchronization shares one backend registry lock. Per-field
epochs persist across ordinary synchronization and are frozen into each prepared
plan. Adoption compares receiver/donor epochs, so skipped plans and equal-value
assignments retain their meaning. An old plan already prepared remains valid in
forward revision order; sequencer-containing graph rollback is rejected before
DSP adoption, with rejected graphs retained through retirement backpressure;
late preparation after newer parameter metadata is explicitly rejected. The
callback uses graph-sized scratch allocated on the control thread.

Actual headless callback checks cover manual/equal assignments, unrelated
recompiles, frozen/skipped plans, rate changes, deletion/recreation, host resets,
shared Session edit/undo/redo/save/reopen and suspension/resume, with no callback
heap activity. These are native parameter authority checks. Exclusive lane
ownership now follows ordered document loads and preserves vacant claims after
winner deletion. Source-provenance import and a real shared-session callback
workflow are now qualified below; Bridge/Listen and direct transport/status
publication remain open.


## Observed SDK binding boundary

The real RackMCP module/SDK 2.6.6 comparison found same-target data loads
self-unbinding their ParamHandles; a third load rebinds. Native binding remains
idempotent as an explicit host correction. The raw comparison stays failed;
all 18 output words and unaffected target values remain exact, and a separate
strict audit checks the declared source/native differences without ignoring
columns. [Evidence and contract](RACKMCP_SEQUENCE_CONTRACT.md#genuine-wrapper-finding-and-deliberate-host-correction).

The separate two-sequencer source probe establishes exclusive ownership:
takeover invalidates the previous claim; deleting the winner does not revive
the displaced writer. Native arbitration now compiles bounded claim identities
outside the callback, preserves live claims over unrelated rebuilds, and applies
explicit acquisition events in document action order. A prepared
plan's requested event and its live incarnation's consumed high-water mark
are distinct. Do not let graph node order become an accidental ownership rule.
Complete-document validation now fences imported identities by source package,
original numeric ID, registered mapping/native kind and exact live parameter.
Unsafe existing bindings reject edits and loads atomically; absent targets stay
dormant and all unbound lanes appear in `omitted_automation`. Exact waits for
those omissions; Partial is explicit. These invariants protect the authored
carrier shared by audio and offline MCP without silently rewriting source state.
The actual saved patch has 13 distinct source target pairs across its two
sequencers, but that absence of collisions does not qualify general ownership.
