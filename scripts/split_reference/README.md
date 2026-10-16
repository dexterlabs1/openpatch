# Split bounded reference qualification

Two separately frozen phases compare the actual pinned Fundamental `Split`
class against original production Rust. The runner never invokes Cargo or a
Rust compiler, even for a version query. Root orchestration builds the example
first, records its actual toolchain, and then runs this SDK comparison serially.
No GUI, audio device, plugin host, speakers or native app is opened.

The retained GPL upstream source is
`native-test-output/split/reference/Split.cpp`, pinned at Fundamental
`10dd0160c664770910e5584b7b00498cc48d9ddd`, SHA-256
`a9529ec742c8740bc1cdb181ae6d58de5604ef0ee89a81279cf4cf5e2a15a24e`.
Its GPL license must be retained alongside it as `LICENSE.md`. Source and
license remain ignored evidence, not vendored implementation. The compiled
prefix ends immediately before `SplitChannelDisplay`; only the single
`plugin.hpp` include becomes `rack.hpp` plus the Rack namespace declaration.
The genuine linked SDK must report version 2.6.6.

## Normal public Engine phase

Each input row is 17 decimal fields: width 0 through 16, then sixteen finite f32
voltages bounded to ±16 V. Inactive protocol lanes are positive zero; both
adapters reject poisoned inactive protocol values. There are 355 fresh rows per
rate: 17 widths × 16 single-hot locations (272), 17 each of all-zero,
alternating signed-zero and alternating ±16 V patterns (51), and 32 deterministic
xorshift32 mixed rows with seed 1779033703. Total: 1,065 rows and 17,040 output
voltage comparisons at 44.1, 48 and 96 kHz.

The C++ adapter writes the input width and supplied storage into the genuine SDK
input and calls the actual class. All sixteen outputs are marked connected mono
before process, reproducing Rack Engine's connected-output bookkeeping; it does
not derive or replace output voltages. It verifies initial `lastChannels == 0`.
The Rust example creates a public `Patch` with a `poly_constant` source and one
Split, resolves sixteen channel-zero output taps, verifies the actual initial
count is zero, and processes two frames to settle one cable. It reads production
`input_channels` and actual tap values/active flags, without a DSP mirror or new
public test API. Output has 33 fields: observed input count and sixteen pairs of
SDK output count/Rust tap-active flag and actual voltage.

Every count must be exact; every output count/active flag must be one. Every
output voltage must match the other implementation and the independent fixture
in exact f32 bits, including ±0. All sixteen outputs are compared even when the
input width is zero or short. Rust's tap-active flag proves channel zero is
active; Core tests independently verify that each output frame is mono. This
phase does not inject poisoned private PortFrame storage.

## Raw storage diagnostic phase

A second `--raw` protocol uses a decimal width and sixteen eight-digit u32 hex
bit strings. Every width 0 through 16 is crossed with four rotations (0, 1, 7,
15) of a fixed sixteen-value bit pattern: ±0, ±16, ±f32::MAX, ±Infinity, two
signed quiet-NaN payloads, normal/subnormal extrema and ±0.5. There are 68 fresh
rows per rate, 204 rows and 3,264 output comparisons in total. Inactive input
slots are deliberately poisoned. The frozen manifest records its input hashes
separately from normal fixtures.

The C++ `--raw` adapter calls the same actual class, with raw input bits copied
without arithmetic. The Rust `--raw` adapter includes the exact private
production `split.rs` by path and re-exports the public host types at the example
crate root. It invokes that production module, not a duplicated algorithm or
public testing hook. Each row starts fresh and checks initial count zero.
Output retains the same count/pair shape but voltages are raw hex bits, avoiding
NaN parsing/equality and signed-zero ambiguity. All sixteen copied values must
match both implementations and independent fixture bits exactly, including
NaN payloads and inactive slots.

This raw phase deliberately bypasses normal producer and cable bookkeeping.
It establishes the module's diagnostic raw-copy boundary, not a claim that a
legal Rack or OpenPatch cable delivers poisoned padding or preserves active
nonfinite input. Graph sanitization and normal zero-padding policies are
unchanged. Lifecycle, bypass, initialization, adoption and allocation require
separate Core and host tests; fresh-row comparisons do not qualify them.

## Freeze and run

The committed fixture declaration was frozen before the first comparison. In a
fresh checkout, verify it without attempting to replace it:

```sh
python3 scripts/split_reference/run.py --check-fixtures
```

`--freeze-fixtures` is only for declaring a new suite with no existing manifest;
its exclusive create refuses to replace a frozen declaration.

Fixture freeze/check commands are pure Python: no build, network, SDK access or oracle. In a fresh checkout, fetch the exact source and its license into ignored storage:

```sh
mkdir -p native-test-output/split/reference
curl --fail --location https://raw.githubusercontent.com/VCVRack/Fundamental/10dd0160c664770910e5584b7b00498cc48d9ddd/src/Split.cpp --output native-test-output/split/reference/Split.cpp
curl --fail --location https://raw.githubusercontent.com/VCVRack/Fundamental/10dd0160c664770910e5584b7b00498cc48d9ddd/LICENSE.md --output native-test-output/split/reference/LICENSE.md
```

Normal qualification requires an externally built example and its actual toolchain
string. Root orchestration runs these steps serially:

```sh
cargo build -p openpatch-core --example split_reference_probe --release
python3 scripts/split_reference/run.py \
  --rust-probe target/release/examples/split_reference_probe \
  --rust-toolchain 'ACTUAL EXTERNALLY RECORDED RUST TOOLCHAIN' \
  --output docs/benchmarks/split-reference.json
```

The runner checks frozen acceptance/generator/input hashes before compilation,
checks source SHA, requires the retained license and existing Rust executable,
and rejects a binary older than its production closure. It compiles only C++.
It retains full upstream source, license, compiled prefix, both binaries,
complete SDK include trees/library, exact production source closure, drivers,
commands, inputs, stdout/stderr traces and an independent artifact hash manifest
under ignored `native-test-output/split/qualification`. The JSON report records
these hashes, platform/toolchain, per-phase counts and first failures. It checks
production closure freshness again after comparison. Any failed row fails the
run; there are no tolerances, exclusions, warmup alignment or fitted gates.
