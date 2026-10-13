# Three-row voltage sequencer

Choose **File → Sequenced analog voice** for a connected example. SEQUENCE 3
uses its first row for pitch, second row for filter movement, and third row for
accents. Eight column buttons enable or disable the main trigger at each step.
Drag a voltage knob, double-click its value to type volts, or double-click the
knob to restore zero. Right-click the module to rotate all three rows and their
trigger flags together in one undo step.

The green column and running indicator report the audio engine's state. The
Clock, Run, and Reset buttons enqueue performance actions; they do not rewrite
the patch. Controls are unavailable without a live audio backend. A queued
receipt is distinct from confirmation that the audio callback applied it.

## Signal contract

`seq3` requires patch version 2. All outputs are mono; inputs read channel one
only, with absent channels treated as zero. Cable transport adds the normal
one-sample delay per edge. This is ideal switched control voltage, without
slew, circuit noise, or de-clicking.

| Port | Meaning |
|---|---|
| `tempo` input | Exponential clock CV, scaled by `tempo_amount` (0–1) |
| `clock` input | External clock; a nonempty connected signal freezes the internal phase |
| `run`, `reset` inputs | Rising-edge run toggle and reset |
| `steps` input | Additive length CV, scaled by `steps_amount` (0–1) |
| `cv1`, `cv2`, `cv3` outputs | Selected column's three voltages, each −10…10 V; held while stopped |
| `trigger` output | Enabled step's 10 V pulse, or gate in passthrough mode |
| `step1`…`step8` outputs | One-hot selected-step 10 V gates, including while stopped and with trigger disabled |
| `steps` output | Effective length minus one volts (0…7 V) |
| `clock`, `run`, `reset` outputs | 10 V timing pulses; Clock follows gate mode when passthrough is enabled |

Tempo is 15–960 BPM before CV. Internal frequency is
`clamp((tempo/60) * 2^clamp(tempoCV*tempo_amount, -64, 64), 0.000001, sample_rate/2)`.
The engine advances phase, consumes a crossing, wraps, then emits outputs.
Pausing freezes phase. Internal crossings during a reset guard are consumed,
without a delayed burst when the guard ends.

Effective length is `clamp(round(steps + stepsCV*steps_amount), 1, 8)`, with half
values rounded away from zero. If length shrinks below the selected column,
that column remains selected until the next accepted clock wraps to column one.

Run toggles playback and emits a 1 ms pulse. Reset selects column one, clears
phase and clock/trigger pulses, and emits a 1 ms RESET pulse. Reset also guards
against clocks for `ceil(sample_rate * 0.001)` samples, including the reset
sample. Manual and external clocks require running state and an expired guard.
Every accepted tick pulses, including a one-step pattern; wrapping does not
emit RESET. Pulse lengths use that same ceiling rule.

CV edges use hysteresis: at or above 2 V fires; at or below 0.1 V rearms.
Detectors track while stopped and guarded, so ignored edges are not replayed.
Queued actions execute in supplied order before the sample's CV run, reset,
and clock logic. Multiple accepted clocks can advance several steps in one
sample; overlapping output pulses naturally merge. With passthrough enabled,
Clock/Trigger use the accepted external gate or internal 50% phase gate;
manual Clock supplies a 1 ms pulse. Pausing or guarding suppresses these gates.

## Editing, replacement, and saving

Pattern cells, enabled flags, tempo, length, amounts, passthrough, and initial
running state are saved as patch parameters. Runtime position and Run-button
state are not saved. **Start running on load** sets initial running state and
also applies immediately when its value or explicit presence changes. Ordinary
compatible edits retain the current position, phase, run state, and pulses.
Changing a clock/run/reset source independently rearms that input's detector.

Deleting and recreating a module gives it a new performance identity, even if
one atomic edit batch recreates the same ID and kind. Old queued actions cannot
target it; its incident delayed cable samples and runtime state are reset.
Loading or fully replacing a document also retires its sequencer identities,
so a fresh preset honors its initial running state even when IDs match.
Compatible parameter/routing edits preserve performance state.
Graph replacement cannot transfer core action queues. The host dispatches new
actions to compatible active and retiring graphs during a crossfade.

## MCP performances

`trigger_module` queues a live `clock`, `run`, or `reset`; `inspect_performance`
reports applied, dropped, obsolete, expired, and backpressure counters.
The queue holds 512 events, with at most 64 actions per module per sample and
512 actions per callback. Events waiting for a newer graph expire after two
audio seconds. Closing the device seals its sender and accounts for remaining
actions. Callback handling uses fixed storage and does not acquire control locks.

For reproducible offline work, `render_audio`, `analyze_signal`, and
`capture_signal` accept `module_events`, for example:

```json
[
  {"frame": 0, "id": "sequence", "action": "reset"},
  {"frame": 4800, "id": "sequence", "action": "clock"}
]
```

Events must be sorted by frame, remain inside the render, and target valid
sequencers. Equal-frame events preserve array order; MIDI dispatches before
module actions. All targets are validated before rendering or creating a WAV.
Offline renders start with fresh runtime state and do not perform on the live
rack. `rotate_sequence` performs one atomic pattern edit with revision checking.
See [MCP](MCP.md) for requests and limits.

## Reference and remaining parity

This original Rust implementation follows an explicit functional contract. The
[VCV SEQ 3 manual](https://vcvrack.com/Free#vcv-seq-3) and
[pinned Fundamental source](https://github.com/VCVRack/Fundamental/blob/10dd0160c664770910e5584b7b00498cc48d9ddd/src/SEQ3.cpp)
are behavioral references, not copied implementation or panel assets.

It is **partial SEQ 3 coverage**, not sample-identical equivalence. In particular:

- The manual describes RESET on wrap, while pinned source only pulses reset
  controls. OpenPatch follows the latter policy.
- Pinned source's ordinary pulse-mode clock/trigger requires a changed step
  index; OpenPatch pulses every accepted tick, including length one.
- OpenPatch's manual clock observes the reset guard. Its CV detectors also
  track while paused/guarded, rather than deferring ignored edges.
- Saved runtime Run state, exact internal clock phase conventions, parameter
  mappings, and direct differential traces against the pinned target remain
  parity work. Native hardware playback and device lifecycle qualification
  remain separate from headless tests.

Tests cover edge thresholds, reset ordering, exact pulses, callback partitions,
length changes, chained modules, mono policy, event limits, stale handles,
identity replacement, state adoption, and allocation/deallocation-free engine
and callback paths. UI tests exercise cells, gate buttons, performance controls,
and the last step output. These prove the stated OpenPatch contract, not the
remaining VCV acceptance gates.
