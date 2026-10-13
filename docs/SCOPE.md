# Signal scope

Open **Scope** in the toolbar or **View → Signal scope**. Right-click any output
jack and choose **Inspect in scope X/Y** to probe it without adding a cable or
changing the audio output. Each lane has an independent source and voice (1–16).

This is a native measurement dock. The older GLSL **Visuals → Scope** is an
artistic master-output visualizer; it is not a calibrated voltage instrument.

## Reading a signal

- **Time:** ten horizontal divisions and eight vertical divisions. Choose the
  time per division, each lane's vertical scale, and its center value. The
  timebase spans 2 ms–2 s across the whole display. Min/max strokes preserve
  narrow peaks within each pixel column; connecting lines show bin means.
- **Units:** `V` is module voltage, `FS` is normalized digital amplitude
  (`1 FS` is digital full scale), and `norm` is a legacy normalized signal.
  Units come from the module catalog, not the shape of its waveform. The scope
  does not clip or normalize the signal. Audio sink outputs already apply their
  module gain and voltage-to-digital conversion.
- **Statistics:** range, DC mean, and RMS use every active sample in the displayed
  capture. A missing voice is inactive, not a zero-volt measurement. There is no
  automatic mono broadcast to another voice. Missing samples leave trace gaps.
- **Trigger:** Auto waits for an edge but produces a free-running capture if
  none occurs; Normal holds its last capture until the next edge; Free does not
  wait for an edge. Choose X/Y, rising/falling, and a raw-unit threshold. The
  detector arms 0.01 units below the threshold and fires 0.01 units above it
  (reversed for falling). A triggered capture has 25% pre-trigger history.
  Display scale and center do not affect the trigger or statistics.
- **Freeze:** holds the displayed capture while transport continues. Resuming
  starts a fresh acquisition. Changing the sources, timebase/trigger settings,
  audio device, or compiled graph clears the old capture so its labels cannot
  silently describe different data.
- **X/Y:** plots paired X and Y samples as a phase portrait. X has ten divisions
  and Y has eight. It draws at most 2,048 paired points; this view is decimated,
  unlike the peak-preserving Time view. The same raw statistics remain visible.

The engine continues computing while speaker playback is paused. The scope
therefore remains useful while the instrument is muted. It needs a working
audio engine; **Retry audio** reconnects an unavailable device. Source identity
is node ID, kind, and output name. Removed or differently typed sources become
inactive on graph submission. **Reconnect** explicitly binds the current choices
again after a source is replaced.

## Sampling contract

Live taps read the selected module outputs in the **same processing pass** as
the audible graph, before host master gain, clipping, device-channel mapping,
and graph crossfade. They do not change `Patch.output`, cable delay, DSP state,
voice count, or the graph's sound. Version 1 retains its block/DAG timing;
version 2 retains one sample per cable. Only the new active graph is probed during
an edit crossfade, not its mixture with the retiring graph.

Compilation resolves endpoint names off the audio thread. Engine and probe
handles travel together through the bounded graph queue. Handles are bound to
the compiled engine that produced them, so an old index cannot probe a different
node in another engine. Each successful graph submission or probe selection has
a new generation; displayed captures show their generation. A generation is a
local audio configuration counter, **not the patch's MCP revision**.

The callback sends raw, timestamped blocks of 64 paired samples through a
128-packet queue. It does not allocate, lock, perform I/O, or wait for the UI.
At 192 kHz that queue holds about 42.7 ms. A busy UI can lose packets; it clears
acquisition on the next detected frame gap and increments the gap count instead
of joining discontinuous data. Old-generation packets are discarded. UI draining
is bounded to 128 packets per update, including while frozen. Capture history
is bounded to 384,000 frames (2 seconds at 192 kHz). These bounds are not a promise
of uninterrupted acquisition under arbitrary system load.

## MCP capture

`capture_signal` provides bounded waveform bins and exact active-sample
statistics to Claude and Codex. It renders a coherent patch snapshot **offline
from reset**, with optional sample-timed MIDI events. It is not a recording of
the live scope, and does not mutate transport, history, or the selected output.
See [MCP tools and example](MCP.md).

## Validation and remaining parity

Headless tests cover taps without audible-output changes, named outputs,
polyphonic inactivity, old-handle rejection, v1/v2 timing, packet offsets across
MIDI subspans, generation swaps, queue overflow, allocation/deallocation checks,
trigger placement, gap resets, and known waveform statistics. Linux CI renders
the native scope using a labeled offline calibration fixture generated by the
engine. It does not qualify physical macOS audio or input interaction.

This is partial progress toward [VCV Scope](https://vcvrack.com/Free#vcv-scope),
not a complete equivalent. Rack-mounted instances, patchable external trigger,
all-voice overlays, persisted scope settings, live MCP trace subscriptions,
calibrated spectrum/pitch measurement, and hardware/device qualification remain.
Scope settings and captured buffers are session-only and do not make the patch
file dirty. There is no claim of compatibility with VCV's exact display timing
or trigger implementation.
