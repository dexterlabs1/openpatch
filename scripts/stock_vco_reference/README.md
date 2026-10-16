# Frozen Stock VCO qualification

The final declaration includes a pre-comparison amendment requested after the first pure-Python freeze: a complete minimum-knob cycle in all four mode combinations and a 65-frame stopped tail drain at each rate. No oracle comparison preceded this amendment.

This bounded suite targets the actual Fundamental `VCO` at commit
`10dd0160c664770910e5584b7b00498cc48d9ddd`, SHA-256
`3745dd5e7fafd3b4260fed6ecdb19fb19f1573943cc5f0b1d55fa30fd2efe67d`.
The retained source and upstream license belong in ignored
`native-test-output/stock-vco/reference`. Compile the actual class prefix against
genuine Rack SDK 2.6.6; do not replace its waveform, filter or reconstruction
code with a reference adapter implementation. Production probes invoke the exact
new `stock_vco`, preserving the existing `multi_vco` behavior.

## Freeze before comparison

```sh
python3 scripts/stock_vco_reference/fixtures.py --freeze
python3 scripts/stock_vco_reference/fixtures.py --check
```

Freeze uses exclusive creation and must run before any numerical oracle output.
The four acceptance files are then immutable. Acceptance and generator hashes,
every expanded input hash, case IDs/ranges/anchors, frame counts and full-buffer
checkpoint counts are retained in `fixtures.json`. The generator performs only
input construction, f32 schema/domain validation and hashing; it contains no
oscillator, reconstruction kernel or DSP oracle. `verify.py` applies these gates
without generating expected oscillator trajectories.

API: `cases(rate,a)` and `safety_cases(rate,a)` expose compressed schedules;
`rows(rate,a)` and `safety_rows(rate,a)` yield `(case,frame_index,fields)`;
`fixture_text(rate,a)` and `safety_fixture_text(rate,a)` expand adapter input;
`check_declaration()` validates and returns the frozen manifest. Every process
and bypass frame is observed. Nothing is discarded for startup, warmup, spectral
alignment, reset, narrow PWM, voice retirement or consumer changes.

## Locked wire protocol

Each input row contains 78 fields:

1. Event: 0 normal; 1 fresh; 2 runtime Initialize; 3 explicit same-rate rebuild/adopt.
2. Six params: frequency semitones, FM amount, pulse width, PWM amount, linear FM,
   soft sync. The SDK sync parameter is `1-soft_sync`.
3. Four ports: pitch, FM, sync, PWM. Each is a count plus sixteen f32 slots;
   inactive slots are positive zero.
4. Consumer mask (sine=1, triangle=2, saw=4, square=8), bypass flag, checkpoint flag.

Every case starts fresh. Parameter changes update SDK parameters in place;
production rebuilds with receiving parameters and adopts previous runtime.
Event 3 also forces this adoption when parameters did not change. Event 2 calls
runtime Initialize, not UI reset-to-default parameters: this pinned class has
no reset override and retains oscillator/filter/buffer history. Call source
`onSampleRateChange` before the first sample; otherwise the DC filters are not
initialized correctly. Use actual SDK `FREQ_C4=261.6256f`.

The explicit bypass shim skips the process method, retains runtime and returns
mono positive-zero on all four outputs, even when unconsumed. This represents
OpenPatch's generic host bypass policy, not proof of native Rack Engine bypass.
Public Engine tests independently exercise the real production bypass path.

Every output row has 218 compact fields: counts4, output-major voltages64,
phases16, directions16, last-sync16, square-latches16, filter lowpass histories64
(sine/triangle/saw/square), buffer indices16 (wave/quartet), divider1 and lamps5
(RGB/linear/soft). A checkpoint row appends 2,048 logical future correction
values in wave/lane/next-sample order. The source view is
`buffer[bufferIndex+offset]` for offset0..31, not stale physical buffer storage.
Checkpoints include startup, divider/buffer boundaries, case end, explicit
Initialize/adoption, bypass/consumer/width changes and analytical anchors.
Drain cases test the full future horizon; continuous output/state comparisons
cover every intervening frame. Full ring contents are not claimed checked on
non-checkpoint frames.

Separate `--kernel` output is 1,024 actual prepared coefficients: 512 impulse
correction values (already minus1), then 512 ramp values. Both use chronological
index `z*16+o` from source table `[o][z]`. Transposition is adapter plumbing, not
kernel generation. Independently require first impulse=-1 and first ramp=+0.

## Numerical gates

Phases, directions, sync history, latches, counts, cursors, divider and raw lamp
state must match exactly. The raw red/green lights can be negative: source
smoothing does not clamp them, and display clamping must not enter the probe.

Prepared kernel error is bounded by **2e-7** absolute; normalized correction and
filter history by **5e-5**; connected active waveform voltage by **5e-4 V**.
There is no relative tolerance, shifting or fitted gate. These are predeclared
engineering budgets for independent double FFT/libm coefficient preparation and
overlapping f32 corrections, not universal mathematical error bounds. All
compared fields must be finite. A failure remains a failure.

Only disconnected/inactive SDK output voltage storage and dead physical buffer
storage are excluded. Output counts always compare. Rust output padding must be
positive zero. Hidden SIMD phase/sync/filter/cursor history remains visible;
correction insertion only touches source-visible lanes even though connected
waveform filter/buffer operations advance all four lanes of an active quartet.

Coverage includes frequency and width endpoints, FM/PWM sign and normaling,
linear zero-frequency stop, sample-rate and0.49 phase caps, rising/falling/zero
and fractional sync boundaries, hard/soft continuation, audio-rate narrow PWM,
the four mode combinations, startup and complete correction tails, quartet
3→1→3/5→4→5/16→1→16, consumer-specific frozen tails and first reconnection,
Initialize/adoption, lights and the declared bypass shim. Exact zero-frequency
and sync-endpoint anchors, drained tails, cursor/divider cadence and frozen
state checks supplement cross-implementation equality.

## Domain, safety and spectral scope

Reference rows use source-ordered f32 exponent arithmetic with finite effective
pitch and biased exponent `f32(p+127)` in `[0,256)`. Zero-exponent and positive
infinity-exponent endpoints are included; positive infinity is subsequently
clamped by the actual source frequency limits. Unused input padding stays zero.
Outside this domain, the SIMD intrinsic conversion/shift may produce wrapped,
negative or nonfinite approximations; it is not categorically scalar C++ shift
undefined behavior. Separate production-only fixtures exercise extreme finite
and nonfinite CV and recovery. They are never sent to the SDK and cannot be
reported as numerical equivalence passes.

Three declared tone cases retain all12,288 frames. A separate spectral diagnostic
may inspect frames4096..12287 using a Hann window, reporting both fundamental
estimates and off-harmonic energy with explicit bin guards/leakage limitations.
This spectral window does not remove any frames from numerical acceptance.
Numerical equivalence does not establish universal alias rejection or better
audio quality. The spectral diagnostic has no fitted acceptance threshold.

Retain source closures, upstream licenses, complete SDK headers/library, actual
compiler flags and FPU/target environment, toolchains, binaries and full traces
with hashes. Prohibit fast-math/contraction and verify production-source freshness
before and after execution. Preparation must remain outside the audio callback;
heap/device deadlines, fresh-rate/incarnation handling, real Engine bypass,
shared native panels, Mac input, projects and MCP history require separate tests.
