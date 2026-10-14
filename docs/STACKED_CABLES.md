# Stacked cables

Version-2 inputs can sum several cables directly. Use this to add transposition
and vibrato to a pitch sequence, combine modulation, or mix signals at a
destination. Each cable retains its own identity and one-sample delay.

## In the rack

Choose **File → Stacked pitch voice** for a three-note chord with a shared
transpose control and gentle vibrato. All three control sources meet at the
VCO's pitch input. The mono transpose and vibrato signals apply to every voice.

- Hold **Cmd** (Ctrl on Windows/Linux) when pressing either jack to start
  another cable. An ordinary drag moves its top plug to the same jack type.
- Valid drops onto occupied version-2 inputs keep every existing cable.
- **Cmd/Ctrl+Shift-drag** clones the top cable, retaining its color.
- **Shift-click** deletes only the top cable immediately on press.
- Add **Shift at drop**, after starting without Shift, to permit an identical
  route. The input menu's **Stack new cable here** also permits this extension.
- Right-click an input to choose, move, or remove one cable by ID and source;
  **Remove all** clears the whole stack.
- Dropping a moved cable on empty space, a wrong-type jack, or a duplicate route
  deletes that cable. Fresh and cloned drafts disappear without altering originals.
- **Esc**, focus loss, modal interruption, stale revisions, and unsupported
  graph edits cancel without changing originals. See [cable gestures](RACK_EDITING.md#cable-gestures).

The jack badge shows the cable count. Separate cable curves and colors help
distinguish stacked routes; the menu also distinguishes two cables with exactly
the same endpoints. Shift-click on module headers still changes the module
selection. Version-1 patches retain their single-cable inputs; explicitly enable
version 2 before stacking.

## Through MCP

Read `inspect_patch` first. Every cable in a live session has an `id`; use it to
address one cable independently of other routes on the same input.

Call `connect` with `mode: "stack"` to add a cable. The default mode, `"reject"`,
rejects an occupied input. Supply an unused `cable_id` to choose its identity,
or omit it for an automatically assigned ID:

```json
{
  "from": "transpose",
  "output": "out",
  "to": "vco",
  "input": "pitch",
  "mode": "stack",
  "cable_id": "transpose-pitch",
  "color": [230, 70, 60],
  "expected_revision": 12
}
```

Use the actual observed revision instead of the example number. Call
`disconnect` with `{"cable_id":"transpose-pitch"}` to remove that one cable.
The older `{"to":"vco","input":"pitch"}` selector removes all cables at
that input. Do not combine the two selector forms.

Optional `color` is an RGB array of exactly three integers from 0 through 255.
Omitting it uses the rack's default palette. Invalid colors reject the whole
operation before IDs or history change. Colors persist in patch JSON, native
projects, recovery snapshots, undo/redo, and module duplicates. When moving a
cable through MCP, pass its observed color to the replacement `connect` call.
Color is presentation metadata and does not identify a DSP route or reset its
delayed frame when only the color changes.

To move a cable, put its disconnect and new connect into one `apply_edits`
transaction, reusing its ID after removal. Preview the same edits with
`preview_edits`; ID assignment is deterministic for the same starting revision.
Failure publishes no partial disconnects, history entries, or runtime resets.
Preview's `changes.cable_order_changed` flags a pure reordering of the same
cable multiset; additions and removals use their existing change lists instead.
`duplicate_modules` preserves cable multiplicity and assigns fresh IDs to every
copied cable. Its existing `none`, `internal`, and `inputs` policies are unchanged.
The `all` policy also copies external outgoing connections to their existing
destinations. Single-module **Duplicate with cables** uses it; selected-group
shortcuts retain the inputs-only policy. See [rack editing](RACK_EDITING.md).

## Signal and identity contract

The input width is the maximum source width. Mono sources broadcast; shorter
polyphonic sources contribute zero on missing lanes. Zero-channel sources
contribute nothing; an entirely zero-channel stack invokes normaling. Identical
routes each contribute, and signed voltages can cancel. A cable reads its
source's previous sample, including its channel count, even in a self-loop.

Source nonfinite values become zero, then `f32` additions run in saved cable
order. There is no averaging or clipping at the input sum. Existing downstream
module bounds remain in effect. This follows the pinned
[Rack input aggregation](https://github.com/VCVRack/Rack/blob/061ccf63c1758599396ac1bb10d47345d9d34076/src/engine/Engine.cpp#L332)
rules. OpenPatch defines saved-order accumulation explicitly; Rack does not
promise a portable ordering among same-input cables. Summation rounding can
therefore differ when routes are reordered.

Persistent IDs contain 1–64 ASCII letters, digits, underscores, hyphens, or
periods. Cable IDs are unique in their own namespace, separate from module IDs.
Duplicate or invalid explicit IDs are errors. Older idless files gain IDs in
memory when loaded, without changing timing, layout, their original disk bytes,
or the clean saved baseline. Saving persists the assigned IDs. Direct core
compilation of idless patches remains supported and does not mutate the input.

Runtime incarnations are separate from saved IDs. Reconnecting, deleting and
recreating, or undoing and redoing a cable resets that cable's delayed frame,
even if the audio callback skips intermediate edits. Untouched routes and source
DSP state continue. Full document replacement starts fresh. The callback uses
precompiled routing and bounded storage; ID allocation and migration happen on
the control thread.

This does not add `.vcv` import, plugin compatibility, or complete Rack plug-order
behavior. Those remain separate parity work.
