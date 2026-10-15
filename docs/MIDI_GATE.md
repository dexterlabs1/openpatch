# MIDI → Gate: implementation and acceptance contract

Status: implemented with bounded actual-class DSP and production-path tests.
This contract, including numerical gates, was fixed before the first production
comparison; full platform and native/device qualification remain separate gates. Existing `midi_cv` and
`midi_cc` remain compatible. New version-2 kind: `midi_gate`.

The behavioral oracle is the actual
[Rack MIDI_Gate class at 8ea470f](https://raw.githubusercontent.com/VCVRack/Rack/8ea470f343058ade6f75f16d503a1eb330e1700c/src/core/MIDI_Gate.cpp),
SHA256 `c83b822aaacea3902d8f9cc85eeff9346ac8e95767e3f2632b8cb0556947a347`.
The [Core manual](https://vcvrack.com/manual/Core#MIDI-Gate) describes note gates,
minimum-duration triggers and velocity mode. Production Rust/panels are original;
the GPL reference class is compiled temporarily, never bundled in the app.

## Signals and assignments

No inputs; sixteen outputs `gate1` through `gate16`. Saved integer `note1` through
`note16` range −1 (unmapped) through 127. Defaults in panel row order are
`48,49,50,51,44,45,46,47,40,41,42,43,36,37,38,39`.
Arrays/presets resolve collisions in cell order, last cell wins. Direct edits and
Learn make the chosen cell win and unmap any other owner in the same transaction.
`velocity` and `mpe` are Boolean, default zero. `channel` is human 0/omni..16,
default omni. The hardware endpoint is selected globally by OpenPatch, labeled
explicitly in the panel; per-module Rack endpoint selection remains a parity gap.

Ordinary MIDI NoteOn with velocity 1..127 sets the mapped cell's gate true,
restarts its f32 gate timer at 0.001 seconds and stores velocity. NoteOff and
NoteOn with velocity zero clear the currently mapped gate flag; neither restarts
the timer. Other messages do nothing. Channel filtering precedes note handling
and Learn. Invalid bytes/channels are rejected by the standard decoder.

All gates/timers/velocities are indexed by cell and storage lane. Mono merges
accepted wire channels into lane zero; MPE uses all sixteen wire lanes. Source
semantics are last-event wins, not a held-note ownership/counting allocator:
releasing a note from one accepted wire channel can release the shared mono gate.
Each process sample publishes every output, even disconnected/unmapped ones, with
one lane in mono or sixteen in MPE. A lane is active when its gate flag is true
or its timer is strictly positive. It publishes 10 V, or velocity/127*10 V in
velocity mode, then subtracts f32 sampleTime from its timer. Otherwise it emits
zero. No timer clamping, ceil approximation or independent wall clock substitutes
for this rule. Same-sample NoteOn/Off still emits the remaining minimum pulse;
repeated positive NoteOn restarts the timer and replaces velocity.

Inactive MPE lanes retain gate flags/velocities and their timer values freeze
when mono is selected. Option changes preserve all cell state. Note assignment
changes, including Learn/Undo/unmapping, preserve cell gates/timers/velocities.
Thus remapping a held cell can leave its gate running, and the old note-off no
longer finds it. This source behavior is deliberate; expose Panic to clear it.
Do not silently change this into note-owned gates or retrigger on reassignment.

## Learn, state and host lifecycle

Learn accepts the first channel-accepted positive-velocity NoteOn, including an
already-mapped or repeated note. It changes/collides mappings BEFORE matching
that same message to a cell, so the learned output starts on that event's sample.
NoteOff, zero-velocity NoteOn, rejected channels and unrelated messages cannot
complete it. One cell per module is armed. Arm/cancel are transient, not history.

Saved module state is the normalized sixteen-note array (`MidiGateMappings`);
options live in parameters. Gates/timers/velocities/Learn are transient and are
never saved or copied into a new instance. Compatible same-ID/kind/rate adoption
preserves them; explicit mapping/option edits affect configuration while keeping
cell signal state and invalidating conflicting Learn. Initialize restores default
notes/options and clears all gates/timers/Learn. Reset-state starts signal state
fresh. Ordinary mapping Undo/Redo restores mappings only, never note events.

The source module's `panic()` clears gates and timers while retaining velocities
and Learn. Keep a gate-clearing primitive with this source behavior for reference
comparison. OpenPatch's HOST panic (MIDI Panic, transport stop, disconnect,
overflow, module Panic action and emergency fallback) also cancels armed Learn,
but never discards an already-owned durable completion. Bypass publishes mono
zero, ignores MIDI and clears gates/timers/armed Learn on transitions to avoid
stuck notes. These host safety policies are explicit differences from a claim of
complete Rack lifecycle parity. Module Panic is available while bypassed and
under a full action queue, without allocating or waiting in the callback.

Add `ModuleAction::MidiPanic` / `midi_panic` for a one-shot module-specific Panic,
accepted only by `midi_gate` for now. It does not change parameters or history.
Actual gate activity drives sixteen original panel indicators; no guessed
pending-note lights. Global input and existing QWERTY/on-screen MIDI access must
remain usable and must not steal focus from note-number text editing.

## Shared durable Learn transport

Extend the proven MIDI CC transport instead of creating a second completion
queue/worker. Core exposes `MidiLearnKind::{Cc,Gate}` and a common Copy completion
`{serial,cell,before:[i16;16],after:[i16;16]}`; the existing CC completion type and
Engine CC methods stay compatible aliases/wrappers. Generic Engine methods take
kind explicitly. Gate adds corresponding typed wrappers for convenience.

Desktop uses one fixed mailbox array, serial counter and completion sequence,
with kind-tagged control/bindings and separate CC/Gate configuration signature
variants. Request identity is kind + backend generation + module token + epoch +
Learn serial + cell. Audio atomically claims Armed→Processing BEFORE dispatching
MIDI. Cancellation can only win before that claim. A completed decision becomes
Ready and remains there until Session commits its mapping history and explicitly
acknowledges the exact identity. Ready is not erased by reset/close/queue overflow.
Only the active graph may Learn; crossfade copies still receive ordinary MIDI.
Reset also cancels Armed requests waiting for a future graph revision. Delayed
module Panic and bypass fences capture the last accepted Learn serial under the
control mutex and cancel only tickets through that cutoff. A newer accepted
request survives gate clearing and is armed afterward. Close and global MIDI
reset retain their unconditional Armed-cancellation policy.

Keep legacy CC trait methods and `MidiCcLearnReceipt` unchanged through explicit
adapters. New generic trait defaults dispatch CC calls to those legacy virtual
methods; Gate defaults unsupported. Production overrides generic methods once,
and legacy CC methods become kind-checked wrappers (avoid default recursion).
Generic receipt adds `kind` and `backend` to the existing identity/payload. One
Weak Session worker reconciles both kinds in shared (frame,sequence) order.
Target lookup, deduplication, ack and binding/publication all validate kind. A
same-textual-ID CC→Gate replacement cannot consume an old receipt.

Commands `learn_midi_gate` / `cancel_midi_gate_learn` use human cells 1..16,
expected_revision, and truthful queued request receipts. Existing CC command
names and behavior remain unchanged. All save/capture/preset/duplicate/init/
replace/Undo/conflicting-edit paths apply the same generic reconciliation guards.
Invalid/stale edits must not cancel tickets; completed audio changes must never
be ignored after cancellation wins elsewhere. Mapping-only history commits one
operation for the whole collision. Actual telemetry is `state.assignments[16]`,
`state.learning` null or `{cell:1based,serial}`, with sixteen measured lights.

## Predeclared verification

Compare raw samples against the actual pinned class at 44.1/48/96 kHz. All sixteen
outputs and every active lane: <=0.00001 V + 0.000001*abs(reference). Exact output
width, maps, Learn completion sample and gate/pulse edge samples. No warm-up,
alignment, threshold exclusions or post-hoc tolerances. Preserve source/SDK/
toolchain/flags/production+driver hashes, per-fixture inputs and worst samples.
Uninitialized reference velocity storage is not read before a real NoteOn.

Fixtures cover startup/defaults, all cells/notes/wire lanes, 1/127 velocities,
all sample rates' pulse boundaries, immediate/short/long notes, repeated notes,
zero-velocity releases, equal-sample event order, channel overlap/filtering,
unrelated messages, MPE/velocity/mono transitions, hidden-lane timer freeze,
unmapped/held-cell remapping, collisions, Learn's first-message order, and actual
JSON mapping/options save/restore. Source-only Panic clears gates/timers; host
cancellation/bypass/initialization are separate production tests and documented
policies, never hidden comparison exemptions.

Production Engine/real callback/Session tests must additionally prove first-
callback/adoption/16-lane/mixed-CC+Gate no heap activity, bounded panic fallback,
remapped-held-cell Panic, reset/disconnect/overflow/bypass, capture/history, stale
kind/instance/epoch/backend rejection, simultaneous cross-kind completions and
ack retry. Shared rack/browser panels, all sixteen precise note entries/Tab,
Learn/Cancel/Panic, actual gate lights, darkroom, keyboard focus and a connected
percussion example are UI acceptance gates. Native Mac window/audio/physical MIDI
qualification stays separate; no global parity claim from increasing kind count.

## Reference result and current use

The [recorded actual-class run](benchmarks/midi-gate-reference.json) passes all
63 cases (21 fixtures × three sample rates), 258,048 frames and 56,513,088 active
lane samples. Every binary trace matched bit for bit on the documented Apple
Silicon SDK/toolchain run. No tolerance changes, exclusions or alignment were
introduced. All reference output ports are connected as probes; disconnected
OpenPatch timer advancement is checked separately in Engine tests. This bounded
DSP result does not qualify physical MIDI endpoints or complete Rack host parity.

Open **File → MIDI drum pads** and use the global MIDI keyboard's **Z / X / C**
keys (C4/D4/E4) or send notes 60/62/64. Click a pad's note field to type an integer,
Tab through all sixteen fields, or right-click for Learn/Cancel. **Panic** clears
this module's gates and armed Learn without changing mappings or history. The
sixteen pad lamps and LRN label use actual callback state. Velocity mode changes
the output amplitude; MPE exposes sixteen wire-channel lanes. Hardware endpoint
selection remains global in Audio / MIDI.
