# Architecture

Cairn is a read-only projection of provider transcripts into one SQLite database. It does not modify, resume, or proxy the source sessions.

```text
provider transcripts (JSONL / SQLite / JSON)
        │
        ▼
  src/providers/*  ── normalize ──▶  ParsedSession
        │
        ▼
  src/indexer.rs  ── changed files in parallel, writes serially
        │
        ▼
  ~/.cairn/cairn.sqlite
    sessions · messages · tool_calls · tool_results
    messages_fts (jieba words) · messages_trigram (raw text)
        │
        ├── search / show / sessions / overview
        └── sql  (read-only authorizer + deadline + row cap)
```

## Module map

| Path | Responsibility |
|---|---|
| `src/cli.rs` | Argument parsing, command dispatch, human/JSON rendering |
| `src/providers/mod.rs` | Provider registry, roots, format dispatch |
| `src/providers/*.rs` | Per-provider discovery and normalization |
| `src/indexer.rs` | Change detection, parallel parsing, transaction, pruning |
| `src/store.rs` | Schema, writes, session refresh, FTS maintenance |
| `src/search.rs` | Three-layer search and ranking |
| `src/query.rs` | Overview, session list, session/message reads |
| `src/sqlguard.rs` | Read-only SQL policy and execution |
| `src/update.rs` | Release check, cached update state, version comparison |
| `src/model.rs` | Normalized session/message/tool records |
| `src/util.rs` | IDs, timestamps, clipping, text extraction |

## Data model

The index has five main tables:

- `sessions`: stable session ID, source, title, project, time bounds, model, message count, source path.
- `messages`: stable message UUID, session, sequence, role, content type, text, token counts, meta flag.
- `tool_calls`: tool name, input JSON, optional file path, timestamp.
- `tool_results`: output, error flag, timestamp.
- `source_files`: file path, source, mtime, size, prefix hash, last index time.

`index_meta` stores the schema version and FTS capability flag.

Two FTS5 tables shadow `messages`:

- `messages_fts` indexes `messages.text_words`, produced by jieba search-mode segmentation, with `unicode61`.
- `messages_trigram` indexes raw `messages.text` with the `trigram` tokenizer.

Both are external-content FTS5 tables kept in sync by triggers on insert, update, and delete.

`cairn schema` prints the authoritative DDL. The agent skill keeps a readable copy in [`skills/cairn-session-search/references/schema.md`](../skills/cairn-session-search/references/schema.md).

## Session and message identity

- Claude keeps the provider UUID as its session ID.
- Other sources use a `source:` prefix, such as `codex:...` or `dimagent:...`.
- If two source files claim the same ID, Cairn appends `#<hash>` derived from the source path.
- Message UUIDs are provider IDs when available; generated IDs are stable within a session and sequence.

Do not construct a source path from a session ID. `sessions.source_path` is the pointer back to the original file or database.

## Indexing lifecycle

1. `Store::init` creates the schema and meta rows if needed.
2. `providers::discover` expands each configured root into source files.
3. The indexer compares mtime, size, and a 64 KiB prefix hash with `source_files`.
4. Unchanged files are skipped.
5. Changed files are parsed in parallel with rayon.
6. Parsed sessions are written serially in one `BEGIN IMMEDIATE` transaction.
7. Deleted files are pruned from the index.

For append-only JSONL, a file that grew without changing its prefix is treated as an append. Existing message UUIDs are skipped and only new messages are inserted. Rewrites, truncations, and SQLite-backed sources are re-parsed as a whole path.

WAL mode, a busy timeout, and `BEGIN IMMEDIATE` make concurrent CLI invocations safe. There is no daemon and no background watcher; each query refreshes first unless `--no-refresh` is passed.

## Update check

`src/update.rs` queries the GitHub releases API at most once every 24 hours for normal commands. It stores `last_checked`, `latest_version`, and `release_url` in `~/.cairn/update-check.json` (or under `$CAIRN_HOME`). The check is best-effort: a network failure is recorded and never blocks the requested command. JSON output suppresses the stderr notice; `cairn version` performs an explicit check unless disabled.

## Search path

`src/search.rs` merges trigram hits, jieba word hits, and a `LIKE` fallback. Trigram hits have priority, then BM25 score, then recency and per-session hit count. See [Search](search.md) for the user-facing semantics.

## SQL guard

`cairn sql` accepts exactly one `SELECT` or `WITH` statement. It opens the index read-only, sets `query_only`, and installs a SQLite authorizer that:

- allows reads and the read-only PRAGMAs needed for introspection;
- denies writes, schema changes, attachments, and unsafe PRAGMAs;
- denies `load_extension`, `readfile`, `writefile`, `edit`, `fts3_tokenizer`, and `zipfile`.

A progress handler enforces a deadline (3 seconds by default), and the result is capped at 500 rows by default (2,000 maximum). This is a safety boundary for agent use, not a general SQL server.

## Limits and boundaries

- Source transcripts are never modified.
- Cloud-only agents such as Amp are not indexed; their history is server-side.
- `messages.text` is capped at 200,000 characters, tool output at 100,000, and tool input JSON at 20,000.
- Token counts are recorded only where the provider emits them; missing values stay `NULL`.
- Transcript text is untrusted data. The skill tells agents to treat instructions inside history as evidence, not commands.
- Search is literal; it is not a semantic index and does not expand synonyms.
