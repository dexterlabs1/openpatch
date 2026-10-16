# Reproduce the stock VCO comparison

The committed declaration is already frozen. Verify it without recreating it:

```sh
python3 scripts/stock_vco_reference/fixtures.py --check
```

Read [the frozen numerical contract](README.md). Keep its four acceptance files
unchanged when reproducing a result. It includes a documented pre-comparison
addition of minimum-frequency cycles and drained tails. The numerical gates
were not fitted to comparison output. The original validator did contain an
incorrect real-number phase bound. [Erratum1](phase_endpoint_erratum.json)
corrects only that independent bound to the actual float32 interval[0,1]; exact
cross-implementation phase equality and all voltage/state/kernel tolerances are
unchanged. The default runner explicitly uses the versioned validator and
identifies the erratum in each result. The original five files and the original
failed result remain immutable.

Retained original traces can be completely revalidated without compiling or
executing DSP:

```sh
python3 scripts/stock_vco_reference/revalidate_phase_endpoint.py \
  --evidence native-test-output/stock-vco/qualification/attempt-002
```

That operation first verifies every artifact, current production closure and
uncompressed input/trace hash, then checks every original frame/checkpoint and
kernel. It retains a separate result under a new revalidation directory. A pass
under the erratum is never presented as a pass against the original invariant.

Fetch the pinned source and license into ignored evidence storage, then build
the actual production probe separately:

```sh
mkdir -p native-test-output/stock-vco/reference
curl --fail --location https://raw.githubusercontent.com/VCVRack/Fundamental/10dd0160c664770910e5584b7b00498cc48d9ddd/src/VCO.cpp --output native-test-output/stock-vco/reference/VCO.cpp
curl --fail --location https://raw.githubusercontent.com/VCVRack/Fundamental/10dd0160c664770910e5584b7b00498cc48d9ddd/LICENSE.md --output native-test-output/stock-vco/reference/LICENSE.md
cargo build --locked --release -p openpatch-core --example stock_vco_reference_probe
python3 scripts/stock_vco_reference/run.py \
  --sdk /path/to/Rack-SDK \
  --rust-probe target/release/examples/stock_vco_reference_probe \
  --rust-toolchain "$(rustc --version)" \
  --rust-build-command 'cargo build --locked --release -p openpatch-core --example stock_vco_reference_probe'
```

Use a genuine Rack SDK 2.6.6. It is a comparison dependency, not an app build
or runtime dependency. The adapter compiles the actual VCO class prefix with
only its plugin header replaced by `rack.hpp` and the Rack namespace. It calls
the source sample-rate hook and asserts fresh runtime state, nearest-even FPU
rounding and SDK version. The compiler disables contraction and fast-math is
not enabled. ARM64 FPCR is recorded. Kernel coefficients and runtime snapshots
are read from the actual processors; adapters and validators contain no
oscillator or kernel implementation.

The runner never invokes Cargo or Rust compilation. The caller records the
external command/toolchain; probe freshness and source hashes are checked,
not independently attested as a reproducible build. Each new attempt keeps
sources, licenses, complete production closure, SDK headers/library, binaries,
commands, FPU/toolchain records, full inputs and raw traces. Inputs and traces
are gzip-compressed losslessly after validation. Earlier attempts remain.
Compiler or process failures also produce a failed result and artifact manifest;
a numerical failure still visits every remaining frame and rate.

Compact output state is compared every frame. Full logical next-32-sample
correction tails are compared at declared checkpoints; dead physical ring
storage is excluded. Prepared kernels have a separate gate. Inactive and
unconsumed SDK output voltage storage is excluded while all counts, phase,
detectors, cursors and lamps remain checked. Rust padding must be positive zero.
Bypass is an explicit OpenPatch host-policy adapter and does not establish
Rack Engine bypass equivalence. Host transport, callback allocation/deadlines,
project state and native input/device behavior require their separate tests.

The frozen fixture tag `all_masks` is overbroad: it tests masks
**0, 1, 2, 4, 5, 8, 10 and 15**, covering each individual waveform on/off and
selected combinations. It does not test every one of the 16 combinations or
256 pairwise transitions. Qualification reports retain that limitation.
Separate spectral diagnostics inspect actual retained tone traces; numerical
agreement alone does not establish universal alias rejection or hardware accuracy.

The separate spectral diagnostic runs only on retained actual traces:

```sh
python3 scripts/stock_vco_reference/spectral.py \
  native-test-output/stock-vco/qualification/attempt-002 \
  docs/benchmarks/stock-vco-spectral.json \
  --numerical-report docs/benchmarks/stock-vco-reference.json
```

Its context preserves the original failed declaration result and verifies the
optional corrected report against all input/trace/production hashes before
recording qualification under the erratum. It applies no spectral acceptance
threshold or universal alias claim.
