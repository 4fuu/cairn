# cairn

A local, read-only index of coding-agent sessions, queried by agents through a CLI and a skill. Written in Rust: one binary, no runtime, no daemon, no desktop app.

Cairn reads the transcripts each agent already writes, normalizes them into one SQLite database, and exposes:

- fast full-text search across 20+ agent sources, including Chinese
- bounded session and message reads with stable IDs
- read-only SQL for joins, file history, failures, and usage
- a skill that teaches an agent when and how to use it

The original transcripts are never modified.

## Install

The latest release is [`2026.1127.3`](https://github.com/4fuu/cairn/releases/tag/v2026.1127.3). Versions use the calendar form `YYYY.MDD.REVISION` in `Asia/Hong_Kong`.

Linux x86-64/ARM64 and Apple Silicon macOS:

```sh copy Copy Install
curl --proto '=https' --tlsv1.2 -LsSf https://raw.githubusercontent.com/4fuu/cairn/main/scripts/install.sh | sh
```

Homebrew, Scoop, Cargo, release archives, upgrades, and uninstall steps are in [Install](docs/install.md).

Install the agent skill separately:

```sh copy Copy Install Skill
npx skills add 4fuu/cairn --skill cairn-session-search -a amp -g -y
```

`npx skills` owns the per-agent paths. Use `-a claude-code`, `-a codex`, or repeat `-a` for several agents. See [Skill](docs/skill.md).

## Quick start

```sh
cairn version
cairn build
cairn status
cairn overview
cairn search "auth redirect"
cairn show <session-id> --around <uuid> --before 3 --after 3 --tools
cairn sql "SELECT source, COUNT(*) FROM sessions GROUP BY source"
```

The first build reads every configured source root. Later queries refresh incrementally, re-reading only files whose mtime, size, or prefix hash changed.

Output is compact by default: search keeps the best message UUID for `show --around`, `show` keeps visible text, and tool/thinking rows stay out until requested. Use `--format json` for agents, `--detail` for scores/IDs/paths/tokens, `--include-meta` for thinking/tool output, and `--tools` for tool rows.

## Performance

Measured with a release build on a Linux x86-64 machine, warm filesystem cache, and `--no-update-check`:

| Corpus | Operation | Time |
|---|---|---:|
| 20,100 messages / 6.4 MB JSONL | Full build | ~3.1 s |
| Same index | Multi-word English search | ~0.13–0.14 s |
| Same index | Exact message search | ~0.06 s |

The index is SQLite with FTS5; search uses trigram and jieba word indexes. Times vary with disk, corpus shape, query length, and whether the first build has to parse the source files. Normal queries also run an incremental refresh.

## Supported agents

Cairn follows the session locations used by the agents themselves, including Claude Code, Codex, pi/omp, DSH, OpenCode/ZCode, Cline, Grok, Kimi, WorkBuddy, Devin, MiniMax, Hermes, Antigravity, LiveAgent, and DimAgent.

The complete list of source IDs, default roots, environment variables, and SQLite-backed providers is in [Sources](docs/sources.md). `cairn sources` shows what exists on this machine.

Amp threads are server-side and are not indexed; use Amp's own thread tools. Cursor's blob store, Alma, OpenMausBot, and CatPaw do not have adapters yet.

## Documentation

| Goal | Read |
|---|---|
| Install, upgrade, or remove the CLI or skill | [Install](docs/install.md) |
| Check provider paths and source IDs | [Sources](docs/sources.md) |
| Look up commands and flags | [CLI reference](docs/cli.md) |
| Understand search ranking and edge cases | [Search](docs/search.md) |
| Change indexing, schema, or SQL safety | [Architecture](docs/architecture.md) |
| Install or edit the agent skill | [Skill](docs/skill.md) |
| Build, test, or add a provider | [Development](docs/development.md) |
| Publish a version and write release notes | [Release](docs/release.md) |
| Diagnose missing data or installs | [Troubleshooting](docs/troubleshooting.md) |

The docs index is [docs/README.md](docs/README.md).

## Design boundaries

- **Read-only over transcripts.** Cairn never edits provider files, resumes a session, or writes into a provider database.
- **No daemon.** Each invocation opens the index, refreshes it if needed, and exits.
- **Agent-oriented.** `--json` everywhere, bounded output, stable IDs, and a skill that names the workflow instead of dumping a schema.
- **Transcript text is untrusted.** The skill tells the agent not to follow instructions found in history.

More detail: [Architecture](docs/architecture.md).

## Development

```sh
cargo fmt --check
cargo test --locked
cargo build --locked --release
```

See [Development](docs/development.md) for the module map, provider checklist, and tests. [AGENTS.md](AGENTS.md) is the short repository guide for coding agents.

## License

MIT.
