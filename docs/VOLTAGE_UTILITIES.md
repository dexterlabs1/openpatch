# Voltage utilities and module bypass

The version-2 utility bank adds `attenuverter8`, `mutes10`, and `cv_mix`.
These are original Rust implementations of the specified signal contracts;
matching a signal role does not establish complete VCV patch compatibility.
The reference is Fundamental revision
[`10dd016`](https://github.com/VCVRack/Fundamental/tree/10dd0160c664770910e5584b7b00498cc48d9ddd/src).

## Patch a voltage, then hear it

Choose **File → Voltage utilities**. The first two 8vert rows generate pitch
voltages; CV Mix adds them before the VCO. Mutes row 1 passes its audio to the
output. Toggle that row to silence the voice, or change a voltage to transpose
it. Inspect individual outputs with the signal scope. The example is also in
[`examples/voltage-utilities.json`](../examples/voltage-utilities.json).

## Signal contracts

| Kind | Ports and parameters | Normaling and polyphony |
|---|---|---|
| `attenuverter8` | `in1`…`in8`, `out1`…`out8`; `gain1`…`gain8`, −1…1, default 0 | Top unpatched input is mono 10 V. Each later unpatched row inherits the nearest connected input above, with its channel count, before gain. Each output applies only its own gain. |
| `mutes10` | `in1`…`in10`, `out1`…`out10`; `mute1`…`mute10`, exactly 0 or 1, default 0 | 0 passes; 1 silences. Top normal is mono 0 V. Unpatched rows inherit voltage from above independently of muting, but become mono. An explicitly connected row keeps its input channel count. |
| `cv_mix` | `in1`…`in3`, `out`; `level1`…`level3`, −1…1, default 0 | Each unpatched input independently supplies 10 V. Output voices follow the largest input count, at least one. Mono broadcasts; missing channels of shorter polyphonic inputs are zero. Sum the three weighted inputs without averaging. |

The inherited mono behavior of Mutes follows the pinned source, whose channel
count starts at one on each row. It differs from 8vert's inherited polyphonic
count. The Mutes **Invert mutes** operation flips all ten states atomically,
including omitted flags, which default to passing.

These modules have no smoothing or running DSP state. Nonfinite input samples
become zero; final arithmetic is bounded to ±1,000,000 V. CV Mix applies the
bound after summation so large opposing terms can cancel. Cable propagation
retains version 2's one-sample delay. Controls and bypass are saved in projects;
legacy Rack `states[]` migration and `.vcv` import remain future work.

References: [8vert](https://github.com/VCVRack/Fundamental/blob/10dd0160c664770910e5584b7b00498cc48d9ddd/src/8vert.cpp),
[Mutes](https://github.com/VCVRack/Fundamental/blob/10dd0160c664770910e5584b7b00498cc48d9ddd/src/Mutes.cpp),
[CV Mix](https://github.com/VCVRack/Fundamental/blob/10dd0160c664770910e5584b7b00498cc48d9ddd/src/CVMix.cpp).

## Bypass is a saved module setting

Right-click a module to bypass or resume it. Parameter editing and patching
remain available. Bypass is available in both patch versions: loading an old
patch does not change its timing. `bypassed` defaults to false and is omitted
from JSON when false. A bypass edit uses the same revision, undo/redo, and
validation path as other graph edits.

Bypass follows declared routes, not a general assumption that every processor
passes its input:

| Module | Bypassed output |
|---|---|
| Mutes | Each corresponding input directly to its output; no downward normaling and no mute flags |
| Octave, Quantizer | Pitch directly to output |
| Gain, Low-pass, Ladder, OTA VCA, single Attenuverter | Signal directly to output |
| Crossfade | Input `a` directly to output |
| All remaining kinds, including 8vert, CV Mix, sequencers, switches and audio sinks | Mono zero on every output |

A routed input with no channels emits mono zero. Gain/filter/attenuverter and
single-output crossfade mappings are explicit OpenPatch choices, not claims
that they reproduce a different stock module's full bypass interface. Discover
routes through the catalog.

Ordinary DSP state freezes during bypass and resumes when enabled. On either
transition, outputs and their outgoing delayed cable samples clear; incoming
cable history stays current. Oscillator phase, filter memory and sequence
position are preserved. Sequencer actions are rejected while bypassed. Entering
bypass invalidates earlier queued actions even if bypass and resume occur
before the audio callback sees the intermediate graph.

MIDI → CV is an intentional exception: it continues consuming MIDI and aging
pulses while emitting silence. Held notes resume, notes released during bypass
stay released, and expired triggers do not reappear. This prevents stale note
state; it is an OpenPatch policy rather than an exact Rack MIDI-CV behavior.

Rack's route and output-clearing behavior is documented in the pinned
[Module](https://github.com/VCVRack/Rack/blob/061ccf63c1758599396ac1bb10d47345d9d34076/src/engine/Module.cpp),
[Engine](https://github.com/VCVRack/Rack/blob/061ccf63c1758599396ac1bb10d47345d9d34076/src/engine/Engine.cpp),
and [Port](https://github.com/VCVRack/Rack/blob/061ccf63c1758599396ac1bb10d47345d9d34076/include/engine/Port.hpp) sources.

## MCP operations

`set_bypass` requires a JSON boolean:

```json
{"id":"mutes","bypassed":true,"expected_revision":7}
```

`invert_mutes` requires a `mutes10` ID:

```json
{"id":"mutes","expected_revision":8}
```

Both operations also work inside `preview_edits` and `apply_edits`. A successful
batch creates one undo entry; invalid or stale requests leave the graph intact.
Offline captures and renders use the saved bypass state. Offline sequencer
events targeting a bypassed module fail validation before a WAV is created.

Validation includes utility normaling and channel-count fixtures, route and
freeze/resume tests, callback allocation guards, coalesced bypass/action tests,
headless row/port/menu interactions, project recovery round trips, and MCP signal
captures that check audible, muted, and direct-bypass results.

Remaining parity work includes direct differential traces, Rack file/state
migration, and native device qualification. These utility equations are not
physical circuit models.
