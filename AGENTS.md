# OpenPatch agent working rules

Read this file and [docs/WORKPLAN.md](docs/WORKPLAN.md) first. Open only the source and contracts relevant to the current work item; the full roadmap is background, not a required reread each turn. User instructions override this file.

## Delivery and focus

- Keep the installed 0.2.6 build 8 stable until the user requests an upgrade or a substantial milestone is ready. Do not package, increment versions or install after every fix/module.
- Validate macOS ARM only by default. Linux/Windows and throughput benchmarks are deferred to an explicitly requested full-platform milestone. MIDI and live performance remain last.
- Work on one product milestone at a time. The user approved the ADSR/Ladder direction and resumed the goal with ‘continue’. Extend that design across the supported catalog in coherent layout families; forty-nine panels now have detailed layouts, including stock utilities, mixers, studio effects, macro/drum voices, SVF, physical resonator, multi-voice/wavetable oscillators, noise/stochastic controls, capture/sequential routing, voltage processors, both quantizers, both eight-step sequencers, Score and both Mid/Side utilities; continue with polyphonic and remaining utility families next. Preserve real units/routing, existing editing/MCP behavior and shared rack/browser rendering. Keep Stock Scope checkpointed until this design milestone is delivered; do not install a release automatically.
- Lighting should use restrained, local halos. The user rejected broad, bright neon clouds; keep readable lenses, soft falloff and visible signal differences without washing the panel in light.
- Each work item needs one user-visible outcome and a short done checklist. Avoid expanding acceptance into unrelated cleanup or exhaustive internal equivalence. Preserve existing frozen references and honest fidelity claims.
- Do not touch user patches/settings or replace a running app. Preserve existing computer-use restrictions; headless validation does not establish a native device test.

## Token and agent discipline

- Default to working solo. Use at most two workers only for clearly independent file ownership. Prefer the user's authorized lower-cost workhorse with a fresh, short task context; do not fork the entire conversation or delegate recursively.
- Use Astra once for an architecture decision or a concrete high-risk DSP/realtime review, not routine layout changes or repeated status audits. Another review needs a new substantive risk or changed design.
- Give workers the worktree, file ownership, interface, constraints and acceptance checks. Request a concise result: changed files, validation, blockers. Root owns serial builds and DSP/reference execution.
- Bound normal tool output to about 1,500 tokens. Read only changed sections and actionable diagnostics. Redirect full logs, traces and manifests to files; print selected totals, pass/fail and evidence paths. Never dump an SDK header inventory, full trace or complete source manifest into chat.
- Record state in one current work item's STATE.json under ignored native-test-output; docs/WORKPLAN.md is the compact public milestone index. Reuse evidence paths instead of copying histories into multiple state files or creating a new worktree for every small change.

## Validation proportional to the change

- Fix compiler/type errors with cargo check or the focused test target before broad suites. A useful test targets a user behavior, state/lifecycle invariant, numerical contract or realtime property; avoid tests that only repeat implementation structure.
- UI-only changes: focused headless interaction/geometry checks and review affected views. No DSP oracle rerun without relevant production DSP changes. Documentation-only changes: links/diff checks; no app build.
- DSP changes: focused numerical/lifecycle and allocation/deallocation checks, plus the relevant existing frozen reference when its production inputs change. Do not regenerate or weaken an oracle to obtain a pass.
- Run the complete macOS workspace/MCP regression once at a milestone candidate, after focused fixes settle. Repeat only checks affected by subsequent changes; preserve logs of failures.
- Reuse an unchanged built binary and its source/toolchain hashes for existing evidence. Test/doc/view changes do not justify repeating unrelated numerical comparisons. If a relevant source/toolchain changes, requalify the affected result explicitly.
- Send runtime/workflow changes through a PR; use the manual workflow to validate a direct main change. One PR CI run is sufficient; do not require both feature-branch push and PR copies. Read one capture set and inspect only actual changed views/differences. Avoid repeated full-catalog screenshot reviews for a local control change.
- Do not run throughput benchmarks on every edit or wait for deferred platforms to approve a macOS change. Measure representative patches when performance changes or at an agreed performance milestone; never claim a performance advantage without comparable measurements.

## Reporting

Report the outcome, relevant validation, remaining blocker and next action. Separate implemented, validated and installed status. Do not count catalog entries as complete VCV parity. Keep source, application, device and hardware claims distinct. Prefer a short milestone update over a chronological tool transcript.
