# cairn

A local, read-only index of coding-agent sessions, queried by agents through a
CLI and a skill. Written in Rust: one binary, no runtime, no daemon, no desktop
app.

Cairn reads the transcripts each agent already writes, normalizes them into one
SQLite database, and exposes:

- fast full-text search across 20+ agents, including Chinese
- bounded session/message reads with stable IDs
- read-only SQL for joins, file history, failures, and usage
- a skill that teaches an agent when and how to use it

The original transcripts are never modified.

## Install

The current release is `2026.1127.2`. Versions use the calendar form
`YYYY.MDD.REVISION` in `Asia/Hong_Kong`, matching the release scheme used by
URI Agent.

### One-line installer

Linux (x86-64 / ARM64) and Apple Silicon macOS:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://raw.githubusercontent.com/4fuu/cairn/main/scripts/install.sh | sh
```

### Homebrew (Apple Silicon macOS)

```sh
brew tap 4fuu/cairn https://github.com/4fuu/cairn
brew install 4fuu/cairn/cairn
```

### Scoop (64-bit Windows)

```powershell
scoop bucket add cairn https://github.com/4fuu/cairn
scoop install cairn
```

### Cargo

```sh
cargo install --git https://github.com/4fuu/cairn
cairn --version
```

Or from a local checkout:

```sh
git clone https://github.com/4fuu/cairn
cd cairn
cargo install --path .
```

Install the skill with `npx skills`:

```sh
npx skills add 4fuu/cairn --skill cairn -a amp -g -y
```

For development:

```sh
cargo run -- --version
cargo test
```

## Quick start

```sh
cairn build
cairn status
cairn overview
cairn search "auth redirect"
cairn show <session-id> --around <uuid> --before 3 --after 3 --tools
cairn sql "SELECT source, COUNT(*) FROM sessions GROUP BY source"
```

Every query refreshes the index incrementally before answering. The first build
reads every configured source root; later builds only re-parse files whose
mtime or size changed.

## Sources

Cairn follows the session locations used by the agents themselves (and by
[magpie](https://github.com/yetone/magpie)'s session layer):

| Source | Default root | Format |
|---|---|---|
| Claude Code | `~/.claude/projects` (`$CLAUDE_CONFIG_DIR`) | JSONL |
| Qoder / Qoder CN | `~/.qoder/projects`, `~/.qoder-cn/projects` | Claude JSONL |
| Codex | `~/.codex/sessions`, `~/.codex/archived_sessions` (`$CODEX_HOME`) | rollout JSONL |
| pi | `~/.pi/agent/sessions` (`$PI_CODING_AGENT_DIR`) | session JSONL |
| omp / oh-my-pi | `~/.omp/agent/sessions` | pi JSONL |
| Raft | `~/.slock/agents/*/.pi-sessions`, `.builtin-sessions` | pi JSONL |
| Cindy | `<config>/Cindy/pi-agent-home/sessions` | pi JSONL |
| DeepSeek Harness | `~/.dsh/sessions` (`$DSH_HOME`) | JSONL / zstd |
| OpenCode | `~/.local/share/opencode/opencode.db` (`$OPENCODE_DB`) | SQLite |
| ZCode | `~/.zcode/cli/db/db.sqlite` | OpenCode SQLite |
| Cline | `~/.cline/data/sessions` | JSON |
| Grok Build | `~/.grok/sessions` (`$GROK_HOME`) | JSONL |
| Droid | `~/.factory/sessions` (`$FACTORY_HOME_OVERRIDE`) | JSONL |
| Kimi Code | `~/.kimi-code/sessions` (`$KIMI_CODE_HOME`) | `wire.jsonl` |
| WorkBuddy | `~/.workbuddy/projects` | JSONL |
| WorkBuddy AI | `~/.workbuddy-ai/projects` | JSONL |
| CodeBuddy | `~/.codebuddy/projects` | JSONL |
| Hermes Agent | `~/.hermes/state.db` and profiles (`$HERMES_HOME`) | SQLite |
| Antigravity CLI (`agy`) | `~/.gemini/antigravity-cli/brain` | transcript JSONL |
| Antigravity app | `~/.gemini/antigravity/brain` | transcript JSONL |
| Devin CLI | `~/.local/share/devin/cli/sessions` | `log.jsonl` |
| MiniMax Code | `~/.minimax/v2/sessions` (`$MINIMAX_DATA_DIR`) | JSONL |
| LiveAgent | `~/.liveagent/sessions` | `session.jsonl` |
| DimAgent | `~/.dim/v2/dimcode.sqlite` (`$DIMCODE_HOME`) | SQLite |

Override any root per invocation:

```sh
cairn --root codex=/path/to/codex --root kimi=/path/to/kimi build
```

`cairn sources` shows which roots exist and how many files each contains.

## Search

`cairn search` is built for recall first, then ranking:

- **Trigram** exact substrings of 3+ characters — code identifiers, Chinese,
  paths, error text.
- **Word index** over jieba search-mode segmentation — English words and
  two-character Chinese terms.
- **LIKE fallback** for 1–2 character queries and when FTS returns nothing.
- Results group by session by default: best snippet plus hit count. Use
  `--all` for message-level hits.
- Snippets are highlighted in a terminal; `--json` returns plain text for
  agents.
- `--context N` adds temporal neighbors; `--include-meta` includes thinking and
  tool output; `--full-text` returns the whole matched message.

The query is literal. It does not interpret `OR`, `NEAR`, or FTS syntax.

## Commands

| Command | Output |
|---|---|
| `cairn overview [--project P] [--limit N]` | Totals, current project, projects, recent sessions |
| `cairn sessions [--project P] [--source S] [--query Q] [--after D] [--before D]` | Session list |
| `cairn search "text" [--project P] [--session ID] [--source S] [--context N] [--include-meta] [--all]` | Grouped session hits or message hits |
| `cairn show ID [--around UUID] [--before N] [--after N] [--tools]` | Session or message window |
| `cairn sql "SELECT ..." [--param JSON] [--max-rows N]` | Guarded read-only SQL |
| `cairn status` | Index path, totals, per-source counts, freshness |
| `cairn sources` | Configured provider roots |
| `cairn build [--full] [--source S]` | Force a refresh |
| `cairn schema` | The actual DDL |

`--json` works before or after the subcommand. `--root`, `--db`, `--no-refresh`
are global too.

## Install the skill

The skill is installed by [Vercel Labs' `skills` CLI](https://github.com/vercel-labs/skills),
which owns the per-agent paths:

```sh
npx skills add 4fuu/cairn --skill cairn -a amp -g -y
```

Use `-a claude-code`, `-a codex`, or repeat `-a` to install for several agents.
Omit `-g` to install into the current project. `npx skills add 4fuu/cairn
--list` lists the available skill without installing it.

The source lives in [`skills/cairn`](skills/cairn). The CLI no longer installs
or copies skill files itself.

## How it works

```text
provider transcripts (JSONL / SQLite / JSON)
        │
        ▼
  adapters: claude, codex, pi, dsh, opencode, kimi, ...
        │  normalize sessions, messages, tool calls, results
        ▼
  ~/.cairn/cairn.sqlite
    sessions · messages · tool_calls · tool_results
    messages_fts (jieba words) · messages_trigram (raw text)
        │
        ├── cairn search / show / sessions / overview
        └── cairn sql  (read-only authorizer + deadline + row cap)
```

Indexing parses changed files in parallel (rayon) and writes them serially in
one SQLite transaction. `BEGIN IMMEDIATE` keeps concurrent CLI invocations
safe. FTS updates are trigger-driven.

## Design boundaries

- **Read-only over transcripts.** Cairn never edits provider files and never
  resumes a session.
- **No daemon.** Each invocation opens the index, refreshes it if needed, and
  exits.
- **Agent-oriented.** `--json` everywhere, bounded output, stable IDs, and a
  skill that names the workflow instead of dumping a schema.
- **Cloud-only agents stay out.** Amp threads are server-side; use Amp's own
  thread tools. Cursor's blob store, Alma, OpenMausBot, and CatPaw are not
  included yet.
- **Transcript text is untrusted.** The skill tells the agent not to follow
  instructions found in history.

## Development

```sh
cargo test
cargo clippy
```

The committed fixtures cover Claude Code, Codex, and pi, including tool calls,
deduplication, Chinese search, token usage, and the SQL guard. The adapters have
also been exercised against magpie's real, sanitized session testdata for
Claude, Codex, pi/omp, DSH, Cline, Grok, Kimi, WorkBuddy, Devin, MiniMax, and
Antigravity.

## License

MIT.
