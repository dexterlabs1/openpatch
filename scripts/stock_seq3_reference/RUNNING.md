# Reproducing the Stock SEQ3 comparison

The five independent declaration files (`acceptance.json`, `fixtures.py`,
`fixtures.json`, `verify.py`, `README.md`) are frozen. Do not regenerate fixtures
or adjust gates after observing results. The runner checks the complete manifest
before executing either processor. The exact gate covers every frame and all
90 fields, with independent protocol and state invariants.

Prepare the genuine Rack SDK 2.6.6 separately. Put the unmodified pinned
`SEQ3.cpp` and `LICENSE.md` in
`native-test-output/stock-seq3/reference/`. The pinned source URL and SHA-256 are
in the declaration. The runner compiles only the actual module prefix before
`SEQ3Widget`; its sole prefix change replaces `plugin.hpp` with `rack.hpp` and
`using namespace rack`. The adapter observes the real public class fields and
calls its actual process, reset, rotation and JSON-loading methods. It contains
no second sequencer or detector implementation.

Build the production Rust probe outside the runner:

```sh
cargo build --locked --release -p openpatch-core --example stock_seq3_reference_probe
rustc --version
```

Pass that exact reported compiler version, rather than an assumed toolchain:

```sh
python3 scripts/stock_seq3_reference/run.py \
  --rust-probe target/release/examples/stock_seq3_reference_probe \
  --rust-toolchain 'THE EXACT rustc --version OUTPUT'
```

`--sdk` can select another SDK directory. The runner never invokes Cargo or
rustc. It records the external build command/toolchain and requires the probe to
be newer than its production-source closure. C++ uses optimization with floating
point contraction disabled; SDK version, rounding mode and ARM FPCR are recorded.
There is no native window or audio-device initialization.

Inputs are streamed through the immutable row encoder and checked against the
frozen hash, avoiding a million-line temporary allocation. Validators stream
all rows and retain only a bounded first-failure list. Low-tempo cases include
full actual periods; there is no startup discard or phase alignment. Each rate
has a separate Rust-only safety phase, with no SDK equivalence claim.

Every attempt gets a new evidence directory. SDK headers/library, source/license,
compiled prefix, adapters, complete production-source closure, toolchain/build
commands, binaries, raw input/trace hashes and deterministic gzip traces are
retained. Failed/partial attempts are retained too. `result.json` and the
published benchmark report explicitly say whether the run passed. Host bypass,
derived length and held-state bookkeeping are declared adapter policies; this
is not complete Rack engine/native/device/import qualification.
