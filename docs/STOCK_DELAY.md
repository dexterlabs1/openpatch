# Stock Delay

`stock_delay` is a separate, bounded implementation of the pinned VCV Free
digital Delay signal path. It adds a clock-synchronized mono delay, feedback,
tone, dry/wet mixing and four signed CV depths. The existing original
`stereo_delay` retains its separate stereo timing and modulation contract.
This is digital behavior compatibility work, not measured hardware calibration
or complete Rack/plugin/host parity.

## Controls and signal contract

Version 2 inputs, in order: `time_cv`, `feedback_cv`, `tone_cv`, `mix_cv`,
`signal`, `clock`. Outputs: `mix`, `wet`.

| Parameter | Stored range | Default | Display |
| --- | --- | --- | --- |
| `time` | 0–1 | `log10(500) / 4` | `0.001 * 10000^time` seconds; 500 ms default |
| `feedback` | 0–1 | 0.5 | 0–100% |
| `tone` | 0–1 | 0.5 | −100% to +100%, dark to bright; center neutral |
| `mix` | 0–1 | 0.5 | 0–100%, dry to wet |
| `time_cv_amount` | −1–1 | 0 | signed percent |
| `feedback_cv_amount` | −1–1 | 0 | signed percent |
| `tone_cv_amount` | −1–1 | 0 | signed percent |
| `mix_cv_amount` | −1–1 | 0 | signed percent |

Normal processing sums all active audio-input channels into mono. Each CV and
the clock read channel zero; auxiliary mono signals are not processed as
independent delay voices. Both normal outputs are mono, with unused output
storage zeroed. `mix` interpolates between dry audio and filtered wet audio;
`wet` remains independent of the mix control. Feedback feeds the preceding wet
sample back into the input. Feedback, tone and mix CV add `CV / 10 * depth`
before clamping their effective controls to 0–1. Time CV adds signed pitch
modulation to the source-style delay-rate calculation.

Unpatched clock uses the source's free-running reference rate. Patched clock
measures rising edges with the source threshold/startup behavior; its measured
rate scales the time control. The millisecond readout describes the free-running
time setting rather than promising an absolute delay after clock modulation.
CLK is the actual engine clock-light state, including its decay, not a UI timer
or an inference from the saved time or a connected cable.

The delay uses a `2^21`-sample circular history, a sixteen-sample output queue
and a continuous mono sinc converter. Its correction ratio follows the source's
queue-length error, bounded to 0.25–4. The filtered wet path preserves the
reference low-pass and high-pass operation order, including the high-pass
angular-frequency convention. This is not a replacement with a generic
fractional-delay interpolator.

## State, memory and host behavior

Each instance allocates an 8 MiB audio history and approximately 49 KB of SRC
state before callback processing. Compatible same-ID/kind/rate edits preserve
the history, queued samples, sinc state, tone filters and clock state. A fresh
incarnation or incompatible sample rate starts fresh. Initialize restores the
parameter defaults while preserving the existing tail, following the source
class's inherited reset behavior.

Bypass freezes delay, resampler, filter and clock advancement and routes the
raw audio input to both `mix` and `wet`, preserving its polyphonic width. Empty
audio input produces mono zero. Normal processing remains summed mono. The
host's one-sample cable delay and graph-transition behavior apply separately.

The live desktop handoff moves compatible Delay runtime ownership in constant
time by swapping preallocated runtimes. When a runtime is moved, the retiring
whole graph stops advancing; its cached raw, pre-gain output supplies the
fading branch while the new graph continues the tail. This prevents advancing
the moved history twice and avoids copying 8 MiB in that handoff. It does not
make all graph adoption constant-time: borrowed/immutable adoption still
copies the large Delay history, and existing StudioFx state can still copy
under its own adoption path. See [architecture](ARCHITECTURE.md).

Nonfinite inputs become zero and finite inputs obey the host voltage bounds.
Extreme time modulation is bounded before exponent construction. Processing
and live handoff use preallocated storage; device deadline and callback-cost
qualification require their own measurements.

## Measured adoption cost

The [release Engine handoff profile](benchmarks/stock-delay-handoff.json) was
recorded on an Apple M1 Max, ARM64/macOS 26.7.1. It measures two prepared,
compatible graphs with resident buffers and includes Engine bookkeeping.
Each workload has 33 trials, alternating copy/handoff order. Independent
copies run once per trial; handoffs run in batches of 256 to reduce timer
overhead. Reported handoff values are per-operation trial means.

| Delay instances | Independent-copy median (µs) | Handoff median (µs/op) | Largest handoff trial mean (µs/op) |
| --- | --- | --- | --- |
| 1 | 393.500 | 0.335 | 2.836 |
| 8 | 1,581.625 | 1.139 | 1.221 |

This measures the difference between retaining an independent history and
destructively transferring ownership; the latter requires retiring its donor
before processing it again. It excludes graph compilation, rendering, donor
retirement and device scheduling. The largest trial mean is not a worst-case
individual handoff or callback time. This is neither a Rack CPU comparison nor
a complete desktop callback-deadline result. Actual callback tests establish
no heap activity on the exercised paths, which does not establish a real-time
deadline budget.

## Source references and numerical acceptance

The fixed module reference is
[Fundamental Delay.cpp at 10dd0160c664770910e5584b7b00498cc48d9ddd](https://github.com/VCVRack/Fundamental/blob/10dd0160c664770910e5584b7b00498cc48d9ddd/src/Delay.cpp).
The mono resampler adapts
[libsamplerate 0.1.9 at f6730d03c3e7660bb6ecad8816f1b09c5825142a](https://github.com/libsndfile/libsamplerate/tree/f6730d03c3e7660bb6ecad8816f1b09c5825142a),
under [BSD-2-Clause](licenses/libsamplerate-0.1.9.txt). The notice is bundled
with the native app and printed by `openpatch-desktop --third-party-licenses`.
The original native faceplate uses no upstream panel artwork.

The [acceptance declaration](../scripts/stock_delay_reference/acceptance.json)
was frozen before the first comparison. Source-defined whole-module cases
compare every output sample from frame zero at identical indices:

`abs(actual - reference) <= 0.002 V + 1e-5 * abs(reference)`.

Source-defined standalone SRC cases require exact input-consumption and
output-generation counts on every call, including calls with no output, plus:

`abs(actual - reference) <= 2e-6 + 2e-6 * abs(reference)`.

No alignment, warm-up removal, startup discard or feedback-specific relaxation
is permitted. The matrix covers 44.1/48/96 kHz, startup, minimum/maximum delay,
tone and feedback extremes, signed and zero-depth CV, abrupt time changes,
clock startup/threshold/tempo/reconnection, audio summation, channel-zero CV,
history wrapping and compatible parameter/adoption continuity.

The upstream 0.1.9 mono SRC has an undefined before-buffer history read in an
independently analyzed ratio-1 to ratio-0.25 startup sequence. Rust supplies
zero history for that invalid region. This is a bounded safety extension,
not a claim to match undefined source memory. A private bounds-guarded build
of the actual C converter classifies affected standalone or whole-Delay
scenarios before numerical SDK execution. Such cases receive native
finite/count/padding safety checks and `fidelity_passed: null`; they receive no
source-fidelity pass. Every source-defined case retains the original count
and numerical gates. The guard is specific to these history reads, not a
general proof that upstream C has no undefined behavior.

The final [guarded actual-class report](benchmarks/stock-delay-reference.json)
records `full_matrix: true`: all 43 whole-Delay cases pass source fidelity over
11,134,750 frames at 44.1/48/96 kHz, with zero worst absolute voltage error.
No whole-Delay case encountered an undefined guarded source read. All six
source-defined direct SRC scenarios, each with 8,192 persistent calls, match
input/output counts and generated samples exactly under the unchanged gates.
Production source hashes remained stable throughout the run and were
reverified afterward.

The independent analytic ratio-1→0.25 SRC startup extension is separately
classified as one source-undefined case. Its `fidelity_passed` remains null;
the native finite/count safety checks pass. It is not counted as a numerical
SRC fidelity pass. The report's aggregate pass covers every source-defined
comparison, native safety checks and stable production hashes; it does not
grant fidelity to undefined source behavior.

The [oracle guide](../scripts/stock_delay_reference/README.md) describes the
actual-class adapters, immutable pins, trace hashes and guarded reporting
procedure. The oracle requires an installed SDK; ordinary portable host CI
does not repeat that SDK comparison. This completed bounded numerical gate
does not establish device behavior or complete host compatibility.

## Native panel and remaining qualification

The original shared 24 HP panel provides four primary faders, four signed CV
depth controls, exact millisecond/second and percentage entry, eight jacks and
the real CLK light. The browser uses the same renderer and port geometry.
Try **File → Clocked delay voice** for a connected envelope/VCA/delay patch
with actual MIX/WET Scope traces. Headless tests cover direct input, precision
entry, navigation, geometry, shared previews, gesture identity and clock-light
rendering; they do not establish native window or device behavior.

The native smoke sequence now declares 97 captures, preserving the previous
94 and appending `rack-stock-delay.png`, `rack-stock-delay-lights-out.png` and
`stock-delay-library.png`. These scenes provide rack, dark-room and browser
coverage for visual review in native CI artifacts. The Rust checks report
1,580 passing tests and six existing
ignored tests; strict Clippy also passes. The release workspace build and all
79 Python/MCP tests pass (21.379 seconds). The macOS ARM bundle's ad-hoc
signature is verified and its bundled notices match the source notices; that
bundle has not been installed or launched.

Platform CI and native capture verification are release gates; the smoke
sequence is configured for 97 screenshots, not an assertion that they have
been executed or reviewed for this change. Full callback-deadline measurement
and native macOS ARM window/input/playback qualification remain open. There
is no hardware calibration, physical audio/MIDI test, VCV import or
plugin-binary compatibility claim.
