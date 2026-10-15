# Random Values

`random_values` holds seven banks of random control voltages. Each output carries
up to sixteen voices. A rising trigger refreshes the corresponding voice across
all seven outputs; the PUSH button refreshes every currently active voice. The
six ranges are 0–10 V, 0–5 V, 0–1 V, ±10 V, ±5 V and ±1 V. Changing range affects
the next draw, leaving existing voltages intact. These signal roles follow the
[VCV Random Values manual](https://vcvrack.com/Free#VCV-Random-Values).

Open **File → Random values voice** for a clocked melody. Random output 1 feeds a
pentatonic quantizer and oscillator; the Scope compares outputs 1 and 2. The
panel and browser share a renderer. The range chooser and precise seed field
work with the usual guarded edit/history commands. Live readouts show actual
held voltages; unavailable telemetry is marked rather than simulated.

## Voices, triggers and controls

The active voice count is `max(1, TRIG channels)`. Unpatched TRIG therefore leaves
one active voice. All 112 voltages remain stored when voices become inactive;
reactivating a voice exposes its retained value. Mono triggers do not broadcast
across sixteen voices. Seven named ports, `out1` through `out7`, retain the
engine's normal one-sample cable propagation delay.

Triggers use 0.1 V low and 1 V high thresholds. An initially high trigger arms the
detector without drawing a new value; a subsequent low/high transition draws.
This avoids replacing a loaded value immediately just because its trigger was
already high. Nonfinite trigger input is treated as zero. Inactive detectors
freeze alongside the held voltages.

PUSH uses ordered runtime press/release actions, not a saved button parameter.
Holding the button does not continually draw. A CV edge and the first effective
PUSH edge in the same sample coalesce for each affected voice. A later deliberate
release/press pair in that sample produces another draw. Emergency release drops
held and pending physical controls without changing held voltages or random state.

## Persistence and reproducibility

The visible controls are `range` (integer 0–5, default 0) and `seed` (integer
0–16,777,215, default 1). A new module fills all 112 values from its seed. Changing
the seed explicitly starts a fresh sequence and fills all values again. A normal
range edit preserves both held voltages and the generator position.

The original generator uses xorshift32 shifts 13/17/5, starting at
`seed XOR 0xA341316C`. It advances before each draw, takes its upper 24 bits divided
by 2²⁴, then applies the selected range with float32 arithmetic. Draw order is
voice first, output second. This is an OpenPatch reproducibility contract, not a
claim to reproduce VCV's random sequence.

Project capture stores the actual 112 float32 values and the current nonzero
32-bit generator state. Reopening retains both the held voltages and the next
draw. Hidden voices are included. Live capture requires one coherent callback
frame at the current patch revision; stale or mixed snapshots cause a retry
rather than silently saving initial values. Dirty observation also includes the
persistent state, so a trigger can make the document unsaved without a parameter
edit. Physical buttons and trigger detector histories remain transient.

The current patch format represents a complete snapshot in 114 reserved
parameters: `value1_1` through `value7_16`, plus `rng_hi` and `rng_lo`, two exact
16-bit integer halves. These are storage fields, hidden from ordinary controls
and search. They must be supplied together or omitted together. Finite saved
voltages may lie anywhere from −10 to +10 V, independently of the currently
selected range, because changing range does not alter existing voltages. A zero
RNG state, partial snapshot, nonfinite value or invalid integer is rejected.

Save, project capture, module presets, duplication and history use the same typed
state. Explicit restores take precedence over state adoption even when configured
parameters compare equal. Initialize restores the control defaults while retaining
held values and random-generator position. Randomize refreshes all 112 values as
one undoable edit. Its required request seed is mixed with the module ID using
FNV-1a, then used as a fresh nonzero generator state. The same ID, request seed
and range therefore produce the same values regardless of intervening triggers;
visible seed and range remain unchanged. PUSH and TRIG instead continue the
current sequence. Reserved storage fields are never randomized separately.

## Validation and boundaries

Core checks cover exact seeded initialization, all six ranges, independent
polyphonic triggers, inactive voice retention, simultaneous manual/CV edges,
ordered retriggers, snapshot validation, subsequent-draw restoration, state
adoption, bypass and callback allocation behavior. Host checks exercise real
callback-triggered values through coherent saving, history and reopening.
Headless UI checks cover control commands, release safety, named sockets,
matching previews and text separation. JSON-RPC integration independently checks
all 112 initial draws for two boundary seeds in every range, then checks a manual
reroll after restoring the saved state.

The [pinned reference module](https://github.com/VCVRack/Fundamental/blob/10dd0160c664770910e5584b7b00498cc48d9ddd/src/RandomValues.cpp)
informed behavior and persistence requirements; its GPL code and artwork were
not ported. The implementation is original MIT Rust. Seed control, deterministic
sequence restoration and explicit startup/manual ordering are OpenPatch policies.
A saved voltage source is not an analog circuit model. Native macOS gesture and
audio-device qualification remains separate from automated checks.
