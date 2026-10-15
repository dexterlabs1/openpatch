# MACRO VOICE

`macro_voice` is a native, functional OpenPatch instrument with eight synthesis
engines and up to sixteen independent voices. It is not a calibrated analog
circuit or a sample-identical Mutable Instruments Plaits port. FM feedback and
additive spectral shaping adapt Emilie Gillet's MIT-licensed Plaits algorithms;
the remaining engines and host integration are original. The full notice ships
in the core and application license inventories.

## Controls and signals

| Control | Range | Default |
|---|---|---|
| Engine | Analog, Wavefold, FM, Harmonic, Wavetable, Chord, Swarm, Metallic | Analog |
| Frequency | 8–16,000 Hz saved value; DSP frequency capped below | 261.62558 Hz |
| Harmonics | 0–1 | 0.3 |
| Timbre | 0–1 | 0.45 |
| Morph | 0–1 | 0.4 |
| Decay | 0–1 | 0.5 |
| Level | 0–1 | 0.8 |

Inputs, in order, are `pitch`, `trigger`, `harmonics`, `timbre`, `morph`, `fm`,
and `level`. Outputs are `out` and `aux`; AUX is an engine-specific alternate
signal, not a second copy of OUT. Both output widths match the active voice count.

Pitch and FM add in volts at 1 V/octave relative to Frequency. The combined
value is limited to ±16 V and the resulting frequency to 1 Hz–0.4 × sample rate.
Harmonics, Timbre and Morph add their respective CV divided by 5 to the panel
value, clamp to 0–1, and smooth with a 4 ms time constant. These controls have
different musical roles in each engine; they are not universal physical units.

An unpatched TRIG, including a connected zero-channel signal, gives continuous
sound. Once its delayed cable frame is acquired, a patched mono 0 V signal
instead selects triggered operation; without a strike its envelope stays closed.
The initial zero-channel cable frame can briefly use continuous normalization,
and existing DC history can decay after the change. A rising input at ≥1 V strikes a decaying envelope;
≤0.1 V rearms the detector. Holding a trigger high does not sustain or repeatedly
strike the envelope. Its exponential time constant is
`0.035 + 5 * decay²` seconds. The envelope also opens an internal one-pole
low-pass gate. This is a functional LPG, not a component-calibrated vactrol.

LEVEL normalizes to 10 V when unpatched or zero-channel; otherwise it clamps to
0–10 V and multiplies the panel Level. OUT and AUX are nominally ±5 V times this
gain, followed by a 5 Hz DC blocker and a final ±10 V safety bound. Nominal level
is not a strict ±5 V peak guarantee. After muting an already sounding voice,
the DC blocker can leave a short tail rather than immediate exact zero.

## Polyphony, timing and edits

The maximum width of PITCH and TRIG sets the voice count, with a minimum of one
and maximum of sixteen. Other CVs do not create voices. Mono inputs broadcast
to all voices; missing lanes of a short polyphonic input read zero. In
particular, a two-channel LEVEL source mutes lanes three onward rather than
giving them its unpatched 10 V normalization. Each voice has independent phase,
envelope, filters, spectra and oversampling history. Retired voices reset before
reactivation.

Wavefold and FM use eight-times oversampling and a 64-tap FIR. Other engines
have different internal histories; parallel engine paths are not delay-matched.
Version-2 cables each add one sample of propagation, as described in the
[engine contract](ENGINE_V2.md). Host callback partitioning must not change the
sample sequence.

Compatible live state adoption requires the same module identity, engine and
sample rate. It preserves synthesis histories across unrelated graph edits and
ordinary control edits. Changing engine or sample rate starts fresh synthesis
histories. Changing only the trigger route rearms only the trigger detector;
it preserves phase, FIR, LPG, DC blocker and other histories. A fresh module
incarnation resets runtime state. Saving a patch stores controls and routing,
not a serialized oscillator history.

## Validation and limits

Core tests cover all eight engines at 8, 44.1, 48, 96 and 192 kHz for finite,
bounded, audible and distinct output, trigger decay, eventual LEVEL muting,
voice retirement, invalid controls, and the signed sine-table boundary.
Six additional public Engine tests compare callback partitions, compatible
state adoption, engine/rate/identity/incarnation reset, trigger-route rearming,
zero-channel normalization and independent polyphonic lane references. They
check both OUT and AUX. The callback allocation guard includes all eight engines
with sixteen voices. Explicit Initialize/reset APIs and live per-lane CV-edit
adoption are separate coverage; these tests do not qualify those operations.
These tests establish OpenPatch behavior; they do not establish Plaits
equivalence, commercial hardware fidelity, or complete spectral/alias rejection
qualification.

For a reproducible offline profile:

```sh
cargo run --locked --release -p openpatch-core --example macro_voice_profile
```

The profile measures each engine separately, at one and sixteen voices with
64/256-frame blocks at 48 kHz. It emits the actual patches, raw timings, channel
checks and finite-output checks as JSON. Compilation, table creation, warmup
and output inspection are outside timed calls. A separate source-only baseline
is reported without subtraction. The selected high-drive settings are not an
exhaustive worst case. With 64 timed blocks, the reported p99 equals the maximum
under this estimator; these short observations are not stable tail bounds.
See [performance measurements](PERFORMANCE.md) for recorded results. Offline
graph timing does not qualify audio-device deadlines, scheduling or dropouts,
and does not demonstrate a speedup over VCV Rack.
