# ADSR EG: implementation and acceptance contract

Status: implemented; reference and host validation described below. The numerical
gate and fixtures were fixed before the first production comparison.

`adsr_eg` is a new version-2 module. Existing `adsr` patches retain their original
RC envelope behavior and parameter units. This is a functional digital envelope,
not a measurement-calibrated model of a particular hardware circuit.

The independent oracle is the actual ADSR DSP class in
[Fundamental commit 10dd016](https://raw.githubusercontent.com/VCVRack/Fundamental/10dd0160c664770910e5584b7b00498cc48d9ddd/src/ADSR.cpp),
compiled temporarily against the Rack SDK. Upstream source and artwork are not
vendored into OpenPatch. Production Rust and panel design are original.

## Interface

Inputs, in order: `attack_cv`, `decay_cv`, `sustain_cv`, `release_cv`, `gate`,
`retrigger`. Output: `out`, labeled ENV, in volts.

Parameters `attack`, `decay`, `sustain`, `release` range from zero to one and
default to 0.5. `attack_amount`, `decay_amount`, `sustain_amount`,
`release_amount` range from −1 to +1 and default to zero. A/D/R displayed values
are time constants from 1 to 10,000 ms, using `10000^value` ms. The midpoint is
100 ms. Attack targets 1.2 normalized units, so this is not time to peak.
Sustain and CV amounts display percentages. PUSH is a runtime held control using
`push_press` / `push_release`; it is not a saved parameter.

## Sample behavior

- Output width is the gate input width, with a minimum of one and maximum of 16.
  Gate reads raw channels; a mono gate does not broadcast. CV and retrigger mono
  inputs broadcast; absent and missing short-poly channels read zero.
- Processing uses rounded groups of four lanes. Extra lanes in the last group
  affect aggregate lights but are never exposed as output channels. Entirely
  inactive groups freeze all histories, cached coefficients and trigger state.
- A/D/S/R modulation adds `CV / 10 * amount`, then clamps to [0,1]. Coefficients
  and sustain update every 16 samples, first on sample 16. Cached coefficients
  and sustain initially equal zero. No hidden warm-up or initial coefficient
  calculation is allowed. A/D/R rate is `10000^(-value) / 0.001`.
- Gate is high at >=1 V without hysteresis. PUSH forces all processed lanes high
  without expanding the output width. A gate rise starts attack. Retrigger uses
  a detector initially high: it must encounter <=0 V before a >=1 V rise. A
  retrigger while gate is low does not start an envelope.
- A forward-Euler f32 update moves toward 1.2 during attack, sustain during gated
  decay, and zero during release. Attack ends after the updated envelope reaches
  one. ENV is ten times the envelope. Preserve the slight attack overshoot;
  do not clamp to 10 V or snap to sustain/zero.
- Stage lamps update every 128 samples, first on sample 128. A/D/S/R and any-gate
  lamps are aggregate Boolean values over all processed lanes, including padding.
  Sustain is `sustain <= envelope < sustain + 0.01`; rest is `envelope < 0.01`.
  PUSH illumination means any gate high, not just a physically held button.

## OpenPatch lifecycle policies

Nonfinite input becomes zero; finite inputs follow the host voltage bounds.
Same-ID/kind/rate graph adoption retains envelope, gates, attacks, trigger state,
cached coefficients, lamp values and divider phases, even when connections or
parameters change. New parameters apply on the next coefficient tick. Pending
actions remain with the receiving engine and are not copied from the old graph.
Sample-rate changes start fresh.

Manual actions are applied in queue order before the sample; final held state
wins. A same-sample press then release is not a synthetic trigger. The desktop
holds a fast UI click until the next distinct UI frame, separately from DSP
semantics. This is not an intervening audio-sample guarantee: if both actions
reach one callback sample, final state still wins. The release ticket retains
its original backend, instance token and action epoch; obsolete tickets cannot
release a replacement instance or a newly initialized press. A valid full-queue
release uses the existing emergency path.
Emergency release clears held and pending controls without clearing envelopes.
Bypass freezes DSP and publishes mono zero; bypass transitions release manual
controls. Initialize and reset clear all envelope, detector, coefficient, divider
and lamp histories and release manual controls. These host lifecycle policies
are separately tested and are not assertions about Rack's lifecycle callbacks.

## Predeclared verification gates

At 44.1, 48 and 96 kHz, compare the actual production module against the temporary
upstream class, sample for sample, without aligning traces or discarding startup.
For every exposed ENV sample:

`absolute error <= 0.002 V + 0.00001 * absolute(reference volts)`.

Output channel counts, lamp values and cadence must agree exactly. There is no
one-sample stage-boundary exemption. Phase-aligned error may be a diagnostic
only. A failure requires correction or a clearly qualified approximation claim;
the gate must not be widened after seeing the result.

Fixtures cover cold start, all coefficient-divider phases, all lamp-divider
phases, shortest/longest times, sustain endpoints, signed CV depths, thresholds
and intermediate trigger levels, held PUSH, rapid/repeated/released gates,
retrigger during attack/decay/release, mono broadcast versus raw gates, and width
shrink/regrowth across four-lane boundaries. Parameter changes exercise cached
coefficient adoption. Deterministic mixed vectors exercise interacting controls.
Reports record source/SDK/toolchain/driver/production hashes, compiler/FMA flags,
case input hashes and the worst-error sample in every case.

Separate host tests cover action validation and queue bounds, emergency release,
adoption, bypass/init/reset, block partition invariance, unchanged old `adsr`
behavior, finite outputs and no callback allocations. Shared live/browser panel
rendering, precise time entry, real stage lamps, held PUSH and fast-click release
need desktop checks. macOS window/input/device-audio verification remains a
separate qualification from offline tests and Linux screenshot checks.

## Reproduce and use

Run `python3 scripts/adsr_eg_reference/run.py --sdk /path/to/Rack-SDK` with Cargo,
rustc, a C++17 compiler and the separately installed Rack SDK available. Source
download and compilation occur in a temporary directory; no Rack application is
launched. The [comparison report](benchmarks/adsr-eg-reference.json) records all
51 cases, 1,966,080 frames, toolchain fingerprints and limitations. It passes the
predeclared same-sample gate at all three rates, with exact channels and lamps;
the worst observed error is 0.0000028611 V. This is bounded numerical agreement,
not bit-exactness or qualification of every possible patch/platform.

The shared rack/browser panel has eight sliders, exact millisecond/percentage
entry with expressions and Tab navigation, six inputs, ENV, PUSH and five real
lamps. Its nominal diagram is a printed guide; route ENV to Scope for measured
per-voice trajectories. The reference animated position-dot display is not yet
implemented. Darkroom preserves lamp and Scope emission.

Open **File → Polyphonic ADSR voice** for a four-voice chord sequence with
per-voice attack modulation, velocity-modulated sustain and velocity-shaped VCA
gain. Its source is [polyphonic-adsr-voice.json](../examples/polyphonic-adsr-voice.json).
Offline render checks establish finite, nonzero audio below full scale; no
speaker playback or native macOS device qualification is implied.
