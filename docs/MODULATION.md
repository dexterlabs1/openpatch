# Clocked modulation

`multi_lfo` is an original Rust functional CV oscillator for version-2 patches.
It supplies simultaneous sine, triangle, saw, and square outputs, exponential
FM, pulse-width modulation, clock synchronization, and hard reset. It is not a
circuit model or a sample-identical VCV implementation. The existing `lfo`
retains its sine/offset and additive-Hz behavior for older patches.

The [VCV LFO manual](https://vcvrack.com/Free#vcv-lfo) is the functional reference
for the four waveforms, output ranges, inversion, and a clock that changes
frequency independently of phase. OpenPatch's exact formulas, phase origins,
clock acquisition, and boundary policies below are explicit implementation
choices. Direct comparison against VCV remains an acceptance task.

## Play it

Choose **File → Clocked modulation**, then **Play**. The gate clock runs at
2 Hz. The LFO's **0.5×** clock ratio produces 1 Hz modulation after the second
clock edge. Its unipolar sine controls the OTA VCA; its triangle passes through
an attenuverter to sweep the ladder filter by ±1 octave. Stereo Audio duplicates
the mono voice into both device channels. The unused saw and square outputs are
available for patching.

The frequency knob covers 1/256–1024 Hz with equal travel per octave and 2 Hz
at its center. With a cable patched to CLK, the display changes to **CLOCK
RATIO**; its center is **1×**. Double-click the displayed value to type Hz or a
multiplier, depending on that display. Clock ratio indicates the requested
setting, not whether two clock edges have arrived. Double-click the knob to
reset to its center. The range button switches **±5 V / 0–10 V**; **Invert**
flips all four shapes within that range.

The example is [clocked-modulation.json](../examples/clocked-modulation.json).
Its filter and VCA retain their documented oversampling latency; the LFO adds
no filtering delay. Each cable adds one sample under engine v2.

## Signal and parameter contract

Inputs, in order: `fm`, `pwm`, `reset`, `clock`.
Outputs, in order: `sine`, `triangle`, `saw`, `square`.

| Parameter | Default | Range | Meaning |
|---|---:|---:|---|
| `frequency` | 2 | 1/256–1024 | Free-running Hz; locked clock multiplier = value / 2 |
| `fm_amount` | 0 | −1…1 | Exponential FM attenuverter; +1 gives 1 V/oct |
| `pulse_width` | 0.5 | 0.01…0.99 | High fraction of square cycle |
| `pwm_amount` | 0 | −1…1 | Signed PWM depth |
| `unipolar` | 0 | Integer 0 or 1 | 0 = ±5 V; 1 = 0–10 V |
| `invert` | 0 | Integer 0 or 1 | Invert before voltage mapping |

At phase `p` in `[0,1)`, normalized shapes are:

- Sine: `sin(2πp)`.
- Triangle: `1 − 4 × abs(p − 0.5)`.
- Saw: `2p − 1`.
- Square: `+1` when `p < width`, otherwise `−1`.

Inversion negates the normalized shape. Bipolar mapping is `5 × shape`;
unipolar mapping is `5 × (shape + 1)`. Phase begins at zero. A sample emits
current phase, then advances. Outputs are intentionally unfiltered CV shapes;
saw and square have discontinuities and can alias when used as audio sources.

`width = clamp(pulse_width + pwm_amount × PWM / 10, 0.01, 0.99)`.
Frequency is `clamp(base × 2^clamp(fm_amount × FM, −64, 64), 0.000001, 1024)` Hz.
The free-running base is `frequency`; the acquired-clock base is
`sample_rate / period_frames × frequency / 2`. Nonfinite input voltages become
zero. Parameters outside their specified bounds reject at compilation.

## Clock, reset, and polyphony

Clock and reset use independent Schmitt detectors: rising at ≥1 V, rearming
at ≤0.1 V. A newly active high input counts once. The first clock rising edge
arms measurement; the second acquires the period in sample frames. Until then,
the LFO free-runs. Clock edges update rate without moving phase. Missing pulses
hold the acquired rate; the next edge measures the entire elapsed interval.
The elapsed-frame counter saturates and restarts on the next edge.

Reset puts phase at zero before that sample's output. A held-high reset does
not repeatedly reset. Reset leaves clock acquisition intact, even if both
inputs rise on the same sample. Cable transport still takes one sample.

FM sets the voice count, with a minimum of one and maximum of 16. Other inputs
do not create voices. Mono auxiliary inputs broadcast; absent lanes in a
short polyphonic input supply zero. A clock port with zero channels clears
acquisition and resumes free-running without a phase jump. A missing lane in
a nonempty clock port holds its previous tempo. Retired and newly activated
voices reset their phase and detector state.

Live state adoption at matching node ID, kind, and sample rate preserves
phases while retaining new parameters. Clock and reset detectors are adopted
independently only when their incoming source ID, source kind, and output port
are unchanged. Rewiring clears the corresponding detector/acquisition state.
Compilation prepares routing and fixed voice storage off the audio thread;
processing and compatible state adoption allocate and free no memory.

## Validation and remaining parity work

Independent public-engine tests cover waveform landmarks and voltage ranges,
FM and PWM, clock acquisition and tempo changes, missing pulses, reset,
polyphonic broadcast/retirement, endpoint-sensitive state adoption, callback
partitioning, and legacy behavior. Internal tests exercise arithmetic bounds,
nonfinite input safety and counter saturation. Allocation tracking covers the
first render and compatible graph adoption. Native headless interaction tests
cover all output jacks, voltage switches, and typed Hz/multiplier conversion.
MCP tests inspect all four outputs and render/undo the bundled voice.

Remaining work includes direct VCV differential comparison, complete native
configuration/bypass parity, and physical device playback checks. Waveform and
clock contracts are tested; this does not establish physical hardware fidelity
or complete Rack LFO equivalence.
