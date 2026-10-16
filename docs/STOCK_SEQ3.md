# Stock SEQ3

`stock_seq3` is a separate eight-step, three-row voltage sequencer targeting the
[pinned Fundamental SEQ 3](https://github.com/VCVRack/Fundamental/blob/10dd0160c664770910e5584b7b00498cc48d9ddd/src/SEQ3.cpp).
The existing `seq3` retains its compensated clock, integer pulse timers and
ordered convenience actions. Existing patches are not migrated implicitly.

The rack and browser share an original **STOCK SEQ3** panel with 24 directly
editable voltages. Tempo displays BPM, with exact entry and fine adjustment.
Clock, Run, Reset and the eight gate buttons use held physical controls. Gate
passthrough is a dropdown; Rotate moves all three rows and gate flags together
in one revision and undo step. The 27 actual source lamp components occupy 19
logical telemetry groups, including yellow selected-step and red out-of-range
indicators. Missing telemetry leaves lamps inactive; configured controls remain
visible. Room lighting does not extinguish emitted light.

## Controls

| Saved parameter | Default | Range | Meaning |
|---|---:|---:|---|
| `tempo` | 1 | −2…4 | Raw log₂ Hz; display is `60 × 2^tempo` BPM |
| `tempo_amount` | 1 | 0…1 | Tempo CV adds octaves per volt |
| `steps` | 8 | Integer 1…8 | Base sequence length |
| `steps_amount` | 1 | 0…1 | Steps CV adds steps per volt before rounding |
| `row1_1`…`row3_8` | 0 | −10…10 V | Three independent eight-cell voltage rows |
| `running` | 1 | 0 or 1 | Durable Run state; live callback state is authoritative |
| `gate1`…`gate8` | 1 | 0 or 1 | Durable trigger-enable flags |
| `passthrough` | 0 | 0 or 1 | Forward the clock gate instead of generating index-change pulses |

Inputs: `tempo`, `clock`, `run`, `reset`, `steps`. Inputs read channel zero.
All sixteen outputs are mono: `cv1`, `cv2`, `cv3`, `trigger`, `step1`…`step8`,
`steps`, `clock`, `run`, `reset`. Selected-step outputs are 10 V; Steps outputs
`effective length − 1` volts. Gate flags affect Trigger rather than row voltages
or the selected-step outputs.

## Timing

Timing follows the source order and float32 arithmetic. Run is processed first,
then Reset, then Clock. The scalar input detectors and button detectors begin
uninitialized, so a startup high arms a detector without firing. Clock detectors
stop updating while paused. Disconnecting the external clock retains its edge
history; an external clock freezes internal phase.

Reset selects step one and clears phase. An external edge is suppressed while
the reset pulse is active; the manual Clock button bypasses that guard. Internal
phase accumulates while the guard suppresses advancement and wraps when the
crossing is accepted. Length changes do not immediately force an existing
selected index into the new range.

Clock pulses follow a **final index change**, including Reset from another
step. A length-one tick need not pulse. Passthrough freezes the pending clock
pulse generator, then resumes it when pulse mode returns. Pulse remaining time
is float32 and may end with a small negative residue; it is not replaced by an
integer sample counter.

Initialize restores eligible controls, selects step one, enables gates and
disables passthrough. It preserves Run, phase, detector histories, pulse tails,
held levels and lamp histories. Compatible graph adoption preserves runtime;
new controls win. Fresh IDs start fresh. Generic OpenPatch bypass freezes this
processor and exposes zero outputs; this is a host policy rather than a claim
about Rack engine bypass.

## MCP and persistent state

Physical actions are `clock_press` / `clock_release`, `run_press` /
`run_release`, `reset_press` / `reset_release`, and `seq_gate1_press` /
`seq_gate1_release` through eight. Sample these levels on separate audio frames
when writing offline traces. Press and release in the same sample leave a low
physical level and do not create an edge. Native short clicks wait for callback
acknowledgment before releasing, then wait for a sampled low level before
accepting another click. Releases remain available while bypassed.

`clock`, `run`, and `reset` remain convenient host extensions. On this kind they
coalesce once per action per sample and bypass button startup suppression;
they are distinct from physical detector behavior. The legacy kind's ordered
actions retain their existing semantics.

Project saves, presets, history and duplication capture one coherent live
Run/gate/passthrough bank. Partial durable edits apply only the requested flags
after runtime adoption, preserving unrelated newer CV/button changes. A row-only
preset does not restore stale flags. At the transport boundary, pending field
assignments compose across prepared graph generations and each field applies
once. State-dependent Session commands require the latest callback revision;
a second separate commit before that publication is rejected. Use one atomic
edit batch for combined flag changes. Rotation and randomization explicitly assign the
eight gate flags; neither assigns Run. Initialize assigns gates and passthrough.
Seeded randomization uses OpenPatch distributions rather than Rack's global
random-generator sequence. A legacy Rack data load without a passthrough field selects true, whereas a new
OpenPatch module defaults to false; the reference adapter tests this distinction.

## Qualification status

The bounded DSP comparison passes: **3,421,221 reference frames**, all **307,909,890
compared float32 fields exact**, and **5,970 separate Rust-only safety frames**.
All 790 retained artifacts, 76 recorded production files and 668 SDK
headers were verified; the five independent frozen declaration files remain
unchanged. After an unrelated Pulses lamp-capacity guard was corrected, the
current-source release probe was rebuilt and its whole executable was byte-for-byte
identical to the executed probe. The original execution evidence is reused through
that identity; no new DSP execution is claimed. The original 790-artifact archive
is unchanged, with a separate current-build addendum recording 75 unchanged source
files and the one Pulses guard change. Pulses is covered separately by its engine
regression. Original system-library bytes were not hashed; that provenance limit
is explicit. See the [original result](benchmarks/stock-seq3-reference.json) and
[current-build equivalence](benchmarks/stock-seq3-build-equivalence.json).
Local host/UI checks pass: 1,884 Rust tests (six existing ignored) and all 92
Python/MCP checks. Platform CI and reviewed native captures passed in
[PR13](https://github.com/dexterlabs1/openpatch/pull/13).
No complete parity, performance advantage, hardware accuracy, native Mac
input/audio-device, general `.vcv` import or external plugin claim is made.

The independent precomparison declaration freezes 3,421,221 reference frames
and 5,970 separate Rust-only safety frames at 44.1, 48 and 96 kHz. Its gate compares
all 90 output/state fields as exact float32 bits, every frame, including all 27
stored lamps. Low-tempo fixtures span full cycles. Physical controls, timing,
load/Initialize/adoption/rotation and source-defined finite exponent boundaries
are included. Undefined source exponents and nonfinite inputs are safety-only.
See [the frozen declaration](../scripts/stock_seq3_reference/README.md).

The actual SDK class, headers, library, compiler command, raw inputs/traces and
production-source hashes are retained separately in ignored evidence. The SDK
and reference implementation are not application dependencies. The UI uses
original artwork, the selected Rajdhani font and OpenPatch's shared panel code.
