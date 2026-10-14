# Polyphonic SUM

SUM combines every active channel of its `signal` input into the single-channel
`out` output. Its **LEVEL** control attenuates the result from 100% to silence.
Existing patches without a level setting retain unity gain. There is no
automatic averaging: for four equal voices, 25% level restores the amplitude
of one voice.

The module and its browser thumbnail use the same panel renderer. The level
knob supports normal parameter gestures and exact entry. **INPUT CHANNELS**
shows the measured input width, including active channels carrying zero volts.
An em dash and **METER UNAVAILABLE** mean that no current audio publication is
available; they do not mean a disconnected input. Channel zero means a published
zero-channel input. Bypassing the module silences its output and freezes its
meter state; the host's bypass appearance identifies this frozen display.

## Signal and meter contract

| Property | Behavior |
|---|---|
| Patch version | Version 2 only; existing version-1 semantics are unchanged |
| Ports | `signal`: 0–16 channels; `out`: always one channel, including empty input |
| Parameter | `level`: continuous 0–1, default 1; saved in patches and presets |
| Audio calculation | Active input channels summed in f32 order, multiplied by level, then clamped to ±1,000,000 V; the graph sanitizes nonfinite source voltages before cable summing |
| Meter | Absolute post-level output relative to 10 V; opposite-polarity voices cancel before measurement |
| Bands | Top to bottom: 0, −3, −6, −12, −24, −36 dB; top indicator turns on at or above 0 dB |
| Timing | Meter samples every 16 processed frames; immediate attack and Euler release with coefficient `10 × (16 / sample_rate)`; brightness and channel count refresh every 512 frames |
| Initial display | Zero bands and zero channels until the first refresh |
| State | Compatible same-ID/kind/rate edits retain envelope, divider, bands and channel count; fresh instances reset; bypass freezes processing |

The meter is a sampled level indicator, **not** a true-peak or clipping detector.
Short transients between its sampling frames can be missed. Its 0 dB mark is
10 V, not a hard limit on module output or a guarantee against device clipping.
SUM adds no audio delay; version-2 cable propagation still adds one sample per
cable.

## MCP and live state

`catalog` describes the level and meter contract. Set `level` using
`set_parameter` or an atomic edit plan; normal revision checking and undo apply.
With a live native audio backend, `inspect_performance` exposes
`performance.modules.<id>.lights.input_channels`, six brightness bands and the
actual published revision/frame. `lights.count` is the number of meter bands
(six), not the input width. SUM has no sequencer state or momentary actions.

The audio callback publishes channel count and meter bands together without
locks, heap work or JSON serialization. A superseded graph's readings remain
unavailable until the current revision is published. Closing the backend clears
availability. Headless MCP inspection does not fabricate live telemetry.

## Reference and qualification

The functional target is [VCV Sum](https://vcvrack.com/Free#sum), with behavior
checked against [Fundamental 2.6.4's pinned Sum source](https://github.com/VCVRack/Fundamental/blob/10dd0160c664770910e5584b7b00498cc48d9ddd/src/Sum.cpp).
This is original Rust code and original panel artwork, not an analog circuit
model. OpenPatch retains its finite-output safety policy; Rack does not impose
the same ±1,000,000 V bound. Extreme f32 overflow is outside the
reference qualification. Full `.vcv` parameter/state import is still absent.

Core tests cover attenuation, signed summing, empty/16-channel inputs, meter
cadence and release, state adoption, bypass, persistence and callback partition
invariance. Desktop tests cover actual callback-to-MCP telemetry, stale revision
rejection, document replacement, allocation-free processing, level entry and
shared browser/panel rendering. Native macOS input and device playback remain
separate qualification gates.

The [reproducible reference harness](../scripts/sum_reference/README.md) compares
production SUM DSP against genuine Rack SDK summation, meter and divider
primitives using an independent scheduling driver. The [recorded macOS ARM run](benchmarks/sum-reference.json)
passed 18 cases / 147,456 frames at 44.1, 48 and 96 kHz: output voltages and
latched channel counts matched exactly; the maximum brightness difference was
2.98 × 10⁻⁷. This checks those algorithms, not execution of the full upstream
module, native UI or hardware. The report records hashes, tolerances and scope.
