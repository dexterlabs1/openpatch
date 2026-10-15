# SHASR reference check

Run from the repository root:

```sh
python3 scripts/shift_register_reference/run.py --sdk /path/to/Rack-SDK
```

Requires Python 3, curl, a C++17 compiler, Rust/Cargo, the workspace's existing
locked dependencies, and a separately obtained Rack SDK with `libRack.dylib`
(macOS) or `libRack.so` (Linux). The default SDK location is the local VCVRackMCP
cache. The script builds the existing core dependency offline to locate its
`serde_json` artifact; it adds no Rust dependencies. C++ and probe builds use a
temporary directory. The sole persistent result is
`docs/benchmarks/shift-register-reference.json` (override with `--output`).

This executes the **actual upstream SHASR class**, not a driver that restates its
sample-and-hold logic. The script fetches and hashes
[`SHASR.cpp` at Fundamental commit 10dd0160c664770910e5584b7b00498cc48d9ddd](https://github.com/VCVRack/Fundamental/blob/10dd0160c664770910e5584b7b00498cc48d9ddd/src/SHASR.cpp),
keeps its prefix before `SHASRWidget`, and changes only its `plugin.hpp` include
to `rack.hpp` plus the Rack namespace. It compiles that prefix in the temporary
directory and links the separately installed SDK's Rack library. No Rack
application, GUI, plugin host, audio device, widget construction, or playback
is started. The temporary upstream source and executable are removed afterward.
No upstream code or panel assets are vendored into OpenPatch.

The original driver sets input channel bookkeeping, keeps RND off, and supplies
manual button levels. A low initial frame arms the actual SDK BooleanTriggers
while leaving every disconnected CV Schmitt detector uninitialized. Each button
has at most one edge per sample. The production Rust source
`crates/openpatch-core/src/shift_register.rs` is compiled directly with `#[path]`
and minimal original host adapters. These adapters provide types, parameter
lookup, and input/output storage; they do not implement shift-register DSP.

Nine deterministic streams run at 44.1, 48, and 96 kHz. Every frame compares all
eight values after decoding decimal traces back to float32, with zero numeric
tolerance. Cases exercise old-hold shifts, patched-input overrides, independent
and split clocks, initial high/intermediate trigger levels, disconnect/reconnect
history, simultaneous PUSH/CLEAR/CV, held manual buttons, and mono channel-zero
sampling from 0/1/2/8/16-channel inputs. The report records input, production
source, driver, transformed-prefix, SDK-header and Rack-library hashes, toolchain,
maximum error and first failures. Any mismatch exits nonzero.

This is deterministic **RND-off electrical behavior coverage**. It does not claim
matching random algorithms or validate OpenPatch's PRNG/ranges/seed, multiple
manual edges within one sample, public Engine routing or cable delays, host
publication, bypass, initialization, state adoption, persistence, MCP, native
input, or audio-device behavior. Inputs are finite and within OpenPatch's safety
bound. Those contracts retain their separate tests.

The harness drivers are original MIT code. The downloaded Fundamental source
retains its upstream [GPL-3.0 license](https://github.com/VCVRack/Fundamental/blob/10dd0160c664770910e5584b7b00498cc48d9ddd/LICENSE.md);
the separately installed Rack SDK retains its own terms in `LICENSE.html` and
`LICENSE-dist.html`. These references do not change the license of upstream
source or authorize copying it into the product.
