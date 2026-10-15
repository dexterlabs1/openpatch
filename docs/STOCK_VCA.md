# Stock VCA

Bounded qualification: the actual-source DSP oracle, six DSP unit tests, five
public Engine tests, one core allocation guard and fifteen desktop tests pass.
Reviewed Linux captures and macOS ARM, Windows and Linux CI passes are recorded
below. Native macOS/Windows window input and physical MIDI/audio-device
qualification remain open, as do complete Rack host/import parity and analog
hardware calibration.

A separate `stock_vca` targets the VCV Free VCA-1 digital behavior, preserving
`analog_vca` and legacy `gain`. This is ideal amplification and a specified CV
response, not a measured analog hardware clone.

## Play and inspect the voice

Choose **File → Stock VCA voice** to open the connected oscillator → Stock VCF →
Stock VCA example. A clock drives ADSR EG into the amplifier's CV input, and Scope
compares the signal before and after amplification. Press **Play** to audition
it.

Drag the **LEVEL** fader or click its printed percentage to enter a value from
0–100%. **LIN** applies CV proportionally; **EXP** applies a fourth-power curve.
The module context menu offers the same response modes. The default is 100%
level and linear response. At 5 V CV and full level, LIN gives 50% gain and EXP
6.25%; both give zero at 0 V and full gain at 10 V. An unplugged CV input leaves
the level control in charge; a connected 0 V cable mutes it.

The meter's columns show each active channel's effective gain, including when
its audio is silent. The fader marks the configured level. Browser inspection
and unavailable telemetry show that setting with an explicit unavailable state;
they do not display invented live gain. Bypass passes the raw signal's channel
width and freezes the gain columns.

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

## Recorded DSP qualification

The [actual-class report](benchmarks/stock-vca-reference.json) passes 72 traces
at 44.1, 48 and 96 kHz: 55,872 frames and 800,328 active output/gain comparisons.
The maximum measured absolute error is 9.54e-7. The frozen gate above remains
`2e-6 + 2e-6 * abs(reference)` for every active value from frame zero, with exact
channel counts. No alignment, warmup or tolerance change was used.

The runner compiles the genuine pinned `VCA_1` class against an installed Rack
SDK, retaining its `std::pow(cv, 4.f)`, and compares actual output metadata,
`lastChannels` and visible `lastGains`. Fresh counterfactuals give exactly 0.5
linear gain versus 0.0625 exponential gain at 5 V CV; the 0 V and 10 V endpoints
agree exactly. Acceptance and input hashes were frozen before comparison.
[Method and reproduction](../scripts/stock_vca_reference/README.md) describe the
adapters, SDK padding, unused output metadata and recorded source/toolchain hashes.

This is finite digital DSP coverage. It does not measure analog hardware or
qualify native macOS controls, audio devices or playback. Public Engine, desktop
callback, panel, native capture and cross-platform gates have their own evidence
below; passing this oracle does not complete VCV module or host parity.

## Recorded host and platform validation

On 2026-10-15, [e50f342 CI](https://github.com/dexterlabs1/openpatch/actions/runs/38770399339)
and [c903b211 CI](https://github.com/dexterlabs1/openpatch/actions/runs/38776627903)
passed correctness/allocation tests, clippy, release builds and MCP checks on
macOS ARM64, Windows x86-64 and Linux x86-64. Linux native window/GLSL smoke also
passed; that step was skipped on macOS and Windows.

The dedicated [callback tests](../crates/openpatch-desktop/src/stock_vca_audio_tests.rs)
cover first-buffer and graph-crossfade heap activity, partition-independent audio
and true gain publication with silent signals, and raw-width bypass with frozen
gain telemetry. [Panel tests](../crates/openpatch-desktop/src/stock_vca_ui_tests.rs)
cover actual input, precise entry, keyboard focus/history, shared rack/browser
rendering and dark-room gain emission. The App example fixture also passes.
Actual Linux normal/dark captures were reviewed: full gain can remain bright
while silent signal plugs remain dark.

These are bounded integration and platform-build passes. They do not establish
native macOS/Windows window input, physical MIDI/audio I/O, analog calibration,
complete Rack state/import compatibility or VCA-2 coverage.

## Measured callback cost

The [release callback profile](benchmarks/stock-vca-callback-profile.json) on an
Apple M1 Max, ARM64/macOS 26.7.1, renders connected VCO → Stock VCF → Stock VCA →
Sum lanes with a shared mono ADSR EG. At 48 kHz in 256-frame stereo buffers
(5.333 ms deadline), observed steady p99 was 0.156 ms for one voice and 0.729 ms
for sixteen voices. The largest observed sixteen-voice first buffer during
graph adoption/crossfade was 1.413 ms. All measured VCA-workload buffers fit the
block deadline. Instrumented and uninstrumented callbacks produced identical
audio; separate callback tests found no heap activity.

These headless observations include the oscillator, filter, envelope, summing
and host conversion. They do not establish isolated VCA throughput, audio-device
latency, hard real-time behavior, or a speed advantage over VCV Rack.
