# Recall Random: source and native contract

Native implementation candidate for PR #37; not installed or in release 0.3.0.
See [the user guide and qualification](RECALL_RANDOM.md).
This is the sixth planned creative instrument, following the five candidates
tracked in [WORKPLAN.md](WORKPLAN.md). It must implement coupled repeatable
rhythms and voltage streams rather than rename independent random draws.

## Pinned source

Mutable Instruments Marbles, `pichenettes/eurorack`
`f4cfa7697095a28d292bff758beb51deb49b5a86`; parent stmlib
`e3bd7c9cc00e4364166f9905c0509b6ffd0535ec`. Relevant firmware, resources,
helpers and build-context files are individually hashed by the
[reference manifest](../scripts/marbles_reference/source_manifest.json).
These components are MIT licensed; retain attribution and the full notice.
The original [manual](https://pichenettes.github.io/mutable-instruments-documentation/modules/marbles/manual/)
is the behavior reference. Native policies below remain distinct from board
ADC/normalization detection, hardware entropy, DAC calibration and CPU timing.

## Required kernel behavior

- One owned fallback LCG and original random stream per instance. Initialize
  T before X1, X2, X3, Y: 80 initial sequence draws. Execute the entire T block,
  then X1, X2, X3, Y in source order. Preserve conditional draws even when loops
  are locked. No per-channel RNG substitution or sample-interleaved rewrite.
- All seven T algorithms, source clock ranges, rate/clock ratios, jitter,
  bias, pulse-width/variation, loop length and independent T/X repeat switches.
  DEJA VU preserves fresh, locked and shuffled sequence behavior; source reset
  resets musical traversal without reseeding or erasing the learned loop.
- All X clock/control/range modes, spread/bias distributions, weighted scale
  quantization, lag processing, external CV register/shifted-sequence behavior,
  and all Y settings/dividers. Keep actual external startup stabilization and
  phase/ramp ownership. Source XY normalization uses undelayed T ramps; public
  T outputs have the firmware's two-source-frame delay.
- NoteFilter runs once per five-frame block. Register reacquisition lasts 20
  subsequent Process calls (3.125 ms at the declared source rate), preserving
  RewriteValue's actual loop/history positions. Array-relative pointer states
  become owned indices, including Clone and the original XY sequence alias.

## Native timing and state ownership

One fixed 32 kHz source core, in five-frame blocks, supports host 8–192 kHz.
The adapter must disclose exact buffer and gate latency and preserve callback
partition invariance. Input edge capture uses fixed bounded chronological
snapshots; queue bandwidth, overflow recovery and physical connection changes
are explicit. It cannot promise lossless arbitrary host-Nyquist pulses.
CV and authored controls are sampled at the declared source block boundary.
RESET applies once per completed block; multiple reset rises in one block coalesce.
Outputs use ideal voltage scaling and host-rate resampling, not DAC circuitry.

Compatible graph adoption keeps RNG, loops, PLL/ramp state, interpolation,
reacquisition, bridge phase and queued edges, while retaining receiving authored
controls and scales. Initialize, fresh reopen and seed changes start fresh.
Bypass outputs zero and freezes runtime. Runtime loops, held inputs and RNG
history do not become authored patch content by accident.

Six weighted scale slots and one active-slot selector persist as typed content.
Keep a positive finite interval, at most 16 ordered finite degree voltages,
byte weights, and at least one weight 255 so every quantizer threshold is
nonempty. Preserve slightly negative recorder degrees from octave-boundary
merging. Do not reduce scales to a twelve-note chromatic bitmask.

Scale editing and capture finish commit through shared revision/history/MCP.
A fixed runtime recorder can publish a bounded result tagged with module
identity, target slot and starting revision; a stale result must never replace
newer authored content. Prepare quantizers outside the callback. Record, stop
and cancel are commands; recorder scratch is transient. Presets, undo/redo,
save/reopen and exact weighted scales share the document authority.

## Explicit original-source boundaries

The source's floating random representation can equal 1.0. Its replay/hash
conversion can then attempt `2^32 -> u32`, undefined in C++. Native ordinary
float draws remain unchanged; the conversion uses an explicit saturating
endpoint policy. Terminal table interpolation and rounded pattern indices
also need declared safe endpoint behavior. Retain source-negative cases rather
than relabel the undefined source as a successful numerical comparison.

Original XY `LoadScale` loads only X1–X3. Y retains a root degree with weight
zero; higher quantization can access an empty mask's index 255. Native Y must
receive a validated scale too. Keep this deliberate correction separate from
raw source behavior, including an original-source sanitizer negative and a
positive host-setup case that invokes the original Y-channel loader explicitly.

## Qualification before counting completion

Genuine C++/audited/Rust evidence must cover all T models; one coupled
fresh/loop/shuffle progression; clock-source, register and sequence-clone
transitions; delayed CV reacquisition; weighted scales/slew/Y; filter/recorder;
and the two source-negative boundaries. The source checkpoint now passes 26 cases / 288,485 observations. All seven T
models match; 21 cases complete in the unmodified sanitizer, while five retain
the genuine terminal-read failure and match every complete defined prefix.
The full guarded reference and audited reference agree with Rust. Separate
original-negative/native-extension checks cover float-word, empty Y mask and
terminal interpolation. See [the precise reference scope](../scripts/marbles_reference/README.md).
This does not complete the native application contract below.

Then validate actual Engine bridge/normaling/reset/latency, compatible adoption,
seed/Initialize/reopen and callback allocation/deallocation. Complete shared
rack/browser rendering and real UI/MCP weighted edit, capture commit/cancel,
stale revision, history, presets and save/reopen. One combined macOS family
CI follows Event Fork, Rhythm Terrain and Recall Random. No whole-Marbles,
physical-hardware, device, performance-advantage or installation claim follows
from raw kernel equality alone.
