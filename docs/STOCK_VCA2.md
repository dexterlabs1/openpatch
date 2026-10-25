# Stock VCA-2

Development addition, not installed. `stock_vca2` recreates the hidden legacy
VCV Fundamental **VCA-2** (model slug `VCA`), so patches that still use it
import and sound as they did. It is a separate kind from [Stock VCA](STOCK_VCA.md)
(VCA-1), which remains the recommended single amplifier.

## Play it

Choose **Stock VCA-2** under **Mixing**, or open
`examples/legacy-vca2-unity-duet.json`: a clocked envelope shapes a low saw
through channel 1's exponential CV, a slow LFO swells a higher voice through
channel 2's linear CV, and [Stock Unity](STOCK_UNITY.md) sums both.

Each channel has a **LEVEL** knob (0–100%, default 100%; click the readout to
type a value), an **EXP** and a **LIN** CV input, **IN** and **OUT**. The two
channels are independent.

- **Voices follow IN.** OUT carries as many voices as the IN cable. With IN
  unpatched, OUT is one voice at 0 V. CV cables never add voices (unlike
  VCA-1, whose width is the widest input).
- **LIN** multiplies the gain by CV/10, limited to 0–1: 5 V gives half gain.
- **EXP** multiplies it by (50^(CV/10) − 1)/49, also from 0 V to 10 V: silent
  at 0 V, full at 10 V and only about 12% at 5 V. With both patched, LIN applies
  first, then EXP.
- A **one-voice CV** controls every voice. A **polyphonic CV** controls each
  voice separately; voices it does not carry read 0 V and fall silent.
- Unpatched CV inputs leave LEVEL in charge. Bypass passes IN 1 to OUT 1 and
  IN 2 to OUT 2 unchanged. The source has no lights or meters, so the panel
  shows none.

## Source contract

Reference: [pinned `src/VCA.cpp`](https://github.com/VCVRack/Fundamental/blob/10dd0160c664770910e5584b7b00498cc48d9ddd/src/VCA.cpp)
(byte-identical at Fundamental 2.6.4, `e0ade9a`), listed `"hidden": true` in the
pinned `plugin.json`.

Inputs `exp1, lin1, in1, exp2, lin2, in2` and outputs `out1, out2` keep the
source enum order. Parameters `level1`, `level2`: 0..1, default 1. Per channel
and voice the f32 operation order is the source's: `v = in * level`; if LIN is
patched, `v *= clamp(lin/10, 0, 1)`; if EXP is patched, `v *= rescale(50^cv, 1,
50, 0, 1)` with `cv = clamp(exp/10, 0, 1)`. A port counts as polyphonic when it
has more than one channel. The two code paths differ as in the source and are
both reproduced:

- one-channel CV: `std::fmax(std::fmin(x, 1), 0)` and libm `powf(50, cv)`;
- polyphonic CV: SSE `min(max(x, 0), 1)` (second operand wins ties, so −0
  becomes +0) and `simd::pow(50, cv) = exp_ps(cv * logf(50))`, Rack's
  sse_mathfun polynomial, ported lane for lane in
  `crates/openpatch-core/src/stock_vca2/kernel.rs`. At 10 V the polynomial
  gives a gain one ulp above 1 (1.0000001); `powf` gives exactly 1.

Every add, subtract, multiply and divide applies Rack's flush-to-zero contract
(`Engine::stepBlock` calls `system::resetFpuFlags`) through the shared
`native_float` helpers, so subnormal inputs, levels and products flush exactly
as in Rack. Host policies outside the source: nonfinite input becomes 0 V and
finite input is bounded to ±1,000,000 V before the source arithmetic; lanes
above a cable's channel count read as zero whatever the host buffer holds.

VCV import maps `Fundamental/VCA/2.6.4` to `stock_vca2`, parameters 0/1 to
`level1`/`level2`. A saved level outside 0..1 leaves the module unresolved
rather than altering it.

## Reference qualification

[Runner, sources and cases](../scripts/fundamental_legacy_reference/README.md):
the genuine class compiled against Rack SDK 2.6.6 with `-O2 -ffp-contract=off
-fno-fast-math` and the engine's FPU flags, compared token for token with the
production module driven through `StockVca2Probe`. Attempt `attempt-003`
(2026-10-24, macOS ARM): **11 VCA-2 cases, 59,122 frames, 1,123,895 compared
values (channel counts and every active output lane), 0 mismatches, no
exclusions.** Cases cover unpatched defaults, the level knob including a
subnormal level, every IN width 0–16 against LIN and EXP widths alone and
together, 20,000-frame mono and polyphonic exponential sweeps through both
clamp limits, signed-zero, subnormal, boundary and large bit patterns,
audio-rate signals and CV on 16 voices, and voice-count changes. Counterfactual:
replacing the polyphonic `exp_ps` with `powf` mismatches 270,186 values of the
polyphonic sweep, so the comparison resolves the difference.

Bounds: the defined-IEEE source domain on macOS ARM. The shipped Fundamental
binary is built with `-O3 -funsafe-math-optimizations`, whose contraction and
reassociation are compiler choices, not source behaviour, and are not claimed.
Inputs are finite and within the host bound. The runner does not exercise
cable delay, bypass, graph edits or callbacks; those have host tests below.

## Host validation

- Core unit tests (`stock_vca2.rs`): parameter validation, the `logf(50)`
  constant against the reference build, width rule, gain law, short polyphonic
  CV muting, nonfinite input.
- Engine tests (`tests/fundamental_legacy.rs`): voices through the engine,
  bypass routes, catalog contract, the duet example renders both channels.
- `engine_v2_contract`: VCA-2 and Unity at five sample rates through control
  edits, adoption, bypass and initialize with no heap activity in the callback.
- MCP/history (`control/fundamental_legacy_tests.rs`, `scripts/test_mcp.py`),
  VCV import (`vcv_import_tests.rs`), shared panel and library tests, and
  [normal and dim captures](../design/reviews/fundamental-legacy-v1/README.md).

Not established: native window input, audio-device playback, analog hardware
behaviour or complete Rack host parity.
