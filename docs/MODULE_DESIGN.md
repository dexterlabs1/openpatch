# Module design language

The selected direction is **01 — Signal Atlas** from the five October 16, 2026
module concepts: warm ivory, strong ink typography, large dark rotary knobs,
large patch sockets and purposeful signal-flow graphics. The concept's fictional
CONTOUR face is a visual reference, not an implemented module or a DSP contract.
The reference images and prompts are retained in
[concept folder](../design/concepts/module-panels-20261016-v1/).

## Every marking needs a meaning

Each module's panel must be derived from its own documented processor, ports
and controls. Do not copy CONTOUR's envelope symbols onto unrelated modules.

- Arrows show a real signal direction or control dependency. A line that joins
  branches must correspond to a real sum, normalization or other documented
  junction; crossing lines do not imply a junction.
- Brackets identify real channel groups, control groups or input/output roles.
  They are not an excuse to draw nonexistent connections between channels.
- A curve describes an actual waveform, response law or scale. Label it where
  its interpretation would otherwise be ambiguous. A stylized envelope does
  not belong on a mixer just to fill space.
- Numeric scales use the actual parameter domain or an explicitly converted
  unit. Distinguish a knob's setting from its squared gain or CV-dependent gain.
  Do not present a raw parameter as a measured voltage.
- Normalization markings describe the actual disconnected-input behavior,
  including the distinction between an absent cable and a physically connected
  zero-channel source. Do not infer normalization from a module's musical role.
- Indicators use actual coherent engine telemetry. No fabricated activity,
  decorative blinking, synthetic waveform or master meter without a source.

An empty area is preferable to a misleading graphic. Every authored marking
should have a corresponding source-backed explanation in its panel code or
module contract, and a behavior or layout check where it affects interaction.

## Shared interaction and rendering

The rack and module browser use the same panel renderer and geometry. The
browser scales that face rather than inventing a second illustration. Titles
are prominent and left aligned, without manufacturer-style OpenPatch branding,
descriptive subtitles or colored header bands. Rotary controls and sockets must
remain identifiable when zoomed out; target sizes, labels and cable endpoints
must not overlap. Numeric entry, fine adjustment, keyboard navigation and undo
remain part of the control contract.

Dark-room emission follows the existing actual signal and lamp observations.
It must preserve legibility and avoid implying a new DSP measurement.

## Current development candidate

The 80-kind catalog now has 76 detailed Signal Atlas conversions, the retained
bespoke Studio Mixer, and three explicitly deferred MIDI panels. The non-MIDI
inventory/design audit is complete. [Actual panel captures and audit evidence](../design/reviews/signal-atlas-two-modules-v1/README.md#non-midi-design-candidate-audit)
show the shared rack/browser renderer, physical controls, source-backed guides,
readable values and restrained local lighting.

Accepted MCP edits use real control anchors: angular knob cursors, moving fader
cursors and saved-switch caps. A human press takes over immediately. Hidden or
context-only controls use named header cues rather than invented visible targets.
Scope preserves calibrated traces and weak-signal AUTO; configuring a diagram
never invents measured activity. Notes and narrow Blank panels retain editing,
resize behavior and physically bounded mounting details.

The installed 0.2.6 build 8 remains the earlier 34-panel checkpoint with stronger
glow. The redesign is [merged](https://github.com/dexterlabs1/openpatch/pull/28);
[full Mac workspace/MCP CI](https://github.com/dexterlabs1/openpatch/actions/runs/39007243527)
and the internal app artifact passed inspection. It has not been installed. This is a macOS/non-MIDI design milestone, not complete Rack parity or
native audio-device qualification. The sections below retain earlier rollout
records; their historical platform gates do not add new qualification claims.

## First implementation

Stock VCA Mix is the first application of this direction. Its four strip outputs
are independent taps after the strip gain and optional CV processing. Those
four signals also feed a sum; the master gain and master CV apply to the mix
output only. Four source-defined strip meters are available. There is no fifth
native master lamp. The strip knob is squared in DSP, the master knob is linear,
and each CV law is separately selectable between linear and fourth power.

The broader panel conversion remains incremental. Selecting a visual direction
does not establish that every existing module has been redesigned or qualified.

Stock VCO and Stock LFO extend Signal Atlas with four large black rotaries in a
single row and enlarged labeled input/output sockets. The pointer and scale arc
show each saved control's bounded position; they are not phase, pitch or output
meters. The printed VCO frequency and FM laws, and LFO clock-relative rate and
PWM dependencies, describe their actual processing. VCO Linear FM/Soft sync
and LFO Offset/Invert remain explicit interactive modes. Their phase and mode
lamps use published runtime state, while a library preview has no invented
activity. Rack and browser use the same renderer and port locations.

For MCP construction edits, each rotary gets a temporary angular cue from the
old normalized setting to the new one at that control's actual center, with
old/new value text tied to its readout. This cue reports an edit, not an audio
measurement. Direct pointer or keyboard input takes over the control. Labels,
readouts, the 68-point rotary targets and enlarged jack material need visible
clearance at the library's reduced scales; moving text is preferable to making
the controls smaller. Application qualification for these panel changes is recorded in
[the combined release report](benchmarks/stock-noise-release-qualification.json).

Stock Noise uses seven separate horizontal process marks terminating at its
seven actual outputs. The marks name the normal draw, Voss rows, white and
pink-derived differences, gray FFT block, and black uniform mapping. They do
not connect independent outputs or suggest a live spectrum. There are no
decorative lamps or fake parameter knobs. Its native reference and application
status are recorded in [Stock Noise](STOCK_NOISE.md).

## Original module catalog tags

Every future original OpenPatch module should declare `"Custom"` and at least
one functional category in its catalog `tags` array. Studio Mixer uses
`["Custom", "Mixing"]`; the two tags put it in both library filters while
retaining Mixing as its musical role. Multiple categories are intentional:
library membership and search match tags case-insensitively, while existing
modules keep their kind-based category. Tags describe where a module belongs;
they do not assert Rack or hardware equivalence. [Studio Mixer](STUDIO_MIXER.md)
documents the console's source-backed meters and signal contract.

## Voice and utility rotary rollout

[PR23](https://github.com/dexterlabs1/openpatch/pull/23) extends the shared black rotary hardware to ten more
voice and utility panels: analog VCO, ladder VCF, FADE, Compare, legacy Process,
Rescale, Gate, Attenuverter, White Noise and Oscillator. Their continuous knobs
use relative adjustment, separate exact-value fields and angular MCP edit cues.
Waveform/law choices remain labeled choices; the noise seed retains its discrete
editor. Knob sizes follow each control cell, with larger primary controls and
smaller secondary controls rather than overlapping neighboring readouts.

These panels share the warm ivory face in the rack and browser. Their existing
signal annotations and logical port endpoints are retained. Increasing socket
sizes on the remaining catalog needs a separate clearance pass. Mixer strips,
envelope stages, sequencer cells and dense effects retain their existing
controls in this slice. The [local qualification report](benchmarks/module-rotary-rollout-local-qualification.json)
records 1,028 passing Desktop tests, strict Clippy, formatting, 114 MCP checks,
an optimized workspace build and a verified signed ARM bundle. Both exact-source
platform CI runs and both 149-image native capture reviews passed before merge.
Its 0.2.3 build 5 candidate is superseded by the qualified 0.2.4 build 6
rotary release described below.


## Oscillator and voice rotary panels

[PR24](https://github.com/dexterlabs1/openpatch/pull/24) extends the same ivory face
and shared black knob hardware to six more panels:

| Panel | Rotary controls | Preserved dedicated behavior |
|---|---|---|
| Multi VCO / Multi LFO | Frequency, pulse width, FM depth, PWM depth | Four actual waveform outputs and existing frequency laws |
| Wavetable VCO | Tune, position, FM depth, position CV depth | Bank display, actual observed lamps, sync and FM mode choices |
| Wavetable LFO | Rate, position, FM depth, position CV depth | Bank display, actual observed lamps, offset and invert choices |
| Stock VCF | Cutoff, resonance, drive and their three signed CV depths | Cutoff in Hz or notes, 100% nominal drive, unchanged processor |
| Stock VCA | Saved level | Separate measured 16-voice gain columns and EXP/LIN CV response |

Each rotary uses relative pointer movement and supports fine adjustment, keyboard
control, exact entry and shared undo. MCP edits register an angular cue at the
actual knob and show old/new values in the control's units. The cue reports the
parameter edit; it never fabricates signal activity or gain telemetry.

The existing 300/300/360/360/270/180-point panel widths and jack centers are
preserved so saved neighboring modules do not start overlapping. Wavetable's
second control row moves up five points; readouts and captions have separate
space at 25% zoom. The original socket material remains pending the dedicated
clearance pass. The [local qualification report](benchmarks/oscillator-voice-rotaries-local-qualification.json)
records the executed checks and their limits. All eight jobs across both
exact-source platform CI runs and both 149-image native screenshot reviews
passed before PR24 merged. The signed macOS ARM candidate is 0.2.4 build 6;
local native Mac window/input/device qualification remains open.


## Larger sockets and crowded-port picking

The 0.2.5 build 7 development candidate increases socket material by 30% on the
sixteen panels from the two rotary rollouts above. Panel widths, rack positions,
canonical port names and cable endpoints stay fixed. Existing enlarged Stock
VCO/LFO, Stock Noise, Stock Process and Stock VCA Mix presentations stay intact.

Caption placement now follows each panel's layout separately from socket size.
Dense Compare and Process output labels sit above their sockets. Wavetable and
Stock VCA edge outputs have above-socket labels; Multi VCO/LFO retain column
legends between their output sockets. Stock VCF's last control row moves upward
two logical points, retaining space between its numeric entry and CV captions.
The existing Compare polarity lamps move five points below their old position
to clear the larger output surround; they still display the same actual state.
No new synthetic activity or DSP measurements are introduced.

At reduced zoom, accessible jack targets can overlap. Press intent, hover
instructions and context-menu selection resolve these overlaps by the nearest
jack center, retaining each port's canonical identity. Right-click ownership
requires an owned press and a matching release; interruptions and changed graph
identity cancel it. A touch-and-hold uses egui's actual long-touch event and a
separate, revision- and kind-checked deferred menu. It does not share the smoke
request queue. Mouse holds do not become touch menus. Existing cable-drop snap
behavior, duplicate-route rules, numeric controls and shared history remain.

The [local qualification record](benchmarks/large-sockets-local-qualification.json)
tracks the executed 16-panel real-font/socket matrix at 25%, 35%, 50% and 100%,
parameter extremes, rack/browser parity, actual cable and menu events, genuine
touch events, interruption handling, and the full Desktop suite. Platform CI
and both native screenshot reviews remain delivery gates for this candidate.
