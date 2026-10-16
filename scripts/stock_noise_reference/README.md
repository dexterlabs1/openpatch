# Stock Noise independent reference — draft before review and freeze

This seven-file source-only declaration was authored without examining the new
production implementation or running a compiler, SDK probe, DSP, tests, GUI or
audio device. Root must independently review and freeze it before first
compiler, SDK or DSP execution. The fixture generator and `--fixtures-only` validator are input-only
preflight tools; their future output is not evidence of DSP parity.

## Source and scope

The oracle uses the **actual** `Noise` class prefix from Fundamental commit
`10dd0160c664770910e5584b7b00498cc48d9ddd`, `src/Noise.cpp`, SHA256
`ba011632c44c2bec645d24d640f6c9cdaa0c7768506a4398285cb31d17983aa7`.
Extraction stops before `struct NoiseWidget :`. The only transformations are
the plugin header replacement, three controlled draw call substitutions (one
normal, pink uniform, black uniform), and value-initialization of the otherwise
indeterminate `freqBuffer[2048]` scratch in one line. The resulting source hash
is `85a18572d472a017a197229a7b4dd919db1af1e51e8d93c06f79841ee4b1d44e`.
`run_sdk.py` checks exact site counts and hashes before compiling; the complete
transformed source is retained in each attempt. All seven colors are mandatory;
no six-color success or gray omission is a passing result.

The gray erratum is deliberate and narrow. Rack's ordered real FFT consumes
packed indices 0–1023, while the Fundamental loop modifies 0–2047. The upper
half is read in the untouched class but not consumed by inverse FFT. Source
audit established the packed range; it did not establish binary behavior.
After the full oracle, the runner separately builds two initialized upper-half
patterns (all `+1.f`, all `-1.f` before FFT overwrites its packed range) and a
packed-range weighting loop variant. Their SHA256 pins are in `acceptance.json`.
It runs each on a retained 48 kHz first-gray-block subset and compares **all**
state/port rows to the zero-initialized baseline with the same predeclared gray
numeric gate. The variants are an audit of the unused scratch and meaningful
range, not additional source claims or a retroactive fix.

Pinned upstream source, Fundamental GPL license/plugin metadata and Rack 2.6.6
Engine/Module/system/license source live under ignored
`native-test-output/stock-noise/reference-research/`. Source hashes are checked
against fixed pins in `run_sdk.py`. The genuine Rack SDK is
`/Users/dbrewer/Library/Caches/VCVRackMCP/build/ck-sdk/vendor/Rack-SDK`;
complete `include` and `dep/include` hashes, `libRack.dylib` hash and existing
2.6.6 SDK attestation are locked before execution. Source transforms, command,
toolchain, binary, diagnostics, entire input/trace and every failed attempt are
retained. This proves behavior for the pinned source plus declared erratum and
installed SDK, not equivalence to a distributed official plugin binary.

Primary sources: [Fundamental Noise](https://raw.githubusercontent.com/VCVRack/Fundamental/10dd0160c664770910e5584b7b00498cc48d9ddd/src/Noise.cpp),
[Fundamental license](https://raw.githubusercontent.com/VCVRack/Fundamental/10dd0160c664770910e5584b7b00498cc48d9ddd/LICENSE-GPLv3.txt),
[Rack FFT wrapper](https://raw.githubusercontent.com/VCVRack/Rack/v2.6.6/include/dsp/fft.hpp),
[Rack Engine](https://raw.githubusercontent.com/VCVRack/Rack/v2.6.6/src/engine/Engine.cpp),
[Rack Module](https://raw.githubusercontent.com/VCVRack/Rack/v2.6.6/src/engine/Module.cpp).
The pinned PFFFT dependency/source provenance and packed-range reasoning are
documented in the ignored `stock-noise-gray-audit.md` preparation memo.

## Protocol and matrix

Each JSONL input case is exactly `id,sample_rate,initial,events`, where initial
is `{output_mask:0..127,normal_profile,normal_bits[8192],uniform_bits[8192]}`.
Bit order is white, pink, red, violet, blue, gray, black. Each set bit creates
an actual module output→sink input cable named
`out0`…`out6`. The module and sink are actual Rack Engine modules. Every event,
including constructor and control/cable events with no sample, yields exactly
one snapshot. `step` processes one Engine frame unless it specifies
`frames:1..2048`; then it processes that many callbacks and snapshots the
final one. Every interior draw and state advancement remains counted. Other events are
`connect`/`disconnect` of actual cables, `reset`, `bypass`, same-incarnation
`adopt` and `recreate`. `adopt` has no Noise controls and keeps its instance,
cables, state and draw cursors; the production side must use compatible graph
adoption. `recreate` removes incident cables, constructs a fresh Noise module
and keeps the Engine frame and actual external controlled draw cursor. The
snapshot's `module_incarnation` starts at zero, advances only on recreation and
stays fixed on compatible adoption. Reset follows
actual Engine/Module behavior; bypass follows actual Engine `processBypass`.

At each of 44.1, 48 and 96 kHz, 128 fresh mask cases process at least two
frames. All 64 gray-consumed masks snapshot frames 1, 1023, 1024 and 1025,
using batched steps to avoid needless intermediate buffer serialization. Six
purposeful histories per rate add all-connected and gray-only 2050-frame
runs, gray disconnect for 17 frames mid-block then reconnect, 260 Voss frames,
isolated family connection transitions, and multiple-consumer/bypass/reset/
adopt/recreate transitions. A seventh focused post-FFT history at every rate
routes nonzero gray into sinks already carrying white/black, removes gray
consumers in both orders, observes retained mixed values at the next
process-time tap, clears the last sink cable, and exercises bypass, reset,
adoption and recreation with established gray history. Six gray diagnostic
profiles at every rate add DC,
Nyquist, k1 low-band sine, k128 in-band sine, impulse and alternating signed
zero; k511 high-out-of-band sine is added at 96 kHz. There are **424 cases**,
17,689 events and 18,113 declared snapshots.
Every declared step/event stays in the trace; boundaries include 1023, 1024,
1025, 2048 and pink 255/256. No declared startup, retired or no-sample row is
discarded.

Controlled random values are exact binary32 bit tables embedded in each input
case. The broad profile keeps its first 16 literal anchor values and derives
the remaining 8176 with labeled SHA-256 counters mapped to bounded finite
f32 values. Diagnostic profiles replace the normal table with explicit
mathematical signals. The fixture derivation does not reuse Rack or production
random code. Both tapes repeat only after 8192 draws, using separate normal
and **one shared uniform** cursor.
Pink's Voss draws precede black's draw, so the black sample depends on how many
Voss registers change. The final-callback ordered normal/pink/black draw log
and both cumulative counts, including batched callbacks, are compared exactly.
This conditional protocol tests
source behavior under controlled draws. Rack's uncontrolled global PRNG stream,
seed, distribution, and interaction with other modules are outside the claim.

Snapshots include real Engine rate/time, bypass state, seven complete raw
16-slot output ports, post-cable sink input ports and process-time `sink_seen`
consumer taps with channel counts, actual cable
registry/counts/consumer mask/incarnation, draw log/cursors, red IIR histories, lastWhite,
lastPink, Voss frame/eight registers, gray frame/entire input and output buffers,
unused black divider clock/division, and actual common lights (empty). No
candidate raw port is written by this adapter. The validator independently
reconstructs physical topology, draw order/counts and freeze/reset/boundary
protocol, with exact black −5 V and white zero anchors. A float64 canonical
DFT of actual gray input checks five output positions for the first completed
gray block at every rate and in every diagnostic profile, including the source's half-bin weighting frequency,
zero DC/Nyquist and unnormalized inverse convention. It complements the
actual-source SDK oracle and cannot replace it.

Six non-gray outputs, all control/state/cable metadata, gray input and all
inactive port slots compare at exact binary32 bits, including signed zero and
FTZ effects. Only FFT-derived gray output buffer and gray-derived module/sink
lane-zero values use a
predeclared bound: `2.5e-4` raw filter units or `5e-4 V` exposed, respectively,
plus `3e-5 × max(abs(actual),abs(expected))`. The validator tracks output,
post-cable sink and one-frame-delayed process-time tap provenance separately.
Rack bypass clears connected output voltages and last-cable removal clears all
sink input slots to exact positive zero; those remain exact. A disconnected
module output retains its raw FFT-derived value, and a sink with remaining
cables retains its prior mixed value until the next Engine cable propagation.
Both SDK and production traces independently pass the full shape, topology,
draw, freeze, hand-anchor and float64 DFT checks before their paired gate.
Every FFT-derived numeric gray pair passes a finite near comparison even
when its two raw bit patterns match, so identical NaN/Inf values fail.
This is a predeclared bound for
single-precision 1024-point PFFFT (about ten butterfly stages each way),
1024 source weighting iterations with 512 inverse-consumed packed pairs,
float unit roundoff `2^-24`, and source gain up to about 16.6 near 80 Hz in
this bounded input domain; it is fixed **before** any result. Nonfinite gray
numbers fail. A failing result cannot lead to dropped fields, warmup exclusions
or widened limits; changing the declaration requires a superseding independent
review and preserves prior failed attempts.

## Root-only review and execution

Input-only preflight, after source review and before freezing:

```sh
python3 scripts/stock_noise_reference/fixtures.py --output INPUTS
python3 scripts/stock_noise_reference/verify.py --inputs INPUTS --fixtures-only
```

An external lock must have `reviewed_before_first_execution:true`; SHA256
`files` for exactly these seven files; `inputs_sha256`; exact successful
`fixture_validation`; `reference_source_sha256` for all seven primary evidence
files; `transformed_source_sha256`; all three `scratch_variant_sha256`; complete
`sdk_header_sha256`; `sdk_library_sha256`; and `sdk_attestation_sha256`. The
author cannot approve the lock. Runners refuse missing or changed provenance.

```sh
python3 scripts/stock_noise_reference/run_sdk.py --inputs INPUTS --lock LOCK
python3 scripts/stock_noise_reference/run_paired.py --probe PROBE --build-manifest MANIFEST --sdk-evidence SDK_ATTEMPT --lock LOCK
```

The production probe must be a separately built release binary with a manifest
binding command, toolchain, binary and complete Core source closure. Its real
Core Engine graph must configure the controlled stream before first draw and
emit the same schema. Paired validation compares every row and preserves all
artifacts and failures. The existing `colored_noise` module, general Rack
patch import, host/device timing and GUI/audio behavior have separate contracts.
