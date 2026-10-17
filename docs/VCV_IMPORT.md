# Rack patch import

OpenPatch can open `.vcv` patches as editable, recoverable projects. Verified mappings become native modules. Unsupported dependencies remain visible placeholders with their original data, assets and cables. Import does not promise that every Rack patch will play.

The base workflow is merged through [PR #30](https://github.com/dexterlabs1/openpatch/pull/30), extended with version-specific voices and shared panels in [PR #31](https://github.com/dexterlabs1/openpatch/pull/31), runtime source `597ebce`. It is not in the installed 0.2.6 build 8. macOS ARM is the validation target. No native audio-device qualification is implied by headless tests.

## Open, review, save

1. Open a `.vcv` file from the workspace. OpenPatch reads JSON, Zstandard JSON or a Zstandard tar archive without extracting files to disk. The original file is read-only.
2. Review the native/unresolved counts and click a placeholder to inspect its dependency. Source-only routes are dashed and carry no invented signal indication.
3. Exact playback stays suspended while unresolved modules or unbound imported automation remain. Explicit partial playback runs the supported subgraph and reports omitted modules, routes and automation lanes. This can change downstream normaling and sound. Select a native monitor output if the source's audio interface is unresolved.
4. Repair a dependency by explicitly selecting a native substitute and mapping its observed source ports, or remove its placeholder and routes. Each action is undoable. A musical substitution is labeled as such and retains the original source; it is not a claim of source or hardware equivalence.
5. Save as a new `.openpatch` project. Original source JSON bytes, unknown fields, source module/cable identities and assets remain in the portable document. Reopening defaults to Exact rather than silently retaining partial permission.

Imports preserve the source row/order and fit the actual native panel widths with spacing. The original source coordinates remain immutable. Saved `.openpatch` projects preserve their authored layout. Camera changes are saved without adding undo steps; module moves share ordinary graph history. A pending host placement blocks a full MCP save until the visible arrangement is finalized.

Portable project JSON from MCP can also be opened in the desktop. Legacy native patch JSON and its layout sidecar remain supported. A legacy patch-only export/replacement refuses to discard an imported project's source data.

Exact/Partial describe dependency omissions, not whole-patch sonic or hardware
equivalence. The inspector also shows musical substitutions and declared host
corrections.

## Supported source identities

Mappings are deliberately pinned to **Fundamental 2.6.4**, source tag commit `e0ade9a23a9b41124ea3f4ec9aa2cb0b98c2529e`. Six mappings share source files with the existing native source-oracle revision `10dd016`. VCO and VCF use separate version-specific processors, qualified against their actual older classes. Original native model identities and patches are unchanged.

| Rack model | Native kind |
| --- | --- |
| VCA-1 | `stock_vca` |
| ADSR | `adsr_eg` |
| LFO | `stock_lfo` |
| Noise | `stock_noise` |
| VCMixer | `stock_vca_mix` |
| LFO2 | `wavetable_lfo`, with its embedded wavetable |
| VCO | `rack264_vco` |
| VCF | `rack264_vcf` |

Unsupported source state, parameter ranges, missing required assets or connected ports outside a mapping's contract leave the module unresolved. Version strings alone are insufficient to certify arbitrary third-party builds. Noise mapping does not promise identical independent random streams. This is software source-behavior compatibility, not measured analog hardware modeling.

The version-specific VCO preserves the source semitone knob, curved waveforms and hard/soft sync encoding. The VCF preserves its non-oversampled RK4 ladder, signed drive and source cutoff range. Its legacy loader treats absent `params` differently from an array missing newer CV-depth entries; explicit zero values stay zero. These models share the detailed rack/browser panels but retain their own identities and source controls.

VCF starts previous-input history at zero because the upstream constructor leaves it uninitialized; undefined C++ cold-start equivalence is unclaimed. Its deterministic module-local noise stream intentionally differs from Rack's shared process-global stream. The import report discloses these limits. [Pinned comparison method and evidence](../scripts/rack264_voice_reference/README.md) cover 45 VCO cases, 78 VCF cases and ten genuine loading/reset cases; they do not establish physical hardware accuracy or whole-patch equivalence. The source-adapted processors carry GPL-3.0-or-later notices; original OpenPatch code remains MIT.

The milestone branch additionally maps **RackMCP Sequencer 2.0.0** to
`rackmcp_sequencer`, using the pinned source
`285e84c5ff2d3ecef25890da07f578af6a0c5ca6`. It preserves source patterns, voices,
steps and all automation lanes. Native bindings cover the two qualified VCA/filter
controls above; unsupported targets remain explicitly unbound. Its mapping is
labeled a musical role because native scheduling and healthy repeated rebinding
are deliberate host corrections. [Contract and limits](RACKMCP_SEQUENCE_CONTRACT.md#source-import-and-automation-authority).

`rackmcp_transport` controls imported sequencers through the live callback queue;
`inspect_rackmcp_sequence` reports actual transport/status or explicit unavailable.
Saved native state captures desired selection and effective running without
rewriting the retained original VCV package. [Transport and persistence](RACKMCP_SEQUENCE_CONTRACT.md#live-transport-and-saved-selection).

Imported automation checks the exact source package, original numeric target ID,
registered mapping and native control. Reusing a referenced native ID for an
unrelated module rejects the edit atomically; missing targets keep dormant
bindings. Clear the binding before replacing that target, or use a fresh ID for
the new module. Original source data remains intact through edits and saves.

Third-party plugins, RackMCP Bridge and Core audio interfaces still require additional mappings or an explicit substitute. Existing VCV binaries are not hosted by this importer.

## MCP boundary

- `import_vcv`: bounded workspace-only `.vcv` import with mapping report.
- `inspect_document`: execution status and a compact, non-loadable reference; request source data for the complete document.
- `substitute_unresolved_module`: explicit native kind, parameters and source-to-native input/output port maps.
- `remove_unresolved_module`: undoable removal of the placeholder and its routes.
- `save_document` / `load_document`: portable document with source assets and layout.
- `set_document_playback`: explicit Exact/Partial permission, shared with history.

Desktop and MCP use the same Session-owned document. DSP receives a derived native graph, never fake unknown modules. Revision/generation fences suspend processing and observations for unresolved Exact documents, including the first callback. Retained signal buffers are not presented as live measurements while suspended.

## Acceptance evidence

The user-supplied “So There I Was” fixture now imports on the milestone branch as **55 modules and 83 cables: 32 native mappings and 23 unresolved dependencies**. This includes its one version-specific VCO, four VCFs, both RackMCP sequencers and Listen. All 13 automation lanes survive; ten bind to qualified VCA/filter controls and three Surge targets remain unbound. Its embedded asset and original JSON bytes survive conversion and native save/reopen; the source file remains unchanged. This proves preservation and the supported subset, not sonic equivalence of the whole patch.

Focused checks cover source/archive limits, repair and undo, native migration/save/recovery/templates, stale UI/MCP edits, layout publication and startup suspension. Headless normal/dim captures use the real shared renderer without opening an audio device. Working logs and captures are recorded in the ignored `native-test-output/rack-import` state; [Full macOS CI](https://github.com/dexterlabs1/openpatch/actions/runs/39016180431) passed 2,378 Rust tests and 117 Python/MCP tests plus packaging and visualizer integration. Installation remains a separate delivery step.


## Listen capture on the milestone branch

`RackMCP / Listen / 2.0.0` maps to the separate native `rackmcp_listen`.
Raw polyphonic L/R passthrough and source-derived analysis have exact bounded
reference checks. Live capture uses a native host-owned SPSC ring, acknowledged
reuse and explicit failure on instance removal/suspension, so its mapping declares
host corrections rather than whole-host equivalence. Unknown parameter slots or
custom source data remain unresolved. Import never starts recording or a device.
Use the module's Capture/Inspect controls or `start_listen_capture`,
`inspect_listen_capture`, `cancel_listen_capture` and `inspect_listen` over MCP.
[Behavior, checks and limits](benchmarks/rackmcp-listen.json).
