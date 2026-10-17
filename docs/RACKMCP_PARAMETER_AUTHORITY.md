# RackMCP parameter automation: implementation decision

Concrete follow-on to the [sequence contract](RACKMCP_SEQUENCE_CONTRACT.md).
The source engine primitives are qualified; the host behavior below is **not yet
implemented**. Preserve the [shared document authority](COMPATIBILITY_DECISION.md).

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
