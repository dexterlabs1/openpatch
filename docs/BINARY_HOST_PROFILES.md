# Existing VCV modules: Mac development profiles

The development source can run the existing **Bogaudio 2.6.47 OFFSET and XFADE**
Mac ARM binaries with the genuine **Rack SDK 2.6.6** runtime. OpenPatch 0.3.1 build
11, currently installed, includes only OFFSET. This work does not update that app.

XFADE blends two audio signals. Its original plugin performs the audio processing;
OpenPatch supplies the native panel, cables, shared history and MCP controls.
The original Rack C++ interface widgets are not hosted.

## What is supported

| Profile | Controls | Inputs | Output |
|---|---|---|---|
| OFFSET | Offset, Scale; offset-first and output-limit options | Offset CV, Scale CV, Signal | Signal |
| XFADE | Mix −1…1, Curve 0…1, Linear 0 or 1 | Mix CV, Signal A, Signal B | Signal |

XFADE requires Signal A to be connected; it determines the voice count, as in the
original plugin. Signal B and Mix CV can be disconnected. A mono Mix CV controls
all voices. Only the qualified default custom-state object is accepted for
XFADE. Other state is preserved in documents but cannot be executed silently.

This remains an explicit, limited host: one hosted module in a patch, at 48 kHz,
with 128-frame callbacks and a 384-frame transport delay. Native modules can
surround it. It does not load arbitrary plugins or scan your plugin folders.
Opening a file never starts plugin execution. Plugin execution is isolated in a
child process; that process is not a security sandbox for untrusted code.

## Try the development version

Build OpenPatch from this source. Supply your own matching plugin and SDK;
neither is included in the app. First build and check the companion:

```sh
python3 scripts/rack_binary_host/run_transport.py --gate xfade \
  --sdk /path/to/Rack-SDK --plugin /path/to/Bogaudio \
  --state scripts/rack_binary_host/fixtures/xfade-default-state.json \
  --output native-test-output/xfade
```

The result names a new evidence directory containing `companion`. Start the
matching development app with explicit paths:

```sh
OPENPATCH_HOSTED_COMPANION=/path/to/evidence/companion \
OPENPATCH_HOSTED_SDK=/path/to/Rack-SDK \
OPENPATCH_HOSTED_PLUGIN=/path/to/Bogaudio \
target/release/openpatch-desktop
```

Open [the quiet XFADE example](../examples/experimental-hosted-xfade.json), review
the hosted panel, choose **Start experimental hosted playback**, and press Play.
The example pins specific plugin/runtime hashes. Different bytes require a
separately qualified identity, rather than silently replacing the saved plugin.

The app and companion must both use transport **v2**. The older 0.3.1 app uses
v1; mixing the two versions fails closed. Existing OFFSET documents retain their
identity and persistent-state format.

## MCP and history

Use `set_hosted_parameter` with `id`, `parameter`, `value`, and the current
`expected_revision`. XFADE parameter names are `mix`, `curve` and `linear`.
`linear` accepts only numeric 0 or 1. Panel and MCP edits use the same history,
acknowledged event queue and save barrier. Ordinary native edits keep the live
child and its transient state when the existing graph-continuity checks pass.

Performance inspection names the capability. Requested/applied parameter values
are a fixed three-element array in the table's parameter order; OFFSET leaves
the third element zero. Model changes, loads and unsupported custom-state edits
cannot reuse an existing execution grant.

## Validation and limits

[The qualification record](benchmarks/rack-binary-xfade.json) links the source
profiles, genuine snapshot and test scope. The headless checks cover A/B
endpoints, linear midpoint, curve and mode changes, mono mix CV across 16 voices,
shared edit/undo/redo, exact persistent capture and fresh-process restoration.
The actual desktop-controller check covers all three parameters, native-edit
continuity and coherent MCP save, with zero parent callback heap activity.

The real-child rejection check exercises wrong/unknown profiles, old transport,
reserved header bytes, fractional linear mode and out-of-range curve values.
Existing OFFSET transport, failure-containment and restoration checks are retained.

Persistent save/reopen is not a sample-exact checkpoint of XFADE's slew history.
Timing, device and source qualifications are recorded separately. There is no
claim of broad VCV compatibility, hardware-model accuracy or a performance
advantage over Rack.
