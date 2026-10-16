# Reproducing the Stock Quantizer comparison

The independent five-file declaration is checked before either processor runs.
Its source-safe finite matrix is separate from Rust-only nonfinite/extreme and
raw-bypass safety fixtures. Do not regenerate inputs or adjust numerical gates
after observing DSP comparisons.

Prepare genuine Rack SDK 2.6.6 separately and put the pinned `Quantizer.cpp` and
its license in `native-test-output/stock-quantizer/reference/`. The runner uses
the unmodified processor class prefix before `QuantizerButton`, replacing only
`plugin.hpp` with `rack.hpp` and its namespace declaration. It archives that
prefix, full source/license, all SDK headers/library, compiler command, adapter,
executables, complete Core source closure and deterministic compressed traces.

Build the actual production probe externally:

```sh
cargo build --locked --release -p openpatch-core --example stock_quantizer_reference_probe
rustc --version
python3 scripts/stock_quantizer_reference/run.py \
  --rust-probe target/release/examples/stock_quantizer_reference_probe \
  --rust-toolchain 'THE EXACT rustc --version OUTPUT'
```

The runner does not invoke Cargo or rustc. It records the build command and
reported compiler, checks the probe is newer than its production source closure,
and preserves failed attempts as well as successful ones. C++ uses optimization
with contraction disabled; SDK version, rounding mode and ARM FPCR are checked.
No native window or audio device is initialized.

The adapter calls actual processing, bypass, reset, rotation and JSON methods.
It never computes a second quantizer. Connected/disconnected output context is
passed to the actual production Rust processor, rather than rewriting output
counts. Uninitialized SDK playing-note storage is not read; the first processing
call establishes validity. Inactive SDK output storage is excluded, while Rust
public frame padding is checked independently. A successful direct-class result
is not complete native, engine-consumer, importer, hardware or Rack-host parity.
