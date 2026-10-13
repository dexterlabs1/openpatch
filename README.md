# OpenPatch

An open-source modular synthesizer in Rust, with MCP control designed into its core.

**Early developer prototype.** You can build patches through Claude or Codex,
render WAV audio, and measure DSP throughput. A visual rack and live audio are
the next milestones. OpenPatch aims to improve performance on large modular
patches; it has **not yet been benchmarked against VCV Rack**.

## Platforms

| Platform | Initial target | Validation |
| --- | --- | --- |
| Linux | x86-64 | Native CI build, Rust tests, MCP integration tests |
| Windows | x86-64 (MSVC) | Native CI build, Rust tests, MCP integration tests |
| macOS | Apple Silicon / ARM64 | Native CI build, Rust tests, MCP integration tests |

The CI workflow builds downloadable binaries for all three targets. These are
unsigned development builds. Audio devices and desktop UI are not implemented
yet, so platform support currently covers the engine, CLI, and MCP server.

## Try it

Install stable Rust from [rustup.rs](https://rustup.rs). On Windows, install the
MSVC C++ build tools; on macOS, install Xcode Command Line Tools. Then:

```sh
git clone https://github.com/dexterlabs1/openpatch.git
cd openpatch
cargo build --release
cargo run --release -- demo demo.wav
cargo run --release -- render examples/tremolo.json tremolo.wav 5
cargo run --release -- visualizer visualizer.html
```

Open the resulting WAV in your audio player. Existing files are never overwritten.
The executable is `target/release/openpatch` (`openpatch.exe` on Windows).

Open `visualizer.html` in a browser and select your rendered WAV for the bundled
Aurora, Spectrum, and Oscilloscope visuals. You can edit and compile GLSL live.
This is a standalone visual preview; native visuals will be integrated with the
live instrument. See [visualizer architecture](docs/VISUALIZERS.md).

## Agent control

The Rust binary is also a local stdio MCP server. It requires no API key or model
SDK: your MCP client provides the model. Start it with an explicit file workspace:

```sh
openpatch mcp --workspace /absolute/path/to/patches
```

Read [the Claude and Codex setup guide](docs/MCP.md) for configuration examples.
Try asking: “Use OpenPatch to create a 440 Hz sine oscillator, modulate its volume
with a 3 Hz LFO, render five seconds, and report the peak and RMS.”

Tools: `catalog`, `inspect_patch`, `replace_patch`, `add_module`, `remove_module`,
`connect`, `disconnect`, `set_parameter`, `set_output`, `save_patch`, `load_patch`,
`render_audio`, `export_visualizer`, and `benchmark`.

Each MCP process owns one independent patch. Edits validate before committing,
with optional revision checks. Files are confined to the configured workspace
and never overwritten. Rendering starts from reset and does not play speakers.

## Current engine

- Sine oscillator, LFO, constant, gain/VCA, two-input mixer, one-pole low-pass, output.
- Sample-accurate modulation within a compiled, deterministic graph.
- Preallocated 128-frame buffers; DSP processing does not allocate, lock, or do I/O.
- JSON patches with stable module IDs, strict validation, mono PCM16 WAV export.
- Mono acyclic graphs for now. Feedback, polyphony, MIDI, and VCV plugins are future work.

## Development

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --release
python3 -m unittest discover -s scripts -v
cargo bench -p openpatch-core
```

On Windows use `python` instead of `python3`. The Python integration tests use
only the standard library. [Architecture and roadmap](docs/ARCHITECTURE.md)
explain the control/DSP boundary and next milestones.
[Benchmark methodology](docs/PERFORMANCE.md) defines how performance will be measured.

## License and independence

MIT. This is original code, not a translated or copied VCV Rack codebase.
OpenPatch is independent and is not affiliated with or endorsed by VCV.
It does not currently load VCV Rack plugins or patches, and uses no VCV branding
or panel graphics. Third-party code contributions must preserve their licenses.
