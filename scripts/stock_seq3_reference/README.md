# Frozen Stock SEQ 3 qualification

This independent matrix targets actual Fundamental `SEQ3` at commit
`10dd0160c664770910e5584b7b00498cc48d9ddd`, SHA256
`5144a35b58c2187fd6611acfdccc7a938e16fd8a8febc8310871aa2f8fde7131`.
Source and genuine SDK2.6.6 headers define behavior; the generator and validator
must not import or mirror the new Rust DSP. Legacy `seq3` is unchanged.

`acceptance.json` fixes the120-input/90-output protocol, raw control order,
source-defined numeric domain, lifecycle adapter boundary and exactf32 gates.
`fixtures.py --freeze` exclusively creates `fixtures.json` before the first
comparison; `--check-fixtures` verifies immutable acceptance/generator/input
hashes. `rows(rate,a)` and `safety_rows(rate,a)` stream `(case,frame,fields)`;
`fixture_text(rate,a,safety=False)` produces the wire input. Manifest entries
retain case offsets and tags. Every processed frame is compared, including
startup, reset collisions, stopped controls and complete slow-clock periods.

Run the actual pinned class prefix with connected mono outputs and real SDK
BooleanTrigger, SchmittTrigger, PulseGenerator, lights and scalar approximation.
Use the same production model through its probe, with no duplicate sequencing
implementation. Preserve source/licenses, SDK/build/FPU/toolchain closure,
binaries, complete inputs/traces and failed reports. No fastmath/contraction.
All reference fields compare exactf32 bits after decimal decoding; safety is
Rust-only. A numeric failure must be retained and exitnonzero. No fitted gates,
frame removal or changed fixture declarations after comparison.

Source surprises are intentional: first high doesnot trigger an uninitialized
edge detector; paused clocks keep detectorhistory; manualclock bypasses reset
suppression; reset can cause a clockpulse when indexchanges; lengthone clocks
neednot pulse; passthrough freezes clockpulse tails; resetguard can accumulate
phase beyond1. Runtime Initialize calls source onReset only; host parameter
Initialize requires separate tests. Durableload missingpassthrough means true;
freshconstructor means false. Heldlevel oracle fixtures do not claim equivalence
for OpenPatch's convenience-action queue or native Rack Engine bypass.

A complete milestone also requires publicEngine routing/delay, durable state
projection, saved Run/gates/pass, presets/copy/undo/rotation, actionidentity and
backpressure, nonallocating callbacks, actual lamptransport, all24cell controls,
all21ports, shared normal/night/browser rendering and real nativeMac tests.
MIDI, stage controls and physical device qualification remain later work.
