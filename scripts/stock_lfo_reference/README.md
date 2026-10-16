# Stock LFO frozen reference contract

This suite is declared before any oracle comparison. `acceptance.json` fixes the
protocol, comparison rules, domain and coverage; `fixtures.py` contains only
deterministic input schedules and schema checks. It does not implement an
oscillator, call an SDK, launch a compiler or query an oracle. The exclusive
`--freeze` operation refuses to overwrite an existing declaration.

```sh
python3 scripts/stock_lfo_reference/fixtures.py --freeze
python3 scripts/stock_lfo_reference/fixtures.py --check
```

After freezing, the first command must not be repeated. The immutable manifest
records acceptance/generator hashes, input hashes, case IDs, frame ranges,
independent anchors and per-rate counts. Every expanded frame is observed; there
is no decimation, discarded warmup or waveform alignment. The reference stream
is bounded below 200,000 frames per sample rate. Rust-only safety rows are
recorded separately and never count as numerical oracle passes.

## Actual implementations and protocol

The numerical target is the actual `LFO` class from Fundamental commit
`10dd0160c664770910e5584b7b00498cc48d9ddd`, retained as
`native-test-output/stock-lfo/reference/LFO.cpp`, SHA-256
`b25498dd4d49d2fce4673a8eaf3de97fa7ef1975e4d9bd732a308c1a1b38a9dd`.
Retain its upstream license. Compile the genuine class prefix, replacing only
the plugin include with SDK includes, against the genuine Rack SDK 2.6.6.
Production uses the exact `StockLfo` implementation through `StockLfoProbe`;
the probe must not contain a second oscillator implementation. Retain the
complete production source closure, SDK headers/library, drivers, binaries,
compiler commands/environment and input/output traces with hashes. Verify
production closure freshness before and after the run.

Input is one 76-field whitespace-separated row per process call:

1. Event: `0` process, `1` fresh then process, `2` runtime Initialize then process.
2. Six parameters: rate, FM amount, pulse width, PWM amount, offset, invert.
3. Four ports in order **FM, PWM, reset, clock**. Each has a channel count then
   sixteen f32 values. Inactive input slots are positive zero.
4. Consumer mask: sine=1, triangle=2, saw=4, square=8.

The source parameter slot `FM2_PARAM` and removed input `FM2_INPUT` are not part
of this protocol. The SDK adapter maps the four named ports to their genuine
enum slots. A parameter change assigns SDK params in place; production may
rebuild and adopt runtime, with receiving parameters winning. Initialize calls
the actual runtime reset method without resetting parameter values. Every case
starts fresh. A segment's Initialize event applies once, not to every repeat.

`rows(rate, acceptance)` yields `(case_name, frame_index, fields)`;
`fixture_text(rate, acceptance)` provides adapter input.
`cases(rate, acceptance)` exposes compressed segments, tags and independent
anchors. `safety_rows`, `safety_cases` and `safety_fixture_text` are the separate
production-only stream. `check_declaration()` returns the validated frozen
manifest. `fixtures.json.rates[rate][reference|safety].case_manifest` maps each
case to its half-open row range and frame-local anchors.

Output has 109 fields: four counts; 64 output-major raw voltage slots; sixteen
phases; sixteen reset-high flags; scalar clock state; clock frequency; timer;
light divider; five lights in RGB, invert, offset order. Initial snapshots are
asserted before processing, including vector reset detectors initially HIGH and
scalar clock detector initially UNINITIALIZED. All four output counts must match
the consumer/width contract. Voltage comparison covers every connected active
lane. SDK disconnected/inactive raw storage is retained as evidence but is not
claimed equivalent to host zero padding. All internal phases, including hidden
quartet lanes, triggers, clock state and lights remain compared on every frame.

## Gates and coverage

Triangle, saw, square, phases, clock frequency/timer, lights and every discrete
state must match exactly, including signed zero for exact float comparisons.
Only sine has a predeclared **3 microvolt absolute** tolerance: SDK uses its
Cephes SIMD approximation while production may use f32 sine. Identical phase
and f32 angle construction are still required. This tolerance is a strict,
unfitted engineering budget for the bounded argument and 5 V arithmetic, not a
proved global SDK/libm error theorem. Failures must remain failures. Compiler
contraction and fast-math are prohibited.

Independent checks supplement cross-implementation equality: active widths and
finite waveform ranges, zero phases at specified reset edges, initial snapshots,
divider cadence, held light values between ticks, exact mode lamps on ticks and
RGB mono/poly relationships. The normal matrix covers all four offset/invert
modes; rate endpoints and fractional rates; all FM widths and signed/fractional
FM amounts; PWM clamping and mono/short-poly normaling; strict pulse comparison;
reset startup HIGH, exact and adjacent thresholds, broadcast and lane-specific
triggers; accepted and rejected scalar-clock periods, startup arming, poly clock
channel-zero behavior, tempo changes, disconnect/reconnect and 65,536 missing
pulse frames; Initialize with retained detector/divider/light history; quartet
transitions 3→1→3, 5→4→5 and 16→1→16; output masks including no consumers,
individual outputs and first reconnect samples; and receiving parameter changes.

## Defined exponent domain and safety boundary

For every processed SIMD lane, compute only the input-domain check
`p=f32(rate+f32(FM*amount))`, `b=f32(p+127)`. Reference rows require finite `p`
and **0 ≤ b < 256**. These boundaries come from the actual SIMD exponent-bit
construction, not an empirically fitted error window. Exponent zero deliberately
produces zero, including effective pitch −127. Exponent 255 deliberately
produces positive infinity, including pitch 128. Finite high frequencies can also
overflow after clock scaling. These cases are included: the source's `fmin`
limits phase increments to 0.5. No high-frequency active sample is excluded.

Outside this frozen domain, SIMD integer conversion/shift can produce wrapped,
negative or nonfinite approximations. LFO uses the SIMD specialization: this is
not categorically described as scalar C++ signed-shift undefined behavior.
Finite extreme FM, out-of-domain pitches and nonfinite diagnostic inputs are
separate Rust-only safety cases. They assert finite bounded output/state and
recovery, without asserting numerical Rack equivalence. The safety stream must
never be sent to the SDK oracle.

The lower clock rejection branch (<0.001 Hz) cannot be reached from a fresh or
initialized float timer at the three declared rates. At 44.1/48 kHz, timer 512 s
has a half-ULP larger than the sample increment; at 96 kHz the same is true at
256 s. Rounded addition returns the same value, and monotonic accumulation
cannot cross this fixed point. The pure Python declaration verifies these
inequalities and fixed points. Thus the smallest reachable measured reciprocal
is at least 1/512 or 1/256 Hz, both above 0.001. We do not inject private timer
state, change sample time, manufacture millions of rows, or claim this branch
was executed. Too-fast rejection remains reachable and is covered.

This is bounded module numerical qualification. Host graph lifecycle and
transport, UI Initialize defaults, bypass, native rendering, real audio devices,
full Rack import and general compatibility require separate evidence.
