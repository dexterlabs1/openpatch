# VIZ reference check

```sh
python3 scripts/viz_reference/run.py --sdk /path/to/Rack-SDK
```

Requires Python 3, curl, a C++17 compiler, Rust, and a separately obtained Rack
SDK. The default SDK path is the local VCVRackMCP cache. On macOS, `xcrun`
locates standard library headers. Builds are temporary; the persistent output
is `docs/benchmarks/viz-reference.json` (override with `--output`). No GUI,
application, plugin host, or audio device is opened.

The C++ driver uses genuine SDK `Input`, `Light`, and `ClockDivider` primitives.
Its independent orchestration follows Fundamental
[`Viz.cpp` at 10dd0160c664770910e5584b7b00498cc48d9ddd](https://github.com/VCVRack/Fundamental/blob/10dd0160c664770910e5584b7b00498cc48d9ddd/src/Viz.cpp).
The script fetches and hashes that source for provenance, but does **not** compile
or vendor the upstream module. SDK header hashes record the reference tested.
Neither upstream source nor SDK is copied into this repository; consult the
separately installed SDK's `LICENSE.html` and `LICENSE-dist.html` for its terms.

The Rust probe compiles production `crates/openpatch-core/src/viz.rs` directly
via `#[path]`, with minimal host types. It compares raw signed brightness, before
UI display clamping: SDK `Light` retains negative values and values above one.
Its actual implementation does not square brightness despite its field comment.
The first update is frame 16, then every 16 frames. Count and lights remain held
between updates. Each lane receives voltage/10 and its negation. Falling values
use linear one-pole decay with coefficient `30 * sampleTime * 16`; rises update
immediately. Inactive lanes receive zero and positive residuals decay normally.

Six deterministic streams run at 44.1, 48, and 96 kHz, comparing every frame's
latched channel count and all 32 signed brightness components. They cover mixed
polarity, dynamic/disconnected channels, signed release, sample-boundary impulses,
channel retirement/reactivation, random polyphony, 10,000 V input, and frequent
state adoption both at and between update boundaries. The SDK reference remains
uninterrupted while the Rust model rebuilds and adopts its old state. The Rust
probe also statically checks that processing returns unit (no output frame).

The minimal host supplies the inactive-lane zeroing normally supplied by engine
bookkeeping. This does not test the real graph, cable delay, safety bounding,
public zero-output metadata, callback publication, UI, MCP, bypass, or sample-rate
changes. Separate integration checks must cover these paths. Test voltages are
finite and within the OpenPatch safety bound; unrestricted voltage parity is not
claimed. This is a headless primitive-and-scheduling comparison, not execution of
the complete upstream module.

Decimal traces are decoded back to float32. Channel counts must match exactly;
brightness tolerance is `3e-6 + 1e-6 * abs(reference)`, allowing floating-point
association differences in the release coefficient. The report records source,
SDK and input hashes, toolchain, maximum error, and first failures. The script
exits nonzero if any comparison fails.
