# String Field

`string_field` is a native Rust adaptation of the Mutable Instruments Rings
(2015) resonator. It has its own OpenPatch identity, shared rack/browser panel
and document/MCP controls. It is not a binary Rack plugin, and it is not a
calibrated model of the Rings codec, CV front end or hardware.

A resonator is excited by an audio input or, with IN unpatched, by a built-in
exciter on each strum. MODEL selects one of the six source models:

* **Modal**: a bank of band-pass modes (up to 60 with one voice; 28, 16 and
  12 with two, three and four). STRUCTURE sets the inharmonicity of the
  partials, BRIGHTNESS and DAMPING their tilt and decay, POSITION the pickup
  weighting of odd and even modes. The internal exciter is a filtered pulse.
* **Sympathetic strings**: eight Karplus-Strong strings per voice (four with
  two voices, two with three or four), tuned by STRUCTURE to intervals around
  the note. With the internal exciter, the first string is plucked by a
  filtered noise burst and the others ring in sympathy.
* **Inharmonic string**: one string per voice with stiffness: STRUCTURE
  below the centre curves the bridge, above it adds all-pass dispersion and,
  near the top, dispersion noise.
* **FM voice**: a two-operator FM voice whose envelopes follow the input's
  level and spectral centroid. STRUCTURE sets the frequency ratio, BRIGHTNESS
  the modulation index, DAMPING the decay, POSITION the feedback (negative
  below the centre).
* **Chord strings**: the sympathetic strings tuned to one of eleven chords
  selected by STRUCTURE (with the source's hysteresis); with three or four
  voices the strings play intervals.
* **String and reverb**: the inharmonic string into the source's stereo
  reverb; DAMPING also sets reverb time and amount, POSITION the stereo mix.

POLYPHONY sets one to four voices. Each strum moves to the next voice (three
voices follow the source's 1-0-2 pattern) so earlier notes ring on. With one
voice the ODD and EVEN outputs carry two sets of partials or pickups; with
more, odd and even voices go to separate outputs. Changing MODEL or POLYPHONY
restarts the resonators, as the firmware buttons do.

Strumming follows the hardware's jack normalization: STRUM triggers when
patched. With STRUM unpatched the module strums on V/OCT changes above 0.4
semitone when V/OCT is patched, on input onsets detected by the source's onset
detector when only IN is patched, and never when neither is (the source stays
silent). The source note filter and strum inhibit (20 blocks, 80 for onsets)
apply.

Captions follow the model (sympathetic strings: INTERVALS; inharmonic string
and string with reverb: STIFFNESS; FM voice: RATIO, INDEX, DECAY, FEEDBACK;
chord strings: CHORD). FREQUENCY reads as a note name, the chord-strings
STRUCTURE as the chord, the attenuverters as signed percent. The guide line
states the configured model and voices. Output lamps show actual delayed
output volts; the strum lamp lights for 60 ms when the kernel accepts a strum.
[Panel review](../design/reviews/string-field-v1/README.md).

## Native mapping and timing

Inputs read channel zero. The exciter input uses a codec scale of ±5 V
(16-bit full scale, clipped beyond it) and passes the firmware's input noise
gate; outputs use ±5 V, and the source limiter keeps them near ±4 V,
soft-clipping towards 5 V. With only one output patched it carries the sum of
both. Mapping, informed by the firmware's `cv_scaler.cc`, is an explicit
OpenPatch policy:

* FREQUENCY: 0..60 semitones above MIDI note 12 (C0). With V/OCT patched it is
  quantized to semitones with the source hysteresis and V/OCT adds 12
  semitones per volt; unpatched, it is continuous and the note is the knob.
* Frequency CV: volts × 9.6 semitones, smoothed as the source smooths its FM
  CV, times the quartic attenuverter, limited to ±48 semitones.
* Structure, brightness, damping, position: knob plus attenuverter ×
  |attenuverter| × volts/5, clamped to 0..1; structure to 0..0.9995, as the
  firmware constrains it. The chord is round(10 × structure) with the source
  hysteresis.
* The total pitch is limited to MIDI notes −36..132, which keeps every source
  pitch-table read and FM phase increment in range.
* STRUM: a rising edge above 1 V after falling below 0.1 V sets the source
  strum for the next kernel block.

Nonfinite volts read zero. The firmware's ADC smoothing of pots and CVs,
calibration and normalization-probe detection are not modelled; jack presence
replaces the probe.

The kernel always runs at its native 48 kHz in 24-frame blocks of 16-bit
codec frames. At 48 kHz the host feeds it directly; at other rates the
windowed-sinc resamplers shared with Cross Matter and Grain Weather bridge to
and from 48 kHz. Latency is constant: 27 frames (0.56 ms) at 48 kHz. Controls
are sampled once per kernel block.

String delay lines (eight strings, 2,048 + 1,024 samples each) and the
32,768-word reverb memory are allocated when the module is built; the
callback performs no allocation, including model/polyphony changes, bridge,
initialize, bypass and adoption paths. Same-rate graph adoption copies the
complete runtime. Initialize and rate changes start a freshly initialized
kernel. Bypass routes IN to both outputs. Saved patches and presets contain
authored controls only, never string or reverb memory.

## Source and qualification

Source: `pichenettes/eurorack` at `f4cfa7697095a28d292bff758beb51deb49b5a86`,
with its stmlib gitlink `e3bd7c9cc00e4364166f9905c0509b6ffd0535ec`. Complete
MIT notices are retained in the adapted files, the Core third-party notice and
the bundled `Rings-algorithms.txt`. The [reference runner](../scripts/rings_reference/README.md)
records 31 compiled source hashes plus four reference-only files
(`rings.cc`, `cv_scaler.cc/h`, `ui.cc`), compiler options and immutable
input/output traces. Tables are extracted bit-for-bit by
`kernel/generate_tables.py`.

The oracle runs the unmodified `rings::Strummer` and `rings::Part` with the
DSP lines of the firmware's `FillBuffer` (input clearing while the exciter is
normalized, the input noise gate and the `Clip16` output conversion), driven
with `PerformanceState`/`Patch` values, one fresh process per case over
zero-initialized static objects as in the firmware.

Reference `attempt-001` compiled the source (C++11 `-O2 -DTEST
-ffp-contract=off`) and ran 71 cases, 163,800 blocks (3,931,200 frames) at
48 kHz. Comparison `rust-comparison-002` ran the production Rust kernel on the
same inputs: **7,862,400 of 7,862,400 int16 odd/even samples and all 163,800
per-block strum flags match bit for bit**, with zero guarded reads and no
excluded samples. The cases cover all six models at polyphony 1, 2, 3 and 4
with the internal exciter and external strums, every model with an external
exciter at one and two voices, continuous excitation, the noise gate, strums
from V/OCT changes and from input onsets, the silent all-normalized state,
retriggers inside the strum inhibit, all eleven chords at each polyphony,
structure 0.9995 with damping 1.0, the crossfade to infinite decay, both
dispersion signs, notes below the delay line (the linear upsampler), high
notes, FM feedback of both signs, ±48-semitone FM sweeps, sympathetic glide,
mid-case polyphony and model changes and a second RNG seed.

The source's global LCG feeds the plucker's noise burst and the dispersion
noise. Each case seeds it before `Init` (`0x21`, the firmware's static initial
value, or 123456789 in one case) and the Rust kernel seeds its per-instance
generator identically. In the firmware the CV scaler's normalization probe
also draws one word per block, so hardware noise sequences are not claimed.

**No source bug needed a deviation.** After the 2019 rename the only commit
touching `rings/dsp` is `43b9c9f` (2020), which initializes
`Resonator::previous_position_`; the pinned revision includes it. Defined but
notable source behaviour is reproduced as is: the pinned `dsp.h` defines
`BRYAN_CHORDS`, so the chord model uses Bryan Noll's table; `lfo_frequencies`
has seven initializers for eight strings (the eighth string's LFO is still);
a modal bank with an odd number of audible modes also runs the next, clamped
filter; several members are zeroed only by static storage and survive model
changes (`String::src_phase_`, `FMVoice::previous_sample_`,
`Plucker::comb_filter_gain_`, the reverb low-pass states).

Undefined-behaviour boundary: out-of-range float-to-integer conversions
saturate on Cortex-M, on the oracle's clang arm64 code and in Rust. Table reads
that would leave an object are guarded and counted (none occurred); the
`lut_4_decades` read at damping 1.0 lands exactly on the last entry with a
zero fraction and returns it, as the source effectively does.

A native fixture freezes two reference cases (450 blocks each) whose controls
are exactly what the host produces for fixed knobs with only IN patched: at
48 kHz the actual module output matches the C++ codec words bit for bit after
its 27-frame latency, including the onset-detected strums. Other host rates
pass through the declared resampler and are tested for level and strum
timing, not bit identity.

This is a bounded portable-source result on macOS ARM. Cortex-M instruction
identity, codec/ADC behaviour, normalization-probe timing and hardware
accuracy are not established.

Native tests cover the host fixture, latency, transpose/chord hysteresis and
pitch limits, all six models sounding and distinct, voice allocation at every
polyphony and odd/even routing, the three normalized strum sources, the
single-output sum, chunk boundaries and adoption, 44.1/96 kHz rates, bypass
routing, initialize versus a fresh module in every model, lamps, invalid
edits, saved patches holding controls only, and allocation-free callback work
at five host rates across model and polyphony changes. Panel geometry at five
scales, model/polyphony-dependent captions, readouts and guide, drag/history
and measured lamps have focused UI tests; MCP tests cover capture, parameters,
undo/redo, initialize, bypass, presets and saved reopen. The example
[`string-field-chord-strings.json`](../examples/string-field-chord-strings.json)
strums two voices of chord strings from a clock while an LFO walks the chords.

## Deviations from Rings

* Fixed 48 kHz kernel behind an original host-rate resampler at other host
  rates; the firmware runs its codec at 48 kHz directly.
* Jack presence replaces the normalization probe; CV/pitch/strum mapping is a
  documented native policy, not the calibrated firmware ADC path, and pots and
  CVs are not low-pass filtered (only the FM CV smoothing and the transpose and
  chord hysteresis are kept).
* A single patched output carries the sum of both outputs (host policy for the
  hardware's output normalling).
* Per-instance random state seeded as the source (`0x21`) instead of the
  firmware's process-global generator, which the normalization probe also
  advances.
* Model and polyphony are panel parameters instead of buttons with saved
  settings; the easter-egg string synthesizer (Disastrous Peace) is not
  included.
* Lamps show output volts and accepted strums; the Rings LEDs are not
  modelled.
