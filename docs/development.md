# Development

## Requirements

- Rust 1.89 or newer
- Node.js 22.20 or newer only for the `npx skills` discovery check

SQLite is bundled through `rusqlite`; no system SQLite is required.

## Build and test

```sh
cargo build --locked
cargo test --locked
cargo fmt --check
cargo build --locked --release
```

`cargo clippy --all-targets --all-features` is useful for inspection. CI currently gates formatting and tests, not clippy warnings.

The release binary is `target/release/cairn`.

## Layout

| Path | Responsibility |
|---|---|
| `src/cli.rs` | CLI and output |
| `src/indexer.rs` | Discovery, incremental builds, pruning |
| `src/providers/` | Per-agent adapters |
| `src/store.rs` | Schema, writes, FTS |
| `src/search.rs` | Search and ranking |
| `src/query.rs` | Session and message reads |
| `src/sqlguard.rs` | Read-only SQL |
| `tests/integration.rs` | End-to-end index/search/show/SQL tests |
| `tests/fixtures/` | Claude, Codex, and pi fixtures |
| `skills/cairn/` | Agent skill source |
| `docs/` | User and maintainer documentation |

## Add a provider

1. Read `src/providers/base.rs` and a similar adapter.
2. Add `src/providers/<name>.rs` with `discover(root)` and `parse(path, source)`.
3. Normalize through `SessionBuilder`; do not write provider files.
4. Register the format and source ID in `src/providers/mod.rs`:
   - `Format`
   - `source_format`
   - `source_names`
   - `default_roots`
   - `discover`
   - `parse_file`
5. If the source is SQLite-backed, add it to the non-append branch in `src/indexer.rs`.
6. Add an integration test using a temp database or fixture.
7. Update [Sources](sources.md) and the source list in [`skills/cairn/SKILL.md`](../skills/cairn/SKILL.md).
8. Run `cargo test --locked` and the skill discovery check.

Provider tests should cover text, tool calls, tool results, timestamps, project path, and usage when the format exposes them.

## Change search carefully

Search has three layers and different ranking paths. A change should be tested with:

- English words and case differences
- Chinese two-character and longer phrases
- code identifiers and paths
- `--include-meta` versus default text-only search
- session grouping versus `--all`

Add cases to `tests/integration.rs` rather than creating a parallel test harness.

## Docs

- Keep the README as the entry point.
- Put provider/root details in `docs/sources.md`.
- Put command/flag behavior in `docs/cli.md`.
- Put search semantics in `docs/search.md`.
- Put internal design and schema ownership in `docs/architecture.md`.
- Put release steps and release-note policy in `docs/release.md`.
- Keep `AGENTS.md` as the short repository index for agents.

When behavior changes, update its owning document and remove stale summaries elsewhere.

## CI

`.github/workflows/ci.yml` runs on pushes and pull requests:

- `cargo fmt --check`
- `cargo test --locked`
- `npx skills add . --list` to verify the skill layout is discoverable

The release workflow is separate and only runs on manual dispatch. See [Release](release.md).
