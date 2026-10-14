# Rack Scope

`scope` is an original Rust, version-2 functional module with a rack display.
It complements the [two-source Signal scope dock](SCOPE.md). Each rack instance
acquires its own patchable X/Y signals and external trigger; it does not change
the dock's controls or capture history.

## Signal and acquisition contract

Inputs are `x`, `y`, and `trigger`; outputs are `x` and `y`. Each output passes
its corresponding input frame through independently, preserving 0–16 channels
and raw volts. Display gain and offset never alter audio. Normal engine rules
still apply: cables delay one sample, stacked inputs are resolved before the
module runs, nonfinite cable samples are sanitized, and the selected mono audio
adapter reads channel zero. Scope does not add broadcasting between X and Y.

Each axis has 16 lanes of 256 minimum/maximum bins. Only active input lanes
contribute samples. Empty bins use internal `+infinity`/`-infinity` sentinels;
they represent no observation, not a measured zero. Partial-bin accumulation is
private. A completed bin replaces its previous contents, so a new capture
progressively replaces the prior complete history. Channel count changes retain
history; inactive lanes contribute no fabricated zeros.

| Parameter | Range/default | Meaning |
|---|---|---|
| `x_scale`, `y_scale` | Integer 0–8; 0 | Display span is `20 / 2^scale` V per screen |
| `x_offset`, `y_offset` | −10…10 V; 0 | Display offset only |
| `time` | 0.005…50 seconds; 0.5 | Requested screen duration |
| `xy` | 0 time / 1 XY; 0 | XY mode runs continuously |
| `trigger_enabled` | 0 free / 1 triggered; 0 | Wait after a complete time-mode capture |
| `threshold` | −10…10 V; 0 | Trigger low threshold |

Time is literal requested seconds, unlike Rack's exponential TIME parameter.
Each bin spans `ceil(time * sample_rate / 256)` samples, with a minimum of one.
The actual screen duration is `256 * samples_per_bin / sample_rate`; rounding
can lengthen the requested duration, especially at very short settings.

A fresh instance captures immediately. After a full triggered time-mode capture,
it waits indefinitely for a rising edge. External trigger normals to X only
when its actual channel count is zero. Each trigger lane has a Schmitt detector:
low is `<= threshold`, high is `>= threshold + 0.001 V`, and any lane can start
capture. Detectors reset on rearm; an initial high only arms the detector and
does not trigger. The rising sample contributes to the next capture. Free and
XY modes restart without waiting. There is no timeout fallback.

## State and live display

Same identity, kind, sample rate, acquisition parameters, and incoming routes
adopt complete history, partial accumulation, counters, and trigger state.
Display scale/offset edits preserve capture. Time, mode, trigger-enable,
threshold, identifiable incoming-route edits, a fresh identity, or a sample-rate
change start fresh. Initialize clears buffers and acquisition state. Parameters
are saved with the patch; captured buffers are runtime state.

Bypass preserves independent raw X/Y passthrough, including zero-channel frames,
and freezes acquisition and displayed state. Resuming continues the adopted
capture; acquisition counters and active channel counts do not advance while
bypassed. Display parameter edits still configure the panel.

The desktop transports snapshots from the actual active audio graph. Each
instance has a 30 Hz publication ceiling. The callback copies at most one
roughly 64 KiB snapshot per callback, selecting due instances round-robin.
Consumer lag may drop publications; this is bounded best-effort telemetry,
not a recording guarantee. No engine capture or snapshot-copy allocation occurs
in the callback. Graph/generation changes discard stale snapshots.

`inspect_scope` reads this live transported state, with explicit voice
selection and nullable min/max bins for empty observations. It does not compile
or render an offline replacement when live data is unavailable. `capture_signal` remains a separate offline-from-reset inspection path.

## Measurement controls and readouts

X/Y range selectors show the nine exact volts-per-screen choices: 20, 10, 5,
2.5, 1.25, 0.625, 0.3125, 0.15625 and 0.078125 V. They map to saved scale values
0–8; the patch format and gain law are unchanged. The scale applies to the plot
area, excluding its header and measurement footer. Rack and browser use the
same renderer.

Each axis shows raw-voltage minimum, maximum and peak-to-peak across its current
active lanes. Display gain and offset do not change these measurements. Only
finite ordered completed bins count; empty or inactive axes show dashes, and
MCP returns null. Zero peak-to-peak on a measured DC signal is a real zero.
Peak-to-peak subtraction uses f64 to keep opposite finite f32 extremes finite.

These are extrema of the stored display bins, which can contain old and new
sweeps during progressive replacement. They do not establish acquisition
start/end timestamps, RMS, or frequency. `observations` counts valid lane-bin
points, not raw samples. MCP exposes the selected voice and all active voices
separately. The snapshot publication frame identifies observation delivery;
`window_seconds` is the nominal rounded full-screen duration.

Zero guides use each axis's display offset and range. A time-mode threshold
guide appears only for enabled X-normalled triggering with active X channels;
offscreen thresholds are indicated without drawing a false in-range line.
The external trigger status uses actual input channel count, including a
connected zero-channel source normaling to X. External triggering has no X-axis
threshold guide. XY runs continuously and suppresses trigger guides. These
readouts describe the same published snapshot, including frozen bypass state.

## Validation and limits

Core tests cover raw independent 0/1/16-channel passthrough, peaks, partial bins,
retired/reactivated lanes, Schmitt edges and external normaling, free/XY/wait,
block partitioning, adoption/reset, bypass, stale handles, and allocation-free
capture/copy/adoption/initialization.

The [bounded reference report](benchmarks/scope-reference.json) pins Fundamental
commit `10dd0160c664770910e5584b7b00498cc48d9ddd` and records 67,500 compared
frames. An independent C++ bin driver uses genuine Rack SDK input and Schmitt
primitives; it does not execute the upstream Scope module or GUI. Binary-exact
seconds isolate fixed bin widths. The report excludes exponential TIME mapping,
graph latency, physical output bookkeeping, bypass/reset policy, and nonfinite
samples. Hash comparisons have theoretical collisions; Rust regressions also
inspect exact bins. This evidence supports the stated acquisition contract,
not full VCV display, timing, measurement, artwork, or hardware parity.
