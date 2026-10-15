# Studio Rack panels and module previews

The browser shows the actual module panel at its default settings, fitted
proportionally into each card. Rack panels and thumbnails share one drawing
path for every built-in kind: the same controls, port positions, labels,
passive lights, typography, and hardware materials. Wide sequencers retain
their real proportions rather than being rearranged into a generic miniature.
The card's title, Inspect control, favorite star, and availability overlay belong to
the browser and sit outside the module artwork.
Wide modules (over 18 HP) use a full-width card; narrower panels share two
columns. Cards preserve the panel proportions and reserve height for the hardware. Hover a card to inspect an enlarged
default panel alongside its description; the inspection view uses the same
renderer and fits the available screen without stretching the module.

![The module library uses actual default module panels](images/openpatch-library.png)

*Native Linux CI capture; macOS is the primary design target.*

The Studio Rack skin uses matte warm metal, semibold model headings and restrained
signal-family accents. Shared generic controls use direct vertical faders;
specialized module panels retain their dedicated editors. Panels, numeric wells
and jack reflections use quieter shared materials, keeping the rack and browser
consistent. Fader handles and hover/focus outlines identify editable values. Outputs retain
charcoal labels and dark socket surrounds; inputs use dark print on the panel.
Permanent model captions match in the rack and browser; patch instance IDs appear
in header tooltips and context menus. Parameter ranges, port positions and cable
hit targets are preserved. Fader targets stay above the separate numeric wells;
clicking the number opens precise entry with a pencil affordance. Fader position
shows the configured value, independently of live signal/gain telemetry, and
dims with the panel in a dark room. See [control gestures](RACK_EDITING.md#parameter-faders).

Persistent **Inspect** uses the same cached renderer at natural size, with
scrolling for wide panels and small windows. It shows catalog defaults under
neutral lighting; controls remain display-only. Search, filters and highlighted-card
navigation do not change the pinned module. Cmd/Ctrl-I inspects the highlighted
module while the library has focus. Escape closes inspection before the library.

Previews show catalog defaults and neutral telemetry. They do not instantiate
DSP or an audio device, read or mutate the live patch, or respond as playable
controls. Clicking or dragging the card hardware operates its library card. Inspect and
favorite controls occupy separate footer regions.
Favorites, search, keyboard navigation, and engine-version gating remain
browser actions. Modules added to the rack receive their own instance IDs and
live parameter and signal state, so those values naturally differ from the
preview once patched. The drag-placement ghost remains an outline.

Only visible cards paint their previews. Each cached preview owns its metadata
and presentation state; a catalog change replaces it. No thumbnail image
assets or separately maintained miniature control layouts are used.

The shared-renderer regressions compare primitive geometry and drawing for all
64 built-in kinds at default settings at 42%, 85%, and 100% scale, including
real rack instance IDs. Interaction tests cover full-width and narrow cards, preview input
isolation and card click/drag behavior over specialized controls. Native Linux
CI captures qualify the appearance; macOS device playback is not part of these
visual checks.

Panel print uses [Barlow SemiBold and Semi Condensed SemiBold](../crates/openpatch-desktop/assets/fonts/README.md),
with its original SIL Open Font License bundled in the Mac application.
