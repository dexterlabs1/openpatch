# MIDI performance and stereo audio

Choose **File → MIDI keyboard voice**, then **Play**. Open **View → Performance
keyboard** to play the on-screen piano or the two computer-keyboard rows.
The default range is **C4–G6** (MIDI 60–91), with 32 pitches and five overlapping
key aliases. This replaces the earlier 13-note A-row layout.

## Computer-keyboard performance

The US-position map follows the [pinned Rack 2 keyboard driver](https://github.com/VCVRack/Rack/blob/8ea470f343058ade6f75f16d503a1eb330e1700c/src/keyboard.cpp):

```text
Semitone offsets  0–16: Z S X D C V G B H N J M , L . ; /
Semitone offsets 12–31: Q 2 W 3 E R 5 T 6 Y 7 U I 9 O 0 P [ = ]
```

Backtick lowers the base by an octave; `1` raises it. The buttons provide the
same shifts. The base ranges from MIDI 0 to 108 in 12-semitone steps; pitches
above MIDI 127 are unavailable and dimmed. Octave changes affect subsequent
presses: each held source retains its original pitch until released. Physical
key positions are used when the platform supplies them, with logical-key
fallback. Native layout coverage beyond US QWERTY remains unqualified; Rack's
separate numpad and mouse-position MIDI drivers remain future work.

Notes use MIDI channel 1 and velocity 127. Initial, unmodified key presses are
processed in event order after the host's editors and dialogs handle input;
OS repeat does not retrigger notes or octave shifts. Text entry, modal windows,
focus loss, modifier shortcuts, stopping playback, and hiding the keyboard
release its notes. Returning focus does not restart a still-held key. If the OS
omits a key release while another window is focused, the first tap of that key
re-arms it; the next press plays. This conservative behavior also applies to
held octave keys, because the current window backend loses OS-repeat identity
on focus changes. Releases consumed by an editor during continuous focus are
reconciled without requiring an extra tap.

Overlapping aliases and the mouse share ownership of each pitch. For example,
holding both comma and Q sounds one C5; releasing only comma sustains Q's note.
The final owner emits note-off. This deliberately improves on Rack's separate
on/off events for overlapping aliases. Octave changes preserve that ownership,
and mouse dragging releases the previous key as it moves to the next. Hardware
MIDI remains an independent producer; its repeated-note policy is described
below. **All notes off** resets performance state without editing the patch.

Headless native-input tests cover all 37 bindings, ordered quick taps, aliases,
physical-key fallback, modifiers, repeats, octave limits and held notes, mouse
ownership, focus/modal guards, and unchanged patch/history state. Linux native
screenshots cover the expanded keyboard at the minimum supported window size;
these checks do not qualify physical keyboard rollover or audio-device latency.

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
