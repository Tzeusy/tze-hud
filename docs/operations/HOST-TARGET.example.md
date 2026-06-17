# Operator host target — example template

The public repo is scrubbed of the real Windows host/user/key. Real values live
in a git-ignored private runbook at `docs/operations/private/tzehouse-windows.local.md`
(create it from this template; it is covered by `.gitignore`).

| Placeholder (used in tracked files) | Fill in locally |
|---|---|
| `windows-host.example` | your Windows host / tailnet node |
| `hud-user` | non-admin SSH user |
| `admin-user` | admin user |
| `hud-ssh-key` | SSH identity filename under `~/.ssh/` |

Provide real values to scripts via `WIN_HOST` / `WIN_USER` env or `--win-host` /
`--win-user` flags; never hardcode them into tracked files.
