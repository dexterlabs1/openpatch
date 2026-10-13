# MCP integration

OpenPatch exposes tools over newline-delimited JSON-RPC on stdio, with protocol
versions `2025-11-25`, `2025-06-18`, and `2025-03-26`. The server negotiates the
version during initialization. Stdout contains protocol messages only; errors
from startup go to stderr. It does not open a network port or call a model API.

Build `cargo build --release --workspace` first. The examples below launch
`target/release/openpatch-desktop --mcp --workspace DIRECTORY`. Replace paths
with absolute paths on your machine. On Windows use `openpatch-desktop.exe`.

The MCP client launches a native window and controls that window's live patch.
GUI controls and MCP tools share the same session and revision counter. Playback
starts paused: press **Play** in the window (or Space when a text field is not
focused) to hear the patch. MCP can edit a playing patch, but has no tool to start
or stop speaker playback. `render_audio` exports a WAV independently of playback.

A window launched manually is a separate session. This stdio integration does
not attach to an already running window. If Claude and Codex each launch the
executable, each gets its own window and patch. Sharing a workspace only shares
saved files. When stdin reaches EOF, the MCP thread exits and the window remains
open; a client that terminates its child process can still close the application.

## Codex

```sh
codex mcp add openpatch -- /absolute/path/openpatch/target/release/openpatch-desktop --mcp --workspace /absolute/path/patches
```

Or add a table to your Codex configuration:

```toml
[mcp_servers.openpatch]
command = "/absolute/path/openpatch/target/release/openpatch-desktop"
args = ["--mcp", "--workspace", "/absolute/path/patches"]
```

On Windows, TOML literal strings avoid backslash escapes:

```toml
[mcp_servers.openpatch]
command = 'C:\OpenPatch\target\release\openpatch-desktop.exe'
args = ['--mcp', '--workspace', 'C:\OpenPatch\patches']
```

See [official Codex MCP configuration](https://developers.openai.com/codex/mcp/).

## Claude Code

```sh
claude mcp add --transport stdio openpatch -- /absolute/path/openpatch/target/release/openpatch-desktop --mcp --workspace /absolute/path/patches
```

See [official Claude Code MCP documentation](https://code.claude.com/docs/en/mcp).

## Claude Desktop

Merge this server into `claude_desktop_config.json`; preserve any existing servers:

```json
{
  "mcpServers": {
    "openpatch": {
      "command": "/absolute/path/openpatch/target/release/openpatch-desktop",
      "args": ["--mcp", "--workspace", "/absolute/path/patches"]
    }
  }
}
```

Use escaped backslashes in Windows JSON paths, e.g.
`C:\\OpenPatch\\target\\release\\openpatch-desktop.exe`.
Client support and setup locations may vary by version. These examples configure
the same local stdio server, not a remote HTTP connector. No existing user client
configuration is changed by this repository.

## Building a patch

1. Call `catalog` to discover modules, port names, parameters, and limits.
2. Call `inspect_patch` for the patch and current revision.
3. Use `preview_edits` to validate a proposed group of edits, then `apply_edits` to commit it atomically. `replace_patch` replaces the entire graph; individual edit tools remain available.
4. Select the audible node with `set_output`. Without an output the patch is silent.
5. Press Play locally to audition the live patch, or use `render_audio` with a new
   `.wav` filename and `seconds` between 0.001 and 30 to export it.
6. Save the graph with `save_patch` and a new `.json` filename.

Include `expected_revision` on edits when coordinating multiple operations. A
stale revision or invalid edit leaves both the patch and revision unchanged.
Tool failures return `isError: true`; malformed protocol calls use JSON-RPC errors.

The desktop starts with the demo patch. Call `inspect_patch` before editing;
include `expected_revision` to detect edits made in the window since your read.
Exchange saved JSON patches to move between separate client processes. These
graph-only MCP commands retain their existing new-name-only behavior. Native
File → Save uses `.openpatch` projects containing graph and layout; MCP does not
yet read or write those envelopes. See [Projects and recovery](PROJECTS.md).
A window that has committed its final close decision rejects subsequent graph
mutations and live module actions with `session is closing`; inspection and exports remain available.

## Capture current project Run state

`capture_project_patch` accepts optional `expected_revision` and returns
`patch`, `revision`, `state_id`, `running` (module ID → boolean), `state_source`,
and `frame`. It projects current SEQ-3 Run into a cloned graph without modifying
history, queuing controls, resetting DSP state, or writing a file. Native Save
and recovery use this same capture path; the MCP result does not include layout.

With live audio, every sequencer must be available at the requested revision
and one completed callback frame. Stale, missing, or incoherent telemetry
rejects the capture. `state_source` is `live` with a frame, `offline_reset` with
configured initial Run when no backend is attached, or `parameters` when there
are no sequencers. The latter two return a null frame. This is a point-in-time
capture, not an acknowledgment of controls awaiting audio processing.

## Initialize and Randomize

`initialize_modules` accepts `ids` and optional `expected_revision`.
`randomize_modules` also requires `seed` (unsigned 32-bit integer). Both work
inside `preview_edits` / `apply_edits`, validate the whole selection atomically,
and preserve cables, bypass and module identities. A preview has no live-state
side effects. The catalog describes per-parameter eligibility and distributions.

Initialize restores default controls and applies a module-specific runtime
hook once; history restores saved controls without replaying that hook.
Randomize produces reproducible saved values; redo does not reroll them.
See the [full editing and live-state contract](RACK_EDITING.md#initialize-and-randomize).

## Module preset library

Capture a module with `capture_module_preset`, review the returned `preset`, then
save those exact contents with `save_module_preset`. Use `list_module_presets`
and `read_module_preset` to browse saved `.opreset` files. Apply reviewed inline
contents with `apply_module_preset`, including the observed `expected_revision`.
Application supports preview and atomic batches; file reads are not repeated at
commit time. Complete live SEQ-3 captures use actual Run state from a coherent
callback snapshot, never an unavailable-state guess. See the
[preset format, history and storage contract](MODULE_PRESETS.md).

## Named outputs, polyphony, and feedback

New sessions and **New patch** use patch version 2. Each cable has a source
`output` (default `out`) and destination `input`. `set_output` accepts `port`
(default `out`); patch JSON stores it as `output_port`. Discover valid names,
minimum patch versions, and polyphony rules in `catalog`.

Version 2 supports 0–16 channels per cable and feedback, with exactly one sample
of delay per cable. Version-1 files retain their mono, same-sample DAG timing;
loading does not convert them. The desktop's **Enable polyphony and feedback**
action explicitly converts a legacy patch and can be undone. An MCP client can
make the same explicit conversion by replacing a patch with `version: 2`.
Conversion changes timing. See [engine contracts](ENGINE_V2.md).

Version-2 inputs also support stacked cables. `connect` defaults to rejecting
occupied inputs; set `mode: "stack"` to add another signal and optionally
`cable_id` to name the new cable. Every live cable has a persistent `id` in
`inspect_patch`. `disconnect` accepts either `cable_id` for one route or
`to` plus `input` for all routes on that input. Preview and commit moves as one
disconnect/connect transaction. See [stacked cable examples and timing](STACKED_CABLES.md).

Select `audio_stereo.out` for explicit L/R playback and stereo WAV export. Each
input sums its voices and converts 10 V to digital full scale; unpatched Right
follows Left. All other selected outputs retain channel-0 mono monitoring.
`poly_sum` explicitly mixes voices to mono, and `poly_split` / `poly_merge`
provide individual channel routing. See [MIDI and audio](MIDI_AUDIO.md).

`render_audio`, `analyze_signal`, and `capture_signal` accept optional sample-timed
`midi_events` and `module_events`, allowing reproducible performances from reset
without changing live notes, sequencer state, or patch history. The trace contract and CLI example are
in the [performance guide](MIDI_AUDIO.md#reproducible-agent-performances).

`analyze_signal` measures any named output offline from reset. It returns the
snapshot revision, channel-count range, and per-channel active-frame count,
minimum, maximum, peak, mean, and RMS. Statistics exclude frames where the
channel is inactive. It does not select a new live output, change history, or
start playback. For example:

```json
{"id": "chord", "port": "out", "seconds": 0.25}
```

With the bundled polyphonic voice this reports three constant pitch channels.
Analyze `vco` or `vca` to measure the three processed voices; analyze `sum` to
inspect their mono sum. Values retain each output's catalog units; they are not
automatically normalized, calibrated hardware measurements, or a live probe.

## Sequencer actions and rotation

`seq3` has three eight-step voltage rows (`row1_1` through `row3_8`), eight
`gate1` through `gate8` flags, and named outputs `cv1`, `cv2`, `cv3`, `trigger`,
`step1` through `step8`, `steps`, `clock`, `run`, and `reset`. Discover defaults,
CV controls, voltage units, and timing in `catalog`. `tempo` is BPM (15–960,
default 120); `steps` selects 1–8 columns. `running` sets the initial run state
for a fresh engine. Live run toggles belong to performance state and do not
rewrite that parameter. An explicit `set_parameter` assignment of `running`
also assigns the live state, even when its saved value already matches.
Loading or fully replacing a patch starts sequencers and sequential switches
from fresh runtime state and invalidates their old queued actions, even when
module IDs match. Ordinary parameter/routing edits preserve compatible state.

`rotate_sequence` takes `id`, `direction` (`left` or `right`), and optional
`expected_revision`. It moves all eight columns, including all three voltage
rows and gate flags, together. Omitted cells use their catalog defaults (0 V;
gates enabled). The rotation commits one revision and one undo step through the
same validation boundary as ordinary parameter edits.

```json
{"id":"seq","direction":"left","expected_revision":7}
```

In desktop mode with a live audio backend attached, `trigger_module` queues a
`clock`, `run`, or `reset` action for a `seq3` module. `run` toggles running;
`clock` advances only while running. `reset` selects the first column and
suppresses clocks for one millisecond, including its own sample. Use optional
`expected_revision` to reject a request against an edited graph.

```json
{"id":"seq","action":"clock","expected_revision":7}
```

The result contains the observed `revision` and a backend `receipt`. A queued
receipt confirms queue acceptance; the audio callback may apply or drop the
action later. `inspect_performance` returns `supported` and a backend
`performance` snapshot with queue/application/drop/backpressure counters. These
counters distinguish accepted requests from actions that reached the engine.
It accepts optional `expected_revision`. Live actions do not create undo entries
or change patch revisions. Headless sessions and desktop sessions without an
audio backend report `supported: false`, `performance: null`, and reject
`trigger_module` as unsupported.

For reproducible offline actions, supply `module_events` to `render_audio`,
`analyze_signal`, or `capture_signal`:

```json
{
  "id":"seq","port":"cv1","seconds":0.25,
  "module_events":[
    {"frame":1200,"id":"seq","action":"clock"},
    {"frame":2400,"id":"seq","action":"reset"}
  ]
}
```

Each event has exactly `frame` (a nonnegative integer), `id` (an existing `seq3`
module for sequence actions, or `logic`/`process`/`pulses` for manual controls), and `action`
(`clock`, `run`, `reset`, `b_press`, `b_release`, `gate_press`, `gate_release`, or `pulse1_press`/`pulse1_release` through `pulse10_press`/`pulse10_release`).
Each action must match its module kind. Frames index the stored patch
sample rate, must be less than the rounded render frame count, and must be
nondecreasing. The list is capped at 65,536 events and at 64 actions for one
module at one frame. Equal-frame actions preserve array order. MIDI events run
before module actions at a tie; module actions run before that sample's CV
run/reset/clock inputs. Both traces are fully validated, including target IDs,
before processing or creating a WAV. Results include `module_event_count` and
`midi_event_count`.

Offline operations start a separate engine from reset and never change live
performance state or graph history. `analyze_signal` and `capture_signal` accept
optional `expected_revision` to guard their coherent snapshot. Analysis and
capture retain raw signal units; WAV export applies the usual selected-output
channel and clipping policy.

The `switch_1to4` and `switch_4to1` modules route clocked signals with 2–4
positions and a saved `declick` parameter. They are controlled through their
clock/reset inputs; they do not accept sequencer `module_events` or
`trigger_module` actions. Their native route indicators use read-only engine
telemetry. See [clocked routing](SWITCHING.md) for mode-specific polyphony,
timing and the stereo example.

## Momentary controls and live module observations

Logic accepts `b_press` and `b_release`; Process accepts `gate_press` and
`gate_release`. Pulses accepts `pulse1_press`/`pulse1_release` through `pulse10_press`/`pulse10_release`. Use these with `trigger_module` for live controls, or as
sample-timed `module_events` for offline render/analysis/capture. Sequence
clock/run/reset actions still require `seq3`. A manual press remains held until
release; it is not a one-sample pulse and is not saved as a parameter or undo
entry. For Logic and Process, the last ordered button state wins before CV evaluation. Pulses preserves same-sample press/release edges as triggers; a click can emit a trigger without leaving a gate held.

```json
{"id":"logic","action":"b_press","expected_revision":7}
```

Send the matching release to end a hold. Releases remain valid during bypass.
A full live queue can return `queued: false`, `emergency_release: true`, and
`safety_pending: true` in the receipt: a separate bounded path clears the hold
and invalidates older queued presses. For Pulses this releases every held row, while ordinary row releases are independent. It does not reset held voltages or DSP
histories. `emergency_release_requests` and `emergency_release_applied` are
separate from ordinary queued/applied/dropped counters. A receipt is acceptance,
not proof that audio has processed it. No live backend means these requests are
unsupported; offline event traces remain available.

`inspect_performance.performance.modules` is keyed by module ID for the
sequencer, sequential switches, the four signal processors, VCA Mix, and Pulses. Each entry has
`kind`, saved `bypassed`, `available`, actual applied `revision` (or null),
`state`, `controls`, and `lights`. Sequence state retains step/running/length.
Manual `controls` reports `pressed` and `pressed_mask` (Pulses rows use bits 0–9); indicator `lights` provides `count`, ten
fixed-capacity brightness pairs, and a `manual` flag. Signal processors use
mono/poly pairs; VCA Mix uses the first value for each channel meter and leaves
the second value zero. Pulses uses one entry per row. Only the first
`count` pairs are meaningful; names and interpretation come from the catalog's
`light_names` and `light_policy`. Unsupported fields are null.

The desktop snapshot reports global `coherent` and a common `frame`. It uses a
bounded read retry around callback publication; contention returns unavailable
module telemetry rather than mixing callbacks. Each available module snapshot
must meet the current requested revision and that common frame.
An unavailable snapshot is not a zero voltage or a released button. Availability
also does not mean playback is running. Values observe the live backend; they
do not provide arbitrary raw waveforms. Use the native scope for live waveforms,
or `capture_signal` for an offline trace from reset.

[Signal processor equations and state policies →](SIGNAL_PROCESSING.md)

## Saved bypass and mute inversion

`set_bypass` accepts `id`, the required JSON boolean `bypassed`, and optional
`expected_revision`. `invert_mutes` accepts a `mutes10` ID and optional revision,
and flips all ten flags as one edit. Both support preview/batch edits and shared
undo/redo. Parameters remain editable during bypass; sequencer performance
actions are rejected and earlier queued actions are invalidated on entry.
See [utility and bypass contracts](VOLTAGE_UTILITIES.md) for routing, frozen state,
and the MIDI exception.

## Offline waveform capture

`capture_signal` returns a bounded waveform trace for one named output and one
**1-based channel**, from 1 through 16. For example, with the bundled polyphonic
voice, capture the second VCO voice (replace revision `7` with the value from
`inspect_patch`, or omit the optional revision guard):

```json
{"id":"vco","port":"out","channel":2,"seconds":0.25,"points":512,"expected_revision":7}
```

`port` defaults to `out`, and `channel` defaults to 1. `seconds` must be
0.001–2; `points` must be an integer from 64 through 2048, default 512. The result
includes the captured `revision`, `id`, `port`, `channel`, exact `sample_rate` and
rounded sample `frames`, `requested_points`, and actual `points`. If the render
has fewer frames than requested points, each frame gets one bin.

Each entry in `bins` contains `start_frame`, exclusive `end_frame`,
`active_frames`, `min`, `max`, and `mean`. Nonempty bins cover every rendered frame
without gaps or overlap. Every active sample contributes, so a one-sample pulse
survives in the extrema even when it falls between display points. `active_frames`
counts frames in which the selected channel exists, including zero-valued gates
or audio. When a channel is absent, its samples are excluded; a bin with no active
samples has null `min`, `max`, and `mean` rather than fabricated zeros.

`summary` reports active-sample `min`, `max`, `dc` (mean), `rms` (including DC),
and `peak` (maximum absolute amplitude), plus total `active_frames`. All five
amplitude statistics are null when the selected channel never exists.
`units` comes directly from the selected port's catalog `output_metadata`:

| `units` | `amplitude_domain` | `digital_full_scale` |
| --- | --- | --- |
| `V` | `voltage` | null |
| `normalized digital amplitude` | `normalized_audio` | 1.0 |
| `legacy normalized` | `legacy_normalized` | null |

Capture returns raw values without clipping, channel summing, or automatic
normalization. For example, a voltage peak of 5 remains 5 V; a normalized digital
audio peak above 1 remains above 1. These are software signal units, not calibrated
measurements of external hardware.

This tool always starts a separate engine **offline from reset**. Optional
`midi_events` and `module_events` reproduce a performance using the same
sample-frame rules as `render_audio`; the response includes `midi_event_count`
and `module_event_count`. It never selects a new
live output, changes history, or changes running notes. The native Scope's live
tap observes the running engine instead; `capture_signal` does not fetch that
live tap. A concurrent edit may advance the session while capture runs, but the
returned revision identifies the coherent snapshot that produced the trace.

## Atomic edits and shared history

`apply_edits` accepts 1–256 graph edits and validates the **final** graph. The
whole transaction commits one revision and one undo step. Temporary disconnected
or unresolved connections inside the transaction are allowed; the final graph
must compile. Any failure leaves the live graph, revision, and history unchanged.
The supported operations are `add_module`, `remove_module`, `connect`,
`disconnect`, `set_parameter`, `set_output`, `set_bypass`, and `invert_mutes`. File I/O, nested transactions,
and history commands are excluded.

For example, pass this argument object to `preview_edits`, using the revision
from `inspect_patch` (replace `7` below):

```json
{
  "expected_revision": 7,
  "edits": [
    {"tool": "add_module", "arguments": {"id": "pitch", "kind": "constant", "params": {"value": 1}}},
    {"tool": "connect", "arguments": {"from": "pitch", "to": "vco", "input": "pitch"}}
  ]
}
```

This assumes an existing `vco` with an unoccupied `pitch` input. Preview returns
`committed: false`, the validated candidate patch, the base revision, and a
`changes` summary of added/removed/modified modules, cables, and output; it does
not mutate the live patch. Submit the same arguments to `apply_edits` to commit.
If a GUI or MCP edit happened meanwhile, the revision check rejects the request;
inspect again and revise the proposal. Preview does not reserve the session.

`undo`, `redo`, and `inspect_history` share the desktop's history, bounded to 128
graph edits and committed module moves. Cmd-Z and Shift-Cmd-Z invoke the same
commands. One completed single/group drag is one checkpoint in this ordered
history; its graph is unchanged, but revision and state identity advance. Moves
and their undo/redo preserve compatible live DSP state. A new move breaks a
parameter gesture group and clears redo like other edits. Revisions always
increase, including undo/redo. History is in memory only; file writes and
playback state are not included. Pan/zoom remain saved view settings without
separate undo entries. The desktop retains layouts for states it has displayed,
so undoing a module deletion
restores its placement. States created and undone entirely between GUI polls may
not have a recorded layout. `state_id` identifies a retained history state; use
`revision`, not `state_id`, for concurrency checks.
Consecutive `set_parameter` calls may use the same `history_group` string to
form a single undo step for one parameter gesture, while each edit still advances
the revision. Use a unique group per gesture; unrelated edits and undo/redo
break the group. Omit this field inside transactions.

## Duplicating module groups

`duplicate_modules` atomically copies a module group using explicit
`modules: [{id, new_id}]` mappings. `cable_mode` is `none` (default), `internal`,
`inputs` (internal plus external incoming cables), or `all` (every incident cable,
including outgoing connections to existing destinations). Selected endpoints
are remapped; external endpoints stay attached. Each cable is copied once with
a fresh ID, preserving repeated routes and order. Saved parameters, bypass and
current SEQ-3 Run are copied; other runtime state starts fresh. The selected
patch output remains unchanged. Version-1 stacking restrictions still apply to
the final graph. The tool supports
`expected_revision`, transaction previews, batches, and one-step undo/redo.
See [duplication arguments and native workflows](RACK_EDITING.md).

## Headless offline mode

The original CLI remains available without a display or audio device:

```sh
/absolute/path/openpatch/target/release/openpatch mcp --workspace /absolute/path/patches
```

For a headless client configuration, use `openpatch` as the command and
`["mcp", "--workspace", "/absolute/path/patches"]` as its arguments. It starts
with an empty patch, renders offline, and has no native window or speaker
playback. It does not share state with desktop processes.

## Execution boundaries

Compilation, JSON parsing, file access, MCP dispatch, and rendering orchestration
run on control threads. The native GUI observes committed revisions, compiles a
device-rate engine, and transfers it to the audio callback through bounded
queues. The callback never locks the session or dispatches MCP commands. Edits
transfer compatible oscillator, capacitor, filter, and resampler state by stable
module ID before crossfading. New or changed module kinds start from reset.

Offline rendering and benchmarks use a coherent patch snapshot and release the
session mutex before processing. MCP requests on that connection remain
sequential while the operation runs, but GUI edits and live audio can continue.
Every offline render starts oscillator/filter state from reset and does not
change the live patch or revision. The device sample rate can differ from the
stored patch sample rate used for WAV export.

Requests and patch files are capped at 1 MiB. Patches are bounded to 256 nodes
and 1,024 cables. File arguments accept simple filenames, not paths; symlinks
are rejected and writes use exclusive creation. Use a workspace owned by you,
not a directory concurrently modified by untrusted users. This is a local tool,
not an OS sandbox or a hardened multi-user file service.

[Ten-row Pulses timing and state contract →](PULSES.md)
