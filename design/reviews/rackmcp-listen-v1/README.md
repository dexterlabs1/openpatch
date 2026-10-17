# Listen shared-panel review

The native `rackmcp_listen` uses the same Signal Atlas renderer in the rack and
browser: four large L/R ports, meaningful independent passthrough arrows, one
red capture lamp and explicit duration/WAV/capture/inspect controls. Capture
results use an inspector; duration uses an anchored menu. No fake rotary controls.

[Normal, unavailable](listen-normal.png) · [Dark, unavailable](listen-dim.png) ·
[Normal, armed](listen-armed-normal.png) · [Dark, armed](listen-armed-dim.png)

These are CPU renders of actual production drawing. Armed fixtures process the
actual Core model for 256 samples with an armed test sink, then display its real
published status. Dark unavailable is intentionally black; the armed image shows
only the real red lamp with restrained local spill. These are not native-window
or audio-device captures. All 85 catalog default rack/browser drawings match at
three scales; Listen geometry/port/label checks cover four scales.

The real shared-session callback/worker workflow separately validates MCP
capture, external-job discovery, octave edit/undo/redo while recording, WAV
export and source-preserving save/reopen. See
[contract/evidence](../../../docs/benchmarks/rackmcp-listen.json).
