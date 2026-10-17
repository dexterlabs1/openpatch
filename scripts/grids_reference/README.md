# Pinned Grids raw reference

`python3 scripts/grids_reference/run.py` fetches and verifies the exact original
firmware, builds the genuine C++ PatternGenerator/Clock/Random/resource files,
builds an ASan/UBSan/bounds variant, and compares both traces byte for byte with
the production Rust raw kernel. Root owns serial execution; writing these files
alone does not establish a pass. `--validate-only` checks the corpus without DSP.

The exact repositories, commits and source hashes are in source_manifest.json;
see NOTICE.md for GPL attribution and the audited portable host overlay. The
overlay excludes the upstream-broken, proven-unused U24Sub fallback and retains
all Grids arithmetic unchanged, with original/overlay hashes and a unified diff. Resources
contain every one of the 25 96-byte nodes, all 1,024 Euclidean words and all 512
tempo words. The trace compares every resource word independently and retains
little-endian table hashes. `generate_resources.py` extracts integers directly
from the pinned C++ resource file. `make_cases.py` deterministically emits the
bounded input corpus; neither generator executes DSP.

Coverage includes all map cells and coordinate endpoints, lane-density bounds,
all 128 source-defined packed option bytes (resolution enum values 0–3), seeds
including zero and high-bit truncation, chaos and
bar perturbation, resolutions/increments 6/3/1, Euclidean lengths 1–32 and density
indices 0–31, reset/retrigger/falling behavior, counter wrap, live mode changes,
and clock BPM 0–511, real phase traversal, wrap, swing, lock and reset. Clock
BPM 512 and map step 96 are explicit original-source undefined reads: the
sanitized original must reject both, and neither is included in valid parity.
Options::unpack also performs an undefined enum load for EEPROM low bits 4–7
before its apparent clamp. Bytes 4 and 7 are explicit sanitizer negatives.
The real failure remains in attempt-002/packed_options/sanitizer.stderr; enum
sanitizing stays enabled. Rust safely clamps those malformed bytes as a guarded
extension; native parameter resolution remains 0–2. Declared source LAST=3 is
valid to load and then clamps to 2.

`native_scan_composite` additionally freezes original `HandleClockResetInputs()`
with `++tap_duration` before it and `IncrementPulseCounter()` afterward, matching
the relevant source ISR ordering. It covers normal nonfactory internal/external
clock operation and reset/clock simultaneity. Input is active-low physical bits;
MIDI is empty. ADC scanning, tap-button debounce/actions, UI, LEDs, shift-register
I/O and real board timing are excluded. The pinned makefiles default to 20 MHz;
`kUpdatePeriod = F_CPU / 32 / 8000` truncates to 78, giving a physical timer rate
of 8,012.8205128 Hz. Native 8 kHz scheduling is a deliberate host adaptation.

Protocol: first line is a u32 seed (original RNG truncates to u16), followed by
integer `opcode repetitions arguments...` rows. Opcodes: 0 packed-options + three
setting bytes + three densities; 1 TickClock(increment); 2 Retrigger; 3 pulse
counter; 4 falling; 5 reset; 10 clock update(BPM,resolution); 11 clock tick;
12 clock wrap(two's-complement i8 byte); 13 clock reset; 14 lock; 15 unlock;
20 dump map(x,y); 30 dump all resources; 40 composite scan(clock,reset).

Every operation emits the 23-integer array:
`[index,state,pulse,step,euc0,euc1,euc2,perturb0,perturb1,perturb2,counter,first,
beat,rng,swing,packed_options,locked,bpm,phase,increment,falling,raising,past]`.
Resource/map rows begin with `e`, `t`, `n` or `m`. Each composite scan also emits
`["s",index,tap_duration,latched_swing,input_bits]` for golden Engine selection.
Reset deliberately preserves the source's output, beat flags, perturbation and
pulse counter. Clock reset preserves increment/falling/lock. These are observed
contracts, including surprising source behavior.

Attempts retain exact source, extracted helper, host overlay, toolchain versions,
preprocessed translation unit, binaries, inputs, complete traces, diagnostics,
negative reports, source/resource hashes and final result. A raw pass establishes
bounded source arithmetic/state agreement, not whole-application, hardware,
interrupt-cycle or audio-device equivalence. Native Engine golden tests are a
separate root-owned check.

## Native Engine golden

The native host corpus additionally applies the original menu's pattern reset
on resolution edits. Reproduce the qualified 49-case corpus and freeze only
original-source output columns (no expected DSP calculation):

```sh
python3 scripts/grids_reference/host_golden.py cases /tmp/grids-host-cases.json
python3 scripts/grids_reference/run.py --cases /tmp/grids-host-cases.json
python3 scripts/grids_reference/host_golden.py freeze native-test-output/grids-reference/attempt-NNN crates/openpatch-core/tests/fixtures/rhythm-terrain-native-host.json
cargo test --locked -p openpatch-core --test rhythm_terrain
```

Replace `attempt-NNN` with the successfully retained attempt. The committed
fixture records its source provenance. The Engine test checks all eight bits
and actual next-step telemetry across 29,400 source scans; input cables alone
are driven one host frame early for the native cable-delay contract. It also
covers adoption across control changes. Raw kernel, native host scheduling and
application/device claims remain separate.
