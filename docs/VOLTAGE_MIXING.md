# Voltage-controlled mixing

**File → Layered stereo morph** opens four oscillators feeding a VCA mixer,
with an LFO moving channel levels and a two-output crossfade. Both outputs
feed the stereo sink. These are original Rust functional processors, not
hardware-calibrated amplifier models. They require engine version 2; existing
`crossfade` patches retain their earlier behavior.

## VCA Mix

`vca_mix` has inputs `in1`…`in4`, `cv1`…`cv4`, and `mix_cv`; outputs are
`out1`…`out4` and `mix`. `level1`…`level4` range from 0 to √2 (default 1),
with squared gain. `mix_level` ranges from 0 to 2 (default 1), with linear
gain. Strip outputs always remain in the sum; the master affects only `mix`.

Unpatched CV gives unity gain. Connected CV gives `max(0, V/10)`; saved
`channel_exp` and `mix_exp` flags independently raise those factors to the
fourth power. Values above 10 V can amplify. Signal inputs alone determine
the shared output width, with a minimum of one. Mono broadcasts; missing
short-poly lanes are zero.

Four meters sum each strip's signed values across voice groups rounded up to
four lanes, including padded mono-broadcast lanes, then divide by 5 before
absolute peak detection. Opposite signs cancel. Attack is immediate; release
uses `value += (target-value)*30/sample_rate`. Every 512 processed samples,
the -24…0 dB range maps to brightness 0…1. Thus mono signals can meter above
their individual voltage. No bypass routes are defined.

Behavior reference: [pinned VCMixer source](https://github.com/VCVRack/Fundamental/blob/10dd0160c664770910e5584b7b00498cc48d9ddd/src/VCMixer.cpp)
and [Rack meter helper](https://github.com/VCVRack/Rack/blob/061ccf63c1758599396ac1bb10d47345d9d34076/include/dsp/vumeter.hpp).

## Fade

`fade` accepts `a`, `b`, and `cv`; outputs are `out` and `complement`.
`position` ranges from 0 to 1 (default 0.5); `cv_amount` ranges from -1 to 1
(default 0). Its saved `law` is 0 for linear or 1 for square-root gains.

For each voice, `x = clamp(position + cv/10 * cv_amount, 0, 1)`. Linear
weights are `(1-x, x)`; square-root weights are `(sqrt(1-x), sqrt(x))`.
`out` mixes A then B with those weights; `complement` swaps the weights.
Both outputs coincide at the center. The square-root law gives constant
power for uncorrelated inputs, without detecting signal correlation.

Width is `max(a.channels, b.channels, 1)`; CV does not create voices. Mono
broadcasts and missing short-poly lanes read zero. Bypass directly routes
A to `out` and B to `complement`, retaining each input's independent width;
an empty input becomes mono zero. Fade has no meter or runtime button.

Behavior reference: [pinned Fade source](https://github.com/VCVRack/Fundamental/blob/10dd0160c664770910e5584b7b00498cc48d9ddd/src/Fade.cpp).

## Host behavior and boundaries

The mixer displays actual amplitude gain in dB. Its channel controls account
for the squared gain law; the master uses the linear law. Saved CV response
and pan-law settings participate in project persistence and shared UI/MCP
undo. Inspect `catalog` for exact parameter bounds and port metadata.

Live `inspect_performance` publishes four mixer meter values through `lights`:
`count=4`, channel brightness in each pair's first element, second element
zero. Unavailable telemetry is shown separately from silence. Compatible
edits retain meter envelopes and divider phase; document replacement starts
fresh state. Resetting parameter values through edits retains those histories.
Bypass freezes meters while the module emits mono zero.

OpenPatch computes bounded arithmetic in f64, maps nonfinite inputs to zero,
and clamps final outputs to ±1,000,000 V. Outputs are calculated even when
unconnected so scope/MCP taps remain usable. These policies and library
rounding prevent a bit-identical claim against the f32/SIMD reference.
Every cable adds the normal version-2 one-sample delay.

Acceptance covers gain/CV boundaries, sparse polyphony, padded metering,
state adoption, callback heap activity, project settings, and MCP signal
capture. Direct execution comparisons against Rack, `.vcv` parameter/state
import, and native device qualification remain open. No third-party panels
or source files are bundled with these processors.
