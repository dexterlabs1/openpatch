# Clocked signal routing

Choose **File → Switched stereo voice**. Four pitch voltages feed a 4-to-1
selector; the selected pitch drives the VCO. A 1-to-4 router sends that voice
alternately to the left and right mixers. Both switches share the gate clock.
Edit the four pitch cells, change either switch's length, or slow the clock to
hear each route. De-click is enabled on the audio router and disabled on the
pitch selector so pitch changes remain immediate.

The panel offers **2 / 3 / 4 steps** and **De-click**. A yellow route indicator
shows the audio engine's selected path; red marks routes outside the current
length. Indicators stay unavailable until the live engine has applied the
current patch revision. Click and drag the jacks just like other rack modules.

## Ports and parameters

Both modules require version 2 patches. `steps` is an integer from 2 through 4,
default 4. `declick` is 0 or 1, default 0. Both are saved parameters and support
the same UI/MCP edits, revision checks and undo as other controls.

| Kind | Inputs | Outputs |
|---|---|---|
| `switch_1to4` | `clock`, `reset`, `signal` | `out1`…`out4` |
| `switch_4to1` | `clock`, `reset`, `in1`…`in4` | `out` |

Clock and reset read channel one only. A clock edge advances the selected
route; reset selects route one. When both edges arrive on the same sample,
reset wins. There is no reset guard: a clock one sample later advances normally.
Reducing length wraps an out-of-range selection to one immediately.

The edge detector arms low at or below 0.1 V and goes high at or above 2 V.
It starts uninitialized: a first high establishes state without an edge, and a
first intermediate value leaves it uninitialized. Normal startup's empty cable
frame arms low before the source's first signal arrives. Each graph cable still
adds one sample of delay.

## Polyphony and de-click

Every output has at least one channel. A router's count follows its signal
input, including on its silent routes. A selector's count is the maximum over
all four signal inputs, including unselected inputs and routes excluded by the
length control. Clock and reset never create voices.

With de-click off, the selected signal is copied with missing channels filled
with zero. A mono signal on a multi-channel selector therefore occupies only
channel one. Other router outputs contain zero volts but retain their channel
count.

With de-click on, four shared route gains move toward the one-hot selection at
400 gain units per second. A full 0→1 ramp takes approximately 2.5 ms. Gains
start at zero. Selector inputs use mono broadcast in this mode, while missing
lanes in short polyphonic inputs remain zero. Old and new routes can overlap
during a transition, including when the length is reduced.

Turning de-click off freezes its gain histories; turning it back on resumes
from those histories. It does not initialize them to the latest raw selection.
All signal values must remain finite: nonfinite values become zero and final
arithmetic is limited to ±1,000,000 V. This is a numerical safety limit, not
physical clipping or a circuit model.

## State and agent access

Compatible edits preserve selected route, gain ramps and unchanged input edge
detectors. Rewiring clock or reset rearms that detector independently. Rate or
kind changes start fresh. Removing and recreating a module, or loading/replacing
a complete document, gives it fresh runtime identity and clears incident delayed
cable history. Runtime route and ramp progress are not saved in patch files.

Switches expose read-only route telemetry to the native host. `trigger_module`
remains a sequencer performance operation and rejects switch targets before
queueing. An agent can patch a clock/reset source or use timed MIDI events with
a MIDI-to-CV source to operate switches. `capture_signal` inspects named outputs
and voices offline from reset; it does not capture the live rack.

Bypass freezes switch position, detectors, and fade gains, and emits mono zero
on every output. It is saved and undoable through the common [bypass controls](VOLTAGE_UTILITIES.md).

## Reference and validation boundaries

Behavior was checked against the
[pinned Fundamental switch source](https://github.com/VCVRack/Fundamental/blob/10dd0160c664770910e5584b7b00498cc48d9ddd/src/SequentialSwitch.cpp)
and [Rack detector dependency](https://github.com/VCVRack/Rack/blob/061ccf63c1758599396ac1bb10d47345d9d34076/include/dsp/digital.hpp).
The [Free manual](https://vcvrack.com/Free#vcv-sequential-switch-1-to-4) describes
the musical controls. The implementation and panels are original Rust and egui;
no upstream code or artwork is included.

OpenPatch uses double-precision gain state and explicit finite-value handling.
No sample-identical claim is made. Direct differential renders, VCV file/state
migration (including older de-click defaults) and native-device
qualification remain acceptance work. The engine and UI tests validate routing,
timing, channel policies, state adoption and identity, and bounded callback
behavior separately from those remaining compatibility requirements.
