# Projects, saving, and recovery

Native projects now keep modules, cables, output selection, module positions,
pan, zoom, and each SEQ-3’s current Run and Push’s HOLD settings in **one `.openpatch` file**. Files live in `~/OpenPatch` by
default, or the directory selected with `--workspace`.

## Everyday controls

| Action | Behavior |
|---|---|
| **New / Cmd-N** | Start an untitled patch from the user template, or the factory analog voice. Clears undo history after the guarded replacement succeeds. |
| **Save template** | Confirm a captured patch as the template for New and future launches. Leaves the current project identity and saved state unchanged. |
| **Save / Cmd-S** | Update the current native project. An unsaved or imported patch opens Save As. |
| **Save As / Shift-Cmd-S** | Create a new `.openpatch` file and make it the current project. Existing filenames are refused. |
| **Open / Cmd-O** | Open a native project or import an older `.json` patch and its optional layout sidecar. |
| **Revert to saved** | Reload the current native project, with Save / Discard / Cancel when changes are unsaved. |
| **Recover unsaved work** | Reopen a recovery copy from a previous session as a separate unsaved project. |

Use Ctrl instead of Cmd on Windows and Linux. The filename field is selected
when the dialog opens; Enter confirms and Escape cancels. Filenames currently
use ASCII letters, digits, hyphens, underscores, and periods, with no directory
components. A standard operating-system file picker remains future work.

The title's asterisk follows saved **content**, including layout and view
changes and live SEQ-3 Run or Push HOLD toggles. Advancing steps and audio frames do not mark
the project changed. Returning to saved content clears the asterisk. A later
Run/HOLD toggle or MCP edit during Save remains unsaved. Open, New, Revert, and Close offer Save / Discard /
Cancel when needed; Save continues the requested action only if no newer edits
remain. Recovery and newly opened projects start with playback paused.

## Reusable starting patches

**File → Save template** captures the native graph, cable identities, committed
layout, view, and coherent Run/HOLD settings when the confirmation opens. Save
uses that captured snapshot even if MCP edits the live patch afterward. Cancel
leaves the existing template untouched. The template is stored separately as
`.openpatch-template.openpatch` in the workspace; it is not a recovery copy or
an ordinary project-list entry. Replacement checks the reviewed file bytes
under a stable lock. If another instance changes the template, review and save
again rather than overwriting that change.

New uses the template, falling back to the built-in analog voice if it is absent
or invalid. Invalid template bytes are preserved for inspection. New starts
clean and untitled with playback paused and fresh undo history, while retaining
host preferences and the module-position lock. Save opens Save As. Existing
unsaved changes still go through Save / Discard / Cancel.

At launch, `--patch NAME.openpatch` explicitly selects a workspace project
(legacy `.json` is also accepted). Invalid explicit files stop startup with an
error. Without an explicit file, previous-session recoveries are offered over
the template or factory starting patch. Startup prepares the document before
exposing MCP, retaining recovery locks throughout; MCP changes made before the
window opens remain current and count as unsaved changes.

## Moving modules

Drag a module header to move it; Shift-click headers to select a group. A
completed move creates one undo step in the same history as parameter and cable
edits. Cmd-Z / Shift-Cmd-Z and MCP undo/redo traverse that shared order. Moves
preserve selection, pan, zoom, and compatible DSP state.

While dragging, Save and recovery use the last committed module positions.
Releasing commits the snapped placement. Escape, focus loss, a modal dialog,
a blocked/no-op drop, or a concurrent graph revision cancels the preview without
adding a move to history. Pan/zoom remain saved view settings rather than
independent undo entries.

## Recovery

When changes are unsaved, OpenPatch writes a separate recovery copy after about
2 seconds without another change, or about every 15 seconds during continuous
editing. Those times are UI scheduling targets, not hard realtime guarantees.
The original project is never overwritten by autosave. Recovery does not mark
work saved.

On startup, a dialog offers copies left by earlier sessions. Recovery loads the
complete graph and layout, pauses playback, and requires **Save As**. The
original project remains unchanged. **Not now** keeps the copies available
under File; **Discard copy** removes that selected recovery. A successful
manual save or an explicit discard clears the current session's recovery.

Each running instance owns its own recovery file through a held lock. Another
instance cannot offer, restore, or delete that active copy. Orphan copies are
claimed at startup; malformed copies remain available for explicit inspection
or discard rather than being silently deleted. Lock companion files are kept
so their identities remain stable across atomic file replacement.

## File and concurrency guarantees

A native file has a versioned JSON envelope:

```json
{
  "format": "openpatch-project",
  "version": 1,
  "patch": { "version": 2, "sample_rate": 48000, "nodes": [], "cables": [], "output": null },
  "layout": { "version": 1, "zoom": 1, "pan": [24, 12], "positions": {} }
}
```

The project format version is separate from the DSP engine version. Importing
or saving a version-1 graph does not change its timing. Unknown document/layout
fields and unsupported versions reject. Native files are limited to 2 MiB;
legacy graph and sidecar reads are each limited to 1 MiB. Nonregular files and
symlinks are rejected.

Save captures one observed graph/layout pair with every SEQ-3 Run and Push HOLD value from
the same completed audio callback at that graph revision. Unavailable, stale,
or mixed-frame live state blocks Save and preserves the previous file and
recovery copy. Recovery retries later. Without an attached audio backend,
capture uses the configured initial Run/HOLD settings. Capture is read-only: it
does not reset steps, oscillator phase, cable delays, or held controls.

Validation, serialization, and
file I/O run on the UI/control thread, without holding the shared session lock
during disk I/O; the audio callback never does this work. The file is written
to a temporary sibling and synced before atomic publication. Failure before
publication leaves an existing project intact. Save As publishes without
clobbering an existing target.

Save-to-current also compares the last read/written bytes with the current
file. An external modification requires reopening or Save As. A stable
companion lock serializes cooperating OpenPatch writers across that comparison
and replacement. An arbitrary external editor that ignores these locks can
still race the final replacement; this is not a filesystem-wide compare-and-
swap guarantee.

On Unix platforms, the parent directory is synced after publication. A failure
at that stage returns the new file token and reports a durability warning,
because publication already happened. Windows currently reports that directory
sync is unavailable; atomic publication still applies, but equivalent
power-loss durability has not been qualified. These tests simulate write
failures; they do not prove every storage device's power-failure behavior.

Open/Revert validate the entire graph and layout before committing. The session
revision is checked at commit; a concurrent MCP edit causes a conflict and
leaves the existing project and view intact. Clean and save-before-action
continuations also recheck projected Run/HOLD content before replacing the graph
or sealing Close. New changes prompt again. Explicit Discard can proceed when
live telemetry is unavailable. Failed saves retain a visible pending decision.

A final Close seals the session against later graph edits and module actions,
while read-only requests and exports can finish. The capture is a completed
callback boundary; sealing does not freeze external clocks or acknowledge
controls already queued for a later audio callback.

## Compatibility and remaining work

Legacy `.json` patches and `.json.layout.json` sidecars import without being
modified. Their first native save uses Save As. An invalid present sidecar
rejects the import instead of partially loading a graph with a bad view.

MCP `save_patch` and `load_patch` remain **graph-only JSON interchange** commands.
They do not read native `.openpatch` envelopes or save the view. Agents can
continue editing the live native session while the user saves complete native
projects through File. Read-only MCP `capture_project_patch` exposes the same
Run/HOLD projection and provenance, without layout or disk writes. Full project-level
MCP operations remain a separate gate.

Native documents preserve SEQ-3 Run and Push HOLD, but not sequence position, oscillator
phase, delay history, held controls, external assets,
custom GLSL, keyboard/device preferences, or the undo stack. Device preferences
remain separate host settings. [Native module presets](MODULE_PRESETS.md) are
available separately. Asset packaging, portable paths,
complete geometry history for states the GUI has never observed, and broader
platform/device qualification remain open.

Tests cover malformed and oversized files, legacy imports, changed-file
conflicts, failures before/after publication, active/orphan recovery ownership,
content-based dirty state, concurrent MCP edits during Save, stale Close/load
decisions, keyboard Save/Cancel continuation, and blocked rack gestures beneath
dialogs. Native CI renders the Save and unsaved-change dialogs on Linux.
