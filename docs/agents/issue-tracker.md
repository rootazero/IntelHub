# Issue tracker: GitHub

Issues and PRDs for this repo live as GitHub issues on https://github.com/rootazero/IntelHub. Use the `gh` CLI for all operations.

## Conventions

- **Create an issue**: `gh issue create --title "..." --body "..."`. Use a heredoc for multi-line bodies.
- **Read an issue**: `gh issue view <number> --comments`, filtering comments by `jq` and also fetching labels.
- **List issues**: `gh issue list --state open --json number,title,body,labels,comments --jq '[.[] | {number, title, body, labels: [.labels[].name], comments: [.comments[].body]}]'` with appropriate `--label` and `--state` filters.
- **Comment on an issue**: `gh issue comment <number> --body "..."`
- **Apply / remove labels**: `gh issue edit <number> --add-label "..."` / `--remove-label "..."`
- **Close**: `gh issue close <number> --comment "..."`

Infer the repo from `git remote -v` — `gh` does this automatically when run inside a clone.

## When a skill says "publish to the issue tracker"

Create a GitHub issue.

## When a skill says "fetch the relevant ticket"

Run `gh issue view <number> --comments`.

## Closed phases

- **GEV P12 (flight layer parity)** — closed 2026-09-21, branch `feat/p12-flight-layer-parity` merged `--no-ff` to main. Deliverables: hub-core `gev_enrichment.rs` + `gev_tracks.rs` (4 routes), console `aircraft-source.ts` adapter, cockpit 5-module polish, acceptance sp6+2 / sp8+5 / probe P12_PROBES. 582 tests green, 410 acceptance all green. Ledger: `docs/superpowers/execution/2026-09-21-gev-p12-ledger.md`.