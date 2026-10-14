# Studio Rack panels and module previews

The browser shows the actual module panel at its default settings, fitted
proportionally into each card. Rack panels and thumbnails share one drawing
path for every built-in kind: the same controls, port positions, labels,
passive lights, typography, and hardware materials. Wide sequencers retain
their real proportions rather than being rearranged into a generic miniature.
The card's title, favorite star, category, and availability overlay belong to
the browser and sit outside the module artwork.

![The module library uses actual default module panels](images/openpatch-library.png)

*Native Linux CI capture; macOS is the primary design target.*

The Studio Rack skin uses neutral brushed aluminum, compact printed headings,
restrained signal-family accent rules, recessed value wells, graphite knobs with
metal edges, and machined jack collars. Outputs have reversed charcoal labels;
inputs use dark print directly on the panel. Long instance names truncate within
the header and value text fits its well. Control
centers, parameter ranges, cable endpoints, and hit areas retain their existing
behavior. Panels scale from shared drawing code; subpixel brushing is omitted
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
45 built-in kinds at default settings at 42%, 85%, and 100% scale. Interaction tests cover preview input
isolation and card click/drag behavior over specialized controls. Native Linux
CI captures qualify the appearance; macOS device playback is not part of these
visual checks.

Panel print uses [Barlow Semi Condensed SemiBold](../crates/openpatch-desktop/assets/fonts/README.md),
with its original SIL Open Font License bundled in the Mac application.
