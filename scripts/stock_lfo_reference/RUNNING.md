# Reproduce the stock LFO comparison

The committed fixtures are already frozen. Verify them; do not recreate their
manifest in a fresh checkout:

```sh
python3 scripts/stock_lfo_reference/fixtures.py --check
```

Read [the frozen contract](README.md) for the numerical domain, all-frame
comparison, safety distinction and exact 76/109-field protocols. Neither the
fixture generator nor the validator implements an oscillator.

Fetch the exact reference and license into ignored evidence storage:

```sh
mkdir -p native-test-output/stock-lfo/reference
curl --fail --location https://raw.githubusercontent.com/VCVRack/Fundamental/10dd0160c664770910e5584b7b00498cc48d9ddd/src/LFO.cpp --output native-test-output/stock-lfo/reference/LFO.cpp
curl --fail --location https://raw.githubusercontent.com/VCVRack/Fundamental/10dd0160c664770910e5584b7b00498cc48d9ddd/LICENSE.md --output native-test-output/stock-lfo/reference/LICENSE.md
```

Build production Rust separately, then run with a separately installed genuine
Rack SDK 2.6.6. `--sdk` can select its directory; the default is the established
local SDK cache. The SDK is needed only for this comparison, not to build or run
OpenPatch.

```sh
cargo build --locked --release -p openpatch-core --example stock_lfo_reference_probe
python3 scripts/stock_lfo_reference/run.py \
  --sdk /path/to/Rack-SDK \
  --rust-probe target/release/examples/stock_lfo_reference_probe \
  --rust-toolchain "$(rustc --version)" \
  --rust-build-command 'cargo build --locked --release -p openpatch-core --example stock_lfo_reference_probe'
```

The runner never invokes Cargo or Rust compilation. The caller records the
actual external command/toolchain. It rejects a probe older than the source
closure and checks source hashes again after execution. It compiles the actual
SDK class prefix, replacing only its plugin include with `rack.hpp` and the Rack
namespace. The adapter asserts SDK version, nearest-even FPU rounding and fresh
state, and records FPCR on ARM64. Compiler flags disable contraction and omit
fast-math. The report records compiler, machine, headers, library and binaries.

Each run keeps a new `native-test-output/stock-lfo/qualification/attempt-NNN`
directory, preserving earlier attempts. Numeric failures still inspect every
frame and produce a failed report and artifact manifest; compilation or process
errors retain their attempt files and stdout/stderr but may abort before that
final manifest. Inputs and raw traces are gzip-compressed without changing their
bytes. The report defaults to `docs/benchmarks/stock-lfo-reference.json`.

Reference frames compare the actual pinned class with actual production
`StockLfoProbe`. Safety frames execute Rust only and do not contribute to oracle
claims. Direct stepping does not qualify graph handoff, callback deadlines,
window input, devices, persistence, native rendering or hardware; those need
their separate tests. The recorded result applies to its actual toolchain and
platform, not every possible SIMD/FPU combination.
