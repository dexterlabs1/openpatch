# First application-host milestone

Planning checkpoint, 2026-10-17, following the successful bounded
[actual-binary test](../scripts/rack_binary_host/README.md) and one focused
architecture advisory. **This application path is not implemented.** The
installed OpenPatch 0.2.7 remains unchanged. The full
[compatibility obligation](COMPATIBILITY_DECISION.md#extensions-and-existing-vcv-binaries)
and native Rust extension API remain separate.

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
native graph and audio device callback. A proposed first transport quantum is
**128 frames at 48 kHz**, introducing a fixed 2.667 ms transport delay plus actual
Rack routing/adapter effects, which must be measured. Other rates must be
explicitly qualified or refused; do not process them as though they were 48 kHz.

Preallocated shared-memory slots carry inputs for quantum n and outputs labeled
for n+1. The callback never waits for the child. Consume only an output with the
exact expected quantum and generation; missing/late output is not a reason to
replay stale CV. Channel counts travel with each frame's voltages, so a
within-block polyphony change is not silently reduced to one block-wide width.
Native-only and Rack-internal scheduling retain their own contracts.

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
