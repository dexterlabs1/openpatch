# Stock Mix

`stock_mix` is a separate six-input polyphonic mixer, bringing the catalog to
75 kinds. Existing `mix6` patches keep their original mono broadcast, arithmetic
and safety contracts. The corrected production Core comparison passes
against the genuine Rack reference. Application checks and remaining limits are
listed below.

Use **File → Stock Mix layered voice** for three oscillators mixed with averaging
and a slow LFO. Average and Invert are direct panel toggles; the same panel
appears in the module browser. The example routes one mixed signal explicitly
to both audio channels and compares it with an oscillator in Scope.

## Signal contract

Inputs `in1`…`in6` feed `out`. `level` ranges from zero to one and defaults to
one. Invert and Average default off. They are persistent Boolean module data
in the reference; OpenPatch exposes durable `invert` and `average` controls.
There are no source module lights or bypass routes.

The output requests the largest effective input channel count, with a minimum
of one. For each lane, the processor adds the six raw input lanes in jack order,
starting from positive zero. It does **not** broadcast mono inputs. For example,
mono `[2]` mixed with `[4, 6, 8]` produces `[6, 6, 8]` at unity level.

Gain starts at `level`, is multiplied by −1 when Invert is enabled, then is
divided by the number of connected input jacks when Average is enabled. An
empty mixer uses denominator one. The lane sum is multiplied by this gain
after all six additions. These are ordered float32 operations; moving the
division after the sum can change the result. A connected zero-volt source
counts toward averaging. A physically connected source requesting zero
channels becomes effective mono zero; an absent cable does not count.

The processor writes complete SIMD quartets **before** updating the output
channel count. Shrinking a connected output then clears the retired active
lanes. Disconnected outputs retain zero channels while raw storage can still
be written. Inactive tails and signed zero are retained in private ports;
public frames expose only active lanes. The module does not clip amplitude:
six +16 V inputs produce +96 V at unity gain.

## Host state and arithmetic domain

Stock Mix uses retained host inputs. Cable transfer runs after the producers
finish a sample, preserving the real first-sample latency and connection
history. A single cable copies finite raw bits, including subnormals. Stacked
cables use ordered additions and host mono expansion; that expansion is
distinct from the mixer's own raw-lane reads. Removing one stacked cable can
leave the previously combined input until the next transfer.

The declared comparison domain is finite float32 source controls in [−16,16]
and level in [0,1], including signed zero, subnormal values, normal boundaries
and underflow. Genuine Rack Engine processing sets nearest rounding and
flush-to-zero/denormals-are-zero (FTZ/DAZ). The native contract applies this
arithmetic behavior locally to Stock Mix and canonical stacked-input additions,
without changing thread-wide floating-point flags. Raw single-cable copies,
control snapshots and inactive storage are not flushed. This does not qualify
the arithmetic of every older module.

Initialize restores level to one while retaining Invert, Average and private
port history. Reference data loading retains a flag when its key is absent;
a present JSON `true` enables it, and any other present value disables it.
Non-object data retains both flags. Saving emits both Boolean keys.
Application randomization and preset/project integration have separate gates;
Rack random-number-stream identity is not claimed.

Entering or leaving bypass clears the connected output's active prefix and
makes its count one. A disconnected output keeps its raw storage. Bypassed
processing makes no new output writes; connection changes still affect ports.
Same-incarnation graph adoption retains private port state while receiving
controls win. Recreation starts a fresh module. Ordered connection and bypass
events must survive graph preparation and coalescing.

## Reference evidence and remaining gates

The reference is the actual
[Fundamental Mixer class at 10dd0160](https://github.com/VCVRack/Fundamental/blob/10dd0160c664770910e5584b7b00498cc48d9ddd/src/Mixer.cpp)
inside genuine Rack Engine 2.6.6. Its source SHA-256 is
`7cebae980040e2d6bd0df03456e6952144c9ab2b5ceddf66dc256608ad69d0d6`.
The arithmetic environment is pinned to
[Rack system.cpp at 8ea470f3](https://github.com/VCVRack/Rack/blob/8ea470f343058ade6f75f16d503a1eb330e1700c/src/system.cpp),
SHA-256 `cc10706acb8029b2769618a09a44233be449d73c59b6f1d2d2e4ed248db85c18`.
The adapter invokes the genuine class and Engine; it does not implement a
second mixer equation. Original upstream source and license notices are
retained with the reference attempt. No upstream artwork is used.

The [independent declaration and protocol](../scripts/stock_mix_reference/README.md)
were reviewed and frozen before the first SDK or production DSP execution.
The genuine SDK gate passes **28,383 cases, all 86,355 snapshots and 708
independent hand anchors** at 44.1, 48 and 96 kHz. At each rate it covers 9,440
fresh cases and 21 ordered histories. A draft cardinality typo was corrected
during input-only preflight; no case, event or anchor was removed or changed.
The failed preflight is retained.

Each case records its constructor state and every event, including events that
run no samples. Fresh cases run two single-frame steps; neither is discarded.
The protocol retains all 16 raw float32 bit patterns and channel counts for
six source outputs, six mixer inputs, the mixer output and two sink inputs.
It also checks actual level, flags, saved data, source controls, frame count,
bypass and all physical cable multiplicities. Independent topology bookkeeping
checks the cable registry after every event. Hand anchors cover arithmetic
order, no mono broadcast, averaging, signed zero, FTZ/DAZ, raw retirement,
partial data loads, reset, adoption, recreation and stacked-input histories.

The **corrected production gate passes** the same 28,383 cases, 86,355
snapshots and 708 anchors with zero discarded snapshots. It uses six actual
Core `poly_constant` sources, Stock Mix, two actual `mult` sinks and real graph
adoption. Both complete JSONL traces are byte-identical, with SHA-256
`59e6df1c9c29b3cae2b8247ea8dacf28d4f09645bf95df6df9d5132b81dbc934`.
See the [paired report](benchmarks/stock-mix-reference.json) and
[SDK report](benchmarks/stock-mix-sdk-reference.json). The production build
provenance retains 82 source-closure files and the actual probe hash.
Comparison covers every field and raw lane, without active-lane filtering,
float tolerance, omitted events or post-failure exclusions.

NaN/infinity injection and finite overflow are outside this real-source matrix;
the ordinary production source cannot express them. Cross-rate adoption,
general Rack import, native devices and hardware accuracy remain separate gates.

The corrected source passes all 2,016 Rust workspace tests, with zero failures
and six existing ignored tests. These include actual flag clicks, bounded level
entry, matching rack/browser primitives at five scales, playable factory signal
paths and layout checks. Strict workspace Clippy and formatting pass. All 99
Python/MCP checks pass against the corrected isolated release CLI, including
numeric transport of the newly discovered rounding boundaries.

The corrected offline performance measurement finds a +5.34% paired median
mono-routing time cost versus PR15, +0.80% for 16-voice routing and mixed-voice
medians near parity. See the [measurement and limits](PERFORMANCE.md#stock-mix-and-explicit-ftzdaz-measured-incremental-cost).
Both timing attempts are retained; only the corrected source is the shipping
candidate. The corrected optimized ARM workspace build and separate ad-hoc
signed bundle pass static validation: its CLI is the exact measured/MCP-tested
binary, all four fonts and notices are present, and no Rack runtime library is
linked. The bundle was not installed or launched. Exact-head CI and all 133
native captures are separate pre-merge gates. Native Mac/device qualification
remains open.

## Additional pre-rounding boundary

A separate [independent supplement](../scripts/stock_mix_rounding_reference/README.md)
was declared and frozen after the first large matrix passed, before processing
any of its new inputs. Arm FZ decides whether a result is tiny **before**
rounding. An exact product or quotient just below minimum normal can round up
to minimum normal under ordinary nearest-even arithmetic; a post-rounding
subnormal check alone misses that case.

The supplement retains 192 cases, 576 complete snapshots and 1,728 hand
anchors across three sample rates, both signs and Invert/Average settings.
It includes exactly-normal and above-normal controls that round to the same
boundary, so clearing every rounded minimum-normal result would fail.
The genuine SDK passed the anchors; the initial Core probe failed the boundary
check.
That real failure and both complete traces remain retained. After correcting
multiplication and division, every supplemental snapshot is byte-identical to
the retained genuine trace. The original large matrix was not changed.
See the [red/green report](benchmarks/stock-mix-rounding-reference.json).

The correction checks only a rounded minimum-normal result, using exact f64
arithmetic to distinguish the tiny case. Addition needs no extra threshold
check because subnormal cancellation of DAZ-normal operands is exact.
The application still does not modify global FPCR/MXCSR state. Qualification
is bounded to the declared ARM reference environment, not a promise of identical
underflow policies across every Rack SDK platform. The initial bundle and
timing attempt are retained as superseded evidence;
checks of the corrected source are reported separately.
