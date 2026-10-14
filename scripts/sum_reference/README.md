# SUM reference check

Run from the repository root:

```sh
python3 scripts/sum_reference/run.py --sdk /path/to/Rack-SDK
```

Requires Python 3, curl, a C++17 compiler, Rust, and a separately obtained Rack SDK.
The default SDK location is the local VCVRackMCP cache. On macOS, the script
uses `xcrun` to locate standard library headers. Builds use a temporary directory;
the only persistent output is `docs/benchmarks/sum-reference.json` (override with
`--output`). No Rack application, GUI, plugin host, or audio device is opened.

The C++ driver uses actual SDK `Input::getVoltageSum`, `VuMeter2`, and
`ClockDivider` implementations. Its orchestration follows Fundamental
[`Sum.cpp` at 10dd0160c664770910e5584b7b00498cc48d9ddd](https://github.com/VCVRack/Fundamental/blob/10dd0160c664770910e5584b7b00498cc48d9ddd/src/Sum.cpp).
The script downloads and hashes that source for provenance; it does **not**
compile or vendor the upstream module. SDK header hashes record the reference
actually tested. The SDK and Fundamental source retain their upstream licenses;
neither is copied into this repository. Consult the separately installed SDK's
`LICENSE.html` and `LICENSE-dist.html` for its terms.

The Rust probe compiles the actual production `crates/openpatch-core/src/sum.rs`
via `#[path]`. Small host adapters supply the required types, parameter lookup,
and finite-value safety bound. Thus this check validates the production Sum DSP
and its state adoption, but does not validate the public Engine, parameter
validation, cable timing, callback publication, MCP, or native UI. Those require
separate integration checks. All vectors stay inside the OpenPatch safety bound;
the report does not claim unrestricted headroom parity with Rack.

Six deterministic streams run at three sample rates, comparing every frame's
output, latched input channel count, and six brightness values. Decimal traces
are decoded back to float32 before comparison. The report records input hashes,
production source hash, toolchain, tolerances, maximum error, and first failures.
The script exits nonzero on a comparison failure.
