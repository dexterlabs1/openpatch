# Following visible agent edits

Enable **View → Agent motion → Follow agent edits** to reveal fresh agent work
outside the current rack viewport. It starts off and is remembered only for the
current window. The checked option indicates that following is armed. Turning
it on waits for the next eligible commit; it does not replay earlier work.

The camera follows the same validated commits that authorize the
[construction effects](CONSTRUCTION_MOTION.md): paced module insertion, cable
construction and parameter changes, plus direct MCP moves of existing cables.
This does not add motion receipts for arbitrary direct MCP parameter or module
commands. A batch remains one atomic graph edit; camera motion does not turn it
into a sequence of delayed DSP changes.

The camera preserves zoom and moves only far enough to reveal the affected
control where a shared-renderer anchor is available, or the changed port. A
parameter without an anchor falls back to its module/header. Nearby targets
include surrounding context where it fits. Widely separated
endpoints use a stable destination priority instead of repeatedly moving between
them. Full motion eases the pan; Reduced reveals immediately. Off prevents
automatic following and disables its checkbox.

Manual navigation stops following until you explicitly enable it again. Cable
and module gestures, value editing, popups, dialogs and lost window focus prevent
automatic camera movement. Work arriving while an editor owns interaction is
not queued for a later surprise pan. Only the latest eligible commit can become
a camera target; a hidden or stalled window does not replay a backlog.

Following changes the displayed viewport, not the patch, module positions,
selection, audio values, shared Undo history or saved project view. Stopping
following leaves the displayed camera in place. Ordinary manual navigation can
take over that view and persist it using the normal project rules. Loading or
restoring a project restores its saved viewport.

## Acceptance boundaries

Headless UI tests must exercise actual receipts and rack geometry for offscreen
parameter edits and moved cables; fresh, duplicate and stale commits; rapid
successive targets; small viewports; manual interruption; motion modes and modal
ownership. Saved-content tests must compare the complete project document,
including its viewport, rather than only checking the graph revision.

Controlled Linux native before/during/after captures provide a separate visual
gate. They do not qualify physical macOS input, window management or audio
playback. Follow mode does not guarantee that every intermediate step is seen
when an agent or the host advances faster than the display can present it.
