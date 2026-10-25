# Grain Weather

`grain_weather` is a native Rust adaptation of the Mutable Instruments Clouds
(2014) granular processor, original firmware only (no Parasites or other
alternate firmware). It has its own OpenPatch identity, shared rack/browser
panel and document/MCP controls. It is not a binary Rack plugin, and it is not
a calibrated model of the Clouds codec, CV front end or hardware.

A stereo live input is recorded continuously into a preallocated buffer unless
FREEZE is on or the freeze gate is high. MODE selects one of the four source
playback modes, each reading that buffer:

* **Granular**: up to 32 overlapping grains (40 mono, 46/57 with the 8-bit
  buffer). POSITION picks where in the buffer grains start, SIZE their length,
  PITCH their transposition. DENSITY above the centre seeds grains at random
  from the source RNG, below it at a regular rate, and the centre band is
  silent; TEXTURE moves the grain window from square to smooth and then adds
  diffusion. SPREAD pans grains at random. TRIG starts one grain.
* **Stretch**: the WSOLA pitch shifter / time stretcher. Windows are aligned by
  the source's 1-bit correlation search; SIZE sets the window, DENSITY the
  diffusion, TEXTURE a low-pass to high-pass filter. TRIG restarts the
  position envelope.
* **Looping delay**: POSITION is the delay time, PITCH the post pitch shifter;
  TRIG taps the delay time. When frozen the buffer loops, SIZE sets the loop
  length and PITCH its playback rate.
* **Spectral**: the STFT phase vocoder (4096-point ShyFFT, hop 1024). SIZE
  warps the frequency axis, DENSITY sets how fast stored spectra refresh and
  randomizes phases, TEXTURE quantizes magnitudes, POSITION selects a stored
  spectrum. FREEZE holds the spectrum; a high TRIG input is the glitch gate.

BLEND is the source's equal-power dry/wet balance, FEEDBACK returns the output
(high-passed and soft-limited) into the recording, REVERB is the source
reverb. QUALITY selects the buffer format exactly as the source does: 16-bit
stereo (1.0 s), 16-bit mono (1.9 s), 8-bit mu-law stereo (4.1 s) or 8-bit
mu-law mono (7.4 s); the 8-bit settings also run the granular engine at 16 kHz
behind the source's 2x converters. Changing QUALITY, or changing MODE to or
from spectral, clears the buffer and releases freeze, as in the source.

Captions follow the mode (stretch: WINDOW, DIFFUSION, FILTER; looping delay:
DELAY; spectral: WARP, REFRESH, QUANTIZE). Readouts use percent, signed
semitones, or for granular density "Random", "Even" or "No grains". The guide
line states the configured buffer and what TRIG does. Output lamps show actual
delayed output volts; the freeze lamp shows the kernel's actual freeze state.
[Panel review](../design/reviews/grain-weather-v1/README.md).

## Native mapping and timing

Inputs read channel zero; an unpatched right input follows the left. Audio
uses a codec scale of ±8 V in (16-bit full scale, clipped beyond it) and ±16 V
out: the source output stage halves its signal before its soft limiter, so the
doubled scale undoes the halving, and the equal-power blend passes the fully
dry signal at −3 dB. Mapping, informed by the firmware's `cv_scaler.cc`, is an
explicit OpenPatch policy:

* Position, size, density, texture and blend: knob plus voltage/5, clamped to
  0..1 (blend drives the source dry/wet directly).
* Pitch: knob semitones (±24) plus 12 per volt, limited to ±48 as in the
  source.
* Spread, feedback and reverb: knob only.
* TRIG: a rising edge above 1 V after falling below 0.1 V sets the source
  trigger for the next kernel block; the input level is the spectral glitch
  gate. FREEZE: the switch, or a high freeze gate with the same thresholds.

Nonfinite volts read zero. The firmware's ADC smoothing, calibration, quantized
pitch-knob curve, blend-knob parameter selection and two-block trigger latency
are not modelled.

The kernel always runs at its native 32 kHz in 32-frame blocks of 16-bit
codec frames: its tables, filter tunings, reverb LFOs and grain sizes assume
that rate. After every block the host runs the firmware main-loop task
(`Prepare`) eight times; the firmware runs it continuously, and eight always
completes the largest WSOLA correlation search and any pending phase-vocoder
frame within the block that queued it. At other host rates, windowed-sinc
resamplers (Blackman-Harris, 32 zero crossings, cutoff 0.45 of the lower rate,
exact integer timing; shared with Cross Matter) bridge to and from 32 kHz. The
measured dry-path response at 44.1/48 kHz is flat within 0.1 dB from 100 Hz to
13 kHz. Latency is constant: 35 frames at 32 kHz and 159 frames (3.3 ms) at
48 kHz. Controls are sampled once per kernel block.

The recording buffer, effect workspaces and phase-vocoder state live in one
preallocated image of the firmware's two static memory blocks (118,784 and
65,408 bytes), addressed exactly as the source allocator lays them out. Mode
and quality changes re-lay out that image in place; the callback performs no
allocation, including mode/quality changes, bridge, initialize, bypass and
adoption paths. Same-rate graph adoption copies the complete runtime, buffer
included. Initialize and rate changes start a freshly initialized kernel with
an empty buffer. Bypass routes LEFT and RIGHT unchanged and freezes
processing. Saved patches and presets contain authored controls only, never
buffer audio.

## Source and qualification

Source: `pichenettes/eurorack` at `f4cfa7697095a28d292bff758beb51deb49b5a86`,
with its stmlib gitlink `e3bd7c9cc00e4364166f9905c0509b6ffd0535ec`. Complete
MIT notices are retained in the adapted files, the Core third-party notice and
the bundled `Clouds-algorithms.txt`. The [reference runner](../scripts/clouds_reference/README.md)
records 42 source hashes plus one variant file, compiler options and immutable
input/output traces. Tables are extracted bit-for-bit by
`kernel/generate_tables.py`; the FFT is the source's ShyFFT, ported with its
butterfly order, workspace use and rotation recurrences (its nine `cosf`/`sinf`
seeds are stored as the binary32 values the source computes).

Reference `attempt-001` compiled the unmodified source (C++11 `-O2 -DTEST
-ffp-contract=off`) and ran 39 cases, 4,061,440 frames at 32 kHz, with one
`Process` and eight (or, in two cases, one) `Prepare` calls per block.
Comparison `rust-comparison-001` ran the production Rust kernel on the same
inputs: **8,122,880 of 8,122,880 int16 left/right samples match bit for
bit**, with zero guarded reads and no excluded samples. The cases cover every
mode at every quality, random and regular grain seeding, triggers, maximum
overlap, the shortest grains, pitch extremes, position/size/density/texture/
pitch sweeps, freeze with the write crossfade, feedback and reverb including
the freeze reverb boost, clipped inputs and dry/wet ramps, WSOLA window-size
changes and partial correlation searches, tap-tempo and frozen loops, the
spectral freeze with phase advance, gated glitches (algorithm chosen by the
source RNG), random-bin
refresh, quantization and warp, and live mode/quality switching (including
the phase vocoder inheriting memory earlier modes wrote, and the pitch shifter
clearing the correlator words it shares).

**The pinned revision's stretch mode is silent.** Two 2023 static-analysis
commits (`fbb53ba`, `0e3756f`, merged in `d1d8839`) each removed one of the
two duplicated `done_ = false` lines from `Window::Start`, so no WSOLA window
ever starts. Grain Weather keeps the line, as every earlier release has it.
The runner therefore builds a second oracle from the same pinned tree with
`clouds/dsp/window.h` taken unmodified from eurorack `2a46873` (hash-pinned).
Both oracles produce identical output for the 30 cases that never enter the
stretch mode (5,786,880 samples, compared against the pinned oracle); the 9
cases that do (2,336,000 samples) are compared against the `2a46873` window
and match exactly. Against the pinned oracle those 9 cases differ in 1,338,849
samples, because there the stretch output is silence.

Granular mode and the spectral mode draw from the source's global LCG. Each
case seeds it before `Init` (`0x21`, the firmware's static initial value, or
123456789 in one case) and the Rust probe seeds its per-instance generator
identically. With no other consumer in the oracle this covers every draw: one
per sample for random seeding (drawn whether or not used), grain panning,
random-bin refresh, phase randomization and glitch selection/patterns. The
firmware's generator state at any moment depends on how long it has run, so
hardware random sequences are not claimed.

`Correlator::EvaluateNextCandidate` shifts right by 32 at word-aligned offsets,
undefined in C++. Cortex-M yields zero, and disassembly of the oracle shows
clang's arm64 code computing `(x >> 1) >> (31 - offset)`, also zero, in both
its vector and scalar loops; the Rust kernel implements zero. Accesses that
would leave an object are guarded and counted (none occurred).

A native fixture freezes two reference cases whose controls are exactly what
the host produces for fixed knob settings: at 32 kHz the actual module output
matches the C++ codec words bit for bit after its 35-frame latency. Other host
rates pass through the declared resampler and are tested for latency,
flatness, gain and behaviour, not bit identity.

This is a bounded portable-source result on macOS ARM. Cortex-M instruction
identity, codec/ADC behaviour, the firmware's main-loop timing and hardware
accuracy are not established.

Native tests cover the host fixture, dry gain and latency at 32/44.1/48/96 kHz,
bridge flatness, all four modes sounding and distinct, buffer length and grain
budget per quality, trigger-only grains, the freeze gate in three modes and
the freeze switch, chunk boundaries and adoption, bypass routing, initialize versus a
fresh module in every mode, invalid edits, saved patches holding controls
only, and allocation-free callback work at six host rates across mode and
quality changes. Panel geometry at five scales, mode/quality-dependent
captions, readouts and guide, drag/history and measured lamps have focused UI
tests; MCP tests cover capture, parameters, undo/redo, initialize, bypass,
presets and saved reopen. The example
[`grain-weather-rainfall.json`](../examples/grain-weather-rainfall.json)
scatters random grains from two voices with a drifting position and clocked
triggers.

## Deviations from Clouds

* `Window::Start` clears `done`, restoring the stretch mode that the pinned
  revision silences (see above).
* Fixed 32 kHz kernel behind an original host-rate resampler; the firmware
  runs its codec at 32 kHz directly. `Prepare` runs eight times per block
  instead of continuously.
* Separate SPREAD, FEEDBACK and REVERB knobs and a BLEND CV that drives dry/wet
  only, instead of the firmware's single blend knob with a selectable target.
  CV/pitch/trigger mapping is a documented native policy, not the calibrated
  firmware ADC path; no quantized pitch-knob curve or trigger latency.
* Per-instance random state seeded as the source (`0x21`) instead of the
  firmware's process-global generator.
* No sample save/load to flash and no bypass/silence UI states; saved patches
  hold controls only.
* Lamps show output volts and freeze state; the Clouds VU/position LEDs are
  not modelled.
