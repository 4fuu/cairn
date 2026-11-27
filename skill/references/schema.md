# Cairn index schema

The index is one SQLite database at `~/.cairn/cairn.sqlite` (override with
`CAIRN_HOME` or `--db`). It is a normalized projection of provider transcripts;
the original files are never modified.

All timestamps are ISO-8601 UTC strings such as `2026-11-27T09:15:02.123Z`, so
lexicographic comparison is also chronological. Missing timestamps are `NULL`.

## Tables

### `sessions`

One row per provider transcript (a Claude Code JSONL file, a Codex rollout, or a
pi session file).

| Column | Meaning |
|---|---|
| `id` | Cairn session ID. Claude keeps its UUID; Codex and pi are prefixed (`codex:...`, `pi:...`). A `#<hash>` suffix appears when two files claim the same ID. |
| `source` | Provider ID: `claude`, `qoder`, `qoder-cn`, `codex`, `pi`, `omp`, `raft`, `cindy`, `dsh`, `cline`, `grok`, `droid`, `kimi`, `workbuddy`, `workbuddy-ai`, `codebuddy`, `opencode`, `zcode`, `hermes`, `agy`, `antigravity`, `devin`, `minimax`, or `liveagent`. |
| `title` | Provider title, explicit title, or the first real user prompt. |
| `project` | Basename of the session's working directory. |
| `project_path` | Full working directory. |
| `started_at`, `ended_at` | Bounds of the indexed messages. |
| `git_branch`, `model`, `version` | Provider metadata when present. |
| `summary` | Provider-emitted session/compaction summary. Derived, not raw evidence. |
| `message_count` | Number of non-meta messages (`messages.is_meta = 0`). |
| `source_path` | Absolute path of the transcript file. |
| `meta_json` | Provider-specific metadata as JSON. |
| `indexed_at` | Last successful index write. |

### `messages`

One row per addressable piece of a transcript. A single assistant turn with
text, thinking, and two tool calls becomes four rows, ordered by `seq`.

| Column | Meaning |
|---|---|
| `uuid` | Stable message ID. Use it with `cairn show --around`. |
| `session_id` | Joins to `sessions.id`. |
| `parent_uuid` | Provider parent link when available. |
| `seq` | Monotonic order inside the session (file order). |
| `timestamp` | Normalized UTC timestamp. |
| `role` | `user`, `assistant`, `tool`, or `system`. |
| `content_type` | `text`, `thinking`, `tool_use`, `tool_result`, `bash`, `summary`, or `custom`. |
| `text` | Searchable text. May be `NULL` for pure tool rows. |
| `text_words` | jieba search-mode segmentation of `text`, used by the word index. Do not query directly; use `messages_fts`. |
| `is_meta` | 1 for thinking, tool results, summaries, and other non-conversation rows. |
| `model` | Model that produced the row when known. |
| `input_tokens`, `output_tokens` | Usage attached to the assistant text row when the provider reports it. |
| `agent_id` | Claude subagent/sidechain ID when present. |
| `source`, `source_path` | Origin provider and transcript path. |
| `extra_json` | Structured extras: tool name/input, tool result link, bash command, truncation. |

`extra_json` shapes:

```json
{"tool_name":"Read","input":{"file_path":"/repo/auth.ts"}}
{"tool_call_id":"toolu_1","is_error":false}
{"command":"pytest tests","exit_code":1,"cancelled":false}
{"truncated":true}
```

### `tool_calls`

One row per tool invocation, including calls whose result is missing.

| Column | Meaning |
|---|---|
| `id` | Provider call ID (or a generated stable ID). |
| `session_id` | Session that made the call. |
| `message_uuid` | The `tool_use` message row. |
| `name` | Tool name, e.g. `Read`, `shell`, `edit`. |
| `input_json` | JSON input, truncated at 20k characters. |
| `file_path` | Best-effort file path extracted from the input. |
| `timestamp` | Call time. |
| `source`, `source_path` | Provider and transcript path. |

### `tool_results`

One row per tool result. `tool_call_id` joins `tool_calls.id`; it is not a
foreign key because a malformed transcript can lose the call.

| Column | Meaning |
|---|---|
| `tool_call_id` | The call this result answers. |
| `session_id` | Session. |
| `message_uuid` | The `tool_result` message row. |
| `content` | Result text, truncated at 100k characters. |
| `is_error` | 1 when the provider marked the result as an error. |
| `timestamp` | Result time. |
| `source`, `source_path` | Provider and transcript path. |

### `source_files`

Incremental-index bookkeeping. One row per transcript file.

| Column | Meaning |
|---|---|
| `path` | Absolute transcript path. |
| `source` | Provider. |
| `session_id` | Session produced by this file. |
| `mtime_ns`, `size` | Change detection signature. |
| `indexed_at` | Last successful parse. |
| `error` | Reserved for parse errors; the current build reports them on stdout instead. |

### `index_meta`

Key/value metadata: `schema_version`, `trigram`, `last_build`, `last_full_build`.

## Full-text tables

`messages_fts` and `messages_trigram` are external-content FTS5 indexes over
`messages.text`. Join them by rowid:

```sql
SELECT m.uuid, m.text
FROM messages_fts
JOIN messages m ON m.rowid = messages_fts.rowid
WHERE messages_fts MATCH '"redirect"'
LIMIT 20;
```

- `messages_fts` uses `unicode61`: good for English words and exact token runs.
- `messages_trigram` indexes raw `text` with `trigram`: exact substrings of 3+
  characters, including Chinese. A 1–2 character query has no trigram; use
  `LIKE '%q%'`.
- `bm25()` is negative: lower is better. `ORDER BY rank` is the FTS5 shorthand.

For normal retrieval, use `cairn search`; it merges both indexes and the LIKE
fallback. Use raw FTS only for a join the CLI cannot express.

## Joins and examples

### Session with its tool activity

```sql
SELECT s.id, s.title, s.project,
       COUNT(DISTINCT tc.id) AS calls,
       SUM(CASE WHEN tr.is_error = 1 THEN 1 ELSE 0 END) AS failures
FROM sessions s
LEFT JOIN tool_calls tc ON tc.session_id = s.id
LEFT JOIN tool_results tr ON tr.tool_call_id = tc.id
GROUP BY s.id
ORDER BY s.ended_at DESC
LIMIT 20;
```

### File history

```sql
SELECT s.id, s.title, s.project, tc.name, tc.timestamp,
       substr(tc.input_json, 1, 240) AS input
FROM tool_calls tc
JOIN sessions s ON s.id = tc.session_id
WHERE tc.file_path LIKE '%/auth.ts'
ORDER BY tc.timestamp DESC
LIMIT 50;
```

`file_path` is best-effort. A path embedded in a shell command is extracted when
it starts with `/`; a relative path may not be present. Fall back to searching
the input JSON:

```sql
WHERE tc.input_json LIKE '%auth.ts%'
```

### Failed tool calls

```sql
SELECT s.id, s.title, tc.name, tr.content
FROM tool_results tr
JOIN tool_calls tc ON tc.id = tr.tool_call_id
JOIN sessions s ON s.id = tr.session_id
WHERE tr.is_error = 1
ORDER BY tr.timestamp DESC
LIMIT 50;
```

Providers sometimes report a failure only in the text (`Exit code 1`, `FAILED`).
Search `tr.content` as well:

```sql
WHERE tr.is_error = 1 OR tr.content LIKE '%Exit code %'
```

### Session timeline including tools

```sql
SELECT m.seq, m.timestamp, m.role, m.content_type,
       CASE m.content_type
         WHEN 'tool_use' THEN json_extract(m.extra_json, '$.tool_name')
         ELSE substr(COALESCE(m.text, ''), 1, 200)
       END AS preview
FROM messages m
WHERE m.session_id = ?
ORDER BY m.seq;
```

### Token usage by day and source

```sql
SELECT substr(m.timestamp, 1, 10) AS day, m.source,
       SUM(COALESCE(m.input_tokens, 0)) AS input,
       SUM(COALESCE(m.output_tokens, 0)) AS output
FROM messages m
WHERE m.timestamp IS NOT NULL
  AND (m.input_tokens IS NOT NULL OR m.output_tokens IS NOT NULL)
GROUP BY day, m.source
ORDER BY day DESC;
```

### Claude subagents

```sql
SELECT m.session_id, m.agent_id, COUNT(*) AS messages
FROM messages m
WHERE m.agent_id IS NOT NULL
GROUP BY m.session_id, m.agent_id
ORDER BY messages DESC
LIMIT 20;
```

## Session IDs and URLs

Cairn IDs are not URLs. To point a human at the source, use `sessions.source_path`
and the session's provider-native ID inside the file. `cairn show` returns the
canonical Cairn ID; use that for follow-up queries.
