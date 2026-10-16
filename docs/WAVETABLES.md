# Wavetable VCO and LFO

`wavetable_vco` and `wavetable_lfo` are separate version-2 modules with up to
16 voices, one `out` port, custom immutable tables and an original shared
rack/browser panel. They implement bounded digital behavior against pinned
Fundamental classes. This is not measured hardware fidelity or complete Rack
host, plugin, file or UI parity.

## Controls and signals

| Module | Inputs | Controls |
| --- | --- | --- |
| VCO | `fm`, `sync`, `position_cv`, `pitch` | `tune` −75…75 semitones, position, signed FM/position CV depths, hard/soft sync, exponential/through-zero linear FM |
| LFO | `fm`, `reset`, `position_cv`, `clock` | `rate` −8…10 log2 Hz, position, signed FM/position CV depths, bipolar/unipolar offset, invert |

Position spans 0…1; both CV depths span −1…1. Position CV adds
`CV × depth / 10` before clamping and crossfading complete waves. The visible
position indicator follows actual CV-influenced position, carried in the same
coherent packet as the five module lights. Default tables are locally generated
sine, triangle, saw and square waves, with 1,024 points each.

The VCO uses C4 tuning and V/oct pitch, eight-times dense prepared audio tables,
harmonic level selection and hard-sync correction. Frequency magnitude is
bounded at Nyquist. Audio tables require a **power-of-two wave length of at
least 32 points**; arbitrary audio FFT lengths are not supported. Preparation
preserves the pinned SDK's packed-FFT Nyquist image and the source's rounded
phase endpoint behavior. These are explicit digital compatibility choices.

The LFO linearly interpolates unfiltered source points and waves. Any integer
wave length of at least two points is accepted if a complete wave fits. With no
clock, rate is `2^rate` Hz; a patched clock supplies the shared measured rate,
scaled by `2^rate / 2`, with a 1,024 Hz ceiling. Reset advances then resets phase
before emitting. Invert precedes offset, then the result is multiplied by 5 V.
The default LFO is unipolar; custom sample amplitudes are not normalized.

VCO width is `max(pitch, FM, 1)`. LFO width is `max(FM, 1)`; its FM reads raw
lanes without mono broadcast. Other auxiliary mono inputs broadcast, missing
short-poly lanes read zero, and clock reads channel zero. Four-lane processing
advances hidden padding within an active group; wholly inactive groups freeze.
Visible widths stay unrounded and inactive output storage is zero.

Compatible same-ID/kind/rate adoption copies bounded phase, detector and sync
correction histories while retaining the receiving parameters and table. Source
changes preserve detector histories. Bypass emits mono zero and freezes the
oscillator and its lights. Initialize restores parameters and the Basic table;
VCO runtime keeps phase/sync history/correction tails while restoring forward
directions, whereas LFO runtime clears phase and clock timing while retaining
trigger histories. Explicit Engine state reset clears runtime independently.

## Files, projects and presets

Use a wavetable module's context menu to import or export a workspace file.
Wave points can be entered explicitly. Otherwise import takes a power-of-two
WAV sample-rate hint, then the current geometry or the 1,024-point default.
The source must contain at least one complete wave; an incomplete trailing wave
is saved but not played.

Supported inputs are RIFF/WAVE PCM8/16/24/32 or float32, little-endian signed raw
`.s8/.i8`, `.s16/.i16`, `.s24/.i24`, `.s32/.i32`, and `.f32`. Integer decoding
uses the documented codec scales; no subsequent amplitude normalization occurs.
Finite float samples preserve their bits, including signed zero. WAV channels
remain interleaved in sample order; they are not mixed to mono. AIFF, compressed
WAV, RF64 and big-endian files are unsupported.

Import accepts at most **524,288 scalar samples** and a **16 MiB source file**.
Malformed, nonfinite, partial-sample, invalid-geometry and oversized input fails
without changing the active graph/history. Workspace names are bounded simple
filenames; reads reject symlinks and nonregular files. Export creates a new
file and refuses overwrite: `.f32` preserves all original sample bits, including
unused trailing points; `.wav` uses the source-style mono PCM16 encoder and
clips to [−1, 1]. Exporting Basic materializes the production default samples.

Projects store typed table content and digest-addressed sample assets inside the
native document; source paths are not playback dependencies. Duplicate modules
and history share immutable asset ownership. Project files are bounded to
64 MiB, current decoded assets to 32 MiB, and unique decoded assets retained by
history to 128 MiB. Derived bank admission has a shared **1 GiB payload budget**,
including preparation scratch reservations; this is not a process RSS limit.
A valid source may still exceed preparation admission. History keeps weak
derived tickets: cold Undo/Redo rebuilds expired banks on the control thread
before changing the live graph.

Portable `.opreset` files are bounded to **4 MiB** and contain exactly the
single module's referenced asset, with digest/content/shape validation. Basic
presets omit custom content and restore Basic when applied. Legacy nonwavetable
preset JSON and annotation partial-merge behavior are unchanged. Ordinary
wavetable preset tool replies contain `preset_summary`, which is deliberately
not a valid portable preset. Set `include_asset_data: true` to request the full
`preset`. File application requires the exact `file_sha256` returned by reading
the file; changed bytes are rejected before the edit commits.

## MCP and publication

| Tool | Purpose |
| --- | --- |
| `import_wavetable` | Decode a workspace file for `id`; optional `wave_points` and `expected_revision`. One atomic edit, with identical reimports treated as no-ops. |
| `set_wavetable` | Bind an existing `asset_id`, `wave_points` and `name`; supports preview and atomic batches. |
| `export_wavetable` | Write a new `.f32` or `.wav` without changing the patch. |
| `inspect_wavetable` | Return compact geometry/asset metadata; optional `offset` and `limit` request at most 256 samples and an 8,192-byte response. |

Decode, hashing, FFT preparation and cache admission run outside the audio
callback. The shared import worker prepares an isolated candidate, then rechecks
revision, target identity and authorization before publication. Cancellation or
a conflicting edit prevents a later commit. The callback receives a complete
immutable bank; it does not load files, plan FFTs, allocate or destroy banks.
The [codec](../crates/openpatch-core/src/wavetable_file.rs),
[preparation ownership](../crates/openpatch-core/src/wave_prepare.rs) and
[workspace tools](../crates/openpatch/src/control/wavetable_tools.rs) define the
bounded formats and transaction behavior.

## Checked numerical scope and remaining gates

The [actual-class oracle](../scripts/wavetable_reference/README.md) pins
Fundamental commit `10dd0160c664770910e5584b7b00498cc48d9ddd` and runs actual
WTVCO/WTLFO classes against a separately supplied Rack SDK. The [checked report](benchmarks/wavetable-reference.json), with source and
executable hashes, passes **153
cases, 626,688 frames, at 44.1, 48 and 96 kHz**. Maximum voltage error is
**1.9073486328125e-6 V**; all LFO voltage samples are exact in that run. The
frozen gate is `0.002 V + 1e-5 × abs(reference)` from frame zero, with exact
discrete observations and separately bounded phase/position/clock observations.
No alignment or startup exclusion is used. The final proof rerun verifies the
production source closure and probe executable hashes and rejects changes during
the run.

Coverage includes custom/default banks, position/CV, FM, hard/soft sync,
clock/reset, widths and poisoned padding, frozen groups/loading, invalid-bank
guards, bank/geometry replacement, source Reset and state adoption. The source
VCO's zero/negative-frequency floating-to-integer path is undefined and is
excluded from the oracle. OpenPatch separately tests signed-frequency wrapping,
phase hold at exact zero, absolute-frequency filtering, sign crossings and
usable sync at zero. Those are safe native behavior guarantees, not reference
fidelity claims for an undefined source path.

Rust implements the runtime DSP and table preparation. Hard-sync numerical step
data was computed offline with the separately licensed GPL Rack SDK tool; no
SDK is linked, invoked by Cargo or shipped at runtime. The data is computed
numerical output, not copied program text or an embedded upstream wavetable.
See the precise [kernel provenance and reproduction method](../scripts/wavetable_kernel/README.md).

Public Engine, callback heap guards, Session/history/file/preset and panel tests
exercise separate local host contracts. The shared rack/browser title layout
uses prominent left-aligned module names without brand/description headers.
The macOS ARM release candidate builds and its ad-hoc bundle signature verifies.
[PR #5](https://github.com/dexterlabs1/openpatch/pull/5) merged after
[exact-source CI](https://github.com/dexterlabs1/openpatch/actions/runs/38833771005)
passed on macOS ARM, Linux and Windows, including strict Clippy, release builds
and 81 Python/MCP tests per platform. Mac/Linux passed 1,673 Rust unit/integration
tests and Windows 1,646; existing ignored/export fixtures remain separate.
All 100 native PNG captures passed exact-set, chunk CRC and decoded-row checks.
The normal/dark wavetable rack, shared browser, legacy utility rack and compact
rack/browser images were visually reviewed: headings are prominent and
left-aligned, static branding/descriptions are absent, and live lamps/Scope
remain visible in dark mode. These Linux-rendered captures do not qualify native
macOS window/input/device/playback. The candidate has not replaced the installed
app or been interactively verified on macOS.
