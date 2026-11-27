# Cairn repository guide

Cairn is a Rust CLI plus an agent skill. It reads local coding-agent transcripts, normalizes them into SQLite, and exposes search, bounded reads, and read-only SQL. The source transcripts are never modified.

## First look

1. Read [README](README.md) for the user-facing entry point.
2. Read [docs/README.md](docs/README.md) for the documentation map.
3. Read [src/cli.rs](src/cli.rs) for command dispatch.
4. Read [src/providers/mod.rs](src/providers/mod.rs) and [src/providers/base.rs](src/providers/base.rs) before touching an adapter.
5. Read [src/indexer.rs](src/indexer.rs), [src/store.rs](src/store.rs), [src/search.rs](src/search.rs), and [src/sqlguard.rs](src/sqlguard.rs) for the data path.
6. Read [tests/integration.rs](tests/integration.rs) and `tests/fixtures/` for the expected behavior.

## Invariants

- Keep provider data read-only. Never write to a provider directory, transcript, or database.
- Normalize through `SessionBuilder`; keep session and message IDs stable.
- Search has three layers: trigram, jieba words, and `LIKE` fallback. Preserve all three when changing search.
- Default search covers `content_type = 'text'` and `is_meta = 0`. `--include-meta` is the explicit escape hatch for thinking, tool results, and tool-use rows.
- Keep default output actionable but bounded: grouped search includes the best message UUID so `show --around` needs no extra search; `show` returns visible text; `--detail` adds nonessential fields, and `--include-meta`/`--tools` add rows.
- `--type` and `--tool` are targeted row filters. They imply the matching hidden rows and must stay independent from `--detail`, which only adds fields.
- `cairn sql` must stay read-only: one `SELECT`/`WITH`, authorizer, deadline, row cap.
- Update checks are cached for 24 hours and write only to the state file; never block or corrupt JSON output.
- The skill is installed by `npx skills` from `skills/cairn-session-search/`. Do not add a `cairn install` command or per-agent install paths.
- A published version and tag are immutable. Fix a public release by publishing a new revision.
- Every release needs a written release note in `docs/releases/vVERSION.md` and on the GitHub release. See [docs/release.md](docs/release.md).

## Commands

```sh
cargo fmt --check
cargo test --locked
cargo build --locked --release
```

Verify skill discovery with:

```sh
npx skills add . --list
```

CI runs formatting, tests, and skill discovery. Clippy is advisory; it is not currently a CI gate.

## Change map

| Change | Start here |
|---|---|
| CLI command or output | `src/cli.rs` |
| Provider discovery or parsing | `src/providers/`, then `src/providers/mod.rs` |
| Incremental indexing | `src/indexer.rs`, `src/store.rs` |
| Schema or FTS | `src/store.rs`, then `docs/architecture.md` |
| Search/ranking | `src/search.rs`, `tests/integration.rs`, `docs/search.md` |
| SQL policy | `src/sqlguard.rs` |
| Version/update check | `src/update.rs`, `docs/cli.md` |
| Skill behavior | `skills/cairn-session-search/` |
| Installers or packaging | `scripts/`, `Formula/`, `bucket/`, `docs/install.md` |
| Release workflow | `.github/workflows/release.yml`, `docs/release.md` |
| CI | `.github/workflows/ci.yml` |

## Add a provider

Follow the checklist in [docs/development.md](docs/development.md#add-a-provider). At minimum:

1. Add `src/providers/<name>.rs` with `discover` and `parse`.
2. Register the format and source ID in `src/providers/mod.rs`.
3. Add a temp-database or fixture test in `tests/integration.rs`.
4. Update `docs/sources.md` and the source list in `skills/cairn-session-search/SKILL.md`.
5. Run `cargo test --locked` and `npx skills add . --list`.

## When behavior changes

A behavior change is not complete until every user-facing description matches it. Update code, tests, docs, and skill text in the same change; do not leave documentation for a follow-up.

Before editing, identify the old wording and all places that describe the behavior:

```sh
rg -n -- '--old-flag|cairn old-command|old/path|old default' README.md AGENTS.md docs skills
```

Check at least:

- README: install/skill commands, quick start, capability summary, performance claims.
- `docs/cli.md`: commands, flags, defaults, and output shapes.
- `docs/search.md`: search, filter, and hidden-row semantics.
- `docs/install.md`: install, upgrade, update-check, and uninstall behavior.
- `docs/sources.md`: provider coverage and root resolution.
- `docs/architecture.md`: data flow, schema, SQL guard, and update state.
- `docs/development.md`: test matrix and provider checklist.
- `docs/release.md` and `docs/releases/`: release process and the current release note.
- `skills/cairn-session-search/SKILL.md` and `references/`: agent-facing commands, filters, and evidence rules.
- `AGENTS.md`: invariants and change map when the rule itself changes.

After editing, verify relative links, run `cargo test --locked`, and run `npx skills add . --list`. If a behavior changed after a release, publish a new revision; do not rewrite historical release notes.

## Documentation ownership

- README: identity, first-run path, critical boundaries.
- `docs/sources.md`: provider roots and environment variables.
- `docs/cli.md`: commands and flags.
- `docs/search.md`: search semantics and limits.
- `docs/architecture.md`: data model, indexing, SQL guard.
- `docs/release.md`: versioning and release checklist.
- `skills/cairn-session-search/`: runtime instructions for the agent.

The owning document holds the full detail; other documents keep only a summary and link. Do not duplicate mutable behavior in several places.
