# First application-host milestone

Planning checkpoint, 2026-10-17, following the successful bounded
[actual-binary test](../scripts/rack_binary_host/README.md) and one focused
architecture advisory. **User-facing hosted activation is not available.** Process
ownership, authored document/history/MCP preparation, core mixed graphs and the
desktop callback adapter are implemented below. The
installed OpenPatch 0.2.7 remains unchanged. The full
[compatibility obligation](COMPATIBILITY_DECISION.md#extensions-and-existing-vcv-binaries)
and native Rust extension API remain separate.

## Implemented transport foundation

The separate [Rust/SDK companion gate](../scripts/rack_binary_host/README.md#rustshared-memory-transport-foundation)
now passes actual binary processing through bounded shared memory, per-frame
polyphony, ordered sample-offset events and persistent restore. It measures one
Rack routing frame in its source/sink harness and verifies a non-silent native
oscillator WAV sample-by-sample. Actual stall/crash/late/malformed/parent-death
cases and eight parent boundary tests pass; parent endpoint calls allocate and
deallocate no heap memory. This is a debug correctness gate, not an application
or device benchmark. Live document/UI/MCP authority, application activation and
sustained realtime/device deadlines remain open.

## Implemented process and document preparation

`binary_host::process::LaunchConfig` prepares a genuine child on the control
thread. It verifies plugin/runtime fingerprints, actual loaded-image receipts
and the complete supported snapshot; private staging belongs to the child
lifetime. An owned transport endpoint can move to the audio thread while the
process supervisor retains only a separate header view. Capturing persistent
state consumes that exact drained endpoint, requires acknowledged processing
and successful stopped-child exit, and reaps the process before staging cleanup.
Wrong fingerprints, pending capture and a different mapping with identical public
fences are refused. Drop cleans up without publishing a successful capture.
No waits, JSON or process operations have moved into endpoint calls.

Project document **version 3** adds an explicit `hosted` node containing binary
identity, a declared port manifest and the full immutable Rack snapshot. Versions
1 and 2 migrate without changing their graph scheduling, source data or layout.
Older OpenPatch builds cannot read the new envelope. Individual hosted snapshots
are bounded to 1 MiB and total hosted state to 32 MiB. Unknown state remains
preservable; the supported OFFSET execution/edit capability rejects unknown
fields rather than discarding them. Cached snapshot hashes keep dirty-state
observation from serializing opaque state every frame.

`set_hosted_parameter` (`offset`, `scale`) and `set_hosted_option`
(`offset_first`, `disable_output_limit`) use the same revision-fenced Session
history as native edits. Parameter/option edits preserve document/save generation
and layout; native edits retain hosted snapshots and incident cables. Full
save/reopen preserves both custom flags and binary identity. Compact inspection
returns state hashes/lengths; full inspection returns the complete snapshot.

These commands do **not** launch a child or enable execution. Exact playback
remains suspended and explicitly requested Partial playback omits hosted nodes
and their routes. Inspection and the inactive rack panel report that the host is
not running. This panel preserves cable locations; it is not the final active
Signal Atlas hosted control panel. Shared Session, actual stdio MCP and native
headless App checks cover edits/history/save/reopen. They do not establish live
hosted controls, coherent live capture or device deadlines.

## Implemented mixed graph and callback adapter

The Core engine now compiles explicit external vertices alongside native nodes,
using the complete cable registry and its existing synchronous transfer rules.
External vertices are not native catalog aliases. Native IDs, kinds, routing,
stacking and normaling remain intact. Engine-bound frame exchange supplies every
external output once and returns the canonical inputs consumed before the
end-of-sample transfer. Unknown/foreign/duplicate handles or malformed frames
are refused before mutation. Ordinary processing without a new external frame
silences that producer after the already-valid previous cable frame; it cannot
repeat the last external CV indefinitely. External payloads and their incident
histories do not silently migrate into a replacement runtime.

`HostedGraphPlan` binds the authored document to this full graph. The first
companion capability still supports one OFFSET, graph v2 at 48 kHz, a connected
signal input and at most one cable per hosted input. Stacked routes remain
preserved but execution is refused: wire v1's pre-summed input cannot reproduce
every raw stacked-input channel/normaling or overflow case. Later island transport
must carry individual source-edge frames into genuine child cables. Native
stacking inside the parent engine already retains its normal behavior.

The desktop callback can own `HostedRunner` inside a prepared graph while process
ownership remains on the control side. Scope, Listen and native events use the
same engine frame path. Callback profile checks use the actual stream rate and whole buffer before
internal chunking; the runner also checks its engine rate. Failure silences the buffer, latches suspension and appears
in `inspect_performance.hosted_runtime`; priming does not masquerade as running.
Core rejections latch both runner and callback failure. A hosted crossfade donor
freezes its final audio instead of submitting again.
Full retirement queues retain the endpoint for control-thread disposal, and stale
hosted revisions cannot become active. Frame exchange, adoption, failure and
retirement checks detect no callback allocation/deallocation.

The real installed binary passes the Core mixed-graph gate: a native one-sample
impulse reaches a native receiver at frame **387**, the declared feedback path
recurs every **388** frames, and simultaneous ordered parameter events at hosted
input frame 657 change native output at **1042**. An actual desktop Callback
fixture also reproduces the single frame-387 impulse, applies both native PUSH
events, then retires/drains on the control thread and captures the original full
snapshot. These are paced debug tests without a native window or audio device.
The desktop fixture includes two one-frame callbacks whose processing exceeded
their 20.8 µs budgets; one small event callback in the Core runner gate also
overran. Neither gate qualifies arbitrary tiny callbacks or sustained real-time
playback. No performance advantage is claimed. [Recorded evidence](benchmarks/rack-binary-mixed-graph.json).

Application launch/configuration, user-facing delayed permission, active
Signal Atlas controls, shared live parameter/state publication and coherent
live save/reopen remain unfinished. The actual Callback fixture explicitly
injects a prepared graph; it does not override document Exact/Partial permission.

## Implemented full-document request boundary

The Session's retained audio ticket now seals the complete immutable document,
load generation, execution request, native state carrier, wave banks and ordered
lifecycle effects together. Startup, device retry and normal application edits
consume this ticket. Scope rebuilds retain its accepted context. Native audio
preparation cannot extract a native projection from a delayed-host request;
until full hosted dispatch is implemented, it refuses that request explicitly.
This prevents a permission flag from accidentally enabling the omitted carrier.

The internal delayed-host request binds an explicit grant to the full graph's
topology and the provisional 48 kHz/Q128/D384/max128 profile. The key retains
module order and identity, structural settings, custom state, routing/order,
output, assets and automation; qualified ordinary knob values and layout do not
revoke it. A new load, topology change or retired incarnation revokes permission.
Undo cannot revive it. Grants are neither saved nor copied into cloned Sessions,
and they never change the meaning of Exact or Partial. Native-only offline
render/analysis refuses a delayed-host request, including from Partial mode.

Changing or revoking permission supersedes an old sealed request while retaining
unacknowledged lifecycle intent in order. Parameter edits within the same grant
retain the exact old ticket for retries. Loads publish their new generation
before notifying the backend, rather than notifying with the old generation.
Inspection distinguishes an internal request from activation (`active: false`).
This boundary is not a user-facing Start command or a live-event admission API.
The next dependency is bounded precommit admission and acknowledged hosted
parameter publication, followed by controller activation/retirement and a
coherent save barrier. Do not expose activation by adding a runnable enum to the
existing native Patch synchronization path. A second focused architecture
advisory identified this concrete authority/rebuild risk; no extra DSP oracle or
device qualification is claimed for these control-path changes.

Local Mac checks pass: 410 CLI/control tests, 242 audio callback tests and 257
project/startup/workflow tests (the desktop filters overlap), plus strict
CLI/desktop all-target Clippy and formatting. Focused review reproduced a
host-only identical-load/undo permission bug; loads now retire the full document's
modules and cables, and the new regression passes. Existing callback DSP and
reference comparisons were not regenerated for these control-only changes.

## User-visible exit

An OpenPatch patch plays native source → actual installed Bogaudio OFFSET →
native output. UI and MCP can edit controls through the same document/history,
undo/redo and save/reopen preserve hosted identity and custom state, and companion
failure is visible without blocking the audio callback. A feedback example
demonstrates the explicitly declared delay. This is a bounded headless plugin
capability, not arbitrary binary or plugin-widget support.

## Scheduling decision to qualify

Start with one companion owning the hosted island, its genuine Rack Engine,
registry and library lifetime. The parent retains the native document/history,
native graph and audio device callback. The initial qualification profile uses
**128-frame quanta at 48 kHz**, **384 frames (8 ms) of transport delay** and at most
**128 frames per actual callback**. This supersedes the earlier proposed n→n+1
schedule. Other rates must be
explicitly qualified or refused; do not process them as though they were 48 kHz.

The actual callback size/submission phase must leave a measured child-compute
window before output is due. A 128-frame transport unit alone establishes no
such window for every device callback size. Qualify the selected buffer profile
and startup/partial-block behavior under representative load before enabling it;
neither padded WAVs nor short paced debug tests qualify device deadlines.

Preallocated shared-memory slots carry inputs for quantum n and outputs labeled
for n+3. Input frames `[128n,128n+127]` supply external outputs at
`[128n+384,128n+511]`. Four slots suffice when due output is consumed before new
input is submitted. Partial input blocks persist across callbacks; startup
outputs silence. The callback never waits for the child. Consume only an output with the
exact expected quantum and generation; missing/late output is not a reason to
replay stale CV. Channel counts travel with each frame's voltages, so a
within-block polyphony change is not silently reduced to one block-wide width.
Native-only and Rack-internal scheduling retain their own contracts.
The measured 387-frame native producer→plugin→native receiver path includes two
native cable frames and one Rack routing frame beyond the 384-frame transport;
that count excludes device buffering and changes with topology.

The cross-process delay is a deliberate scheduling difference. Neither
timestamps nor output alignment restores arbitrary sample-scale feedback.
Execution plans and status must disclose it. Exact import playback rejects a
topology whose source semantics cannot be represented; running it with delay
requires an explicit execution permission. Do not reinterpret today's Partial
permission or silently activate an imported unresolved node.

The full goal retains the need to qualify tighter mixed feedback. A likely
later route is colocating the necessary native Rust DSP and hosted modules under
one sample scheduler in the companion. That is provisional: preserve both
native and Rack behavior and measure it before selecting the final architecture.
Wrapping Rust modules as Rack modules alone would not prove that contract.

## Protocol and authority

Protocol v1 needs capability negotiation, graph preparation/activation,
sample-offset parameter events, bounded audio/CV transfer, persistent-state
capture/restore and health. Fence every transfer with protocol version, document
generation, plan revision, child incarnation, sequence and frame/quantum identity.
Specify limits for modules, ports, channels, events and serialized state before
accepting data. Simultaneous events need a deterministic order. Transport must
reject malformed sizes/counts and unsupported versions before consuming data.

`ProjectDocument` owns authored parameters, hosted identity, binary/runtime
fingerprints and opaque persistent state. Native UI and MCP submit the same
revision-fenced commands; the child is an executor, never another history or
document authority. Import provenance and repair remain recoverable. Effective
controls/durable changes need coherent publication, with explicit assignment
intent preserved through graph adoption, undo and redo.

Initially capture persistent state at an explicit quiescent execution barrier,
with the brief suspension reported. A failed/stale capture cannot register a
successful current save. Restart/undo can restore acknowledged persistent state;
they do not promise continuation of an unrecorded transient DSP history.

Crash, malformed output or a missed deadline invalidates the affected result;
output silence for the affected execution domain and surface/suspend dependent
execution through the shared playback contract. Never hold an old CV block
indefinitely. A restarted child gets a new incarnation; old results cannot pass
the new fence. No library load, state JSON, allocation, process wait or blocking
IPC belongs in the device callback.

## Done checks

1. A real patch plays/renders through the actual binary. Measured impulse,
   polyphony, sample-offset parameter events and explicit feedback match the
   declared schedule, including startup and rate refusal.
2. Native UI/MCP edit, undo/redo and save/reopen preserve controls, source/hosted
   identity and both OFFSET custom flags through the shared authority.
3. Injected stalls, crashes, malformed and stale output cannot block the callback
   or cross revision/incarnation boundaries; failure and restart remain visible.
4. Measured end-to-end latency and deadline misses on this Mac support the chosen
   quantum under a documented representative load. Report the actual result;
   merely passing numerical anchors cannot establish realtime viability.

No automatic installation or general compatibility claim follows. Packaging,
redistribution/license review, widget policy, arbitrary-model assumptions and
the wider host matrix remain open after this vertical slice.
