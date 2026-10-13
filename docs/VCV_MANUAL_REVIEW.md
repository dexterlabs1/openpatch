# VCV manual review and acceptance priorities

Reviewed 2026-10-13 against the [official Rack manual](https://vcvrack.com/manual/)
and OpenPatch source. This supplements the [roadmap](ROADMAP.md) and
[module coverage](MODULE_COVERAGE.md); it does not certify parity.
The reference is Rack 2 Free standalone. Module-specific manuals and pinned
implementations are still needed where the general manual leaves behavior open.

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
| 2 | Cmd/Ctrl output cable stacking and plug dragging. [Getting started](https://vcvrack.com/manual/GettingStarted) | `rack.rs` supports cable editing and Shift-drop stacking. Qualify the documented modifier workflow, occupied ports, stacked-plug selection, detach/cancel, and undo using actual pointer events. |
| 3 | 25–400% zoom, keyboard zoom/pan, and module-position locking. [Menu bar](https://vcvrack.com/manual/MenuBar), [Key commands](https://vcvrack.com/manual/KeyCommands) | Current rack zoom is 40–180%; Cmd/Ctrl+0 and mouse navigation exist. Extend the range and shortcuts, add a movement lock, and test cursor anchoring and text-entry/keyboard conflicts. |
| 4 | Hovered-module deletion and clearing patch cables. [Key commands](https://vcvrack.com/manual/KeyCommands), [Menu bar](https://vcvrack.com/manual/MenuBar) | Current deletion requires selection; `app.rs` lacks Clear cables. Add a hovered fallback and a single shared-history clear action. Preserve selection semantics and reject stale MCP/UI edits. |
| 5 | Reusable user template for New. [Menu bar](https://vcvrack.com/manual/MenuBar) | `app.rs` creates an empty v2 patch and offers example patches. Add Save template and guarded template loading without losing dirty work or recovery data. |
| 6 | Broader computer-keyboard playing range. [Getting started](https://vcvrack.com/manual/GettingStarted) | `keyboard.rs` currently maps 13 notes. Expand the layout and verify focus loss, octave shifts, held-note release, and shortcut conflicts. |

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

Prioritize the first two interaction rows after the current performance batch,
then navigation/templates and bounded hardware-input work. Each delivered row
needs source changes, focused behavioral tests, shared undo/MCP checks where
applicable, and native workflow evidence. Keep unresolved or untested behavior
marked partial. The general manual contains unfinished sections and cannot
replace version-pinned module tests or hardware measurements.
