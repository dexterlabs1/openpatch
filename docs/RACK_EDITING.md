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
| Duplicate with inputs | Shift-Cmd-D | Internal connections and incoming connections from outside the selection |
| Remove selected modules | Delete / Backspace | Removes the selection and its incident cables |

Copies retain saved parameters and bypass flags. Their IDs are new, and their
running DSP states start fresh. Existing modules keep their live state during
the edit. Cloning does not change the patch's selected audio output or connect
the new group's outputs to existing downstream modules.

The new group is placed on free rack grid space while preserving its relative
geometry, then selected and brought into view. Placement and selection change
only after the revision-checked graph edit succeeds. A stale GUI snapshot or
invalid candidate leaves the current layout intact. Duplication and group
deletion are each one graph undo/redo action. Layout snapshots restore the
observed duplicated positions; arbitrary layout-only movement still lacks
project-wide undo. Native projects save the resulting graph and placement.

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
ends selected, and `inputs` also copies external incoming connections. Named
ports are preserved. A selected self-loop becomes a self-loop on its copy.
External outgoing cables remain attached to the originals.

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

The two duplication shortcuts follow the pinned Rack **selected-group**
behavior. Rack's hovered-single-module duplication with cables also copies
outgoing cables, relying on stacked inputs. OpenPatch supports
[stacked inputs](STACKED_CABLES.md), but continues to use selected-group
duplication semantics for single and multiple selections. That hovered-module
shortcut behavior remains explicit parity work, as do Rack preset conversion,
factory templates and complete project-layout history.

References: [Rack group serialization and duplication](https://github.com/VCVRack/Rack/blob/061ccf63c1758599396ac1bb10d47345d9d34076/src/app/RackWidget.cpp),
[hovered-module cloning](https://github.com/VCVRack/Rack/blob/061ccf63c1758599396ac1bb10d47345d9d34076/src/app/ModuleWidget.cpp),
and [input cable summing](https://github.com/VCVRack/Rack/blob/061ccf63c1758599396ac1bb10d47345d9d34076/src/engine/Engine.cpp).
