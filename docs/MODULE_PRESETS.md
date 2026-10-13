# Module presets

Right-click a module's panel or header and open **Presets** to save its settings
or browse the preset library. Presets belong to one module kind: a VCO preset
loads into a VCO, and a SEQ-3 preset loads into a SEQ-3. Cables, module identity,
selection and panel placement stay in place.

The save window captures the settings once and displays them for review. Choose
a new `.opreset` filename to save those exact settings. A later knob turn or Run
button press does not silently change the reviewed capture. Refresh the capture
to save the latest state instead. Existing files are never overwritten.

The browser lists compatible presets from the current workspace (`~/OpenPatch`
by default), with search, paging and errors for invalid files. Read a preset to
preview its settings, then apply it. Apply uses the reviewed contents, so editing
the file on disk afterward cannot change that operation. If the patch changes
while the window is open, refresh the target before applying. Closing or
canceling the window leaves the rack unchanged.

## What a preset contains

A complete capture includes all effective saved controls, saved menu settings,
and bypass. Omitted ordinary parameters are filled with their catalog defaults.
SEQ-3 also captures its **actual current Run state**, which may differ from the
saved initial setting. It does not capture step position, oscillator phase,
trigger detectors, meters, held manual buttons, queued actions, cables or layout.
Loading does not implicitly Initialize the module.

```json
{
  "format": "openpatch-module-preset",
  "version": 1,
  "kind": "seq3",
  "params": {"running": 0, "row1_1": 2.5},
  "bypassed": false
}
```

Hand-authored partial presets are supported: only the listed controls change.
Omitting `bypassed` preserves bypass; omitting `running` preserves both SEQ-3's
current Run state and the presence of its saved initial setting. An explicit
Run value always assigns that state, including when the saved parameter already
contains the same number. This does not create a Run button pulse or reset the
sequence's step, phase, trigger history or detectors.

## Live capture and undo

Live SEQ-3 capture reads one coherent telemetry snapshot from the latest completed
audio callback at the current patch revision. Its frame number belongs to the
same snapshot as its Run state. It does not predict queued future button presses.
If the current graph has not reached the callback, capture reports that live
state is unavailable and can be retried; it never silently substitutes the
initial Run setting. Headless/offline sessions with no audio backend use the
initial/default state and label the capture `offline_reset`.

Preset application is one undo step. SEQ-3 history records the actual before and
after Run states separately from the saved initial parameter. Undo and redo
restore those recorded states, even if the Run button was used in the meantime;
they do not rewind step position or phase. The same custom-state history applies
to Initialize, Randomize, explicit Run assignments, and sequencer copy/removal
workflows. Ordinary knob edits and pattern rotation retain their parameter-only
history and do not restore unrelated Run state. Initializing still resets
module-specific runtime state only once; undo does not replay that event.

Preset loading releases held momentary buttons without resetting Process's
sample/hold or slew memory. Explicit Run loads cancel older queued sequencer
actions; subsequent actions wait for the corresponding graph. State loads work
while bypassed and preserve the module's incarnation. The callback applies them
after ordinary state adoption using fixed storage, without file access, JSON,
allocation or control locks.

## MCP tools

| Tool | Arguments | Result |
|---|---|---|
| `capture_module_preset` | `id`, optional `expected_revision` | `preset`, `revision`, `id`, `state_source`, `frame` |
| `save_module_preset` | `filename`, reviewed `preset` | Saves those exact contents under a new name |
| `list_module_presets` | Optional `kind`, `query`, `offset`, `limit` | Sorted library page with validity/error details |
| `read_module_preset` | `filename` | Parsed, validated `preset` contents |
| `apply_module_preset` | `id`, reviewed `preset`, optional `expected_revision` | Updated session snapshot |

`apply_module_preset` also works inside `preview_edits` and `apply_edits`. Put the
observed revision on the outer transaction. Invalid kinds, fields, parameters,
values, versions, stale targets or an invalid final graph leave the patch and
history unchanged. Preview performs no runtime state load. The final graph checks
sample-rate-dependent bounds and patch-version compatibility.

## Files and remaining compatibility

Preset files live directly in the workspace. Names use ASCII letters, digits,
`-`, `_` and `.`, are at most 120 bytes long, and cannot start with a dot or include
a path. Reads are limited to 64 KiB and reject symlinks, directories and other
non-regular files. New files are written to exclusive temporary files and
published with an atomic no-clobber hard link; an unsupported filesystem returns
an error. This is not a claim of power-loss durability on every filesystem.
Library pages default to 50 entries and allow at most 100; the library scan
rejects more than 1,024 matching filenames rather than silently hiding them.

This is OpenPatch's native format. Rack `.vcvm` conversion, factory presets,
user templates/defaults for new modules, preset rename/delete/overwrite and
cross-workspace browsing remain unfinished. Native project Save/recovery also
capture current Run coherently; step position and other DSP histories restart
on load. Graph-only MCP interchange retains configured parameters. These
contracts do not claim that every project/history path captures all live state.

The custom Run behavior follows Fundamental 2.6.4
[SEQ-3 serialization](https://github.com/VCVRack/Fundamental/blob/e0ade9a23a9b41124ea3f4ec9aa2cb0b98c2529e/src/SEQ3.cpp)
and Rack 2.6.4's
[module history](https://github.com/VCVRack/Rack/blob/8ea470f343058ade6f75f16d503a1eb330e1700c/src/history.cpp).
See also the [Rack preset workflow](https://vcvrack.com/manual/Presets).
