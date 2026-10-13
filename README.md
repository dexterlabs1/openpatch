<p align="center">
  <img src="docs/images/openpatch-banner.svg" alt="OpenPatch — patch sound, shape motion" width="100%">
</p>

<p align="center">
  A native modular synthesizer in Rust.<br>
  Build a voice, follow the voltage, and make it move.
</p>

<p align="center">
  <a href="https://github.com/dexterlabs1/openpatch/actions/workflows/ci.yml"><img src="https://github.com/dexterlabs1/openpatch/actions/workflows/ci.yml/badge.svg" alt="Build and test"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-8bb9a6" alt="MIT license"></a>
  <a href="Cargo.toml"><img src="https://img.shields.io/badge/built_with-Rust-dea584" alt="Built with Rust"></a>
</p>

<p align="center">
  <a href="#get-started-on-macos">Get started</a> ·
  <a href="#play-the-rack">Controls</a> ·
  <a href="docs/MCP.md">Claude &amp; Codex</a> ·
  <a href="#where-it-stands">Project status</a> ·
  <a href="#go-deeper">Documentation</a>
</p>

![OpenPatch native rack with a VCO, ladder filter, OTA VCA, ADSR, gate clock, and audio output](docs/images/openpatch-rack.png)
<p align="center"><sub>Native desktop app, rendered on Linux with Mesa in CI. macOS is the primary design target.</sub></p>

- **An instrument you can patch.** Turn rotary controls, connect cables, and arrange modules on rack rails. Live edits preserve oscillator phase and circuit memory.
- **A reference analog voice.** Explore a nonlinear transistor ladder, OTA VCA, and switched RC envelope alongside an idealized ramp VCO, gate clock, and voltage/audio interface. Each model has [documented assumptions and validation](docs/ANALOG_MODELS.md).
- **Sound into motion.** Open the optional Aurora, Spectrum, or Scope dock. Edit GLSL shaders in the app while the last working shader keeps running.
- **A rack agents can work with.** Claude and Codex can inspect, preview batch edits, patch, undo/redo, analyze individual output channels, tune, save, and render through a local MCP server using the same validated controls as the UI.

## Get started on macOS

For Apple Silicon, install stable [Rust](https://rustup.rs), Xcode Command Line Tools (`xcode-select --install`), and Python 3. Build a native application bundle:

```sh
git clone https://github.com/dexterlabs1/openpatch.git
cd openpatch
cargo build --locked --release -p openpatch-desktop
python3 scripts/package_macos.py
open dist/OpenPatch.app
```

The app opens a connected reference voice with playback paused. Press **Play** or **Space** to hear it. To run directly from source, use `cargo run --locked --release -p openpatch-desktop`.

This is a **v0.2 development preview**, built from source. The local `.app` is ad-hoc signed; it is not notarized. CI also builds Linux and Windows targets; platform notes are below.

## Play the rack

| Action | Control |
|---|---|
| Play / pause | **Space** |
| Add a module | **Enter**, **+ Add module**, or right-click empty rack space |
| Patch / unplug | Drag or click between input and output in either direction; drag a connected input to move its cable; right-click an input to unplug |
| Adjust a control | Drag vertically; hold **Cmd** for fine adjustment; double-click to reset |
| Type a value | Double-click the displayed value |
| Arrange / remove | Drag a module header; right-click a module for options, or **Delete** the selection |
| Pan / zoom | Scroll or middle-drag; pinch or **Cmd-scroll** to zoom; **Cmd-0** resets the view |
| Save / open / undo / redo | **Cmd-S** / **Cmd-O** / **Cmd-Z** / **Shift-Cmd-Z** |

Compatible jacks glow and snap during patching. Dropping onto an occupied input replaces its cable atomically; **Esc** or an invalid drop keeps the existing connection. The quantizer’s piano keys toggle scale notes. One knob drag is one undo step. Split and Merge expose all 16 ports; right-click an output jack to select that specific output for monitoring. Polyphonic playback uses the first channel unless you route through SUM.

Use **Ctrl** in place of Cmd on Linux and Windows. Keyboard shortcuts apply outside text fields. **Esc** cancels patching or closes an editor.

Patches live in `~/OpenPatch` by default. **Save as** creates a new JSON file and an adjacent layout sidecar for module positions and the view. Existing patch files are kept; choose a new name to save another version.

<details>
<summary><strong>See the visualizer dock</strong></summary>

![OpenPatch native rack with the audio-reactive visualizer dock open](docs/images/openpatch-visuals.png)

<sub>Linux / Mesa CI render. The dock uses native OpenGL; it requires a compatible GPU and driver.</sub>

[Visualizer modes and shader interface →](docs/VISUALIZERS.md)

</details>

<details>
<summary><strong>Try a clocked random voice</strong></summary>

![Clocked random voice with seeded noise, sample-and-hold, and a piano-key quantizer](docs/images/openpatch-utilities.png)

Choose **File → Clocked random voice**. Noise supplies candidate pitches; a clock
samples them, and the quantizer selects notes from a C-major pentatonic scale.
Click its piano keys to change the scale, or edit the seed for another repeatable
sequence. This is an original OpenPatch patch using the documented utility
contracts, not an imported VCV project.

<sub>Native Linux / Mesa CI render.</sub>

</details>

## Claude & Codex

Follow the [MCP setup guide](docs/MCP.md) for Claude Code, Claude Desktop, or Codex configuration. The client launches OpenPatch with `--mcp` and controls the live patch in that window. Press **Play** locally to audition changes; agents can also export WAV files without speaker playback.

Each launch owns its own app session. The separate CLI supports headless MCP, offline rendering, and visualizer export.

## Where it stands

**Parity is the baseline:** match VCV Rack Free 2 host functionality and the 49 Core / Free modules, then extend usability, performance, visuals, and agent control. The [parity roadmap](docs/ROADMAP.md) tracks concrete acceptance gates. OpenPatch has not reached parity yet.

**Twenty-two built-in kinds:** six reference-voice modules, five voltage utilities, four polyphonic utilities, and seven legacy modules. Try **File → Polyphonic voice** for a three-note chord, or **File → Clocked random voice** for a repeatable pentatonic patch. The browser distinguishes model classes. The [module coverage plan](docs/MODULE_COVERAGE.md) tracks **49 VCV Core / Free targets**; it is a roadmap, not 49 implemented equivalents.

macOS ARM is the current product focus. CI builds and tests **macOS ARM64, Linux x86-64, and Windows x86-64**, including native window and shader checks on Linux. Audio devices and GPU drivers need broader hands-on testing.

New patches use the [version-2 engine](docs/ENGINE_V2.md): named outputs, up to 16 channels per cable, and feedback with one sample of delay per cable. Old patches retain their original mono timing until explicitly converted. Hardware output and WAV export currently use the first channel; **SUM** mixes polyphonic voices to mono. MIDI, device selection, multichannel hardware audio, VCV patch/plugin compatibility, and broader module coverage remain ahead.

**Performance goal:** low CPU use and predictable audio deadlines on large patches. Callback tests check allocation-free processing and graph swaps. A speedup over VCV Rack has **not** been demonstrated; see the [measurement methodology](docs/PERFORMANCE.md).

## Go deeper

| Guide | What it covers |
|---|---|
| [Engine v2](docs/ENGINE_V2.md) | Named ports, polyphony, feedback timing, state migration, and legacy compatibility |
| [Analog models](docs/ANALOG_MODELS.md) | Circuit equations, voltage conventions, reference comparisons, and limits |
| [Parity roadmap](docs/ROADMAP.md) | Host functionality, acceptance gates, delivery order, and MCP expansion |
| [Module coverage](docs/MODULE_COVERAGE.md) | The 49 Core / Free targets and acceptance criteria |
| [MCP integration](docs/MCP.md) | Client configuration, tools, revision checks, and file safety |
| [Visualizers](docs/VISUALIZERS.md) | Native rendering, shader uniforms, and optional HTML export |
| [Architecture](docs/ARCHITECTURE.md) | DSP, audio/control thread boundaries, persistence, and roadmap |
| [Performance](docs/PERFORMANCE.md) | Benchmarks and fair comparisons |

<details>
<summary><strong>Linux, Windows, and contributor checks</strong></summary>

Linux requires ALSA, udev, XKB, Wayland, and OpenGL development libraries. On Debian / Ubuntu:

```sh
sudo apt-get install libasound2-dev libudev-dev libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libgl1-mesa-dev
cargo run --locked --release -p openpatch-desktop
```

Windows requires stable Rust with the MSVC toolchain and C++ build tools. Run the same Cargo command; use `python` instead of `python3` for repository scripts.

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo build --locked --release --workspace
python3 -m unittest discover -s scripts -v
cargo bench --locked -p openpatch-core
```

See the [CI workflow](.github/workflows/ci.yml) for the platform build matrix and native smoke tests.

</details>

---

[MIT licensed](LICENSE). Original Rust code and artwork. OpenPatch is independent of VCV and is not affiliated with or endorsed by VCV. Dependency licenses remain with their respective authors.
