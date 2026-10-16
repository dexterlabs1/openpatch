# Random modulation

`random` is an original Rust implementation of a bounded stock Random digital
behavior contract. It is separate from `random_values`, whose seven outputs
hold independent random voltages. Random adds a shared clock, probability,
spread and four simultaneous ways to move between voltage targets. Neither
instrument is a calibrated hardware model.

## Use it

Choose **File → Random motion voice**. STEP controls a scaled, quantized pitch;
EXP moves a filter; TRIG starts a fast envelope; LIN and SMTH feed the rack
Scope. All five outputs are used. The saved native seed makes fresh renders
repeatable. The example is [random-motion-voice.json](../examples/random-motion-voice.json).

The 24 HP rack panel and browser thumbnail share one renderer, prominent
left-aligned title and the same control/port geometry. Rate readout and typed
entry use Hz while the patch parameter stores log2 Hz. Ratios and signed CV
depths display percentages. **Edit random seed…** in the module context menu
opens exact integer entry; it does not add an invisible control to panel Tab
navigation. Generic precision entry, keyboard adjustment, Initialize,
Randomize, presets, duplication and shared UI/MCP history remain available.

| Parameter | Default | Stored range |
| --- | --- | --- |
| `rate` | 1 (2 Hz) | log2(0.002)…log2(2000) Hz |
| `probability` | 1 | 0…1 |
| `spread` | 1 | 0…1 |
| `shape` | 1 | 0…1 |
| `rate_cv`, `probability_cv`, `spread_cv`, `shape_cv` | 0 | −1…1 |
| `unipolar` | 0 | 0 bipolar; 1 unipolar |
| `seed` | 1 | integer 1…16,777,215; native extension |

The six named inputs are `rate_cv`, `shape_cv`, `trigger`, `external`,
`probability_cv` and `spread_cv`. Outputs are `stepped`, `linear`, `smooth`,
`exponential` and `trigger`; there is no `out` alias. Rate CV adds octaves,
whereas the other depths scale incoming voltage by 1/10 before clamping the
effective control to 0…1. Control and clock inputs read channel zero only.

## Timing and voltages

Internal rate uses the specified rounded fifth-order exponential. Patching
`trigger` selects external clocking: a Schmitt detector rearms at 0.1 V and
fires at 2 V. An initially high clock arms without firing. Clock frequency is
measured from an f32 timer, including the first edge after startup, and remains
in effect between edges. Internal clock phase is retained while external
clocking is selected. Each frame advances by at most half a cycle.

A raw clock event flashes RATE and updates external timing even if probability
rejects it. Accepted events move the prior target to the interpolation origin,
choose/capture the next target, reset interpolation phase and emit a 1 ms,
10 V mono trigger. Phase advances before producing that frame's CV. It reaches
one and then holds. Probability equality is accepted: an explicitly supplied
zero draw at probability zero is accepted by the pinned comparison rule.

Without `external`, a native uniform target lies in −5…5 V or 0…10 V, and
SPREAD blends it with the previous target. Offset changes affect future random
targets, not existing held values. With external voltage patched, its active
lanes are sampled directly, without spread or offset. External width sets the
four CV output widths, with a minimum of one and maximum of sixteen. Clock,
phase and interpolation weights remain shared; inactive held lanes are retained
and can reappear when width grows. Unused output storage is zero.

STEP divides each transition into 1…16 upward-rounded fractions, selected by
SHAPE. LIN reaches its target over the selected fraction of the period; SMTH
uses a cosine easing of that progress. EXP uses the specified shape-dependent
exponential with explicit near-zero/near-one branches. Shape zero reaches the
target immediately. The five lamps show actual rate/accepted events, effective
shape/spread and offset state; the host transports these values coherently and
preserves their emission in dark-room mode.

Version-2 cables add their normal one-sample transport delay. There is no extra
module delay or independent per-voice clock. A zero-channel host input behaves
as unpatched, including normalization to the internal clock/random source; this
is the native engine boundary, not a claim about Rack's separately connected
zero-width cable bookkeeping.

## State, safety and real-time ownership

A compatible identity/kind/sample-rate edit preserves held voltages, phases,
clock measurement, Schmitt state, pulse and lamps, while receiving the new
controls. An unchanged seed preserves the native PRNG; a changed seed replaces
only its sequence. Initialize restores default controls while keeping runtime
histories, consistent with the source's inherited reset. Explicit Engine state
reset, a new identity or a different sample rate starts fresh runtime. Bypass
emits mono zero on all outputs and freezes runtime until restored.

Projects and presets store parameters and seed, not running voltages or PRNG
progress. Reopening starts a fresh sequence. Randomize changes ordinary controls
and the offset switch while leaving seed and CV depths intact. These differ
from Random Values' independently documented durable held-state contract.

The native generator is xorshift32 with shifts 13/17/5, initial state
`seed XOR 0xA341316C`, and a uniform value from its upper 24 bits divided by
16,777,216. Probability consumes a draw only below one; accepted internal
captures consume a target draw. This is deterministic native behavior, not the
same sequence as Rack's global random generator.

Supported Engine/probe rates are 8…192 kHz. Nonfinite incoming values map to
zero; finite input is bounded to ±1,000,000 V and the effective internal-rate
exponent to −120…120. This explicit native safety domain is separate from
undefined upstream arbitrary-input exponent paths. Runtime processing and
compatible adoption use fixed storage; preparation and graph ownership stay
outside the callback. No file access, lock or PRNG allocation occurs in a step.

## Qualification boundaries

The [actual-class harness](../scripts/random_reference/README.md) pins Fundamental
`10dd0160c664770910e5584b7b00498cc48d9ddd` and compiles the genuine class only in
ignored temporary/build storage against the separately supplied Rack SDK 2.6.6.
Only the uniform call sites receive a controlled f32 stream. Production Rust
uses the same step function with those draws; the clock/interpolation logic is
not replaced in either adapter. The frozen gate checks every frame from zero,
all five outputs, all sixteen output/held storage lanes, lamps, clock/timer/
Schmitt/pulse state, event counts and draw consumption. There is no alignment,
startup exclusion or uncontrolled global-PRNG identity claim.

The final [checked report](benchmarks/random-reference.json) passes 75 cases,
484,548 frames and 152,342,424 comparisons at 44.1/48/96 kHz, with zero measured
voltage/state error. All 225 compressed input/reference/Rust trace artifacts
have verified compressed and raw hashes, with an evidence index, both test
binaries and the exact build command retained in the ignored evidence directory.
All 74 production source-closure hashes were rechecked. The gate remains
20 µV + 2 ppm for CV and tighter explicit state tolerances; exact discrete
observations cannot be hidden by that voltage tolerance.

Twelve core contract/public Engine tests cover delayed routing, sixteen-lane
feedback, block partitioning, lifecycle, seed changes, all outputs/taps and
first-callback allocation/reallocation/deallocation guards. The native callback
pipeline also verifies real light packets and revisions at one/sixteen lanes
without heap activity. Shared panel/browser controls, typed entry, keyboard and
cable gestures, the factory voice and native room-light rendering have headless
integration coverage. All 82 Python/MCP tests pass, including new Random
transactions, captures, presets, history and portable save/load. Strict Clippy,
format checks and the workspace release build pass. The separate Mac ARM app
bundle has verified ad-hoc signing/notices and no Rack SDK runtime dependency;
it has not been installed or launched.

Supported-platform CI and visual review of the three new Random scenes among
103 configured native captures remain release gates. Numerical agreement and
headless host tests do not qualify native window/input/device behavior, audio
latency, performance relative to Rack or full host/plugin/import compatibility.
Rack pre-2.0 parameter migration and general `.vcv` conversion are not supplied
by this new native kind.
