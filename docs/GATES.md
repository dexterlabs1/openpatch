# Gates: edges, flip-flop, timing, and delay

`gates` provides six related gate-processing outputs in one native module. It is an original Rust functional implementation, guided by [VCV Gates](https://vcvrack.com/Free#Gates), rather than a circuit model or a sample-identical port.

Choose **File → Delayed gate voice** for a connected stereo example. The clock drives Gates; its timed GATE plays the left envelope and DELAY plays the right. Turn LENGTH to change the interval, or press RESET to clear the timing state. The patch is [delayed-gate-voice.json](../examples/delayed-gate-voice.json).

## Controls and signals

| Control or port | Behavior |
|---|---|
| LENGTH | Saved duration in seconds, 1 ms–10 s; default 100 ms; logarithmic knob and randomization |
| LENGTH CV | Multiply the duration by `2^CV`; evaluated on every active sample |
| IN | Gate signal; rises at 2 V and falls at 0.1 V, retaining state between thresholds |
| RESET | Rising CV resets its own lane; the panel button resets all active lanes |
| RISE / FALL | Separate 10 V, 1 ms pulses for input transitions |
| FLIP / FLOP | Complementary 0/10 V states; every input rise toggles FLOP; reset restores FLIP |
| GATE | Retriggerable 10 V timed gate; falling IN does not shorten it |
| DELAY | The input gate's historical state at the current duration in the past |

IN determines 1–16 output channels, with one channel when disconnected. A mono LENGTH CV broadcasts; short polyphonic LENGTH inputs zero-pad. RESET is channel-local and does **not** broadcast a mono reset across voices. A physical RESET press affects active voices only.

IN starts low, so an initially high input produces a rise. RESET CV starts uninitialized: an initially high reset arms the detector, and a later low-to-high transition resets. Nonfinite signal voltages become zero. The reference behavioral source is [Fundamental Gates at 10dd016](https://github.com/VCVRack/Fundamental/blob/10dd0160c664770910e5584b7b00498cc48d9ddd/src/Gates.cpp); implementation code and artwork are not copied.

## Timing and bounded history

OpenPatch calculates duration in f64 from the validated f32 seconds parameter. LENGTH CV is clamped to −32…+32 after nonfinite values become zero. These numerical bounds, integer timing, and the seconds parameter are explicit OpenPatch policies.

RISE and FALL pulses last `ceil(sample_rate/1000)` active samples, including the edge sample. GATE increments its sample age before testing expiry. It is high while that age is strictly below `duration × sample_rate`; a duration shorter than one sample can therefore expire on its triggering sample. Changing LENGTH can shorten or extend an active gate, but cannot revive an expired gate. A new IN rise restarts it.

DELAY keeps at most 1,023 transitions per voice in a circular buffer allocated when the graph compiles. On the current targets, transition storage is 261,888 bytes per Gates module, reserved for all sixteen lanes before audio processing. Lookup uses binary search and does not consume entries: increasing LENGTH can revisit retained history. Capacity eviction drops the oldest entry. If no retained transition is old enough, DELAY is low. This bounds event count rather than guaranteeing a fixed number of seconds at every event rate.

Each sample evaluates IN transitions, then reset, then outputs and delay lookup, then inserts any current IN transition. Reset clears history before lookup; a coincident IN transition is recorded afterward. Even a sub-sample delay takes at least one sample. Ordinary cables add their usual one-sample delay.

History is recorded even when DELAY is unconnected, unlike the reference's connection-dependent recording. A module sample clock advances while its active lanes process. Dormant lanes retain their detector, pulse, flip/flop, and gate-age state; their recorded timestamps continue aging. Bypass silences all outputs and freezes the whole module. Timestamp rebasing prevents counter overflow while preserving ordering and relevant ages.

## Editing and persistence

- **Panel RESET:** clears FLOP, the timed gate, and delay history. It preserves input detector state and any RISE/FALL pulse tails.
- **Initialize:** restores the LENGTH default and releases manual controls, preserving DSP history.
- **Compatible graph edits:** preserve timing and recorded transitions while adopting the new LENGTH. Module movement and undoable parameter edits use this path.
- **Fresh load, duplicate, changed sample rate, or new module identity:** starts new runtime state. Presets/projects save LENGTH and bypass, not flip/flop, pulse tails, held buttons, or delay history.

Releasing RESET clears its physical held state without resetting again. Pointer exit, focus loss, dialogs, bypass, removal, and queue-overload safety release prevent a stuck button. A fast click retains its reset event even when both edges arrive in one GUI frame.

Six output indicators use separate mono/poly colors. They update every 32 samples with immediate rises and decaying brightness. Their fading tails are not a claim that the corresponding output remains high.

## MCP and verification

`reset_press` and `reset_release` work with live `trigger_module` and sample-timed offline `module_events`. They are distinct from SEQ-3's `reset` action. Releases remain accepted while bypassed. Queue receipts report acceptance; `inspect_performance` reports callback-applied controls and lights. Gates has no persistent custom-state flag.

The validation suite covers thresholds, startup, coincident reset/edges, retriggering, variable duration, channel policies, delayed lookup and capacity eviction, dormant voices, timestamp rebasing, adoption, initialization, allocation-free processing, manual control safety, and all nine native jack targets. Direct differential traces against the VCV executable and macOS hardware playback remain separate qualification work.

`cargo run --release -p openpatch-core --example gates_profile` measures a three-node source/Gates graph at one and sixteen voices, plus compatible adoption. It warms the delay history before measurement. Results include source oscillator and routing costs and do not establish a performance advantage over Rack.
