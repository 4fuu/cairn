# Install

Cairn ships as a single Rust binary with SQLite and its FTS5 extensions bundled. There is no runtime, daemon, or desktop app.

## Supported platforms

| Platform | Architecture | Install |
|---|---|---|
| Linux | x86-64, ARM64 | one-line installer or release archive |
| macOS | Apple Silicon (ARM64) | Homebrew, one-line installer, or release archive |
| Windows | x86-64 | Scoop or release archive |
| Other | source build | `cargo install --git` |

Intel macOS binaries are not published. Build from source on an Intel Mac.

## One-line installer

Linux x86-64/ARM64 and Apple Silicon macOS:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://raw.githubusercontent.com/4fuu/cairn/main/scripts/install.sh | sh
```

The installer:

1. resolves the latest GitHub release;
2. downloads the archive and `SHA256SUMS`;
3. verifies the archive checksum;
4. installs a versioned directory under `~/.local/bin`;
5. creates the `cairn` launcher symlink.

Use a specific version by passing it as the first argument:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://raw.githubusercontent.com/4fuu/cairn/main/scripts/install.sh | sh -s -- 2026.1127.5
```

Set `CAIRN_INSTALL_DIR` to change the install directory. Add it to `PATH` if the installer reports that it is missing.

## Homebrew (Apple Silicon macOS)

```sh
brew tap 4fuu/cairn https://github.com/4fuu/cairn
brew install 4fuu/cairn/cairn
```

## Scoop (64-bit Windows)

```powershell
scoop bucket add cairn https://github.com/4fuu/cairn
scoop install cairn
```

## Cargo

```sh
cargo install --git https://github.com/4fuu/cairn
```

From a local checkout:

```sh
git clone https://github.com/4fuu/cairn
cd cairn
cargo install --path .
```

## Install the skill

The skill is distributed separately from the binary and installed by [Vercel Labs' `skills` CLI](https://github.com/vercel-labs/skills). The CLI owns the per-agent directories; Cairn does not copy skill files itself.

```sh
npx skills add 4fuu/cairn --skill cairn-session-search -a amp -g -y
```

Use `-a claude-code`, `-a codex`, or repeat `-a` for several agents. Omit `-g` to install into the current project. `--copy` copies instead of linking.

```sh
npx skills add 4fuu/cairn --skill cairn-session-search -a claude-code -a codex -g -y
npx skills add 4fuu/cairn --list
```

Update or remove the installed skill:

```sh
npx skills update cairn-session-search -g
npx skills remove cairn-session-search -g -y
```

See [Skill](skill.md) for the layout and runtime behavior.

## Verify

```sh
cairn version
cairn status
```

`cairn version` prints the current version and checks the latest GitHub release. Normal commands check at most once every 24 hours and print an update notice to stderr; the state lives in `~/.cairn/update-check.json` (or `$CAIRN_HOME/update-check.json`). Disable it with `--no-update-check` or `CAIRN_NO_UPDATE_CHECK=1`.

`cairn status` builds the index on first use. If a source is missing, run `cairn sources` and see [Sources](sources.md).

## Uninstall

- One-line installer: remove `~/.local/bin/cairn` and `~/.local/bin/cairn-*`.
- Homebrew: `brew uninstall cairn`.
- Scoop: `scoop uninstall cairn`.
- Skill: `npx skills remove cairn-session-search -g -y`.

The index is separate from the binary. Remove `~/.cairn` if you also want to delete the local index.
