# Struck resonator

`modal_resonator` is an original physical model of a simply supported stiff
string / beam. A strike or external force excites its modes; an ideal velocity
pickup converts motion to volts. It is a specified linear, truncated model,
not a measured replica of a commercial module, a recording reconstruction, or
a full acoustic radiation simulation.

Open **File → Struck resonator** for a repeating struck voice with a rack Scope.
Move the strike and pickup positions to change the modes you excite and hear;
increase stiffness to move from string harmonics toward a beam's inharmonic
spectrum. The module's sliders and exact value entry share the normal patch
history and MCP parameter controls. The mode diagram shows configuration at
zero pitch CV; it does not claim to display live motion.

## Physical definition

The reference body is 1 m long with total mass 1 kg, fixed transverse
displacement and zero bending moment at both endpoints. Its spatial modes are
`φ_n(x) = sin(nπx/L)`, with modal mass `m = M/2 = 0.5 kg`.
For modes `n = 1…16`:

```
f_n = f1 sqrt((1 − B)n² + Bn⁴)
ω_n = 2π f_n
σ_n = ln(1000) / decay × (1 + damping × (n − 1))
q̈_n + 2σ_n q̇_n + ω_n² q_n = φ_n(x_strike) F / m
```

Here `q` is displacement in metres and `v=q̇` is velocity in m/s. These
frequencies follow the linear tension-and-bending equation
`μ y_tt + damping − T y_xx + EI y_xxxx = F δ(x−x_strike)`.
The controls imply nonnegative tension and bending rigidity:
`T = μ (2π f1)² (1−B) L²/π²` and
`EI = μ (2π f1)² B L⁴/π⁴`.
The loss law is a chosen positive modal damping model, not a measured material.

`frequency` specifies the first **undamped natural frequency**. A decaying
mode rings at `sqrt(ω_n²−σ_n²)/(2π)`, which can be noticeably lower at low pitch
and short decay. `decay` is the first mode's amplitude-envelope RT60; higher
modes have RT60 `decay/(1+damping(n−1))`.

The force input is held constant over each sample interval, with 1 V = 1 N.
A rising strike applies an instantaneous impulse
`J = 0.01 × strike_strength N·s`, changing each modal velocity by
`J φ_n(x_strike)/m`; this does not scale with sample rate.
The ideal pickup outputs
`25 × level × Σ φ_n(x_pickup) v_n` volts. This is a contact velocity signal,
not pressure at a microphone. A sustained force settles to displacement with
zero velocity; it does not produce a permanent DC pickup voltage.

## Solver and boundaries

Each retained mode uses the exact constant-coefficient state transition over
one sample. For a constant force let `q_eq = φ_n F/(mω_n²)`, `u=q−q_eq`,
`d=sqrt(ω_n²−σ_n²)`, `c=cos(dh)`, `s=sin(dh)/d`, and `r=exp(−σ_n h)`:

```
q_next = q_eq + r [(c + σ_n s)u + s v]
v_next = r [−ω_n² s u + (c − σ_n s)v]
```

This is exact for the retained linear modes with sample-held controls and
force, up to floating-point error. It is not an exact continuous-time solution
for arbitrary modulation between samples. Fixed parameters and no excitation
must dissipate mechanical energy. Changing frequency or stiffness can perform
work on the body; preservation of displacement and velocity does not promise
energy conservation during modulation.

Modes above 0.45 × sample rate are omitted and their state is cleared.
Pitch CV clamps the first natural frequency to 20 Hz…0.45 × sample rate.
There is no claim of alias-free abrupt impulses, force steps, parameter changes,
or arbitrary audio-rate pitch modulation. A 16-mode truncation omits higher
spatial detail; it does not approximate the frequencies of retained modes.
Runtime displacement and velocity are not saved in patch JSON.

## Interface

Version 2 only; all three inputs contribute to voice count, with a minimum of
one and maximum of sixteen. Mono inputs broadcast; missing short-poly lanes
supply zero. Each cable contributes the engine's normal one-sample delay.

| Control | Default | Range / meaning |
|---|---:|---|
| Natural frequency | 220 Hz | 20…min(4,000 Hz, 0.45 × sample rate) before pitch CV |
| Stiffness | 0.2 | 0–1; string to beam |
| Decay | 2 s | 0.1–30 s; first-mode RT60 |
| Damping | 0.3 | 0–1; additional upper-mode loss |
| Strike position | 0.23 | 0.01–0.99 of body length |
| Pickup position | 0.61 | 0.01–0.99 of body length |
| Strike strength | 1 | 0–2; 0.01 N·s per unit |
| Level | 1 | 0–2; pickup output gain |

Inputs are `pitch` (1 V/oct), `strike` (Schmitt trigger), and `excite` (force).
Output is `out` (volts). Strike rises at 1 V, rearms at 0.1 V, and an initial
high voltage triggers once. Bypass routes raw `excite` volts to `out` and freezes
mechanical state. Matching live graph replacements retain displacement and
velocity, applying new parameters immediately; changing the strike connection
resets its edge detector. Initialize and sample-rate changes reset the model;
retired voices restart from rest. During active processing, inputs and output are bounded to ±1,000,000 V;
nonfinite input becomes zero, invalid/extreme internal state resets, and states
below 1e−30 in their respective SI units are zeroed. Those safety rules
sit outside the ideal linear mechanical model. The machine-readable catalog
also exposes these contracts. MCP can
construct and edit this module with existing patch tools and observe its output
with `capture_signal`, `analyze_signal`, or a live rack Scope. This adds a new
built-in model, not live generation of arbitrary DSP code.

## Why modal synthesis first

Analytic modes make this body's boundary conditions, frequencies, excitation,
and pickup observable and testable with fixed storage. Waveguides are a strong
option for simple travelling-wave strings and tubes; finite-difference grids
allow richer geometry and distributed contact at greater numerical cost.
Those are separate model choices, not requirements to disguise inside this
module. [Smith's modal expansion chapter](https://www.dsprelated.com/freebooks/pasp/Modal_Expansion.html)
explains the physical-mode interpretation;
[Bilbao's numerical sound synthesis overview](https://www2.ph.ed.ac.uk/~sbilbao/nsstop.html)
provides the broader spatial-model context.

The implementation is original Rust. No third-party module source, artwork or
recorded sound is included. Hardware calibration, nonlinear hammer contact,
coupled bodies, radiation, and live motion visualization remain future work.

## Validation and performance

Analytical tests compare impulse and held-force responses at 8–192 kHz,
measure free decay and ringing frequency, check mechanical energy loss,
spatial reciprocity, omitted modes, polyphonic retirement, live state adoption,
initialize and bypass. A tracked allocator checks processing and adoption,
including audio-rate pitch changes. Public MCP tests exercise construction,
measurement, reciprocal strike/pickup edits, undo, saving and offline WAV export.

The [independent coefficient check](../scripts/modal_resonator/coefficient_check.py)
compiles the actual production coefficient implementation and compares 45
corners against a 65-digit oracle. Its [recorded result](benchmarks/modal-coefficients.json)
has maximum relative error below 1.8e−13 (acceptance bound 1e−11). This checks
numerical coefficient evaluation, not hardware fidelity.

The [offline workload benchmark](../scripts/modal_resonator/measure.py) measures
one second of 48 kHz engine processing in 128-frame blocks, with five trials.
The [recorded ARM64 result](benchmarks/modal-resonator.json) includes source hashes
and the environment. Compilation and the first block are excluded.

| Voices | Pitch | Median processing time for 1 s |
|---:|---|---:|
| 1 | fixed 0 V | 5.86 ms |
| 1 | 80 Hz sine, +/-0.1 V, mono broadcasts | 21.60 ms |
| 16 | fixed 0 V | 45.91 ms |
| 16 | 80 Hz sine, +/-0.1 V, mono broadcasts | 272.75 ms |

These sequential offline timings exclude device scheduling, rendering, graph
replacement, concurrent clients and callback tail latency. They are not a
realtime guarantee, worst-case independent polyphonic modulation test, or a
performance comparison with VCV Rack.
