# Built-in visualizers

Visuals are a core OpenPatch product feature, alongside audio synthesis and MCP
control. They must never stall the audio callback.

## First visual prototype

`openpatch visualizer visualizer.html` exports the bundled standalone preview.
Open it in a modern browser, select a local audio file, and press Play. It uses
Web Audio analysis and WebGL2, with built-in Aurora, Spectrum and Oscilloscope
modes plus an editable GLSL fragment shader. All files stay local. The preview
has its own audio playback; it is not yet attached to a native OpenPatch audio
device or live graph.

The preview shader profile is **webgl2-glsl-es-300-v1**: a fixed fullscreen vertex
shader, GLSL ES 3.00 fragment shader, `main()` entry point, and explicit time,
resolution, level and audio-texture uniforms. The editor's bundled source is the
reference for the exact texture layout and uniforms. Shader compilation errors
leave the previous working shader active. Custom shaders execute on the GPU and
can still stall rendering; this prototype is not a sandbox for untrusted shaders.

## Native design

The planned desktop compositor uses wgpu with Metal on macOS (including Apple
Silicon), Direct3D 12 on Windows, and Vulkan on Linux. Native built-in shaders
will use WGSL; a GLSL importer needs a separately versioned, validated subset.
WebGL2 GLSL ES shaders are not promised to translate unchanged through Naga.

Audio-to-visual transfer will use a preallocated bounded ring or snapshot buffer.
The audio callback only copies bounded sample data. A worker calculates FFT,
RMS, peak, envelope and onset estimates. The renderer consumes the latest
complete analysis frame at the display rate. Slow visual consumers drop frames;
the audio thread never waits for the GPU or an analysis worker.

The analysis contract will include audio sample timestamp, sample rate, waveform,
log spectrum, RMS, peak, band energy, and onset confidence. Beat/tempo estimates
must be identified as estimates rather than sample-clock truth.

Future MCP commands should enumerate visual modes, select presets, adjust
parameters, compile supported shaders, inspect compile diagnostics, and capture
frames. Shader compilation and frame capture belong to the visual/control
threads. They must not trigger filesystem or GPU work from DSP processing.
