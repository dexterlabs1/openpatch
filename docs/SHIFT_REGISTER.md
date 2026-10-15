# Sample & Hold / Analog Shift Register

`sh_asr` provides eight mono sample-and-hold rows with independently patchable
IN and TRIG inputs. Open **File → Shift register canon** for a three-voice
pentatonic canon: rows 1, 3 and 5 send successively older notes to three voices,
and Scope compares rows 1 and 3. The module and its browser thumbnail share the
same renderer, including signal and trigger normalization guides.

## Sampling and normalization

Each rising trigger captures the row's IN voltage. An unpatched TRIG inherits
the preceding row's **event**, while a patched TRIG breaks that chain even when
its voltage is LOW. An unpatched IN samples the preceding row's **old held
value**. All rows in one capture pass use the values held before that pass;
newly captured voltages do not cascade through all eight rows immediately.
These roles follow the [VCV Free manual](https://vcvrack.com/Free#VCV-Sample-Hold-Analog-Shift-Register).

PUSH triggers row 1 and its uninterrupted trigger-normalization chain. CLEAR
zeros all eight holds. RND enables the random normal at IN 1; when disabled,
that unpatched input samples zero. Patching IN 1 overrides its normal, but RND
still consumes one random draw whenever row 1 triggers. Other rows consume no
draws. RND defaults off, the random range defaults to 0–10 V, and all holds
start at zero without consuming any draws.

The inputs read channel 1 (engine index 0), and all eight outputs are always
mono. A zero-channel input acts unpatched. Finite sampled signals clamp at the
engine safety bound of ±1,000,000 V; nonfinite signals become zero. This is an
ideal digital hold without droop or circuit noise. Legacy `sample_hold` retains
its previous independent semantics.

## Triggers and manual ordering

Connected trigger detectors use LOW ≤0.1 V and HIGH ≥1 V. An initially HIGH
input establishes HIGH without capturing; an initial intermediate voltage leaves
the detector uninitialized. Disconnected detectors freeze. Reconnecting resumes
the detector's retained hysteresis state.

PUSH uses `push_press` / `push_release` runtime actions; CLEAR uses
`reset_press` / `reset_release`. Holding a button does not repeatedly activate
it. Ordered runtime actions are an OpenPatch policy: connected CV edges are
evaluated once, and the first effective PUSH edge coalesces with those edges.
Each additional release/press pair in the same sample performs another capture
pass against the preceding pass's values. An effective CLEAR anywhere in that
sample wins after all captures and zeros the holds. Random draws made during
those captures remain consumed. Repeated presses of an already held button
do nothing. Emergency release clears buttons and queued actions, preserving
holds, RNG and detector history.

Bypass emits eight mono zeros and freezes holds, RNG and detectors. Bypass
transitions release physical controls. The engine's normal one-sample delay per
version-2 cable still applies; it is separate from the row shift ordering.

## Deterministic state and editing

Visible controls are `random` (0/1, default 0), `range` (0–5, default 0), and
`seed` (integer 0–16,777,215, default 1). The six ranges are 0–10 V, 0–5 V,
0–1 V, ±10 V, ±5 V and ±1 V. RND and range affect future captures. A seed edit
restarts the generator without altering holds or trigger detectors.

The original deterministic generator uses xorshift32 shifts 13/17/5, initialized
with `seed XOR 0xA341316C`. It advances before each draw and maps the upper
24 bits divided by 2²⁴ through the selected float32 affine range. It does not
claim to reproduce VCV's random sequence.

Coherent project capture, MCP saving, native projects and recovery preserve all
eight actual float32 holds and the next nonzero 32-bit RNG state. Presets,
duplication and undo/redo use the same typed state. A full explicit snapshot
wins over live adoption, including when the configured values compare equal.
Ordinary edits preserve holds, RNG and detectors at the same identity/kind/rate.

Storage uses ten hidden reserved parameters: `value1`…`value8`, `rng_hi` and
`rng_lo`. All ten must be supplied together or all omitted. The RNG halves are
exact integers 0–65535, with a nonzero combined state. Held voltages must be
finite and within ±1,000,000 V, regardless of the selected random range. Hidden
storage fields never become ordinary controls or browser search terms.

Initialize restores RND off, range 0, seed 1, zero holds and the default RNG,
and releases physical controls/actions. It preserves trigger detectors to avoid
synthetic edges; undo restores the prior complete saved state. Host Randomize
is disabled for this entire module and changes no parameters, holds or RNG.
Use RND and PUSH for deliberate random capture.

## Validation and boundaries

Tests cover independent shift vectors, split trigger chains, simultaneous
captures, initial HIGH and disconnect/reconnect history, CLEAR priority, random
consumption, snapshot validation, seed-only resets, graph adoption, callback
partitioning and heap activity. Host tests drive actual callback inputs through
saving, reopening, presets, duplication, history and recovery. JSON-RPC tests
independently calculate random draws and expected delayed row values.

The [headless reference harness](../scripts/shift_register_reference/README.md)
executes the actual pinned SHASR class with RND off against the production Rust
module: 27 cases at three sample rates compare 221,184 held voltages exactly.
Its [report](benchmarks/shift-register-reference.json) records source, driver and
SDK hashes; it excludes random-sequence equivalence and host lifecycle behavior.
Automated headless UI checks cover direct controls, release safety, hidden storage, shared
previews and numeric readout activation.

Behavior was specified from the manual and
[pinned SHASR reference](https://github.com/VCVRack/Fundamental/blob/10dd0160c664770910e5584b7b00498cc48d9ddd/src/SHASR.cpp).
The implementation is original MIT Rust; upstream GPL code and artwork were not
ported. Deterministic RNG restoration, initial detector behavior and ordered
manual events have explicit OpenPatch contracts. Native macOS gestures and
audio-device playback remain separately unqualified.
