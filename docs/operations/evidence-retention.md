# Evidence & Test-Artifact Retention Policy

Defines what generated evidence is tracked in git versus kept ephemeral, so the
repo neither loses representative validation proof nor accumulates artifact
sprawl.

## Categories

| Location | Tracked? | Purpose |
|---|---|---|
| `test_results/` | **No** — git-ignored (`.gitignore`) | Ephemeral local test output. Regenerated on every run; never committed. |
| `docs/evidence/<area>/` | **Selectively** (force-add) | Durable, representative proof for a milestone (live runs, soak summaries, reachability). Committed only when it backs a specific bead/report. |
| `docs/reports/` and `docs/reports/artifacts/` | **Yes** | Curated closeout reports and the specific artifacts they cite. |

## Rules

1. **`test_results/` is never committed.** It is git-ignored by design. Do not
   `git add -f` anything under it.
2. **`docs/evidence/` is opt-in, not automatic.** A run that produces evidence
   leaves it untracked by default. Commit it only when it is the durable proof
   for a bead or report, using an explicit force-add:
   ```bash
   git add -f docs/evidence/<area>/<representative-artifact>
   ```
   Commit one representative artifact set per milestone — not every soak run.
3. **Prefer summaries over raw streams.** Commit `*.meta`, summary `*.md`, and
   small CSV/JSON summaries; leave large raw logs/frame dumps untracked unless a
   report specifically cites them.
4. **Name with provenance.** Evidence dirs/files should carry the bead id and a
   date (e.g. `docs/evidence/<area>/<bead-id>-YYYYMMDD/`) so retention and
   pruning are unambiguous.
5. **Prune on supersession.** When a newer evidence set supersedes an older one
   for the same claim, remove the stale tracked copy in the same change that
   adds the replacement.

## Current untracked evidence

As of this policy's introduction, `docs/evidence/text-stream-portals/soak-*`
directories exist untracked. Under this policy they stay untracked unless a
specific bead/report needs one as durable proof, in which case force-add only
that representative set.

## Optional lint

A `just evidence-lint` recipe MAY be added to warn on stray untracked
`docs/evidence/**` artifacts older than a threshold; it is advisory, not a
blocking gate. (Follow-on, not required by this policy.)
