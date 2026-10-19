# Recall Random source checkpoint

Work in progress: source kernels are not registered as a playable module yet.
See docs/RECALL_RANDOM_CONTRACT.md. The root-owned runner fetches39 SHA-pinned
original files and compiles their genuine T/XY/ramp/quantizer/lag/resources.

Run `python3 scripts/marbles_reference/run.py --validate-only` for bounded input
checks, `--compile-only` for original/audited/Rust compilation without DSP, or
without either flag for the complete source comparison. `--extensions-only`
checks the three original undefined endpoints and explicit native corrections.

Original firmware globals are zero-initialized. One shared fallback RNG is
initialized before T then X1,X2,X3,Y. Source32kHz/5-frame blocks and actual source
call order remain intact. Inputs are authored by make_cases.py; it computes no
expected DSP. The independent C++ outputs are compared as integer float bits.

A single audited host overlay guards exact terminal interpolation: the next
value repeats the terminal knot instead of reading outside the raised-cosine
array. Every attempt retains the original header, its manifest hash, the overlay
hash/diff and a separate unmodified ASan/UBSan/bounds executable. For every case,
all complete records before an original terminal read are checked too. Any
unclassified original failure rejects the run. Positive cases explicitly record
whether the original Y-channel loader is also invoked; public LoadScale only
loadsX1–X3, and empty Y quantizers are a separately retained original negative.
No original cached source is overwritten, and no sanitizer is disabled.

Attempt004 passed26 cases/288485 observations, including all7T models, clock
sources, register/reacquisition/clone changes, six weighted scales, repeat/shuffle,
parameter endpoints, NoteFilter and ScaleRecorder.21 cases complete with the
unmodified sanitizer; five retain original terminal-read negatives.273490 total
complete original-defined observations match, including those prefixes. Full
traces match the guarded C++ and audited C++ exactly. This is bounded kernel
qualification with explicit host corrections, not whole-Marbles/hardware proof.

Attempt005 separately confirms original float-word, emptyY and terminal-lookup
failures, then verifies the Rust saturation/empty-mask/terminal-repeat policies
against independently specified endpoint values. Exact logs/binaries/input/source
copies and result.json remain under ignored native-test-output/marbles-reference.
Native bridge, weighted-content/capture authority, shared UI/MCP and the next
family platform gate are still required before counting a sixth instrument.
