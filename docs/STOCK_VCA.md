# Stock VCA

Status: work in progress. The standalone DSP oracle passes; host/UI integration
and native captures have not been compiled or qualified yet. This branch is
a recovery checkpoint, not a release.

A separate `stock_vca` targets the VCV Free VCA-1 digital behavior, preserving
`analog_vca` and legacy `gain`. This is ideal amplification and a specified CV
response, not a measured analog hardware clone.

## Frozen contract and acceptance

Declared before implementation/oracle comparison, 2026-10-15. Reference:
[pinned VCA-1.cpp](https://github.com/VCVRack/Fundamental/blob/10dd0160c664770910e5584b7b00498cc48d9ddd/src/VCA-1.cpp)
and [VCV Free manual](https://vcvrack.com/Free#VCA). Target VCA-1, not the older
VCA-2 implementation. The oracle temporarily compiles the genuine class against
an installed Rack SDK; upstream source/assets are not bundled in the product.

Version 2; inputs `cv`, `signal` in reference order, one `out` output.
`level` ranges 0..1, default 1, displayed 0–100%. `response` is a saved integer
switch: 0 = exponential, 1 = linear (default). Fractional/nonfinite/unknown
parameters reject. The native UI exposes both modes directly and in the module
menu, with one shared rack/browser panel and direct level entry.

Every frame, width is max(1, CV width, signal width), up to 16. Both mono inputs
broadcast; short polyphonic inputs supply zero for missing channels. Unconnected
or zero-channel CV normalizes to unity. Connected zero-volt CV mutes. Effective
gain = level when CV absent; otherwise level * clamp(CV / 10, 0, 1), with the
clamped CV raised to the fourth power in exponential mode. Output = signal *
effective gain, preserving f32 multiplication order. Nonfinite inputs become
zero; finite inputs obey the host +/-1,000,000 V bound. Inactive output storage
is zero. Processing and gain acquisition continue with unused outputs.

The meter reports per-channel effective gain, not audio amplitude: silent audio
can have full gain. Record it each frame. Expose it through existing coherent
module telemetry (`ModuleLightState`: count16, input_channels active width,
brightness[channel][0] gain, second component zero). Unexposed inactive gain
slots clear, while the source retains old array storage outside its visible
width. Fresh telemetry is one channel with zero gain. Host UI publication has
its existing bounded callback cadence; no invented signal or animation drives
the meter. Browser/unavailable telemetry shows configured level and an explicit
unavailable state without fabricated live gain.

Same ID/kind/rate adoption preserves gain telemetry while applying new controls;
new identity/rate starts fresh. Bypass passes raw signal (no CV gain/broadcast)
and freezes meter state. Existing host delayed-cable and bypass-transition
policies apply. Initialize resets parameters and clears gain telemetry;
Reset clears telemetry. This immediate meter clearing is a declared host policy:
the reference has no custom reset handler and its visual values update next
process. No persistent DSP state exists beyond telemetry. No callback heap
allocation, reallocation, locks or module-string routing is allowed.

Predeclared oracle gate, every connected output and active gain from frame zero
at 44.1/48/96 kHz, no warmup/alignment/startup exceptions:

`abs(actual - reference) <= 2e-6 + 2e-6 * abs(reference)`.

Actual output and gain channel counts must match exactly. Compare genuine SDK
lastChannels/lastGains; do not synthesize expected width from the input formula
alone. Fix fixtures before the first comparison: defaults, level0/1/intermediate,
both responses, absent versus connected-zero CV, negative/10V/overrange CV,
mono broadcast in both directions, mismatched 0/1/3/5/16 widths, poisoned padding,
16→1→5→16 shrink/regrow, audio-rate audio/CV, response/level edits. Include fresh
counterfactuals proving linear/exponential output differs at partial CV, while
both agree at endpoints; silent-audio gain must remain observable.

Public Engine tests separately exercise cable delay, output selection, temporary
taps, unused-output gain updates, bypass, reset/adoption and callback partition
independence. Real callback tests cover first processing, telemetry and graph
crossfade without heap activity. Benchmark one/sixteen voice callbacks against
the block deadline, recording machine and limits; do not infer Rack superiority
or device latency from throughput. Native scenes must show actual measured
polyphonic gains in normal/dark room; preserve all example Notes within their
painted bounds. Cross-platform CI and actual Linux captures are release gates;
native Mac window/input/playback remains separate qualification.
