# Branch and integration status

Audited 2026-10-18 against `main` at `b1e84a6`. OpenPatch 0.3.0 build 10 is
published and installed on the user's Mac. This audit changes no application
code or installed files.

## Ready and delivered

All completed feature lines are already merged. The old `work/wordpatch` tip
`9518256` is patch-equivalent to merged `165a03c`; its README/updater/site changes
are also delivered through the newer Patch Engine line. Its unmerged hosting
ancestry is preserved in the Rack-host checkpoint below. The old release
checkpoint contains only a superseded workflow variant, not another feature.

Removed **38 stale remote branches** and **61 stale local branches** after backup.
Merged histories remain reachable from `main`; both historical Wordpatch tips
and the old workflow checkpoint also remain in the recovery bundle. Clean
historical worktrees were detached from deleted branches. Their files and ignored
source-reference/build evidence were retained in OpenPatch Archive.

## Unfinished product work

| Work | Review location | Remaining merge requirements |
| --- | --- | --- |
| Recall Random | [Draft #37](https://github.com/dexterlabs1/openpatch/pull/37), `work/recall-random` | Native 32 kHz/five-frame bridge, explicit latency and state adoption, six weighted scale slots, revision-fenced capture, shared panel/telemetry/UI/MCP/history/preset/reopen and callback no-heap checks, then macOS milestone CI. |
| Existing-binary Rack host | [Draft #38](https://github.com/dexterlabs1/openpatch/pull/38), `work/rack-binary-feasibility` | Current main integrated; explicit experimental activation/shared controls, coherent MCP save dispatch and ordinary graph/Scope continuity implemented. Focused host regressions, actual public workflow and the bounded 60-second MacBook Pro Speakers / 48 kHz / 128-frame device gate pass. Native-window and final milestone CI review remain in progress. Wider binary/GUI compatibility and redistribution remain separate. |

Recall Random is synchronized with the audited `main`. Its bounded input check
accepts 26 cases and three source-negative cases; this audit did not rerun DSP
qualification. It is not yet a registered module. The draft's automatic full CI
was canceled because native integration is incomplete; rerun the milestone gate
when it is ready. The Rack-host draft intentionally preserves its historical
checkpoint rather than guessing at unsafe conflict resolutions. Neither draft
is a release candidate.

The remaining fifteen creative instruments and the larger parity roadmap still
apply. MIDI/live performance remain last; routine validation remains macOS first.

## Protected local work

Three dirty checkouts were left untouched, including their original indexes:

| Checkout | Protected content | Recovery snapshot |
| --- | --- | --- |
| `~/Projects/openpatch` (`wip/legacy-local-workspace`) | Old local DSP/vendor/panel experiments and documentation edits. The checkout's base predates the released integration; do not reset, pull or blindly merge these changes. Compare individual behaviors against current main first. | `archive/local-legacy-main-20261018-195906` |
| `OpenPatch Archive/openpatch-mcp-layout` | Old uncommitted row-placement implementation. Current main already contains the placement API and broader lifecycle tests; compare before treating any remainder as new work. | `archive/local-mcp-layout-20261018-195906` |
| `OpenPatch Archive/openpatch-stock-scope` | Historical installation-doc edits and untracked reference harness. Current main already ships the qualified Stock Scope harness; preserve the older proof material without regressing current documentation. | `archive/local-stock-scope-20261018-195906` |

Claude's `work/patch-engine` checkout/branch remains available even though its
committed work is already merged. `gh-pages` remains a separate deployment
branch. Neither was detached or overwritten. The dirty legacy checkout was moved to the protected local branch
`wip/legacy-local-workspace`, without modifying its files or index. Its inherited
`origin/main` upstream was removed to prevent an accidental pull into old work.
Local `main` is synchronized with `origin/main`. Start new work in the clean
`~/Projects/openpatch-stock-scope-integration` checkout; the legacy workspace
remains a recovery/triage area.

## Recovery and future cleanup

A verified complete Git bundle, branch/worktree/PR inventories and exact commits
containing all tracked and untracked nonignored changes are stored locally at:

`~/Projects/OpenPatch Archive/repository-backups/20261018-195906/`

The bundle is `openpatch-before-cleanup.bundle`; `dirty-snapshots.json` maps each
protected workspace to its exact parent, snapshot and original status. For
recovery, use `git bundle verify <bundle>` and `git clone <bundle> <new-folder>`.
Ignored proof/build artifacts remain in their original archived checkouts and
are not represented by a Git bundle. No hard reset, clean, force push, user-patch
removal or forced app quit was used.

For the next milestone, keep one branch per coherent feature, use a draft PR
with explicit completion checks, and delete its branch after merge. Do not
recreate a branch for a feature already delivered by an equivalent commit.
