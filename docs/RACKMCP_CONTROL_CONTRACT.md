# RackMCP 2.0.0 control and Listen contract

Source audit, 2026-10-17; a design contract, not an implemented support claim.
Reference: genuine RackMCP commit `285e84c5ff2d3ecef25890da07f578af6a0c5ca6`,
frozen at `native-test-output/rack-control-import/reference/rackmcp` (prefix `R/`
below). Preserve the sequencing-family contract alongside this host boundary.
Core Notes and audio I/O remain needed, separately tracked host boundaries;
this document does not establish their mappings or device qualification.

## Bridge: source behavior

- `R/src/plugin.cpp` starts the singleton service at plugin initialization;
  adding/removing Bridge only changes its presence count. It has no signal ports.
  The first Bridge widget step attaches a scene-owned command pump which survives
  module removal. Without that pump, queued operations fail `BRIDGE_NOT_READY`.
- `R/src/rackside/BridgeModule.cpp`: LINK is authenticated-client presence (full),
  running with no clients (0.15), or offline (zero); WRITE is the lease hint;
  WARN is running and history unsaved. DSP updates lights every 512 samples.
  The display shows instance prefix, listening port, client count, writer name
  or read-only, last operation, and unsaved warning. These are actual host facts.
- `R/src/rackside/Handlers.cpp::handleStatusGet` includes instance/session IDs,
  patch epoch, actual sample rate, patch name, saved flag, Bridge/pump presence
  and current writer lease. A saved flag is history's `isSaved()`, not an audio
  health indicator, successful capture, or promise that future edits persist.
- `R/src/rackside/RackBridge.cpp`: startup creates fresh instance/session UUIDs,
  loads/creates a private pairing secret, opens an ephemeral loopback port and
  publishes heartbeat discovery manifests. Shutdown closes clients, joins the
  capture worker, removes the manifest and scrubs the in-memory secret.
- `R/src/core/service.cpp`, `lease.cpp`: hello/welcome and a single-use nonce
  authenticate HMAC using the pairing secret and instance/session identity.
  Authenticated readers do not automatically own mutation authority. One
  connection holds a 30-second renewable writer lease; same-owner acquisition
  extends it, expiry/release/disconnect relinquishes authority. Mutations need
  authorization both when enqueued and before UI-thread execution.
- Reset pairing is an edge-triggered request handed from DSP to the UI, also
  available in the context menu. It rotates the persisted secret and restarts
  the listener, disconnecting clients/leases while retaining instance/session
  IDs; failed restart withdraws discovery. It is not a patch reset.
- Neither Bridge nor Listen implements custom module JSON state. Source
  pairing/discovery/session/lease/job state belongs outside saved module data.
  `R/src/rackside/PatchFiles.cpp` warns when a saved patch lacks Bridge because
  a fresh Rack scene cannot establish its command pump on reopen.

## Listen: source audio and job behavior

- `R/src/rackside/ListenModule.{hpp,cpp}`: L/R inputs 0/1 and outputs 0/1,
  no parameters. Each output copies its matching input's channel count and raw
  per-channel voltage unchanged. No gain, normalization, downmix or mono-copy
  is applied to these outputs. Configured bypass routes each input to its output;
  bypass skips normal processing, so it also stops capture production/light updates.
- Normal processing increments `framesSeen` even when unarmed. Only an armed
  capture pushes a stereo frame containing each input's polyphony sum, with
  absent R equal to zero. A preallocated SPSC ring holds `2^18` frames; a full
  ring drops the new frame and increments `dropped`, never blocks or allocates.
  Every 256 samples it publishes connection hints and the actual armed lamp.
- `R/src/rackside/ListenService.cpp`: select an explicit Listen ID or require
  exactly one. `listen.start` is a read-authorized operation, not a writer-lease
  mutation; it requires 0.1–30 seconds and optional boolean `writeWav`.
  One capturing/analyzing job is allowed globally, with eight recent jobs kept.
  Start snapshots sample rate and targets `ceil(seconds * rate)` collected frames.
- The worker clears the ring, arms, drains, disarms, then analyzes/optionally
  writes WAV off the audio thread. Poll reports capturing/analyzing/done/failed,
  collected-frame progress and final metrics or error. Dropped frames are
  reported, not reconstructed; collecting the target can extend wall duration.
  The job does not adapt its rate when the engine rate changes mid-capture.
- No arriving frame for two seconds fails the job (stopped engine, bypass or
  removed module); silence still arrives as real frames. Shared tap ownership
  prevents use-after-free on deletion. Shutdown disarms/fails unfinished work.
  Analysis uses the connection hints; WAV alone duplicates L into R when R is
  absent. Never infer stereo output routing from that export convention.

## OpenPatch reuse and gaps

Use the [shared document authority](COMPATIBILITY_DECISION.md) for edits/history,
source preservation, save/reopen and Exact/Partial permission. Reuse
`crates/openpatch/src/control.rs` (`SharedSession`, revision checks, backend),
`control/project_capture.rs` (coherent saved-state capture), `mcp.rs` and
`local_mcp.rs`/`attach.rs` (same-user Unix socket session discovery, bounded
clients, shared desktop session and revocable transport authorization).
Existing `LocalServer::session_id/client_count` are real observations, but are
not Rack's instance identity, authenticated-client count or writer lease.

Native attach has no Rack pairing-secret/HMAC or single-writer lease contract.
Add explicit native control authority/status where required; label unsupported
source transport fields honestly. Import must not start a listener, rotate
secrets, connect clients, bind audio hardware or invent an endpoint/writer.
Bridge must provide useful real native session/save/control status and actions;
an inert panel labelled compatible does not meet this contract.

Listen needs a distinct compiled raw passthrough model and real bounded live
tap, with bypass/deletion/suspension/rate-change lifecycle specified. Reuse
callback publication and unavailable/stale handling from `inspect_performance`
and `inspect_scope`, not their data shape as a fictitious audio recording.
`control/capture.rs::capture_signal` renders a reset offline engine (up to two
seconds); it cannot satisfy an armed live Listen job. Control owns job selection,
authorization and worker lifecycle; DSP owns copying/ring production; UI owns
shared rack/browser panels, real lamps/status and explicit capture actions.

## Acceptance and attribution

1. Native UI/MCP share truthful Bridge status, save state and control authority;
   pairing/lease differences are declared and import creates no network effects.
2. Listen passes empty/mono/16-channel raw audio and bypass; real armed capture
   reports sums, overflow, unavailable/stalled/deleted/suspended lifecycle honestly.
3. One musical excerpt supports edit/history/save/reopen and observed live capture;
   full-fixture source preservation and unresolved boundaries remain explicit.
4. Keep macOS ARM validation focused; installed build and frozen proofs stay stable.

RackMCP is by Zachary Scheffler (`R/plugin.json`), GPL-3.0-or-later (`R/LICENSE`).
Any source adaptations must retain attribution/provenance and follow the project's
GPL distribution notices; original OpenPatch code remains MIT. This audit copies
no implementation and establishes neither Rack protocol nor whole-patch parity.
