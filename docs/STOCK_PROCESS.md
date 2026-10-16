# Stock Process

`stock_process` is an original Rust implementation of the digital behavior of
[Fundamental Process at the pinned revision](https://github.com/VCVRack/Fundamental/blob/10dd0160c664770910e5584b7b00498cc48d9ddd/src/Process.cpp).
It is a functional Rack reference, not a physical hardware model. The older
`process` utility keeps its independent behavior and saved-patch identity.

## Signal and controls

The inputs, in source order, are `slew_cv`, `signal`, and `gate`. The six
outputs are `sh1`, `sh2`, `track_hold`, `hold_track`, `slew`, and `glide`.
`signal` alone determines the number of processed voices, including zero.
An exactly mono CV or gate input broadcasts its channel-zero voltage to every
signal voice. Every other auxiliary width reads the same raw lane, even if that
lane is outside the auxiliary port's active channel count. This behavior depends
on the common host's raw port storage.

The durable `slew` control is log2(seconds per volt), with authored range
`log2(0.001)` through `log2(10)` and default `log2(0.001)`. Rack displays it
as milliseconds per volt. OpenPatch retains raw saved finite values outside
the authored range as a native extension; Rack's ordinary parameter JSON reload
clamps to that range. A value at or below the source minimum selects
infinite slew rate and ignores `slew_cv`. Otherwise the engine adds the raw
CV to the knob, evaluates Rack's ordered `exp2_taylor5((-pitch) + 30)` and
divides by `2^30` before multiplying by sample time. No additional voltage or
pitch clamp is part of this module.

The momentary gate control is runtime state. Press and release set Rack's
`GATE_PARAM` to 1 and 0; the comparison uses its raw value `> 0`. OpenPatch does
not save the momentary gate in projects or presets. Rack's base parameter JSON
can serialize it, so this persistence choice is not a Rack import promise.
The one actual light displays this button state
immediately on each callback, including when there are no signal voices. Gate
CV does not light it. Each voice turns on at gate CV `>= 2 V` or a pressed
button, and turns off at gate CV `<= 0.1 V` with the button released. Voltages
between those levels retain the previous voice state.

On a rising edge, the current signal becomes both the first sample and the
hold value, while the previous first sample moves to the second sample. On a
falling edge, the current signal becomes the hold value. `track_hold` emits
the hold value while high and the live signal while low; `hold_track` does the
reverse. Two independent filters slew the sample outputs. A third filter jumps
to the live input while high and slews while low. The glide filter jumps except
when the gate is high and its float32 `on_time` has become strictly greater
than 1 ms **after** the current sample's increment. It then slews toward the
input. All four filters retain separate histories.

Every active lane writes all six outputs, whether or not an output has a cable.
The host applies SDK `setChannels` semantics to each port afterward. A zero
width signal input skips every voice engine, including filter and `on_time`
updates; all outputs still receive `setChannels(0)`. Shrinking to fewer voices
also freezes retired engine histories, which resume if the width grows again.
Connected retired output slots clear under the host lifecycle; disconnected
outputs retain their raw writes and storage.

## Lifecycle and inspection

The diagnostic snapshot copies all sixteen engine records, including inactive
ones, the two source parameters, the manual light brightness, and sample rate
and sample time. The bounded in-place sample-rate event on a live graph with
exactly one `stock_process` and otherwise only `poly_constant` sources and
`mult` sinks updates sample time without clearing histories, raw ports, cable
frames, source controls, or the observer clock. The event validates the full
graph and requested rate before changing anything. A normal compile/adopt at a
different sample rate remains a cold graph rebuild for every module kind,
including Process.
Cold construction clears all engine fields. Reset restores the two parameter
defaults while preserving histories and the previous light until the next
callback. Compatible same-rate live graph adoption carries engine histories,
raw momentary gate, and light; the receiving graph's slew takes precedence.

No GPL source or panel artwork is vendored. The upstream source and Rack SDK
serve as behavioral references; this module, its controls, and its UI are
original OpenPatch work under the repository's license. Source-defined
arithmetic uses ordered float32 operations with the common Rack FTZ/DAZ shadow.
For the SDK's undefined out-of-range float-to-integer exponent conversion, the
Rust diagnostic path uses its defined saturating cast. This is a safety rule
outside the source-defined numerical domain, not a Rack parity claim.

## Use it in the rack

**File → New File → Stock Process voice** opens a connected example. A slow
bipolar sine is reduced to ±0.6 V, the clock drives Gate CV, and GLIDE controls
Stock VCO pitch. Scope X shows the original scaled source and Y shows glide.
Try moving the pitch cable between S/H 1, S/H 2, the two tracking outputs, SLEW
and GLIDE to hear their different responses.

The shared rack/browser face uses the same large rotary slew control, nine
ports and momentary gate button. Click the time value for exact ms/V entry:
0 selects instantaneous slew and ignores slew CV; positive entries must be
greater than 1 ms/V because the source's 1 ms/V minimum selects the
instantaneous branch. The one manual gate lamp follows the actual button,
not incoming Gate CV. Plug lights show actual port voltages separately.

MCP uses `set_parameter` with the authored raw log2(s/V) range and
`trigger_module` with `gate_press` / `gate_release` on a live backend. Actions
return queue receipts, not callback-application confirmations. Project and
preset loading retain finite raw slew values outside the authored editing
range as an explicit native policy. The held manual gate is runtime-only.

## Reference qualification

The independently reviewed declaration was frozen before either DSP run. An
optimized actual OpenPatch Engine and the genuine Rack 2.6.6 ARM Engine running
the pinned Process class match **all 13,599 complete snapshots across 3,444
cases** at 44.1, 48 and 96 kHz. The full traces have the same SHA256; no fields,
lanes, initial samples or snapshots were discarded. Coverage includes all 64
output-consumer masks, signal widths 0–16, independent auxiliary widths, exact
Schmitt thresholds, cumulative float32 timing, raw retained port slots, topology,
bypass, Initialize, adoption, recreation and bounded in-place rate changes.

[Reference report](benchmarks/stock-process-host-reference.json) records the
frozen declaration, actual production binary and complete compressed traces,
inputs and build provenance. The [unchanged reference declaration](../scripts/stock_process_reference/README.md)
describes the finite numerical domain and reproducible protocol. Its original
source-only wording is retained as part of the pre-execution freeze.

Six safety tests cover all 28 separately frozen nonfinite/extreme descriptors.
Public nonfinite sources/parameters reject; finite extremes finish and recover
after a fresh Process incarnation and safe inputs. Raw values can become
nonfinite outside the source-defined Taylor exponent domain; these cases do
not establish SDK equivalence. Fifteen realtime tests pass, including the
Process first callback, all sixteen voices, actions, adoption, bypass and reset
with no heap allocations or deallocations.

The combined local checks pass: 2,142 distinct Rust tests across the workspace
and corrected callback run, including a final 1,025-test Desktop run; 114
Python/MCP checks; strict Clippy and formatting; and all seven prior frozen
Core reference gates. The [release record](benchmarks/stock-process-release-qualification.json)
binds the exact optimized 0.2.2 build 4 binaries and source closure. Static ARM
packaging verifies the signature, code sections, fonts, notices and all four
visualizers, including Ferrofluid. Platform CI and native render review are
required delivery gates, tracked in the pull request checks. This bounded source/host evidence is not a hardware calibration,
universal numerical proof or official plugin-binary equivalence claim.
