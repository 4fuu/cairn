# Sources

Cairn follows the session locations each agent already writes. The defaults mirror [magpie](https://github.com/yetone/magpie)'s session layer where one exists.

## Default roots

| Source ID | Default root | Format |
|---|---|---|
| `claude` | `~/.claude/projects` (`$CLAUDE_CONFIG_DIR`) | JSONL |
| `qoder` | `~/.qoder/projects` (`$QODER_CONFIG_DIR`) | Claude JSONL |
| `qoder-cn` | `~/.qoder-cn/projects` (`$QODERCN_CONFIG_DIR`) | Claude JSONL |
| `codex` | `~/.codex/sessions`, `~/.codex/archived_sessions` (`$CODEX_HOME`) | rollout JSONL |
| `pi` | `~/.pi/agent/sessions` (`$PI_CODING_AGENT_DIR`) | pi JSONL |
| `omp` | `~/.omp/agent/sessions`, plus `~/.omp/profiles/*/agent/sessions` | pi JSONL |
| `raft` | `~/.slock/agents/*/.pi-sessions`, `.builtin-sessions` (`$SLOCK_HOME`) | pi JSONL |
| `cindy` | `<config>/Cindy/pi-agent-home/sessions`, `<config>/CindyGlobal/...` | pi JSONL |
| `dsh` | `~/.dsh/sessions` (`$DSH_HOME`) | JSONL / zstd |
| `opencode` | `~/.local/share/opencode/opencode.db` (`$OPENCODE_DB`) | SQLite |
| `zcode` | `~/.zcode/cli/db/db.sqlite` | OpenCode SQLite |
| `cline` | `~/.cline/data/sessions` (`$CLINE_DATA_DIR`, `$CLINE_SESSION_DATA_DIR`) | JSON |
| `grok` | `~/.grok/sessions` (`$GROK_HOME`) | JSONL |
| `droid` | `<FACTORY_HOME_OVERRIDE>/.factory/sessions`, default `~/.factory/sessions` | JSONL |
| `kimi` | `~/.kimi-code/sessions` (`$KIMI_CODE_HOME`) | `wire.jsonl` |
| `workbuddy` | `~/.workbuddy/projects` (`$WORKBUDDY_CONFIG_DIR`) | JSONL |
| `workbuddy-ai` | `~/.workbuddy-ai/projects` | JSONL |
| `codebuddy` | `~/.codebuddy/projects` (`$CODEBUDDY_CONFIG_DIR`) | JSONL |
| `hermes` | `~/.hermes/state.db` and `~/.hermes/profiles/*/state.db` (`$HERMES_HOME`) | SQLite |
| `agy` | `~/.gemini/antigravity-cli/brain` | transcript JSONL |
| `antigravity` | `~/.gemini/antigravity/brain` | transcript JSONL |
| `devin` | `~/.local/share/devin/cli/sessions` | `log.jsonl` |
| `minimax` | `~/.minimax/v2/sessions` (`$MINIMAX_DATA_DIR`) | JSONL |
| `liveagent` | `~/.liveagent/sessions` (`$LIVEAGENT_HOME`, `$K_BRAIN_HOME`) | `session.jsonl` |
| `dimagent` | `~/.dim/v2/dimcode.sqlite` (`$DIMCODE_HOME`) | SQLite |

`raft` and `cindy` are only discovered when their parent directories exist. `omp` also discovers profile-specific session directories.

## Root overrides

Override one or more roots for a single invocation:

```sh
cairn --root codex=/path/to/codex --root kimi=/path/to/kimi build
cairn --root dimagent=/path/to/dim/v2 search "migration"
```

`--root` uses `SOURCE=PATH` and is repeatable. `--source` filters queries; it does not change discovery.

`cairn sources` reports every configured root, whether it exists, and how many files it contains.

## SQLite-backed sources

OpenCode, ZCode, Hermes, and DimAgent are read from their SQLite databases rather than JSONL files. Cairn opens these databases read-only with a busy timeout.

SQLite sources are replaced as a whole path when their file changes. They do not use the append-only optimization used for JSONL transcripts.

### DimAgent

DimAgent v2 stores sessions under `$DIMCODE_HOME/v2/` when `DIMCODE_HOME` is set, otherwise under `~/.dim/v2/`.

- Primary database: `dimcode.sqlite`
- Legacy/documented name: `dimglobal.sqlite` (also discovered)
- Session rows: `sessions`
- Message parts: `messages.parts` (`text`, `thinking`, `tool_use`, `tool_result`)
- Model: `session_states.selectedModelId`
- Usage: `usage_run_stats`, with `usage_ledger` as a fallback

## Not indexed

- **Amp**: threads are server-side; use Amp's own thread tools.
- **Cursor**: the local blob store is not a stable transcript format.
- **Alma, OpenMausBot, CatPaw**: no adapter yet.

Cairn never edits provider files, resumes sessions, or writes into a provider database.
