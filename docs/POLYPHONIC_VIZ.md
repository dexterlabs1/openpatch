# Polyphonic voltage visualizer

`poly_viz` is a version-2 input-only measurement module: one polyphonic `signal`
input, no parameters, and **no outputs**. Connect it alongside an existing signal
path to see the polarity and activity of up to 16 channels. It cannot be selected
as the monitored output or used as a cable source or signal-capture tap.

The original Rust implementation follows the specified signal/display role of
VCV Free VIZ. It is a functional digital utility, not a hardware model or a claim
of complete Rack compatibility. No VCV implementation or artwork is vendored.

## Signal and display contract

Every 16 processed sample frames, starting at frame 16, VIZ latches the input
channel count and updates all 16 paired indicators. Each pair receives signed
targets `[voltage / 10, -voltage / 10]` for green and red. Inactive channels have
zero targets; their positive release tails remain visible while their channel
numbers dim. All other frames retain the previous values and count.

A target at or above the current brightness updates immediately. A lower target
updates with `brightness += (target - brightness) * 30 * delta_time`, where
`delta_time = (1 / sample_rate) * 16`, evaluated in float32. Internal values are
signed, unsquared and unclamped, including values below zero and above one.
Only the UI clamps brightness to its visible range. Consequently this is a
sampled polarity indicator, not a peak detector: pulses between update frames
can be missed. It is not a calibrated oscilloscope or RMS meter.

Version-2 cables introduce one sample of delay. OpenPatch's existing graph
safety and stacked-input rules still apply; unrestricted-voltage parity with
Rack is not claimed. No passthrough or dummy output is invented for VIZ, and
input-only graphs render silence when no output is selected.

Bypass freezes the divider, count and lights. Compatible live edits preserve
state for the same module identity, kind and sample rate. New document/module
incarnations and sample-rate changes start with zero count and brightness.
Runtime brightness is not saved in patch JSON.

## Native UI and MCP

The rack and browser use the same panel renderer: two columns of eight numbered
channels, paired polarity indicators and one separated signal jack. Numbers
show active channels independently of brightness. Missing live telemetry is
explicitly unavailable, rather than fabricated zero activity.

The fixed-size callback telemetry bridge publishes all 16 pairs and their count
with the graph revision and module identity. `inspect_performance` exposes them
under `modules.<id>.lights`, including `input_channels`, `count: 16` and
`brightness`. VIZ has no sequencer state or manual performance controls. A
headless stdio server without a live desktop attachment cannot supply live
indicator state. Catalog, patch editing, save/load, history and bypass use the
same session operations as other modules.

## Validation and remaining gates

The [reference harness](../scripts/viz_reference/README.md) compares production
Rust processing against genuine Rack SDK Input, Light and ClockDivider
primitives with independently written orchestration. The
[recorded report](benchmarks/viz-reference.json) covers 18 streams and 147,456
frames at 44.1/48/96 kHz; channel counts and all 32 raw brightness components
matched exactly in the recorded environment. It does not execute the complete
upstream module or GUI.

Separate tests cover zero-output nodes before/between/after ordinary nodes,
invalid output selection and taps, cable delay, signed release, channel
retirement, block partitioning, state migration, bypass, callback allocation,
revision-safe live publication, MCP rejection atomicity, persistence, and
native/browser rendering. Native tests include actual pointer cable drops and
retiring-channel release tails. The Linux smoke fixture displays both polarities
using actual offline DSP snapshots; it is labelled as such.

Full Rack import/state mapping, full upstream host comparison, and native macOS
window/input/device qualification remain open. This module does not establish
whole-program parity or a performance advantage over VCV Rack.
