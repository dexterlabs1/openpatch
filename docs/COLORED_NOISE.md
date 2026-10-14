# Seven-color Noise

`colored_noise` is an original Rust implementation of the seven signal roles in
[VCV Noise](https://vcvrack.com/Free#vcv-noise). It has no inputs and seven
simultaneous, mono outputs. The existing `white_noise` module keeps its original
parameters and random sequence.

Open **File → Noise colors** for a connected example: white and pink feed the
Scope, and pink feeds Audio Out at level 0.2. Change cables to audition each color.
The rack and browser use the same panel renderer. Level has a direct slider;
level and seed also support precise entry and shared undo. Panel curves show the
configured digital filter response relative to 1 kHz, not a live measurement.

| Output | Signal contract |
|---|---|
| White | Gaussian white noise; flat power spectral density. |
| Pink | Gaussian source through sixteen interlaced pole/zero sections; approximately −3 dB/octave, with a 1 Hz lowest pole. |
| Red | White through a leaky integrator with a 5 Hz low-frequency plateau; approximately −6 dB/octave above that corner. |
| Violet | First difference of white; approximately +6 dB/octave, curving near Nyquist. |
| Blue | First difference of pink; approximately +3 dB/octave, curving near Nyquist. |
| Gray | Inverse A-weighting approximation: twelve peaking sections, fourth-order high-pass at 60 Hz and fourth-order low-pass at min(22 kHz, 0.46 × sample rate). Qualified response band: 80 Hz to min(20 kHz, 0.4 × sample rate). |
| Black | Independent uniform random voltage within ±5 V at level 1; flat power spectral density, with a different amplitude distribution from white. |

At level 1, each of the six Gaussian colors has a nominal stationary RMS of
5/√2 V. Black has RMS 5/√3 V. `level` multiplies every output, ranges from 0 to
2, and defaults to 1. Gaussian outputs are not peak-limited. The example's lower
level leaves headroom at the audio sink; the module's voltage specification does
not imply every downstream path can accept arbitrarily large peaks.

## State, timing and cost

`seed` is an integer from 0 through 16,777,215, default 1. All streams advance
whether their ports are connected or selected for output. White drives red,
violet and gray; separate Gaussian draws drive pink and blue. Black uses its own
PRNG stream. Consequently some colors are deliberately correlated; they are not
seven independent random sources. Box–Muller transforms the seeded uniform
sequence into Gaussian pairs.

Compatible graph replacement preserves the PRNG, Gaussian spare and filter
histories for the same module identity, sample rate and seed. A new level applies
immediately. A seed change, sample-rate change or runtime reset restarts the
sequence with zero filter history. Parameter Initialize restores seed and level
to their defaults. Patch files save these parameters, not transient noise state.
A given environment reproduces its seeded sequence; bit-identical floating-point
results across architectures are not promised.

Every callback sample performs fixed filter and random-generator work. There is
no callback FFT, coefficient fitting, allocation or lock. Coefficients and
stationary variance normalization are computed on the control thread, using a
bounded sixteen-rate cache. Normalization integrates the actual digital filter
power response on a logarithmic frequency grid. Starting from zero history
requires settling before stationary RMS measurements.

The module requires patch version 2. Every output carries one channel. Version-2
cables retain the engine's one-sample propagation delay. Saved bypass mutes this
source and freezes its random streams and filter histories until resumed.

## Validation and compatibility boundary

The independent waveform harness in `scripts/noise_validation/validate.py` uses
unclipped float32 samples from the production DSP, a two-second warmup, twelve
seconds per seed and three seeds at 44.1, 48 and 96 kHz. It pools seed statistics
and Welch power spectra. Gates fixed before measurement are:

- RMS within 6% of the nominal value, or 12% for red's low-frequency variance.
- Power spectral slopes within 0.5 dB/octave of 0, −3, −6, +6 and +3, fitted
  between 200 Hz and min(5 kHz, 0.15 × sample rate); black is also tested for
  flatness and bounded voltage.
- Gray's octave-smoothed inverse-A shape within 2.5 dB after a single fitted
  level offset, between 100 Hz and min(16 kHz, 0.35 × sample rate).

The [recorded waveform experiment](benchmarks/colored-noise-spectrum.json) passed
all three sample rates. Worst pooled RMS error was 1.75%; the largest slope error
was 0.028 dB/octave. Gray's maximum octave-smoothed shape error was 0.366 dB.
Those are observations for the stated seeds and duration, not tighter universal
guarantees than the declared budgets.

An [offline ARM64 engine measurement](benchmarks/colored-noise-throughput.json)
at 48 kHz, five trials of two seconds each, measured median throughput factors
of 157×, 13.4× and 3.04× real time for 1, 16 and 64 Noise instances. All seven
outputs advance in each instance. This excludes graph compilation, device I/O,
graphics and real-time scheduling; it does not establish callback deadline
reliability or superiority over VCV Rack.

Core tests also cover filter stability, finite corner-rate output, deterministic
seeds, adoption/reset, named ports, bypass, invalid edits and callback heap
behavior. Analytic gray response checks cover 8–192 kHz separately from the
three-rate waveform experiment. These checks establish a declared OpenPatch
contract, not measured equivalence to every VCV sample or a real-world device.

The [pinned VCV implementation](https://github.com/VCVRack/Fundamental/blob/10dd0160c664770910e5584b7b00498cc48d9ddd/src/Noise.cpp)
was a behavioral reference; its GPL implementation and panel artwork were not
ported. OpenPatch's original MIT filters, seeded streams, level control,
connection-independent advancement and soft gray band edges differ from that
reference. There is no claim of identical waveforms, identical edge spectra or
complete VCV Noise calibration. Native macOS input and playback qualification
remain separate from headless tests and Linux screenshot checks.
