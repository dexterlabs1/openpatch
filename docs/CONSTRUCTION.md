# Visible patch construction

Agents can build a patch through real, paced transactions. The native rack shows
the current step count and **Pause**, **Resume**, and **Cancel** controls. Each
completed step is a graph revision and one ordinary Undo entry. This operates on
existing module kinds; it does not generate new DSP modules.

New construction cables briefly reveal along their actual rack curves. Both
plugs and a faint full route appear immediately because the connection is
already committed. Hover and pickup show the normal cable without the reveal;
editing remains available throughout. **Animate cables** disables this effect
immediately and remembers the choice for the current window.

## Submit a plan

Read `inspect_patch` first, then send `start_construction` with that revision.
For an empty patch at revision 0, this adds a source, a level control, then a
connection and output selection:

```json
{
  "expected_revision": 0,
  "title": "Build a signal path",
  "interval_ms": 500,
  "steps": [
    {"label": "Add source", "edits": [
      {"tool": "add_module", "arguments": {"id": "source", "kind": "constant", "params": {"value": 0.25}}}
    ]},
    {"label": "Add level control", "edits": [
      {"tool": "add_module", "arguments": {"id": "level", "kind": "gain", "params": {"gain": 0.5}}}
    ]},
    {"label": "Connect signal and select output", "edits": [
      {"tool": "connect", "arguments": {"from": "source", "to": "level", "input": "signal"}},
      {"tool": "set_output", "arguments": {"id": "level"}}
    ]}
  ]
}
```

The constant is a simple control signal, not an audible oscillator. Use `catalog`
to discover musical sources and their ports. The plan does not start playback.

Every intermediate graph is validated on a detached snapshot before the plan is
accepted. That work releases the live session lock. Registration rechecks the
revision and rejects competing plans; no edit is committed during preflight.
The first step waits the requested interval. Each later interval starts after
the preceding step finishes, so delayed workers do not produce catch-up bursts.

Limits: 1–64 steps, 1–8 edits per step, at most 256 edits total; title up to 128
UTF-8 bytes and each label up to 160 bytes. Intervals are 100–2000 ms (default
500 ms). Only the graph operations accepted by `apply_edits` are allowed. Nested
transactions, file operations, playback, live triggers and history commands are
excluded. Only one running or paused plan is allowed per session.
On attached socket connections, preflight shares the process-wide limit of two
expensive jobs with offline renders and analysis. A busy response makes no edit;
retry after an active job finishes.

## Inspect and control

The start result contains a `plan_id`. Use it for subsequent controls:

| Tool | Arguments | Effect |
|---|---|---|
| `inspect_construction` | Optional `plan_id` | Latest plan summary, or `idle` before any plan |
| `pause_construction` | `plan_id` | Stop before the next atomic step |
| `resume_construction` | `plan_id`, `expected_revision` | Resume a paused plan only if its revision and the caller's revision match the rack |
| `cancel_construction` | `plan_id` | Retain completed work and stop future steps |

Progress includes `status`, `completed_steps`, `total_steps`, `base_revision`,
`expected_revision`, `current_revision`, `last_revision`, `last_label`,
`next_label`, and `error`. Status is `running`, `paused`, `completed`,
`cancelled`, `conflicted`, or `failed`. Only the latest plan summary is retained.

Pause and Cancel serialize with a step: a step already executing may finish
before the control returns, but no subsequent step starts after a successful
pause/cancel receipt. Cancellation retains completed edits; Undo reverses each
step separately. Existing history retains at most 128 snapshots, so construction
can evict older history like ordinary editing.

Any external graph revision, Undo/Redo or committed module move stops the plan
as `conflicted`. Human edits are retained. A conflicted plan cannot resume:
inspect the current rack and submit a newly reviewed plan. Pausing does not
reserve the rack. The native New/Open/Close document flow pauses running work
before capturing the document; dismissing its confirmation leaves work paused.

## Lifetime and current boundaries

The plan belongs to the shared session, so disconnecting an MCP client does not
cancel it while the desktop or another owner retains that session. Closing the
session stops future steps. Plans are not saved in projects or restored after
reopening. Offline renders and cloned sessions do not inherit construction.

Progress reports **committed graph edits**, not confirmation that the audio
callback has applied them. The control worker advances independently of GUI
repaints. A stalled or hidden window can miss intermediate frames; this is not
a guarantee that every step was displayed. The cable reveal is edit feedback,
not an indication of signal propagation or audio timing. It runs for 280 ms and
tracks at most 128 new cables simultaneously. Larger batches display remaining
cables normally; they are never queued for later animation.

Only a single consecutive construction revision observed by the UI can start
a reveal. Skipped revisions, human edits, project replacement and Undo/Redo
clear active reveals and show the current graph immediately. Moving, recoloring
or promoting an existing cable does not start another reveal. A hidden window
does not replay expired effects. Turning animation back on affects future
commits only. Speaker playback remains a local user action. Audio-application
acknowledgements, subscriptions and creating new DSP modules remain future work.

Headless tests cover real MCP dispatch, transport disconnect, transaction
preflight, history, conflicts, lifecycle and rendered panel button interaction.
Cable tests cover curve subdivision, unchanged static rendering, revision
eligibility, bounded lifetime, stacked identities and connected-jack pickup.
Native macOS window/input and device-playback qualification remain outstanding.
