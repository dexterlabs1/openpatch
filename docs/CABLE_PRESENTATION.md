# Cable presentation

Open **View** to adjust cable appearance while working in a patch. The menu stays
open during slider adjustments and scrolls in a small window. Drag, use the
keyboard, or edit a slider's percentage. Double-click its rail to reset it.

| Control | Range | Default and reset | Effect |
| --- | --- | --- | --- |
| Cable opacity | 0–100% | 100% | Fades cable bodies and shadows together. Plugs, signal lights and patching guidance retain their visibility. Hovering a connected port reveals its cables; an active dragged cable stays visible. |
| Cable tension | 0–100% | 50% | Changes the hanging curve from loose to taut with a slight gravity bow, keeping both ends attached to their jacks. |
| Plug light brightness | 0–100% | 100% | Dims signal lenses and their halos together. At zero, signal-light observation and publication stop; module lamps and Scope continue normally. |

These are host preferences, saved in the workspace's `.host-settings.json`.
Changes preview during a drag and save when it ends. Keyboard entry and resets
save immediately. New, templates, examples, open/import, revert, recovery and
history retain these preferences. They do not change patch content, dirty state,
undo history or audio output. Reset rack view changes navigation while retaining
cable appearance. Browser panels remain neutral previews.

Construction tip/ring accents and connection-validity guidance remain visible
while cable opacity is zero. Each actual plug's signal light is controlled
separately from cable opacity and room illumination. See [signal lights](PORT_LIGHTS.md).

## Behavior and reference

The [Rack manual](https://vcvrack.com/manual/MenuBar#view) documents cable opacity
and tension with 50% double-click resets. Its
[pinned startup settings](https://github.com/VCVRack/Rack/blob/8ea470f343058ade6f75f16d503a1eb330e1700c/src/settings.cpp#L31-L34)
start opacity at 50% and tension at 100%. OpenPatch retains its existing fully
visible cables and half-tension curves by default. Its own cubic artwork is
unchanged at 50%; all other tensions use the same curve for committed routes,
drafts, stacked fan-out, construction prefixes/tips and decorative hover tests.
At 100% the cubic is straight; at 0% the hanging curve is longer. Short, reversed
and coincident endpoints remain finite. The route and electrical cable delay
are independent of this geometry.

Plug brightness with observation disabled at zero is an OpenPatch extension.
Rack's [pinned Light bloom control](https://github.com/VCVRack/Rack/blob/8ea470f343058ade6f75f16d503a1eb330e1700c/src/app/MenuBar.cpp#L204-L335)
controls halos; its detector continues independently of that display setting.
No Rack graphics are imported.

## Observation lifetime and validation

Positive brightness changes scale visible emission and preserve the detector's
signed history. Crossing zero clears the displayed measurement. Re-enabling
waits for an actual acquisition on the existing seven-sample audio phase and a
fresh packet, including rapid off/on changes between callbacks. A control epoch
prevents cached or unread packets from a previous enable period reappearing.
Startup/reopen applies the saved intent before the stream starts; queued graph
replacement, Scope rebuilds and crossfades respect the current intent.

The observer's buffers and endpoint registry remain prepared off the audio
thread. Toggling does not allocate, deallocate, lock, resolve strings, recompile
the patch, add consumers or change signal frames in the callback. The sample
clock continues while observation is off. The global publication limit remains
30 Hz of audio time, with at most one packet per callback.

Focused regressions use actual egui input for drag preview/save, keyboard edits,
reset and popup lifetime, plus project lifecycle persistence. Drawing tests check
all shadow/body alpha, independent cap/emission colors, tension extremes across
five zooms and stacked/reversed/short/looping routes, draft visibility and animated
prefix geometry. Actual headless callback tests cover startup off, rapid toggles,
fresh acquisition, old queued intent, both engine backends, Scope rebuilds,
crossfades, heap activity and identical audio/Scope samples. These tests do not
establish physical device latency or native Mac input qualification.

The [release callback profile](benchmarks/cable-presentation-callback-profile.json)
records all trials, environment and source hashes for a 65-module, sixteen-voice,
128-port fixture on an Apple M1 Max at 48 kHz. Switching observation off at runtime
measured a median 1.168 ms per 256-frame buffer; startup off measured 1.165 ms,
with a paired median difference of 2.09 µs. Both observers stayed unacquired.
The separate enabled/disabled comparison measured a paired median 124 µs of
observation work per buffer. These whole-graph timings include audio processing
and ordinary control synchronization; they exclude device scheduling and UI work.
