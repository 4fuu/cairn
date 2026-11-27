# CLI reference

`cairn` opens the local index, refreshes it incrementally, and answers from SQLite. Every command accepts `--json` for agent use.

## Global options

| Option | Meaning |
|---|---|
| `--db PATH` | Index path. Default: `~/.cairn/cairn.sqlite`, or `$CAIRN_HOME/cairn.sqlite` |
| `--json` | Print JSON instead of terminal text |
| `--no-refresh` | Skip the incremental refresh before the command |
| `--root SOURCE=PATH` | Override a provider root. Repeatable |

Global options work before or after the subcommand.

## `cairn build`

Build or refresh the index.

| Option | Meaning |
|---|---|
| `--full` | Re-read every source instead of using mtime/size/hash checks |
| `--source S` | Only index one source ID. Repeatable |
| `--quiet` | Suppress the human-readable report |

The first build reads every configured root. Later builds skip unchanged files. SQLite-backed sources are re-read as a whole path when they change.

## `cairn status`

Print index path, session/message/tool totals, per-source counts, and freshness. Refreshes first unless `--no-refresh` is passed.

## `cairn sources`

Print every configured provider root and whether it exists. This is the first check when a source seems missing.

## `cairn overview`

Show totals, projects, the current project, recent sessions, and source counts.

| Option | Meaning |
|---|---|
| `--project P` | Restrict to a project name or path |
| `--cwd PATH` | Resolve the current project from this directory. Default: process cwd |
| `--limit N` | Number of projects and recent sessions. Default: 8 |

## `cairn sessions`

List sessions, newest first.

| Option | Meaning |
|---|---|
| `--project P` | Project name or path |
| `--source S` | Source ID |
| `--query Q` | Match session title or summary |
| `--after D` | Session end/start at or after an ISO date or timestamp |
| `--before D` | Session end/start at or before an ISO date or timestamp |
| `--limit N` | Default: 20, capped at 200 |
| `--offset N` | Skip rows |

## `cairn search`

Search message text and return session groups by default.

```sh
cairn search "auth redirect" --limit 10
cairn search "Date.now" --all --json
cairn search "迁移" --project my-app --context 3
```

| Option | Meaning |
|---|---|
| `--project P` | Project name or path |
| `--session ID` | Restrict to one session |
| `--source S` | Restrict to one source ID |
| `--after D` / `--before D` | Time window |
| `--limit N` | Default: 20, capped at 200 |
| `--context N` | Add up to N neighboring messages to the best grouped hit |
| `--include-meta` | Include thinking, tool results, and tool-use rows |
| `--full-text` | Return the complete matched message instead of only a snippet |
| `--all` | Return message hits instead of grouping by session |

The query is literal. It does not parse `OR`, `NEAR`, or FTS syntax. See [Search](search.md) for ranking and edge cases.

## `cairn show`

Read a session timeline or a window around one message.

```sh
cairn show <session-id> --json
cairn show <session-id> --around <uuid> --before 3 --after 3 --tools --json
```

| Option | Meaning |
|---|---|
| `--around UUID` | Center the window on a message UUID |
| `--before N` | Messages before the center |
| `--after N` | Messages after the center |
| `--limit N` | Default: 200 |
| `--tools` | Include tool calls/results and meta messages |

Without `--around`, `show` returns the visible text timeline in order.

## `cairn sql`

Run one read-only `SELECT` or `WITH` query against the index.

```sh
cairn sql "SELECT source, COUNT(*) FROM sessions GROUP BY source"
cairn sql "SELECT name, input_json FROM tool_calls WHERE file_path LIKE ?" --param '"%auth%"'
```

| Option | Meaning |
|---|---|
| `--param JSON` | Bind one positional `?` parameter. Repeatable |
| `--max-rows N` | Default: 500, capped at 2,000 |
| `--timeout SECONDS` | Default: 3 |

The guard allows only one statement, rejects writes and unsafe PRAGMAs, blocks extension/file functions, and enforces a deadline. See [Architecture](architecture.md#sql-guard).

## `cairn schema`

Print the actual SQLite DDL used by the index. This is the authoritative schema for SQL queries; the agent skill keeps a readable reference in [`skills/cairn/references/schema.md`](../skills/cairn/references/schema.md).

## JSON output

`--json` returns stable objects for scripts and agents. Search returns `sessions[]` or `hits[]`; `show` returns `session` plus `messages[]`; `sql` returns `columns`, `rows`, `truncated`, `row_count`, and `elapsed_ms`.

The CLI exits non-zero on invalid input, missing sessions, denied SQL, or index errors. When `--json` is present, errors are printed as `{"ok": false, "error": "..."}`.
