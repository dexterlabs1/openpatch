# Stock Quantizer

`stock_quantizer` is a separate polyphonic pitch quantizer targeting
[pinned Fundamental Quantizer](https://github.com/VCVRack/Fundamental/blob/10dd0160c664770910e5584b7b00498cc48d9ddd/src/Quantizer.cpp).
It provides the stock half-semitone lookup behavior, a durable twelve-note scale,
pre-offset and actual played-note observations. The older `quantizer` retains
its nearest-pitch, upper-tie and empty-scale passthrough policies; existing
patches are not migrated.

Open **File → Quantized stock voice** to hear the difference. The
[nine-module, twelve-cable example](../examples/stock-quantized-voice.json)
sends Stock SEQ3's first voltage row through a C-major scale into Stock VCO.
The other rows modulate Stock VCF cutoff and resonance; the sequencer gates an
ADSR EG through Stock VCA and stereo output. Scope X shows raw pitch and Scope Y
shows quantized pitch. The Notes module explains the controls in the rack.

## Controls and signal behavior

| Saved parameter | Default | Range | Meaning |
|---|---:|---:|---|
| `offset` | 0 | −1…1 V | Added before quantization; panel displays −12…12 semitones |
| `scale_mask` | 4095 | Integer 0…4095 | Twelve enabled-note bits, C through B; edited with the keyboard |

There is one `pitch` input and one `pitch` output, both at 1 V/octave. There is
no offset CV input. Version 2 processes up to sixteen pitch voices independently,
without mono broadcasting. With a connected output the output width is the
input width, or one channel when the input is empty. Normal host input padding
is zero, so an unpatched input quantizes the pre-offset.

The processor adds the raw offset in float32, multiplies by 24, floors the
result and uses Euclidean octave/range decomposition. Its cached table selects
an enabled integer note for each of the 24 half-semitone ranges. Equal integer
distances select the lower note. This is not a continuous nearest-pitch search:
with only C and D enabled, 0.1 V still produces C; the transition to D is at
0.125 V. The transition from D to the next C is at 0.625 V. The same pattern
repeats in negative octaves.

An **empty scale quantizes chromatically**. It does not silence the output or
pass raw pitch through. The empty bank remains visibly empty even though all
chromatic notes are available to the fallback quantizer.

A disconnected output has zero channels, while normal processing still updates
played-note observations. This consumer context is handled by the production
processor. The direct-class proof does not by itself qualify the host's cable,
selected-output and tap routing; those use separate public Engine tests.

Bypass passes raw active pitch through without offset or quantization, with
connected empty input producing mono zero. It freezes the played-note observation.
The sixteen-slot public output frame has positive-zero inactive padding in both
normal processing and bypass. Editing the scale while bypassed still changes the
configuration used when processing resumes.

## Keyboard, saved state and agent edits

The rack and library share the same original panel. Click a key to toggle a
note; drag across keys to paint the initial enabled/disabled choice. The gesture
uses revision-checked parameter edits grouped into one undo step. The **−1 / +1
semitone** buttons rotate the enabled scale. **Presets** offers Chromatic, Major,
Minor, Pentatonic and None; None selects the empty chromatic-fallback bank.
Pre-offset supports the shared fader and exact-value editor. Native gesture and
render qualification are tracked separately below.

Configured note colors describe the enabled bank. Cyan markers describe the
actual union of notes selected across the current processed voices. They are
not inferred from the scale, a single monitored voice or output voltage. The
source has `NUM_LIGHTS = 0`; OpenPatch carries this information in optional
`playing_notes` metadata rather than inventing a lamp array. A fresh instance
has no observation until its first normal process. Missing telemetry displays
**PLAYED —**. Browser previews do not fabricate playback.

Every normal frame replaces the played-note union, including while the output
is disconnected. Reset, scale edits, loading, rotation and bypass retain the
last observation until processing replaces it. Same-rate runtime adoption also
retains it, while the receiving scale and offset win. A sample-rate change
starts without an observation. The pinned source's uninitialized pre-process
`playingNotes` storage is never read by the reference adapter.

The scale is durable saved state (`StockQuantizerNotes`) and is represented by
`scale_mask` in native patch parameters. Saves, module presets, duplication and
history use the host state path. Runtime reset enables all twelve notes and
refreshes the table without clearing the last observation or changing offset;
UI Initialize additionally restores the parameter defaults. OpenPatch seeded
Randomize selects uniformly among all 4096 banks and randomizes offset; it does
not reproduce Rack's global random stream.

MCP can set `offset` in volts and `scale_mask` as an integer through
`set_parameter`. `rotate_scale` accepts an `id` and integer `semitones` from
−12 through 12; it preserves offset, identity and cables and supports atomic
batches and previews. Supply the current `expected_revision` when coordinating
with native edits. One rotation is one revision and undo step.

The reference loader exercises actual source `enabledNotes` JSON arrays.
A missing key or empty array retains the existing bank; a short boolean array
replaces only its present prefix, preserving the remaining notes. Entries after
the twelfth are ignored. Loading recomputes the table and retains the last
played-note observation. This is a bounded source-state contract, not a general
`.vcv` patch importer. Malformed nonboolean JSON is outside the frozen comparison.

## Numerical qualification and limits

The [actual-class report](benchmarks/stock-quantizer-reference.json) passes the
first numerical comparison under the independently frozen declaration:
**267,483 reference frames** at 44.1, 48 and 96 kHz, with **11,745,768 exact
meaningful-field comparisons**, including **3,988,761 active output values**.
There are zero mismatches and zero measured absolute error. Counts, the enabled
bank, all 24 cached ranges, initialized played-note booleans and offset are
checked on every frame; validity bookkeeping is also checked. No settling
frames, skipped rows or numeric tolerances are used.

Every one of the 4096 masks is loaded through the actual JSON path. All 24 ranges
are sampled at positive and negative octaves with interiors, boundaries,
adjacent float32 values and actual float32 arithmetic transition pairs.
Additional cases cover offsets, integer-domain endpoints, widths 0–16,
consumer changes, duplicate notes, state replacement, partial JSON, rotations,
reset, adoption and bypass. Hand-derived C/D, single-C, offset and lifecycle
anchors provide independent checks. See the [frozen declaration and validator](../scripts/stock_quantizer_reference/README.md).

The retained evidence comprises 791 artifacts, a 77-file production closure and
668 genuine SDK headers, with source, compiler, executable and input/trace
hashes. The reference uses the actual pinned class with Rack SDK 2.6.6; the Rust
probe wraps the production processor. This run qualifies the recorded macOS ARM
compiler/SDK environment. An unused precomparison declaration was archived
before correcting JSON object-key canonicalization in its self-check; input
hashes, anchors and numerical gates did not change. The final five declaration
files were frozen before the first comparison and remain unchanged.

The normal domain keeps the C++ float-to-int conversion and subsequent signed
integer arithmetic defined. For
`q = floor(f32(f32(input + offset) * 24))`, the mathematical bound is
−2,147,483,640 ≤ q ≤ 2,147,483,647. The lower bound also protects
`eucDiv(q, 24) * 24` from signed overflow. Actual float32 endpoint fixtures use
q = ±2,147,483,520. This is not a promise of numerical parity for every finite
float or for source-undefined inputs.

Another **315 Rust-only safety frames** pass separately. Nonfinite inputs become
zero during normal quantization. Finite extremes outside the reference domain
use Rust's defined saturating float-to-i32 conversion and i64 Euclidean
arithmetic, without an arbitrary ±1,000,000 V clamp. Outputs remain finite.
Raw bypass deliberately preserves active NaN/Inf diagnostics; it is not covered
by the normal-processing finite-output rule. Independent saturation and recovery
anchors check this extension, without executing undefined C++ cases.

SDK inactive/disconnected output storage is excluded from numerical equality;
Rust padding is checked independently. Normal inactive input storage is zero.
Poisoned raw inactive storage, pre-process source bool contents and identical
Rack random streams are not qualified. No hardware accuracy, performance
advantage, native device, complete host or complete Rack import claim is made.

## Host and delivery status

The shared panel, native factory entry, callback state transport and public
Engine/Session tests are implemented. The complete Rust workspace passes **1,923 tests**, with zero failures and six
existing ignored tests. This includes all 17 focused Quantizer desktop tests,
callback scale painting, shared geometry at five zooms and the factory example.
Strict Clippy, formatting, all 94 Python/MCP checks (zero skipped) and an
optimized ARM workspace build pass. A separate ad-hoc signed Mac bundle matches
the tested release sections and UUIDs, including exact bundled fonts/licenses;
it was not installed or launched. All 127 configured native captures were
validated and visually reviewed. All eight jobs passed on both the exact-head
push and PR runs before [PR14](https://github.com/dexterlabs1/openpatch/pull/14)
merged. Native Mac window/input and physical audio-device qualification remain
separate open gates.
