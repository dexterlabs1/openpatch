# Experimental original Rack plugin host

This is the first compatibility milestone for an on-demand module library.
It loads an **unmodified, already-installed Rack 2 plugin binary** in a separate
process, instantiates one of its models, runs its original DSP, and saves/restores
Rack module JSON. It does not replace the native engine or modify the desktop
app, its current patch, installed plugins, Rack settings, or `/tmp/Rack2`.

The qualification covers **Fundamental VCO 2.6.4** and **Audible Instruments
Plaits 2.0.0** on macOS ARM64 using the Rack 2.6.6 SDK. The plugin and runtime
hashes and measured results are in
[the qualification report](../../docs/benchmarks/rack-host-qualification.json).
This is an actual dynamic-plugin host, not a source extraction or algorithm port.

## Load original Plaits in the desktop

The desktop now has an experimental **RACK PLAITS** module. It uses the original
Audible Instruments binary through this separate worker, with OpenPatch controls,
seven polyphonic CV inputs, and Main/Aux outputs. It is distinct from the native
Plaits-inspired **MACRO VOICE** instrument.

```sh
# Build the current worker and write ~/OpenPatch/rack-host.json for the desktop.
python3 scripts/rack_host/host.py configure
```

The configuration identifies the worker executable, local SDK, plugin directory,
and Rack resource directory. The desktop reads this only when an original Plaits
module is loaded. `OPENPATCH_RACK_HOST_CONFIG` can select another configuration
path. Rerun `configure` after changing worker source, SDK, or plugin installation.

The native audio callback exchanges bounded atomic voltage frames with a
background service. Only that service communicates with the child process.
Audio uses variable-width little-endian binary frames; JSON is limited to block
controls and acknowledgements. The bridge adds **4,096 samples** of latency
(85.3 ms at 48 kHz). The status bar reports late samples and dropped input frames.
Worker failures produce silence and an error; no child I/O, mutex, allocation,
or blocking wait occurs in the audio callback. Worker state persists across
control edits and scope changes; retiring graphs cannot consume its stream.

A paced live test at 48 kHz exercises four original Plaits voices and a midstream
engine change. The qualified run produced **zero late samples and zero dropped
frames**. Separate allocation tests cover first callback, queue overflow,
missing output, and graph adoption. This is a first live bridge, not a claim of
low-latency performance or compatibility with the rest of the Rack library.
See the [live qualification report](../../docs/benchmarks/rack-plaits-live.json).
When Plaits polyphony grows, the host mutes only newly activated lanes for 256
samples while original old-width buffered frames retire; this prevents exposing
uninitialized lanes without changing the plugin binary or its DSP algorithms.

The module's exposed synthesis controls and CV wiring save in normal OpenPatch
projects. Worker-local DSP histories are not embedded in those project files.
Native offline WAV tools explicitly reject this live module; the standalone
worker renderer below remains available for original-plugin offline previews.

## Try it

Requirements: Python 3.11+, Apple C++ command-line tools, a locally installed
Rack 2 SDK, and trusted compatible plugin binaries. No Python packages are needed.
Run these commands from the repository root. Defaults match this development
machine; pass `--sdk /path/to/Rack-SDK` and `--system /path/to/Rack/resources`
**before** the subcommand on another installation.

```sh
# Read only the installed manifests; no module code is loaded.
python3 scripts/rack_host/host.py catalog --query Plaits

# Build the isolated worker against the existing SDK.
python3 scripts/rack_host/host.py build

# Load this plugin only, create Plaits, and inspect its real control metadata.
python3 scripts/rack_host/host.py describe \
  --plugin "$HOME/Library/Application Support/Rack2/plugins-mac-arm64/AudibleInstruments" \
  --model Plaits

# Original Main/Aux outputs become the two WAV channels, at 0.12 audio/volt.
python3 scripts/rack_host/host.py render \
  --plugin "$HOME/Library/Application Support/Rack2/plugins-mac-arm64/AudibleInstruments" \
  --model Plaits --seconds 4 --output /tmp/plaits.wav \
  --save-state /tmp/plaits.rack-module.json

# Restore the same plugin binary/runtime, parameters, custom data, and input CV.
python3 scripts/rack_host/host.py render \
  --plugin "$HOME/Library/Application Support/Rack2/plugins-mac-arm64/AudibleInstruments" \
  --model Plaits --state /tmp/plaits.rack-module.json \
  --output /tmp/plaits-restored.wav

# Run real-binary qualification and optionally export a harmonic-engine preview.
python3 scripts/rack_host/test_host.py --preview /tmp/original-plaits.wav
```

`catalog --plugins DIR` selects a different manifest directory. The default local
index contains 1,132 visible modules on the qualified machine; this is installed
content, not a claim of whole-library compatibility. Catalog search requires
neither the SDK nor building/running the worker. Build output stays in ignored
`target/rack-host/` and is keyed by adapter source, SDK library/headers, and compiler.

## Protocol and state

The client starts one worker with an explicit plugin directory, model slug,
sample rate, Rack system-resource directory, and disposable private user folder.
The worker uses Rack's genuine `init`, `Model::createModule`, module lifecycle,
`Module::process`, and engine serialization APIs. It registers only that plugin
with Rack's model registry. The one module has stable local ID 0. Plugin stdout
is redirected to the diagnostic log so it cannot corrupt the protocol.

JSON-line commands are `describe`, `set_params`, `set_inputs`, `process`, `save`,
and `restore`. Controls and input requests are fully validated before applying
them. `process` accepts 1–4,096 frames and explicit output IDs; every returned
sample contains the output's actual 0–16 voice channels. Inputs hold constant
voltage frames between requests. Changing connectivity dispatches Rack port
events. Rendering splits the request into bounded blocks and reports peak,
RMS, and clipping. WAV export selects one voice lane from one or two outputs.

The saved envelope identifies format/version, plugin/model/version, sample rate,
the plugin's own module JSON (including custom `data`), input voltage frames,
and SHA-256 hashes of the plugin binary and Rack library. Restore rejects
mismatches instead of silently changing the instrument. The wrapper is a prototype
module-preset format, **not** an `.openpatch` project or a complete Rack `.vcv` patch.

Rack module JSON preserves parameters and whatever custom state the plugin
serializes. It generally does not include oscillator phases, delay buffers, or
every other DSP history. Preset restoration is tested across fresh processes;
sample-exact continuation after a process restart is not claimed.

The client bounds response size and request time, terminates a stalled worker,
and reports process failure. A failed render leaves an existing output file
intact. All worker cleanup and serialization happen outside a realtime callback.

## Qualification and remaining work

The nine real-binary tests cover manifest-only browsing, original VCO frequency
and polyphonic widths at 44.1/48/96 kHz, callback-partition-independent samples,
Plaits engine differences, custom state and four-voice restoration, rejected
malformed/control/state requests, child-process failure and recovery, and WAV
export. The test run verifies that installed plugin binaries remain unchanged.

The standalone CLI remains an offline test interface. The broader integration
does **not** yet provide:

- Original NanoVG module panels or a remote desktop module browser.
- A Rack graph with cables, expanders, inter-module messages, or graph timing.
- A low-latency bridge or qualification beyond the live Plaits test above.
- MIDI, Rack patch storage, sample-backed module assets, or global plugin settings.
- A remote module index, plugin downloads, accounts, purchases, or cache eviction.
- Compatibility claims for other plugins, SDK builds, or operating systems.

Production integration should host a Rack graph in the worker, improve latency
and recovery, and preserve the native callback's nonblocking contract. The
synchronous CLI transport must never run in that callback. Original panels need
a separate UI bridge; parameter metadata
alone cannot reproduce arbitrary custom widgets.

The on-demand library can then keep searchable metadata separate from executable
packages, resolve a selected module to its plugin package, retain exact versions
needed by saved projects, and download/cache only approved packages. It must use
permitted distribution sources; this prototype does not access VCV accounts or
assume permission to reuse VCV's download service.

## Licenses and upstream

`worker.cpp` is original GPL-3.0-or-later host code; see [COPYING](COPYING).
`host.py` and `test_host.py` are original MIT-licensed client/test code covered by
the repository's root license. The GPL worker is optional and is not linked into
or included with the MIT native desktop build. Rack and plugin binaries are
used from the local installation and are not copied into the repository or app.
Any distributed Rack-based worker/packages must carry the applicable licenses,
source obligations, notices, and asset permissions.

Reference material:

- [Rack loader](https://github.com/VCVRack/Rack/blob/v2/src/plugin.cpp)
- [Rack module API](https://vcvrack.com/docs-v2/structrack_1_1engine_1_1Module)
- [Rack licensing](https://github.com/VCVRack/Rack/blob/v2/LICENSE.md)
- [Fundamental](https://github.com/VCVRack/Fundamental)
- [Audible Instruments](https://github.com/VCVRack/AudibleInstruments)
- [VCV Library database](https://github.com/VCVRack/library)

