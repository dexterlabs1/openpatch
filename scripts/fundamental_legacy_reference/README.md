# Fundamental VCA-2 and Unity reference

Prepared 2026-10-24 for Stock VCA-2 (`stock_vca2`) and Stock Unity (`stock_unity`). This folder compiles the genuine hidden legacy `VCA` and `Unity` classes from the pinned VCV Fundamental source against the Rack SDK and compares the production Rust modules against them, bit for bit.

## Sources

`source_manifest.json` pins `src/VCA.cpp`, `src/Unity.cpp`, `plugin.json` (where both models are `"hidden": true`) and `LICENSE.md` at Fundamental `10dd0160c664770910e5584b7b00498cc48d9ddd` by SHA-256, plus the Rack SDK 2.6.6 library and the headers the classes use (`simd/sse_mathfun.h`, `simd/functions.hpp`, `dsp/vumeter.hpp`, `dsp/digital.hpp`, `math.hpp`, `engine/Port.hpp` and others). Both module files are byte-identical at Fundamental 2.6.4, the version VCV patch import accepts. `NOTICE.md` records licences and boundaries.

## Method

`run.py` fetches and hash-checks the sources, extracts each module class up to its widget (only `plugin.hpp` replaced by `<rack.hpp>`), and compiles `reference_probe.cpp` with `c++ -std=c++17 -O2 -ffp-contract=off -fno-fast-math` against `libRack.dylib`. This is the defined-IEEE source domain on macOS ARM: the shipped Fundamental binary is built by the Rack plugin toolchain with `-O3 -funsafe-math-optimizations`, whose contraction and reassociation choices are not part of the source and are not claimed. The probe calls the genuine `rack::system::resetFpuFlags()` first, as `Engine::stepBlock` does, so every operation runs with nearest rounding and flush-to-zero. Ports are loaded as a Rack cable leaves them: the channel count set and lanes at and above it zero.

The production side is `crates/openpatch-core/examples/fundamental_legacy_reference_probe.rs`, built with `cargo build --release --locked`. It drives the real Core modules (`StockVca2Probe`, `StockUnityProbe`) through the same token protocol; VCA-2 input lanes above the channel count are poisoned with NaN to prove the module reads them as zero. Every float crosses the protocol as an exact 32-bit pattern and is compared as text, so signed zeros and flushed subnormals count.

Protocol, one line per event: VCA-2 `F level1 level2` then six ports (`exp1 lin1 in1 exp2 lin2 in2`) of channel count and 16 lanes, answered by each output's channel count and active lanes. Unity `F average1 average2` then twelve ports of channel count and first-channel voltage, answered by MIX 1, INV 1, MIX 2, INV 2, the ten light brightnesses, both meter states, the light divider clock and the merge flag; `M 0|1` sets "Merge channels 1 & 2"; `R` resets (parameter defaults from the genuine `ParamQuantity` objects, then the genuine `Unity::onReset()`, because `Module::onReset(ResetEvent)` needs a running app); `D json` loads module data into a fresh module and prints what it saves back.

## Cases

`make_cases.py` writes `cases.json` (schema 1): 22 declared cases, 35 runs once expanded over sample rates (Unity runs at 44.1, 48 and 96 kHz where timing matters; VCA-2 is stateless). They cover unpatched defaults; the level knob including a subnormal level; every IN width 0–16 against linear and exponential CV widths, mono and polyphonic, alone and together; dense mono (powf) and polyphonic (SSE exp approximation) exponential sweeps through both clamp limits; signed zero, subnormal, tiny, boundary and large CV and signal bit patterns; audio-rate signals and CV; voice-count changes; every Unity connection mask in sum and average with polyphonic cables; merge with every switch combination; meter release over four seconds into the flush-to-zero range; slow ramps through every light band; reset; saved data; and noise with changing modes. Each case states its purpose.

## Reproduction

From the repository root (requires Python 3, curl, clang++, cargo and the Rack SDK at `~/Library/Caches/VCVRackMCP/build/ck-sdk/vendor/Rack-SDK`, or `--sdk PATH`):

```sh
python3 scripts/fundamental_legacy_reference/make_cases.py      # only to regenerate cases.json
python3 scripts/fundamental_legacy_reference/run.py --validate-only
python3 scripts/fundamental_legacy_reference/run.py
```

Each run creates `native-test-output/fundamental-legacy-reference/attempt-NNN` with the harness snapshot, fetched sources and extracted headers, the compiler and cargo commands and versions, the reference probe binary, per-case input and output hashes, the first mismatches if any, an artifact SHA-256 manifest and `result.json`. Acceptance is identical output tokens on every line of every case. Results are recorded in `docs/STOCK_VCA2.md` and `docs/STOCK_UNITY.md`.
