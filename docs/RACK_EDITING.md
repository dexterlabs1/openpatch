# Selecting and duplicating modules

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
shortcut behavior remains explicit parity work, as do module presets and
complete project-layout history.

References: [Rack group serialization and duplication](https://github.com/VCVRack/Rack/blob/061ccf63c1758599396ac1bb10d47345d9d34076/src/app/RackWidget.cpp),
[hovered-module cloning](https://github.com/VCVRack/Rack/blob/061ccf63c1758599396ac1bb10d47345d9d34076/src/app/ModuleWidget.cpp),
and [input cable summing](https://github.com/VCVRack/Rack/blob/061ccf63c1758599396ac1bb10d47345d9d34076/src/engine/Engine.cpp).
