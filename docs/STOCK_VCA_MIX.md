# Stock VCA Mix

`stock_vca_mix` is a separate original Rust functional implementation of the
pinned [Fundamental VCMixer signal contract](https://github.com/VCVRack/Fundamental/blob/10dd0160c664770910e5584b7b00498cc48d9ddd/src/VCMixer.cpp).
Existing `vca_mix` patches keep their original processor, limits and persistence.
This is a digital reference, not a hardware-validated circuit model, a VCV plugin
binary loader or a general `.vcv` importer.

## Signals and controls

Nine inputs have the native order `mix_cv, in1, in2, in3, in4, cv1, cv2, cv3, cv4`.
The outputs are `mix, out1, out2, out3, out4`. The maximum signal-input width,
with a minimum of one, determines voice count; CV widths never add voices.
Exactly one channel broadcasts across a four-lane group. Other widths read the
actual raw lanes, including padding and retained tails.

Each strip multiplies its signal by the square of `level1` through `level4`.
A physically connected strip CV divides by 10, floors negative values at zero
and optionally takes the fourth power. CV above 10 V is not capped. A cable
carrying zero channels remains physically present and applies its raw CV data;
it is distinct from an absent CV cable. Each processed strip feeds both its
independent output and the ordered sum. The sum then receives the **linear**
`mix_level` multiplier and the independent master CV law. Master controls do
not affect the strip outputs.

Arithmetic is source-ordered float32 with Rack-style signed subnormal flushing.
There is no generic output clamp. Four-lane stores precede channel-count
updates; the common native host retains or clears raw storage according to
actual connection history. Each version-2 cable contributes one sample of delay.

The four actual strip meters observe the signed padded-lane sum divided by five,
then its absolute magnitude. Their immediate attack, float32 release at lambda
30 and 512-frame lamp divider are retained. This is not an RMS measurement or a
fifth master meter. No bypass routes are configured: bypass freezes meters,
divider and module lamps, while the native host handles output transitions.

## Editing, state and panel markings

Interactive strip settings are bounded to `0..=f32(sqrt(2))`; the master is
`0..=2`. `channel_exp` and `mix_exp` are durable Boolean aliases represented as
0 or 1, not additional native Rack parameter indices. Interactive MCP validation
checks the incoming number before float32 rounding. Saved projects and presets
retain all finite float32 gain settings, including values outside editing bounds.

Initialize assigns one to all five saved gain controls and clears both flags.
Its deferred runtime hook must preserve the receiving graph's authoritative
controls as well as existing meter, divider, light and port history. An Initialize
followed by a newer edit, preset or Undo must not overwrite that newer state at
the next callback. Compatible graph adoption retains runtime history while the
receiving controls win. New incarnations start fresh.

The diagnostic legacy loader reads the native `chExp`/`mixExp` JSON keys: true
sets a flag, any other present value clears it, and absent keys/nonobject roots
retain it. The saver emits both Boolean keys. This diagnostic path does not
establish general Rack project import.

The panel follows the selected [Signal Atlas design rules](MODULE_DESIGN.md).
Its topology must show real strip taps, sum and master-only processing rather
than decorative connections. Rack and browser share its actual renderer.

## Qualification

The independent seven-file declaration, complete inputs and source/SDK hashes
were reviewed and frozen before the first compiler or DSP execution. Across
44.1, 48 and 96 kHz, the initial genuine Rack Engine and optimized Rust Core
comparison pass **39,021 cases, 130,743 complete snapshots, 91,722 events and
348,900 independent hand anchors**. The validator checks 2,353,374 independent
state/topology invariants. All declared fields, inactive raw storage, internally
generated nonfinite values, four meters and four module lamps are exact; no
constructor, first frame or no-sample event is discarded.

The final-source comparison after the Initialize authority fix passes the same
unchanged gate and produces the same complete trace. The
[retained report](benchmarks/stock-vca-mix-host-reference.json) contains both
paired attempts, source/build hashes and links to lossless input and trace files.
The complete trace SHA-256 is
`bf90867c792d5185e3c054698241fc11fa714bad3831f2de52f1188511db77b0`.
The oracle uses actual pinned source with genuine Rack 2.6.6 ARM SDK and explicit
`-O2 -ffp-contract=off`. This proves the declared pinned-source-plus-SDK scope,
not equivalence to every official distributed plugin build or hardware.

Independent application review identified and corrected deferred Initialize
control authority, callback meter registration, modal input ownership, absolute
fader input on rotary controls and misleading raw-CV equations. Actual callback
and pointer/key regressions cover those fixes. The current local checks pass
2,065 Rust tests (six existing ignored) and 109 Python checks, including the
real stdio MCP tests. The factory renders two seconds of finite, unclipped stereo
audio offline. Shared panel tests cover rack/browser equality, text separation
from 25% zoom and clearance around enlarged socket rings.

The source uses the selected Signal Atlas visual direction: no colored top band,
large relative rotary controls and a real strip-sum/master schematic. Both native
Linux CI runs produced all 139 expected captures, including the unchanged 136
existing scenes. The reviewed new scenes show the same rack/browser geometry
and measured strip lights in dark-room mode. See the
[native browser preview](../design/concepts/module-panels-20261016-v1/native/stock-vca-mix-library.png).

The unchanged Stock Mix, Stock Octave, rounding, Mid/Side and output-lifecycle
reference gates all pass with the final Core. Four existing offline workloads
also retain exact analysis statistics and all measured checksums against the
previous build. Five paired rounds produce median timing changes from about
−0.03% to +1.31%; this is not evidence of a speedup. The
[qualification record](benchmarks/stock-vca-mix-release-qualification.json)
retains complete compressed regression and benchmark reports, source/binary
hashes, capture manifests and static ARM bundle checks.

Native Mac window/input/audio-device checks remain unqualified. No CPU
advantage, full catalog redesign or complete VCV module parity is claimed.
