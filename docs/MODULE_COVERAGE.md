# Rust module coverage

Inventory verified **2026-10-12** against the official [VCV Core manual](https://vcvrack.com/manual/Core), [Core library listing](https://library.vcvrack.com/Core), and [VCV Free module manuals](https://vcvrack.com/Free).

The first bounded compatibility target is the **12 Core modules plus 37 VCV Free modules** listed below. Core is built into Rack; Free is its included synthesis collection. This is an inventory and implementation plan, not a claim that OpenPatch implements 49 equivalents. The wider third-party plugin ecosystem is open-ended and is not covered by this inventory. Additional plugins need individually specified behavior, implementation scope, and validation.

OpenPatch uses original Rust DSP. Functional coverage, fidelity to a specified circuit, and compatibility with VCV behavior are separate acceptance criteria. It does not currently load VCV plugin binaries or import complete VCV patches. A matching module name, panel, or musical role does not establish equivalence.

## Current implementation baseline

The seven existing kinds remain legacy utilities with their original patch semantics: `oscillator` (sine/additive-Hz FM), `lfo` (sine/offset), `constant`, `gain` (direct CV multiplication), `mixer` (two-input sum), `lowpass` (one pole), and `output` (selected mono signal). They are not accurate hardware models or complete VCV equivalents. Native playback and a master-signal visualizer exist; neither supplies the complete Audio or Scope feature set.

The selected first target is a **generic VCO → VCF → VCA voice with ADSR**, using explicitly specified reference circuits and voltage interfaces. The ramp VCO, OTA VCA, RC ADSR, gate/audio interfaces, and nonlinear four-pole ladder are implemented with numerical and integration tests. The ladder has bounded numerical solving, oversampling, and independent ODE-reference comparisons. These remain reference models rather than complete VCV equivalents or calibrated hardware clones; see [model validation](ANALOG_MODELS.md).

## Implemented mono utility contracts

Five additional original models fit the current two-input/one-output engine. No VCV or third-party plugin source code was copied; these Rust algorithms were authored against the contracts below. All five are labeled `functional` in the machine-readable catalog, with port names, parameter defaults, bounds, and discrete parameter steps. They extend the usable toolkit; they do not establish full VCV module parity.

| Kind | Implemented behavior | Explicit gaps against related VCV modules |
|---|---|---|
| `quantizer` | Nearest enabled 12-tone note at 1 V/oct; integer `scale_mask` bits C through B; `offset` and `offset_cv` add volts before quantization. Exact ties choose the higher pitch, including below zero. Empty mask passes the adjusted pitch through. | Mono only; no piano display or factory-scale preset browser. The tie and empty-scale policies are OpenPatch contracts, not claimed VCV equivalence. |
| `sample_hold` | One held voltage; captures the same-sample signal when trigger rises to 1 V, rearms at 0.1 V or below; starts at `initial`. Ideal hold without droop. | No eight-row shift register, normalization chain, clear/random controls, or saved runtime voltage in patch JSON. |
| `white_noise` | Single Gaussian white-noise output with `level` in RMS volts and an integer 24-bit `seed`. Reproducible sequence within an execution environment; no peak clipping. | No colored-noise outputs, VCV-specific level calibration, or identical VCV random sequence. |
| `attenuverter` | One channel computes `gain * signal + offset`; gain −1…1, offset −10…10 V; unpatched signal is 10 V. | No eight-channel bank or input propagation; no Rescale range limiter or slew behavior. |
| `crossfade` | Two inputs, one output; `mix` selects a at 0 and b at 1. Linear gains or square-root gains for constant power with uncorrelated inputs. | No CV control input, complementary output, stereo routing, or automatic signal-correlation compensation. |

Nonfinite utility inputs map to zero. Arithmetic safety bounds are ±1,000,000 V and can override quantizer membership at that extreme. Quantization and sample-and-hold deliberately introduce discontinuities without antialias filtering. These models add no processing delay; cable propagation retains OpenPatch's immediate DAG evaluation rather than Rack's one-sample-per-cable timing.

Live graph replacement preserves held voltage and trigger hysteresis by matching node ID and kind. It also preserves noise PRNG state and the pending Gaussian sample when the seed is unchanged, including after a level edit; changing the seed explicitly restarts the sequence. Loading version-1 patch JSON restores parameters and starts from `initial`/`seed`, rather than serializing running DSP state. Stateless utility parameter edits take effect immediately.

Validation includes an independent nearest-note search over all 4,095 nonempty masks at signed octave boundaries, empty-scale and tie tests, exact trigger/hold event sequences, crossfade endpoint and center gains, noise distribution/RMS and lag correlations, JSON parameter round trips, invalid-parameter rejection, graph-state adoption, callback-size/order independence, and allocation tracking on the first callback and graph replacement. These are contract tests; direct VCV differential testing and full spectral/parity acceptance remain future work. Functional roles were checked against the official [Free manuals](https://vcvrack.com/Free) and [voltage/trigger standards](https://vcvrack.com/manual/VoltageStandards).

## Core inventory

Names follow the Core library; parenthesized aliases match the manual. The Audio family contains three distinct modules. [Official Core inventory](https://library.vcvrack.com/Core)

| Module | Group | Rust status | Next acceptance test or prerequisite |
|---|---|---|---|
| Audio 2 | Device I/O | Partial: mono playback only; voltage adapter implemented | Stereo input/output, device selection, voltage/full-scale conversion, reconnect |
| Audio 8 | Device I/O | Planned | Eight-channel routing and device round trip |
| Audio 16 | Device I/O | Planned | Sixteen-channel routing and channel isolation |
| MIDI to CV (MIDI-CV) | MIDI input | Planned | Note/gate/velocity events, voice allocation, disconnect recovery |
| MIDI CC to CV (MIDI-CC) | MIDI input | Planned | Controller mapping, endpoints, event timing |
| MIDI to Gate (MIDI-Gate) | MIDI input | Planned | Note mapping, short notes, held gates |
| MIDI Map (MIDI-Map) | Parameter control | Planned | Learn/persist mappings; concurrent GUI/MCP edits |
| CV to MIDI (CV-MIDI) | MIDI output | Planned | Pitch conversion, balanced note-on/off, panic |
| CV to MIDI CC (CV-CC) | MIDI output | Planned | Quantization, rate control, endpoint round trip |
| Gate to MIDI (CV-Gate) | MIDI output | Planned | Threshold crossings and stuck-note prevention |
| Blank | Layout | Planned | Resize, placement, save/load |
| Notes | Documentation | Planned | Unicode text and patch persistence |

## VCV Free inventory

All names below are from the [official Free manual index](https://vcvrack.com/Free). “Related legacy” means a narrower utility exists, not a passing equivalence test. The proposed tests are OpenPatch work items, not reported test results.

| Module | Group | Rust status | Next acceptance test or prerequisite |
|---|---|---|---|
| VCO | Oscillator | Partial: reference ramp VCO implemented; related legacy sine | Voltage pitch tracking, waveform spectra/alias error; then modulation and sync matrix |
| Wavetable VCO | Oscillator | Planned | Validated file loading, interpolation, tuning and alias budget |
| VCF | Filter | Partial: reference ladder implemented; legacy one-pole is separate | Analytic small-signal response, drive/IMD, resonance and numerical convergence; inventory missing ports/features |
| VCA | Amplifier | Partial: reference OTA VCA implemented; related legacy gain | Voltage gain law, saturation and CV response; distinguish circuit model from VCV behavior |
| LFO | Modulation | Related legacy sine only | Waveforms, voltage ranges, clock/reset behavior |
| Wavetable LFO | Modulation | Planned | Table traversal, reset and low-frequency timing |
| Delay | Effect | Planned | Fractional delay, interpolation, feedback stability, modulation; allocate buffers off callback |
| ADSR EG | Envelope | Partial: reference RC ADSR implemented | Capacitor trajectories, timing convention, sustain, release and retrigger from every stage |
| Mix | Mixing | Related legacy two-input sum | Channel count, gain, normaling and summation policies |
| VCA Mix | Mixing | Planned | Per-channel/master CV, independent outputs, headroom |
| 8vert | Voltage utility | Partial: single `attenuverter` with 10 V normalization and offset | Eight channels and downward input propagation |
| Mutes | Routing | Planned | Independent mute states, normaling and persistence |
| Pulses | Gate utility | Clock-gate interface implemented; not equivalent | Pulse duration and held-gate behavior across sample rates |
| Scope | Measurement | Master visualizer only | Arbitrary-node probing, voltage calibration, triggering and timing |
| SEQ 3 | Sequencing | Planned | Step/reset ordering, clock boundaries and saved sequence |
| Sequential Switch 1 to 4 | Routing | Planned | Selected-path routing and clock/reset transitions |
| Sequential Switch 4 to 1 | Routing | Planned | Input selection and clock/reset transitions |
| Octave | Pitch utility | Planned | Exact pitch-voltage increments and boundaries |
| Quantizer | Pitch utility | Partial: mono `quantizer`, 12-note mask and pre-offset | Polyphony, scale UI/presets, direct VCV boundary comparison |
| Split | Polyphony | Planned; engine prerequisite | Channel extraction and unused-channel policy |
| Merge | Polyphony | Planned; engine prerequisite | Channel ordering/count and unpatched inputs |
| Sum | Polyphony | Planned; engine prerequisite | Channel reduction, gain and empty input |
| Viz | Measurement | Planned; engine prerequisite | Per-channel voltage display calibration |
| Mid/Side | Stereo | Planned; named-output prerequisite | Encode/decode identity and gain normalization |
| Noise | Source | Partial: seeded Gaussian `white_noise` with RMS control | Colored outputs, spectral-density budget and VCV level calibration |
| Random | Modulation | Planned | Trigger timing, distribution and repeatability |
| CV Mix | Mixing | Planned | Offset/gain combination and voltage endpoints |
| Fade | Mixing | Partial: mono `crossfade`, linear and square-root laws | CV modulation and complementary output |
| Logic | Gate processing | Planned | Truth tables and threshold hysteresis |
| Compare | Voltage processing | Planned | Boundary comparisons and simultaneous events |
| Gates | Gate processing | Planned | Timing, retrigger and overlapping-event cases |
| Process | Voltage processing | Planned | Transfer curves, endpoints and operation order |
| Mult | Routing | Cable fan-out exists; dedicated module planned | Output identity and channel propagation |
| Rescale | Voltage processing | Partial: affine `attenuverter` only | Configurable range limiting and remaining transfer behavior |
| Random Values | Source | Planned | Sampling events, statistics and repeatability |
| Push | Gate utility | Clock-gate interface implemented; not equivalent | Press/release/latch state and persistence |
| Sample & Hold Analog Shift Register | Sampling | Partial: single `sample_hold`, hysteresis and live-state continuity | Eight rows, shift ordering, clear/random modes and runtime JSON persistence |

## Delivery order and acceptance rules

1. **Validate the reference voice.** Publish circuit equations, component values, voltage scaling, assumptions, and unsupported physical effects. Test the ramp source, ladder, OTA VCA, and RC envelope independently and together. Compare circuit solvers against an independent high-accuracy reference; compare physical hardware only when a specific measured target exists. Preserve old patches and continuous oscillator/capacitor state during edits.
2. **Extend engine contracts before broad module coverage.** Add named multiple outputs, more input ports, explicit voltage conventions, polyphonic channel rules, timing semantics, and delay/feedback handling. The current mono DAG with one output per node is not a general Rack engine. Keep audio callbacks bounded and allocation-free; document oversampling latency and parallel-path alignment.
3. **Implement the remaining inventory by functional group.** Utilities and measurement first, then modulation/sequencing, expanded oscillators/effects, polyphony, and hardware/MIDI interfaces. Promote each row only with a documented port/parameter contract, persistence tests, DSP/event tests, and native integration checks. Record partial coverage explicitly rather than counting a generic substitute as complete.

For voltage-oriented modules, use the [VCV voltage conventions](https://vcvrack.com/manual/VoltageStandards) as an interoperability reference: pitch in V/oct, explicit audio/CV levels, hysteretic gate detection, and documented conversion at device boundaries. Legacy normalized signals require explicit conversion. VCV also specifies cable timing and polyphonic behavior; adopting voltage units alone does not reproduce those semantics.

Completion of this inventory would establish a bounded set of Rust functional equivalents only after the specified tests pass. It would not establish compatibility with every third-party module, identical VCV DSP, or measured fidelity to unspecified analog hardware.
