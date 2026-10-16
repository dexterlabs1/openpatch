# Stock Mid/Side

`stock_mid_side` is a separate stereo encoder/decoder. Existing `mid_side`
patches retain their original CV scaling and voice policies.

The encoder produces `mid = (left + right) / 2` and
`side = ((left - right) / 2) × encode_width`. The decoder independently normals
its unpatched mid and side inputs to the encoder's raw outputs. It produces
`left = mid + side × decode_width` and `right = mid - side × decode_width`.

Each effective width is `max(knob + (CV / 10) × 2, 0)`. The knobs range from
zero to two; positive CV can increase width beyond two. Mono width CV broadcasts
to existing voices. Signal inputs read individual lanes without mono broadcast.
CV alone does not create voices.

The encoder uses the larger left/right channel count. When either decoder input
is connected, its channel count comes from those external decoder inputs; otherwise it
follows the encoder. Raw output storage retains the stock rounded-quartet writes
and inactive tails. A disconnected output stays at zero channels; a connected
output requested at zero channels becomes mono zero. Public frames project only
active lanes. These distinctions matter for internal normaling and live edits.

There are no source module lights or bypass routes. Entering or leaving bypass
clears each connected output's active prefix; the bypassed processor makes no
new output writes. Disconnect/reconnect operations still affect its port state.
Initialize resets the width parameters without erasing private port history.

Choose **File → Mid/Side stereo voice** for a connected example. Two Stock VCOs
feed the encoder; a slow Stock LFO moves decoder width. Scope displays the
actual mid and side signals, and Stereo Audio receives the decoded left/right
outputs. The rack and library use the same panel renderer and named jack layout.
Encoder/decoder headings and width captions occupy separate lines at all five
checked rack/browser scales; fader and jack targets stay in place.

## Reference evidence

The reference is [Fundamental MidSide.cpp at 10dd0160](https://github.com/VCVRack/Fundamental/blob/10dd0160c664770910e5584b7b00498cc48d9ddd/src/MidSide.cpp),
executed inside the genuine Rack 2.6.6 Engine. The independent declaration and
all five acceptance files were frozen before the first execution. The complete
paired gate remains unchanged.

The final optimized production comparison passes **74 histories and all 890
event snapshots**, including **1,472 hand anchors**. It checks six real Core
source modules, the actual Stock Mid/Side processor, and eight real sink inputs.
Every source, module and sink port count and raw lane is compared after every
event, including edits that process no samples. The complete production JSONL
trace is byte-identical to the retained genuine SDK trace. See the
[paired report](benchmarks/mid-side-paired-host-reference.json) and
[independent SDK report](benchmarks/mid-side-sdk-host-reference.json).

The separate [output lifecycle proof](benchmarks/output-lifecycle-reference.json)
covers 224,988 edge histories and 15,749,160 exact fields. That proof executes
zero DSP samples and is not itself a Mid/Side audio comparison.

The common host transport retains private ports across version-2 graph rebuilds.
Native models read their retained inputs before processing; cable transfer runs
after all producers finish. Existing models keep their established logical-frame
DSP path and publish through an explicit producer adapter. This does not claim
that every older model's private behavior matches a Rack module.

## Delivery status

The final paired host and output-lifecycle gates pass. Across the final Core,
Session and Desktop suites, 1,983 Rust tests pass with zero failures and six
existing ignored tests. All 96 Python/MCP checks, strict workspace Clippy and
formatting pass. Two public MCP tests check all four 96-frame outputs, controls,
presets, Initialize, persistence and atomic rollback. The shared-panel and
playable factory tests pass, including a reproduced-before-fix Scope retry case.
Three appended native scenes bring configured captures to 130, preserving every
previous capture request and the CI timeout.

The final paired performance screen preserves all reported signal statistics
and checksums, but adds processing cost against main `5844d26`: median paired
time increases are 30.11% for mono routing, 10.24% for sixteen-channel routing,
2.01% for the mixed mono voice and 0.24% for the mixed sixteen-voice workload.
The last difference is near measurement variation. This always-on transport
also costs CPU in legacy patches without Stock Mid/Side. All four experiments,
including the reverted optimization, remain in the [performance report](benchmarks/host-transport-overhead.json);
see [measurement boundaries](PERFORMANCE.md#retained-host-port-transport-measured-incremental-cost).
An isolated optimized ARM workspace build and separate ad-hoc signed Mac bundle
pass signature, binary-section/UUID, font, notice and icon checks. The local bundle
uses the exact measured and MCP-tested CLI; it was not installed or launched.
Platform CI and reviewed native captures remain pending. No hardware accuracy or performance advantage is
claimed. Native Mac window/input and physical audio-device qualification remain
separate open gates.

Raw arithmetic follows float32 source order without amplitude clipping.
Diagnostic nonfinite results may remain in private module storage. Outgoing
cable transfer sanitizes each nonfinite source value separately; this is not a
promise that all private raw diagnostics are finite.
