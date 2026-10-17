# Compatibility boundary

Decision recorded 2026-10-16 following one focused Astra planning advisory. This records the compatibility contract. The importer/native workflow is implemented on `work/rack-import` and undergoing qualification; the extension host remains separate. See [current import support and limits](VCV_IMPORT.md).

## Stable identities and state

Native module kind IDs remain stable. Original, legacy and source-behavior models retain distinct identities; do not silently replace an old module with a stock model. Introduce explicit versioned module-state migrations when a model's persisted state changes. Keep patch-format/scheduling version separate from individual module-state versions.

An import maps a pinned `(plugin, model, version)` identity to a supported native kind and explicit port, parameter, state and layout transformations. Record source provenance and mapping version. Tests use complete real patches and the relevant boundary cases, not merely matching module counts.

## Preserve unsupported content

Unknown modules become explicit placeholders retaining their source identity, stable module ID, source JSON/state, layout, incident cables and referenced assets. Round trips must preserve that payload so adding support later can recover the module. Report missing dependencies and substitutes; never quietly claim an approximate module is equivalent or discard unrecognized state. Extend the native project container deliberately for provenance/opaque payloads rather than inserting hidden source parameters into DSP nodes.

## Shared control boundary in progress

`ProjectDocument` is the portable authority; Session's Core `Patch` is only the validated native state carrier. Unknown nodes and incident source routes are retained in the document, not passed to DSP as fake modules. Source payloads/assets are immutable and shared through history. Normal edits preserve source references; deleting/recreating a native ID does not inherit its old provenance or opaque routes. A legacy patch-only save or replacement cannot silently erase source data.

The shared MCP boundary provides `inspect_document`, `replace_document`, `load_document`, `save_document` and `set_document_playback`. Compact references omit source JSON and asset bytes and are explicitly not loadable documents. Full saves preserve them. Every load defaults to Exact; partial permission is session history, not an automatic saved fallback. Suspended saves explicitly report persisted fields rather than fabricated live signal state. Duplicating incident unresolved routes is currently rejected for cable modes that would silently omit them; the full document editor still needs that operation.

Execution permission travels with revision/generation-fenced audio plans. Explicit suspension adopts frozen native state without processing either active or donor engine, retires donors, cancels stale input and hides signal observations. Waiting for a newer runnable plan freezes execution/publication without canceling held MIDI or newly accepted actions. Runtime persistent fields are captured before crossing from runnable to suspended, including undo/redo; explicit history state assignments retain priority.

Bounded `.vcv` container decoding is implemented. Pinned conversion mappings, native placeholders, the native project-store migration and the complete opening/repair/play/save/reopen collection remain unfinished. This boundary is not a shipped generic Rack importer or a VCV binary host.

## Extensions and existing VCV binaries

A versioned Rust module manifest/API and a host for existing VCV C++ binaries are separate deliverables. New native extensions use the same identity/state/migration rules as imports. Existing binary compatibility remains an explicit open host-matrix gate; choose a separate compatibility host boundary before implementing it. Do not load an arbitrary existing VCV library directly into the Rust audio callback or claim that porting a few modules supplies ecosystem compatibility.

Pin supported module/version mappings and audit licenses/assets for each port or host. Keep source behavior, public plugin ABI, official distributed-binary identity and physical hardware accuracy as distinct claims. General-purpose on-the-fly DSP authoring does not block the usable import milestone.

## First acceptance collection

Start with the user's existing musical patches, including the recorded “So There I Was” reconstruction, and a small factory set covering polyphony, feedback and effects. Opening, repairing, playing, saving and reopening must preserve declared supported behavior and unknown payloads. Expand that support matrix in coherent batches; keep unsupported third-party families visible. Choose additional cases when a specific dependency or bug requires them.
