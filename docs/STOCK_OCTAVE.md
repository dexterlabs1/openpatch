# Stock Octave

`stock_octave` is a separate original Rust implementation of the pinned
[Fundamental Octave signal contract](https://github.com/VCVRack/Fundamental/blob/10dd0160c664770910e5584b7b00498cc48d9ddd/src/Octave.cpp).
The existing `octave` utility keeps its own contract and saved-patch behavior.
This is a digital functional reference, not a hardware-validated analog model.

## Signal behavior

The `pitch` input determines voice count, with at least one processed lane.
The `shift_cv` input supplies an additional octave shift: one channel broadcasts;
otherwise each raw lane is read independently. CV never creates pitch voices.
The `out` port carries pitch in volts at 1 V/octave.

The processor rounds the saved parameter halfway away from zero, converts it
to a signed integer, then adds the rounded CV in float32 before the final signed
integer conversion. That order matters for large finite values. The integer
shift is converted to float32 and added to the raw pitch lane. Rack-style
arithmetic flushes subnormals; single-cable and bypass copies retain raw bits.
There is no generic ±1,000,000 V clamp in this native processor.

Active lanes are written before the output channel count changes. Connected
retired lanes clear according to the common Rack-style host lifecycle;
disconnected storage retains writes and untouched tails. Every cable contributes
one sample of delay in a version-2 patch. The configured bypass copies active
raw pitch lanes and channel count, while freezing the last processed shift.

## Editing and saved state

The shared rack and browser panel provides nine authored selector positions
from −4 through +4. Exact entry and MCP `set_parameter` accept finite values
within that range, including fractions; the DSP performs the rounding. A loaded
raw parameter may contain any finite float32 value. Projects and module presets
retain it without silently snapping or clamping it. Choosing a selector returns
the parameter to the editing range.

Authored shift and the **actual last channel-zero effective offset** are separate.
The runtime readout is unavailable before coherent callback telemetry arrives;
it is never fabricated from CV parameters. Octave has no engine Light objects.
Its transient display value freezes in bypass, survives compatible live graph
adoption, and is not serialized into a project or preset. Initialize resets the
parameter without inventing a runtime-display reset.

The diagnostic legacy loader implements the old `data.octave` migration:
an integer is converted from signed int64 to float32 without UI clamping;
a present noninteger becomes zero; an absent key or nonobject retains the
parameter. Ordinary parameter persistence remains separate. This diagnostic
loader does not establish general `.vcv` import compatibility.

## Qualification and limits

The independent declaration, inputs, adapter and validator were frozen before
any compiler or DSP execution. Input-only validation confirmed 30,120 cases,
91,479 constructor/event snapshots, 3,885 explicit anchors and 225,207 conversion
domain checks across 44.1, 48 and 96 kHz. The first genuine Rack 2.6.6 ARM Engine
run and first optimized production Core comparison pass that unchanged gate.
Both complete JSONL traces are byte-identical (SHA-256
`d25afd221a2790b55b109d5572790adfe86962ffd9dc15a3ac9e0b9401bfddc2`).
The [retained report](benchmarks/stock-octave-host-reference.json) links both
lossless traces and the inputs, frozen declaration and build/source hashes.
All 2,044 Rust tests (six existing ignored), 106 Python checks and strict
workspace Clippy/formatting pass. The factory renders stereo offline. Older
regression gates also pass: [the compact regression report](benchmarks/stock-octave-regressions.json)
records the rerun Mid Side, output-lifecycle and rounding results, alongside the
separately built paired Stock Mix result. Lossless [Stock Octave regression](benchmarks/stock-octave-regressions-001.json.gz)
and [Stock Mix paired](benchmarks/stock-mix-paired-reference-attempt-001.json.gz)
reports retain full provenance, limitations and failure fields.

The [lossless performance report](benchmarks/stock-octave-performance-attempt-001.json.gz)
compares the 75-kind baseline with the 76-kind Stock Octave candidate on one
Apple M1 Max. At 48 kHz, each of four fixed workloads ran five paired 5-second
measurements after warmup; all paired checksums match. Paired median candidate
cost relative to baseline was +0.879% for mono legacy routing, +0.137% for mono
mixed Stock/analog, +0.460% for 16-lane legacy routing and +0.432% for 16-lane
mixed Stock/analog. The latter three paired ratio ranges cross parity; the mono
legacy routing range is +0.692% to +1.050%. Timing covers DSP and checksum
reduction, excluding compilation, patch replacement and RPC. This is one host
and four workloads, without callback, device, UI or SDK timing, thermal or power
control, or full sample-trace equivalence. No speed advantage is claimed.

Optimized packaging, platform CI and native capture review remain pending before
delivery; native-device qualification is separate.

The matrix includes every pitch/CV width from 0 through 16, signed half ties and
neighbors, every finite pitch exponent, wide conversion cases, signed zeros,
subnormals, all raw lanes, output consumers, bypass and topology edits without
processing, legacy JSON, reset, recreation and receiving-parameter adoption.
Complete snapshots include actual compiled cable endpoints and source controls.
No inactive lanes, constructor states or first frames are discarded.

Undefined C++ float-to-int conversions and nonfinite controls are kept in a
separate safety stream and never executed by the Rack oracle. Rust rejects
nonfinite saved parameters, saturates undefined integer conversions, and maps
NaN integer conversions to zero. Local safety tests are not Rack parity evidence.

The sampled matrix does not prove every float combination, cross-rate adoption,
general Rack import, native device deadlines, cross-platform numeric identity or
hardware accuracy. Full VCV host acceptance remains open. Upstream GPL reference
source, its license and the genuine SDK remain isolated in retained test evidence;
they are not linked into the shipped application. OpenPatch uses its own panel.
