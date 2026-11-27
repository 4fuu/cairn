# Cairn documentation

The [README](../README.md) is the entry point. This directory owns the details.

| Document | Read it when |
|---|---|
| [Install](install.md) | Installing, upgrading, or removing the CLI or skill |
| [Sources](sources.md) | Checking which agents are indexed, where their files live, or adding a root override |
| [CLI reference](cli.md) | Looking up a command, flag, or JSON output |
| [Search](search.md) | Understanding ranking, Chinese/English behavior, or why a query missed |
| [Architecture](architecture.md) | Changing indexing, the schema, search, or the SQL guard |
| [Skill](skill.md) | Installing or editing the agent-facing skill |
| [Development](development.md) | Building, testing, or adding a provider |
| [Release](release.md) | Publishing a version, updating package metadata, or writing release notes |
| [Release notes](releases/README.md) | The required post-release note format |
| [Troubleshooting](troubleshooting.md) | Diagnosing missing data, search misses, or install problems |

## Ownership

- The README owns the project identity, first-run path, and critical boundaries.
- `docs/sources.md` owns the provider list and root resolution rules.
- `docs/cli.md` owns command and flag behavior.
- `docs/search.md` owns search semantics.
- `docs/architecture.md` owns the data model and indexing/query design.
- `docs/release.md` owns versioning and the release checklist.
- `skills/cairn/` owns the instructions an agent loads at runtime.

When behavior changes, update the document that owns it and only adjust summaries elsewhere.
