# State-variable filter

`svf_filter` is an original functional two-pole filter with low, band and high
outputs. Its source implementation uses a host-rate topology-preserving
state-variable recurrence, preceded by 16× oversampled memoryless drive.
Ladder and OTA VCA processing are separate and unchanged. This is not a
hardware model or a claim of VCV/Surge equivalence.

| Control | Range | Behavior |
| --- | --- | --- |
| Cutoff | 5–20,000 Hz | Base cutoff; effective cutoff is limited to .45 times the host rate |
| Resonance | 0–1 | Damping `k = 2 - 1.95 × resonance` |
| Drive | 0–1 | Gain `1 + 8 × drive`, with a smooth blend below .1 |
| Cutoff CV amount | -4–4 oct/V | Exponential cutoff modulation from the delayed CV |

The signal port determines one to sixteen active voices. Each voice has its
own filter state, FIR histories and cutoff-CV queue. Mono CV broadcasts;
missing short-poly CV channels and disconnected CV use zero. Nonfinite signal
and CV samples become zero; signal input, filter states and output voltages
are bounded. The host-rate filter equations and output definitions are retained.

For drive `d`, define `g = 1 + 8d`,
`curve(x) = 5 × tanh(xg/5) / tanh(g)` and
`b = smoothstep(clamp(d/.1, 0, 1))`. The drive evaluates
`x + b × (curve(x) - x)` sixteen times in chronological order per host sample.
At zero drive the blend is linear; at and above .1 it selects the previous
static saturation curve. Values below .1 therefore change the earlier
host-rate filter's sound and remove its discontinuity at zero. Gain, blend and
the normalization denominator are calculated during graph construction and
shared across voices; division in the curve is retained.

Four 2× FIR stages have lengths 129/33/17/33. The host-adjacent Blackman sinc
cuts off at .45 times the host rate; higher stages are halfbands. Interpolation
and phase-zero decimation together add **76 host samples** of signal latency,
including at zero drive. Cutoff CV receives the same integer delay. This is
about 1.72/1.58/.79 ms at 44.1/48/96 kHz, separate from graph cables and device
buffers. Wanted frequencies in the FIR transition band are attenuated even
when drive is zero.

Parallel dry/filtered paths are not delay compensated. Two cascaded SVFs add
152 samples, and feedback paths include the module delay in addition to one
sample per version-2 cable. The added phase changes can change cancellation,
feedback tone and transient timing in existing patches. Bypass routes the
signal directly to low, with band/high at mono zero; FIR, CV and filter state
freeze. Bypass therefore changes path latency, and resuming processing can
release retained history. The host's existing output/cable clearing and graph
crossfade policies remain in effect.

All sixteen voice lanes are allocated during graph construction. Processing,
voice retirement/regrowth, reset and compatible state adoption use fixed
storage. Retired and newly activated lanes start with cleared filter/FIR/CV
histories. Same-rate compatible adoption carries those histories and queue
positions forward, while applying new parameter values; different rates or
module kinds start fresh. Parameter edits retain audio/CV history and apply
the current drive, cutoff and resonance parameters to subsequent processing.

The [public multistage study](benchmarks/svf-antialias/multistage/README.md)
records 144 Engine, 198 highband and 234 dense single-tone cases at
44.1/48/96 kHz. These 576 prototype cases pass the declared alias and retained
magnitude gates against an independent continuous-source reference. The worst
driven off-harmonic residual is -67.21 dB relative to fixed 5 V peak; linear
magnitude is gated through 18 kHz. The measurements are bounded by the stated
stimuli, amplitudes and settings. They do not establish arbitrary-signal,
two-tone, rapid-modulation or hardware fidelity, and transition-band loss must
not be counted as unrestricted alias improvement.

Integration tests compare all three output bit patterns and retained state with
the unhoisted implementation across 240 count/rate/drive configurations. A separate
implicit trapezoidal solver checks modulated cutoff outputs and distinguishes
76-sample CV timing from 75/77-sample alternatives. Tested drive/cutoff/resonance
changes preserve histories, remain bounded, and recover to silence. These bounds
are numerical checks, not a promise of click-free arbitrary parameter jumps.

Public-engine tests instrument allocations and deallocations through first
processing, state adoption, reset, bypass and 16→1→0→16 voice changes. Additional
checks establish the declared delay through actual cables, direct bypass, and
low-gain feedback continuity across block partitions and reordered graph rebuilds.

The [integrated-source timing screen](benchmarks/svf-integration-m1max.json)
measures the built implementation separately from the prototype. At 16 voices
and maximum drive, 128-frame p99 was .817/.820 ms at 48/96 kHz on one M1 Max.
The largest 96 kHz block was 1.692 ms, exceeding its 1.333 ms nominal budget.
The initial prototype p99 failure and subsequent repeats remain documented;
none establishes audio-device stability or a performance advantage over Rack.
See [performance evidence](PERFORMANCE.md#svf-nonlinear-drive-integration-and-prototype-screen).
