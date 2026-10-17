# Pinned Stages source reference

The raw Rust generator is source-qualified on macOS ARM; the native instrument is a development candidate, not installed. [Native contract and bounds](../../docs/SEGMENT_GARDEN.md).

The probe compiles the genuine, unchanged SegmentGenerator, resources, Tides ramp extractor and stmlib units/RNG at the commits and SHA256s in source_manifest.json. Static-zero objects mirror firmware storage. Each physical generator receives the actual shared array of 13-step hysteresis note quantizers, with an explicit null-pointer variant to test both modes. A declared seed controls the original shared PRNG.

Root runs serially:

```
python3 scripts/stages_reference/run.py --validate-only
python3 scripts/stages_reference/run.py
python3 scripts/stages_reference/run.py --cases scripts/stages_reference/completion_cases.json
```

Then run `compare.py --reference` with each runner's printed evidence directory. The retained checkpoint below uses attempts003/004; fresh checkouts number their own attempts from001.

Each attempt retains source, inputs, output float bits, source gates/segment indices/RNG, compiler flags and hashes. A second executable audits the same positive inputs with undefined-behavior, bounds, float-cast-overflow and float-division sanitizers; its complete records must match the optimized source. Every failed attempt remains unchanged.

Cases cover all twelve standard single-stage dispatch modes at block sizes 1 and 8, delayed sample-and-hold, signed input changes, 36-segment capacity, mixed stage/loop transitions, all seven sequencer directions with enabled/disabled hardware note quantizers, slave contours and hidden state during shrinking groups and leader/slave/leader reconfiguration. Source activity is recorded so silent output cannot hide a comparison domain.

The raw port supports only the standard RAMP, STEP and HOLD configurations; ALT audio oscillators are explicitly rejected, not substituted. Endpoint guards reject primary 2 and secondary 1 before source LUT overreads. The initial expected-negative frame-zero tapped-clock fixture actually produced finite Stages LFO output. Its failed expectation is retained in attempt 001. The source frequency return that failed in Tides is unused by this standard Stages path; attempt 002 then showed that finite visible output still hides an internal frequency division by zero under the stricter sanitizer. The 8,192-frame fixture is retained as an audit-negative: optimized output must be finite and the sanitizer must diagnose the internal division. It is excluded from the 61 positive cases, and no source or sanitizer checks were weakened. Native gate priming must be declared separately. Native safe-endpoint, RNG ownership, sample-clock bridge and jack/voltage policies are separately declared adaptations.


## Qualification checkpoint, 2026-10-17

Frozen attempt003 passed 61 cases/780,288 records and retained two guarded LUT boundaries plus the internal division audit-negative. Supplementary attempt004 adds eight faster completion/capture cases and 172,608 records. These strengthen transition coverage without replacing any base fixture or modifying the source. Final Rust comparisons005/006 match all 952,896 records exactly, including float bits, segment metadata and shared PRNG state. Their retained source/toolchain hashes include the production kernel/resources and reused Tides extractor. Standalone probe compilation exposes that same extractor through a small module alias; Cargo compiles a single shared implementation.

The native Engine acceptance retains two compact genuine C++ golden traces: independent LFO output and six linked outputs through live gate edits. The latter exercises actual slave contours and five stage indices. It accounts for native graph cable delay separately from the declared eight-source-tick bridge buffer. Safety extensions, clock priming, output calibration policy, document/history, UI and actual-device claims remain distinct from this raw oracle.
