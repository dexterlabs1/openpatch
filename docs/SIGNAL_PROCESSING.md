# Logic, comparison, and voltage shaping

The version-2 kinds `logic`, `compare`, `process`, and `rescale` extend gate and
control-voltage patching. They are digital signal processors, not component-level
hardware models. Existing `sample_hold` and `attenuverter` retain their original
contracts. Reference behavior comes from Fundamental revision
[`10dd016`](https://github.com/VCVRack/Fundamental/tree/10dd0160c664770910e5584b7b00498cc48d9ddd/src).

## Connected example

[`gate-shaping-voice.json`](../examples/gate-shaping-voice.json) combines a slow
LFO comparator with a faster clock through Logic. Their AND output samples a
second LFO waveform in Process. Rescale maps the held voltage into one octave,
and Quantizer selects a pentatonic pitch before the VCO. The example exercises
all four processors in a connected voice; manual B and Gate controls let you
change its capture pattern while performing.

## Logic

Inputs are `a` and `b`; outputs are `not_a`, `not_b`, `or`, `nor`, `and`, `nand`,
`xor`, and `xnor`. Every result is 10 V for true or 0 V for false. An input is
true at or above exactly 1 V, without hysteresis. Manual B overrides B high.

All outputs use the maximum input count, at least one. Mono inputs broadcast;
missing lanes of shorter polyphonic signals are zero. An unpatched input is
zero. Output indicators distinguish mono and polyphonic activity; the manual
indicator reflects the button alone. No bypass routes are defined.
[Logic reference](https://github.com/VCVRack/Fundamental/blob/10dd0160c664770910e5584b7b00498cc48d9ddd/src/Logic.cpp).

## Compare

Inputs `a` and `b` use the maximum channel count, at least one, but **mono does
not broadcast**. Missing lanes are zero; `b_offset` (−10…10 V, default 0) is
then added to every B lane. Let A and B denote those adjusted inputs.

| Output | Value |
|---|---|
| `max`, `min` | Maximum and minimum of A and B |
| `clip` | A limited to −abs(B)…abs(B) |
| `lim` | A minus the clipped result |
| `clip_gate`, `lim_gate` | 10 V when A is strictly outside those limits, and its complement |
| `greater`, `less` | 10 V for A>B or A<B, respectively |

Equality does not count as clipping; both comparison gates are low when A=B.
The four gate indicators report activity across the actual lanes. No bypass
routes are defined. [Compare reference](https://github.com/VCVRack/Fundamental/blob/10dd0160c664770910e5584b7b00498cc48d9ddd/src/Compare.cpp).

## Process

Inputs are `slew_cv`, `signal`, and `gate`. Signal sets the active lane count;
auxiliary mono inputs broadcast. Zero signal channels produce mono-zero outputs
without advancing any lane. Inactive histories freeze and resume on regrowth.

Gate rises at 2 V or manual press; it falls at 0.1 V or below after manual
release. On a rise, `sh2` receives the previous capture and `sh1` captures signal.
Both captures pass through independent slew filters. A shared hold value captures
on both edges: `track_hold` holds during high and tracks during low;
`hold_track` does the opposite. `slew` jumps while high and slews while low.
`glide` jumps until a high gate has lasted more than 1 ms, then slews.

The `slew` parameter stores log₂(seconds per volt), from log₂(0.001) to log₂(10).
Its minimum means instantaneous movement and ignores slew CV. Otherwise CV adds
to this logarithmic time; the panel displays ms/V. Histories start at zero and
are not stored in patch files. There are no bypass routes.
[Process reference](https://github.com/VCVRack/Fundamental/blob/10dd0160c664770910e5584b7b00498cc48d9ddd/src/Process.cpp).

## Rescale

`signal` feeds `out` with the same channel count, at least one; missing input is
zero. First compute signal × gain × multiplier + offset. Gain is −1…1 (default
**0**); offset is −10…10 V (default 0). The multiplier offers 1, 10, 100, or 1000.
Saved `min`/`max` limits default to −10/+10 V.

Without reflection the output clamps between the limits. `reflect_min` reflects
below minimum then clamps at maximum; `reflect_max` reflects above maximum then
clamps at minimum. Enabling both repeatedly folds into the interval, including
negative inputs. If max≤min the result is min; limits are never silently swapped.
Indicators compare the pre-fold voltage against inclusive bounds. Context options
are validated saved parameters. Bypass directly routes signal to output.
[Rescale reference](https://github.com/VCVRack/Fundamental/blob/10dd0160c664770910e5584b7b00498cc48d9ddd/src/Rescale.cpp).

## Runtime controls and live observations

Logic and Process buttons are momentary performance controls. They do not add
undo steps or save a held state in the document. Live and offline actions use
`b_press` / `b_release` for Logic and `gate_press` / `gate_release` for Process.
Actions of another module's type are rejected. Within a single sample, ordered
button changes resolve to the final held state before CV processing.

The native host releases held controls on mouse-up outside a button, focus
loss, or an inactive rack. Releases remain possible while bypassed. A full live
queue uses a separate emergency-release path that invalidates older queued
presses without resetting Process histories. Entering bypass also clears manual
holds, including a bypass/resume that the callback sees as one graph update.

`inspect_performance` returns coherent per-module controls and
indicator snapshots. These are read-only observations of the running engine,
not arbitrary waveform captures. Existing `capture_signal` remains an offline
render from the saved patch plus an optional event trace.

## Numerical and compatibility boundaries

Every cable still adds exactly one sample of delay. Parameters compile on the
control thread; sample processing, event handling, and compatible state adoption
must allocate and free no heap memory. Nonfinite inputs become zero and final
arithmetic uses the existing ±1,000,000 V safety bound.

Process uses native exponential evaluation rather than Rack's fifth-order
approximation, with the exponent clamped to −64…64 for extreme CV. This is a numerical
difference, not sample-identical Rack emulation. Regression fixtures cover parameter ranges, alternate voices, edge ordering,
folding boundaries, manual-release failures, and persistence. Direct Rack
comparisons, legacy `.vcv` state migration, and native-device qualification
remain separate acceptance work.
