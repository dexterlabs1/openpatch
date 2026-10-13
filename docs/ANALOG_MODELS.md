# Reference analog voice

OpenPatch now includes a specified **idealized reference circuit**, not a measured
replica of a commercial synth. The default desktop patch connects a ramp VCO,
transistor ladder, clock-gated RC ADSR, OTA VCA, and voltage/audio interface.
All six kinds are available through MCP's catalog and patch tools.

## Voltage contract

`analog_vco`, `ladder_vcf`, `analog_vca`, `adsr`, and `gate` exchange volts.
Audio is nominally ±5 V; envelopes/gates use 0–10 V. Pitch and filter CV are
1 V/octave. `audio_out` divides by 10 so ±10 V maps to digital ±1 before master
volume. Unpatched inputs are zero except VCA CV, normalled to 10 V.
These are explicit interface choices consistent with [VCV's voltage conventions](https://vcvrack.com/manual/VoltageStandards).
The seven legacy utilities retain their earlier normalized semantics. Connections
do not perform automatic unit conversion. The catalog describes each family.

## VCO: ideal ramp core

`analog_vco` specifies an ideal exponential current source, a 1 nF capacitor,
and instantaneous reset at a 5 V ramp excursion: `I = C × 5 V × frequency`.
An ideal gain of two and −5 V offset produce the bipolar saw; a comparator
produces the pulse. Frequency is `base × 2^pitch + fm`, with linear FM at 1 Hz/V.
It is bounded to 0–0.4×sample-rate during modulation. Waveform 0 is saw, 1 pulse.

A two-sample polynomial BLEP reduces reset/comparator discontinuity error.
This is a **behavioral idealization**, not a transistor/exponential-converter
simulation. Finite reset time, drift, temperature tracking, sync, and realistic
wave shaping are absent. BLEP is not alias-free; its current test measures
waveform error against a truncated Fourier series, not a complete alias budget.
See [Pekonen and Holters' oscillator research](https://pekonen.cc/papers/dafx12-nonlinear-phase-oscillators/)
for the bandlimited-discontinuity approach.

## VCF: matched-transistor ladder

`ladder_vcf` uses the circuit differential equations developed by
[Huovilainen (DAFx 2004)](https://www.dafx.de/paper-archive/2004/P_061.PDF).
Its transistor assumptions are matched devices, infinite beta, and no Early
effect. OpenPatch specifies four 10 nF capacitors at 298.15 K, an input divider
of 0.01, and ideal output gain of 100. Those interface values are our reference
design choices, not measured Moog or Doepfer values.

Let `VT = kB T/q`, `qi = capacitor_voltage/(2VT)`,
`u = 0.01 × drive × input_volts/(2VT)`, `k = 4 × resonance`, and `ω = 2πfp`:

```
dq1/dt = ω [tanh(u − k q4) − tanh(q1)]
dqi/dt = ω [tanh(q(i−1)) − tanh(qi)]   for i=2,3,4
output_volts = 100 × 2VT × q4
```

The knob is the **individual pole frequency**, not the overall −3 dB point.
At zero resonance, four stages give −12.04 dB at that frequency. Small-signal
DC gain decreases with resonance; there is no artificial bass compensation.
An ideal zero-state circuit remains silent without excitation, even at an
unstable resonance setting. No hidden noise is injected to start oscillation.

We solve the coupled feedback with f64 implicit trapezoidal integration and
bounded Newton iterations. This is not the paper's Euler implementation.
Failure holds the last capacitor state and increments diagnostics; the solver
never retries indefinitely or hides failure behind an output limiter.

## VCA: ideal OTA differential pair

`analog_vca` models a matched differential pair at 25°C, without linearizing
diodes. Its voltage divider is `a=0.01`, maximum nominal bias current is
100 µA at 10 V CV and gain 1, and the ideal transimpedance resistance is
`R=2VT/(a×100µA)`, approximately 51.4 kΩ:

```
output = (2VT/a) × tanh(a × input/(2VT)) × gain × clamp(CV/10, 0, 1)
```

It has unity small-signal gain at nominal bias and compresses larger inputs.
This is a specified ideal OTA circuit, **not an LM13700 chip model**. Real
devices add finite bandwidth, offsets, compliance limits, and other effects;
the [TI LM13700 datasheet](https://www.ti.com/lit/ds/symlink/lm13700.pdf) illustrates
why a static differential-pair equation alone cannot establish chip accuracy.

## ADSR: switched capacitor

`adsr` specifies one 1 µF capacitor with ideal switches and comparators. Attack
charges toward 12 V and switches to decay at 10 V. Decay approaches
`10 × sustain`; release approaches zero. For each stage,
`Vnext = target + (V − target) × exp(−dt/RC)`.

Attack time is the time from zero to 10 V (`RC=A/ln(6)`). Decay and release
times specify 99% settling (`RC=D/ln(100)` or `R/ln(100)`), not time constants.
The threshold transition is quantized to a host sample, with at most one sample
of event timing error for an unmodulated stage. Retrigger starts attack from the
existing voltage while gate is high. Gate release works during any stage.
Gate and retrigger have Schmitt thresholds of 1 V rising and 0.1 V falling.
No leakage, switch resistance, comparator delay, or capacitor absorption is modeled.

## Sampling, continuity, and validation limits

The ladder and VCA each use 8× processing, matched signal/control interpolation,
and FIR decimation. Two 513-tap Blackman-windowed sinc filters add **64 host
samples per module**, including signal and CV interpolation. Their passband is
specified through 0.4×host sample rate, with a transition below Nyquist.
Latency is not automatically compensated across parallel rack paths. A VCF→VCA
path adds 128 samples (2.67 ms at 48 kHz), in addition to device buffering.
Finite oversampling still permits sufficiently high nonlinear harmonics to alias.

Live graph edits carry oscillator phase, capacitor voltage, and FIR history
forward for unchanged module IDs/kinds. Newly inserted or replaced kinds start
fresh. Graph swaps still crossfade over 5 ms; this is an editing policy, not a
physical component property. Processing and state transfer allocate no memory.

Tests cover pitch ratios, RC trajectories and thresholds, VCA DC/control laws,
audio-rate modulation alignment, continuous-state transfer, callback partition
invariance, and allocation behavior. Ladder tests compare small-signal response
with the analytic transfer function and nonlinear behavior with an independently
integrated finer-step RK4 reference. Those comparisons validate the specified
equations and numerics; they do not establish physical-hardware accuracy.

The optimized implementation keeps the same circuit equations, `tanh`, FIR
coefficients, oversampling rate, Newton limits, convergence tolerance and failure
policy. Mirrored histories remove circular addressing inside FIR loops, and
independent interpolation phases are accumulated together while retaining the
addition order of each sample. The ladder reuses already evaluated currents at
accepted solver states. A frozen scalar implementation from `98edb73` checks
exact output/callback bits, solver state and diagnostics, including failed-step
recovery. Bit equality is checked within each build/platform; it does not imply
that different platforms' math libraries produce identical bits. These checks
complement the independent physical-response tests above. See
[performance measurements](PERFORMANCE.md) for results and the memory tradeoff.

Next validation targets are wider spectral alias/IMD sweeps, measured component
tolerances and supply effects, hardware captures, additional VCO waveform outputs,
and parallel-path latency policy. [Module coverage](MODULE_COVERAGE.md) tracks the
larger VCV scope.
