# CLI reference

`cairn` opens the local index, refreshes it incrementally, and answers from SQLite. The default output is compact human-readable text; `--format json` is the machine-readable form.

## Global options

| Option | Meaning |
|---|---|
| `--db PATH` | Index path. Default: `~/.cairn/cairn.sqlite`, or `$CAIRN_HOME/cairn.sqlite` |
| `--format text\|json` | Output format. Default: `text` |
| `--json` | Alias for `--format json` |
| `--detail` | Add IDs, scores, paths, model, and token fields |
| `--no-update-check` | Skip the once-a-day release check |
| `--no-refresh` | Skip the incremental refresh before the command |
| `--root SOURCE=PATH` | Override a provider root. Repeatable |

Global options work before or after the subcommand.

Default output is intentionally bounded. Search returns the best hit plus a session ID and message UUID; `show` returns visible text; tool output and thinking require `--include-meta` or `--tools`. `--detail` adds the nonessential fields without changing which rows are selected.

## `cairn version`

Print the current version and, when available, the latest release.

```sh
cairn version
cairn version --format json
```

The command checks GitHub unless `--no-update-check` or `CAIRN_NO_UPDATE_CHECK=1` is set. Normal commands check at most once every 24 hours and print an update notice to stderr, never stdout. The check state is stored in `~/.cairn/update-check.json` (or `$CAIRN_HOME/update-check.json`). JSON output suppresses the notice.

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
| `--type TYPE` | Filter content type; repeatable or comma-separated (`text`, `thinking`, `tool_use`, `tool_result`, `tool`, `bash`, `summary`) |
| `--tool NAME` | Filter tool calls/results by tool name; repeatable or comma-separated, case-insensitive |
| `--full-text` | Return the complete matched message instead of only a snippet |
| `--all` | Return message hits instead of grouping by session |

The query is literal. It does not parse `OR`, `NEAR`, or FTS syntax. See [Search](search.md) for ranking and edge cases.

By default, each grouped result includes the session ID, title, project, source, hit count, timestamp, best message UUID, and snippet. The UUID is deliberate: it lets an agent call `show --around <uuid>` without another search. `--detail` adds score, sequence, role, and content type. `--include-meta` changes which message types can match; `--detail` does not.

`--type` and `--tool` are targeted filters. They imply the matching rows, so `--type thinking` works without `--include-meta`; `--type tool` means both `tool_use` and `tool_result`. Use `--tool read` to keep only calls/results for that tool.

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
| `--type TYPE` | Filter content type; repeatable or comma-separated (`text`, `thinking`, `tool_use`, `tool_result`, `tool`, `bash`, `summary`) |
| `--tool NAME` | Filter tool calls/results by tool name; repeatable or comma-separated, case-insensitive |

Without `--around`, `show` returns the visible text timeline in order. By default, messages contain sequence, role, content type, timestamp, and text. `--tools` adds tool-use/tool-result rows and their payloads; `--detail` adds UUID, model, token counts, and the meta flag. `--detail` does not add tool rows by itself.

`--type` and `--tool` select rows directly. For example, `show <id> --type thinking` reads only thinking, `show <id> --type tool` reads calls and results, and `show <id> --tool read` keeps only the `read` tool's calls/results.

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

Print the actual SQLite DDL used by the index. This is the authoritative schema for SQL queries; the agent skill keeps a readable reference in [`skills/cairn-session-search/references/schema.md`](../skills/cairn-session-search/references/schema.md).

## JSON output

`--json` returns stable objects for scripts and agents. Search returns `sessions[]` or `hits[]`; `show` returns `session` plus `messages[]`; `sql` returns `columns`, `rows`, `truncated`, `row_count`, and `elapsed_ms`.

JSON is compact by default. Session and search objects keep the fields needed to choose the next call; `--detail` restores full session metadata, scores, UUIDs, sequence numbers, roles, content types, model, and token fields. `cairn version --json` returns `version`, `latest`, `update_available`, and `release_url`.

The CLI exits non-zero on invalid input, missing sessions, denied SQL, or index errors. When `--json` is present, errors are printed as `{"ok": false, "error": "..."}`.
