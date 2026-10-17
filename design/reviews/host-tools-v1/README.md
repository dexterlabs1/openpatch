# Host tools and footer review

2026-10-17. These are CPU renders of the application's actual egui rack, toolbar,
footer and anchored controls. They do not open a native window or audio device.
The installed 0.2.7 build 9 is unchanged.

- [Audio performance dropup](audio-performance-dropup.png): bounded load history,
  a 100% processing-deadline guide, current/peak/mean budget use and overruns.
  The graph is explicitly marked **SIMULATED DATA · SCREENSHOT FIXTURE**. Its
  illustration values are used only by this rendering test. Production history
  samples coherent `AudioTimingSnapshot` publications, rejects non-finite values
  and duplicate callbacks, and retains at most 180 observations. No device
  deadline, hardware underrun or whole-machine CPU measurement is claimed.
- [MCP session dropup](mcp-session-dropup.png): the stopped bridge's actual UI
  anchors above its footer button. Starting/stopping and attachment-copy controls
  retain their existing shared-session behavior. No connection is fabricated.
- [Narrow footer](footer-room-narrow.png): at 940 × 680 the Room slider remains
  visible alongside audio/MCP controls. Its host preference persists without
  modifying the project. Organize and the four rack view buttons remain visible
  in the second toolbar band.

The panels toggle from their buttons and dismiss on Escape or an outside click.
They use anchored menu containers with scrolling instead of floating windows.
File → Open recent preserves ten distinct successful opens/saves, including Rack
imports, in newest-first order. Failed operations do not create history entries;
missing files remain visible as unavailable.

Focused interaction checks cover finite/bounded history, empty measurements,
panel placement/dismissal, small-window controls, persisted room/cable settings,
recent ordering and missing files, and one-step Organize undo including unresolved
imported panels. Evidence logs are under
`native-test-output/rack-control-import/ui-panels-*-007.log`. The render is
reproducible with the ignored `render_audio_mcp_and_footer_review` test and
`OPENPATCH_UI_PANEL_REVIEW=design/reviews/host-tools-v1`.
