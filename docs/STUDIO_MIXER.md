# Studio Mixer

Studio Mixer is an original, resizable performance console for version-2 patches. Add it from **Custom** or **Mixing** in the module library, or open **File → New File → Studio Mixer FX** for a factory patch with two effect send/return loops. The catalog identifies it with `tags: ["Custom", "Mixing"]`. It is an OpenPatch design, not a VCV Rack module or a hardware-equivalence claim.

## Work the console

The **− / +** buttons set the visible channel count from 1 to 16. Each strip has an input, a level fader, pan, sends A/B, mute, solo, and a two-lane post-fader meter sourced from the engine's actual signal state. Right-click the strip header for trim, low and high shelves, high-pass cutoff, and independent pre/post choices for the two sends. The header names the cable source. The master section controls main level, stereo return A/B levels, and send A/B master levels.

Widening the console moves any overlapping modules to its right along the same rack row as a collision chain. The new placement is saved with the channel edit; undo/redo restores the prior layout. Reducing the channel count hides the unused input jacks and silences those strips. Their cables and parameter settings stay in the patch, so expanding restores them. Channel count changes are ordinary saved, undoable parameter edits, whether made in the rack or through MCP.

## Signal path and ports

| Port | Behavior |
|---|---|
| `in1`–`in16` | One bounded input per strip; only the selected 1–16 strips process audio. |
| `return_a`, `return_b` | Stereo effect returns summed into the main bus after the strips. They never feed either send. |
| `send_a_cv`, `send_b_cv` | Channel-zero CV, clamped from 0–10 V to a 0–1 multiplier on the corresponding send master. Unpatched means full send level. |
| `left`, `right` | Main stereo bus after the master fader. |
| `send_a_left/right`, `send_b_left/right` | Two stereo FX send buses before returns. |
| `cue_left`, `cue_right` | Pre-master cue. With solos active, it carries soloed strips; otherwise it follows the pre-master main bus. |

Each strip applies trim, a 20–1000 Hz high-pass filter, ±12 dB low and high shelves, pan/balance, and a smoothed level fader. A mono cable is centered at unity in both lanes. A two-channel cable keeps its left and right lanes. Cables with 3–16 channels fold to mono using the sum divided by `√N`, then feed both lanes. Mute and solo attenuate the strip's main and send contributions. Each send can tap before or after that strip's level fader; returns cannot feed sends, so an internal FX feedback path is absent. Processing and parameter changes retain bounded state across compatible graph replacements.

The engine has 20 logical input ports and eight named mono output ports. Its native-host diagnostic shadow has only 16 input slots; raw native-host input snapshots and observers are unavailable for this custom mixer. Logical cables, returns, send CV, and signal port lights still cover all 20 inputs. This is an internal diagnostic limit, not a limit on patching the console.

## Qualification

The [public qualification report](benchmarks/studio-mixer-release-qualification.json) records passing Core/callback checks, 1,013 desktop tests, 113 release-executable MCP checks, strict Clippy, and macOS ARM, Linux and Windows CI. The native Linux captures include the console, lights-out mode and shared browser panel. Local Mac GUI and physical audio-device playback were not exercised. The bounded performance comparison covers four unchanged workloads; it does not measure the console’s own processing cost.
