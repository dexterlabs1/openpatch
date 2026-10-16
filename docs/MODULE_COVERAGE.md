# Rust module coverage

Inventory verified **2026-10-12** against the official [VCV Core manual](https://vcvrack.com/manual/Core), [Core library listing](https://library.vcvrack.com/Core), and [VCV Free module manuals](https://vcvrack.com/Free).

The first bounded compatibility target is the **12 Core modules plus 37 VCV Free modules** listed below. Core is built into Rack; Free is its included synthesis collection. This is an inventory and implementation plan, not a claim that OpenPatch implements 49 equivalents. The wider third-party plugin ecosystem is open-ended and is not covered by this inventory. Additional plugins need individually specified behavior, implementation scope, and validation.

OpenPatch uses Rust DSP, including explicitly attributed licensed algorithm adaptations. Functional coverage, fidelity to a specified circuit, and compatibility with VCV behavior are separate acceptance criteria. It does not currently load VCV plugin binaries or import complete VCV patches. A matching module name, panel, or musical role does not establish equivalence.

## Current implementation baseline

Six additional native reconstruction tools—SCORE, DRUM, SVF, stereo delay, reverb and sidechain compression—extend the catalog; they are original algorithms, not new claims of completed Rack plugin equivalence. See [score and studio contract](SCORE_AND_STUDIO.md).

VIZ adds a dedicated input-only 16-channel polarity display, bringing the total
to 52 kinds; [signal and validation contract](POLYPHONIC_VIZ.md). Rack Scope adds
independent polyphonic X/Y acquisition and passthrough, bringing the catalog to
53 kinds; [Scope contract and bounded reference](RACK_SCOPE.md).
The original [physical modal resonator](MODAL_RESONATOR.md) brings the catalog to
54 kinds. It is an additional acoustic model, not completion of a stock VCV target.
[Seven-color Noise](COLORED_NOISE.md) brings the catalog to 55 kinds,
with original filters and explicitly bounded spectral qualification.
[Random Values](RANDOM_VALUES.md) adds seven polyphonic held-voltage outputs and
coherent saved state, bringing the catalog to 56 kinds.
[Sample & Hold / Analog Shift Register](SHIFT_REGISTER.md) adds eight mono holds,
normalized clock and old-value signal chains, bringing the catalog to 57 kinds.
[ADSR EG](ADSR_EG.md) adds a separate stock-style polyphonic envelope, bringing
the catalog to 58 kinds while preserving the original RC envelope.
[MIDI CC](MIDI_CC.md) adds sixteen controller outputs, ordinary 7-bit input,
14-bit pairs, smoothing, MPE and durable Learn, bringing the catalog to
59 kinds. Its original Rust DSP passed 45 comparisons of the actual pinned Rack
class (15 fixtures × 44.1/48/96 kHz), with zero measured voltage error and exact
metadata in the [recorded run](benchmarks/midi-cc-reference.json). This is bounded
DSP evidence; native window/input/audio-device and physical MIDI qualification
remain separate gates.
[MIDI → Gate](MIDI_GATE.md) adds sixteen note-gate outputs, velocity, MPE,
minimum-duration pulses, Learn and Panic, bringing the current catalog to
60 kinds at that milestone. Its source contract deliberately preserves output-cell state across
remapping; clearing a remapped held gate requires its new note-off or Panic.
The [actual-class report](benchmarks/midi-gate-reference.json) records 63 passing
cases (21 fixtures × 44.1/48/96 kHz), 258,048 frames and 56,513,088 active lane
samples, with bit-identical binary traces, zero voltage error and no metadata
mismatches. This covers the pinned DSP, SDK queue and source JSON with the
explicit driver channel rule; all outputs were probed. Native host, hardware,
durable transport/Session behavior and complete Core parity have separate gates.

[Notes and Blank](ANNOTATIONS.md) add two zero-port annotation kinds, bringing the
catalog to 62 at that milestone. Notes stores bounded Unicode text; Blank stores a single
HP width with a 3 HP minimum. Both share revision-checked UI/MCP transactions,
presets, duplication, Initialize and history. GUI width/position resizing is one
undo step, and browser thumbnails use the actual panels. These functional
contracts do not qualify native text/IME/clipboard interactions or complete Rack
layout parity; direct graph-only MCP/preset width edits cannot enforce collisions
against the separately owned host layout.

[Stock VCF](STOCK_VCF.md) adds a separate two-times IIR digital ladder with
low-pass and high-pass outputs and all six controls, bringing the
catalog to 63 at that milestone. The existing circuit ladder retains its sound
and patch schema.
The [actual-class report](benchmarks/stock-vcf-reference.json) passes 54 cases
(2,096,640 frames) and 21 fresh-history resonance audits, with zero measured
f32 voltage error. Consumer/reconnect, lifecycle and UI gates are tracked
separately; this does not complete all filter or host compatibility.

[Stock VCA](STOCK_VCA.md) adds a separate ideal polyphonic amplifier with level,
linear/fourth-power CV response and per-channel effective gain telemetry,
bringing the catalog to **64 kinds at that milestone**. The reference OTA VCA and legacy
`gain` keep their existing contracts. Its [actual-class report](benchmarks/stock-vca-reference.json)
passes 72 traces at 44.1/48/96 kHz: 55,872 frames, 800,328 active output/gain
comparisons and maximum absolute error 9.54e-7 under the frozen
`2e-6 + 2e-6 * abs(reference)` gate. Six DSP unit tests, five public Engine tests
and one core allocation/deallocation guard pass. Dedicated desktop callback and
panel tests, reviewed Linux normal/dark captures, and macOS ARM/Windows/Linux CI
also have [recorded bounded passes](STOCK_VCA.md#recorded-host-and-platform-validation).
Native macOS/Windows window input, physical MIDI/audio I/O and complete Rack
host/import parity remain open.

[MACRO VOICE](MACRO_VOICE.md) adds eight synthesis engines in one native kind,
bringing the catalog to **65 kinds at that milestone**. Its sixteen independent voices,
OUT/AUX outputs and trigger-driven low-pass gate are an original OpenPatch
instrument. FM feedback and additive shaping adapt MIT-licensed Plaits
algorithms; this is neither a sample-identical Plaits port nor an additional
completed stock VCV target.

[Stock Delay](STOCK_DELAY.md) adds a separate clock-synchronized mono delay,
MIX/WET outputs, feedback/tone controls and four signed CV depths, bringing the
catalog to **66 kinds at that milestone**. The existing original stereo delay keeps its
own contract. The mono sinc adaptation credits BSD-2-Clause libsamplerate
0.1.9. The original 24 HP rack/browser panel uses actual CLK telemetry.
Live Delay runtime handoff avoids copying its 8 MiB history; borrowed adoption
still copies it, and this is not a universal constant-time graph-adoption claim.
The final [guarded report](benchmarks/stock-delay-reference.json) passes all
43 source-defined whole-Delay cases over 11,134,750 frames at 44.1/48/96 kHz,
with zero worst absolute error and stable production hashes. Six defined SRC
scenarios also match counts/samples exactly. The independent source-undefined
startup case passes native safety checks and receives no fidelity pass.
The release workspace build, 79 Python/MCP tests and 1,580 Rust tests pass
(six existing Rust tests ignored), as does strict Clippy. The
[adoption microprofile](benchmarks/stock-delay-handoff.json) records one/eight
Delay ownership transfers; it is not a full callback-deadline or Rack CPU
comparison. The macOS ARM bundle's ad-hoc signature/notices are verified,
without installing or launching it. [Three-platform CI and all 97 native
captures passed](STOCK_DELAY.md). Native input/device and full callback-deadline
qualification remain open; this bounded DSP result does not complete host parity.

[Wavetable VCO and LFO](WAVETABLES.md) brought the catalog to **68 kinds** at that milestone.
Their original Rust DSP, immutable prepared banks, guarded workspace files,
portable presets and local host lifecycle/heap tests cover bounded digital
behavior. The checked actual-class SDK run passes 153 cases and 626,688 frames
at 44.1/48/96 kHz, with maximum voltage error 1.9073486328125e-6 V under the
unchanged 2 mV + 10 ppm gate; LFO voltages are exact in that run. Audio geometry
is explicitly limited to powers of two of at least 32 points. Zero/negative VCO
frequency is a safe native extension tested separately from the source's
undefined path. The [production-hash proof](benchmarks/wavetable-reference.json)
and local Mac ARM release build pass. Three-platform CI and 100 native captures
have [recorded bounded passes](WAVETABLES.md#checked-numerical-scope-and-remaining-gates); native interaction/device and full host parity are
separate acceptance gates.

[Random modulation](RANDOM.md) brings the current catalog to **69 kinds**.
This separate instrument adds probability, spread, internal/external clocking,
shared-phase interpolation and external polyphonic sample targets. Its checked
actual-class comparison passes 75 cases / 484,548 frames at three sample rates
with zero measured voltage/state error using identical controlled random draws.
Native seeded randomness is separately specified; uncontrolled Rack PRNG sequence
identity is not claimed. Retained trace/source hashes, local host/heap/MCP tests and a separate Mac ARM
release build pass; platform CI, rendered capture review and native device
qualification remain explicit gates.

The prior 45-kind baseline comprises: seven legacy utilities, six reference-voice
kinds, five functional utilities, four version-2 polyphonic routing/source
utilities, MIDI → CV, stereo audio, four stereo/pitch/routing processors, a four-output clocked LFO, a three-row eight-step sequencer, two sequential switches, three voltage/mute banks, four gate/voltage processors, two CV-controlled audio mixers, a four-output audio oscillator, a ten-row manual trigger/gate source, a manual/CV gate source with a persistent HOLD latch, and a six-output gate/trigger processor. This count is not a count of completed VCV equivalents.

The seven existing kinds remain legacy utilities with their original patch semantics: `oscillator` (sine/additive-Hz FM), `lfo` (sine/offset), `constant`, `gain` (direct CV multiplication), `mixer` (two-input sum), `lowpass` (one pole), and `output` (selected mono signal). They are not accurate hardware models or complete VCV equivalents. Native playback, a master-signal visualizer, and a calibrated two-source signal scope exist; these do not yet supply the complete Audio or Scope feature set.

The selected first target is a **generic VCO → VCF → VCA voice with ADSR**, using explicitly specified reference circuits and voltage interfaces. The ramp VCO, OTA VCA, RC ADSR, gate/audio interfaces, and nonlinear four-pole ladder are implemented with numerical and integration tests. The ladder has bounded numerical solving, oversampling, and independent ODE-reference comparisons. These remain reference models rather than complete VCV equivalents or calibrated hardware clones; see [model validation](ANALOG_MODELS.md).

Version 1 retains mono, same-sample DAG evaluation. Version 2 implements named
source outputs, 0–16 channels, explicit voice policies and one sample of delay
per cable, including feedback cycles. Independent engine contract tests cover routing, state migration and MIDI; polyphonic hardware I/O and full module parity remain separate work.
See [ENGINE_V2.md](ENGINE_V2.md) for exact routing, normalization and state rules.

## Implemented functional utility contracts

Five additional original models use at most two inputs and one output. No VCV or third-party plugin source code was copied; these Rust algorithms were authored against the contracts below. All five are labeled `functional` in the machine-readable catalog, with port names, parameter defaults, bounds, and discrete parameter steps. They retain mono version-1 behavior; version 2 adds the declared voice policies below. They extend the usable toolkit; they do not establish full VCV module parity.

| Kind | Implemented behavior | Explicit gaps against related VCV modules |
|---|---|---|
| `quantizer` | Nearest enabled 12-tone note at 1 V/oct; integer `scale_mask` bits C through B; `offset` and `offset_cv` add volts before quantization. Exact ties choose the higher pitch, including below zero. Empty mask passes the adjusted pitch through. In v2, pitch sets voice count and offset CV follows auxiliary broadcast rules. | Native piano-key scale editing is implemented; no factory-scale preset browser. The tie and empty-scale policies are OpenPatch contracts, not claimed VCV equivalence. |
| `sample_hold` | One held voltage per voice; captures the simultaneously received signal when trigger rises to 1 V, rearms at 0.1 V or below; starts at `initial`. Ideal hold without droop. In v2, trigger sets voice count; mono signal broadcasts. | No eight-row shift register, normalization chain, clear/random controls, or saved runtime voltage in patch JSON. |
| `white_noise` | Single Gaussian white-noise output with `level` in RMS volts and an integer 24-bit `seed`. Reproducible sequence within an execution environment; no peak clipping. | No colored-noise outputs, VCV-specific level calibration, or identical VCV random sequence. |
| `attenuverter` | Computes `gain * signal + offset`; gain −1…1, offset −10…10 V; unpatched signal is 10 V. In v2, signal sets voice count. | No eight-section control bank or input propagation; no Rescale range limiter or slew behavior. |
| `crossfade` | Two inputs, one output; `mix` selects a at 0 and b at 1. Linear gains or square-root gains for constant power with uncorrelated inputs. In v2, maximum input count sets voices with mono broadcast. | No CV control input, complementary output, dedicated stereo interface, or automatic signal-correlation compensation. |

Nonfinite utility inputs map to zero. Arithmetic safety bounds are ±1,000,000 V and can override quantizer membership at that extreme. Quantization and sample-and-hold deliberately introduce discontinuities without antialias filtering. These models add no internal processing delay; cable propagation is immediate in version 1 and one sample per cable in version 2. Zero-channel inputs use unpatched normalization; missing channels of a short polyphonic auxiliary input use zero instead.

Compatible live graph replacement preserves held voltage and trigger hysteresis by matching node ID and kind. It also preserves noise PRNG state and the pending Gaussian sample when the seed is unchanged, including after a level edit; changing the seed explicitly restarts the sequence. Patch JSON stores parameters, not running DSP state: fresh compilation starts from `initial`/`seed`, while the host can separately adopt compatible live state. In version 2, retired voices reset before reactivation. Stateless utility parameter edits take effect immediately.

Validation includes an independent nearest-note search over all 4,095 nonempty masks at signed octave boundaries, empty-scale and tie tests, exact trigger/hold event sequences, crossfade endpoint and center gains, noise distribution/RMS and lag correlations, JSON parameter round trips, invalid-parameter rejection, graph-state adoption, callback-size/order independence, and allocation tracking on the first callback and graph replacement. These are contract tests; direct VCV differential testing and full spectral/parity acceptance remain future work. Functional roles were checked against the official [Free manuals](https://vcvrack.com/Free) and [voltage/trigger standards](https://vcvrack.com/manual/VoltageStandards).

## Implemented version-2 routing/source utilities

| Kind | Implemented behavior | Remaining scope |
|---|---|---|
| `poly_constant` | 0–16 configured constant-voltage channels, `v1`…`v16`, with explicit zero-channel output; default count 3. | Original source/testing utility; not a claimed stock VCV equivalent. |
| `poly_split` | `signal` to 16 independently named mono outputs `out1`…`out16`; missing input channels emit mono 0 V. | Native channel-count indicators and direct VCV behavior/UI comparison remain open. |
| `poly_merge` | `in1`…`in16` to `out`; uses channel 0 of each input, keeps holes zero. `channels=0` automatically uses the highest nonzero-channel input; 1–16 forces count. Empty automatic input gives zero channels. | The zero setting selects automatic mode, not forced zero; native channel indicators and full VCV configuration parity remain open. |
| `poly_sum` | Active-channel sum with 0–1 level, mono output, six post-level meter bands and actual input-channel count; empty input gives mono zero. | Native device qualification and Rack import remain open; [SUM contract](POLYPHONIC_SUM.md). |

Independent tests exercise sparse Merge/Split routing, all 16 channels, zero
channels, explicit source-port selection, mono-adapter versus summed output,
feedback timing, retirement and edge-history continuity. These establish the
specified functional subsets. They do not mark the related VCV inventory rows
complete or qualify multichannel audio hardware.

## Stereo, mixing and pitch utilities

`mid_side`, `mix6`, `octave`, and `mult` implement their documented signal roles
with explicit OpenPatch policies for unspecified boundaries. See
[ROUTING_MODULES.md](ROUTING_MODULES.md) for equations, voltage scaling, native
controls, voice counts, safety behavior, and the stereo study. `midi_cv` and
`audio_stereo` contracts are in [MIDI_AUDIO.md](MIDI_AUDIO.md). None of these
additions imply complete stock-module or host parity.

## Clocked modulation

`multi_lfo` adds four phase-coherent CV outputs, exponential FM, PWM, independent
clock/reset detectors, voltage-range switching, inversion, and up to 16 voices.
See [MODULATION.md](MODULATION.md) for its exact timing and state contracts,
native controls, example voice, validation and remaining parity work. The legacy
`lfo` is unchanged. This is a functional digital oscillator, not a circuit model.

## Audio-rate modulation

`multi_vco` adds four audio outputs, exponential/linear FM, a latched PWM
square, and fractional hard/soft sync. Its behavior is compared with an actual
Fundamental 2.6.4 reference; reconstruction and voice-retirement differences
remain. See [AUDIO_OSCILLATOR.md](AUDIO_OSCILLATOR.md). It is functional DSP,
separate from the idealized capacitor/comparator `analog_vco` model.

## Voltage banks and bypass

`attenuverter8`, `mutes10`, and `cv_mix` add the specified row and mixing
contracts. Both engine versions support saved module bypass with declared
routes, silent fallbacks, preserved DSP state, and bounded action cancellation.
See [VOLTAGE_UTILITIES.md](VOLTAGE_UTILITIES.md) for normaling, polyphony, MIDI
behavior, MCP operations, examples, and remaining compatibility work.

## Gate and voltage processing

`logic`, `compare`, `process`, and `rescale` implement their documented complete
signal interfaces, including momentary controls, dedicated native panels, and
actual engine indicator telemetry. Live MCP inspection exposes coherent
per-module states and controls; offline actions support repeatable manual gates.
See [SIGNAL_PROCESSING.md](SIGNAL_PROCESSING.md) for numerical differences, state
policies, and the connected gate-shaping example.

`pulses` adds ten independent manual trigger/gate pairs, indexed runtime actions,
row-specific panel controls, and coherent held-button telemetry. See
[PULSES.md](PULSES.md) for pulse timing, safety, state and the manual voice example.

## Core inventory

Names follow the Core library; parenthesized aliases match the manual. The Audio family contains three distinct modules. [Official Core inventory](https://library.vcvrack.com/Core)

| Module | Group | Rust status | Next acceptance test or prerequisite |
|---|---|---|---|
| Audio 2 | Device I/O | Partial: selectable output device, stereo sink and voltage conversion | Hardware input, configurable rate/buffer, actual device reconnect and loopback qualification |
| Audio 8 | Device I/O | Planned | Eight-channel routing and device round trip |
| Audio 16 | Device I/O | Planned | Sixteen-channel routing and channel isolation |
| MIDI to CV (MIDI-CV) | MIDI input | Partial: `midi_cv`, 12 outputs, 1–16 voices, hardware ingress and 32-pitch QWERTY/mouse performance | Actual MIDI hardware timing/reconnect qualification, complete Rack behavior comparison and remaining configuration parity |
| MIDI CC to CV (MIDI-CC) | MIDI input | Partial: `midi_cc`, sixteen assignments/outputs, 7-bit and paired 14-bit CC, smoothing, MPE, channel filter, saved channel-zero values and audio-authoritative Learn | Global endpoint selection differs from per-module Rack routing; nonstandard signed gamepad input is excluded. Complete native/device and hardware timing/reconnect qualification; [contract](MIDI_CC.md), [45-case pinned DSP report](benchmarks/midi-cc-reference.json). |
| MIDI to Gate (MIDI-Gate) | MIDI input | Partial: `midi_gate`, sixteen normalized note assignments/outputs, exact f32 minimum pulses, fixed/velocity voltages, mono/MPE, channel filter, durable Learn and Panic | Source-style held-cell remapping is deliberate; hardware input selection is global. The [63-case actual-class report](benchmarks/midi-gate-reference.json) has bit-identical probed DSP traces; native Mac/audio/input and physical MIDI timing/reconnect qualification remain open. Host transport/Session behavior requires its separate tests; [contract](MIDI_GATE.md). |
| MIDI Map (MIDI-Map) | Parameter control | Planned | Learn/persist mappings; concurrent GUI/MCP edits |
| CV to MIDI (CV-MIDI) | MIDI output | Planned | Pitch conversion, balanced note-on/off, panic |
| CV to MIDI CC (CV-CC) | MIDI output | Planned | Quantization, rate control, endpoint round trip |
| Gate to MIDI (CV-Gate) | MIDI output | Planned | Threshold crossings and stuck-note prevention |
| Blank | Layout | Functional: `blank`, single saved 3–96 HP width, left/right edge preview, direct entry, collision-aware GUI resize, one-step width/position history, presets and duplication | Native gesture qualification; graph-only MCP/preset width edits lack layout collision validation; [contract](ANNOTATIONS.md). |
| Notes | Documentation | Functional: `notes`, multiline Unicode, draft Apply/Cancel/conflict handling, shared UI/MCP content transactions, presets and project/recovery persistence | Native IME/clipboard/input qualification; explicit 8192 UTF-8 byte bound; [contract](ANNOTATIONS.md). |

## VCV Free inventory

All names below are from the [official Free manual index](https://vcvrack.com/Free). “Related legacy” means a narrower utility exists, not a passing equivalence test. The proposed tests are OpenPatch work items, not reported test results.

| Module | Group | Rust status | Next acceptance test or prerequisite |
|---|---|---|---|
| VCO | Oscillator | Partial: `multi_vco` has four simultaneous outputs, polyphonic FM/PWM and hard/soft sync, original step reconstruction, and direct pinned-reference comparisons; separate reference ramp VCO retained | Reconstruction residual, high-frequency aliasing, voice-retirement compatibility and native qualification remain open; [contract](AUDIO_OSCILLATOR.md) |
| Wavetable VCO | Oscillator | Implemented bounded `wavetable_vco`: prepared custom audio tables, position/CV, pitch/FM, hard/soft sync, immutable publication and local host/heap tests | Checked 153-case family SDK comparison; power-of-two audio lengths >=32 only, source nonpositive-frequency path excluded. Hash proof, local Mac build, three-platform CI and reviewed native captures pass; native input/device qualification remains; [contract](WAVETABLES.md). |
| VCF | Filter | Separate `stock_vcf` digital model with LP/HP, signed cutoff/resonance/drive CV, source-style oversampling and noise; circuit ladder and legacy one-pole preserved | Pinned digital comparison and host lifecycle/consumer tests; native interaction/device, import and measured hardware qualification remain separate; [contract](STOCK_VCF.md) |
| VCA | Amplifier | Separate `stock_vca` targets VCA-1 with level, linear/fourth-power CV, mono broadcast and measured per-channel gain telemetry; reference OTA VCA and legacy gain retained | [72-trace actual-class DSP report](benchmarks/stock-vca-reference.json), callback/panel tests, reviewed Linux captures and platform CI have [bounded passes](STOCK_VCA.md#recorded-host-and-platform-validation); native macOS/Windows window input, physical I/O and complete Rack host/import parity remain open. Hidden VCA-2 remains a separate target. |
| LFO | Modulation | Implemented documented signal role: `multi_lfo`, four waveforms, FM/PWM, clock/reset and voltage modes | Direct VCV waveform/clock comparison and full native configuration parity remain open; OpenPatch defaults to bipolar output, whereas Fundamental 2.6.4 defaults to unipolar |
| Wavetable LFO | Modulation | Implemented bounded `wavetable_lfo`: custom-table interpolation, FM, clock/reset, offset/invert, coherent position and local host/file/preset tests | Exact LFO voltages in the checked family SDK run; integer wave lengths >=2. Hash proof, local Mac build, three-platform CI and reviewed native captures pass; native input/device qualification remains; [contract](WAVETABLES.md). |
| Delay | Effect | Implemented bounded `stock_delay`: clock-synchronized mono sinc delay, feedback/tone, MIX/WET, four signed depths and shared native panel | Final guarded source-defined DSP matrix, Rust/MCP checks and release build pass; source-undefined analytic SRC case has safety evidence only. Adoption microprofile is separate from callback deadlines. Three-platform CI and native captures passed; native input/device remains open; [contract](STOCK_DELAY.md) |
| ADSR EG | Envelope | Implemented stock-style signal role: separate `adsr_eg`, four signed CV depths, gate/retrigger/PUSH, 16 voices and stage lamps; original RC `adsr` preserved | Pinned DSP comparison and original shared panel are independently checked; native macOS interaction/device qualification, Rack import and reference animated envelope-position display remain open; [contract](ADSR_EG.md) |
| Mix | Mixing | Implemented documented signal role: `mix6`, six inputs, level, invert, averaging | Full native/context-menu parity and VCV differential acceptance remain open |
| VCA Mix | Mixing | Implemented signal role: `vca_mix`, four squared-gain strips, independent outputs, master VCA, linear/quartic CV and live meters | Direct reference traces, Rack import/state mapping, native qualification; [contract](VOLTAGE_MIXING.md) |
| 8vert | Voltage utility | Implemented signal role: `attenuverter8`, eight gains, downward input/polyphony propagation, voltage/percent presentation | Direct reference traces and Rack import/UI parity; [contract](VOLTAGE_UTILITIES.md) |
| Mutes | Routing | Implemented signal role: `mutes10`, ten flags, atomic invert, pinned mono inheritance and direct bypass | Direct reference traces, legacy state polarity migration and native qualification; [contract](VOLTAGE_UTILITIES.md) |
| Pulses | Gate utility | Implemented signal role: `pulses`, ten independent 1 ms triggers and held gates, native buttons and indexed MCP actions | Explicit integer timing/event policies; direct VCV comparison and native qualification remain open; [contract](PULSES.md) |
| Scope | Measurement | Partial: rack instances with independent 16-lane X/Y min/max bins, external trigger, raw passthrough, saved parameters, calibrated range controls, extrema/trigger readouts and live MCP inspection; separate calibrated two-source dock | Full display/measurement parity, persisted captures and hardware qualification; [rack contract and bounded reference](RACK_SCOPE.md), [dock contract](SCOPE.md) |
| SEQ 3 | Sequencing | Partial: 3×8 voltage editor, five inputs/sixteen outputs, gates, length/tempo CV, passthrough, atomic rotation and bounded live/offline actions | Direct pinned-reference traces, timing/mapping differences, runtime state persistence and native qualification; [contract](SEQUENCER.md) |
| Sequential Switch 1 to 4 | Routing | Implemented documented signal role: `switch_1to4`, 2–4 routes, clock/reset, de-click and live indicators | Differential reference traces, legacy state mapping and native-device qualification; [contract](SWITCHING.md) |
| Sequential Switch 4 to 1 | Routing | Implemented documented signal role: `switch_4to1`, polyphonic selection, mode-specific mono behavior and de-click | Differential reference traces, legacy state mapping and native-device qualification; [contract](SWITCHING.md) |
| Octave | Pitch utility | Implemented documented signal role: `octave`, −4…+4 knob and rounded CV | VCV boundary comparison and full host workflow acceptance remain open |
| Quantizer | Pitch utility | Partial: `quantizer`, 12-note mask, pre-offset and v2 pitch-driven voices | Factory-scale presets, broader per-voice boundary cases and direct VCV comparison |
| Split | Polyphony | Partial: `poly_split`, 16 named mono outputs and zero padding tested | Native channel indicators and complete VCV behavior/configuration comparison |
| Merge | Polyphony | Partial: `poly_merge`, sparse ordering, automatic/forced count and empty inputs implemented | Explicit forced-zero mode, native indicators and remaining VCV configuration comparison |
| Sum | Polyphony | Implemented signal interface: `poly_sum`, output attenuation, six meter bands and live channel count | Native device qualification, Rack import and unrestricted headroom differences remain; [contract](POLYPHONIC_SUM.md) |
| Viz | Measurement | Implemented signal/display role: `poly_viz`, input-only, 16 paired indicators, channel activity and sampled signed smoothing | Full upstream host comparison, Rack import and native device qualification remain; [contract](POLYPHONIC_VIZ.md) |
| Mid/Side | Stereo | Implemented documented equations: `mid_side`, encoder/decoder, widths and CV, decoder normaling | Unspecified VCV CV scale/channel details and full host behavior need differential qualification |
| Noise | Source | Seven-output `colored_noise`: white, pink, red, violet, blue, inverse-A gray and uniform black; deterministic seed and level; [contract](COLORED_NOISE.md) | Original filters with declared spectral and voltage budgets; source waveform/filter equivalence and native device qualification remain open. Legacy `white_noise` retained. |
| Random | Modulation | Implemented bounded `random`: six inputs, shared clock/phase, probability/spread, four interpolated CV outputs and mono TRIG, external-driven 1–16 lanes, native seed and shared panel | Checked 75-case controlled-draw actual-class comparison, retained proof and local host/MCP/release checks pass; platform CI/capture review pending. Global PRNG identity and native device parity are not claimed; [contract](RANDOM.md). |
| CV Mix | Mixing | Implemented signal role: `cv_mix`, three bipolar gains and independent 10 V normals | Direct reference traces, Rack import and native qualification; [contract](VOLTAGE_UTILITIES.md) |
| Fade | Mixing | Implemented signal role: `fade`, bipolar CV depth, complementary outputs, linear/square-root laws and independent bypass routes; legacy `crossfade` remains unchanged | Direct reference traces, Rack state mapping and native qualification; [contract](VOLTAGE_MIXING.md) |
| Logic | Gate processing | Implemented signal role: `logic`, eight Boolean outputs, 1 V threshold, manual B and mono/poly indicators | Direct reference traces, native qualification; no hysteresis in this reference role; [contract](SIGNAL_PROCESSING.md) |
| Compare | Voltage processing | Implemented signal role: `compare`, eight outputs, B offset, raw mono zero-padding and strict comparisons | Direct reference traces and native qualification; [contract](SIGNAL_PROCESSING.md) |
| Gates | Gate processing | Implemented signal role: `gates`, six polyphonic edge/flip-flop/timed/delayed outputs, bounded transition history, native RESET and MCP events | Direct reference traces, native qualification, explicit numerical/connection policies; [contract](GATES.md) |
| Process | Voltage processing | Implemented signal role: `process`, two held samples, track/hold pairs, slew, glide and manual gate | Reference exponential/timing differences, direct traces, runtime state import and native qualification; [contract](SIGNAL_PROCESSING.md) |
| Mult | Routing | Implemented signal distribution: `mult`, eight exact frame copies | Full VCV host behavior and boundary comparison remain open |
| Rescale | Voltage processing | Implemented signal role: `rescale`, affine gain, multiplier, limits/reflection and pre-limit indicators | Direct reference traces, arbitrary legacy multiplier migration and native qualification; [contract](SIGNAL_PROCESSING.md) |
| Random Values | Source | `random_values`: seven outputs × sixteen held voices, per-voice TRIG and manual PUSH, six ranges, coherent saved voltages and generator state; [contract](RANDOM_VALUES.md) | Original seeded generator and explicit edge ordering; VCV random-sequence equivalence, Rack import and native device qualification remain open. |
| Push | Gate utility | Implemented signal role: `push`, manual/CV push and HOLD, XOR gate, rising-gate trigger, coherent saved latch and native controls | Direct reference traces, saved-state import, smoothed reference lights and native qualification; [contract](PUSH.md) |
| Sample & Hold Analog Shift Register | Sampling | Implemented `sh_asr`: eight mono rows, old-value shift normaling, independent clocks, PUSH/CLEAR, RND and coherent held-state persistence | Complete reference qualification, native device interaction and Rack import remain open; [contract](SHIFT_REGISTER.md) |

## Hidden legacy modules and existing patches

The 49-module baseline above counts the 12 Core and 37 visible Free modules.
The [pinned Free manifest](https://github.com/VCVRack/Fundamental/blob/10dd0160c664770910e5584b7b00498cc48d9ddd/plugin.json)
also retains two hidden legacy models. They remain part of existing-patch
compatibility even though the public manual's index omits them. Track them in
addition to the visible inventory; do not silently substitute a different model
when importing an old patch.

| Model slug | Module | Rust status | Acceptance gate |
|---|---|---|---|
| `VCA` | VCA-2 | Planned | Both amplifier channels, linear/exponential control inputs, gain law, polyphony and saved parameter mapping |
| `Unity` | Unity | Planned | Both mixing sections, sum/average behavior, normaling, polarity and saved state |

## Delivery order and acceptance rules

1. **Validate the reference voice.** Publish circuit equations, component values, voltage scaling, assumptions, and unsupported physical effects. Test the ramp source, ladder, OTA VCA, and RC envelope independently and together. Compare circuit solvers against an independent high-accuracy reference; compare physical hardware only when a specific measured target exists. Preserve old patches and continuous oscillator/capacitor state during edits.
2. **Qualify the implemented engine foundation.** Version 2 now supplies named input/output ports (including Pulses’ 20 outputs), 0–16-channel frames, per-module voice policies and one-sample cable feedback. Preserve version-1 render fixtures; expand state/event cases and measure one/16-voice callback performance on target hardware. Keep audio callbacks bounded and allocation-free; document oversampling latency and parallel-path alignment. See [ENGINE_V2.md](ENGINE_V2.md).
3. **Complete non-MIDI inventory and creative workflows.** Prioritize design/patching, utilities and measurement, modulation/sequencing, expanded oscillators/effects, polyphony, projects/compatibility, and MCP/visual tools. Promote each row only with a documented port/parameter contract, persistence tests, DSP/event tests, and native integration checks. Record partial coverage explicitly rather than counting a generic substitute as complete.
4. **MIDI and live performance last.** Per user direction on 2026-10-15, defer MIDI Map and controller-driven parameter authority, remaining MIDI interfaces, stage/keyboard workflows, expanded hardware I/O and live device qualification until the final delivery phase. Existing MIDI remains supported; full 49-module and host parity still requires these gates. See the [delivery order](ROADMAP.md#delivery-order).

For voltage-oriented modules, use the [VCV voltage conventions](https://vcvrack.com/manual/VoltageStandards) as an interoperability reference: pitch in V/oct, explicit audio/CV levels, hysteretic gate detection, and documented conversion at device boundaries. Legacy normalized signals require explicit conversion. VCV also specifies cable timing and polyphonic behavior; adopting voltage units alone does not reproduce those semantics.

Completion of this inventory would establish a bounded set of Rust functional equivalents only after the specified tests pass. It would not establish compatibility with every third-party module, identical VCV DSP, or measured fidelity to unspecified analog hardware.
