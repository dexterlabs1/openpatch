<p align="center">
  <img src="docs/images/openpatch-banner.svg" alt="OpenPatch" width="100%">
</p>

<p align="center">
  An open-source modular synthesizer built in Rust.<br>
  Make music by connecting modules, or build a patch with help from Claude or Codex.
</p>

<p align="center">
  <a href="#download">Download</a> ·
  <a href="#get-started">Get started</a> ·
  <a href="#use-claude-or-codex">Claude &amp; Codex</a> ·
  <a href="docs/ROADMAP.md">Roadmap</a>
</p>

![OpenPatch with a Melancholy Ambient song built by the Patch Engine](docs/images/patch-engine-hero.png)

## Download

**OpenPatch 0.3.3 · build 16**

| Your computer | Download |
|---|---|
| Mac with Apple silicon · macOS 12 or later | [Download for Mac](https://github.com/dexterlabs1/openpatch/releases/download/v0.3.3/OpenPatch-macos-arm64.zip) |
| Windows PC · 64-bit Intel or AMD | [Download for Windows](https://github.com/dexterlabs1/openpatch/releases/download/v0.3.3/OpenPatch-windows-x86_64.zip) |
| Steam Deck or Linux PC · 64-bit Intel or AMD | [Download for Steam Deck / Linux](https://github.com/dexterlabs1/openpatch/releases/download/v0.3.3/OpenPatch-linux-x86_64.tar.gz) |

Extract the download, then:

- **Mac:** move `OpenPatch.app` to Applications and open it.
- **Windows:** open `openpatch-desktop.exe`. Keep the extracted files together.
- **Linux / Steam Deck:** run `./install.sh` from the extracted folder, then open OpenPatch from your application menu. On Deck, install in Desktop Mode, then add it to Steam. In Game Mode the Deck's controls work with Steam's default Gamepad layout, and the interface fits the screen. The included `STEAM-DECK.txt` lists the buttons.

The Mac app is not notarized by Apple. The Windows app is unsigned.

[Release notes and checksums](https://github.com/dexterlabs1/openpatch/releases/tag/v0.3.3)

## Get started

1. Open OpenPatch and press **F1** to open the **Patch Engine**.
2. Pick a genre, mood and texture, then choose **Build**. Try **Surprise** for a random combination.
3. Like it? Choose **New sounds** to keep the song with different instruments, **New song** to keep the instruments with a new song, or **Evolve** for a small change to both.
4. Turn knobs, drag cables between ports, and add modules to change the sound.
5. Save your patch with **Cmd-S** on Mac or **Ctrl-S** on Windows/Linux.

There are 13 genres, from Deep House and Dub Techno to Progressive House, Hypnagogic, Berlin School, IDM and Melodic Techno, plus Ambient Piano. Scroll down in the panel for **Controls**: energy, complexity, weirdness, space and song length.

Every song has a short **genome** code, shown under **Details** in the panel. The same code always builds the same song. Choose **Reveal**, **Showtime** or **Instant** to change how quickly a song appears in the rack. [Patch Engine guide](docs/PATCH_ENGINE.md)

![A Hypnagogic song built from the words Mysterious and Echoing](docs/images/patch-engine-hypnagogic.png)

To build from scratch, close the Patch Engine and press **Enter** to open the module library. **Space** starts or pauses playback when the Patch Engine is closed.

## What you can do

- **Generate whole songs:** the Patch Engine writes the chords, bass, melody, drums and song structure, then builds the patch from the best instruments in the module library: FM and wavefolding voices, wavetables, physical models and analog-style filters. Every result is an ordinary patch you can change.
- **Make music:** combine oscillators, filters, envelopes, sequencers, drums and effects. Studio Mixer has up to 16 channels and two effects send/return loops.
- **Explore new instruments:** Tidal Functions, Frame Scene, Segment Garden, Event Fork, Rhythm Terrain and Recall Random add evolving modulation, scene mixing, envelopes, random gates, drum patterns and repeating pitch phrases. [Module guide](docs/MODULE_COVERAGE.md)
- **Keep a patch readable:** organize modules into rows, highlight signal paths, change cable styles, and dim the room to see the lights.
- **Make visuals:** choose from nine visualizers, including Ferrofluid, or edit GLSL shaders in the app. [Visualizer guide](docs/VISUALIZERS.md)
- **Work with an assistant:** watch Claude or Codex add modules, connect cables and adjust values in the rack. You can undo those edits just like your own.

[Controls and shortcuts](docs/RACK_EDITING.md) · [Saving and recovery](docs/PROJECTS.md) · [Example patches](examples)

## Use Claude or Codex

Choose **Connect AI** in the toolbar and follow the [setup guide](docs/MCP.md). MCP lets an assistant inspect your patch, add modules, connect cables, change settings, save and render audio.

Connecting to an already open window works on Mac and Linux. On Windows, start a session with `openpatch-desktop.exe --mcp`.

## Still in development

OpenPatch is an early release. It does not yet match all of VCV Rack's features or module library.

You can [import supported VCV patches](docs/VCV_IMPORT.md), but not every module can be converted. Existing VCV plugins do not load automatically. An experimental Mac host supports one pinned Bogaudio OFFSET build with separate setup. [Experimental hosting guide](docs/BINARY_HOST_V1.md)

Windows audio devices and Steam Deck game mode still need hands-on testing.

[Roadmap](docs/ROADMAP.md) · [Current work](docs/WORKPLAN.md) · [Report a problem](https://github.com/dexterlabs1/openpatch/issues)

## Build from source

Install stable Rust and your platform's build tools. On Mac, install Xcode Command Line Tools. Linux dependencies are listed in the [build workflow](.github/workflows/ci.yml).

```sh
git clone https://github.com/dexterlabs1/openpatch.git
cd openpatch
git checkout v0.3.3
cargo build --locked --release -p openpatch-desktop -p openpatch --bins
cargo run --locked --release -p openpatch-desktop
```

The app and command-line tool are in `target/release`. To package the Mac app, run `python3 scripts/package_macos.py` with Python 3.11 or later.

## License

Original OpenPatch code is [MIT licensed](LICENSE). The distributed app also includes GPL code and is provided under [GPL-3.0-or-later](crates/openpatch-core/LICENSE-GPL-3.0.txt). See the [third-party notices](crates/openpatch-core/LICENSE-THIRD-PARTY) for details.

OpenPatch is an independent project and is not affiliated with VCV. [Release source code](https://github.com/dexterlabs1/openpatch/tree/v0.3.3)
