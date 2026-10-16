# Random actual-class qualification

This original MIT adapter compiles the genuine Fundamental `Random` class from
commit `10dd0160c664770910e5584b7b00498cc48d9ddd` in owned build storage against the
installed Rack SDK 2.6.6. It verifies the exact upstream source hash before use.
No upstream DSP source or Rack assets are tracked here. The temporary source
changes only the `plugin.hpp` include, removes the widget/registration suffix,
and redirects the two `random::uniform()` calls to a controlled f32 draw stream.
The algorithm and actual Rack DSP, ports, and lights remain in the source class.

The production Rust example calls `RandomProbe::step_with_uniform`, which uses
the production processing function with the same explicit draw stream. This
qualifies deterministic behavior conditional on those draws. It makes **no
claim of sequence parity with Rack's uncontrolled global PRNG**.

The root agent builds and executes probes. The scaffold author only performed
Python syntax and fixture checks. Build the production probe after all source
edits, then run the entire frozen suite from the repository root:

```sh
cargo build -p openpatch-core --example random_reference_probe
python3 scripts/random_reference/run.py \
  --rust-probe target/debug/examples/random_reference_probe \
  --build-dir native-test-output/random/oracle-final
```

The runner itself never invokes Cargo, opens a GUI, or accesses an audio device.
It compiles the C++ adapter and uses a retained byte-identical copy of the
separately built Rust executable. `--source` and `--sdk` can point to existing local files. The
default source is the ignored pinned download at
`native-test-output/random/reference/Random.cpp`. The default report is
`docs/benchmarks/random-reference.json`. No frame-count override exists.

`--build-dir` must identify a new or empty directory owned by this run; a
nonempty directory or symlink is rejected. Put it under ignored
`native-test-output/` so genuine upstream source and generated evidence remain
untracked. The directory retains the exact upstream source, transformed source
prefix, original C++ adapter, both executable probes, and `compile-argv.json`.
The report records the complete actual compiler argument list, including the
platform SDK flags, Rack include/library paths and linked library flags.

Every case has three unique files under `traces/`, named
`RATE-CASE.input.txt.gz`, `RATE-CASE.sdk.txt.gz`, and `RATE-CASE.rust.txt.gz`.
The input and both raw traces are compared before compression. Gzip headers
have fixed timestamps and no embedded filenames. The report preserves each
raw SHA256, saved relative path, gzip SHA256, and raw/compressed byte counts.
The runner rejects duplicate names and uses exclusive file creation throughout;
it verifies exactly 75 inputs, 75 SDK traces, and 75 Rust traces plus their
compressed hashes before writing `evidence-index.json`. That index is hashed
in the report. No large generated trace is tracked in the repository.

If `--build-dir` is omitted, the same artifacts use temporary storage which is
removed at exit. The report explicitly declares whether evidence was retained
and records its build-directory location; hashes alone do not imply retention.

`acceptance.json` fixes tolerances and required cases before any Rust comparison.
`fixtures.json` freezes its hash, the generator hash, and every complete input
hash. `--freeze-fixtures` refuses to overwrite an existing manifest and exits
without compiling or comparing. `--check-fixtures` performs a pure Python check.
Neither tolerances nor per-case gates can be adjusted by CLI options. A passing
report requires all 75 traces at 44.1, 48, and 96 kHz. Cases have 4,096 frames;
the source-default trace has `sample_rate + 512` frames so its default 2 Hz
clock fires at least twice. The complete suite contains 484,548 frames.

The protocol is newline-delimited, whitespace-separated text with f32 values
rounded to nine significant decimal digits; no NumPy dependency is required.
Each 115-field input row contains:

1. Event: `0` ordinary, `1` Initialize/source defaults with runtime retained,
   `2` fresh identity/runtime, or `3` explicit host parameter adoption.
2. Ten source-order parameters: rate, shape, unipolar, unused legacy mode,
   probability, spread, rate CV, shape CV, probability CV, spread CV.
3. Six ports in source order: rate, shape, trigger, external, probability, spread.
   Each port has its channel count followed by all 16 voltage storage slots.
4. Two controlled uniform slots. Only actually consumed draws count; unused
   slots never advance the native production PRNG.

Each 129-field output row contains, in order:

- Five channel counts and then five arrays of 16 output voltage slots, in
  source order: stepped, linear, smooth, exponential, trigger.
- All 16 `lastVoltage` and all 16 `nextVoltage` values.
- Phase, clock frequency, internal clock phase, external timer, pulse remaining,
  Schmitt state (`LOW=0`, `HIGH=1`, `UNINITIALIZED=2`).
- Five actual source light values: rate, shape, probability, spread, unipolar.
- Uniform draws consumed in this frame.

All five outputs are connected in the SDK adapter, preserving real output
channel metadata. The trigger output is always mono. Actual source
`Output::setChannels()` clears output padding on shrink; inactive held arrays
remain untouched. The runner compares every output/storage lane, both complete
held arrays, phases, measured clock frequency, external timer, pulse state,
Schmitt state, light acquisition and decay, per-frame draw counts, and exact raw
and accepted clock events. It independently checks inactive held bits and the
source light recurrence. Startup high and intermediate inputs test the SDK's
three-state Schmitt initialization before either threshold is crossed.

Source input slots beyond channel count are zeroed by Rack cable bookkeeping,
including raw slot zero of disconnected ports. The source adapter implements
that host boundary. Production receives poisoned padding directly, proving its
equivalent missing-channel behavior. CV and trigger ports read only lane zero;
external voltage alone determines the four interpolation outputs' width.

Initialize restores parameters and retains runtime; the source class has no
custom `onReset()`. A separate fresh-identity fixture constructs a fresh runtime.
Adoption uses the production constructor plus `adopt`, preserving all held,
clock, trigger, pulse, and light state while applying new parameters.

All fixtures are finite. Internal computed rate stays in `[-120,120]`; the
runner rejects arbitrary rates that can reach undefined Rack scalar
`exp2Floor` integer conversion/shift paths. Shape zero uses defined IEEE float
division and is included. Nonfinite host policy and public Engine graph,
callback, bypass and persistence behavior require separate integration tests.

Reports include exact source/prefix hashes, every SDK header and library hash,
adapter/generator/fixture/acceptance hashes, complete local Rust source closure,
both probe binary hashes, per-trace hashes, per-case errors, event counts and
the complete frozen acceptance declaration. Retained runs also record every
compressed evidence path/hash and an evidence index. Qualification is finite digital
coverage for the reported compiler/SDK/platform combination.
