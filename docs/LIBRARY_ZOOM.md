# Module browser preview zoom

Use **Library → Preview** to choose 25%, 35%, 50%, 71%, 100%, 141% or 200%.
The default is 50%. At 100%, a panel has the same logical dimensions as that
module in a rack at 100% zoom. This changes the library preview size independently
of rack zoom, audio processing and patch contents.

All cards use one preview scale. A wide sequencer stays wider than a narrow
oscillator instead of being squeezed into a miniature card. Cards wrap to the
available library width; scroll horizontally to see a panel wider than the dock,
and vertically to reach later rows. Search, filters and the preview control stay
above the scrolling panels. You can still resize the library or pin **Inspect**
to see a default panel and its descriptions at natural size.

The panel artwork is the actual shared rack renderer, with catalog defaults and
neutral telemetry. Preview controls remain display-only: clicking or dragging
the panel operates its module card. **Inspect** and the favorite star have their
own regions. Increasing preview zoom does not make these controls playable.

Preview zoom is a host preference, stored as `library_zoom` in the workspace's
`.host-settings.json`, alongside favorites and cable appearance. It is retained
across project changes and does not add a patch edit or undo step. Malformed
preferences fall back to 50%; numeric values are bounded to 25–200%.

## Reference and acceptance

Rack's [pinned browser implementation](https://github.com/VCVRack/Rack/blob/8ea470f343058ade6f75f16d503a1eb330e1700c/src/app/Browser.cpp#L980-L994)
offers the same half-octave zoom steps from 25% to 200%. Its previews scale the
actual module dimensions. OpenPatch uses its own docked card layout and default;
this is a browser workflow feature, not a claim of complete browser equivalence.

Acceptance covers actual menu and keyboard interaction, preview proportions at
the zoom extremes, wide-panel scrolling, small windows, unchanged card actions,
offscreen preview caching, and host-only persistence. Native Linux captures
provide visual checks of those paths. They do not qualify physical macOS input
or audio devices.
