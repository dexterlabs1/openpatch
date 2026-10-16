# Merge bounded reference qualification

This harness compiles the actual pinned Fundamental `Merge` class prefix against
the installed genuine Rack SDK (default installation: Rack SDK 2.6.6). The only
source replacement is `plugin.hpp` with `rack.hpp` and its namespace declaration;
the prefix ends before `MergeChannelDisplay`. Full upstream source and its GPL
license remain in ignored `native-test-output/merge/reference`, and the runner
retains them with the compiled prefix in ignored qualification evidence. No
upstream implementation or artwork is vendored into the repository.

The original C++ adapter reads 273 fields per row: OpenPatch's saved channel
selection, then sixteen input records containing width and sixteen voltages.
Saved `0` maps to Rack Auto (`-1`); saved `-1` maps to Rack forced zero (`0`).
Missing SDK input lanes are zeroed as Rack engine cables would provide. It emits
the actual SDK output count, actual `automaticChannels`, and sixteen raw output
voltages. It does not recompute the module's result.

The Rust Cargo example constructs sixteen `poly_constant` sources and one Merge
through the public `Patch`/`Engine` API for every row. It processes two frames to
settle the existing single-cable delay and reports actual Merge observation
metadata and its final output frame. There is no custom DSP mirror or probe API.
Poisoned inactive values in the input protocol are ignored by both adapters:
the Rust adapter zeroes inactive source parameters, which `poly_constant`
validates even when inactive. The public Engine constructs zero-padded frames;
this harness does not directly inject or test poisoned internal `PortFrame`
padding. Core unit tests cover that separate read boundary.

Freeze and check the acceptance and generated input hashes before any oracle run:

```sh
python3 scripts/merge_reference/run.py --check-fixtures
```

The committed `fixtures.json` is already frozen. When initially declaring a new
suite, `python3 scripts/merge_reference/run.py --freeze-fixtures` creates it and
refuses to overwrite an existing declaration. Check is pure Python: no SDK,
network, subprocess, compilation or comparison. After freeze, changing the
acceptance, fixture generator or fixture bytes fails validation. `run.py` is
separately hashed in evidence. Acceptance fixes 18 saved selections times 304
input patterns (272 single occupied-slot/width combinations and 32 deterministic
mixed patterns), at 44100, 48000 and 96000 Hz: 16,416 rows total. Active finite
voltages stay within ±16 V and include signed voltages and occupied zero volts.

In a fresh checkout, obtain the pinned reference source and license in ignored
storage. The runner checks the exact source digest before compiling it:

```sh
mkdir -p native-test-output/merge/reference
curl --fail --location https://raw.githubusercontent.com/VCVRack/Fundamental/10dd0160c664770910e5584b7b00498cc48d9ddd/src/Merge.cpp --output native-test-output/merge/reference/Merge.cpp
curl --fail --location https://raw.githubusercontent.com/VCVRack/Fundamental/10dd0160c664770910e5584b7b00498cc48d9ddd/LICENSE.md --output native-test-output/merge/reference/LICENSE.md
```

Build Rust separately, then execute the harness with a separately installed SDK:

```sh
cargo build -p openpatch-core --example merge_reference_probe --release
python3 scripts/merge_reference/run.py --rust-probe target/release/examples/merge_reference_probe
```

The runner never invokes Cargo. It requires SDK 2.6.6 at runtime, rejects a Rust probe older than its production source closure, and checks that production sources remain unchanged throughout qualification. No fixture row is skipped; the predeclared inactive SDK storage boundary remains outside the active-voltage target. If the root build command differs, supply its
exact text through `--rust-build-command`. `--sdk` selects another genuine SDK.
The runner compiles only the C++ adapter and records its exact argument vector.
It retains source closure and binary hashes, SDK include trees and linked
library, both executables, build output, gzip-compressed inputs and raw traces,
and an artifact hash manifest. The deterministic JSON report defaults to
`docs/benchmarks/merge-reference.json`; ignored evidence defaults to
`native-test-output/merge/qualification`.

Every row requires exact output and automatic counts, independently derived
from fixture widths and the saved selection. Every active voltage requires exact
f32 bits against both the actual SDK class and fixture first-lane values. Rust
inactive output values must be zero. Raw SDK inactive output lanes are retained
but excluded from voltage parity: the upstream class writes sixteen output lanes
even when its forced emitted count is shorter. A failed acceptance exits nonzero.

Each fixture is a fresh class and graph; this is bounded digital behavior
coverage. State adoption, initialization, bypass, callback partition and realtime
allocation have separate core host tests. This harness does not qualify UI,
artwork, audio devices, physical cable latency, hardware, or full module parity.
