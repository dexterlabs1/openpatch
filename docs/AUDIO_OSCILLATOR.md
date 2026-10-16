# Four-output audio oscillator

This page documents the older `multi_vco` contract and its measured limits.
The separate [Stock VCO](STOCK_VCO.md) targets the newer pinned processor with
step/slope correction and within-frame sync. Existing patches keep this older
model's behavior.

`multi_vco` is an original Rust functional oscillator for version-2 patches.
Its behavior target is the VCO in **Fundamental 2.6.4**, pinned to
[`e0ade9a`](https://github.com/VCVRack/Fundamental/blob/e0ade9a23a9b41124ea3f4ec9aa2cb0b98c2529e/src/VCO.cpp).
It is not a physical circuit model or a completed VCV-equivalence claim.
The existing `analog_vco` keeps its separate ramp/comparator model, output,
FM scaling, frequency ceiling, and saved-patch behavior.

## Play it

Choose **File → Synced waveform voice**. Three pitch channels drive a master
and a slave oscillator. The master's sine synchronizes the slave; a slow LFO
modulates its FM and pulse width. Four slave waveforms feed VCA Mix, then
Stereo Audio. The bundled patch is
[synced-waveform-voice.json](../examples/synced-waveform-voice.json).

The **MULTI VCO** panel has frequency, FM depth, pulse width, and PWM depth
knobs. Double-click a value to type Hz or percent; double-click a knob to
restore its default. **LFM** selects linear FM and **SOFT** selects soft sync.
Both modes are saved and participate in undo/redo and atomic MCP edits.
All four waveform jacks are available simultaneously.

## Signal contract

Inputs: `pitch`, `fm`, `sync`, `pwm`.
Outputs: `sine`, `triangle`, `saw`, `square`. There is no `out` alias.

| Parameter | Default | Range / unit |
|---|---:|---|
| `frequency` | 261.62558 | 11.562326–5919.911 Hz; ±4.5 octaves around C4 |
| `fm_amount` | 0 | −1…1 |
| `pulse_width` | 0.5 | 0.01…0.99 |
| `pwm_amount` | 0 | −1…1 |
| `linear_fm` | 0 | Integer 0 or 1 |
| `soft_sync` | 0 | Integer 0 or 1 |

Pitch uses 1 V/oct. Exponential FM adds `fm_amount × FM` to pitch in octaves.
Linear FM adds `261.62558 × fm_amount × FM` Hz to the pitched frequency;
negative frequencies stop the oscillator. Phase advance is capped at 0.35
cycles per sample. Nonfinite incoming voltages become zero. Width is
`clamp(pulse_width + pwm_amount × PWM / 10, 0.01, 0.99)`.

The sine has a quadratic curve and triangle/saw have rational curves;
these intentionally differ from ideal sine and straight-line ramps. Nominal
waveform scale is 5 V peak. Reconstruction can overshoot that value, and the
curved saw has a nonzero mean. Square uses a 20 Hz bilinear high-pass and 0.95
level compensation, so its plateau and amplitude depend on duty and pitch.

Square is a latched pulse: wrap starts a traversal, and passing the current
width threshold ends it. Moving PWM back across the phase does not restart
that pulse before the next wrap. This matters for audio-rate modulation.

Pulse and wrap events are processed chronologically, including two transitions
inside one sample. Fundamental 2.6.4 can lose an edge for very narrow pulses
when it processes wrap first. OpenPatch deliberately corrects this defect;
its pulse output is not sample-identical to that reference in these cases.

Phase advances before each output sample. Sync interpolates the zero crossing within the frame and accepts it when
its fraction is in `(0, 1]` and the current sample is nonnegative. Besides
negative-to-nonnegative crossings, the pinned reference also triggers on a
positive signal falling exactly to zero; OpenPatch preserves this boundary.
An initially positive signal and a zero-to-positive transition do not trigger.
Soft sync changes direction for the next interval. Hard sync keeps the
remaining fractional interval after reset; periodic edge processing precedes
sync processing. An unpatched, zero-channel sync input holds detector history.

Pitch sets 1–16 voices, minimum one. Auxiliary mono inputs broadcast;
unpatched inputs and missing lanes in a short polyphonic auxiliary input
supply zero. Every voice has its own phase, pulse latch, sync detector,
reconstruction tail, and square filter.

## Live editing and real-time behavior

Matching node ID, kind, and sample rate preserve running state during graph
replacement. Changed incoming sync routing clears only the sync detector.
Retired voices reset before reactivation. This differs from Rack's retained
SIMD-lane histories on voice shrink/regrowth and remains a compatibility gap.
Bypass clears every output and freezes the oscillator state until restored.
Saved patches contain controls and wiring, not running waveform history.

Coefficient generation and allocation occur during preparation on the control
thread. The callback uses fixed voice and correction storage. Tests cover the
first callback, graph replacement, and allocator/deallocator activity.
There is no explicit output delay line; reconstruction tails affect phase and
frequency response. Version-2 cables independently add one sample of transport.

## Independent validation

The comparison reference executes the actual pinned Fundamental VCO with the
[official Rack 2.6.4 SDK](https://vcvrack.com/downloads/Rack-SDK-2.6.4-mac-arm64.zip)
in a headless harness, with no UI or audio-device initialization. Reference
source and its GPL notices are kept outside this MIT repository. No reference
source or generated reference table is compiled into OpenPatch.

`scripts/compare_multi_vco.py` sends the same sparse control events to that
reference and the Rust test exporter, then measures simultaneous jack voltages.
It covers 44.1/48/96 kHz, tuning, hard/soft sync, audio-rate PWM, narrow pulses,
linear-FM stop, mono auxiliary broadcast, short-poly padding, mode edits,
sync connection changes, and voice retirement. Residuals include frequency
drift and reconstruction response; they are not an alias-only measurement.

To build the exporter, run `cargo test --locked -p openpatch-core --lib --no-run`
and pass the resulting test executable to the comparison script:

```sh
python scripts/compare_multi_vco.py \
  --rust-test-bin target/debug/deps/openpatch_core-<hash> \
  --reference-bin /path/to/external/oracle \
  --output native-test-output/vco-reference-comparison
```

The script requires NumPy. The external oracle accepts sparse CSV controls and
emits little-endian float32 `[frame][16 voices][4 waveforms]`; the same layout
is exported by the ignored Rust test `multi_vco::tests::export_reference_case`.
Outputs are sine, triangle, saw, square, with inactive voices zero-filled.

Reconstruction uses an independently authored minimum-phase step correction
based on the general method in [Brandt, *Hard Sync Without Aliasing* (2001)](https://www.cs.cmu.edu/~eli/papers/icmc01-hardsync.pdf).
It corrects value jumps; curved-wave derivative discontinuities and extreme
modulation still need spectral qualification. No zero-alias or calibrated
hardware claim follows from the functional tests.

The independent alias diagnostic uses coherent FFT tones and measures energy
outside DC and all legitimate in-band harmonic bins. It can underestimate
aliases that land on those bins. Run it with the same Rust test executable:

```sh
python scripts/analyze_multi_vco_alias.py \
  --rust-test-bin target/debug/deps/openpatch_core-<hash> \
  --output native-test-output/vco-alias-analysis
```

This is separate from the direct VCV residual. A better alias result cannot
by itself prove waveform, sync, or circuit fidelity.

## Measured limits of this implementation

The [direct reference results](benchmarks/multi-vco-reference-m1max.json) record
21 cases and hashes of the executables, input cases, and raw traces. Selected
RMS residuals in volts, after discarding the first 512 frames:

| Case | Sine | Triangle | Saw | Square |
|---|---:|---:|---:|---:|
| C4, 48 kHz | 0.0011 | 0.0012 | 0.177 | 0.237 |
| 1 kHz tone | 0.0095 | 0.0095 | 0.340 | 0.462 |
| 5 kHz tone | 0.0398 | 0.0398 | 0.994 | 0.536 |
| Hard sync | 0.0054 | 0.0964 | 0.182 | 0.238 |
| Audio-rate PWM | 0.0011 | 0.0012 | 0.177 | 0.827 |

These are material differences, not passing tolerances. Reconstruction uses
an original f64 radix-2 FFT, a 512-point four-term Blackman–Harris sinc over
closed ±16 host samples, an unpadded causal cepstrum with log floor −30, and a
16× interpolated step table. Its response differs from Rack's float32/FFT
backend. OpenPatch's frequency calculation and phase accumulation also differ;
small tuning differences accumulate into measurable trace residuals.

The audio-rate PWM residual additionally includes OpenPatch's corrected
boundary-event reconstruction. Extreme narrow-pulse differences are intentional:
for a 4798.828125 Hz square at 99% duty, the coherent off-harmonic diagnostic
improved from approximately −10.1 dB with wrap-first pulse handling to −71.1 dB
with chronological handling, relative to 5 V peak. The pinned reference also
shows the missed-edge artifact. This is a specific correction, not a general
claim of better audio quality than VCV Rack.

The [30-case static spectral sweep](benchmarks/multi-vco-alias-m1max.json)
includes 1%, 10%, 50%, 90%, and 99% duty across six frequencies. At approximately
4.8 kHz, the measured sine/triangle/saw off-harmonic levels remain −52.2/−33.9/
−42.4 dB. Curved-wave derivative correction, stronger high-frequency rejection,
dynamic spectral testing, voice-history compatibility, and broader native
qualification remain open. The VCO inventory row remains **partial**.

## Offline throughput

On the development Mac, three one-second release runs had median realtime
factors of **260×** for one VCO voice, **34.1×** for sixteen voices, and **53.2×**
for the three-voice synced example. Every oscillator computes all four outputs.
The [raw benchmark](benchmarks/multi-vco-throughput-m1max.json) includes patches,
individual runs, and the existing circuit workloads for context. This measures
offline DSP throughput, not device latency, callback deadlines, or performance
relative to VCV Rack. Sixteen full circuit voices still fell below realtime.

Reproduce with `python scripts/benchmark_engines.py --oscillators` after building
the release CLI.
