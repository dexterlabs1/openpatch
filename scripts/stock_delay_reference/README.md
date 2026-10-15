# Free Delay source qualification

`acceptance.json` was frozen before the first Rust comparison. Its SHA256 is
`4e274862e9e8effa4a177eeb4d737005fd25a07e66d272dd4d5833cc52e36e50`;
the runner refuses a modified declaration. Bounds apply from frame zero with
identical frame indices and no alignment or startup exclusion.

The C++ oracle downloads the immutable Fundamental commit
`10dd0160c664770910e5584b7b00498cc48d9ddd`, retains the actual `Delay` class
prefix, replaces only its `plugin.hpp` include with `rack.hpp` plus the Rack
namespace, and omits `DelayWidget` and plugin registration. It compiles against
the supplied SDK headers and `libRack`, including the genuine
`src_new(SRC_SINC_FASTEST, 1, ...)` converter. The SDK SRC version must identify
itself as 0.1.9. Its immutable upstream reference is libsamplerate commit
`f6730d03c3e7660bb6ecad8816f1b09c5825142a`.

The Rust probe includes production `stock_delay.rs` directly, with minimal
`Node`, parameter validation and `PortFrame` host adapters. It applies validated
parameter-only updates in place and rebuilds/adopts at explicit adoption
markers. This adapter amendment was requested by the owning agent after the
acceptance declaration and before execution; the immutable acceptance lifecycle
sentence describes the earlier adapter. Inputs, numerical bounds and explicit
adoption markers are unchanged. The C++ class continues without a reset. Every visible output must
remain mono, finite, and within the fixed pointwise voltage bound; the Rust
probe separately asserts unused output storage is zero.

Before any full Delay comparison, six direct SRC scenarios check per-call
input consumption, generated counts and each output sample wherever the actual
C source has defined history reads. Each scenario
starts a new converter and preserves it across 8,192 calls. Calls use no more
than 16 input/output frames and always use `end_of_input=false`. Inputs are
deterministic arbitrary buffers; they deliberately do not reoffer unconsumed
data. This exercises the API state independently of the history FIFO.

Before passing an artificial stream to the SDK, the runner compiles the actual
pinned C 0.1.9 sources in private temporary storage. It injects one bounds-check
helper at the two mono sinc history reads, preserving all index calculations
and control flow. A read outside the allocated history exits with status 87
before evaluating the invalid read. Such a scenario is recorded as
`source_undefined` with `fidelity_passed: null`, and receives a complete native
finite-output/count-bound safety check instead. It receives no SDK fidelity
pass. Defined scenarios still require the unchanged exact counts and sample
bounds against genuine SDK symbols. Whole Delay comparisons run when all
defined comparisons and native safety checks pass. This preflight is specific
to mono history bounds and is not a general C undefined-behavior proof.

An additional analytic extension scenario starts with a ratio-1 call followed
by ratio-.25 calls, with 16 input/output frames each. Before execution, source
analysis predicted call index 5, output offset 10, current index 35, and a
history read at -1. The runner verifies that independent C guard event. This
does not claim that the same sequence is reachable through Delay. The report
preserves exact guarded source/configuration text and hashes. Frozen
acceptance and its numerical bounds remain unchanged.

Every whole Delay trace also runs through the actual class prefix with that
same guarded C converter before genuine SDK numerical execution. All public
SRC and converter symbols are privately renamed while compiling the C objects
and the actual Delay adapter, so this does not rely on shared-library symbol
interposition. `libRack` continues to supply the actual Module/engine support.
The driver labels any invalid source read with the exact audio frame and SRC
call. If one occurs, that specific case becomes
`source_undefined_whole_delay`, receives `fidelity_passed: null`, skips unsafe
SDK execution, and still runs the entire native mono/finite/padding safety
trace. Every other case proceeds independently and every source-defined SDK
comparison retains its fixed pointwise bound. This classification is declared
before observing numerical comparison failures; no case name is excluded in
advance.

Production file hashes are recorded both before compilation and after the
run. If any change, the report sets
`final_rerun_required_due_to_production_changes: true`, cannot pass, and cannot
claim a completed full matrix. Source-undefined cases are listed separately;
the aggregate `passed` field applies only to the explicit `passed_scope` and
does not grant those cases an SDK fidelity pass.

Run only when compiler/oracle execution is authorized:

```sh
python3 scripts/stock_delay_reference/run.py \
  --serde-rlib /absolute/path/to/already/built/libserde_json-HASH.rlib
```

The runner never invokes Cargo, a Rack plugin/runtime, a GUI or an audio device.
It compiles temporary standalone probes with `c++` and `rustc`. `--case` and
`--rate` restrict diagnostic runs; such a report cannot claim `full_matrix`.
`--resampler-only` runs the direct SRC stage alone. `--retain-traces DIR` retains
the exact binary inputs and SDK/native results, including SHA256 hashes in the
JSON report. Without it, traces are removed after each comparison.

The report records upstream pins and source hashes, the compiled class-prefix
hash, SDK header/library hashes, SRC identity, production and driver hashes,
the serde dependency hash, compiler identities/flags, platform, case input and
output hashes, worst samples, and the first failures. It also observes actual
SDK `Module::processBypass` widths and raw samples for audio widths 0/1/5/16;
this is a source observation, not a host bypass qualification. Default output is
`docs/benchmarks/stock-delay-reference.json`.

The suite includes a 12-second maximum-time impulse at 44.1/48/96 kHz, all tone
and feedback endpoints, signed CV depths, ignored CV at zero depth, abrupt
delay jumps, patched clock startup before any edge, clock threshold behavior,
tempo changes and unpatch/reconnect, audio polyphonic summation with auxiliary
channel-zero behavior, and native parameter/adoption continuity. The
2,200,000-frame 44.1 kHz case advances beyond the `2^21` history ring position
with a short delay. This tests physical ring wrapping; it does not claim the
queue fills at a supported ten-second delay.

Host graph/cable timing, public validation, bypass, initialize/reset,
sample-rate changes and clock-light rendering require separate host checks.
This audio adapter does not qualify those paths.

Binary protocols are little endian. Delay input frames are `u32 adoption`,
eight `f32` raw parameters in source order, then six ports in source order:
each port is `u32 width` followed by `width` `f32` samples. Delay output frames
are two `u32` widths and two `f32` voltages (mix, wet). SRC input calls are
`u32 input_frames`, `u32 output_capacity`, `f64 ratio`, and the input `f32`
samples. SRC results are `u32 used`, `u32 generated`, and the generated `f32`
samples.
