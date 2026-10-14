# VCV manual review and acceptance priorities

Reviewed 2026-10-13 against the [official Rack manual](https://vcvrack.com/manual/)
and OpenPatch source. This supplements the [roadmap](ROADMAP.md) and
[module coverage](MODULE_COVERAGE.md); it does not certify parity.
The reference is Rack 2 Free standalone. Module-specific manuals and pinned
implementations are still needed where the general manual leaves behavior open.

## Review coverage and DSP follow-up (2026-10-14)

This is a selective manual/source review, not a page-by-page certification of the
whole manual. The DSP page has now been read end to end; its linked textbooks,
papers, and external sites have not all been reviewed. Review coverage and
implementation acceptance are separate claims.

The [DSP page](https://vcvrack.com/manual/DSP) discusses bandlimiting,
nonlinear aliasing, filter responses, circuit equations and numerical integration,
and measuring performance before and after optimization. Those principles inform
the following OpenPatch acceptance work; they do not establish that it is done.

| Area | Current evidence | Required next evidence |
| --- | --- | --- |
| Oscillator spectra | Ideal VCO has a BLEP waveform-error test. MULTI VCO has direct voltage comparisons and an alias-energy lower-bound probe; derivative discontinuities remain. | Sweep pitch, PWM, FM and sync at 44.1/48/96 kHz against a properly filtered high-rate reference. Separate legitimate harmonics from aliases and declare error thresholds before optimization. |
| Nonlinear circuits | Ladder/OTA use 8× interpolation and decimation; ladder has analytic small-signal and independent RK4 comparisons. | Broader amplitude, resonance, two-tone IMD and modulation sweeps; record alias power, solver failures and latency. Hardware accuracy additionally requires identified hardware and measured captures. |
| DRUM and SVF | Original functional algorithms. Metallic hat squares remain at host rate. The [SVF source integration](SVF_FILTER.md) adds 16× multistage drive and matched 76-sample signal/CV delay while retaining host-rate TPT filtering. Its [frozen prototype](benchmarks/svf-antialias/multistage/README.md) passes 576 bounded single-tone alias/magnitude cases; this is separate from integration and device qualification. | Verify integrated-source spectra, voice/CV lifecycle, parameter transitions, latency in feedback/parallel paths and callback cost. Initial prototype 96 kHz/16-voice p99 exceeded budget; repeats do not erase that result. Broader amplitudes, two-tone IMD and rapid modulation remain open. No hardware or VCV equivalence is established. |
| Studio effects | Delay/reverb/compressor have bounded processing and response tests. | Reference impulse/decay and dynamics curves over rates/settings, modulation sidebands, stereo behavior and parameter-transition checks. No Surge/Plateau/Pressor identity claim. |
| Performance | Circuit optimization compares preserved arithmetic; callback diagnostics and offline benchmarks exist. | Matched workloads, voice counts, rates, latency and quality; callback deadline tails and graph-swap cost on devices. Changing a ladder into an SVF is a fidelity tradeoff, not an equal-quality speedup. |

Each acceptance report must name the module/source version, stimuli, sample rate,
reference method, thresholds, measured results and remaining limitations.
Passing structural or finite-output tests alone does not satisfy these gates.
See [analog model evidence](ANALOG_MODELS.md), [oscillator evidence](AUDIO_OSCILLATOR.md)
and [studio module limits](SCORE_AND_STUDIO.md).

## What the existing foundation supports

Rack documents voltage signals, one sample of cable delay, primary-input voice
counts, secondary mono broadcast, and zero-padding of short polyphonic controls.
OpenPatch v2 implements these contracts in `poly.rs`, with tests described in
[ENGINE_V2.md](ENGINE_V2.md). The legacy v1 engine has intentionally different
timing and units. [Voltage standards](https://vcvrack.com/manual/VoltageStandards).

Zero-channel signals can act unpatched, and a cable can carry up to 16 channels.
OpenPatch supports this representation; individual module policies and the
hardware sinks still need separate qualification.
[Polyphony](https://vcvrack.com/manual/Polyphony).

Our circuit models retain their documented numerical and spectral tests.
Bandlimiting, nonlinear processing, response accuracy, and realtime cost must
be assessed separately. The manual's DSP guidance does not validate a specific
oversampling factor, solver, or commercial-hardware model.
[DSP](https://vcvrack.com/manual/DSP), [OpenPatch model evidence](ANALOG_MODELS.md).

## Native interaction priorities

These track remaining acceptance work as well as delivered subsets. Source references
are relative to `crates/openpatch-desktop/src/`.

| Priority | Reference behavior | Current evidence and acceptance work |
|---|---|---|
| 1 | Right-click parameter entry, including expressions and note values. [Key commands](https://vcvrack.com/manual/KeyCommands) | Both entry paths share a bounded expression/note evaluator, conversions, and revision checks. Pinned evaluator comparisons and intentional precedence corrections are documented; whole-application parity is not claimed. [Expression contract](PARAMETER_EXPRESSIONS.md), [editing contract](RACK_EDITING.md#exact-parameter-values). |
| 2 | Cable pickup, creation, deletion and cloning. [Getting started](https://vcvrack.com/manual/GettingStarted) | Implemented top-plug movement, Cmd/Ctrl creation from either end, Shift-press top deletion, Cmd/Ctrl+Shift cloning with persisted color, invalid/empty moved-drop deletion, and v2 occupied-input sibling preservation. Pointer, history, persistence and MCP tests cover the gesture contract. Both port menus support selected pickup/clone, atomic move-all/delete-all and palette/custom RGB colours. Independent endpoint ordering now persists with guarded promotion, exact undo, and unchanged DSP cable-array order. Physical native input qualification remains open. [Cable gestures](RACK_EDITING.md#cable-gestures). |
| 3 | 25–400% zoom, keyboard zoom/pan, and module-position locking. [Menu bar](https://vcvrack.com/manual/MenuBar), [Key commands](https://vcvrack.com/manual/KeyCommands) | Implemented 25–400% zoom, centered keyboard stepping/100% reset, pointer-anchored wheel/pinch and extra mouse buttons, held-arrow panning with modifier speeds, and a persistent mouse-position lock. Navigation has focus/text/dialog/gesture guards; project and recovery retain the expanded range. Arrow speed is elapsed-time based. See [rack navigation](RACK_EDITING.md#rack-navigation). |
| 4 | Hovered-module deletion and clearing patch cables. [Key commands](https://vcvrack.com/manual/KeyCommands), [Menu bar](https://vcvrack.com/manual/MenuBar) | Implemented selection-first Delete/Backspace with hovered fallback and OS repeat, plus Edit → Clear cables backed by atomic MCP `clear_cables`. Captured revisions reject stale UI edits; text/focus/gesture guards apply. Tests cover undo, native persistence, full cable capacity, and cable retirement without node-state resets. [Editing contract](RACK_EDITING.md#clear-cables). |
| 5 | Reusable user template for New. [Menu bar](https://vcvrack.com/manual/MenuBar) | Implemented: File → Save template captures a confirmed native snapshot; New/Cmd-Ctrl-N loads the user template or factory voice, with guarded dirty-work handling, clean untitled identity, and fresh undo history. Startup resolves explicit files before recovery/template and MCP exposure. See [Projects](PROJECTS.md). |
| 6 | Broader computer-keyboard playing range. [Getting started](https://vcvrack.com/manual/GettingStarted) | Implemented the pinned 37-key QWERTY map (32 pitches), backtick/1 octave changes, original-pitch releases, modifier/repeat/focus guards, and a 32-note mouse keyboard. Shared alias/mouse ownership avoids premature note-off. Numpad driver and physical-device qualification remain open. [Performance contract](MIDI_AUDIO.md#computer-keyboard-performance). |

Vertical knob dragging, fine/coarse modifiers, double-click reset, module-library
entry, middle-button panning, and native save/recovery already have implementations.
They still need workflow qualification alongside the missing interactions.

## Host and module requirements that remain substantive

Core Audio bridges hardware in both directions and exposes device rate/block
configuration; MIDI includes dedicated controller mapping and output utilities.
OpenPatch currently has selectable stereo output and MIDI input, but lacks
hardware audio input, independent multichannel device outputs, MIDI output, and
controller-to-parameter mapping. Its engine follows the selected device rate.
Acceptance should include channel routing, device reconnection, bounded queues,
saved configuration, and physical loopback checks on macOS.
[Core manual](https://vcvrack.com/manual/Core), [current I/O contract](MIDI_AUDIO.md).

Rack documents per-module CPU metering and configurable engine threads.
OpenPatch currently measures aggregate callback budgets; neither that diagnostic
nor faster offline rendering establishes equivalent scheduling or device stability.
[Menu bar](https://vcvrack.com/manual/MenuBar), [measurements](PERFORMANCE.md).

Plugin support also involves persistence and host services, including custom
module data and expander communication. Keep Rust extensions, original module
implementations, `.vcv` import, and existing C++ binary loading as distinct
deliverables. Similar DSP or a matching panel does not establish any of them.
[Plugin guide](https://vcvrack.com/manual/PluginGuide).

## How to use this review

The Core manual and implementation are not identical specifications. For example,
the manual describes channel aftertouch, while Rack's pinned parser also handles
matching-note key pressure. OpenPatch currently ignores key pressure. Its three
allocation modes also need differential tests for full voice allocation and
release behavior; matching mode names do not prove matching policies.
[Pinned MIDI parser, commit 8ea470f](https://github.com/VCVRack/Rack/blob/8ea470f343058ade6f75f16d503a1eb330e1700c/include/dsp/midi.hpp).

The same audit identified missing MPE, bend-to-pitch range controls, wheel
smoothing, and a MIDI-specific initialization reset. Verify these against a
version-pinned executable before changing established OpenPatch patch behavior.
Test pressure routing, wheel channel counts, initialization with held notes,
and saved controller settings as a dedicated MIDI-CV parity batch.
[Pinned MIDI-CV module](https://github.com/VCVRack/Rack/blob/8ea470f343058ade6f75f16d503a1eb330e1700c/src/core/MIDI_CV.cpp).

Parameter expressions also need a precise compatibility contract. We compiled
Rack's pinned tinyexpr dependency with its recorded flags and checked operator
precedence. It evaluates `-2^2` as `-4`, `2^3^2` as `512`, and `2^-2` as `0.25`.
The same evaluator incorrectly produces `-4` for `(-2)^2`; its upstream tests
explicitly omit known problematic parentheses cases. These are isolated
dependency results, not a test of the complete Rack application. OpenPatch's
parser uses conventional exponent grouping and documents these corrections alongside supported syntax.
[Reproducible reference results](benchmarks/parameter-expression-reference.json),
[pinned dependency tests](https://github.com/codeplea/tinyexpr/blob/4e8cc0067a1e2378faae23eb2dfdd21e9e9907c2/smoke.c#L575-L594).

Next interaction work includes palette settings, keyboard-driver/device
qualification, and bounded hardware-input work. Each delivered row
needs source changes, focused behavioral tests, shared undo/MCP checks where
applicable, and native workflow evidence. Keep unresolved or untested behavior
marked partial. The general manual contains unfinished sections and cannot
replace version-pinned module tests or hardware measurements.
