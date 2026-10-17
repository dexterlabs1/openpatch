# Stock Scope

`stock_scope` is the separate Rust realization of the pinned Fundamental Scope acquisition contract. The existing `scope` kind and saved patches keep their independent behavior. This is a digital source-behavior model; it is not a physical instrument or hardware timing claim.

The source model and application integration have passed their focused qualification checks. The production probe matched all 4,242 frozen genuine-Rack snapshots without discarding observations. Seven Core checks, 66 affected desktop Scope checks and the separate 46-record local safety probe passed. Complete macOS milestone CI remains pending; this is not an installed release or a physical-device qualification.

## Source controls and ports

The eight source parameters retain their order: `x_scale`, `x_offset`, `y_scale`, `y_offset`, `time`, `xy`, `threshold`, `trigger_disabled`. Source scale quantities are integer octaves from 0 to 8; X/Y offsets and trigger threshold are −10 to +10 V. A scale of 0 is 20 V/screen, divided by two for each octave. TIME is a raw log2 inverse-seconds value: its default 1 means a nominal 500 ms/screen, with configured nominal limits of 5 ms to 50 s. The next recording bin uses the ordered source fifth-order exponential and `ceil`, rather than treating the raw parameter as seconds.

Inputs are X, Y and external trigger. Outputs are independent X and Y passthroughs. The canonical raw host owns cable connection transitions, retired lanes, consumers and bypass. The source selects external trigger when that raw input has a positive channel count; physical cable presence is separately observable and is not a substitute for that source predicate. The two source lamps show XY mode and enabled triggering, and hold their last actual processed values while bypassed.

`trigger_disabled=0` enables triggering; 1 disables it. After all 256 bins complete, time mode can wait for an eligible edge. All selected trigger lanes are evaluated using sixteen LOW/HIGH/UNINITIALIZED Schmitt states. A first high arms a detector. An eligible edge resets the recording indices and all detectors before including that frame in the new capture. XY or disabled triggering restarts on the next process call without an edge.

## Acquisition and lifecycle

One authoritative completed store contains 256 × 2 × 16 minimum/maximum points. Thirty-two private partial points accumulate the current bin. Changing voice width does not manufacture zero samples or erase inactive completed histories. The common display snapshot borrows those completed arrays; no second acquisition Scope or reconstructed waveform supplies the source state.

Initialize applies the actual eight parameter defaults and the source Reset callback. That callback clears completed points only: partial extrema, recording indices, retained channel counts, detector states and lamp values survive. Fresh recreation is cold. Compatible same-incarnation parameter or route adoption retains acquisition while receiving controls win. A bounded in-place rate event changes actual Engine timing and the next bin width without clearing history; an ordinary graph rebuild at a different rate keeps the existing host policy for other modules. No source bypass routes exist: capture and lamps freeze while host outputs follow their bypass lifecycle.

Completed history can span different TIME settings, sample rates or resets. Nominal screen duration and the next bin width are metadata about current acquisition settings, not measured timestamps for every stored bin.

## Native view and MCP

Rack and browser use the same Signal Atlas module renderer, with larger rotary controls, real voltage/time calibration and the eight source controls. A separately labeled native AUTO/MANUAL preference fits the display without becoming a ninth source parameter or mutating capture, raw measurements, the Engine revision or Session undo history. Temporary view state is keyed by module incarnation. Before real publication, previews and new modules show unavailable capture and neutral lamps. TIME fields use nominal milliseconds per screen while storing the raw source control; successive drags remain one undoable gesture, and a stale text editor cannot overwrite a newer edit.

The live publication path carries borrowed completed bins through preallocated packets with one shared budget across both Scope kinds. MCP inspection retains exact kind, incarnation and revision checks. Stock metadata identifies raw controls and bits, actual held lamp bits, acquisition rate, nominal duration and next-bin width. Extrema remain measurements of actual completed bins, with selected-voice and all-active-voice bases; temporary UI fitting is separately identified.

## Independent reference and qualification boundary

The reference is Fundamental `src/Scope.cpp` at `10dd0160c664770910e5584b7b00498cc48d9ddd`, SHA-256 `948a5034696084bd894d256a9fdecd620799e611c689e6ae5b0e19ff0b73c54f`. The genuine Rack 2.6.6 ARM Engine owns its module, sources, sinks, raw ports, cables, Reset, bypass and live rate events. The frozen declaration includes 1,056 cases, 3,186 events, 4,242 complete snapshots and 503 hand anchors at 44.1/48/96 kHz, including a complete 50-second sweep at each rate. Every snapshot compares all 8,192 completed and 32 partial points, sixteen detector enums, parameters, lamps, counters, raw port storage and the actual cable registry. No warmup or inactive payload is dropped.

The ordinary immediate-parameter diagnostic comparison has a declared finite domain, including TIME in [−6, 8]. The public authored constructor uses configured quantity ranges and snap policy. Forty-six separate nonfinite, extreme finite and out-of-band TIME records qualify only the local Rust safety policy, not equivalence in an undefined or unobserved SDK domain. The [frozen protocol](../scripts/stock_scope_reference/README.md) and retained evidence define the exact boundary.

The upstream GPL source and separately obtained Rack SDK are behavioral/oracle references; no upstream panel artwork or source class is shipped in the application. Original Rust model, host adapter and UI are kept distinct from the separately obtained reference build. Source equivalence does not establish official distributed plugin-binary identity, full upstream GUI/statistics behavior, VCV patch import, physical audio-device timing or complete host/49-module parity.

## Application evidence

- Seven focused Core tests cover authored quantities, independent polyphonic ports, acquisition adoption, source initialize/recreation, bypass, bounded live rate changes and heap-free processing.
- The 66 affected desktop checks include real Callback → Session inspection, shared publication limits, kind/incarnation fencing, retained acquisition across TIME/threshold/routing edits, initialize and same-ID recreation. Callback execution/adoption is checked for allocation and deallocation.
- Panel checks cover physical-unit TIME entry and keyboard bounds, successive field/rotary moves with one undo/redo, stale entry rejection, native AUTO history separation, actual weak-signal measurements, bypass-held lamps and controls clearing jack targets at reduced zoom.
- All 46 unchanged local safety inputs ran through the actual Engine: 12 finite updates accepted; 34 invalid/out-of-domain updates rejected atomically. These results carry no SDK-equivalence claim.
- Workspace Clippy with warnings denied, formatting and the 81-kind shared default rack/browser drawing check passed. [Actual panel captures](../design/reviews/stock-scope-v1/README.md) document the disconnected default view in normal and dim rooms.

The frozen numerical proof is reused because executable Core/host/probe logic is unchanged since that comparison. The current source audit permits only test changes, desktop package-version metadata and one comment/Clippy-only attribute; removing those two diagnostic lines restores the frozen model file byte for byte. Full logs, manifests, binary hashes and the explicit reuse audit are retained in the integration worktree under `native-test-output/stock-scope/`.
