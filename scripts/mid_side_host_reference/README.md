# Genuine Rack MidSide host-boundary declaration

This independent adapter exercises **Rack 2.6.6 `Engine::stepBlock(1)`**, pinned
Fundamental `10dd0160c664770910e5584b7b00498cc48d9ddd/src/MidSide.cpp`, real
`Engine::addCable/removeCable/bypassModule/resetModule/setParamValue`, and SDK
Port bookkeeping. It does not emulate the Engine, call MidSide directly, or
inspect OpenPatch DSP. There is no audio driver, fallback thread, GUI, hidden
warmup, settling-only comparison, or discarded first sample. One Engine is
created for each case, with one processing thread, CPU metering disabled, and
48 kHz sample rate. Engine frame zero and each topology mutation are observed.

The five files in this directory are the pre-execution declaration. Root must
archive their SHA-256 hashes **before the first compiler/probe execution**.
Adapters may need separately recorded build-only fixes; fixture/anchor changes
require an explicit versioned correction retaining the original failure. No
observed output may be used to silently tune anchors or acceptance.

## Protocol

Input is one compact JSON **case object** per stdin line from `cases.json`'s
`cases` array, not the enclosing object. Each case has `id`, `sample_rate`, and
an ordered `events` array. Supported events:

- `source {port,values}`: replace one of six held source configurations (0–16
  active values); this does not mutate physical ports until the next sample.
- `connect {id,source,port,destination,input}`: actual Engine cable insertion;
  roles are `source`, `module`, `sink`. IDs are case-local stable names.
- `disconnect {id}`: actual Engine cable removal and deletion.
- `param {index,value}`: Engine immediate parameter assignment, without smoothing.
- `bypass {value}`: Engine bypass API, including redundant assignments.
- `reset`: Engine reset event, preserving source-defined runtime/port storage.
- `step`: exactly one genuine Engine sample.

One JSON snapshot is emitted after **every** event. Keys are `case`, `event`
(zero-based), `op`, `frame` (actual `Engine::getFrame()`), `bypassed`, `params`
(two f32 values), `source_config` (six active-value arrays), `inputs` (six
MidSide ports), `outputs` (four **raw private** MidSide ports), `source_outputs`
(six ports), `sink_inputs` (eight ports), `input_cables` (six actual physical
cable counts), and `output_cables` (four actual consumer counts). A port is
`{channels: integer, values: [16 numbers]}`. Raw output slots are never cleared
or normalized by the adapter. Cable counts come from the Engine's actual
registry (`getCableIds/getCable/hasCable`), not the fixture map. The Sink has
no DSP; its physical inputs are written only by the genuine Engine.

The Source uses SDK `setChannels` and writes declared finite held active slots,
zeroing its unused source output storage. A declared width zero while physically
connected becomes **mono zero** under SDK rules, not physical disconnection.
Physical last-disconnect separately reaches zero channels. All inactive
MidSide/Sink input slots must be zero. Module output tail storage is compared
raw, including disconnected encoder buffers used by decoder normaling.

The runner supplies `reference_module.hpp`: the exact pinned source prefix
before `struct MidSideWidget`, replacing only `#include "plugin.hpp"` with
`#include <rack.hpp>` and `using namespace rack;`. Keep its source hash and
transformation in the run record. Link against genuine SDK libRack; retain
compiler command/toolchain, library/header hashes and all raw input/output.
The adapter is C++17. No custom mock Engine or direct-process fallback is allowed.

## Hand-derived anchors and scope

74 cases / 890 event snapshots. Sixty-four fresh cases cover all sixteen
consumer masks at four exact encoder/decoder width pairs. Held L=6/R=2 produces
M=4 and S=2 at unity; widths (0,1), (1,0), (2,0.5) give explicit anchored results.
The first sample consumes the newly connected mono-zero input, while its final
cable phase receives the held source values. Every later sample is retained.

Ten additional cases cover second-consumer handover; stacked-input removal
(one versus last); stacked poly-to-mono retirement; 16→1→declared0→disconnected;
connected versus disconnected private encoder stale tails with a wider decoder
override; independent decoder normals; both bypass edges with cable changes
and no-op assignments; parameter-only reset; exact finite CV widths. Anchors
are selected boundary snapshots from source reading and simple hand arithmetic,
not a reimplementation of the DSP. In particular, removing one stacked cable
preserves the **old combined input** until the next cable phase; the first
module sample after removal must still consume it.

`python3 verify.py SDK.jsonl` validates every snapshot's identity, order, frame,
shape, finite storage, inactive input zeros, and every declared anchor. It
proves **SDK anchors only**. `--paired OPENPATCH.jsonl` additionally validates
and compares every complete snapshot, including all raw inactive output slots,
using exact IEEE-754 binary32 bits for voltages/parameters and exact discrete
fields. Nonzero exit means failure. Extra or missing events fail. Only this
paired mode can qualify OpenPatch host behavior; no direct-DSP result substitutes
for it. This is a bounded host-transition contract, not full Rack or hardware
parity, performance, scheduling stress, or nonfinite-domain qualification.
