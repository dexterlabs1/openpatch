# Stereo and polyphonic routing

Four engine-v2 modules extend the stock utility coverage with original Rust
implementations. These are mathematical signal processors, not analog circuit
models. Every cable still contributes one sample of delay; internal normaling
does not introduce an extra cable delay.

## Mid/Side

`mid_side` contains an encoder and decoder. It exposes inputs `left`, `right`,
`mid`, `side`, `encode_cv`, `decode_cv`, and outputs `mid`, `side`, `left`, `right`.
The native panel separates the encoder and decoder into columns.

For encoder width `e` and decoder width `d`:

```
mid   = (left + right) / 2
side  = (left - right) / 2 * e
left  = mid + side * d
right = mid - side * d
```

Each width knob ranges from 0–2, default 1. The effective width is
`clamp(knob * width_cv / 10, 0, 2)`. An unpatched width CV is 10 V. Negative CV
collapses the width to zero. These CV scaling and clamping details are explicit
OpenPatch policies; the reference manual does not specify them completely.

Each decoder input independently defaults to the corresponding encoder output.
Patch an external Mid or Side signal to replace that path. With both decoder
inputs unpatched and both widths at 1, the decoded result reproduces the encoder
inputs. Along this normalled path, setting either width to zero collapses the
stereo difference. For
polarity inversion or swapping, process the Side signal through a negative gain
and reconnect it; negative width is not supported by the width controls.

The encoder's voice count is the maximum of its L/R counts, with at least one
lane. The decoder uses its effective Mid/Side counts in the same way. Mono inputs
broadcast; missing lanes of shorter polyphonic signals are zero. Width CV follows
those lanes and does not increase their count. Zero-channel inputs use unpatched
normaling. The encoder outputs are bounded before the decoder uses them.

Choose **File → Stereo width study**, then Play. Two slightly detuned oscillators
feed the encoder. Turn either Width control to hear the stereo image narrow or
widen. The decoder outputs connect independently to STEREO AUDIO's L/R inputs;
headphones or stereo speakers make the difference easiest to hear.

## Mix 6

`mix6` sums inputs `in1`…`in6` into `out`. `level` is 0–1, default 1. `invert`
changes the output polarity, and `average` divides the sum by the number of
active input jacks before applying level. Both settings default off.

A patched 0 V signal counts toward the average; a zero-channel frame does not.
An empty mixer produces mono zero. The output uses the largest input voice count,
with mono broadcast and zero for missing lanes in shorter polyphonic inputs.
Averaging uses the same active-jack denominator for every voice. Complete sums
are averaged and scaled before the final arithmetic safety bound is applied.

## Octave

`octave` adds an integer pitch-voltage offset to `pitch` and emits `out`.
`octaves` is an integer from −4 through +4, default 0. `shift_cv` adds a rounded
integer offset, with half-integer ties rounded away from zero: +0.5 V adds one
octave and −0.5 V subtracts one. The CV shift is not restricted to the knob's
range. One volt always represents one octave.

Pitch determines voice count, with mono zero when unpatched, allowing Octave to
serve as a voltage source. Shift CV broadcasts from mono, or applies per voice;
missing lanes of a shorter polyphonic CV are zero. CV alone does not create more
voices. Final results retain the engine's ±1,000,000 V arithmetic safety bound;
there is no additional musical pitch-range limiter.

## Mult

`mult` copies `signal` to `out1`…`out8`. It has no parameters. Every output carries
an identical frame, including the input's zero-, one-, or sixteen-channel count.
It adds no processing delay internally. Routing through Mult instead of a direct
cable adds one more cable, and therefore one sample of total path latency.

## Validation and scope

Mid/Side, Mix 6, and Octave treat nonfinite input values as zero. Their arithmetic
outputs use the engine's ±1,000,000 V safety bound; Mult performs an exact frame
copy without arithmetic. Inactive output slots remain zero for engine-generated
frames.

The tests cover the stated equations, normaling and external overrides, width
control, averaging with zero-valued connections, pitch shifts, channel counts,
polyphonic isolation, finite arithmetic, named ports, persistence, callback
partitioning, and allocation-free processing/state adoption. Native gesture tests
cover named-port cable dragging and mixer toggle controls. DSP tests do not prove
identical behavior for every VCV context-menu option, bypass route, or unspecified
boundary condition.

The [official VCV Free manuals](https://vcvrack.com/Free) describe the target
functional roles and the Mid/Side equations. Their unspecified width-CV details,
negative rounding ties, and detailed channel choices are documented above as
OpenPatch policies. These implementations do not copy VCV source or artwork.
