# Wavetable actual-class oracle

This is an original MIT test adapter and deterministic synthetic fixture generator.
The pinned GPL reference implementation is fetched or read from the ignored
`native-test-output/wavetable/reference/` directory and transformed **only in a
throwaway build directory**. No upstream DSP source or panel artwork is included
in this directory or copied into production code.

The numerical acceptance contract was frozen before any Rust comparison:

```
acceptance.json SHA256
4d3668cfe2e84f1b4ecf75945685d49474d3db70c4bbb12a40b0c4c046f605b0
```

Every visible voltage must satisfy `abs(actual-reference) <= 0.002 +
0.00001*abs(reference)` from frame zero, with no startup exclusion, alignment or
aggregate-metric substitution. This follows the repository's driven DSP budget;
2 mV is small relative to the nominal 10 V span and allows independent FFT/libm
rounding. Width, trigger masks, sync direction and other discrete observations
are exact. Phase observations allow 2e-6 cycles, table position 2e-6 waves, and
clock values `1e-6 + 1e-5*abs(reference)`. The JSON is checked against its frozen
hash by the runner. A future contract change must be explicitly justified and
must not be represented as qualification against this contract.

## Source and dependencies

Fundamental commit: `10dd0160c664770910e5584b7b00498cc48d9ddd`.
Files are fetched with curl using normal TLS verification from
`https://raw.githubusercontent.com/VCVRack/Fundamental/<commit>/src/`.

| File | SHA256 |
| --- | --- |
| WTVCO.cpp | `37723b2d01b950090400e65982681e8d8abded59191090b7aaa4d5ffbb37d337` |
| WTLFO.cpp | `20e9634eedcab363122217d9b09cb0dbafa7a9967e923fc1fbb4038004064077` |
| Wavetable.hpp | `bc45992469e938a229aad35d78d44f16ae6d0664addbc556dfcd46d9263bbbb5` |
| dr_wav.h | `d435cea2b6828840ba40d61e23247c41a335be1cefc2e5734a05e62fec1accb0` |

`dr_wav.c` is the same pinned file, containing only `DR_WAV_IMPLEMENTATION`
and its header include; its hash is calculated from those exact pinned bytes.
The header offers MIT No Attribution and Unlicense alternatives. It is compiled
only in the throwaway directory. Retained actual module virtual `onAdd` and
`onSave` methods reference the real load/save methods and therefore need these
symbols even though the oracle never calls those lifecycle methods.

Each actual module has only `plugin.hpp` replaced by `rack.hpp` and the Rack
namespace import, and its widget/registration suffix removed. Wavetable retains
its actual preparation, FFT, filtering, interpolation, JSON, load/save, and menu
methods unchanged numerically; `interpolatedAt()` uses `vector::at()` in the
throwaway build to reject final-buffer overruns before they can become reference
data. In-bounds cross-row reads at a phase rounded to exactly one are retained.
`WTDisplay` and its unused preview table are omitted.
The adapter calls the actual class `process()` and actual `setWaveLen()`.
`Rack-SDK/libRack.dylib` supplies the real SDK FFT/PFFFT and minBLEP support.
No independently reimplemented C++ DSP is used as the reference.

Default SDK location:
`~/Library/Caches/VCVRackMCP/build/ck-sdk/vendor/Rack-SDK`.
The runner hashes the linked SDK library and relevant headers in its report.
C is compiled with `cc -std=c99 -O2 -ffp-contract=off`; C++ with
`c++ -std=c++17 -O2 -ffp-contract=off -fno-fast-math`. On macOS the runner
resolves the SDK using `xcrun --show-sdk-path` only in the executable build mode.
`--mac-sdk` can provide that path explicitly. The generated
`compile-commands.json` records all actual argument arrays.

## Running

These commands never run Cargo, a plugin, GUI, audio device or speakers. The
SDK mode runs only the small test executable linked to Rack's library. Run
compiler commands serially with other repository work.

```sh
# Python-only deterministic fixture checks; no SDK or compiler execution.
python3 scripts/wavetable_reference/run.py --self-check

# Downloads/verifies and prepares temporary actual-class source; no compilation.
python3 scripts/wavetable_reference/run.py --prepare-only \
  --build-dir /tmp/openpatch-wavetable-oracle-preflight

# Compile and smoke-test an actual class. --frames marks this non-qualifying.
python3 scripts/wavetable_reference/run.py --sdk-only --kind vco \
  --case harmonic --rate 48000 --frames 32 \
  --build-dir /tmp/openpatch-wavetable-oracle-smoke

# Full SDK-only suite. It verifies two fresh instances have bit-identical traces.
python3 scripts/wavetable_reference/run.py --sdk-only \
  --build-dir /tmp/openpatch-wavetable-oracle-sdk

# Compare an already-built independent Rust adapter using the same protocol.
python3 scripts/wavetable_reference/run.py --sdk-only \
  --rust-probe /absolute/path/to/rust_probe \
  --build-dir /tmp/openpatch-wavetable-oracle-comparison \
  --output native-test-output/wavetable/comparison.json
```

There are 25 VCO and 26 LFO cases at 44.1, 48 and 96 kHz, 4096 frames per
case/rate. `fixtures.json` is the explicit coverage manifest. Python creates
harmonic, DC endpoint and saw banks from mathematical formulas, rounding every
sample to IEEE f32 before sending the **same bytes** to both implementations.
The default-bank case calls the actual class constructor's reset bank in C++;
Rust must use its production default. Inputs and raw output traces are retained
with hashes in the temporary build directory. SDK-only output makes no claim of
Rust numerical fidelity. A complete-suite qualification requires a Rust adapter,
all cases, all rates, full lengths, and every check passing. Public host API
checks listed below are also required separately.

## Binary adapter protocol, version OWT1

Both adapters read stdin, write binary stdout, report errors on stderr, exit
nonzero on failure and process exactly one stream per process. Integers are
little-endian u32; all floating values are little-endian IEEE f32. There is no
padding. The C++ adapter checks its native representation. This protocol is
intended for the Unix/macOS qualification runner.

Header, 32 bytes, Python format `<IIfIIIII`:

| Field | Meaning |
| --- | --- |
| magic | `0x3154574f`, bytes `OWT1` |
| kind | 0 VCO; 1 LFO |
| sample_rate | f32 Hz |
| frame_count | number of frames |
| flags | bit 0 enables observations; only 0 or 1 permitted |
| wave_length | samples per wave |
| sample_count | total concatenated samples |
| default_bank | 1 keeps production default and requires count 0; 0 installs explicit bank |

An explicit bank has exactly `sample_count` f32 values after the header. Default
bank has no sample bytes. All fixture VCO bank lengths are valid powers of two
at least 32, except explicitly guarded invalid-bank tests. The custom bank
installer assigns samples then calls actual `setWaveLen(0)` and
`setWaveLen(new_length)` to force preparation even when lengths match. Bank
publication occurs before processing the next frame.

Each frame contains:

1. u32 event code and its optional payload.
2. All 7 VCO parameters, or all 6 LFO parameters, in actual source enum order.
3. Four input ports in actual source enum order. Each port is a u32 channel count
   followed by exactly 16 f32 storage values. Width is 0..16. Both adapters must
   ignore the poison values beyond width and clear their inactive storage to 0.

VCO parameter order: removed mode, soft, frequency in semitones, position,
FM depth, position CV depth, linear. LFO parameter order: offset, invert,
frequency in log2 Hz, position, FM depth, position CV depth. Four ports:
FM, sync/reset, position, pitch/clock.

| Event | Action before parameters/ports and process |
| --- | --- |
| 0 | none |
| 1 | actual source `onReset()` semantics, including reset to default bank |
| 2 | replace bank; payload `u32 wave_length, u32 sample_count, f32 samples[count]` |
| 3 | mark loading true; retain all bank memory |
| 4 | mark loading false |
| 5 | install empty bank, wave length zero |
| 6 | reset only bank to default |
| 7 | actual `setWaveLen()`; payload u32 new length; rebuild before next process |
| 8 | Rust reconstructs/adopts its current complete state, then applies parameters; C++ mutable module does nothing |

Actual source `onReset()` does **not** clear every oscillator field: VCO keeps
phase/minBLEP/history while resetting directions and bank; LFO clears phase and
clock timer/frequency while keeping its trigger objects. Event 1 specifically
qualifies those actual method semantics. Fresh-instance, host reset, state
adoption and full lifecycle policies need explicit real-host tests as well.
The production parameter API is intentionally not assumed by this scaffold;
its test-only Rust adapter must map actual parameter units and adoption calls.

Output per frame starts with u32 visible channel count then exactly 16 f32
voltages, with inactive output slots normalized to zero. Actual Rack output is
marked connected before process so source `setChannels()` is observed. When
flags=1, append observations in this exact order:

- 16 f32 phases, group-major then lane-major.
- 16 f32 VCO directions, or zeros for LFO.
- 16 f32 VCO previous sync values, or zeros for LFO.
- f32 LFO clock frequency and elapsed clock timer, or zeros for VCO.
- f32 `lastPos` in wave units.
- 4 u32 LFO reset trigger masks, one bit per SIMD lane, or zeros for VCO.
- u32 LFO clock trigger state (`LOW=0, HIGH=1, UNINITIALIZED=2`), or zero for VCO.
- u32 loading, u32 wave length, u32 total sample count.

The full output is 304 bytes/frame, Python format `<I67f8I`. With observations
omitted it is 68 bytes/frame. The runner currently always requests observations.
Comparisons include hidden phase/trigger lanes to expose whole-group freeze and
partial-group advancement; visible widths stay unrounded. Hard-sync minBLEP
state adoption is exercised by subsequent voltage tails, not by exporting the
private buffer representation.

## Defined source boundary and OpenPatch policy

**Do not send zero or negative VCO frequencies to this source oracle.** The
source computes `log2(Nyquist/frequency)` and then converts the result to
`size_t` in `getWave()`. Negative frequency creates NaN; exact zero creates
infinity. Those floating-to-integer conversions have undefined behavior. The
positive linear FM fixture keeps every SIMD lane positive, including hidden
lanes. Through-zero fidelity must not be claimed from an arbitrary compiler's
result on the undefined path.

Rust must separately have safe functional tests for negative frequency,
negative-to-positive crossings and exact zero: finite outputs, forward/backward
phase wrapping, phase hold at zero, bandlimit chosen using absolute frequency,
continuous state across sign changes, and usable sync at zero. These are
required OpenPatch behaviors, independently of reference fidelity.

The upstream Wave points context-menu setter changes `waveLen` without calling
`interpolate()`, leaving stale prepared memory. This scaffold uses the actual
`setWaveLen()` method to rebuild in a defined fixture. OpenPatch policy is to
prepare and publish the replacement bank atomically. Stale-bank reads are
excluded, and no claim is made to reproduce that UI bug.

Separate production tests must cover serialization, bank save/load fidelity,
validated parameters and nonfinite host inputs, actual graph state adoption,
whole-node reset/bypass behavior, ownership and atomic publication of tables.
A test-only loading event does not qualify concurrent publication safety.
