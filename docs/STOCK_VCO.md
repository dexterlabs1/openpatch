# Stock VCO

`stock_vco` is a separate four-output audio oscillator targeting the
[pinned Fundamental VCO](https://github.com/VCVRack/Fundamental/blob/10dd0160c664770910e5584b7b00498cc48d9ddd/src/VCO.cpp).
It uses original Rust processing and original panel artwork. Existing
`multi_vco`, `analog_vco` and `sine` patches retain their contracts and sound.
This is a digital oscillator behavior target, not a physical circuit model.

Choose **File → Stock modular voice** for a connected example: Stock LFO
modulates Stock VCO, its saw feeds Stock VCF and Stock VCA, and an ADSR shapes
the voice. Scope displays the filter's real low-pass and high-pass outputs.
The gate source starts automatically during playback. No MIDI is required.

## Controls and signals

Inputs: `pitch`, `fm`, `sync`, `pwm`.
Outputs: `sine`, `triangle`, `saw`, `square`.

| Saved parameter | Default | Range | Meaning |
|---|---:|---:|---|
| `frequency` | 0 | −76…76 | Semitones relative to the f32 C4 reference, 261.6256 Hz |
| `pulse_width` | 0.5 | 0.01…0.99 | Square high fraction before PWM |
| `fm_amount` | 0 | −1…1 | Signed exponential or linear FM depth |
| `pwm_amount` | 0 | −1…1 | Width adds `PWM / 10 × depth` |
| `linear_fm` | 0 | 0 or 1 | Exponential FM or linear FM |
| `soft_sync` | 0 | 0 or 1 | Hard reset or soft direction reversal |

The Frequency fader travels in semitones. Its readout and exact editor use
`261.6256 × 2^(frequency / 12)` Hz, with the inverse conversion at commit.
MCP saves and edits semitones: `frequency: -12` means one octave below C4.
The displayed control value is the unmodulated reference, not a measured
instantaneous pitch. Exact-entry commands retain graph revision guards.
FM and PWM depths are deliberately excluded from Randomize.

The four faders and two mode buttons use the same renderer in the rack and
browser. Each control has a visible MCP construction cue at its real target;
manual input cancels that cue. A preview has no runtime lamps or animation.

## Oscillator behavior

Pitch sets the channel count, with a minimum of one. Input mono signals
broadcast across the active quartet; missing short-poly lanes read zero in
ordinary Engine frames. The exponential uses source-ordered f32 arithmetic.
Exponential FM changes pitch in octaves; linear FM adds `C4 × FM × depth` Hz.
Frequency clamps to 0…sample-rate/2 and the phase advance caps at 0.49 cycles
per sample. Linear FM can stop the oscillator; it is not through-zero FM.
PWM clamps the resulting high fraction to 0.01…0.99.

The ideal saw and triangle, ninth-order sine approximation and latched square
receive original prepared minimum-phase step and slope corrections. Every
connected waveform has its own correction history and approximate 20 Hz DC
blocker. Corrected signals may overshoot nominal ±5 V. Matching this digital
model is not a claim of zero aliasing or a hardware measurement.

An enabled sync input retains its own last value. A rising crossing through
zero with a fractional position in `(0,1]` splits processing within the sample.
Hard sync resets phase and processes the remainder; soft sync reverses and
continues from the crossing. Ordinary waveform crossings are handled before
and after that event. Falling-to-zero and zero-to-positive inputs do not
constitute a rising crossing. An absent sync input freezes its detector history.
Moving PWM can introduce a correction at the start of a parked or moving frame.

Active four-lane groups advance hidden phase/detector/filter histories, but
correction insertion is limited to visible lanes. Entire inactive groups freeze
and resume. Each unconsumed waveform freezes its correction cursor and filter
history; its source-defined phase processing continues. Consumers include
cables, the selected output and temporary analysis taps. Unconsumed outputs
have zero channels and zero-filled host storage during ordinary processing.

## State and observation

Same-ID/kind/sample-rate graph replacement preserves bounded runtime state;
receiving controls win. Source changes preserve detector history for this kind.
Fresh instances and rate changes start fresh. Initialize restores the six
control defaults without resetting oscillator runtime: the target has no
runtime reset override. Generic OpenPatch bypass freezes runtime and emits
mono zero on all outputs; the comparison's bypass adapter is an explicit host
policy shim, not proof of Rack Engine bypass behavior. Ordinary cables retain
OpenPatch's one-sample transport delay.

Five actual lamp components are phase red/green/blue, Linear FM and Soft sync.
They update every 16 processed samples. Mono phase lamps use the source's
signed smooth-release state; polyphony uses blue. The renderer clamps visual
brightness only, preserving signed observations for analysis. Lamps emit in a
dark room. Stale graph/backend packets cannot represent current observations;
headless MCP without an audio callback reports performance as unavailable.
These values, phase and reconstruction histories are not saved parameters.

Two float32 boundaries need explicit treatment. A tiny negative soft-sync
residual can wrap to exactly1.0 because `phase - floor(phase)` rounds in f32;
OpenPatch preserves that observable reference endpoint. The original independent
validator incorrectly required a half-open real-number interval. A versioned
[phase-endpoint erratum](../scripts/stock_vco_reference/phase_endpoint_erratum.json)
corrects only that invariant, retaining exact phase comparisons and every other
gate. The original failed result and declaration remain preserved.

Separately, an extremely early positive correction event can make
`1 - subsample` round to1, yielding oversample row16 in the reference's
16-row table. Row padding does not legalize that access. Rust safely maps that
endpoint to the following sample of row0. A dedicated finite-sync regression
qualifies this native safety extension without executing the reference's
out-of-bounds path. No source-equivalence claim applies to undefined reference
behavior, including that path; the bounded recorded matrix is not a proof for
all possible finite sync inputs.

## Qualification status

The acceptance and fixtures are declared independently before the first
comparison. The actual pinned class is compiled separately against the genuine
Rack SDK; no SDK or reference panel/source is built into the native app.
The gate compares every output count, connected active voltage and compact
runtime field on every frame, with full logical correction tails at declared
checkpoints. Kernel preparation is compared separately. The frozen oracle matrix covers
eight consumer masks, not all 16 combinations or 256 transitions; its overbroad
`all_masks` tag is corrected explicitly in the reproduction/report scope.
No startup frames,
phase alignment or difficult in-domain cases are discarded. Rust-only safety
cases remain separate from numerical equivalence.

The [recorded report](benchmarks/stock-vco-reference.json) passes all 552,428
reference frames (280 cases per rate at 44.1/48/96 kHz), all 1,956 separately
labelled Rust-only safety frames and1,024 prepared kernel coefficients under
the explicit phase-endpoint erratum. Every compared active output voltage,
filter history and checkpoint correction tail has zero measured error. All
exact count, phase, detector, direction, latch, cursor, divider and lamp checks
pass. Full logical tails were inspected at 2,645 checkpoints per rate. All 787
retained artifact files and the unchanged 75-file production closure were
verified before complete revalidation. The original failed declaration result
and its raw evidence remain unchanged.

The [separate spectral report](benchmarks/stock-vco-spectral.json) records
72 Hann-window measurements from actual retained tone traces. Its harmonic
guards and leakage limitations are explicit; it is not an alias-only power
measurement or a universal quality claim. Its qualification context distinguishes
the original failed invariant from the corrected source-domain result.

Local validation passes 1,829 full-workspace Rust tests (six existing ignored),
plus the added actual-production phase-endpoint regression. All 90 MCP tests,
strict Clippy/formatting and the ARM workspace release build pass. The separate
Mac app bundle passes deep strict ad-hoc signing, ARM64, exact font/notices/icon
and runtime dependency checks, with all executable file-backed sections and
UUIDs matching the tested releases. It was not installed or launched.
Both exact-source CI runs passed all eight platform/visualizer jobs in [PR12](https://github.com/dexterlabs1/openpatch/pull/12). All 121 native captures passed inventory and PNG integrity checks; eight actual views were reviewed and ten prior reviewed views were byte-identical. Native Mac input and audio-device qualification remain open.
[Reproduction instructions](../scripts/stock_vco_reference/RUNNING.md) explain
fresh comparisons and complete retained-trace revalidation.

Full host/import compatibility, native macOS window/device interaction,
physical hardware accuracy and a measured performance advantage over Rack
remain separate acceptance gates.
