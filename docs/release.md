# Release

## Versioning

Cairn uses a calendar version:

```text
YYYY.MDD.REVISION
```

The date is the release date in `Asia/Hong_Kong`. The month is not padded, the day is two digits, and the first release of a day uses revision `0`.

Examples:

- `2026.1127.0`: first release on 2026-11-27
- `2026.1127.1`: second release that day
- `2026.1127.2`: third release that day

Never reuse a published tag or version. If a release needs a fix after it is public, increment `REVISION` and publish a new release.

## Prepare

1. Update `Cargo.toml` and `Cargo.lock` to the new version.
2. Update the current-release line in the README.
3. Update any document whose behavior changed.
4. Run the local checks:

```sh
cargo fmt --check
cargo test --locked
cargo build --locked --release
```

5. Commit and push to `main`.

The release workflow reads the version from `Cargo.toml` and rejects it when its date does not match today's `Asia/Hong_Kong` date.

## Publish

Dispatch the release workflow from `main`:

```sh
gh workflow run release.yml --ref main
gh run watch
```

`.github/workflows/release.yml`:

1. validates the version and date;
2. runs `cargo fmt --check` and `cargo test --locked`;
3. builds Linux x86-64, Linux ARM64, macOS ARM64, and Windows x86-64 archives;
4. generates `SHA256SUMS`;
5. updates `Formula/cairn.rb` and `bucket/cairn.json` with checksums and commits them;
6. creates the GitHub release at tag `vVERSION`;
7. runs the Linux installer smoke test against the published release.

The release assets are:

```text
cairn-VERSION-x86_64-unknown-linux-gnu.tar.gz
cairn-VERSION-aarch64-unknown-linux-gnu.tar.gz
cairn-VERSION-aarch64-apple-darwin.tar.gz
cairn-VERSION-x86_64-pc-windows-msvc.zip
SHA256SUMS
```

## Verify

```sh
gh release view "v$VERSION"
gh release download "v$VERSION" --dir /tmp/cairn-release
(cd /tmp/cairn-release && sha256sum -c SHA256SUMS)
```

Also verify that the public installer and `cargo install --git` still work when the release changes packaging.

## Write the release note

**Every release requires a written release note after the workflow succeeds.** The generated GitHub notes are only a starting point; replace them with a short, human-written note.

1. Create `docs/releases/v$VERSION.md` from the template in [`docs/releases/README.md`](releases/README.md).
2. Cover highlights, changes, fixes, upgrade notes, and verification.
3. Publish it to the GitHub release:

```sh
gh release edit "v$VERSION" --notes-file "docs/releases/v$VERSION.md"
```

4. Commit the note file.

A release is not complete until the GitHub release body matches the committed note.

## Package metadata

`scripts/update-package-metadata.py` generates the Homebrew formula and Scoop manifest from `SHA256SUMS`. The release workflow runs it and commits the result. Do not hand-edit checksums or URLs.

## If a release fails

- A build failure: fix the source, push, and dispatch the workflow again with the same version if the tag was not created.
- A failure after the tag exists: do not move or reuse the tag; publish a new revision.
- A bad public release: publish a new revision with a fix. Do not rewrite published history.
