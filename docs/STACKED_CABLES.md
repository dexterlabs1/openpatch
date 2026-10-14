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
  route. **Create new cable** uses ordinary duplicate rejection; late Shift is
  required to permit an identical route.
- Right-click either jack for **Create new cable**, top-cable clone/delete,
  and **Move all cables** or **Delete all cables**. Each listed cable also has
  its own pickup, clone, delete, and color controls.
- **Move all cables** moves valid members and deletes selected members whose
  new routes duplicate existing destination cables or earlier accepted moves.
  Existing destination siblings remain intact. The whole result has one undo
  step; cancellation or failed graph validation changes nothing.
- Set a cable color from the palette or enter custom RGB bytes and choose
  **Apply colour**. Palette choices and Apply are disabled when they match the
  cable's current effective RGB, including its default palette color.
- Dropping a moved cable on empty space, a wrong-type jack, or a duplicate route
  deletes that cable. Fresh and cloned drafts disappear without altering originals.
- **Esc**, focus loss, modal interruption, stale revisions, and unsupported
  graph edits cancel without changing originals. See [cable gestures](RACK_EDITING.md#cable-gestures).

Input and output plugs have independent saved ordering. A different cable can
be on top at each end of the same stack. Picking up a selected non-top plug and
returning it to its jack raises only that end, with one undo step. Escape or
focus loss cancels provisional ordering as well as routing. Already-top returns
remain no-ops. Save/reload and undo/redo preserve both orders.

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
projects, recovery snapshots, undo/redo, and module duplicates. Set an existing
cable's color with `set_cable_color`, for example:

```json
{"cable_id":"transpose-pitch","color":[30,180,240],"expected_revision":12}
```

Color is presentation metadata and does not identify a DSP route or reset its
delayed frame when only the color changes. A direct call with the same stored
color validates the request and revision, then leaves revision, history, and
redo unchanged. Preview reports metadata edits in `changes.changed_cables`.

Use `disconnect_cables` with `cable_ids` to remove a chosen group in one undo
step. The array must contain 1–1,024 unique, existing IDs. All IDs validate before
any removal; an unknown, repeated, or stale selection changes nothing.

Move one or more exact cable ends with `move_cables`:

```json
{
  "cable_ids": ["transpose-pitch", "vibrato-pitch"],
  "end": "input",
  "node": "second-vco",
  "port": "pitch",
  "duplicate_mode": "reject",
  "expected_revision": 12
}
```

`end` selects the `input` or `output` endpoint. The destination module and port
must exist and match that direction, even when a conflict would otherwise
discard every selected cable. IDs and colors are retained. Moved and untouched
cables retain their saved array positions. Moving an end promotes it at its
destination; the opposite end retains its order. The `cable_ids` array accepts 1–1,024 unique existing IDs and controls
collision evaluation order. The native rack supplies saved cable order.

The default `duplicate_mode`, `reject`, fails the entire operation on an
identical route. `discard` removes only selected members that conflict with
unselected routes or earlier accepted moves; other selected members move and
existing destination cables stay intact. `stack` allows identical routes in
version 2. Selected cables whose requested endpoint is already unchanged stay
intact, including intentional identical stacks. A direct all-unchanged move
validates the request and revision, then preserves revision, history, and redo
unless `promote: true` requests a change to the selected plug order.
Version 1 still rejects occupied inputs and cycles during final validation.

These cable tools support `preview_edits` and `apply_edits`; batch no-ops retain
ordinary transaction history semantics. Failure publishes no partial color
changes, moves, removals, history entries, or runtime resets. Changed or deleted
selected cable frames reset; source modules and untouched sibling frames
continue. Put `expected_revision` on the outer transaction when batching.
Preview's `changes.cable_order_changed` flags a pure reordering of the same
cable multiset; additions and removals use their existing change lists instead.
`duplicate_modules` preserves cable multiplicity and assigns fresh IDs to every
copied cable. Its existing `none`, `internal`, and `inputs` policies are unchanged.
The `all` policy also copies external outgoing connections to their existing
destinations. Single-module **Duplicate with cables** uses it; selected-group
shortcuts retain the inputs-only policy. See [rack editing](RACK_EDITING.md).

## Independent plug order

Optional `input_order` and `output_order` fields are unsigned integers in
0–65,535. At each jack, lower ranks paint first; missing ranks behave as zero,
and ties keep saved cable-array order. Old patches remain valid. The resolver
never reorders the cable array or changes audio accumulation order.

Use `promote_cables` to raise selected plugs at one end:

```json
{"cable_ids":["transpose-pitch"],"end":"input","expected_revision":12}
```

Selection retains its existing bottom-to-top order at each jack. Promotion
normalizes ranks only when effective stacking changes, with bounded dense ranks
and untouched opposite-end metadata. A direct already-top call validates its
arguments and revision, then preserves history and redo. A real promotion
changes saved content and revision, with no DSP route retirement or signal reset.

`move_cables` accepts `plug_order`, an exact permutation of its selected IDs in
bottom-to-top order, independently of `cable_ids` collision evaluation order.
The native rack supplies both. Set `promote: true` to apply promotion even when
the requested route is unchanged. New connections and clones append at both
ends; module duplicates receive fresh ranks rather than conflicting copied ranks.
Recoloring leaves ordering intact. Preview reports rank metadata separately in
`changes.changed_cable_order`; `changes.cable_order_changed` continues to mean
actual array reordering, which can affect floating-point summation.

This strengthens Rack history intentionally: OpenPatch cancels provisional
promotion on Escape/focus loss and restores exact ranks on undo/redo. The pinned
[Rack save code](https://github.com/VCVRack/Rack/blob/8ea470f343058ade6f75f16d503a1eb330e1700c/src/app/RackWidget.cpp)
stores independent endpoint ranks, but its
[cable history actions](https://github.com/VCVRack/Rack/blob/8ea470f343058ade6f75f16d503a1eb330e1700c/src/history.cpp)
recreate plugs without those ranks.

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
