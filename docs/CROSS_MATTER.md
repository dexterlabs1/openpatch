# Cross Matter

`cross_matter` is a native Rust adaptation of the Mutable Instruments Warps
(2014) modulator. It has its own OpenPatch identity, shared rack/browser panel
and document/MCP controls. It is not a binary Rack plugin, and it is not a
calibrated model of the Warps codec, CV front end or analog hardware.

Two audio inputs, CARRIER and MODULATOR, meet in one of seven algorithms.
ALGORITHM morphs continuously through crossfade, fold, analog (diode) ring,
digital ring, XOR, comparator and vocoder; between neighbours the source
renders both algorithms and interpolates, per oversampled sample, from the
previous block's position to the new one. TIMBRE is each algorithm's own
parameter: fold depth, ring drive, XOR blend, comparator mode, or the vocoder's
formant shift. LEVEL 1 and LEVEL 2 drive the two input amplifiers, which add
soft saturation as they rise. Cross-modulation runs at 6x oversampling through
the source polyphase converters; the vocoder is the source 20-band decimated
filter bank with envelope followers, release/freeze and a limiter. Between
comparator and vocoder the source crossfades through the raw modulator.

CARRIER selects the external input or a built-in carrier. With SINE, TRIANGLE
or SAW the carrier input becomes the carrier's frequency/phase modulation input,
LEVEL 1 sets its pitch (MIDI 36–96, shown in Hz) and a patched LEVEL 1 CV adds
1 V/oct. Cross-modulation uses sine/triangle/saw; the vocoder uses the matching
saw, pulse or filtered noise, and both render through the transition. AUX
carries both inputs after their level stages (external carrier) or the raw
built-in carrier, exactly as the source wires it.

MODE switches to the source's hidden frequency shifter (the firmware "easter
egg"). The knobs become SHIFT (built-in quadrature carrier, about ±1.6 kHz) or
PHASE (external carrier through the source Hilbert transform), UP / DOWN
balance, FEEDBACK and DRY / WET; OUT and AUX carry opposite shift directions.

Captions and readouts follow the selected mode: the algorithm readout names
the region or the blend ("Analog / Digital"), levels show percent or the
carrier pitch, and the guide line states the configured signal path. The two
lamps show actual delayed output volts. [Panel review](../design/reviews/cross-matter-v1/README.md).

## Native mapping and timing

Inputs read channel zero; outputs are mono. Audio uses the Warps codec scale:
±8 V is 16-bit full scale, inputs clip beyond it, and outputs use the same
scale (aux is already half-scale in the source). Away from 96 kHz, band-limited
resampling of saturated material can overshoot 8 V by under 10%. Mapping, informed by the
firmware's `cv_scaler.cc`, is an explicit OpenPatch policy:

* Algorithm: source pot curve of the knob plus voltage/5, clamped to 0–1, then
  the source's low-end bleed correction.
* Timbre: knob plus voltage/5, clamped.
* Levels: knob squared; a patched level CV multiplies it by voltage/5, clamped.
* Built-in carrier pitch: 60 × LEVEL 1 + 36, plus 12 × LEVEL 1 CV volts.
* Frequency shifter: the algorithm knob is the shift pot; algorithm CV
  voltage/5 is the shift CV and adds to the phase knob.

Nonfinite volts read zero. The firmware's ADC smoothing, calibration and
normalization probe are not modelled; an unpatched input simply reads 0 V.

The kernel always runs at its native 96 kHz in 60-frame blocks: its pitch
tables, filter-bank coefficients and converter FIRs are designed for that rate.
At other host rates, windowed-sinc resamplers (Blackman-Harris, 32 zero
crossings, cutoff 0.45 of the lower rate, exact integer timing) bridge to and
from 96 kHz. Measured round-trip response at 44.1/48 kHz is flat within 0.01 dB
to 1 kHz and 0.1 dB to 16 kHz. Latency is constant: 105 frames (2.2 ms) at
48 kHz and 63 frames at 96 kHz. Controls are sampled once per kernel block.

Same-rate graph adoption copies the complete runtime. Initialize and rate
changes start a freshly initialized kernel with empty resamplers. Bypass routes
CARRIER to OUT and MODULATOR to AUX unchanged and freezes processing. Saved
patches and presets contain authored controls only. The callback performs no
allocation, including bridge, vocoder, initialize, bypass and adoption paths.

## Source and qualification

Source: `pichenettes/eurorack` at `f4cfa7697095a28d292bff758beb51deb49b5a86`,
with its stmlib gitlink `e3bd7c9cc00e4364166f9905c0509b6ffd0535ec`. Complete
MIT notices are retained in the adapted files, the Core third-party notice and
the bundled `Warps-algorithms.txt`. The [reference runner](../scripts/warps_reference/README.md)
records 25 source hashes, compiler options and immutable input/output traces.
Tables are extracted bit-for-bit by `kernel/generate_tables.py`.

Reference `attempt-002` compiled the unmodified source (C++11 `-O2 -DTEST
-ffp-contract=off`) and ran 30 cases, 603,600 frames at 96 kHz. Comparison
`rust-comparison-004` ran the production Rust kernel on the same inputs:
**1,206,840 of 1,206,840 int16 main/aux samples match bit for bit** inside the
source's defined domain, across every algorithm and blend, the transition, the
vocoder (formant, release, freeze), drive extremes, clipping, noise-gate tails,
every built-in carrier including the noise carrier, deep FM, pitch-table
limits, the frequency shifter with each carrier, and live mode switching.

Two cases also reach source undefined behaviour, reported separately and never
counted as matched: the shifter's phase rotation reads one element past
`lut_sin` when the interpolated phase overshoots 1.0 (one block, 2 of 120
samples differ by up to 3 LSB), and overdriven inputs at maximum timbre push the
wavefolder's index past `lut_bipolar_fold` (two blocks, 20 of 240 samples
differ). C++ reads neighbouring memory there; the Rust kernel clamps to the
table and counts the event. An in-range variant of the shifter case matches
exactly.

A native fixture freezes two reference cases whose controls are exactly what
the host produces for fixed knob settings: at 96 kHz the actual module output
matches the C++ codec words bit for bit after its 63-frame latency. Other host
rates pass through the declared resampler and are tested for latency, flatness
and pitch, not bit identity.

This is a bounded portable-source result on macOS ARM. Cortex-M instruction
identity, codec/ADC behaviour, the firmware's shared random stream (its CV
scaler also draws noise each block) and hardware accuracy are not established.

Native tests cover the host fixture, carrier pitch at 44.1/48/96 kHz, distinct
algorithm regions, chunk boundaries and adoption, bypass routing and freeze,
initialize versus a fresh module, invalid edits and allocation-free callback
work at five host rates across vocoder, transition, noise-carrier and shifter
paths. Panel geometry at five scales, mode-dependent captions, drag/history and
measured lamps have focused UI tests; MCP tests cover capture, parameters,
undo/redo, initialize, bypass, presets and saved reopen. The example
[`cross-matter-morphing-duet.json`](../examples/cross-matter-morphing-duet.json)
sweeps the algorithm and timbre with two LFOs.

## Deviations from Warps

* Fixed 96 kHz kernel behind an original host-rate resampler; the firmware runs
  its codec at 96 kHz directly.
* CV/level/pitch mapping is a documented native policy, not the calibrated
  firmware ADC path; no audio-input normalization probe.
* Per-instance random state seeded as the source (0x21) instead of the
  firmware's process-global generator.
* Out-of-table reads that are undefined in the source are clamped.
* Output lamps show output volts; the Warps RGB algorithm LED is not modelled.
