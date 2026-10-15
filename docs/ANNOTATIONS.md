# Notes and Blank — implementation contract

Status: implemented; acceptance fixed before implementation. These are original Rust/UI
functional counterparts of Rack's [Core Notes and Blank](https://vcvrack.com/manual/Core#Blank),
not copied artwork or VCV file/binary compatibility.
Pinned source behavior inspected at Rack 8ea470f343058ade6f75f16d503a1eb330e1700c:
[Notes](https://github.com/VCVRack/Rack/blob/8ea470f343058ade6f75f16d503a1eb330e1700c/src/core/Notes.cpp)
and [Blank](https://github.com/VCVRack/Rack/blob/8ea470f343058ade6f75f16d503a1eb330e1700c/src/core/Blank.cpp).
Notes stores multiline text and Initialize clears it. Blank defaults to 10 HP,
resizes by either horizontal edge, snaps to HP and has a 3 HP minimum. OpenPatch
bounds width at 96 HP and text at 8192 UTF-8 bytes; limits are explicit host policy.

## Authoritative data and interfaces

New version-2 kinds `notes` and `blank` have no signal ports, parameters or DSP
state. They cannot be selected as audio/capture outputs or cable endpoints.
Node gains optional typed `content: ModuleContent`, a serde tagged enum:
`{"type":"notes","text":"..."}` or `{"type":"blank","width_hp":10}`.
Absent content means empty Notes or 10 HP Blank; existing nodes/files serialize
unchanged. Content is rejected on mismatched kinds, unknown fields and version 1.
One authoritative Blank width lives in Node content, never a second layout copy.
Notes uses fixed 16 HP; panel height and HP use the established rack dimensions.
The text is valid Unicode, permits newlines/tabs, and is bounded by UTF-8 bytes.
Bundled fallback faces cover the demonstrated Japanese, arrows and musical
symbols alongside the existing Latin faces. Font coverage is finite; this does
not claim every Unicode script, complex shaping or color emoji support.

`set_module_content` replaces validated typed content, revision checked, undoable,
and accepted in atomic mixed `apply_edits`/preview/construction transactions.
`add_module` may receive typed content. Capture/apply native presets, duplicate,
Initialize, project/recovery save/load and undo/redo preserve or intentionally
reset content. Preset omission preserves existing content; Initialize clears Notes
and restores Blank to 10 HP. Randomize is unsupported. JSON inspection reports
actual saved content, without pretending there are realtime signal controls.

## Editing and layout

Notes uses a real multiline editor with wrapping, scrolling and normal clipboard
shortcuts. The draft is isolated until explicit Apply (one history entry); Escape
or Cancel discards it. Keyboard performance/rack shortcuts cannot steal text input.
An external revision while editing must show a conflict and preserve the draft;
never overwrite the document silently. Reopen/reload explicitly to retry.
Browser thumbnails paint the same panel, showing neutral example content; no
separate artwork. Readable scaled text and padding follow shared panel styles.

Blank supports left/right edge drag, HP-snapped preview, and direct integer width
entry. Left drag keeps the right edge fixed, updating position with width in one
history operation. Collisions reject/block the proposed rectangle; other modules
are never implicitly moved. Escape/lost context restores the original geometry.
A completed drag commits one shared history action, and a no-op commits none.
Expected revision guards reject stale gestures. Width must restore correctly even
when intermediate MCP revisions were never painted; all placement, hit-testing,
selection, browser preview, duplication and reveal use the same width accessor.
Direct MCP width changes keep the left position; overlaps are not silently moved,
and the GUI reports/blocks subsequent colliding gestures (layout is host metadata).
Existing observed-state layout history limitations remain explicit; this milestone
does not claim to solve arbitrary external position editing.

## Verification gates

- Core parsing: Unicode/limits/type/kind/version validation, old-file compatibility,
  true zero ports/output rejection and zero callback allocation/deallocation on
  first execution and state adoption; content edits preserve active synthesis.
- Session/MCP: atomic mixed edits, stale/invalid rollback, revision guard, history,
  Initialize, duplicates, presets with missing/invalid/Unicode content and bounds.
- UI: draft focus/clipboard/conflict/cancel/apply behavior; both resize edges,
  snapping/min/max/collisions/zoom/pan/cancellation; one history operation;
  unseen width revisions followed by undo; all catalog rack/browser parity.
- Project/recovery round trips with notes text, custom width and layout positions.
- Linux actual native smoke captures normal/dark Notes+Blank scenes; existing
  scenes retained, real capture receipts and images inspected.
- Full Rust, strict Clippy/format, MCP integration, macOS/Linux/Windows CI, staged
  ARM64 bundle code/resources/signature verification before filesystem install.

Native macOS UI/IME/clipboard/device runtime remain separate unverified gates.
No Mac GUI launch/control or permission workaround is authorized by this contract.
Notes uses an emissive screen (including its empty placeholder), consistent with
its readout role; Blank, grips and faceplate print remain passive in dark room.
No performance advantage over Rack is claimed from adding silent annotation kinds.


## Recorded local verification

The full workspace run passed 1,338 Rust tests (four existing ignored tests),
including 20 desktop annotation interaction/host tests, strict content and
Session transaction tests, zero-port rejection, and callback allocation/state-
adoption checks. The offscreen release regression failed before its fix and
passed afterward. Existing 62-kind rack/browser primitive comparisons cover
0.42×, 0.85× and 1× scales. Platform CI and actual native captures are separate
release gates; native macOS runtime and physical-device tests remain unverified.

Try **File → Annotated drum rack** for saved performance instructions beside a
3 HP Blank and the connected MIDI drum voice. Notes is plain text; rich text,
embedded assets and complete `.vcv` import are outside this contract.
