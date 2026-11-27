# Troubleshooting

## `cairn: command not found`

Add the install directory to `PATH`. The one-line installer uses `~/.local/bin` by default. Homebrew and Scoop manage their own paths.

## No sessions or a missing source

```sh
cairn sources
```

`exists: false` means the provider root was not found. Check the provider's environment variable and the paths in [Sources](sources.md). To point at a non-default location:

```sh
cairn --root codex=/path/to/sessions build
```

`cairn sources` counts files under each root; a root can exist but be empty.

## Search returns nothing

- Default search covers normal text only. Add `--include-meta` for thinking, tool results, and tool-use rows.
- Tool-call arguments are not full-text indexed. A command that appears only in `tool_calls.input_json` will not match.
- One- and two-character queries use `LIKE`; try a three-character substring.
- Search is literal. Try a shorter identifier or a phrase that actually appears in the transcript.
- Check the project/source/time filters; an over-narrow `--project` or `--after` can hide the hit.

## The index looks stale

Every query refreshes incrementally. Force a full re-read with:

```sh
cairn build --full
```

If the database itself is damaged, remove `~/.cairn/cairn.sqlite` and rebuild. The index is derived data; the source transcripts are untouched.

## SQL is rejected

`cairn sql` accepts one `SELECT` or `WITH` statement. Writes, schema changes, unsafe PRAGMAs, extensions, and file functions are denied. Use `cairn schema` to inspect the actual tables, and `--max-rows`/`--timeout` to keep queries bounded.

## Installer checksum failure

The installer verifies the archive against `SHA256SUMS`. Retry the download; if it still fails, download the archive and `SHA256SUMS` manually from the release page and compare with `sha256sum`.

## `npx skills` does not find Cairn

```sh
npx skills add 4fuu/cairn --list
```

The skill must be under `skills/cairn-session-search/SKILL.md`. `npx skills` requires Node.js 22.20 or newer. See [Skill](skill.md).

## macOS Intel

Only Apple Silicon macOS archives are published. Build from source:

```sh
cargo install --git https://github.com/4fuu/cairn
```

## Windows

Use the Scoop manifest or the x86-64 zip from the release page. The one-line shell installer is for Linux and Apple Silicon macOS.

## Update notices are noisy or blocked

Normal commands check the latest GitHub release at most once every 24 hours. The state is in `~/.cairn/update-check.json`; delete it to force the next check. Disable checks with:

```sh
cairn --no-update-check status
```

or set `CAIRN_NO_UPDATE_CHECK=1`. Update notices go to stderr, so they never corrupt JSON output. `cairn version` performs a check unless update checks are disabled.

## Source sessions are not modified

Cairn never edits a provider file, resumes a session, or writes into a provider database. If a source is being written while indexed, the next refresh picks up the new bytes.
