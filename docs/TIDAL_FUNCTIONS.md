# Tidal Functions

`tidal_functions` is a native Rust adaptation of the Tides 2018 portable
four-output generator and clock extractor. It has its own OpenPatch identity,
shared rack/browser panel and document/MCP controls. It is not a binary Rack
plugin or a calibrated model of the analog hardware.

Five rotaries set frequency, slope, shape, smoothness and shift. Select AD,
looping or AR operation; Gates, Amplitude, Slope/Phase or Frequency relationships;
and control/audio processing. A separate nine-position clock ratio spans 1/16–16.
The default Amplitude/Shift configuration leaves output 1 audible. Amplitude
scans overlapping channel gains, so other outputs can legitimately be silent.

Seven inputs accept pitch, four shape CVs, trigger and clock. They read channel
zero; outputs 1–4 are mono. Pitch is 1 V/octave. Each shape CV adds voltage/5
to the normalized knob, clamped to 0–1. The free frequency control spans
0.001–20,000 Hz; effective frequency is limited to a quarter of the host rate.
Clock overrides free frequency even with trigger patched. With both connected,
clock still supplies frequency while trigger controls the generator; only the
external extracted ramp is withheld. Gates mode has shaped, raw, end-of-attack
and end-of-release outputs; these are not four interchangeable logic gates.

Source output floats pass through at unity volts. Nominal looping waveforms
span ±5 V, AD/AR functions 0–8 V and end-stage gates 8 V; shaping and mode
interpolation determine the actual samples. The four paired lamps show actual
generated polarity/magnitude, including when no output consumer is connected.
The panel's relationship guide describes configuration, not a live scope.

## Native timing and lifecycle

The adapter collects six frames, samples scalar CV on the last frame, then emits
the generated block with six frames of delay. Host callback sizes do not change
that cadence. Native Schmitt thresholds are 0.1/1 V. First sampled trigger/clock
levels prime their detectors without inventing an insertion edge. Suppressing
the extractor's first RISING flag prevents its source zero-duration clock-start
failure before processing; this is an explicit safety adaptation.

Trigger/clock connection changes discard partial/queued blocks and restart the
six-frame pipeline while preserving generator phase. Clock connection changes
reset the extractor. Ordinary CV connection changes preserve the pipeline.
Same-rate graph adoption retains all runtime state and uses receiving controls;
output-mode changes reset source smoothing filters only. Initialize and rate
changes create fresh state. Bypass emits mono zero and freezes processing.
Saved patches/presets contain authored controls, not transient waveform state.

## Source and bounded qualification

Source: `pichenettes/eurorack` at
`f4cfa7697095a28d292bff758beb51deb49b5a86`, with its stmlib gitlink
`e3bd7c9cc00e4364166f9905c0509b6ffd0535ec`. Complete MIT notices are retained
in the adapted files, Core third-party notice and bundled
`Tides-algorithms.txt`. The [reference runner](../scripts/tides2_reference/README.md)
records exact source hashes, compiler options and immutable input/output traces.

On macOS ARM, the production Rust raw kernels matched all **129,840 frames in
34 finite reference cases bit for bit**, including four outputs, frequency,
gate/clock flags and extracted ramp. Both implementations reject the retained
frame-zero control-clock negative case. Coverage includes all ramp/output
combinations, clock ratios, audio AR clocking, control changes and resets, nonzero Amplitude gain scans, plus
the declared 4/6/8-sample cadences. It is a bounded portable-source result;
other compiler/platform behavior, firmware ADC/DAC scheduling and physical
hardware accuracy are not established.

Separate native tests cover 648 clock-start rate/mode/ratio combinations,
clock-plus-trigger priority, CV mapping, reconnects, output masks, graph adoption,
irregular host blocks, bypass, initialization, fresh reopen and allocation-free
callback work. Actual Engine samples also match retained C++ fixture voltages
after the explicit six-frame native delay. Shared panel geometry, drag/history,
measured lamps and library categories have focused tests; MCP exercises capture,
parameter editing, undo/redo, presets, invalid edits and saved reopen.
