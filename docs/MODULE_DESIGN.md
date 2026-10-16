# Module design language

The selected direction is **01 — Signal Atlas** from the five October 16, 2026
module concepts: warm ivory, strong ink typography, large dark rotary knobs,
large patch sockets and purposeful signal-flow graphics. The concept's fictional
CONTOUR face is a visual reference, not an implemented module or a DSP contract.
The reference images and prompts are retained in
`design/concepts/module-panels-20261016-v1/` in the design worktree.

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
the controls smaller. Application qualification for these panel changes is
pending.

Stock Noise uses seven separate horizontal process marks terminating at its
seven actual outputs. The marks name the normal draw, Voss rows, white and
pink-derived differences, gray FFT block, and black uniform mapping. They do
not connect independent outputs or suggest a live spectrum. There are no
decorative lamps or fake parameter knobs. Its native reference and application
status are recorded in [Stock Noise](STOCK_NOISE.md).
