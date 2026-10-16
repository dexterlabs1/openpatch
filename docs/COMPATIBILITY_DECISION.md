# Compatibility boundary

Decision recorded 2026-10-16 following one focused Astra planning advisory. This records the contract for the next import milestone; it does not claim the importer or extension host is implemented.

## Stable identities and state

Native module kind IDs remain stable. Original, legacy and source-behavior models retain distinct identities; do not silently replace an old module with a stock model. Introduce explicit versioned module-state migrations when a model's persisted state changes. Keep patch-format/scheduling version separate from individual module-state versions.

An import maps a pinned `(plugin, model, version)` identity to a supported native kind and explicit port, parameter, state and layout transformations. Record source provenance and mapping version. Tests use complete real patches and the relevant boundary cases, not merely matching module counts.

## Preserve unsupported content

Unknown modules become explicit placeholders retaining their source identity, stable module ID, source JSON/state, layout, incident cables and referenced assets. Round trips must preserve that payload so adding support later can recover the module. Report missing dependencies and substitutes; never quietly claim an approximate module is equivalent or discard unrecognized state. Extend the native project container deliberately for provenance/opaque payloads rather than inserting hidden source parameters into DSP nodes.

## Extensions and existing VCV binaries

A versioned Rust module manifest/API and a host for existing VCV C++ binaries are separate deliverables. New native extensions use the same identity/state/migration rules as imports. Existing binary compatibility remains an explicit open host-matrix gate; choose a separate compatibility host boundary before implementing it. Do not load an arbitrary existing VCV library directly into the Rust audio callback or claim that porting a few modules supplies ecosystem compatibility.

Pin supported module/version mappings and audit licenses/assets for each port or host. Keep source behavior, public plugin ABI, official distributed-binary identity and physical hardware accuracy as distinct claims. General-purpose on-the-fly DSP authoring does not block the usable import milestone.

## First acceptance collection

Start with the user's existing musical patches, including the recorded “So There I Was” reconstruction, and a small factory set covering polyphony, feedback and effects. Opening, repairing, playing, saving and reopening must preserve declared supported behavior and unknown payloads. Expand that support matrix in coherent batches; keep unsupported third-party families visible. Choose additional cases when a specific dependency or bug requires them.
