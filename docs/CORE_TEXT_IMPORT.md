# Core Notes and the audio-device boundary

The development branch imports **Core / Notes / 2.6.6** as the existing native
Notes module. The installed 0.2.7 build 9 predates this mapping and remains
unchanged. This adds four editable annotations from the reference patch; it adds
no DSP model or catalog kind.

## Pinned source and conversion

The reference is VCV Rack v2.6.6, commit
[`061ccf63c1758599396ac1bb10d47345d9d34076`](https://github.com/VCVRack/Rack/tree/061ccf63c1758599396ac1bb10d47345d9d34076).
[Notes loading](https://github.com/VCVRack/Rack/blob/061ccf63c1758599396ac1bb10d47345d9d34076/src/core/Notes.cpp)
uses optional `data.text`, defaulting to empty; legacy root `text` overrides it.
[Generic module loading](https://github.com/VCVRack/Rack/blob/061ccf63c1758599396ac1bb10d47345d9d34076/src/engine/Module.cpp)
uses `bypass` in preference to the old `disabled` field. The mapping preserves
Unicode, whitespace and newline bytes, and applies this Boolean fallback only to
this exact Notes identity. Original module records and original archive bytes
remain immutable even after native annotation edits.

Notes has no parameters or signal ports. Native text is limited to **8192 UTF-8
bytes**, and the native panel/editor/font intentionally differ from Rack. The
mapping is labeled `MusicalRole`, not identical Rack rendering. It reuses original
OpenPatch artwork; no Core panel SVG or Rack widget implementation is copied.
Native initialization clears text; edit/initialize/undo/redo/save/reopen continue
through the shared project authority.

Absent data or an object containing only optional string `text` is supported.
Malformed/null text, NUL, over-limit text, unknown data keys, nonempty parameters,
unsupported versions and invalid legacy Boolean state remain unresolved with
their source intact. Every supplied text candidate is validated, including one
shadowed by legacy text. Archive-level malformed parameters/bypass and impossible
incident ports remain structural errors. Broader text-size/version support is
not claimed.

## Why AudioInterface2 stays unresolved

The same pin's [Core Audio processor](https://github.com/VCVRack/Rack/blob/061ccf63c1758599396ac1bb10d47345d9d34076/src/core/Audio.cpp)
has two rack inputs and two outputs from hardware input. Its output LEVEL spans
0–2 and applies **level squared**, reaching gain 4, with source DC blocking,
polyphonic summing/right normalization, buffering, resampling and output clipping.
The existing native stereo output has a different gain/filter contract; it is not
an equivalent processor or automatic substitute.

[Saved audio-port state](https://github.com/VCVRack/Rack/blob/061ccf63c1758599396ac1bb10d47345d9d34076/src/audio.cpp)
includes driver/device name, rate, block size and channel offsets. Replaying that
loader can select hardware. Import preserves those fields as source metadata and
does not invoke device selection. The exact known 2-input/2-output source shape is
validated even while the module remains unresolved. The import report explains
this boundary and reports a saved zero output level as muted.

The actual reference output is muted (LEVEL 0), with source rate 24 kHz and block
size 256. Do not silently unmute it or replace its saved device/rate/filter
behavior. A future explicit host adaptation must preserve mute and disclose the
hardware, timing, gain and filter differences. This source audit is not a native
audio-device or physical-hardware qualification.

## Validation and remaining scope

Focused checks cover Unicode and the byte limit, legacy precedence, unsupported
state, source ports and retained bytes/provenance. Shared control and native App
workflows exercise edit, initialization, history, coherent save and fresh reopen;
actual MCP stdio exercises imported annotation editing/export/reload. The complete
reference patch remains **55 modules and 83 cables**, with **37 mapped and 18
unresolved** on this branch. Its four Notes contain 138/160/112/103 UTF-8 bytes.
The original user file is unchanged. These counts describe supported instances,
not complete VCV functionality or sonic equivalence of the whole patch.

The preceding sequencing/control checkpoint and installation are documented in
[the control contract](RACKMCP_CONTROL_CONTRACT.md) and
[its evidence](benchmarks/rackmcp-control.json). Existing native Notes drawing is
unchanged; no numerical DSP oracle is rerun for this text/load translation.


Focused local validation passes 21 importer cases, eight native import workflows,
the two explicit real-fixture checks and the actual stdio workflow. Strict
CLI/desktop all-target Clippy and workspace formatting pass. The source Audio2
port test independently accepts real 0/1 endpoints and rejects nonexistent port
2. These local checks precede the
follow-up PR CI and do not replace the full goal's remaining compatibility work.
