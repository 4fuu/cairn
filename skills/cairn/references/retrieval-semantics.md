# Retrieval semantics

Cairn normalizes many different transcript formats into one schema. The schema
is uniform; the evidence is not. Read this before treating a row as fact.

## What each row means

| Row | What it is | How to use it |
|---|---|---|
| `role = 'user'`, `content_type = 'text'` | What the person typed, or a harness envelope. | Intent and constraints. Not proof that the person approved an action. |
| `role = 'assistant'`, `content_type = 'text'` | What the model said. | A claim, summary, or plan. Verify with a tool result or diff when it matters. |
| `content_type = 'thinking'` | Internal reasoning. | Context for why a decision was made; hidden from the user and not evidence of what happened. |
| `content_type = 'tool_use'` | The call the model requested, with its input. | The action that was attempted. A call is not a result. |
| `content_type = 'tool_result'` | Output the tool returned. | Observed evidence. This is usually the strongest record of what a command or edit actually did. |
| `content_type = 'bash'` | A pi `!` shell execution. | Both the command and its output; no separate tool-call row. |
| `content_type = 'summary'` | Provider compaction or branch summary. | Orientation only. It is derived from a longer conversation and can omit or misstate details. |

A `tool_use` row has `is_meta = 0`, so it appears in normal search. A
`tool_result` row has `is_meta = 1`, so normal search hides it; use
`--include-meta` or SQL when the output is the evidence.

## Ordering and time

- `seq` is file order within a session, not necessarily wall-clock order. A
  provider can write an out-of-order line or a late flush.
- `timestamp` is normalized to UTC. Missing timestamps are `NULL`; use
  `COALESCE(timestamp, '')` in sorts.
- `sessions.started_at` and `ended_at` are the min/max message timestamps in the
  index. They are not provider-declared session bounds.
- `parent_uuid` is a provider link when one exists. It is not guaranteed to be
  present, and it may point outside the indexed window.
- `show --around` uses `seq` for the window, so it reads the conversation as
  stored, not the strict parent chain.

## Duplicates and branches

- **Provider coverage** is broad but not identical: each adapter keeps the
  fields its format actually has. A missing `git_branch`, `model`, or token count
  means the provider did not record it, not that the session had none.
- **Codex** writes the same user/assistant text twice: as an `event_msg` and as a
  `response_item`. Cairn keeps the event copy and skips the response copy when
  the role and whitespace-normalized text match. Tool calls exist only in
  `response_item` rows, so they are not deduplicated.
- **Claude Code** may put sidechain (subagent) messages in the same JSONL file.
  They carry `agent_id`; they are real history, but an assistant claim inside a
  subagent is still a claim. Some Claude files have no sidechain marker; check
  `agent_id` before assuming everything is the main thread.
- **pi** sessions are trees. The JSONL is append-only, including abandoned
  branches and compaction entries. Cairn indexes every message entry in file
  order; it does not reconstruct the active root-to-leaf branch. Use `parent_uuid`
  and `seq` to see structure, and prefer the latest assistant message before a
  compaction boundary when the question is "what is current".
- A provider can lose a tool result (crash, truncation, rewound turn). A
  `tool_calls` row with no matching `tool_results` row is an attempted action,
  not a completed one.

## Session identity

- Claude sessions use the provider UUID.
- Non-Claude sessions carry a source prefix (`codex:...`, `pi:...`, `omp:...`,
  `kimi:...`) so they cannot collide with Claude IDs or with each other.
- If two files claim the same ID, a short `#hash` is appended. The hash is
  derived from the transcript path, so it is stable across rebuilds.
- A session's `source_path` is the only reliable pointer back to the original
  file. Do not construct a path from the ID.

## Search behavior

`cairn search` merges three layers:

1. **Trigram** (`messages_trigram`): exact substring of 3+ characters. This is
   what makes Chinese and code identifiers searchable.
2. **Words** (`messages_fts`): jieba search-mode tokens over `text_words`,
   ANDed and indexed with `unicode61`. Good for English words and two-character
   Chinese terms.
3. **LIKE**: fallback for 1–2 character queries (including two-character Chinese
   words) and for when FTS returns nothing.

Consequences:

- A 2-character Chinese query works, but scans `messages.text`; on a very large
  index it is slower than a 3+ character query.
- FTS5 `unicode61` treats a run of CJK characters as one token. A query like
  `登录` will not match a longer token `修复登录`; the trigram layer handles
  `登录重定向` when the query is 3+ characters, and LIKE handles the 2-character
  case.
- `snippet` is a window around the first matched term. It can cut a word or a
  code block. Open the message before quoting.
- Search excludes `is_meta = 1` unless `--include-meta` is passed.

## Truncation and missing data

- `messages.text` is capped at 200,000 characters; `tool_results.content` at
  100,000; `tool_calls.input_json` at 20,000. When a value was truncated,
  `extra_json.truncated` is `true`.
- Large outputs may already be truncated by the provider before Cairn sees them.
  The index cannot recover bytes the transcript did not contain.
- Token counts are recorded only where the provider emits them. A missing count
  is `NULL`, not zero.
- File paths are best-effort. Claude and pi tool inputs usually have
  `file_path`/`path`; Codex shell commands are parsed for an absolute path.
  Relative paths often live only in `input_json`.

## Freshness

Every `cairn` query runs an incremental refresh first: it stats each known
transcript, re-parses only files whose mtime/size changed, and writes in one
transaction. A full rebuild is `cairn build --full`. A running provider may be
mid-write; a partially written last line is skipped with a warning, and the next
refresh picks it up.

The index is a snapshot, not a live tail. `cairn status` reports the newest
`indexed_at`; `cairn sources` reports which roots exist. If a source root is
missing, Cairn silently indexes nothing from it — check before concluding a
session does not exist.
