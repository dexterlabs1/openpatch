# OpenPatch

An open-source **native desktop modular synthesizer in Rust**, with live audio,
GPU visualizers, and MCP control for Claude and Codex.

**Early prototype, v0.2.** OpenPatch now has a native rack window: add modules,
adjust parameters, connect cables, and hear the patch through your audio device.
Aurora, Spectrum, and Scope visuals run inside the app with an editable GLSL shader.
It aims to handle large modular patches efficiently; it has **not yet been
benchmarked against VCV Rack**.

## Launch the desktop app

Install stable Rust from [rustup.rs](https://rustup.rs). On macOS, install Xcode
Command Line Tools; on Windows, install the MSVC C++ build tools. Linux needs
ALSA, udev, XKB, Wayland, and OpenGL development packages:

```sh
# Debian / Ubuntu only
sudo apt-get install libasound2-dev libudev-dev libxkbcommon-dev libwayland-dev libgl1-mesa-dev
```

```sh
git clone https://github.com/dexterlabs1/openpatch.git
cd openpatch
cargo run --release -p openpatch-desktop
```

The app opens a tremolo patch with playback paused. Press **Play** (or Space).
Click **OUT**, then an input jack to connect a cable. Right-click an input to
unplug it. Select a module header to remove it or choose it as the audio output.
Use the library to add modules; change sliders to hear the result live.

Patch files live in `~/OpenPatch` by default. **Save new** writes a new JSON file;
existing files are never overwritten. Select a saved filename and press **Load**.
**Undo last edit / load** recovers the previous displayed patch. Closing with
unsaved edits prompts you to keep working or discard. There is no autosave yet.

On macOS, create a double-clickable application bundle:

```sh
cargo build --release -p openpatch-desktop
python3 scripts/package_macos.py
# The native application is now dist/OpenPatch.app
```

The bundle is locally ad-hoc signed, not notarized. CI builds both desktop and
CLI binaries for **Linux x86-64, Windows x86-64, and macOS ARM64**, plus a zipped
Mac app. Hardware audio and GPU verification currently covers an Apple Silicon
Mac; other platforms are compile/test targets pending device testing.

## Visualizers

The native **Visual Field** has Aurora, Spectrum, and Scope modes driven by the
live audio output. **Edit GLSL** opens the shader editor. Compile errors leave the
last working shader running. The visualizer uses desktop OpenGL / GLSL 330 core,
not a browser or webview. It requires a compatible GPU/driver.

The older standalone HTML visualizer is still available as an optional export.
See [visualizer details](docs/VISUALIZERS.md).

## Agent control

Launch the desktop executable as an MCP server to give Claude or Codex control
of **the patch in that same window**:

```sh
/path/to/openpatch-desktop --mcp --workspace /absolute/path/to/patches
```

Use this command in your MCP client configuration; stdout is JSON-RPC only.
Each launch creates its own window and session. It does not attach to an
already-running app. Press Play locally to hear agent changes. The UI and MCP
share validation and revision checks; the audio callback never takes their lock.
Read [the Claude and Codex setup guide](docs/MCP.md) for configuration examples.

The separate CLI continues to support offline/headless MCP and WAV rendering:

```sh
cargo run --release -p openpatch -- mcp --workspace /absolute/path/to/patches
cargo run --release -p openpatch -- demo demo.wav
cargo run --release -p openpatch -- render examples/tremolo.json tremolo.wav 5
cargo run --release -p openpatch -- visualizer visualizer.html
```

## Engine and current limits

- Seven built-in modules: sine oscillator, LFO, constant, VCA, two-input mixer,
  one-pole low-pass, and output.
- Deterministic acyclic mono graph, preallocated 128-frame DSP buffers.
- Native audio at the device's sample rate; mono is duplicated across its channels.
- Bounded graph exchange and reclamation outside the callback, a 5 ms graph
  crossfade, and smoothed transport/master gain. Graph edits currently reset DSP
  state; this is not yet a phase-preserving automation system.
- Callback tests check zero allocations and deallocations during processing and
  graph swaps. Spectrum analysis and GPU work happen on the UI thread.
- Default audio device only; device selection, MIDI, polyphony, feedback/delays,
  plugin loading, and VCV patch compatibility are future work.
- Native MCP controls patches; transport, shaders, and master volume are local
  controls for now. GPU driver behavior and audio latency need wider testing.

## Development

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo build --locked --release --workspace
python3 -m unittest discover -s scripts -v
cargo bench -p openpatch-core
```

On Windows use `python` instead of `python3`.
[Architecture](docs/ARCHITECTURE.md) describes thread boundaries and the roadmap.
[Performance methodology](docs/PERFORMANCE.md) explains the benchmark limits.

## License and independence

MIT. This is original code, not a translated or copied VCV Rack codebase.
OpenPatch is independent and is not affiliated with or endorsed by VCV.
It does not load VCV Rack plugins or patches, and uses no VCV branding or panel
artwork. Third-party dependencies retain their respective licenses.
