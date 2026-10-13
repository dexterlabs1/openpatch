# Building and editing a rack

Use **Shift-click on module headers** to build a selection. A plain click on
an unselected module selects that module; dragging a selected header moves the
group while retaining its relative placement. **Cmd-A** selects the rack's
modules. Click empty rack space to clear selection. Keyboard shortcuts apply
outside text editors and dialogs; use Ctrl instead of Cmd on Windows/Linux.

The module context menu and keyboard offer:

| Action | Shortcut | Copied cables |
|---|---|---|
| Duplicate | Cmd-D | None |
| Duplicate selection with inputs | Shift-Cmd-D with a selection | Internal connections and incoming connections from outside the selection |
| Duplicate hovered module with cables | Shift-Cmd-D with no selection | All incoming and outgoing connections; self-patches become self-patches on the copy |
| Remove selected modules | Delete / Backspace | Removes the selection and its incident cables |

Keyboard duplication gives any nonempty selection priority, including a single
selected module. With no selection, hover a module to duplicate it. The pointer
must be inside the rack; text editing, dialogs and active drags suppress these
shortcuts. Right-clicking an unselected module instead targets that module,
without replacing a selection elsewhere. Its menu offers **Duplicate with
cables**; a selected module's menu offers **Duplicate with inputs**.

Copies retain saved parameters, bypass flags and captured SEQ-3 Run state. Their
IDs are new; other running DSP state starts fresh. Existing modules retain their
live state. Group duplication leaves external destinations alone. Single-module
duplication with cables adds the copy's outputs to the existing destinations,
stacking them with the original connections. The selected audio output remains
unchanged. Legacy version-1 patches cannot stack inputs: the native single-module
with-cables action is unavailable when it would copy an external outgoing cable.
Incoming-only copies still work in version 1.

Copies are placed on free rack grid space to the right, preserving group
geometry. Group duplication selects the copies; single-module duplication
preserves the previous selection. The new copies are brought into view.
Placement and selection change
only after the revision-checked graph edit succeeds. A stale GUI snapshot or
invalid candidate leaves the current layout intact. Duplication and group
deletion are each one graph undo/redo action. Layout snapshots restore the
observed duplicated positions. Native projects save the resulting graph and placement.

Each completed single/group drag creates one revision-checked checkpoint in
that same history. Undo/redo follows the order of moves, parameter edits and
cable edits, including commands from MCP. Checkpoints preserve compatible DSP
state, selection and viewport. Drag previews are excluded from Save/recovery;
only accepted snapped positions become committed geometry. Escape, focus loss,
a modal dialog, a blocked/no-op drop, or a newer graph revision cancels the
preview without adding history. Pan/zoom remain persisted view settings without
separate undo entries.

## Exact parameter values

Right-click a knob to type a value directly in its parameter menu. Double-clicking
the displayed value also opens an editor. **Enter** or **Apply** commits a valid
value as one shared undo action; **Escape** or **Cancel** discards the draft.
Clicking outside the parameter menu cancels it. Losing app focus or opening
a modal editor also discards the draft. The editor shows the control's units
and accepted range. Gain controls accept
decibels, including `-inf` for silence; stepped controls require a permitted step.
Type a number, expression, or note name: `220*2`, `A4`, `C#4`, or `C4v`.
The current value is selected when the editor opens, and **Result** previews the
evaluated value in the control's entry units. Typing does not change the patch
until Apply or Enter. See [expression syntax and compatibility](PARAMETER_EXPRESSIONS.md).

An open draft belongs to the patch revision at which it was opened. If another
edit, including an MCP command, changes that revision, reopen the control before
applying a value. The command also checks that original revision when committing,
so a concurrent change cannot be overwritten between drawing and applying the
editor. Rejected drafts add no undo entry.

![Direct frequency entry in a knob context menu](images/openpatch-parameter-entry.png)

*Native Linux CI capture with a focused-editor fixture; playback is paused.*

## Shared MCP operation

`duplicate_modules` uses an explicit, nonempty source-to-destination mapping.
Source IDs and destination IDs must each be unique. Every source must exist,
and every destination must be a valid unused module ID.

```json
{
  "modules": [
    {"id": "vco", "new_id": "vco_copy"},
    {"id": "filter", "new_id": "filter_copy"}
  ],
  "cable_mode": "inputs",
  "expected_revision": 12
}
```

Use the revision returned by `inspect_patch`, rather than the example value.
`cable_mode` defaults to `none`; `internal` copies only connections with both
ends selected, and `inputs` also copies external incoming connections. `all`
copies every connection touching a mapped module, including external outgoing
connections. It remaps each selected endpoint and retains each external endpoint.
Each original cable produces at most one copy, so self-loops and internal edges
are not doubled. The multi-module `all` mode is an explicit MCP extension beyond
Rack's group shortcut. Named ports, repeated routes and source cable order are
preserved; copied cables receive fresh IDs and start with empty delay buffers.
Original cables retain their IDs and order.

The operation works in `preview_edits` and `apply_edits`. Later operations in
the same transaction can edit the copies. Validation covers the final graph,
including engine version, module/cable limits, and cycle restrictions. Failure
does not change the patch, revision, undo/redo history, or live identities.
The operation returns the usual session snapshot; the supplied mappings
already identify every new module. Headless MCP changes the graph only; native
selection and placement are presentation state.

## Initialize and Randomize

Right-click a module or selected group and choose **Initialize** or
**Randomize**. The corresponding shortcuts are **Cmd-I** and **Cmd-R**
(Ctrl on Windows/Linux). Hover an unselected module to target it, or a selected
module to target the group; over empty rack space the shortcuts use the current
selection. Each action is one undoable graph edit. The action
retains module IDs, cables and their order, bypass flags, audio output,
selection and panel positions.

Initialize restores eligible saved controls to their descriptor defaults.
It also runs the module's specific initialization hook after live state has
been adopted:

| Module | Runtime effect beyond saved controls |
|---|---|
| Four-output VCO | Keeps phase, sync direction, correction tails and square filter memory |
| Four-output LFO | Resets phase and clock acquisition/timing; preserves clock and reset edge detectors |
| SEQ-3 | Returns to step one; keeps live Run state, phase, pulses and trigger detectors |
| Sequential switches | Keeps selected position, detectors and declick history |
| Process | Keeps sample/hold and slew history |
| Other modules | Retains the existing state-adoption behavior for parameter edits |

Initialization releases held manual buttons and cancels older queued module
actions. It works while bypassed and even if saved controls already equal
their defaults. It does not recreate the module or retire its cables.
MIX6 retains its menu-based invert and average modes.
SEQ-3's saved `running` setting retains both its value and its presence, so
initializing a paused live sequence does not inadvertently start it.

Undo and redo restore saved controls and captured SEQ-3 Run state; they do not
replay the runtime hook or rewind unsaved DSP phase or step position. If Initialize is immediately undone before the audio
callback receives a graph, its pending runtime hook still runs once. A
per-incarnation generation prevents later graph edits and history traversal
from repeating it. No control lock or allocation is needed in the callback.

Randomize samples only eligible saved parameters. Discover each control's
`initialize` eligibility and `randomization` distribution in `catalog`.
Distributions are disabled, linear, log2, inclusive integer, or enum; numeric
bounds respect the patch sample rate. VCO/LFO FM and PWM attenuverters retain
their settings. SEQ-3 randomizes voltage rows and gate flags while preserving
tempo, length, their CV amounts, Run and passthrough. Saved menu settings also
retain their values: [MIX6 modes](https://github.com/VCVRack/Fundamental/blob/e0ade9a23a9b41124ea3f4ec9aa2cb0b98c2529e/src/Mixer.cpp), VCA MIX response modes, switch declick, FADE
law, Merge channel count, and [Rescale multiplier/reflection modes](https://github.com/VCVRack/Fundamental/blob/e0ade9a23a9b41124ea3f4ec9aa2cb0b98c2529e/src/Rescale.cpp). Randomize does not select
a random live sequencer/switch step. Restoring its history entry restores the
sampled controls without drawing new values.

Both tools support atomic preview and commit:

```json
{"tool":"initialize_modules","arguments":{"ids":["vco","seq"]}}
{"tool":"randomize_modules","arguments":{"ids":["vco","seq"],"seed":12345}}
```

Use those objects inside `preview_edits` or `apply_edits`; include the observed
`expected_revision` on the outer request. Direct tool calls accept that field
too. IDs must be unique, existing, and number 1–256. A randomization seed is a
required unsigned 32-bit integer. Given the same starting patch, module IDs,
catalog and seed, preview and apply produce the same values. Each module gets
`FNV-1a32(UTF-8 ID) XOR seed`; its eligible parameter names are sorted and
sampled using SplitMix64. Thus changing the order of selected IDs does not
change the result. The UI supplies a fresh seed for each invocation.

The parameter policies for OpenPatch-specific modules are explicit product
choices, not claims of identical random distributions to Rack. The [native module preset library](MODULE_PRESETS.md) captures complete settings
and current SEQ-3 Run state. Rack `.vcvm` import and factory templates remain
unfinished.

The reference for initialization/history behavior is Rack 2.6.4
([module actions](https://github.com/VCVRack/Rack/blob/8ea470f343058ade6f75f16d503a1eb330e1700c/src/app/ModuleWidget.cpp),
[base module](https://github.com/VCVRack/Rack/blob/8ea470f343058ade6f75f16d503a1eb330e1700c/src/engine/Module.cpp),
[history](https://github.com/VCVRack/Rack/blob/8ea470f343058ade6f75f16d503a1eb330e1700c/src/history.cpp))
and Fundamental 2.6.4's
[LFO](https://github.com/VCVRack/Fundamental/blob/e0ade9a23a9b41124ea3f4ec9aa2cb0b98c2529e/src/LFO.cpp),
[SEQ-3](https://github.com/VCVRack/Fundamental/blob/e0ade9a23a9b41124ea3f4ec9aa2cb0b98c2529e/src/SEQ3.cpp),
and [VCO](https://github.com/VCVRack/Fundamental/blob/e0ade9a23a9b41124ea3f4ec9aa2cb0b98c2529e/src/VCO.cpp).

## Live identity across coalesced edits

The host tracks an incarnation for every node, independently of its saved ID.
Removing and recreating an ID, changing its kind, or replacing the document
starts a new incarnation even if the audio callback skips intermediate graph
revisions. Compatible parameter/cable edits retain the incarnation.

At graph adoption, recreated nodes retain fresh compiled DSP state and empty
incident cable history. Unchanged nodes and edges continue normally. The
previous graph remains available for the existing short host crossfade.
Identity comparison and state adoption use bounded preallocated storage; no
control mutex, allocation, deallocation, or JSON operation runs in the callback.

## Rack parity boundary

The shortcut cable policies and selection priority follow pinned Rack 2.6.4.
OpenPatch places groups to their right rather than centering them around the
pointer, and resolves collisions using its existing rack-grid placement policy.
It preserves global source cable order rather than appending internal cables
before external inputs. Cable colors are derived from each fresh ID and can
differ from the source cable. Full plug-order/color behavior, Rack preset conversion,
factory templates and complete project-layout history remain open.

References: [Rack group serialization and duplication](https://github.com/VCVRack/Rack/blob/8ea470f343058ade6f75f16d503a1eb330e1700c/src/app/RackWidget.cpp),
[hovered-module cloning](https://github.com/VCVRack/Rack/blob/8ea470f343058ade6f75f16d503a1eb330e1700c/src/app/ModuleWidget.cpp),
and [keyboard dispatch](https://github.com/VCVRack/Rack/blob/8ea470f343058ade6f75f16d503a1eb330e1700c/src/app/Scene.cpp).

## Docked module library

Click the library toolbar icon (or press Enter with the rack focused) to browse modules in the
resizable right sidebar. Search names, descriptions, tags, controls and ports;
filter by musical role or star favorites. Favorites are saved with host preferences.
The library stays open while you patch cables or edit the rack.

Click a module or use Up/Down and Enter in search to add it near the visible rack
center and reveal it. Drag a tile onto the rack to preview its snapped placement,
then release to add it without changing the view. Occupied module positions are
avoided. Dropping outside the rack, pressing Escape during a drag or losing window
focus cancels the add. Escape with library keyboard focus closes the sidebar.

An add is one undoable host transaction, including the new module's placement.
A conflicting concurrent MCP edit rejects the stale add without moving modules or
changing selection. Modules requiring the polyphonic engine offer the existing
upgrade confirmation before they can be added to a version-1 patch.
