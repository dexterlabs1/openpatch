# Rack tools review

2026-10-17. Actual CPU renders of the shared native painter, with no native window
or audio device. These development changes are not installed in 0.2.7 build 9.

- [Signal Path](signal-path.png): toolbar arrow button or **P**, then click a cable.
  Predecessors of its source and successors of its destination are highlighted,
  including CV dependencies and connected branches. Arrowheads follow output →
  input; unrelated cables dim. A blank click clears the trace; Escape exits.
  Inspection blocks parameter/plug edits and changes no document or history.
  Feedback traversal terminates; same-ID cable moves retrace and deletion clears
  the selection. This follows module-level routing, not unknown internal port
  dependencies or measured signal amplitude.
- [Lights-off inspection](signal-path-dark.png): thin routing guides and arrows
  remain legible without adding a broad glow or altering actual signal lamps.
- [Cable materials](cable-materials.png): Classic, Woven, Tracer and Translucent
  share the same geometry, hit detection, tension and MCP animation. Surface
  marks are bounded and disappear at small zoom levels. Materials persist as a
  host preference; color/patch identity and DSP are unchanged. The default stays
  Classic. Curve slack points down rather than pushing beyond the horizontal
  plug span, including stacked and reversed cables. Tension reduces sag; this is
  a gravity-shaped presentation, not a physical cable-length simulation.
- [Black anodized hardware](black-screws.png): a seated dark head, captive washer,
  recessed hex socket and restrained machined rim replace corner dots. Detail
  reduces at thumbnail scales and follows room lighting. Atlas, generic and
  unresolved panels use the shared hardware painter; browser/rack default
  drawing equality remains guarded across the catalog.

**Organize** uses actual native/imported connectivity and dynamic panel widths.
Choose Auto or 1–8 rows; it keeps connected voices together, places signal stages
in order where possible, handles feedback deterministically, and proposes finite,
nonoverlapping rack-grid positions. It applies through the existing one-step
layout history and preserves original positions for Undo. Position lock or an
active gesture refuses the action. This is a graph-aware heuristic, not a claim
of globally minimum cable length.

Reproduce with the `signal_path` tests (set `OPENPATCH_SIGNAL_PATH_REVIEW`), the
ignored `cable_material_review` and `mounting_screw_material_review` tests. Run
`rack_organize_tests`, `cable_finish_`, `cable_presentation` and
`every_catalog_preview_matches_actual_default_module_drawing` for the focused
behavior checks. Retained failure logs and final regression totals are recorded
in the current work item's ignored STATE.json.
