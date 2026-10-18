# Rhythm Terrain

Development candidate, not installed. Rhythm Terrain is a three-lane drum-map
and Euclidean rhythm generator adapted from the pinned Mutable Instruments
[Grids firmware](https://github.com/pichenettes/eurorack/tree/f4cfa7697095a28d292bff758beb51deb49b5a86/grids),
with an independent OpenPatch panel and explicit native timing/input policies.
It belongs to Custom, Clock, Sequencers, Modulation and Random.

## Controls and patching

TEMPO is an integer 0–480 BPM. Below 40 selects external CLOCK; 40–480 selects
the internal source clock. A tapped lock overrides configured TEMPO, including
at 30–39 BPM, until released. Resolution selects 4, 8 or 24 PPQN; source clock
increments are respectively 6, 3 or 1. Internal mode ignores CLOCK voltage.
SEED is an exact nonzero 16-bit source LFSR seed, 1–65,535. Changing it starts
a fresh instrument.

X/Y choose a location in the full 25-node drum map; FILL 1–3 set the three
source density thresholds. CHAOS sets per-bar perturbation. Enabling SWING
repurposes CHAOS as swing depth and disables drum perturbation. Euclidean mode
uses X/Y/CHAOS as lengths 1–32, calculated from each byte as `(byte >> 3) + 1`;
FILL bytes select the original 32 density-table entries. Euclidean mode ignores
swing. Integer interpolation, threshold/accent decisions, all map nodes,
Euclidean tables and source RNG advancement remain source-derived.

Each matching CV adds `voltage / 5` to its configured normalized control. The
result clamps to 0–1, multiplies by 255 and rounds to the source byte. Nonfinite
input voltages read as zero. All inputs use channel zero; all eight outputs
are mono ideal 0/5 V.

TRIG 1–3 expose the lane bits. Normal AUX 1–3 expose the three accents in drum
mode or three cycle starts in Euclidean mode. ALTERNATE changes AUX 1 to common
accent/cycle start, AUX 2 to clock and AUX 3 to bar/all-cycle reset. This follows
the executable source, whose header description is inconsistent. Extra CLOCK
and RANDOM outputs expose source bits 6 and 7; these are native additional
jacks. GATE mode clears all output bits on a serviced falling clock edge.
Trigger mode uses the source pulse counter and clears all bits when it reaches
8; the random output follows that clearing policy too.

## Timing, inputs and tap authority

The native bridge executes exactly 8,000 source scans per second at supported
host rates 8–192 kHz, with the first scan after one source interval and held
outputs between scans. Each scan increments tap elapsed, handles direct button
actions, runs source CLOCK then RESET processing, evaluates any pending clock
tick, increments the pulse counter and publishes outputs. This order gives a
nominal source 1 ms trigger seven published intervals (0.875 ms), plus host
sample quantization. Ordinary native cables add one host frame separately.
The actual board's default 20 MHz clock and integer timer period 78 yield
8,012.8205128 Hz; native 8 kHz is an intentional host policy.

CLOCK/RESET Schmitt inputs start low, rise at 1 V and fall at 0.1 V. Combined
simultaneous host changes enter one of 64 fixed FIFO slots together with TAP
state. Each source scan consumes one snapshot. This captures short pulses
that physical polling could miss and stretches queued events in order. The
sustainable combined transition rate is 8,000 snapshots/s, or at most 4,000 Hz
for a complete two-edge clock cycle. Backlog adds up to 64 scans of latency.
On overflow, the queue discards its backlog and reconciles the latest levels;
a sticky overflow lamp reports the loss. CV and controls use their values at
the service scan, not historical edge time.

Runtime actions are `tap_press`, `tap_release`, `release_tap` and `reset`.
With TAP TEMPO enabled, direct press intervals calculate integer
`480000 / elapsed_scans`; 30–480 BPM locks the source clock and resets phase.
Zero, expired or invalid intervals safely unlock. Saved TEMPO edits remain
saved while the tap clock has authority. RELEASE TAP or disabling TAP TEMPO
unlocks, applying configured tempo on the defined control scan. With TAP TEMPO
disabled, the physical TAP control follows the reset-button path. Physical
switch debounce, long-press menus, ADC/pot scheduling and board timer arithmetic
overflow are excluded.

## Reset, lifecycle and observation

Musical reset clears source step/pulse/Euclidean indices while retaining output
bits, pulse duration, RNG, perturbation, beat flags and cached swing. In normal
nonfactory external mode RESET alone is transparent. Internal or locked RESET
also retriggers and resets clock phase. Simultaneous reset and clock retains
the source's double evaluation where applicable. The native Reset action
follows the source reset-button path without Retrigger. Factory EEPROM behavior
from the first five power cycles is excluded. Resolution edits reset pattern
indices and update increment while retaining clock phase.

Compatible same-ID/kind/rate/seed graph adoption preserves the kernel, clock/tap
authority, scan phase, input FIFO and hysteresis. Receiving controls/CV apply
at the next scan; pending actions belong to the receiving graph. Bypass freezes
runtime, emits mono zero and releases the physical TAP hold. Initialize,
seed/rate changes and fresh reopen restart transient runtime. Configured
controls and seed persist through preset/save/reopen; the live clock, lock,
queued edges and pattern position are transient.

The first eight lamps report actual source output bits. Additional indicators
report actual tap lock and sticky FIFO overflow. Held TAP is runtime control
telemetry. Internal quarter-note frequency is actual BPM/60; external mode has
no fabricated frequency. Position reports the actual **next** source step/32.
No output/position telemetry is implied before the first source scan. The
adapter uses fixed storage and performs no callback allocation, deallocation,
locking or filesystem work.

## Source qualification and bounds

The [reference harness](../scripts/grids_reference/README.md) pins eurorack
`f4cfa7697095a28d292bff758beb51deb49b5a86` and its avrlib submodule
`276b2887e4110ca913294fcbb313163dfb28a448`. Original C++ PatternGenerator,
Clock, Random and resources execute directly; the composite scan mechanically
extracts the original `HandleClockResetInputs()` helper. Host stubs cover AVR
memory access and unused hardware interfaces. The audited host overlay selects
the original portable integer arithmetic and excludes one proven-unused broken
`U24Sub` fallback; it retains original/overlay hashes and a unified diff.

Actual source attempt `native-test-output/grids-reference/attempt-004` passed
**49 cases and 377,402 operations** with exact original/sanitized/Rust traces.
It compares every word of all 25 nodes, 1,024 Euclidean words and 512 tempo
words, with pinned table hashes. Cases cover map endpoints, densities, valid
option bytes, seeds, chaos/bar perturbation, all resolutions, Euclidean
lengths/densities, resets, retriggers, falling edges, byte-counter wrap, live
modes and clock phase/swing. The
[actual Engine golden test](../crates/openpatch-core/tests/rhythm_terrain.rs)
passed **29,400 frames** from the frozen original-helper trace, checking all
eight output bits and next-step telemetry through real graph adoption and
native cables. The [golden authoring tool](../scripts/grids_reference/host_golden.py)
selects original source output columns and does not calculate expected DSP.

Real source-undefined boundaries remain sanitizer negatives: BPM 512, map step
96 and malformed EEPROM resolution bits 4/7. Enum sanitizing remains enabled.
Declared enum LAST=3 is valid to load then clamps; Rust safely clamps malformed
bytes as an explicit guarded extension, while native resolution controls accept
only 0–2. Raw LUT BPM 0–511 is source-defined; the native control stops at 480.
Original failed attempts remain retained evidence.

This establishes bounded source-state/arithmetic agreement and an actual Engine
path. It does not establish board electrical/CPU timing, hardware accuracy,
native-window interaction or audio-device qualification. Focused UI/MCP and family
release validation are tracked separately in [WORKPLAN.md](WORKPLAN.md).

GPL-3.0-or-later attribution, corresponding-source retrieval and bundled notices
are retained. Original OpenPatch source keeps its MIT notices; combined binaries
containing this firmware adaptation are GPL-3.0-or-later. No installation or
release change is implied by these development results.


## Native workflow and example

The shared rack/browser panel supplies seven knobs, six selectors, exact seed
entry and all sixteen cable targets. Output lamps, tap lock, overflow, next step
and effective tempo use actual engine telemetry. Unavailable telemetry is not
silence. Native gestures use the shared revision/history path; TAP actions are
transient and do not become saved controls. Fast clicks wait for host HIGH/LOW
publication so the ordered source input queue preserves them. Emergency releases
cancel older queued presses, including on bypass or module replacement.

The [soft-groove example](../examples/rhythm-terrain-soft-groove.json) routes the
three lanes into a quiet kick, clap and filtered noise tick. Each accent passes
through an explicit `6 + 0.8 * volts` bias, producing 6–10 V instead of muting
unaccented hits. These existing drum voices are original functional models;
Grids produces the rhythm, not the drum synthesis. The 14-second, 48 kHz stereo
PCM16 offline render peaks at 0.078186, RMS 0.008065, with zero clipped samples.
This is a numerical check, not a listening test.

[Actual normal/dim shared panel review](../design/reviews/rhythm-terrain-v1/README.md).
Focused checks cover actual Engine source frames, partition invariance, callback
allocation/deallocation at five rates, shared UI geometry/history/seed controls,
real callback action routing, and actual stdio MCP capture, undo/redo, atomic
validation, presets and save/reopen. Family-level macOS CI remains pending.
