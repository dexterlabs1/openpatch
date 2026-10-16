# Polyphonic Split

Split routes channel 1 of its `signal` input to `out1`, channel 16 to `out16`,
and each channel between to its corresponding named output. All sixteen outputs
are mono, including silent and missing input channels. A mono input goes to
`out1` only: Split does not broadcast it across the other outputs.

The **INPUT CHANNELS** screen reports the processed input width from 0 to 16.
A connected mono signal at 0 V reads **1**, not zero. An em dash means that a
valid current observation is unavailable. Rack modules and browser thumbnails
use the same panel; previews do not invent a live count. The numeric screen
remains visible in a dark room. There are no module signal lamps in the original
Split; OpenPatch's port activity lights remain a separate host feature.

## Controls, observation, and MCP

Split has no saved parameters or configuration menu. Initialize leaves its last
observed input count alone until the next processed sample, as does the reference
module's default reset handler. Presets, project files, recovery and duplication
carry its identity, cables and bypass state without saving a measured count.

The live callback exposes
`performance.modules.<id>.lights.input_channels` through `inspect_performance`.
The optional integer is initially zero, updates each processed sample, and shares
its graph revision and publication with the module snapshot. `lights.count` is
zero and brightness is zero: a count does not fabricate signal lamps. A headless
MCP session without an audio callback reports no live performance snapshot.

Same-identity, same-kind, same-rate edits preserve observations until processing
supplies new ones. Fresh instances and changed sample rates start at zero.
OpenPatch's host bypass freezes Split processing and its observations. Pending
revisions, replaced instances, invalid counts, and a closed backend must not
present stale observations as current measurements.

MCP uses the existing `connect`, `disconnect`, `capture_signal`, and
`analyze_signal` tools. Select `out1` through `out16` explicitly when observing an
output. There is no `input_channels` parameter to set.

## Routing boundary and reference

The functional target is the [pinned Fundamental Split class](https://github.com/VCVRack/Fundamental/blob/10dd0160c664770910e5584b7b00498cc48d9ddd/src/Split.cpp#L29-L36).
Its process method deliberately copies all sixteen raw input storage slots,
even those outside the input width, to help diagnose buggy modules. OpenPatch's
original Rust splitter preserves those raw bits at the module boundary. This
includes signed zero and nonfinite diagnostic storage; Split performs no clipping
or arithmetic on a copied voltage.

This diagnostic behavior differs from what a normal cable supplies.
[Rack's cable engine](https://github.com/VCVRack/Rack/blob/8ea470f343058ade6f75f16d503a1eb330e1700c/src/engine/Engine.cpp#L351-L394)
copies active source slots only, replaces nonfinite active values with zero,
and clears destination slots retired by shrink. A forced-short or zero-width
Merge therefore cannot reveal its inactive raw storage through an ordinary
cable into Split. Normal OpenPatch producers also provide zero-padded frames.
Merge → Split transitions through 16, 1, 0 and back to 16 channels are covered
separately by a public Engine test. OpenPatch's internal poisoned-edge diagnostic
fixture is not a claim that Rack transports poisoned inactive source storage.

Connected reference outputs are mono because
[Rack cable insertion](https://github.com/VCVRack/Rack/blob/8ea470f343058ade6f75f16d503a1eb330e1700c/src/engine/Engine.cpp#L956-L962)
initializes newly connected outputs to one channel; Split itself changes only
their voltage. OpenPatch emits its sixteen named mono outputs independently of
whether a cable or tap currently consumes them.

OpenPatch uses original Rust implementation and panel artwork. Reference code
and its license are retained in ignored qualification evidence, not vendored
as OpenPatch implementation. Numerical comparison, native captures, hardware,
Rack import, and real audio-device behavior are separate qualification gates;
none alone establishes complete Rack compatibility.

The [frozen reference harness](../scripts/split_reference/README.md) and
[recorded macOS ARM result](benchmarks/split-reference.json) qualify 1,065 normal
public-Engine cases (17,040 output values) and 204 direct-module diagnostic
cases (3,264 raw-bit values) at 44.1, 48 and 96 kHz. Every observed count and all
sixteen output values match the actual pinned SDK class exactly; raw diagnostic
slots have no exclusions. Independent checks verified 786 retained artifacts,
73 production source files, and 668 SDK headers. This records the specified
compiler/SDK/platform and fixture domain; it is not hardware qualification or
an unrestricted host-compatibility claim. Native capture qualification is pending.
