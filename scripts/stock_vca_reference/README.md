# Stock VCA qualification

The runner compiles the genuine pinned Fundamental `VCA_1` class against an
installed Rack SDK in temporary storage. Only `plugin.hpp` is replaced by
`rack.hpp`; UI classes are omitted. No upstream source or Rack asset is bundled.
The production Rust module is compiled unchanged by standalone `rustc`.

Run from the repository root using an existing compatible serde_json dependency:

```sh
python3 scripts/stock_vca_reference/run.py \
  --serde-rlib target/debug/deps/libserde_json-8c08f8c1dd2ce75e.rlib
```

Choose the existing serde_json rlib for the current Rust toolchain. `--sdk` accepts
another installed SDK. The runner never invokes Cargo, opens a GUI, or accesses
an audio device. Its default report is `docs/benchmarks/stock-vca-reference.json`.

`acceptance.json` fixes the pointwise gate and cases. `fixtures.json` froze all
input hashes and generator hash before the first comparison; the runner rejects
changes. `--freeze-fixtures` was run once before comparison and refuses to replace
an existing declaration. No tolerance, waveform alignment, or warmup exception
is fitted to the result.

Each frame compares actual SDK output width when connected and actual
`lastChannels`/visible `lastGains`. An unused SDK output retains zero connection
metadata while its actual `lastGains` and `lastChannels` still update; the host
always publishes active-width output frames. Rack cable adapters zero padding
before the genuine class reads it; poisoned padding enters production Rust to
verify its explicit missing-channel boundary. Source inactive gain storage is
not visible, while host inactive gains and output storage must clear.

Independent fresh traces at CV 0, 5, and 10 V prove both response modes agree at
endpoints and differ at partial CV. This is finite digital compatibility coverage,
not a hardware, GUI, audio-device, latency, or performance-superiority claim.
Public Engine and callback tests separately qualify integration and lifecycle.
