## Agent skills

### Issue tracker

Issues and specs live in GitHub Issues on seelowen-maito/loft, via the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

The five default labels, each named after its role (needs-triage, needs-info, ready-for-agent, ready-for-human, wontfix). See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: `GLOSSARY.md` and `docs/adr/` at the repo root. See `docs/agents/domain.md`.

### Sweep rules

What the workspace sweep and Clean count as build output is one table in `backend/src/sweep.rs`. To teach it a new stack, see `docs/sweep-rules.md`.

### Agents' workspaces

They live in `~/.local/share/loft/<checkout>-<hash>/agent-local-storage/`, outside any repo, never in the checkout (`backend/src/storage.rs`, ADR 0005). Code finds them through `cfg.storage_dir`, never from the checkout's path.
