# Plug-light detector reference qualification

This harness compares OpenPatch's production detector and seven-sample observer clock with the **actual exported `rack::engine::Module::doProcess` in a genuine Rack SDK library**. The C++ adapter supplies port fixtures and no-op module callbacks; it contains no translated light detector. Rust includes `crates/openpatch-core/src/port_lights.rs` unchanged, with only the crate's two maximum-count constants supplied by the adapter.

Run from the repository root:

```sh
python3 scripts/port_lights_reference/run.py --sdk /path/to/Rack-SDK
```

The runner invokes `c++` and `rustc` directly. It does not invoke Cargo, launch Rack/OpenPatch, construct an audio engine/device, open a GUI, or access audio/MIDI devices. Builds live in an automatically removed temporary directory. The default SDK path is the local macOS ARM cache used during qualification. On macOS the compiler uses the `xcrun` SDK headers. Source verification uses `/usr/bin/curl` with ordinary TLS verification.

`acceptance.json` fixes 18 cases, 4,096 samples each, at 44,100/48,000/96,000 Hz. The acceptance rule is identical for every raw color component:

```text
abs(Rust − SDK) <= 2e-6 + 2e-6 × abs(SDK)
```

`fixtures.json` freezes the generator, acceptance and all 54 input hashes **before the first comparison**. The runner refuses changed inputs or thresholds. `--freeze-fixtures` is a one-time initialization path and refuses to overwrite the manifest; it performs neither compilation nor comparison. Do not regenerate it to make a failed comparison pass.

Every sample compares both endpoints' signed, unclamped RGB state, including frames between acquisitions. Current SDK port channel counts and frame numbers are checked exactly against the supplied metadata; detector-acquired channel state between ticks is not equated with current fixture width. SDK normal/bypass callback counts prove `doProcess` traversed the requested paths. The fixture includes positive/negative/zero mono, reversal, decay, overrange, audio-rate bipolar values, two-voice [3,4], sixteen sparse/full/varying/silent voices, width transitions, poisoned finite inactive lanes, cadence impulses, and distinct input/output signals.

Source provenance is pinned to Rack `8ea470f343058ade6f75f16d503a1eb330e1700c`. The installed SDK is **2.6.6**, whose official source tag resolves to `061ccf63c1758599396ac1bb10d47345d9d34076`; it must not be described as a binary built from the audit commit. The runner verifies byte-identical `Port_step`/`doProcess` blocks between those source commits, verifies the seven-sample divider, verifies local Port/Light headers against the audit commit, and checks the library's runtime `APP_VERSION`. Output records library, source, header, driver, fixture, production-source and probe-binary SHA256 values along with compiler/platform information. The relevant detector source block SHA256 is `7c69a4d9a5ebf41bb83e1832fac49663c12a4ed78d9da6e46b27909ad07c63c7`.

The first recorded macOS ARM run passed **54 cases, 221,184 frames and 1,327,104 component comparisons**, with maximum absolute error **4.76837158203125e-7**. The report is [port-lights-reference.json](../../docs/benchmarks/port-lights-reference.json). The production source hash in that report identifies the exact qualified code; later changes require another run against the same frozen fixtures.

This is finite detector/cadence qualification. Public Engine routing, block partitioning, graph adoption, consumer masks, realtime allocations/retirement, telemetry publication and UI painting require their own integration tests. Rack's zero channels means disconnected; OpenPatch's physically connected zero-channel case and display-only nonfinite/overflow handling are intentionally separate core-test contracts. No acoustic, device-latency, hardware, native-pointer or cross-platform equivalence is claimed.

The adapters and generator are original MIT-licensed code. Rack's linked SDK/source is separately licensed; see its [pinned license](https://github.com/VCVRack/Rack/blob/8ea470f343058ade6f75f16d503a1eb330e1700c/LICENSE.md). No Rack graphics or source implementation is copied into this harness.
