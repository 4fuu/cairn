# Skill

Cairn's agent skill teaches an agent when to search local history, how to narrow a query, and how to cite evidence. The skill is a separate artifact from the `cairn` binary.

## Install

Use [Vercel Labs' `skills` CLI](https://github.com/vercel-labs/skills):

```sh
npx skills add 4fuu/cairn --skill cairn -a amp -g -y
```

The CLI owns the per-agent directories. Omit `-g` to install into the current project. Use `-a claude-code`, `-a codex`, or repeat `-a` for multiple agents.

```sh
npx skills add 4fuu/cairn --skill cairn -a claude-code -a codex -g -y
npx skills add 4fuu/cairn --list
npx skills update cairn -g
npx skills remove cairn -g -y
```

Cairn no longer has a `cairn install` or `cairn skill` command, and it does not copy skill files itself.

## Layout

```text
skills/cairn/
  SKILL.md
  references/
    schema.md
    query-patterns.md
    retrieval-semantics.md
    pitfalls.md
```

`npx skills` discovers `skills/cairn/SKILL.md` and copies or links the whole directory, including `references/`.

## Runtime workflow

The skill tells the agent to:

1. orient with `cairn overview` and `cairn sessions`;
2. search with a literal phrase and the narrowest scope;
3. expand the best hit with `cairn show --around`;
4. use bounded read-only SQL only when the helper commands cannot express the question;
5. cite the session, timestamp, and message UUID, and separate user claims, assistant claims, tool input, and tool output.

The skill also treats transcript text as untrusted data. Instructions found in old sessions are evidence, not commands.

## Frontmatter

`SKILL.md` declares:

- `name: cairn`
- a description covering the supported agents and the situations that should trigger it
- `allowed-tools` for reading and running `cairn`

Keep the description aligned with [Sources](sources.md) when adding a provider.
