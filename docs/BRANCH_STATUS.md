# Branch and integration status

Updated 2026-10-18 after PRs #37 and #38 merged. OpenPatch 0.3.0 build 10 is
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

## Finished product work

| Work | Merged review | Validation |
| --- | --- | --- |
| Recall Random | [#37](https://github.com/dexterlabs1/openpatch/pull/37), `e6b6164` | Native Engine, weighted scales/capture, shared UI/MCP/history and callback no-heap checks; genuine source comparison; [complete macOS gate](https://github.com/dexterlabs1/openpatch/actions/runs/39191841453). |
| Experimental Rack binary host | [#38](https://github.com/dexterlabs1/openpatch/pull/38), `b0ce1fe` | Shared activation/controls/save, native/Scope continuity, native-window review, final physical Mac device test and [complete macOS gate](https://github.com/dexterlabs1/openpatch/actions/runs/39193043027). One pinned OFFSET and fixed audio profile; wider compatibility remains separate. |

Both finished feature branches are deleted after merge. Their local worktrees
and ignored validation evidence are retained. Source is integrated without
changing the installed 0.3.0 build or published downloads. Fourteen further
creative instruments and the larger parity roadmap remain; MIDI/live
performance stay last.

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
