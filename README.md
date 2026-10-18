<p align="center">
  <img src="docs/images/openpatch-banner.svg" alt="OpenPatch — patch sound, shape motion" width="100%">
</p>

<p align="center">
  A native modular synthesizer for macOS, Windows and Linux.<br>
  Compose with the Patch Engine. Shape the sound in a living rack.
</p>

<p align="center">
  <a href="#download-openpatch-030">Download 0.3.0</a> ·
  <a href="#patch-engine">Patch Engine</a> ·
  <a href="#play-and-shape-the-rack">Explore the rack</a> ·
  <a href="docs/MCP.md">Claude &amp; Codex</a> ·
  <a href="#scope-and-status">Status</a>
</p>

![OpenPatch with the Patch Engine, Studio Mixer and a Warm Downtempo rack](docs/images/patch-engine-steamdeck.png)

## Download OpenPatch 0.3.0

**Version 0.3.0 · build 10.** Desktop app and command-line/MCP tools are included in every package.

| Platform | Download | Launch |
|---|---|---|
| macOS 12+ · Apple silicon | [OpenPatch-macos-arm64.zip](https://github.com/dexterlabs1/openpatch/releases/download/v0.3.0/OpenPatch-macos-arm64.zip) | Move `OpenPatch.app` to Applications. See the signing note below. |
| Windows PC · x86_64 | [OpenPatch-windows-x86_64.zip](https://github.com/dexterlabs1/openpatch/releases/download/v0.3.0/OpenPatch-windows-x86_64.zip) | Open `openpatch-desktop.exe` in the extracted folder. |
| Steam Deck / Linux · x86_64 | [OpenPatch-linux-x86_64.tar.gz](https://github.com/dexterlabs1/openpatch/releases/download/v0.3.0/OpenPatch-linux-x86_64.tar.gz) | Run `./install.sh` from the extracted folder, then launch **OpenPatch** from the application menu. |

1. Download the package for your machine.
2. Extract the entire archive; keep the package files together.
3. Follow the launch instructions above, then press **F1** to open the Patch Engine.

The macOS app is ad-hoc signed and is not Developer ID signed or notarized. The Windows package is unsigned and runs as a portable application. Linux installs in `~/.local` without root access and preserves user patches and settings; it can also run directly as `./openpatch-desktop`.

On Steam Deck, install from **Desktop Mode → Konsole**. The package's `STEAM-DECK.txt` explains adding OpenPatch to Steam and configuring Steam Input. For example, after extracting into Downloads:

```sh
cd ~/Downloads/OpenPatch-linux-x86_64
./install.sh
```

If upgrading from an older Patch Engine preview, run this installer once to install the new updater.

Update later with `~/.local/bin/openpatch-update` or **Update OpenPatch** in the application menu. Restart the app after updating. [All releases and notes →](https://github.com/dexterlabs1/openpatch/releases)

## Patch Engine

Choose a **genre**, **mood** and **texture** to compose a piece and build it in the rack. Keep the seed to rebuild its composition, then edit the modules and cables as your own patch.

- **Eight genres:** Deep House, Dub Techno, Techno, Ambient, Lo-fi Hip Hop, Synthwave, Downtempo and UK Garage, plus a separate C418-inspired mode for original, spacious piano pieces.
- **Eight moods and eight textures:** combine choices such as Dreamy, Dark or Warm with Liquid, Dusty or Minimal. The seed determines harmony, patterns, layers and arrangement.
- **Reveal, Showtime or Instant:** choose how the rack takes shape. Construction uses real edits and shared undo history.
- **Light mode:** limits optional layers using the composition's estimated cost budget. It starts enabled on Linux; it is not a device-specific performance guarantee.
- **Keyboard or touch:** **F1** opens/closes the panel; **Up/Down** chooses a row, **Left/Right** changes an option, **Enter** builds, **Space** or **/** surprises, and **Esc** closes. These keys apply outside text fields.

For Steam Input, explicitly map the D-pad to arrow keys, **A → Enter**, **X → Space**, **B → Esc**, and a back button to **F1**. Touch controls work directly. **PageUp/PageDown** zoom, **Home** resets the view, and **F11** toggles full screen.

The bundled CLI can generate the same seeded composition:

```sh
openpatch patch-engine "Deep House" Dreamy Liquid --seed 7 --info
```

Use the executable's path if it is not on your PATH. `--reveal` and `--showtime` export construction plans.

## Play and shape the rack

Signal Atlas panels put large controls, readable values, live lamps and scopes beside the patch. The library and rack share the same panel rendering.

- **Patch and arrange:** connect jacks, stack signals, clone modules, type precise values and save presets. **Organize** offers automatic layout or one to eight rows, with undo.
- **Follow a route:** toggle **Signal path** in the toolbar or press **P** while the rack is idle. Adjust cable colours, materials, tension and transparency to keep a busy patch readable.
- **Keep tools nearby:** the footer's **DSP** and **MCP** controls open anchored panels for performance and session details.
- **Mix and animate:** [Studio Mixer](docs/STUDIO_MIXER.md) supplies up to 16 channels and two stereo FX loops. **Horizon, Phosphor, Ridges, Pulsar and Vector** draw the signal as lines and trails; Aurora, Spectrum, Waveform and **Ferrofluid** add other views. Edit **GLSL** in the app while the last working shader stays visible. [Visualizer guide →](docs/VISUALIZERS.md)
- **Work with agents:** Claude and Codex can inspect, patch, automate, save and render through MCP. Watch modules, cables and parameter changes animate in the same rack, with shared undo, paced construction and optional camera follow. [Construction →](docs/CONSTRUCTION.md) · [Motion controls →](docs/CONSTRUCTION_MOTION.md)

| Everyday action | Control |
|---|---|
| Play / pause outside Patch Engine | **Space** or the transport button |
| Open the module library | **Enter**, the library button, or right-click empty rack space |
| Patch / unplug | Drag between jacks; Shift-click a jack to delete its top cable |
| Edit an exact value | Right-click its control or click its printed value |
| Select / duplicate | **Shift-click** headers; **Cmd/Ctrl-D** duplicates |
| Save / open | **Cmd/Ctrl-S** / **Cmd/Ctrl-O** |
| Undo / redo | **Cmd/Ctrl-Z** / **Cmd/Ctrl-Shift-Z** |

[Full rack controls](docs/RACK_EDITING.md) · [Projects and recovery](docs/PROJECTS.md) · [Signal scope](docs/SCOPE.md)

## Five creative instruments included

These native, source-derived instruments have their own panels and declared host behavior. Their linked contracts explain the supported modes, examples and fidelity limits.

| Instrument | Explore |
|---|---|
| [Tidal Functions](docs/TIDAL_FUNCTIONS.md) | Four related functions, envelopes and audio-rate waveforms, with clock ratios and shape controls. |
| [Frame Scene](docs/FRAME_SCENE.md) | Four-channel scene control and mixing with up to 64 keyframes and editable transitions. |
| [Segment Garden](docs/SEGMENT_GARDEN.md) | Six RAMP, STEP and HOLD stages that form functions, envelopes and sequences through gate grouping. |
| [Event Fork](docs/EVENT_FORK.md) | Two seeded Bernoulli gate routers with probability, toggle and latch controls. |
| [Rhythm Terrain](docs/RHYTHM_TERRAIN.md) | Three-lane drum maps and Euclidean rhythms, with fill, accents, clocking and swing. |

Factory examples are included in each release's `examples` folder. The broader [creative-instrument plan](docs/CREATIVE_MODULES_20.md) remains in progress.

## Connect Claude or Codex

Open the **MCP** panel and follow the [MCP setup guide](docs/MCP.md). On macOS and Linux, the bundled `openpatch attach` CLI connects clients to an existing window; multiple clients share its live document and undo history. Windows supports launching a desktop MCP session with `openpatch-desktop.exe --mcp`; attachment to an existing Windows window remains separate work.

The CLI also supports headless MCP and offline rendering. [Session controls](docs/MCP_SESSIONS.md) describe connection behavior and revision checks.

## Scope and status

OpenPatch 0.3.0 contains **91 native module kinds**, including the five creative instruments above. This catalog count does not establish complete VCV Rack parity. Native audio-device and Steam Deck game-mode behavior depend on the host setup and remain distinct from automated application checks.

[VCV patch import](docs/VCV_IMPORT.md) converts explicitly supported source identities into native modules while retaining source data, assets and unresolved dependencies. Unsupported modules remain visible and recoverable; partial playback requires an explicit choice. Consult the supported mapping before relying on a patch. **Existing VCV plugin binaries are not hosted by this release.** [Compatibility boundary →](docs/COMPATIBILITY_DECISION.md)

[Current work plan](docs/WORKPLAN.md) · [Roadmap](docs/ROADMAP.md) · [MIDI and audio scope](docs/MIDI_AUDIO.md)

## Build from source

Install stable Rust and the native platform build tools. macOS needs Xcode Command Line Tools; Linux needs a C/C++ toolchain, `pkg-config` and the ALSA, udev, xkbcommon, Wayland and OpenGL development packages (see the [CI dependency list](.github/workflows/ci.yml)). Python 3.11+ is used for packaging.

```sh
git clone https://github.com/dexterlabs1/openpatch.git
cd openpatch
git checkout v0.3.0
cargo build --locked --release -p openpatch-desktop -p openpatch --bins
cargo run --locked --release -p openpatch-desktop
```

The desktop and CLI binaries are in `target/release` (`.exe` on Windows). On macOS, `python3 scripts/package_macos.py` creates `dist/OpenPatch.app`, including the CLI. `scripts/package_release.py` packages prebuilt binaries for each supported release target.

## License and source

Original OpenPatch code is [MIT licensed](LICENSE). Combined binaries include GPL-adapted processors and are distributed under [GPL-3.0-or-later](crates/openpatch-core/LICENSE-GPL-3.0.txt). [Third-party notices and source pins](crates/openpatch-core/LICENSE-THIRD-PARTY), font notices and a corresponding-source link ship with every package.

The [v0.3.0 source tree](https://github.com/dexterlabs1/openpatch/tree/v0.3.0) and [release source archives](https://github.com/dexterlabs1/openpatch/releases/tag/v0.3.0) provide the matching release source. OpenPatch uses original native artwork and is independent of VCV, with no affiliation or endorsement.
