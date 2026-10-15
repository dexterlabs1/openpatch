# MIDI CC: implementation and acceptance contract

Status: implemented with bounded reference DSP, Session, callback and headless
UI checks. Numerical gates and lifecycle decisions below were fixed before the
first production comparison. Native macOS window, audio and physical MIDI
qualification remain separate gates. The existing
`midi_cv` module keeps its current behavior. New version-2 kind: `midi_cc`.

The behavioral oracle is the actual
[Rack MIDI-CC DSP class at 8ea470f](https://raw.githubusercontent.com/VCVRack/Rack/8ea470f343058ade6f75f16d503a1eb330e1700c/src/core/MIDICC_CV.cpp).
The [Core manual](https://vcvrack.com/manual/Core#MIDI-CC) says 14-bit is unsupported;
the pinned implementation supports it. This implementation follows the pinned
source for that behavior. Production Rust and panel artwork are original; the
reference class is compiled temporarily, never included in the shipped product.

## Interface and signal behavior

No inputs; outputs `out1` through `out16`. Assignments `cc1` through `cc16` are
integer CC numbers −1 (unmapped) through 127, default 1 through 16. Assignment
collisions unmap the prior cell; patch/preset arrays resolve in cell order, with
the last cell winning. A direct assignment or Learn changes the chosen cell and
unmaps any existing owner in one operation. `smooth` defaults 1, `mpe` defaults 0,
`high_resolution` defaults 0; these are Boolean parameters. `channel` is 0/omni
through 16, using human channel numbering. The selected hardware MIDI endpoint is
global to OpenPatch and must be labeled global in the interface.

The standard MIDI decoder accepts only ordinary 7-bit data bytes and sixteen
wire channels. The nonstandard signed gamepad extension is excluded. Accepted
ControlChange events update stored controllers even when no output uses them;
other channel messages do nothing. Normal mode merges accepted wire channels
into storage channel zero and mono outputs. MPE uses all sixteen wire channels
and sixteen lanes per output; the selected channel filter still applies.

There are 128 committed controller values per wire channel. High-resolution mode
stores CC0..31 into a separate pending MSB bank without committing a value.
CC32..63 commits both its own LSB and the corresponding pending MSB. Other CCs
commit normally. These rules also apply to controllers not currently mapped.
Targets use `(MSB * 128 + optional LSB) / 16256`, clamped to [0,1], times 10 V.
Only mappings below 32 include an LSB in high-resolution mode; LSB controllers
remain independently assignable. An MSB of 127 with LSB zero already reaches 10 V.

Each cell/lane has a normalized f32 exponential filter. With smoothing enabled,
a target difference strictly below one uses the Rack-style forward-Euler filter
with time constant 1/30 s; a difference >=1 or disabled smoothing jumps directly.
Unmapped outputs publish zero without changing filters. Disconnected outputs
freeze filters; MIDI reception and learning continue. Compiled cables, the
selected graph output, and explicit capture/probe taps count as consumers. An
unconsumed output has no active output lanes; reconnecting resumes its filter.
Inactive MPE lanes freeze when MPE is disabled. Mode changes do not clear storage
or filter histories.

## Persistence, adoption and host lifecycle

Saved state contains sixteen assignments and all 128 committed values from
storage channel zero. `cc_value0`..`cc_value127` are hidden integer parameters
0..127, default zero. Pending MSBs, other-channel values, filters and Learn state
are transient. Coherent live capture must save actual audio state, including
recent controller movement. Controller movement creates neither graph rebuilds
nor undo entries.

Same ID/kind/rate adoption preserves all committed/pending values, filters and
active Learn state, subject to explicit configuration/initialization epochs.
Changed assignments affect mapping only and never restore stale controller
values. Full saved-state restoration is reserved for explicit load/preset/state
restoration. Mapping edits and their Undo/Redo change mappings only, preserving
current controller movement. Full load/restoration changes saved channel-zero
values and mappings but preserves transient filter histories on a compatible
instance. Fresh instance/sample-rate changes start with zero filters.

Initialize restores default maps/options and zero controller/pending values,
and cancels Learn, but retains filter histories. Explicit reset-state clears
transient histories as well. Bypass publishes mono zero and freezes DSP/MIDI;
bypass transitions cancel Learn and clear pending MSB pair state. Panic,
endpoint disconnect and transport overflow cancel Learn and clear pending MSB
pair state while retaining committed values/maps/filters. These are explicit
host policies rather than unqualified claims about Rack device lifecycle.

## MIDI Learn and history

Arm/cancel are transient commands, not saved Learn parameters. Learn arms one
cell per module, accepts the first channel-accepted CC whose incoming value
differs from its currently committed controller value, and applies its new
mapping/collision on that same audio sample. Learn observes the value before
high-resolution pairing rules update storage. Repeated equal CCs and rejected
channels must not complete Learn.

The request identity includes backend, module token, configuration epoch, Learn
serial and cell. Each armed module reserves one durable completion slot. Atomic
armed/completed/cancelled arbitration prevents a completion from being lost to
cancellation or queue overflow. Audio publishes old/new mapping arrays. Session
reconciles the completion under its mutex, independently of GUI visibility, and
records one undoable assignment operation. An unrelated MCP edit preserves the
ticket. A conflicting mapping edit, preset, Initialize, Undo, bypass or removal
must resolve a completed ticket or cancel an armed ticket before proceeding.
Obsolete tickets cannot target a replacement instance, epoch or backend.

## Predeclared verification

Compare every exposed lane and sample against the temporarily compiled actual
pinned class at 44.1, 48 and 96 kHz. No warm-up, trace alignment or threshold
exemption. Voltage error must be <=0.0001 V + 0.000001*abs(reference volts).
Mappings, Learn completion sample, output width, pairing behavior, disconnected
filter freeze and jump decisions must agree exactly. Record all worst samples,
input hashes, source/SDK/toolchain/compiler flags and production/driver hashes.
Nonstandard gamepad input and per-module hardware endpoint selection are outside
this reference scope and remain explicit parity gaps.

Fixtures cover all sixteen cells/channels, unmapped/colliding assignments,
startup and smoothing boundaries, MPE/mono transitions, accepted/rejected channel
filters, unrelated messages, MSB-only/LSB-first/reversed/interleaved pairs,
127+127 saturation, disconnect/reconnect, stored unmapped CC reassignment,
same-sample event ordering, persistence, adoption, initialization and reset.
Separate real callback/Session tests must prove durable Learn completion,
conflicting/unrelated MCP edits, capture/save/Undo, stale token/epoch/backend
safety, queue reset/backpressure and first-callback/adoption allocation bounds.
The shared sixteen-cell panel must match browser previews, expose human-readable
assignments/options and explicit Learn/Cancel, and use actual telemetry. Native
Mac hardware MIDI, window/input and audio-device verification are separate gates.

## Results and use

Add **MIDI CC** from the library, or open **File → MIDI controller voice** for a
connected voice. Select a hardware input in **Audio / MIDI**; this selector is
global. Click an assignment to type a controller number, or right-click it for
**Learn MIDI CC** and **Cancel MIDI Learn**. Tab visits all sixteen assignments.
Learn requests are queued intent; the panel's LRN indicator comes from actual
callback state. A completed Learn records one undo step, including any displaced
mapping. Smooth, MPE, 14-bit and channel controls apply to this module.

The [reference report](benchmarks/midi-cc-reference.json) records 45 cases,
221,184 frames and 36,284,064 active lane samples. All recorded binary traces
matched bit for bit on the documented Apple Silicon SDK/toolchain run. The
initial failed comparison and the resulting filter-granularity correction are
preserved; the numerical gate and fixtures did not change.

Separate production-path tests cover the callback's event sample, durable
completion, cancellation races, reset of future graph tickets, active-only
crossfade Learn, movement-preserving mapping Undo/Redo, coherent capture, backend
replacement and headless reconciliation. Audio-thread reset/overflow/close paths
are checked for allocation and deallocation. Headless UI tests cover all sixteen
editors, Learn actions, telemetry, normalized fallback assignments, and shared
rack/browser rendering. This does not establish hardware calibration, native Mac
interaction, MIDI driver behavior, or complete Rack host compatibility.
