# Frozen Frames keyframer reference

This probe builds the genuine, unmodified `frames/keyframer.cc` and complete
`frames/resources.cc`, with `stmlib/stmlib.h`, at the commits and actual SHA-256
hashes in `source_manifest.json`. No generated C++ replacement or source fix is
used. Original authorship and MIT permissions are retained in `NOTICE.md` and
in each fetched source. Rust tables are exact integer extractions from that
pinned resource file. This reference covers keyframe interpolation and 2164 DAC
codes, not the module's analog audio transfer, storage, firmware timing or hardware.

Construction is explicit: a static zero-initialized C++ object, TEST `Init()`
(which does nothing), `Clear()`, four linear/response-zero settings and four
zero immediate values. Cases then apply their declared operations. Rust `new`
declares those same finite defaults, rather than claiming a TEST Init behavior.

Root runs serially:

```
python3 scripts/frames_reference/run.py --validate-only
python3 scripts/frames_reference/fetch.py
python3 scripts/frames_reference/run.py
python3 scripts/frames_reference/compare.py --reference native-test-output/frames-reference/attempt-002
```

Every attempt also builds a second executable with `-fsanitize=undefined,bounds`
and `-fno-sanitize-recover=all`. All positive inputs run through that executable,
with zero diagnostics required and complete output hashes matching the optimized
reference. Guards still run before excluded source accesses. A passed attempt
requires both the optimized reference and this defined-domain audit.

Every attempt retains sources, harness, cases, compiler identity, exact compile
command, all input/output records and artifact hashes. C++ flags are C++11,
`-O2 -DTEST -ffp-contract=off -fno-fast-math`. Each Rust comparison first verifies
the immutable C++ artifacts and retains its actual production sources/toolchain.
Integer JSON fields must all match exactly, including four levels, four DAC
codes, RGB, position, nearest_keyframe and FindNearestKeyframe results.

The cases exercise empty immediate values, one key, multiple keys, 64 keys,
all six easings, every channel, response endpoints and blends, nonzero/equal/
descending ramps, insertion/replacement/removal/Clear, SampleAnimation, strict
tolerances, all 65,536 ConvertToDacCode gains at responses 0 and 255, and selected
127 and other blend bounds. `D_RANGE` is expanded into individual retained input
and output lines. The response-zero sweep is suitable for deriving a separately
declared native inverse DAC transfer, without inventing a source audio oracle.

Known source quirks are retained: FindNearestKeyframe returns the first match
within a strict tolerance (not minimum distance); Evaluate's nearest_keyframe
is its own indexing formula, with u16 wrap; full capacity rejects replacement;
Clear preserves immediate/settings/last outputs; removing the last key leaves
stale unused slots. RemoveKeyframe can match such a stale slot and reduce the
logical count again, and that defined quirk is covered in the editing case.

The following undefined source domains are excluded from positive equivalence:

* Evaluate after the last timestamp at 64 keys reads `keyframes_[64]`.
  The positive 64-key case ends at timestamp 65535. A separate harness guard
  rejects this access before calling the unmodified source.
* Evaluate an exact nonfirst key timestamp with a curved easing computes scale
  65536, then reads easing table index 1025 (one beyond its 1025 entries), even
  though the multiplied interpolation coefficient is zero. Positive curved
  cases use adjacent timestamps; linear/step exact-key endpoints remain covered.
  A separate harness guard rejects the undefined curved endpoint.

The Rust port supplies a zero sentinel for missing storage and a zero-coefficient
safe endpoint for the curved table boundary. These are declared native safety
policies, with no source-equivalence claim in the excluded domain. Firmware
Save/Calibrate/persistence are outside this raw kernel.

Bounce has an additional source quirk: its table interpolation multiplies a
signed negative slope by an unsigned uint32 scale fraction, then shifts
logically. The native kernel preserves that defined promotion, including
overshoot, rather than silently repairing it to a signed interpolation. Those
overshoots can make the subsequent signed level multiplication overflow for
large channel deltas. Positive bounce cases conservatively limit all deltas to
20000 (below the 32-bit product bound), and exclude easing-mode bounce
SampleAnimation, whose full-range ramp can overflow. A third explicit harness
guard rejects this domain before source execution. Rust uses explicit wrapping
multiplication there as a declared native extension; no C++ equivalence claim
covers signed-overflow inputs.


## Root qualification checkpoint (2026-10-17)

Historical attempt001 preceded the stricter Bounce integer-domain audit and is
not qualification evidence for that domain. Its snapshot remains unchanged.
`attempt-002` passed all 25 positive cases, all three pre-execution guards and
the separate UBSan/bounds audit with byte-identical records. Initial Rust
comparison001 matched all 163,013 integer records. Final comparison002 rechecks
the formatted production kernel against that same immutable attempt002; its
source/toolchain hashes are retained separately from the C++ oracle.

The native VCA transfer is a separate, declared model. `build_native_gain.py`
verifies the immutable response-zero 65536-gain sweep and generates a monotone
4096-entry inverse with exact endpoint saturation. Its source trace hash is
retained in `frame_scene/native_gain.rs`. It is not analog calibration.
[Native contract, structured authoring, limits and example](../../docs/FRAME_SCENE.md).
