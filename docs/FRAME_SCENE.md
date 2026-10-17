# Frame Scene

`frame_scene` is a four-channel scene controller/mixer. It adapts the pinned
Frames integer keyframer and resources, with a separate, declared ideal native
VCA path. It is an independent OpenPatch instrument; no automatic VCV mapping,
binary plugin compatibility or calibrated analog-hardware identity is claimed.

Store up to **64 keyframes**, each with a timestamp and four levels. FRAME and
its attenuverted CV scan the scene. Each track selects Step, Linear,
Accelerating, Decelerating, Smooth or Bounce easing and a linear/exponential
response. Source Step switches at the interval midpoint. With no frames, the four LEVEL controls set immediate gains. With
frames, select a timeline column and edit its stored levels using the same large
knobs or lane points. Drag a top tick to move its timestamp. Selection is local
UI state and never moves playback. The source capacity is 64; the initial
twenty-frame backlog limit was an error.

The browser and rack share the same 36 HP Signal Atlas renderer, selected font,
large rotaries, sockets and meaningful authoring timeline. The solid LIVE
playhead, gain lamps, scene RGB and step lamp use actual published processing
state. Without telemetry the playhead is dashed and explicitly CONFIGURED.
Timeline points show authored data, not an invented audio waveform. Native
parameter construction cues and value bubbles use the existing shared paths.

## Native signal contract

Inputs are ALL, channels 1–4 and FRAME CV; outputs are channels 1–4, MIX and STEP.
Inputs read channel zero and outputs are mono. A physical cable breaks a
normalization even when its source currently has zero channels. Unpatched ALL
uses the selected 0/5/10 V offset; unpatched channel inputs use ALL.

MIX sums only channel outputs without a physical cable. The engine records
physical output presence separately from consumer demand: a tap, port-light
observer or selected Engine output cannot alter this routing. Other existing
models retain their previous demand behavior.

Native position is `round(clamp(frame + frame_cv/10 * modulation,0,1)*65535)`.
Integer evaluation is cached until that timestamp changes. The firmware's
hardware ADC/control cadence is not reproduced. Scanning across any stored
timestamp emits a 5 V pulse for `ceil(sample_rate/1000)` frames in either
direction. A jump across multiple keys emits one pulse. First observation,
Initialize and replacement content do not invent a crossing.

Source levels feed the exact source 12-bit DAC conversion, including response.
The ideal VCA transfer then uses a fixed 4096-entry monotone inverse of the
source response-zero compensation curve: the first 16-bit gain whose source DAC
code is at most the requested code, with exact zero/full saturation endpoints.
This explicitly chosen transfer preserves meaningful response control without
treating DAC code as gain or claiming measured analog volts/calibration.
Individual inputs and MIX are bounded to ±1,000,000 V for finite native safety.

## Authored content and MCP

`ModuleContent::FrameScene` persists strictly sorted unique timestamps, four
`u16` values, unique stable IDs and a wrapping next-ID counter. Storage compiles
into a fixed 64-entry array; no Vec, authoring edit or JSON runs in the callback.
Moving/reordering preserves IDs. Collisions, capacity overflow, stale IDs,
invalid arrays and stale document revisions reject atomically. Update at
capacity remains valid. UI content gestures accept only their own next-revision
echo; external edits cancel a held gesture instead of inheriting its motion.

MCP `edit_frame_scene` takes a module ID and one typed add/update/remove/clear
operation. Update/remove address the stable `keyframe_id`, never a sorted index.
Values are four source 16-bit controls, not voltages. For example:

```json
{"id":"scene","edit":{"operation":"add","timestamp":0,"values":[65535,0,32768,16384]},"expected_revision":12}
```

Updates can share `history_group` for one undoable gesture. The operation also
works in atomic edits and paced construction. `set_module_content` supports
bulk data; presets, duplication and saved reopen preserve it. Ordinary
`set_parameter level1..4` controls immediate levels only when no keys exist;
stored levels use the authoring operation. Easing/response, FRAME and modulation
remain ordinary parameters. Initialize retains keys and resets runtime
observations; reopening starts fresh. Bypass emits mono zero and freezes state.

[Actual shared panel, browser thumbnail and dim-room review](../design/reviews/frame-scene-v1/README.md).

## Qualification and source limits

The [pinned source runner](../scripts/frames_reference/README.md) retains all
five upstream files and MIT notices at eurorack
`f4cfa7697095a28d292bff758beb51deb49b5a86`, stmlib
`e3bd7c9cc00e4364166f9905c0509b6ffd0535ec`.
On macOS ARM, the raw Rust kernel matches **163,013 integer records across 25 cases** exactly,
including levels, DAC codes, palette, positioning and source editing behavior.
The same positive inputs pass a separate C++ undefined-behavior/bounds sanitizer
build with identical records. Cases include all six curves, varied responses,
uneven/nonzero/equal/descending tracks and full 65,536-gain DAC sweeps at both
response endpoints.

Three source-undefined domains are excluded from equivalence: capacity 64
after-last array reads/removal, curved easing at scale 65536 reading beyond its
table, and signed multiplication overflow reached by large Bounce ramps.
Native guards extend the array endpoints safely and use explicit wrapping for
the overflow domain. Defined source quirks remain, including Step's fixed-point
endpoint rounding and unused-slot nearest metadata. This is not a claim that
the original C++ behavior is defined for all hardware/firmware inputs.

Focused native tests cover 64-frame restore, full-capacity updates, ID movement,
physical normaling versus taps/observation, responses, pulses, adoption,
Initialize/bypass/reopen and callback allocation/deallocation. UI tests cover
geometry, stored/manual editing, authoritative telemetry, MCP control anchors,
accepted gesture echoes and external-change cancellation. Shared-session and
stdio MCP checks cover grouped history, atomic errors, presets and saved reopen.

[Evolving voice example](../examples/frame-scene-evolving-voice.json) combines
Tidal Functions scanning four authored scenes with a low analog voice, ladder,
VCA and Studio Mixer. It requires the development build; installed 0.2.7 lacks
these two new instruments. The [Frames manual](https://pichenettes.github.io/mutable-instruments-documentation/modules/frames/manual/)
documents the hardware's keyframing and jack architecture; native timing and
transfer choices above are OpenPatch policies.
