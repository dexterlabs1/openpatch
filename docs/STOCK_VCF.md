# Stock VCF

A separate `stock_vcf` targets the pinned VCV Free digital filter. Existing
`ladder_vcf` patches retain their original oversampled circuit model. This is
bounded digital behavior compatibility, not a measured hardware clone.

## Frozen implementation and acceptance contract

Contract established before the first source comparison, 2026-10-15. Reference:
[pinned VCF.cpp](https://github.com/VCVRack/Fundamental/blob/10dd0160c664770910e5584b7b00498cc48d9ddd/src/VCF.cpp),
[VCV Free manual](https://vcvrack.com/Free#VCF). The oracle compiles the actual
upstream module temporarily against the Rack SDK; upstream DSP source is not
bundled in OpenPatch. Rust code independently implements the numerical model.

Version 2, maximum 16 channels. Inputs in reference order: `cutoff_cv`,
`resonance_cv`, `drive_cv`, `signal`. Outputs: `lowpass`, `highpass`.

| Parameter | Stored range | Default | Display |
| --- | --- | --- | --- |
| `frequency` | `(log2(8 / C4) + 5) / 10` to `(log2(22000 / C4) + 5) / 10` | 0.5 | `C4 * 2^(10 * frequency - 5)` Hz |
| `resonance` | 0 to 1 | 0 | 0–100% |
| `drive` | −1 to 1 | 0 | 0–200%, `(drive + 1) * 100` |
| `cutoff_cv_amount` | −1 to 1 | 0 | signed percent |
| `resonance_cv_amount` | −1 to 1 | 0 | signed percent |
| `drive_cv_amount` | −1 to 1 | 0 | signed percent |

C4 = 261.6256f32 Hz, matching the pinned SDK constant. Frequency CV adds volts/octave times signed depth.
Resonance and drive CV add `CV / 10 * depth` before clamping. Resonance feedback
is `10 * resonance^2`; input drive is `(1 + drive)^5`. Audio is divided by 5
before filtering and multiplied by 5 at each output. Cutoff uses the reference
fifth-order exponential approximation and is limited below Nyquist.

The model uses two-times IIR all-pass oversampling, four nonlinear ladder
integrators and independent LP/HP downsamplers. Keep reference operation order
and f32 precision, with no fast-math or fused replacements. Noise excitation is
retained: one scalar uniform draw per processed four-lane group per base frame,
shared by its four lanes, scaled by 1e-6 internally.

Audio input alone sets width, minimum one. Read raw audio without mono
broadcast, advance rounded four-lane groups including padding, and freeze whole
inactive groups without clearing histories. Auxiliary mono CV broadcasts;
short polyphonic CV supplies zero for missing channels. Inactive output storage
is zeroed and never exposed as additional voices.

Both outputs disconnected freezes the entire model, including RNG. A singly
disconnected output freezes only its own downsampler. Consumers include cables,
the selected audio output and temporary capture taps. Newly compiled consumer
masks stay owned by the receiving graph during state adoption.

Same ID/kind/rate adoption preserves ladder, upsampler, both downsamplers and
RNG histories while applying new parameters. New identity or rate starts fresh.
Bypass routes the raw audio input independently to both outputs and freezes
DSP; OpenPatch's existing delayed cable/transition policy still applies.
Explicit Initialize/Reset clears filter history and restarts its deterministic
per-module RNG stream. Rack's RNG is shared globally and its module reset does
not reset that global RNG: this is an intentional host policy difference.
Independent module streams avoid unrelated modules changing noise excitation.
No runtime heap allocation, reallocation or locks are permitted in processing
or adoption. Nonfinite inputs become zero; finite inputs obey host voltage bounds.

## Required validation

The predeclared driven, non-self-oscillating oracle cases compare every connected
output sample from frame zero at 44.1/48/96 kHz, with no alignment, prewarming,
startup discard or reconnect exception:

`abs(actual - reference) <= 0.002 V + 1e-5 * abs(reference)`.

Separately declared self-oscillation cases retain raw traces and compare fixed
windows: startup threshold time within max(2 ms, 1%), fundamental within 0.1%,
RMS within 1%, peak within 2%. Both traces must actually oscillate. The driver
must fix threshold, estimator, windows, durations and several nonzero seed pairs
before its first comparison. Failing driven cases cannot be reclassified.
The genuine SDK uniform generator must remain unchanged; match its seed warm-up
and float conversion and report draw counts and initial/final state.

Cover impulses, DC, drive and signed CV extremes, audio-rate modulation, mono
CV broadcast, short CV, poisoned padding, widths 1/4/5/16 and 16→1→5→16,
LP/HP/both/neither connectivity, disconnected-output tails and parameter
adoption. Public Engine tests separately exercise delayed cables, selected
outputs, temporary taps, bypass, reset and callback partition independence.
Allocation tests cover first processing, taps, adoption and initialization.
Real callback profiling must identify the machine, block deadline and
single-/16-voice and graph-crossfade scenarios; throughput alone is not a
hardware latency qualification.

The rack and browser share one panel renderer with Hz entry, signed depth
controls and clear LP/HP labels. Headless input/geometry tests and actual Linux
normal/lights-out captures qualify that presentation. Native macOS input,
playback/device behavior, full VCV import/plugin compatibility and measured
analog hardware equivalence remain separate outstanding acceptance work.

## Measured callback cost

The [release callback profile](benchmarks/stock-vcf-callback-profile.json) on an
Apple M1 Max, ARM64/macOS 26.7.1, renders a connected VCO → VCF → Sum voice at
48 kHz in 256-frame stereo device buffers (5.333 ms deadline). Observed steady
p99 was 0.128 ms for one voice and 0.688 ms for sixteen voices. The largest
observed sixteen-voice first buffer during graph adoption/crossfade was 1.367 ms;
all measured buffers fit the block deadline. The actual callback and its
uninstrumented path produced identical audio. Separate first-buffer/adoption
tests found no heap activity.

These are headless observations, including the oscillator, summing and host
conversion. They do not establish audio-device latency, hard real-time behavior,
hardware xrun performance, or a speed advantage over VCV Rack.

## Pinned digital comparison

The [actual-class report](benchmarks/stock-vcf-reference.json) passes 54 cases
at 44.1/48/96 kHz: 2,096,640 matrix frames, with zero measured f32 voltage error.
Nine of these cases exercise autonomous self-oscillation across three fixed
seeds. Actual connected SDK output widths and initial/final RNG states also
match. Twenty-one additional fresh-history resonance audits cover all four
CV/depth sign combinations and both zero-depth extreme-CV cases. Each sign
combination measurably changes both outputs; zero-depth extreme CV reproduces
its zero-CV baseline exactly. The original pointwise and oscillation tolerances
were preserved throughout this coverage expansion.

The harness compiles the actual pinned C++ class against a separately installed
Rack SDK, then compares the production Rust module. Reproduce with:

```sh
python3 scripts/stock_vcf_reference/run.py --sdk /path/to/Rack-SDK
```

The recorded oracle is ARM64/macOS with the toolchains and exact source/header/
library hashes listed in its report. Workspace CI on other operating systems
checks OpenPatch behavior; it does not independently repeat this SDK oracle.
Driver adapters omit graph delay and host/UI behavior, which have separate
Engine, callback, MCP, interaction and screenshot checks. The deterministic
module-local RNG and Initialize policy remain intentional host differences.
