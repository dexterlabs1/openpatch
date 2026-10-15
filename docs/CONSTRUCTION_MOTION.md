# Visible agent construction

An MCP client can use `start_construction` to build a patch as real, successive
steps. Each successful step changes the shared patch immediately and creates one
Undo entry. The native rack provides brief motion cues for those exact commits:
modules arrive, cables draw from source to destination, and parameter changes
get a control accent with their previous and current values.

Use one meaningful action per step for a deliberate pace. A step may group edits
when they must validate together. The default interval is 500 ms; a plan has at
most 64 steps, each with at most eight edits. Pause stops future steps between
transactions. Cancel retains completed work; Undo reverses it. Playback remains
under the user's control.

## Motion controls

**View → Agent motion** and the construction strip offer **Full**, **Reduced** and **Off**:

- Full gives a fixed-position module arrival over 320 ms, a source-to-destination
  cable reveal over 300 ms followed by a 120 ms destination settle, and a 360 ms
  parameter accent. A quiet header cue identifies the affected module.
- Moving an existing cable uses a 420 ms unplug/travel/settle gesture: 55 ms
  lift, 290 ms eased travel and 75 ms settle. Input and output ends can move,
  including both ends in one validated transaction. The curve follows current
  pan, zoom, module positions, cable tension and independent plug ordering.
- Reduced uses a static accent or value-change badge for 300 ms.
- Off immediately settles the display. Re-enabling affects future commits.

These are presentation effects. The actual graph, saved numeric values, occupied
sockets and hit targets already reflect the committed patch. Parameters sent to
DSP are not interpolated by this feature. A faint complete cable route and its
real plugs remain visible during the reveal. Hovering or picking up a cable
shows its ordinary complete form. Editing a highlighted control yields to the
user immediately. Motion does not move the camera or change selection.
The final cable route, sockets and signal lights are already correct during a
move. Traveling plugs are passive decoration and do not emit fake signal lights.
Pointing at or picking up the cable immediately ends its decorative motion.

Control anchors come from the actual current renderer, so they follow pan,
zoom, panel size and control mappings. Where a specialized control has no anchor,
a named header badge describes its change. Discrete choices do not sweep through
invented intermediate values. Browser previews stay neutral.

## Commit receipts and expiry

`inspect_construction` includes an additive `last_commit` object after a
successful step, otherwise null. It contains `plan_id`, zero-based `step_index`,
`before_revision`, `revision`, `label`, `age_ms`, `invalidate_existing_cables`,
`new_modules`, `new_cables`, `moved_cables` and `parameter_changes`.

Targets are derived from validated before/after graphs, including catalog
defaults. Equal values, hidden state and initial parameters of new modules do
not produce redundant value cues. Repeated parameter writes collapse to the
initial and final values. Module removal/recreation is an arrival. Existing
cable IDs animate a move only when a real `move_cables` operation preserves their
identity and changes their route. `moved_cables` contains each ID and exact
`before`/`after` source, output, destination and input. Disconnect/recreate,
replaced modules, discarded cables and net no-ops never imitate a move;
retirement still invalidates earlier cable reveals.

Direct MCP `move_cables` calls and transactions also publish `last_move` through
`inspect_construction`, including when no construction plan exists. It contains
`before_revision`, `revision`, monotonic `age_ms` and up to 128 `moved_cables`.
The UI prefers the matching construction receipt, so one commit never animates
twice. Failed edits do not restart the age; unrelated revisions and history clear
the direct receipt. A manually completed GUI move consumes its own exact revision
without suppressing a later MCP move, including when GUI sync was temporarily busy.

There are at most 128 visible targets per receipt, shared across all target
types. Overflow displays normally. The UI checks exact plan, step and revision
identity and validates targets against the graph. Unrelated edits, history,
revision gaps, new plans and invalid receipts clear stale effects. Receipt age
comes from a monotonic commit clock; stale work does not animate as fresh when
a hidden window returns. Already committed effects can finish after Pause,
Cancel or completion. There is no historical presentation queue, and a stalled
window is not guaranteed to show every intermediate step.

## Validation

Backend tests cover effective defaults, equal and repeated writes, hidden state,
recreation, shared bounds and receipt lifecycle. Pure presentation tests inject
monotonic time and check identity, deduplication, age, expiry and motion settings.
Headless rendering tests cover exact cable prefixes, final curves, stacked and
polyphonic geometry, control anchors, fallback badges and interaction takeover.
A real atomic worker commit compares final graph/history and layout with
Full and Off modes. Controlled worker tests separately verify progressive steps. Linux native captures inspect several controlled animation
phases, with the captured receipt’s presentation age normalized to zero under
the fixture clock. The authoritative host receipt is unchanged. These checks do not qualify native macOS input or audio-device playback.
