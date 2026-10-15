# Signal lights on cable plugs

Connected plugs display the signal measured at their own port. Positive mono
signals emit green, negative mono signals red, and polyphonic signals blue.
Fast bipolar audio can blend the mono colors through the detector's decay.
Blue indicates measured polyphonic signal energy, not merely a configured voice
count. Silent polyphonic signals remain dark after previous activity decays.

Each input observes its actual summed input buffer. For example, +5 V and −5 V
stacked into one input light the two source outputs but cancel at the input.
Only the top plug at each endpoint emits; changing a cable's body color does
not change the light's meaning. A loose dragged end never shows activity.
Attached ends in a dragged bundle retain their own available measurements.

**View → Plug light brightness** adjusts the lenses and halos together. At zero,
their signal observation and publication stop. Re-enabling waits for a fresh
audio observation. Cable opacity and tension are independent controls; see
[cable presentation](CABLE_PRESENTATION.md).

Emission remains visible at zero room brightness. Hover, patching validity,
selection and output-monitor rings remain separate decorations. A VCA's gain
lamps show gain: they can remain bright with no signal while its plugs are dark.
Port tooltips distinguish measured channel count, invalid signal and unavailable
measurement. Master mute and transport pause do not imply an inactive internal
signal graph; stopping the audio stream makes its measurements unavailable.

## Detector and transport

An opt-in observer reads existing signal buffers once every seven audio frames,
starting at frame zero. It does not add consumers, scope taps, demand masks or
write signal buffers. Mono targets are −V/10 and +V/10; polyphonic magnitude is
sqrt(sum(V²))/10 without dividing by voice count. Rises are immediate; falls use
the reference's Euler smoothing coefficient of 30 with elapsed seven-sample
audio time. Signed and over-range history is retained; display conversion clamps
each RGB channel separately. Invalid active samples or arithmetic overflow reset
only the visual observer and suppress its emission.

OpenPatch preserves native signal units. Some older modules use normalized audio
rather than voltages; their lights can consequently be dimmer. No guessed unit
conversion changes the sound or signal graph.

Storage is prepared off the audio thread for at most 2,048 unique connected
endpoints. A fixed triple-buffer payload publishes the active graph at most
30 times per second of audio, with at most one publication per callback.
The UI builds its immutable map off the callback and rejects mismatched graph
tokens/revisions, unacquired states and packets at least 250 ms old. Repeated
reads do not renew freshness. Graph replacement preserves compatible detector
phase/history; changed routes and recreated endpoints reacquire their signal.

## Reference and validation boundaries

Behavior was reviewed against Rack commit
[`8ea470f`](https://github.com/VCVRack/Rack/blob/8ea470f343058ade6f75f16d503a1eb330e1700c/src/engine/Module.cpp#L294),
including its actual `Port_step`, `Light` and port magnitude arithmetic.
The [reproducible reference runner](../scripts/port_lights_reference/README.md)
uses genuine Rack SDK 2.6.6 `Module::doProcess`, whose relevant source blocks and
headers match the reviewed pin. Its frozen finite fixtures passed 54 cases across
44.1, 48 and 96 kHz: 221,184 frames and 1,327,104 raw RGB comparisons, with maximum
absolute error 4.77e−7 against a fixed absolute/relative tolerance of 2e−6.
See the [machine-readable report](benchmarks/port-lights-reference.json).

Release measurements on an Apple M1 Max at 48 kHz used actual sixteen-channel
signals, alternating enabled/disabled trials and identical output checks:

| Connected endpoints | Extra median time per sample | Extra share of one CPU core |
| --- | --- | --- |
| 16 | 50 ns | 0.24% |
| 256 | 974 ns | 4.68% |
| 2,048 | 7,900 ns | 37.9% |

The maximum synthetic graph already exceeds the single-core real-time budget
with observation disabled: 26.2 µs per sample, versus a 20.8 µs deadline. Enabling
observation raises it to 34.1 µs. This stress case establishes a measured limit,
not a promise that every graph within the cable count limit runs in real time.
The separate actual callback fixture, with 65 modules and 128 connected endpoints,
adds a median 130 µs per 256-frame buffer, or 2.44% of one core. These measurements
exclude physical device scheduling and UI drawing. Raw trials, environment and
qualified source hashes are in the [core profile](benchmarks/port-lights-core-profile.json)
and [callback profile](benchmarks/port-lights-callback-profile.json).

This qualifies finite numerical behavior, not analog hardware, physical device
latency or performance superiority. Public-engine, callback and headless drawing
tests cover integration separately. OpenPatch uses its own plug artwork; no Rack
SVGs or other separately licensed graphics are imported.
