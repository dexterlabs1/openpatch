# MIDI performance and stereo audio

Choose **File → MIDI keyboard voice**, then **Play**. Open the performance
keyboard from **View** to play with the mouse or `A W S E D F T G Y H U J K`.
The octave buttons release held notes before transposing. Computer-keyboard
notes use MIDI channel 1 and velocity 100. Text entry, modal windows, focus loss,
and hiding the keyboard release its notes; OS key repeat does not retrigger.

**Audio / MIDI** selects the output device and one hardware MIDI input. Device
choices are saved separately from patches in `.host-settings.json` in the
workspace. The output's sample rate follows the device; changing outputs pauses
playback. Hardware input uses midir's CoreMIDI, Windows MIDI, or ALSA backend.
No external controller is required for the on-screen keyboard.

## MIDI → CV

Engine-v2 kind `midi_cv` provides 1–16 fixed voice lanes (default 4). Pitch is
`(note − 60) / 12` V, with C4 at 0 V; use the VCO's 261.6256 Hz base frequency.
Gate is 0/10 V, velocity and aftertouch are 0–10 V, pitch wheel is −5…5 V,
and modulation wheel is 0–10 V. Pitch wheel is a separate output and does not
silently change pitch. Note releases retain pitch and velocity; fixed lane counts
allow downstream envelope and filter release tails to finish.

The twelve outputs are `pitch`, `gate`, `velocity`, `retrigger`, `aftertouch`,
`pitchwheel`, `modwheel`, `clock`, `clock_div`, `start`, `stop`, and `continue`.
Retrigger and clock/transport pulses last 1 ms. `clock_div` defaults to one pulse
per 24 incoming clocks, beginning on the first clock after reset or Start.
`channel=0` accepts all wire channels; 1–16 selects wire channel 0–15 respectively.
`allocation` selects Rotate (0), Reuse (1), or Reset (2). One voice uses last-note
priority with held-note fallback. Rotate searches circularly for a free lane,
then steals the next lane when full. Reuse selects a previously used lane for the
same note and wire channel, otherwise the lowest free lane, stealing lane 1 when
full. Reset selects the lowest free lane, steals lane 1 when full, and compacts
active lanes on release. Repeated note-ons are counted; a reused lane stays held
until their note-offs balance. Other duplicate voices release in lane order.
These are OpenPatch's contracts, not a claim of complete Rack MIDI-CV equivalence.

Sustain CC64, modulation CC1, all sound off CC120, controller reset CC121,
all notes off CC123, pitch bend, channel pressure, timing clock, and transport
events are handled. Aftertouch follows channel pressure; poly pressure messages
are accepted and intentionally ignored. CC123 releases pressed notes while
respecting sustain. CC120 clears that wire channel's held notes, sustain,
controllers, gates, and retriggers. CC121 resets its controllers and releases
sustained notes. Voices, channel filtering, and allocation
changes reset held notes. Patch edits otherwise preserve compatible MIDI state.
Notes themselves never enter patch history or trigger graph compilation.

Pause/resume, input disconnection, reconnect, and queue overflow reset the entire
performance state, including computer-keyboard notes. Play held notes again after
a reset. The **All notes off** button provides the same recovery. Independent UI
and hardware queues each have one producer; reset generations invalidate old
queued events. Hardware timestamps map to the audio sample clock: late events
apply at the next processed sample, and future events are capped at 100 ms.
Events at the same sample retain producer order, with hardware before UI ties.
Events reach both graphs during a live edit crossfade.

## Stereo output

Engine-v2 kind `audio_stereo` has `left` and `right` inputs and a two-channel
`out`. Each input sums its polyphonic voices at unity gain, then converts volts
to digital audio with 10 V = 1.0 and `gain` (0–2, default 1). An unconnected or
zero-channel Right input follows Left. The bundled keyboard voice uses this
normalization; connect a distinct signal to Right to create stereo.

Only selecting **STEREO AUDIO → out** defines a stereo device/WAV sink. Selecting
another polyphonic output retains the previous channel-0 mono monitor behavior.
Stereo maps to the first two hardware channels; additional device channels are
silent. A mono device receives `(L + R) / 2`. Legacy mono patches still duplicate
across hardware channels. The visualizer observes the L/R average; peak metering
uses the larger channel magnitude. File output is interleaved 16-bit PCM with one
or two channels. Audio input, more than two independent hardware output channels,
explicit sample-rate/buffer controls, and MIDI output/mapping remain future work.

## Reproducible agent performances

Both MCP `render_audio` and `analyze_signal` accept an optional `midi_events`
array. Events use absolute sample frames and zero-based wire channels:

```json
[
  {"frame":0,"event":{"type":"note_on","channel":0,"note":60,"velocity":100}},
  {"frame":24000,"event":{"type":"note_off","channel":0,"note":60,"velocity":0}}
]
```

The events must be sorted by frame, lie within the requested render, and contain
at most 65,536 entries. Frame numbers must be strictly below the rounded render
frame count; a zero-frame render accepts only an empty trace. Equal-frame events
run in supplied order, immediately
before that sample. Invalid fields, channel numbers, or timing fail before a WAV
is written. Offline rendering starts from reset state and never changes live
performance, patch revision, or history. This is offline performance control;
MCP does not yet inject live notes into an existing window.

```sh
cargo run --release -p openpatch -- render examples/midi-keyboard-voice.json \
  midi-phrase.wav 2 --midi examples/midi-phrase.events.json
```

The example trace is authored at 48 kHz. Event frames do not automatically retime
when the patch sample rate changes. Offline traces and scheduler tests do not
qualify actual MIDI hardware latency or device reconnect behavior; those still
need hands-on macOS testing.

References: [VCV Core manual](https://vcvrack.com/manual/Core) for the target
functional roles and [midir API](https://docs.rs/midir/0.11.1/midir/struct.MidiInput.html)
for host input integration. OpenPatch's implementation is original Rust.
