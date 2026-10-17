# Segment Garden

Development candidate requiring the current source build. Not installed. The native instrument uses the pinned MIT-licensed Stages STANDARD3 generator: RAMP, STEP and HOLD. ALT audio firmware, chain expanders, hardware ADC/DAC calibration and binary VCV compatibility are outside this module contract.

## Six physical stages

Each stage stores a type, loop flag, normalized primary (`level1..6`) and secondary (`shape1..6`) control. The default first stage loops as a useful independent LFO; other stages default to nonlooping ramps. `seed` is an exact integer 1..16777215, default 1. It seeds one module-wide deterministic source PRNG. Ordinary MCP parameter edits, presets, duplication, save/reopen and shared document history carry these controls.

Inputs are `gate1..6` and `cv1..6`; outputs are `out1..6` plus `active1..6`. All use channel zero; any physical gate cable starts a group, including a zero-channel source. Unpatched prefix stages remain independent. A patched stage leads the following stages up to the next patched gate. Inspecting/tapping an output does not change grouping.

Grouped stage outputs preserve the original behavior: a leader emits the complete envelope/sequence, while each slave emits `1-phase` during its own stage, otherwise zero. ACTIVE emits 5 V when the delayed source segment selects that physical stage. It is a separate signal, rather than a substitute for the progress contour.

The contextual controls follow source dispatch. Independent looping RAMP is an LFO; triggered RAMP is a decay or tapped LFO. STEP is portamento without a trigger and delayed sample-and-hold with one. Independent HOLD supplies a 16-bit CV delay, triggered HOLD a timed pulse, and triggered looping HOLD a probabilistic gate. Mixed groups retain source loop/transition rules. A nonlooping RAMP/HOLD leader followed only by STEP stages forms a sequencer: the leader selects reset/address and direction, with seven directions. RAMP-led sequences use the firmware's 13-state hysteresis note quantizer; HOLD-led sequences remain unquantized. In multistage groups a sole looping HOLD ignores its time control; the panel labels this HELD. The first RAMP and later internal ramps have distinct shape/level roles.

Six fixed kernels preserve hidden runtime through leader/slave/leader repatching, including the source's distinct configuration reset rules and stale sentinel-phase reference. Note-quantizer state belongs to physical slots, and the seeded RNG advances in physical processing order. Reopening and Initialize start fresh; same-rate graph adoption retains queues, phases, source-block cursors and output buffers. A changed seed applies at the next source block; sample-rate changes start fresh. Generic bypass emits mono zero and freezes this runtime.

## Explicit native clock and voltage bridge

The source runs at 31,250 Hz in eight-sample blocks. Native rates 8,000..192,000 Hz use a bounded integer clock accumulator, at most four source ticks per host frame. Generated blocks play during the next eight source ticks, adding 256 microseconds of source buffering plus native sampling quantization. Ordinary native graph cables add their existing one-host-sample delay separately. Zero-order holding is a declared modulation-rate bridge, not an audio anti-aliasing claim.

Types, loops, physical grouping, CV controls and seed changes latch at the start of a source block. Primary is `clamp(level + CV/8,-1,1.999995)`; secondary is the normalized shape control. An unpatched CV contributes zero. Nonfinite inputs read zero. Ideal output is the source value clamped to[-1,1] and scaled by 8 V. This is an explicit nominal voltage policy, not measured analog calibration or the hardware's full rail/ADC transfer.

Native Schmitt thresholds are 0.1 V low and 1 V high. Each input captures edges in order into a fixed 32-event queue; a source tick consumes at most one edge. Isolated short pulses can stretch to source-clock resolution. An inserted gate's observed edges wait for the next block's group latch, rather than being consumed by the old unpatched group. Queue overload clears queued events and reconciles to the latest host level, with a sticky real overload lamp cleared by Initialize. Arbitrarily dense host edges cannot retain exact timing on a slower clock. New native kernels prime only their clock extractor with one low source sample; raw source construction remains unchanged.

The shared rack/browser face uses source-meaningful control labels and physical-patch CONFIGURED group guides. Published lamps report actual delayed bipolar outputs, stage activity and overload. No telemetry means no fabricated activity. Configuration print is distinct from callback telemetry.

[Shared panel and matching browser/dim-room review](../design/reviews/segment-garden-v1/README.md).

[Three-instrument evolving voice example](../examples/segment-garden-evolving-voice.json).

## Source evidence and safety boundaries

Source is pinned to eurorack `f4cfa7697095a28d292bff758beb51deb49b5a86` and stmlib `e3bd7c9cc00e4364166f9905c0509b6ffd0535ec`. The [reference runner](../scripts/stages_reference/README.md) freezes unmodified C++ sources, static-zero initialization, actual shared note quantizers, seeded PRNG, inputs, compiler identity, float-bit records and artifact hashes. A separate executable audits every positive case with undefined-behavior, bounds, float-cast and floating-division sanitizers and requires identical records.

The base 61 cases cover 780,288 records; eight added completion/capture cases cover 172,608 more. They include all twelve standard dispatch combinations, block sizes 1/8, all seven sequencer directions, enabled/disabled note quantization, signed inputs, sample-and-hold across block boundaries, actual timed completion, simultaneous gate/completion priority, 36-stage storage, slave contours, shrinking groups and hidden leader/slave/leader state. Rust equality is qualified against frozen evidence, rather than output from another Rust implementation.

Source primary 2 and secondary 1 can index one past their tables; native lookups safely clamp to the last element. A rare rounded random draw can index beyond full 36-stage storage; the native raw kernel guards that case while preserving the original in-array sentinel behavior for shorter groups. The native module exposes six stages. These extensions are excluded from equivalence claims.

The sample-zero tapped-clock case deserves a distinct boundary: optimized Stages LFO output is finite, but the stricter audit catches an internal division by zero in the discarded extracted-frequency path. Historical attempts 001/002 retain the incorrect initial expectations; attempt 003 keeps this as an explicit audit-negative alongside two guarded LUT endpoints. No source code or sanitizer gate was weakened to obtain a pass. Native clock priming is a separately tested adaptation.

Focused acceptance includes genuine C++ golden traces through the actual Engine, physical grouping versus taps, timed/slave outputs under live edits, ordered narrow edges, insertion/overload handling, source-state adoption, seed latching and callback allocation/deallocation checks. Native-window/audio-device qualification and complete family Mac CI remain separate gates.
