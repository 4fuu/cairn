# Release notes

Write one note per release in this directory, named `vVERSION.md`, after the release workflow succeeds. The GitHub release body must use the same text.

## Template

```markdown
# Cairn vVERSION

## Highlights

- The one or two changes a user should know first.

## Changes

- User-visible features or behavior changes.

## Fixes

- Bugs fixed, with the affected provider or command.

## Upgrade notes

- Breaking changes, migration steps, or "No action required".

## Verification

- Tests, platform builds, installer smoke test, and any manual checks.
```

## Publish the note

```sh
gh release edit "v$VERSION" --notes-file "docs/releases/v$VERSION.md"
```

Commit the note file with the release follow-up. A release is not complete until the GitHub release body contains the written note.
