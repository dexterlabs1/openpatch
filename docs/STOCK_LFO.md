# Stock LFO

`stock_lfo` provides four stock-style modulation waveforms, exponential FM, PWM,
a shared clock input, polyphonic reset, and actual phase/mode indicators.
It defaults to unipolar output. The older `multi_lfo` and `lfo` kinds retain
their saved controls, defaults, waveform origins and timing.

The rack and library share one original **STOCK LFO** panel. Its phase lamp is
red/green for mono and blue for polyphony. Offset and Invert lamps reflect the
running Engine's published state and remain emissive in a dark room. A preview
has no live lamps or clock reading. The configured controls remain available.

## Controls and signals

Inputs: `fm`, `pwm`, `reset`, `clock`.
Outputs: `sine`, `triangle`, `saw`, `square`.

| Saved parameter | Default | Range | Meaning |
|---|---:|---:|---|
| `rate` | 1 | −8…10 | Logarithmic rate; unpatched frequency is approximately `2^rate` Hz |
| `pulse_width` | 0.5 | 0.01…0.99 | Square-wave high fraction before PWM |
| `fm_amount` | 0 | −1…1 | Signed exponential depth; +1 gives one octave per volt |
| `pwm_amount` | 0 | −1…1 | Width adds `PWM / 10 × depth` before clamping |
| `offset` | 1 | 0 or 1 | Bipolar ±5 V or unipolar 0–10 V |
| `invert` | 0 | 0 or 1 | Invert the shape before adding unipolar offset |

The Rate readout uses the Engine's actual shared clock-frequency state: exactly
2 selects Hz; another valid value selects multiplier `2^rate / 2`. This follows
the reference quantity, including its 2 Hz special case. The clock state is a
rate basis, not a guarantee that an external clock has acquired a tempo: the
unpatched basis is 2 Hz, and a fresh module starts at 1 until it processes input.
Without valid telemetry, the panel shows **CLOCK —** and both configured rate
interpretations. Exact entry uses Hz in that unavailable state and shows its
unit explicitly. An open editor keeps the units it had when opened, even if
clock telemetry changes. A newer graph revision still prevents stale edits.

## Timing and polyphony

The numerical target is the [pinned Fundamental LFO](https://github.com/VCVRack/Fundamental/blob/10dd0160c664770910e5584b7b00498cc48d9ddd/src/LFO.cpp).
Each sample advances phase, applies a reset edge, then emits all connected
waveforms. Offset changes waveform origins as well as voltage mapping; a reset
unipolar sine begins at zero. Saw and triangle use nearest-even rounding at
phase boundaries. These are unfiltered CV waveforms: discontinuities can alias
at audio rates. Adding antialiasing would change this target.

FM sets the channel count, with a minimum of one. FM reads each raw lane without
mono broadcasting. PWM and reset broadcast mono; missing short-poly lanes are
zero in ordinary Engine frames. Hidden lanes in an active four-lane group keep
advancing; wholly inactive groups freeze and resume their phase/reset history.
Connected outputs emit the active width; unconsumed outputs emit zero channels.
Consumers include cables, the selected output, and temporary analysis taps.
Phase, clock and lamp processing continue without output consumers.

Clock measurement is shared across voices and reads channel zero only. Its
scalar detector begins uninitialized: an initial high arms it without an edge;
low at or below 0.1 V then high at or above 2 V produces an edge. A rising edge
measures the elapsed f32 timer, clears it, and accepts 0.001–1000 Hz. Rejected
fast measurements still clear the timer. Disconnecting sets the basis to 2 Hz
and preserves detector/timer history. Missing pulses retain the last accepted
rate. Clock edges change rate without resetting phase.

Reset detectors begin high, so a held initial high does not reset repeatedly.
A low at or below 0.1 V arms each lane; a high at or above 2 V resets it after
that sample's phase advancement. Whole-module Initialize clears phases and the
clock timer and sets the basis to 1; it retains detector histories, lamps and
the lamp-divider phase. The UI/MCP Initialize command also restores the six
control defaults. Randomize leaves the two modulation depths unchanged.

## State, observation and MCP

Same-ID/kind/sample-rate graph adoption retains bounded phase, detector, timer
and lamp state while receiving controls win. Fresh instances and rate changes
start fresh. OpenPatch bypass freezes processing and observations. Every cable
still contributes one sample of delay; direct-class comparisons omit that host
transport, and graph tests cover it separately.

Live `inspect_performance` reports optional
`performance.modules.<id>.lights.clock_frequency`. Five brightness components
are phase red, green, blue, Invert and Offset. Lamps update every 16 processed
samples; clock-frequency observation updates each sample. Stale graph revisions,
foreign instances, invalid fields and a closed backend must not supply a valid
current reading. A headless MCP process without a callback reports performance
as unavailable. These observations are not saved parameters or preset state.

MCP uses the usual `set_parameter`, `connect`, `capture_signal`, `analyze_signal`,
preset, history and project tools. Select a named output explicitly. Rate is
saved as log2 Hz, so `rate: 2` requests 4 Hz while unpatched, or a 2× multiplier
with a clock. Parameter changes preserve runtime phase; a reset is a separate
signal or Initialize action.

## Recorded comparison and remaining gates

The [frozen acceptance](../scripts/stock_lfo_reference/acceptance.json) was
declared before the first comparison. The [macOS ARM report](benchmarks/stock-lfo-reference.json)
records 693 cases per rate and 393,046 compared frames at 44.1, 48 and 96 kHz.
Every output count, phase, detector, clock, divider, lamp and connected active
triangle/saw/square value matched the actual class exactly. Maximum sine
error was 0.954 µV, below the predeclared 3 µV threshold. All 1,572 separately
labelled Rust-only safety frames passed; they are not oracle-equivalence cases.
Independent checks verified 782 retained artifacts, 74 production source files
and 668 SDK headers. [Reproduction instructions](../scripts/stock_lfo_reference/RUNNING.md)
keep the reference SDK separate from the native app.

The compared domain includes exponent-zero behavior and positive-infinity
frequency intermediates consumed by the half-cycle/sample cap. Outside that
frozen approximation domain, finite exponential pitch saturates to [−127,128];
nonfinite input voltages become zero. Finite CV is not arbitrarily clipped
before multiplication. Disconnected/inactive output storage is zero in the
OpenPatch host and excluded from raw SDK voltage comparison; all counts and
internal state remain compared. No active sample or warmup frame is discarded.
The timer's <0.001 Hz rejection branch is unreachable from fresh state at the
three tested rates because f32 accumulation reaches a fixed point before 1000 s;
the acceptance records the arithmetic proof rather than claiming it ran.

This is digital behavior qualification for the recorded compiler/SDK/platform
and domain. Platform/native captures, real input/device interaction, Rack
state/import mapping and full host parity remain separate gates. It is not a
hardware circuit model. OpenPatch uses original Rust and original panel artwork;
GPL reference source and its license remain in ignored comparison evidence.
