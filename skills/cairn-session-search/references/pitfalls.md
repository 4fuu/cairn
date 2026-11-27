# Pitfalls

## `sql` rejects a query

The guard is intentionally narrow:

```text
only read-only SELECT/WITH queries are allowed
exactly one SQL statement is allowed per call
only read-only SELECT queries are allowed
query exceeded the 3s deadline
```

- One statement. Put the semicolon only at the end, or omit it.
- `SELECT` or `WITH` only. `PRAGMA`, `ATTACH`, `EXPLAIN`, and writes are denied.
- Functions such as `load_extension`, `readfile`, and `writefile` are denied.
- The default row cap is 500. Use `--max-rows` deliberately; do not stream a
  whole table into the transcript.
- A query that joins two FTS tables can exceed the deadline. Filter one side
  first, or use `cairn search`.

## `no session matches` or `ambiguous session prefix`

`cairn show` accepts an exact session ID or an unambiguous prefix. If the prefix
matches two sessions, the error lists both. Use the full ID from
`cairn sessions --json`.

## `message ... is not in session ...`

`--around` requires a message UUID from the same session. UUIDs are returned by
`cairn search` and by `cairn show`. A tool result's UUID is on its own
`tool_result` row, not on the assistant's `tool_use` row.

## Search returns nothing

Check, in order:

```bash
cairn status --json
cairn sources --json
cairn search "shorter phrase" --limit 20 --json
cairn build --full --json
```

- The provider root may not exist on this machine.
- The index may be empty (`sessions: 0`).
- The phrase may be 1–2 characters and hidden inside a larger CJK token; the
  LIKE fallback should still find it, but try a 3+ character phrase.
- The evidence may be only in tool output; add `--include-meta`.
- The session may be in a different project; drop `--project`.

## `messages.text` is `NULL`

Pure tool-call rows have no text; the call is in `tool_calls`. Pure tool-result
rows have `content` in `tool_results` and usually text as well. If you need the
result, join:

```sql
SELECT tc.name, tr.content
FROM tool_calls tc
JOIN tool_results tr ON tr.tool_call_id = tc.id
WHERE tc.session_id = ?;
```

## `file_path` is `NULL` or wrong

`file_path` is extracted from tool input keys (`file_path`, `path`, `filePath`,
`notebook_path`, `target_file`) or from an absolute path in a shell command. A
relative path in a command, a patch body, or a custom tool schema may not be
extracted. Fall back to:

```sql
WHERE input_json LIKE '%src/retry.ts%'
```

## FTS5 `MATCH` syntax error

Raw FTS queries need quoted phrases:

```sql
WHERE messages_fts MATCH '"redirect loop"'
```

An unquoted `-`, `:`, `*`, or `OR` is FTS syntax, not literal text. `cairn
search` quotes automatically; raw SQL does not.

## `bm25()` scores look negative

FTS5 returns negative BM25 scores because lower is better. Use `ORDER BY rank`
or `ORDER BY bm25(table)` ascending. Do not sort descending and call the top
result the best.

## Timestamps do not sort

All timestamps are ISO-8601 UTC with milliseconds, or `NULL`. Use
`COALESCE(timestamp, '')` in a mixed sort. Do not compare a raw epoch value to a
normalized string.

## Tool names differ by provider

Claude uses names such as `Read`, `Edit`, `Bash`; Codex may use `shell`,
`apply_patch`, `function_call`; pi uses lowercase names such as `read`, `edit`,
`bash`. Inspect before filtering:

```sql
SELECT name, COUNT(*) FROM tool_calls GROUP BY name ORDER BY COUNT(*) DESC;
```

## A session has `message_count = 0`

The session contains only meta rows (thinking, tool results, summaries) or its
visible messages were all skipped as harness envelopes. Use `show --tools` or
query `messages` directly.

## Index path and permissions

The default is `~/.cairn/cairn.sqlite`. Set `CAIRN_HOME` to move the whole
directory, or pass `--db` for one invocation. Read-only sandboxes may block the
index refresh; `--no-refresh` queries an existing index without writing, but a
missing or stale index is not repaired automatically.

## Provider-specific surprises

- Claude Code sometimes writes a task notification or command envelope as the
  first user message. Cairn skips known envelopes when choosing a title, but the
  envelope remains a message.
- Claude sidechains share the parent file; `agent_id` distinguishes them.
- Codex `event_msg` text can be split across several `text_elements`; Cairn
  joins them with newlines.
- pi compaction and branch summaries are `content_type = 'summary'` and
  `is_meta = 1`; they are not the conversation.
- pi stores empty or aborted sessions; a file can index to zero visible messages.
