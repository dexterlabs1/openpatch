# Built-in visualizers

Visuals are a core OpenPatch product feature, alongside audio synthesis and MCP
control. They must never stall the audio callback.

## Native live visualizer

`openpatch-desktop` includes an optional OpenGL view in the native analysis dock.
Click the **Visuals** toolbar icon or choose **View → Visualizers**, then drag the
dock's divider to resize. Scope and Visuals are tabs in the same dock, keeping
space available for the rack and library. Aurora, Spectrum, Waveform, and Ferrofluid modes
react to the same audio output as the rack. Playback
starts paused; press Play in the window to hear the patch and drive its visuals.
The animated background continues while paused, with audio input fading to zero.
Ferrofluid renders a head-on black liquid sculpture with reflective spikes and
studio lighting. Low-frequency energy swells the body; upper bands reshape the
surface and add rapid ripples. Fast attack/release envelopes
and a relative peak pulse make the movement prominent during playback. A soft
energy mapping retains variation instead of flattening louder signals at a hard
ceiling. This is a visual interpretation, rather than a physical simulation.
Ray marching has a fixed 120-step limit and stays outside the audio callback.

No browser or local audio-file selection is needed for this native view.

The **Edit GLSL** window edits a GLSL 330 core fragment shader against a fixed
fullscreen vertex shader. The built-in source in
[`visuals.frag`](../crates/openpatch-desktop/assets/shaders/visuals.frag) defines the interface:

| Input | Meaning |
| --- | --- |
| `in vec2 v_uv` | Normalized coordinates across the visual field |
| `out vec4 frag_color` | Fragment color written by `main()` |
| `uniform vec2 u_resolution` | Visual field width and height in UI points |
| `uniform float u_time` | Seconds since the window opened |
| `uniform float u_level` | Recent decaying peak after transport and volume gain |
| `uniform float u_wave[256]` | Best-effort waveform, oldest sample first |
| `uniform float u_spectrum[64]` | Hann-windowed DFT magnitudes from the waveform |
| `uniform int u_mode` | 0 Aurora, 1 Spectrum, 2 Waveform, 3 Ferrofluid |
| `uniform vec4 u_ferro` | Smoothed low, middle, high and peak envelopes, each 0..1 |

Compile/link errors keep the previous working shader active and show driver
diagnostics. Sources are limited to 64 KiB. Restore source restores the editor
text; compile it to replace the running shader. Visual modes and custom shader
source are local UI state and are not currently saved in patch JSON or exposed
through MCP tools.

The callback publishes every fourth output sample into 256 atomic slots and
updates a cursor, plus a decaying peak value. This is an approximate scope window
of 1,024 device frames (about 21 ms at 48 kHz), rather than an atomic capture of
one audio block. The UI reads the latest values and computes a small 64-bin DFT.
It is a linear display spectrum of decimated samples, not a calibrated full-band
FFT, log spectrum, RMS meter, or beat/tempo detector. GPU rendering and analysis
never run inside the audio callback.

## Standalone browser preview

`openpatch visualizer visualizer.html` exports the bundled standalone preview.
Open it in a modern browser, select a local audio file, and press Play. It uses
Web Audio analysis and WebGL2, with built-in Aurora, Spectrum and Oscilloscope
modes plus an editable GLSL fragment shader. All files stay local. The preview
has its own audio playback and is separate from the native OpenPatch device and
live graph. The `export_visualizer` MCP tool exports this browser preview too.

The preview shader profile is **webgl2-glsl-es-300-v1**: a fixed fullscreen vertex
shader, GLSL ES 3.00 fragment shader, `main()` entry point, and explicit time,
resolution, level and audio-texture uniforms. The editor's bundled source is the
reference for the exact texture layout and uniforms. Shader compilation errors
leave the previous working shader active. Custom shaders execute on the GPU and
can still stall rendering; this prototype is not a sandbox for untrusted shaders.

## Rendering boundaries and future work

The native renderer currently uses eframe's Glow/OpenGL backend, not wgpu or
WGSL. Native GLSL 330 and browser GLSL ES 3.00 are separate interfaces; shaders
are not promised to work unchanged between them. Shader compilation and GPU
draws stay on the native UI/render thread. A costly custom shader can stall the
window or GPU; it does not acquire the audio session lock and is not a sandbox
for untrusted shaders.

A future analysis pipeline can add coherent timestamped frames, full-band FFT,
RMS, band energies, and onset estimates on a dedicated worker. It must keep the
audio producer bounded and avoid waiting for visual consumers. Beat/tempo
estimates must be identified as estimates rather than sample-clock truth.

Future MCP commands should enumerate visual modes, select presets, adjust
parameters, compile supported shaders, inspect compile diagnostics, and capture
frames. Shader compilation and frame capture belong to the visual/control
threads. They must not trigger filesystem or GPU work from DSP processing.
