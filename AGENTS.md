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
- `cairn sql` must stay read-only: one `SELECT`/`WITH`, authorizer, deadline, row cap.
- The skill is installed by `npx skills` from `skills/cairn/`. Do not add a `cairn install` command or per-agent install paths.
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
| Skill behavior | `skills/cairn/` |
| Installers or packaging | `scripts/`, `Formula/`, `bucket/`, `docs/install.md` |
| Release workflow | `.github/workflows/release.yml`, `docs/release.md` |
| CI | `.github/workflows/ci.yml` |

## Add a provider

Follow the checklist in [docs/development.md](docs/development.md#add-a-provider). At minimum:

1. Add `src/providers/<name>.rs` with `discover` and `parse`.
2. Register the format and source ID in `src/providers/mod.rs`.
3. Add a temp-database or fixture test in `tests/integration.rs`.
4. Update `docs/sources.md` and the source list in `skills/cairn/SKILL.md`.
5. Run `cargo test --locked` and `npx skills add . --list`.

## Documentation ownership

- README: identity, first-run path, critical boundaries.
- `docs/sources.md`: provider roots and environment variables.
- `docs/cli.md`: commands and flags.
- `docs/search.md`: search semantics and limits.
- `docs/architecture.md`: data model, indexing, SQL guard.
- `docs/release.md`: versioning and release checklist.
- `skills/cairn/`: runtime instructions for the agent.

Update the owning document when behavior changes; do not duplicate the same detail in several places.
