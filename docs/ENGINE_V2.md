# Engine version 2 contract

This specifies the engine foundation milestone. Implementation and acceptance
tests must agree with this contract; it does not establish complete VCV Rack
module, project-file or plugin compatibility. Version 1 remains supported.

## Files and public API

The existing `Patch`, `Node` and `Cable` types remain the wire model. `Node`
still contains a stable ID, kind and numeric parameter map. The additions are:

| Field | Meaning | Missing-field default |
|---|---|---|
| `Patch.version` | Engine semantics selector | `1` |
| `Patch.output` | Selected source node, or `null` for silence | `null` |
| `Patch.output_port` | Selected source port | `"out"` |
| `Cable.from` / `Cable.output` | Source node and port | Output defaults to `"out"` |
| `Cable.to` / `Cable.input` | Destination node and port | No implicit input |
| `Cable.id` | Persistent identity, unique within the patch's cables | Absent in older files; assigned on session/project load |

`Patch::default()` creates version 1. `Patch::new_v2()` creates version 2.
Loading or saving a version-1 file must not silently upgrade its timing.
Changing a patch to version 2 is a behavioral migration: every cable gains one
sample of delay. Port names and directions are validated before compilation;
unknown versions, ports and parameters are errors. Version 2 accepts multiple
cables at one input, including identical routes with separate cable identities;
an output may fan out to multiple inputs. Version 1 retains one cable per input.
See [stacked cables](STACKED_CABLES.md) for native gestures and MCP operations.

Version 1 accepts only the existing mono kinds and their implicit `out` source
port. It rejects the version-2-only polyphonic utilities, non-`out` source
selection and cycles. Its original signal units and render behavior remain
unchanged. Version 2 accepts named outputs and feedback cycles.

`PortFrame { channels: u8, values: [f32; 16] }` represents one output sample.
The active range is 0–16 channels. Values outside that range are zero and must
never leak old voice samples. `Engine::process_poly_block` writes consecutive
frames into caller-owned storage. `Engine::process_block(&mut [f32])` remains a
mono adapter: channel 0, or zero when there are no channels. It does not sum
voices. Use `poly_sum` for a deliberate mono mix. Selecting a source output
reads its current frame without adding an extra imaginary cable.

`process_audio_block(&mut [[f32; 2]])` exposes explicit L/R only when the selected
sink is `audio_stereo.out`; other selections duplicate mono. `output_channels()`
reports this layout. `handle_midi(MidiEvent)` delivers validated events to all
MIDI → CV modules without allocation. Interleaved offline rendering and timed
traces are described in [MIDI and audio](MIDI_AUDIO.md).

## Sample timing and feedback

In version 2, a cable delivers the source's completed previous-sample frame,
including its channel count. All modules read the old cable bank; after all
modules run, current outputs are published for the next sample. Evaluation
order of modules cannot change the result. A cable through a utility adds another sample
of delay even if the utility is mathematically an identity.

Stacked inputs sum those delayed frames in saved cable order, without averaging.
Their channel count is the maximum contributing width. A mono source broadcasts
to that width; shorter polyphonic sources contribute zero above their active
range. An all-zero-channel stack remains unpatched for normaling. Source NaN/Inf
values become zero before addition, including the single-cable case. Addition
uses `f32`; the completed input sum is not clipped or sanitized, although a
downstream module may apply its documented arithmetic bound. Cable order can
affect floating-point rounding; node order and callback partitioning cannot.

Initial and newly connected cables contain a zero-channel frame, so downstream
inputs initially use their unpatched behavior. They do not start as one-channel
zero signals. Self-loops and multi-node loops follow the same rule as every
other cable. Callback length never determines feedback delay. Module-internal
oversampling and filter delays remain additional, documented latency.

Runtime processing uses preallocated frames, routing indices and DSP states.
Compilation resolves IDs and allocates storage on the control thread. Ordinary
polyphonic modules reserve up to 16 independent voice states but execute only
their active voices. No allocation, deallocation, lock, JSON parsing or graph
search belongs in the sample loop.

## Channels, normalization and voice state

Each module declares its voice-count policy. A primary input usually determines
the active voices, with at least one voice for ordinary modules so an unpatched
oscillator, normaled utility or envelope can execute its defined behavior.
Sources without a polyphonic input produce one voice unless their explicit
configuration says otherwise. Split, Merge and Sum have special rules below.

| Ordinary module | Voice-count input |
|---|---|
| `oscillator`, `lfo` | `fm` |
| `analog_vco` | `pitch` |
| `gain`, `lowpass`, `output`, `ladder_vcf`, `analog_vca`, `audio_out`, `attenuverter` | `signal` |
| `adsr` | `gate` |
| `sample_hold` | `trigger` |
| `quantizer` | `pitch` |
| `mixer`, `crossfade` | Maximum count of `a` and `b` |
| `constant`, `gate`, `white_noise` | One voice |

For a secondary input, one channel broadcasts to every active voice. Multiple
channels are matched by index, with zero for missing indices. Zero channels
means unpatched, invoking the port's declared normalization. Do not treat a
short polyphonic input as unpatched for its missing voices. Existing defaults
remain: `gain.cv` is 1, `analog_vca.cv` is 10 V, and `attenuverter.signal` is
10 V; other inputs default to zero.

When a voice becomes inactive, retire its state; reactivation starts from the
module's configured initial state. This prevents a removed envelope, held
voltage or oscillator voice from unexpectedly resuming old memory. Graph
replacement preserves active compatible voice state by stable node ID and
kind, without overwriting the new graph's parameters. New kinds/IDs start
fresh. Cable history survives only matching cable identity and unchanged
source/destination node-and-port endpoints with compatible kinds, port contracts
and sample rate. Copy both
values and channel count; rewired/new edges start empty. State adoption does
not modify the old engine used by the host's transition crossfade.

The desktop additionally tracks node and cable incarnations across control
revisions. Its filtered adoption path leaves recreated nodes and their incident
edge history fresh, and resets recreated cables independently of their source
modules, even when remove/readd or undo/redo revisions coalesce before the
callback. Same-ID compatible edits continue normally. Direct core callers can
still compile idless older patches without mutation; unique idless routes retain
endpoint-based adoption across node/cable reorder. See
[rack editing and runtime identity](RACK_EDITING.md).

## Foundation utilities

These are original functional utilities, not complete reproductions of VCV
panels or configuration menus. They require version 2.

| Kind | Ports and parameters | Behavior |
|---|---|---|
| `poly_constant` | `out`; integer `channels` 0–16 (default 3); `v1`…`v16` | Configured voltage per active channel; channels may explicitly be zero. |
| `poly_split` | `signal`; `out1`…`out16` | Every output is mono; indices beyond the input count produce 0 V. |
| `poly_merge` | `in1`…`in16`; `out`; integer `channels` 0–16 | `channels=0` selects automatic highest occupied input index; 1–16 forces the count. Holes remain zero; each input contributes channel 0. No connected signals in automatic mode produces zero channels. |
| `poly_viz` | `signal`; no outputs | Input-only 16-channel polarity display; sampled signed smoothing and latched channel count every 16 frames. [State and telemetry contract](POLYPHONIC_VIZ.md). |
| `scope` | `x`, `y`, `trigger`; outputs `x`, `y` | Independent raw 0–16-channel passthrough with 256 min/max bins per axis/lane. Literal seconds timebase, external-trigger normaling, bypass freeze and allocation-free borrowed snapshots; [contract](RACK_SCOPE.md). |
| `poly_sum` | `signal`; `out` | One-channel sum of all active input channels with `level` 0–1 (default unity); empty input produces mono zero. No automatic averaging. [Meter and state contract](POLYPHONIC_SUM.md). |

Merge's zero setting means automatic, not forced zero. Use a zero-channel
`poly_constant` when explicit virtual unpatching is required. This difference
must remain visible in catalog/help text.

## Saved bypass

`Node.bypassed` defaults to false and is omitted from serialized false values.
Both patch versions support it without changing their cable timing. Compiled
routes copy designated input frames directly; no-route and zero-channel routes
emit mono zero. Ordinary DSP state freezes. Compatible transitions preserve
incoming history and clear the source outputs/outgoing history. MIDI consumes
events and ages pulses while its outputs remain silent. See the [full contract](VOLTAGE_UTILITIES.md).

## Acceptance checks

- Parse and render a literal legacy JSON fixture without the new fields;
  preserve its zero-delay output and reject version-2-only features in v1.
- Verify each cable delays both voltage and count exactly one sample; test
  zero, one and sixteen channels, sparse Merge inputs and Split zero padding.
- Verify a self-loop and two-node recurrence against explicit expected samples,
  including reordered nodes/cables and irregular callback lengths.
- Check unpatched normalization, secondary mono broadcast and short-poly zero
  fill separately. Test voice shrink/regrowth for fresh retired-state behavior.
- Preserve compatible state and unchanged edge frames during a graph swap;
  clear rewired edges even when their source node was already producing audio.
- Check allocation/deallocation in rendering and state adoption; benchmark
  both one-voice and sixteen-voice graphs before making performance claims.

Independent regression tests belong in
`crates/openpatch-core/tests/engine_v2_contract.rs`. Native audio still requires
its separate device tests; a polyphonic engine alone does not add multichannel
hardware input/output, MIDI allocation or complete stock-module behavior.

The timing and voice-policy baseline follows the primary VCV
[voltage standards](https://vcvrack.com/manual/VoltageStandards),
[polyphony contract](https://vcvrack.com/manual/Polyphony), and
[Split/Merge/Sum descriptions](https://vcvrack.com/Free), checked 2026-10-12.
