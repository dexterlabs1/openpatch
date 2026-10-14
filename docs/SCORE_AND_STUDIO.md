# Scores and studio modules

OpenPatch can rebuild longer modular arrangements through MCP using native live DSP. These are original algorithms, not binary-compatible VCV plugins or claims of identical third-party sound.

## SCORE

`score_sequencer` stores structured `score` data on its node: four tracks, 1–64 steps, up to four voices per track, velocity, ratchets, ties, seeded probability, and up to sixteen automation lanes. Notes are semitone offsets from C4. Its pitch outputs use 1 V/oct, gates and velocity use 0–10 V. The automation output packs one voltage lane per channel; use SPLIT to route them.

`tempo`, `swing`, and `running` remain normal editable parameters. A patched clock advances one step per edge; reset restarts the score. The panel shows the saved four-track pattern and ports. Detailed note/automation editing currently uses MCP, not a piano-roll editor. `catalog` describes the bounded score schema. For example:

```json
{"id":"score","kind":"score_sequencer","params":{"tempo":128},"score":{"length":16,"tracks":[{"voices":2,"steps":[{"step":0,"notes":[-12,-5],"gate":0.8}]}],"lanes":[{"length_bars":64,"points":[{"bar":0,"value":0},{"bar":8,"value":1,"curve":"ease_in"}]}]}}
```

Add this with `add_module`, or update its data with `set_score`. Both support atomic batches, validation, undo and redo. Changed score data restarts its transport; unrelated graph edits preserve playback. Project files, duplication and module presets preserve score data. Tempo/swing changes retain transport; parameter changes do not recompile the graph for every automated step. Lane interpolation uses the destination point’s curve; equal-time points preserve deliberate jumps. Seeded probability changes by loop and reproduces from reset.

## Native processing

| Module | Behavior | Limits |
| --- | --- | --- |
| DRUM | Triggered sine kick, noise clap, metallic closed hat | Original synthesis, not Kickall/XOX emulation |
| STATE VARIABLE | Polyphonic low/band/high TPT filtering with cutoff CV | Original two-pole filter, not Surge/VCV VCF identity |
| STEREO DELAY | Independent fractional times, crossfeed, filtered feedback and modulation | Mono per L/R port; sum chords before input |
| SPACE | Stereo feedback-delay-network reverb, independent dry/wet and predelay | Original tank; no Plateau algorithm parity |
| DUCK | Stereo-linked RMS compressor, external sidechain, soft knee | 5 V detector reference; original dynamics, not Pressor identity |

All buffers are allocated during graph compilation. Callback processing and compatible state adoption use bounded storage. Effects preserve histories across unrelated edits. Their DSP is tested for signal behavior, finite output, and allocation-free processing; this does not establish physical hardware calibration or superiority over Rack.

## Full arrangement export

`render_arrangement` streams 0.001–300 seconds from a reset graph to a new PCM16 WAV in the MCP workspace. It uses fixed audio scratch storage and reports preclip peak/RMS and clipping counts. It never plays speakers, changes the live graph, or overwrites a file. Use `render_audio` for short sample-frame MIDI/module-event traces; the long renderer currently handles autonomous patches and saved scores.

```json
{"filename":"arrangement.wav","seconds":120.02}
```

The `scripts/rebuild_stiw.py` reconstruction helper accepts an inspected source JSON, translates its saved score automatically (with optional native score overrides),, authors the graph through real stdio MCP, then saves a native project and explicit substitution manifest. It preserves the source file. A reconstruction is an editable native rendition, not a transparent `.vcv` import.

The reconstruction uses cascaded native SVFs for its four-pole lowpasses by default; `--circuit-filters` selects the more expensive nonlinear ladder alternative. Both are explicit substitutions. The source Audio-2 DC-filter setting is preserved through the stereo output’s optional `dc_block` control (10 Hz high-pass; defaults off for existing OpenPatch projects). The reference setting is documented by the [Rack 2.6.6 audio source](https://github.com/VCVRack/Rack/blob/v2.6.6/src/core/Audio.cpp).
