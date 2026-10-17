# Fundamental 2.6.4 voice reference

This suite compares `rack264_vco` and `rack264_vcf` with the actual Fundamental
classes at `e0ade9a23a9b41124ea3f4ec9aa2cb0b98c2529e`. These are separate model
identities; the newer `stock_vco` and `stock_vcf` and their frozen gates are unchanged.

The C++ adapters include the unchanged upstream class prefix, replacing only
`plugin.hpp` with the genuine Rack SDK headers and omitting widgets. The Rust
adapters link the actual compiled `openpatch_core` library. They do not copy DSP
or substitute host types. Parameter edits rebuild/adopt production Rust runtime;
the C++ instance receives the corresponding controls in place.

## Reproduce on macOS ARM

Use the genuine Rack SDK **2.6.6**, Rust/Cargo, Python 3 and Xcode command-line
tools. Fetch the pinned upstream `src/VCO.cpp` and `src/VCF.cpp` into
`native-test-output/rack-264-voice/reference/`; the runner verifies their SHA-256
before compiling. The pin and exact source hashes are in the acceptance files.
The source and complete GPL notices remain attributed in the repository.

```sh
python3 scripts/rack264_voice_reference/run.py --module vco \
  --target native-test-output/rack264-build \
  --output native-test-output/rack-264-voice/vco-local-001
python3 scripts/rack264_voice_reference/run.py --module vcf \
  --target native-test-output/rack264-build \
  --output native-test-output/rack-264-voice/vcf-local-001
```

Pass `--sdk /absolute/path/to/Rack-SDK` if needed. Output directories must be new.
`--case` and `--rate` select a subset for diagnosis. Root runs Cargo and numerical
comparisons serially. No GUI, audio device, package or installed app is opened.

The runner freezes its declaration and driver before any numerical comparison,
then retains raw traces, diagnostics and a report. It records source, adapters,
SDK library/header closure, production source manifest, compiler profile and
binaries. VCO's original 42-case declaration remains in its recorded attempt;
the separately declared slow-cycle extension adds three cases without changing
those fixtures or their tolerances. Neither existing Stock oracle is modified.

## Protocol and limits

Each line contains 77 finite numbers: action, observe flag, connected-output
mask, six controls, then four ports (channel count and 16 voltages each).
Hidden input storage must be positive zero. Actions are step, runtime-state
Initialize, and same-rate host rebuild/adopt. Host control-default restoration
is separate; supplied controls are applied before each action.

VCO returns width and 64 canonical voltages. Observations also include all
16 lane histories, logical correction tails, ring positions, wave values and
lamp/divider state. VCF returns width, 32 canonical voltages and **two exact u64
RNG words**; observations add all 16 lanes' four integrator states and previous
input. Inactive/disconnected published voltages are zero; raw disconnected SDK
storage is outside the claim.

VCF arguments additionally supply two RNG seed words and an initial previous
input value. Its upstream constructor leaves that history uninitialized. Both
adapters explicitly assign all 16 histories before processing and report the
assignment and seeded state. This establishes a controlled comparison, **not
equivalence to undefined C++ cold-start behavior**. Rust deliberately starts
fresh modules at zero and uses a module-local stream rather than Rack's shared
process-global RNG. Reset preserves that stream and previous input.

`acceptance.json` and `vcf_acceptance.json` declare the cases and tolerances.
Driven samples are compared from frame zero, with no alignment or startup
exemption. VCF autonomous self-oscillation additionally has fixed startup,
late RMS/peak and tuning metrics; later accumulated phase is not a declared
pointwise contract. The VCO's safe definition of an SDK interpolation endpoint
does not establish equivalence to the SDK's undefined one-past-end access.

`sdk_vcf_load.cpp` separately audits genuine registry resolution,
`Module::fromJson`, parameter defaults and ResetEvent ordering. Import must
distinguish absent `params` (fresh CV depths zero) from a present array missing
the newer depth entries (legacy load depths one).

These finite fixtures do not establish physical hardware accuracy, third-party
plugin/binary compatibility, device deadlines or full Rack parity. Actual Core
voice, persistence/adoption and callback-allocation checks live in
`crates/openpatch-core/tests/rack264_voice.rs`. Shared panels and source-preserving
import workflows need their own application checks.
