# Native MCP control panels

Bridge and Sequencer use the same Signal Atlas renderer in the rack and module
browser. These are CPU captures of production drawing, not native-window or
audio-device qualification. Fixture labels and telemetry are explicit; the actual
connection/save and callback transport checks are separate from drawing fixtures.

## Bridge

[Normal](bridge-normal.png) · [Room off](bridge-dim.png) ·
[Offline preview](bridge-offline-normal.png)

One app-owned local service, three native host lamps, bounded client/save/last
operation display and explicit Start/Stop/Restart/Save/Details controls. Browser
previews stay offline. The actual lifecycle tests exercise sockets and listener
failure, including discovery withdrawal, restart generations and removal of all
panels without stopping the service. Save tests exercise real disk publication
with concurrent replacement/edits and coherent published effective controls.

## Sequencer

[Live rack](rackmcp-live-rack-normal.png) ·
[Live room off](rackmcp-live-rack-dim.png) ·
[Authored thumbnail](rackmcp-authored-thumbnail-normal.png) ·
[Live thumbnail](rackmcp-live-thumbnail-normal.png)

Actual active/pending pattern, clock, playhead and position observations accompany
anchored pattern, now/wrap and seek controls. Authored/browser views draw retained
patterns without pretending to have live telemetry. Existing shared transport
operations own all actions; drawing/polling neither adds history nor drives time.

[Contract and limitations](../../../docs/RACKMCP_CONTROL_CONTRACT.md#native-bridge-and-live-sequencer-checkpoint-2026-10-17).
