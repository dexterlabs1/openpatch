# Studio Rack panels and module previews

The browser shows the actual module panel at its default settings, fitted
proportionally into each card. Rack panels and thumbnails share one drawing
path for every built-in kind: the same controls, port positions, labels,
passive lights, typography, and hardware materials. Wide sequencers retain
their real proportions rather than being rearranged into a generic miniature.
The card's title, favorite star, category, and availability overlay belong to
the browser and sit outside the module artwork.
Wide modules (over 18 HP) use a full-width card; narrower panels share two
columns. Cards preserve the panel proportions and reserve height for the hardware. Hover a card to inspect an enlarged
default panel alongside its description; the inspection view uses the same
renderer and fits the available screen without stretching the module.

![The module library uses actual default module panels](images/openpatch-library.png)

*Native Linux CI capture; macOS is the primary design target.*

The Studio Rack skin uses neutral brushed aluminum, compact printed headings,
restrained signal-family accent rules, recessed value wells, graphite knobs with
metal shoulders, crisp beveled panel edges, and machined jack collars. Outputs
have reversed charcoal labels and dark socket surrounds; inputs use dark print
directly on the panel. Permanent model captions match in the rack and browser.
Patch instance IDs appear in header tooltips and context menus. Long titles fit
their panel width, and value text fits its well. Specialized sequencer and utility
fields share the same gray readout material as knob values. The analog VCO and ladder VCF have a
large primary frequency/cutoff knob above a pair of secondary controls. Knobs
and numeric editors follow that layout in both the rack and browser; parameter
ranges and cable endpoints are unchanged. Panels scale from shared drawing code; subpixel brushing is omitted
at small scales to keep thumbnails and zoomed-out panels clear.

Previews show catalog defaults and neutral telemetry. They do not instantiate
DSP or an audio device, read or mutate the live patch, or respond as playable
controls. Clicking or dragging anywhere on a preview operates its library card.
Favorites, search, keyboard navigation, and engine-version gating remain
browser actions. Modules added to the rack receive their own instance IDs and
live parameter and signal state, so those values naturally differ from the
preview once patched. The drag-placement ghost remains an outline.

Only visible cards paint their previews. Each cached preview owns its metadata
and presentation state; a catalog change replaces it. No thumbnail image
assets or separately maintained miniature control layouts are used.

The shared-renderer regressions compare primitive geometry and drawing for all
45 built-in kinds at default settings at 42%, 85%, and 100% scale, including
real rack instance IDs. Interaction tests cover full-width and narrow cards, preview input
isolation and card click/drag behavior over specialized controls. Native Linux
CI captures qualify the appearance; macOS device playback is not part of these
visual checks.

Panel print uses [Barlow Semi Condensed SemiBold](../crates/openpatch-desktop/assets/fonts/README.md),
with its original SIL Open Font License bundled in the Mac application.
