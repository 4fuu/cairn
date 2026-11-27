# Query patterns

Copyable retrieval plans. The CLI refreshes the index before each command, so
these are safe to run directly. Keep first passes bounded; expand only hits that
answer the question.

## 1. First pass: orient, recall, evidence

```bash
cairn overview --json
cairn sessions --limit 10 --json
cairn search "the literal topic" --limit 10 --json
```

The default result is grouped by session: `sessions[]` entries are the best hit
in each session plus a `hits` count. Add `--all` for message-level `hits[]`.

Then expand the best hit:

```bash
cairn show <session-id> --around <uuid> --before 3 --after 3 --tools --json
```

Why: `overview` is a map, `sessions` finds the right conversation, `search` finds
the right message, `show` supplies the evidence. Do not answer from titles alone.

## 2. Locate a session without a topic

By project and time:

```bash
cairn sessions --project my-repo --after 2026-11-01 --limit 20 --json
```

By title fragment:

```bash
cairn sessions --query "auth redirect" --limit 10 --json
```

By source:

```bash
cairn sessions --source pi --limit 20 --json
```

A project name matches `sessions.project` exactly and `sessions.project_path` as
a prefix. A path such as `/home/me/repo` works too.

## 3. "How did we fix X?" (evidence trail)

1. `cairn search "error text or feature name" --limit 12 --json`
2. Pick the session with the most relevant user prompt, not the highest-ranked
   assistant paragraph.
3. `cairn show <session-id> --around <uuid> --before 5 --after 8 --tools --json`
4. Look for `tool_use` → `tool_result` pairs. The fix is usually in an `edit` or
   `write` call, not in the assistant's summary.
5. If multiple sessions match, compare their last assistant text and file paths:

```bash
cairn sql "SELECT s.id, s.title, s.ended_at,
                  COUNT(DISTINCT tc.file_path) AS files
           FROM sessions s
           JOIN tool_calls tc ON tc.session_id = s.id
           WHERE tc.file_path LIKE '%/auth/%'
           GROUP BY s.id
           ORDER BY s.ended_at DESC
           LIMIT 20" --json
```

## 4. File history: where was this file touched?

```bash
cairn sql "SELECT s.id, s.title, s.project, tc.name, tc.timestamp,
                  substr(tc.input_json, 1, 200) AS input
           FROM tool_calls tc
           JOIN sessions s ON s.id = tc.session_id
           WHERE tc.file_path LIKE '%/src/retry.ts'
           ORDER BY tc.timestamp DESC
           LIMIT 50" --json
```

If `file_path` misses a relative path, search the input:

```bash
cairn sql "SELECT s.id, s.title, tc.name, tc.timestamp
           FROM tool_calls tc
           JOIN sessions s ON s.id = tc.session_id
           WHERE tc.input_json LIKE '%src/retry.ts%'
           ORDER BY tc.timestamp DESC
           LIMIT 50" --json
```

Then read the interesting session around the call. A file that appears in many
sessions with small edits is usually a hot spot; a file that appears once in a
large diff is usually a migration.

## 5. Failures and retries

```bash
cairn sql "SELECT s.id, s.title, tc.name, tr.timestamp,
                  substr(tr.content, 1, 500) AS output
           FROM tool_results tr
           JOIN tool_calls tc ON tc.id = tr.tool_call_id
           JOIN sessions s ON s.id = tr.session_id
           WHERE tr.is_error = 1
              OR tr.content LIKE '%Exit code %'
              OR tr.content LIKE '%FAILED %'
           ORDER BY tr.timestamp DESC
           LIMIT 50" --json
```

To see what the agent tried next, take the session ID and:

```bash
cairn show <session-id> --around <tool_result-uuid> --before 2 --after 6 --tools --json
```

Do not assume a failure was fixed because a later message says "fixed". Look for
a successful command or a diff.

## 6. What changed in a session

```bash
cairn sql "SELECT m.seq, m.role, m.content_type,
                  CASE m.content_type
                    WHEN 'tool_use' THEN json_extract(m.extra_json, '$.tool_name')
                    ELSE substr(COALESCE(m.text, ''), 1, 240)
                  END AS preview
           FROM messages m
           WHERE m.session_id = ?
           ORDER BY m.seq" --param '"<session-id>"' --json
```

Filter to edits:

```bash
cairn sql "SELECT tc.timestamp, tc.name, tc.file_path,
                  substr(tc.input_json, 1, 300) AS input
           FROM tool_calls tc
           WHERE tc.session_id = ?
             AND lower(tc.name) IN ('edit','write','multiedit','apply_patch')
           ORDER BY tc.timestamp" --param '"<session-id>"' --json
```

Tool names differ by provider. Inspect `SELECT DISTINCT name FROM tool_calls`
before relying on a name list.

## 7. Subagent conclusions

Claude sidechains carry `agent_id`; Codex subagents appear as separate sessions
with a parent link in `session_meta` (the parent link is provider-specific and
may not be normalized). Start with:

```bash
cairn sql "SELECT m.session_id, m.agent_id, COUNT(*) AS messages,
                  substr(MAX(CASE WHEN m.role = 'assistant' AND m.content_type = 'text'
                                  THEN m.text END), 1, 400) AS last_assistant
           FROM messages m
           WHERE m.agent_id IS NOT NULL
           GROUP BY m.session_id, m.agent_id
           ORDER BY messages DESC
           LIMIT 20" --json
```

For the exact conclusion, read the sidechain messages:

```bash
cairn show <session-id> --tools --json
```

Remember that an assistant's subagent summary is a claim; the sidechain's tool
results are the evidence.

## 8. Activity and token usage

```bash
cairn sql "SELECT substr(timestamp, 1, 10) AS day, source,
                  COUNT(*) AS messages,
                  SUM(COALESCE(input_tokens, 0)) AS input,
                  SUM(COALESCE(output_tokens, 0)) AS output
           FROM messages
           WHERE timestamp IS NOT NULL AND is_meta = 0
           GROUP BY day, source
           ORDER BY day DESC
           LIMIT 30" --json
```

Projects by activity:

```bash
cairn sql "SELECT project, COUNT(*) AS sessions, SUM(message_count) AS messages,
                  MAX(COALESCE(ended_at, started_at)) AS last_seen
           FROM sessions
           WHERE project IS NOT NULL
           GROUP BY project
           ORDER BY last_seen DESC
           LIMIT 20" --json
```

Token counts exist only where the provider records them. Missing values are
`NULL`, not zero.

## 9. Search only tool output

```bash
cairn search "connection refused" --include-meta --limit 10 --json
```

This includes thinking, tool results, and summaries because they have
`is_meta = 1`. For exact SQL:

```bash
cairn sql "SELECT m.session_id, m.uuid, m.content_type,
                  substr(m.text, 1, 500) AS output
           FROM messages m
           WHERE m.is_meta = 1
             AND m.text LIKE '%connection refused%'
           ORDER BY m.timestamp DESC
           LIMIT 20" --json
```

## 10. Raw FTS for a join

```bash
cairn sql "SELECT m.session_id, m.uuid, m.timestamp,
                  snippet(messages_fts, 0, '>>', '<<', '…', 16) AS excerpt
           FROM messages_fts
           JOIN messages m ON m.rowid = messages_fts.rowid
           WHERE messages_fts MATCH '\"redirect loop\"'
           ORDER BY rank
           LIMIT 20" --json
```

Use `messages_trigram` for raw-text excerpts of 3+ characters; use
`messages_fts` for jieba word matches (its snippet shows segmented text). For
1–2 characters, use `LIKE '%字%'`.

## 11. Compare two sessions

```bash
cairn sql "SELECT id, title, project, started_at, ended_at, message_count, model
           FROM sessions
           WHERE id IN (?, ?)" --param '"<id-a>"' --param '"<id-b>"' --json
```

Then compare their edit calls:

```bash
cairn sql "SELECT session_id, name, file_path, substr(input_json, 1, 300)
           FROM tool_calls
           WHERE session_id IN (?, ?)
           ORDER BY session_id, timestamp" \
  --param '"<id-a>"' --param '"<id-b>"' --json
```

## 12. When the index looks empty

```bash
cairn status --json
cairn sources --json
cairn build --full --json
```

If a root is missing, the provider is not installed or lives elsewhere. Pass
`--root claude=/path/to/projects` (repeatable) or set `CAIRN_HOME`. Do not
conclude that the history does not exist until the source root has been checked.
